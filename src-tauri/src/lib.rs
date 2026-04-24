use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{Manager, State};

mod auth;
mod channels;
mod chat;
mod config;
mod notify;
mod platforms;
mod refresh;
mod settings;
mod streamlink;
mod tray;

use channels::{Channel, ChannelStore, Livestream, SharedStore};
use chat::ChatManager;
use notify::NotifyTracker;
use platforms::{parse_channel_input, Platform};
use settings::{SharedSettings, Settings};

struct AppState {
    store: SharedStore,
    http: reqwest::Client,
    notifier: Arc<NotifyTracker>,
    settings: SharedSettings,
}

impl AppState {
    fn new() -> anyhow::Result<Self> {
        let store = ChannelStore::load()?;
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "livestreamlist/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/mkeguy106/livestreamlist)"
            ))
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let settings = settings::load().unwrap_or_else(|e| {
            log::warn!("settings load failed, using defaults: {e:#}");
            Settings::default()
        });
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            http,
            notifier: Arc::new(NotifyTracker::new()),
            settings: Arc::new(parking_lot::RwLock::new(settings)),
        })
    }
}

fn err_string<E: std::fmt::Display>(e: E) -> String {
    format!("{e}")
}

#[tauri::command]
fn list_livestreams(state: State<'_, AppState>) -> Vec<Livestream> {
    state.store.lock().snapshot()
}

#[tauri::command]
fn list_channels(state: State<'_, AppState>) -> Vec<Channel> {
    state.store.lock().channels().to_vec()
}

#[tauri::command]
fn add_channel_from_input(
    input: String,
    state: State<'_, AppState>,
) -> Result<Channel, String> {
    let parsed = parse_channel_input(&input)
        .ok_or_else(|| format!("couldn't recognise '{input}' as a channel URL"))?;
    let channel = Channel {
        platform: parsed.platform,
        channel_id: parsed.channel_id,
        display_name: parsed.display_name,
        favorite: false,
        dont_notify: false,
        auto_play: false,
        added_at: Some(Utc::now()),
    };
    state
        .store
        .lock()
        .add(channel.clone())
        .map_err(err_string)?;
    Ok(channel)
}

#[tauri::command]
fn remove_channel(unique_key: String, state: State<'_, AppState>) -> Result<bool, String> {
    state.store.lock().remove(&unique_key).map_err(err_string)
}

#[tauri::command]
fn set_favorite(
    unique_key: String,
    favorite: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    state
        .store
        .lock()
        .set_favorite(&unique_key, favorite)
        .map_err(err_string)
}

#[tauri::command]
async fn refresh_all(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<Livestream>, String> {
    let store = Arc::clone(&state.store);
    let client = state.http.clone();
    let notifier = Arc::clone(&state.notifier);
    let snapshot = refresh::refresh_all(store, client).await.map_err(err_string)?;

    // Fire desktop notifications for offline→live transitions, and update
    // the tray tooltip with the new counts.
    let channels = state.store.lock().channels().to_vec();
    let notify_enabled = state.settings.read().general.notify_on_live;
    if notify_enabled {
        notifier.detect_and_notify(&app, &channels, &snapshot);
    } else {
        // Still advance the tracker so re-enabling doesn't retro-fire every
        // currently-live channel.
        notifier.seed(&snapshot);
    }
    let live = snapshot.iter().filter(|l| l.is_live).count();
    tray::update_tooltip(&app, live, snapshot.len());

    Ok(snapshot)
}

#[tauri::command]
fn launch_stream(
    unique_key: String,
    quality: Option<String>,
    state: State<'_, AppState>,
) -> Result<u32, String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    streamlink::launch(
        channel.platform,
        &channel.channel_id,
        quality.as_deref().unwrap_or("best"),
    )
    .map_err(err_string)
}

#[derive(serde::Serialize)]
struct ImportResult {
    added: u32,
    skipped: u32,
    total_seen: u32,
}

#[tauri::command]
async fn import_twitch_follows(
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let token = auth::twitch::stored_token()
        .map_err(err_string)?
        .ok_or_else(|| "not logged in to Twitch".to_string())?;
    let identity = auth::twitch::stored_identity()
        .ok_or_else(|| "missing Twitch identity — log in again".to_string())?;

    let follows = platforms::twitch::fetch_followed_channels(
        &state.http,
        &token,
        &identity.user_id,
    )
    .await
    .map_err(err_string)?;

    let mut added = 0u32;
    let mut skipped = 0u32;
    let total_seen = follows.len() as u32;

    for f in follows {
        let channel = Channel {
            platform: Platform::Twitch,
            channel_id: f.broadcaster_login.clone(),
            display_name: if f.broadcaster_name.is_empty() {
                f.broadcaster_login.clone()
            } else {
                f.broadcaster_name
            },
            favorite: false,
            dont_notify: false,
            auto_play: false,
            added_at: Some(Utc::now()),
        };
        if state.store.lock().contains(Platform::Twitch, &channel.channel_id) {
            skipped += 1;
            continue;
        }
        match state.store.lock().add(channel) {
            Ok(()) => added += 1,
            Err(_) => skipped += 1,
        }
    }

    Ok(ImportResult {
        added,
        skipped,
        total_seen,
    })
}

#[tauri::command]
async fn list_socials(
    unique_key: String,
    state: State<'_, AppState>,
) -> Result<Vec<platforms::twitch::SocialLink>, String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    match channel.platform {
        Platform::Twitch => platforms::twitch::fetch_socials(&state.http, &channel.channel_id)
            .await
            .map_err(err_string),
        // Other platforms land in Phase 3b+.
        _ => Ok(Vec::new()),
    }
}

#[tauri::command]
fn chat_open_popout(
    unique_key: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;

    let livestream = state
        .store
        .lock()
        .snapshot()
        .into_iter()
        .find(|l| l.unique_key == unique_key);

    let (url, title) = match channel.platform {
        Platform::Youtube => {
            // YouTube's popout chat needs the live video id; we only have one
            // when the channel is live right now.
            let Some(ls) = livestream.as_ref().and_then(|l| l.title.as_ref().map(|_| l)) else {
                return Err("channel isn't live".to_string());
            };
            let id_field = ls
                .thumbnail_url
                .as_ref()
                .and_then(|u| yt_id_from_thumbnail(u))
                .or_else(|| ls.game_slug.clone()); // legacy fallback; unused
            let Some(video_id) = id_field else {
                return Err("couldn't resolve the YouTube video id".to_string());
            };
            (
                format!("https://www.youtube.com/live_chat?is_popout=1&v={video_id}"),
                format!("YouTube · {}", channel.display_name),
            )
        }
        Platform::Chaturbate => (
            format!("https://chaturbate.com/{}/", channel.channel_id),
            format!("Chaturbate · {}", channel.display_name),
        ),
        Platform::Twitch => (
            format!("https://www.twitch.tv/popout/{}/chat?popout=", channel.channel_id),
            format!("Twitch · {}", channel.display_name),
        ),
        Platform::Kick => (
            format!("https://kick.com/popout/{}/chat", channel.channel_id),
            format!("Kick · {}", channel.display_name),
        ),
    };

    let label = format!("popout-{}", slugify(&unique_key));
    // If a window with this label already exists, just focus it.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url.parse().map_err(err_string)?),
    )
    .title(title)
    .inner_size(460.0, 700.0)
    .min_inner_size(320.0, 480.0)
    .build()
    .map_err(err_string)?;

    Ok(())
}

fn yt_id_from_thumbnail(url: &str) -> Option<String> {
    // YouTube live thumbnails look like
    // `https://i.ytimg.com/vi/{video_id}/hqdefault_live.jpg?…`.
    let trim = url.trim();
    let marker = "/vi/";
    let start = trim.find(marker)? + marker.len();
    let rest = &trim[start..];
    let end = rest.find('/').unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // Only http(s) — don't let the renderer hand us file:// or javascript: URIs.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!("refusing to open non-http url: {url}"));
    }
    streamlink::open_browser(&url).map_err(err_string)
}

#[tauri::command]
fn chat_connect(
    unique_key: String,
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<(), String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    let key = channel.unique_key();
    chat.connect(channel.platform, channel.channel_id, key)
        .map_err(err_string)
}

#[tauri::command]
fn chat_disconnect(unique_key: String, chat: State<'_, Arc<ChatManager>>) -> Result<(), String> {
    chat.disconnect(&unique_key);
    Ok(())
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.read().clone()
}

#[tauri::command]
fn update_settings(
    state: State<'_, AppState>,
    patch: Settings,
) -> Result<Settings, String> {
    {
        let mut g = state.settings.write();
        *g = patch;
    }
    let snapshot = state.settings.read().clone();
    if let Err(e) = settings::save(&snapshot) {
        log::warn!("saving settings failed: {e:#}");
        return Err(err_string(e));
    }
    Ok(snapshot)
}

#[tauri::command]
fn list_emotes(
    unique_key: String,
    chat: State<'_, Arc<ChatManager>>,
) -> Vec<chat::Emote> {
    chat.list_emotes(&unique_key)
}

#[tauri::command]
fn chat_send(
    unique_key: String,
    text: String,
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<(), String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;

    // Normalize and length-cap. Per-platform formatting (PRIVMSG / REST body)
    // happens in the platform task.
    let clean = text.replace(['\r', '\n'], " ");
    let clean = clean.chars().take(500).collect::<String>();
    if clean.trim().is_empty() {
        return Ok(());
    }

    match channel.platform {
        Platform::Twitch | Platform::Kick => chat.send_raw(&unique_key, clean).map_err(err_string),
        _ => Err("sending not yet supported for this platform".to_string()),
    }
}

#[derive(serde::Serialize)]
struct AuthStatus {
    twitch: Option<auth::twitch::TwitchIdentity>,
    kick: Option<auth::kick::KickIdentity>,
}

#[tauri::command]
async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, String> {
    let twitch = auth::twitch::status(&state.http)
        .await
        .map_err(err_string)?;
    let kick = auth::kick::status(&state.http)
        .await
        .map_err(err_string)?;
    Ok(AuthStatus { twitch, kick })
}

#[tauri::command]
async fn twitch_login(
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<auth::twitch::TwitchIdentity, String> {
    let identity = auth::twitch::login(&state.http)
        .await
        .map_err(err_string)?;
    // Auth state changed — reconnect any live Twitch chat tasks so they
    // pick up the new credentials.
    chat.reconnect_platform(Platform::Twitch, &state.store);
    Ok(identity)
}

#[tauri::command]
fn twitch_logout(
    chat: State<'_, Arc<ChatManager>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    auth::twitch::logout().map_err(err_string)?;
    chat.reconnect_platform(Platform::Twitch, &state.store);
    Ok(())
}

#[tauri::command]
async fn kick_login(
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<auth::kick::KickIdentity, String> {
    let identity = auth::kick::login(&state.http).await.map_err(err_string)?;
    chat.reconnect_platform(Platform::Kick, &state.store);
    Ok(identity)
}

#[tauri::command]
fn kick_logout(
    chat: State<'_, Arc<ChatManager>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    auth::kick::logout().map_err(err_string)?;
    chat.reconnect_platform(Platform::Kick, &state.store);
    Ok(())
}

#[tauri::command]
fn replay_chat_history(
    unique_key: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<chat::models::ChatMessage>, String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    chat::log_store::read_recent(channel.platform, &channel.channel_id, limit.min(1000))
        .map_err(err_string)
}

#[tauri::command]
fn open_in_browser(unique_key: String, state: State<'_, AppState>) -> Result<(), String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    let url = match channel.platform {
        Platform::Twitch => format!("https://www.twitch.tv/{}", channel.channel_id),
        Platform::Youtube => {
            if channel.channel_id.starts_with("UC") {
                format!("https://www.youtube.com/channel/{}", channel.channel_id)
            } else {
                format!("https://www.youtube.com/@{}", channel.channel_id)
            }
        }
        Platform::Kick => format!("https://kick.com/{}", channel.channel_id),
        Platform::Chaturbate => format!("https://chaturbate.com/{}/", channel.channel_id),
    };
    streamlink::open_browser(&url).map_err(err_string)
}

/// WebKitGTK on NVIDIA + Wayland (KDE Plasma) crashes with "Error 71 (Protocol
/// error) dispatching to Wayland display" when the DMABUF renderer is enabled.
/// Disabling it falls back to a software path that renders correctly.
///
/// Also opt out of the compositing crash seen on some Mesa/Nvidia stacks.
/// No-op everywhere else.
#[cfg(target_os = "linux")]
fn apply_linux_webkit_workarounds() {
    fn set_if_unset(key: &str, val: &str) {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, val);
        }
    }
    set_if_unset("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_webkit_workarounds() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_linux_webkit_workarounds();
    let state = AppState::new().expect("failed to initialize app state");

    let http_for_chat = state.http.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(
            |app: &tauri::AppHandle, _argv: Vec<String>, _cwd: String| {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            },
        ))
        .manage(state)
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            let chat_mgr = ChatManager::new(app.handle().clone(), http_for_chat.clone());
            app.manage(chat_mgr);
            tray::build(&app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_livestreams,
            list_channels,
            add_channel_from_input,
            remove_channel,
            set_favorite,
            refresh_all,
            launch_stream,
            open_in_browser,
            open_url,
            list_socials,
            chat_connect,
            chat_disconnect,
            chat_send,
            chat_open_popout,
            list_emotes,
            replay_chat_history,
            get_settings,
            update_settings,
            auth_status,
            twitch_login,
            twitch_logout,
            kick_login,
            kick_logout,
            import_twitch_follows,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
