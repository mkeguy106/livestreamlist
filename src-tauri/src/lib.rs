use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

mod auth;
mod channels;
mod chat;
mod config;
mod embed;
mod notify;
mod platforms;
mod player;
mod refresh;
mod settings;
mod streamlink;
mod tray;
mod users;
mod window_state;

use channels::{Channel, ChannelStore, Livestream, SharedStore};
use chat::ChatManager;
use notify::NotifyTracker;
use platforms::{parse_channel_input, pronouns::PronounsCache, Platform};
use player::PlayerManager;
use settings::{Settings, SharedSettings};
use users::{UserMetadata, UserMetadataPatch, UserStore};

struct AppState {
    store: SharedStore,
    http: reqwest::Client,
    notifier: Arc<NotifyTracker>,
    settings: SharedSettings,
    users: Arc<UserStore>,
    pronouns: Arc<PronounsCache>,
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
        let users = Arc::new(UserStore::open_default().unwrap_or_else(|e| {
            log::warn!("user store open failed, using empty: {e:#}");
            // Empty store with no path won't persist, but avoids panicking on
            // start. Realistically open_default already handles parse errors;
            // this fallback only fires if the config dir itself is unreadable.
            UserStore::open(std::path::PathBuf::from("/dev/null"))
                .unwrap_or_else(|_| panic!("could not even fall back to /dev/null user store"))
        }));
        let pronouns = PronounsCache::new(http.clone());
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            http,
            notifier: Arc::new(NotifyTracker::new()),
            settings: Arc::new(parking_lot::RwLock::new(settings)),
            users,
            pronouns,
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
fn add_channel_from_input(input: String, state: State<'_, AppState>) -> Result<Channel, String> {
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
    let unique_key = channels::channel_key_of(&unique_key).to_string();
    state.store.lock().remove(&unique_key).map_err(err_string)
}

#[tauri::command]
fn set_favorite(
    unique_key: String,
    favorite: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let unique_key = channels::channel_key_of(&unique_key).to_string();
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
    let yt_browser = state.settings.read().general.youtube_cookies_browser.clone();
    let snapshot = refresh::refresh_all(store, client, yt_browser)
        .await
        .map_err(err_string)?;

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
    player: State<'_, Arc<PlayerManager>>,
) -> Result<u32, String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    player
        .launch(
            channel.unique_key(),
            channel.platform,
            &channel.channel_id,
            quality.as_deref().unwrap_or("best"),
            None, // Turbo lives on a sibling branch; wire up when it lands
        )
        .map_err(err_string)
}

#[tauri::command]
fn stop_stream(unique_key: String, player: State<'_, Arc<PlayerManager>>) -> bool {
    player.stop(&unique_key)
}

#[tauri::command]
fn list_playing(player: State<'_, Arc<PlayerManager>>) -> Vec<String> {
    player.playing()
}

#[derive(serde::Serialize)]
struct ImportResult {
    added: u32,
    skipped: u32,
    total_seen: u32,
}

#[tauri::command]
async fn import_twitch_follows(state: State<'_, AppState>) -> Result<ImportResult, String> {
    let token = auth::twitch::stored_token()
        .map_err(err_string)?
        .ok_or_else(|| "not logged in to Twitch".to_string())?;
    let identity = auth::twitch::stored_identity()
        .ok_or_else(|| "missing Twitch identity — log in again".to_string())?;

    let follows =
        platforms::twitch::fetch_followed_channels(&state.http, &token, &identity.user_id)
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
        if state
            .store
            .lock()
            .contains(Platform::Twitch, &channel.channel_id)
        {
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
            let Some(ls) = livestream
                .as_ref()
                .and_then(|l| l.title.as_ref().map(|_| l))
            else {
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
            format!(
                "https://www.twitch.tv/popout/{}/chat?popout=",
                channel.channel_id
            ),
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

    let popout = tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url.parse().map_err(err_string)?),
    )
    .title(title)
    .inner_size(460.0, 700.0)
    .min_inner_size(320.0, 480.0)
    .build()
    .map_err(err_string)?;

    let _ = popout.set_focus();
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
fn embed_mount(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    embeds: State<'_, Arc<embed::EmbedManager>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<bool, String> {
    embeds
        .mount(&app, &state.store, &unique_key, x, y, width, height)
        .map_err(err_string)
}

#[tauri::command]
fn embed_position(
    embeds: State<'_, Arc<embed::EmbedManager>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    embeds
        .position(&unique_key, x, y, width, height)
        .map_err(err_string)
}

#[tauri::command]
fn embed_unmount(embeds: State<'_, Arc<embed::EmbedManager>>, unique_key: String) {
    embeds.unmount(&unique_key);
}

#[tauri::command]
fn embed_set_visible(embeds: State<'_, Arc<embed::EmbedManager>>, visible: bool) {
    embeds.set_visible_all(visible);
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
fn update_settings(state: State<'_, AppState>, patch: Settings) -> Result<Settings, String> {
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
fn get_user_metadata(user_key: String, state: State<'_, AppState>) -> Result<UserMetadata, String> {
    let (platform_str, user_id) = user_key
        .split_once(':')
        .ok_or_else(|| format!("invalid user_key {user_key}"))?;
    let platform = Platform::from_str(platform_str)
        .ok_or_else(|| format!("unknown platform {platform_str}"))?;
    Ok(state
        .users
        .get(&user_key)
        .unwrap_or_else(|| UserMetadata::new_default(platform, user_id.to_string())))
}

#[tauri::command]
fn set_user_metadata(
    app: tauri::AppHandle,
    chat: State<'_, Arc<ChatManager>>,
    state: State<'_, AppState>,
    user_key: String,
    patch: UserMetadataPatch,
) -> Result<UserMetadata, String> {
    let (platform_str, user_id) = user_key
        .split_once(':')
        .ok_or_else(|| format!("invalid user_key {user_key}"))?;
    let platform = Platform::from_str(platform_str)
        .ok_or_else(|| format!("unknown platform {platform_str}"))?;

    let was_blocked = state.users.is_blocked(&user_key);
    let new_blocked = patch.blocked.unwrap_or(was_blocked);

    let result = state
        .users
        .apply(&user_key, platform, user_id, patch)
        .map_err(err_string)?;

    // If we just transitioned to blocked, fan out a moderation event to every
    // currently-connected channel so already-rendered messages get purged
    // client-side. The per-channel chat task picks the same store up on the
    // next message and drops it server-side.
    if !was_blocked && new_blocked {
        let login = if !result.last_known_login.is_empty() {
            result.last_known_login.clone()
        } else {
            // Fall back to user_id so the frontend still has *something* to
            // match against if the row was created without hints.
            user_id.to_string()
        };
        for chan_key in chat.connected_keys() {
            let ev = chat::models::ChatModerationEvent {
                channel_key: chan_key.clone(),
                kind: "user_blocked".into(),
                target_login: Some(login.clone()),
                target_msg_id: None,
                duration_seconds: None,
            };
            let _ = app.emit(&format!("chat:moderation:{chan_key}"), ev);
        }
    }
    Ok(result)
}

#[tauri::command]
async fn get_user_profile(
    state: State<'_, AppState>,
    channel_key: String,
    user_id: String,
    login: String,
) -> Result<platforms::twitch_users::UserProfile, String> {
    let broadcaster_login = channel_key
        .strip_prefix("twitch:")
        .ok_or_else(|| format!("non-twitch channel_key {channel_key}"))?;

    let broadcaster_id =
        platforms::twitch_users::fetch_user_by_login(&state.http, broadcaster_login)
            .await
            .map_err(err_string)?
            .id;

    platforms::twitch_users::build_profile(
        &state.http,
        &state.pronouns,
        &broadcaster_id,
        &user_id,
        &login,
    )
    .await
    .map_err(err_string)
}

#[tauri::command]
fn get_user_messages(
    state: State<'_, AppState>,
    channel_key: String,
    user_id: String,
    limit: usize,
) -> Result<Vec<chat::models::ChatMessage>, String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == channel_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {channel_key}"))?;
    chat::log_store::read_user_messages(
        channel.platform,
        &channel.channel_id,
        &user_id,
        limit.min(1000),
    )
    .map_err(err_string)
}

#[tauri::command]
fn list_blocked_users(state: State<'_, AppState>) -> Vec<UserMetadata> {
    let mut v: Vec<_> = state
        .users
        .snapshot()
        .into_iter()
        .filter(|m| m.blocked)
        .collect();
    v.sort_by(|a, b| {
        a.last_known_display_name
            .to_lowercase()
            .cmp(&b.last_known_display_name.to_lowercase())
    });
    v
}

#[tauri::command]
fn list_emotes(unique_key: String, chat: State<'_, Arc<ChatManager>>) -> Vec<chat::Emote> {
    chat.list_emotes(&unique_key)
}

#[tauri::command]
async fn chat_send(
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
        Platform::Twitch | Platform::Kick => {
            chat.send_raw(&unique_key, clean).await.map_err(err_string)
        }
        _ => Err("sending not yet supported for this platform".to_string()),
    }
}

#[derive(serde::Serialize)]
struct AuthStatus {
    twitch: Option<auth::twitch::TwitchIdentity>,
    kick: Option<auth::kick::KickIdentity>,
    youtube: YoutubeAuthStatus,
    chaturbate: ChaturbateAuthStatus,
}

#[derive(serde::Serialize)]
struct YoutubeAuthStatus {
    /// Configured browser name, if any (`chrome`, `firefox`, …).
    browser: Option<String>,
    /// True when a manually-pasted cookies file is on disk.
    has_paste: bool,
}

#[derive(serde::Serialize)]
struct ChaturbateAuthStatus {
    signed_in: bool,
    last_verified_at: Option<String>, // RFC3339, None when not signed in
}

#[tauri::command]
async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, String> {
    let twitch = auth::twitch::status(&state.http)
        .await
        .map_err(err_string)?;
    let kick = auth::kick::status(&state.http).await.map_err(err_string)?;
    let browser = state.settings.read().general.youtube_cookies_browser.clone();
    let has_paste = auth::youtube::cookies_file_present();
    let chaturbate = match auth::chaturbate::load().map_err(err_string)? {
        Some(stamp) => ChaturbateAuthStatus {
            signed_in: true,
            last_verified_at: Some(stamp.last_verified_at.to_rfc3339()),
        },
        None => ChaturbateAuthStatus {
            signed_in: false,
            last_verified_at: None,
        },
    };
    Ok(AuthStatus {
        twitch,
        kick,
        youtube: YoutubeAuthStatus { browser, has_paste },
        chaturbate,
    })
}

#[tauri::command]
async fn twitch_login(
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<auth::twitch::TwitchIdentity, String> {
    let identity = auth::twitch::login(&state.http).await.map_err(err_string)?;
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
async fn youtube_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    auth::youtube::login_via_webview(app.clone())
        .await
        .map_err(err_string)?;
    clear_youtube_browser_pref(&state);
    if let Err(e) = auth::youtube::inject_into_main_webview(&app) {
        log::warn!("post-login cookie injection failed: {e:#}");
    }
    Ok(true)
}

#[tauri::command]
fn youtube_login_paste(
    text: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let cookies = auth::youtube::parse_pasted(&text).map_err(err_string)?;
    auth::youtube::save(&cookies).map_err(err_string)?;
    clear_youtube_browser_pref(&state);
    if let Err(e) = auth::youtube::inject_into_main_webview(&app) {
        log::warn!("post-login cookie injection failed: {e:#}");
    }
    Ok(true)
}

#[tauri::command]
fn youtube_logout(state: State<'_, AppState>) -> Result<(), String> {
    auth::youtube::clear().map_err(err_string)?;
    clear_youtube_browser_pref(&state);
    Ok(())
}

#[tauri::command]
async fn chaturbate_login(app: tauri::AppHandle) -> Result<bool, String> {
    auth::chaturbate::login_via_webview(app)
        .await
        .map_err(err_string)?;
    Ok(true)
}

#[tauri::command]
fn chaturbate_logout(
    embeds: State<'_, Arc<embed::EmbedManager>>,
) -> Result<(), String> {
    embeds.unmount_platform(Platform::Chaturbate);
    auth::chaturbate::clear().map_err(err_string)?;
    Ok(())
}

fn clear_youtube_browser_pref(state: &State<'_, AppState>) {
    let mut g = state.settings.write();
    if g.general.youtube_cookies_browser.is_some() {
        g.general.youtube_cookies_browser = None;
        let snapshot = g.clone();
        drop(g);
        if let Err(e) = settings::save(&snapshot) {
            log::warn!("clearing youtube_cookies_browser failed: {e:#}");
        }
    }
}

#[tauri::command]
fn youtube_detect_browsers() -> Vec<auth::youtube::DetectedBrowser> {
    auth::youtube::detect_browsers()
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
    // The React side sends a stream-level key (which for live YT may
    // include a :video_id suffix). Find the Channel by the channel-level
    // key, but keep the original stream key around to look up the
    // Livestream's video_id for per-stream YT URLs.
    let channel_key = channels::channel_key_of(&unique_key).to_string();
    let (channel, livestream) = {
        let guard = state.store.lock();
        let ch = guard
            .channels()
            .iter()
            .find(|c| c.unique_key() == channel_key)
            .cloned()
            .ok_or_else(|| format!("unknown channel {unique_key}"))?;
        let ls = guard
            .snapshot()
            .into_iter()
            .find(|l| l.unique_key == unique_key);
        (ch, ls)
    };
    let url = match channel.platform {
        Platform::Twitch => format!("https://www.twitch.tv/{}", channel.channel_id),
        Platform::Youtube => {
            // Prefer per-video URL when the livestream has a video_id —
            // matches Qt's per-stream open behaviour for multi-concurrent
            // channels. Falls back to the channel landing page otherwise.
            if let Some(vid) = livestream.and_then(|ls| ls.video_id) {
                format!("https://www.youtube.com/watch?v={vid}")
            } else if channel.channel_id.starts_with("UC") {
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

/// Linux-specific runtime workarounds, applied before Tauri boots.
///
/// 1. `WEBKIT_DISABLE_DMABUF_RENDERER=1` — WebKitGTK on NVIDIA + Wayland (KDE
///    Plasma) crashes with "Error 71 (Protocol error) dispatching to Wayland
///    display" when the DMABUF renderer is enabled. Disabling it falls back
///    to a software path that renders correctly.
/// 2. `GDK_BACKEND=x11` — Wayland's protocol does not expose absolute window
///    position to clients (`outer_position` always returns `(0, 0)`), so
///    `tauri-plugin-window-state` cannot persist or restore position on a
///    native Wayland session. Forcing GTK onto Xwayland restores standard
///    X11 semantics: real coordinates in/out, `set_position` honored, full
///    window-state persistence. Revisit once either the Wayland protocol
///    grows an opt-in way for clients to request/report absolute positions
///    (see xdg-shell discussions) or the ecosystem migrates away from
///    position-persistence UX.
///
/// Both use `set_if_unset` so a user can opt out via environment override
/// (e.g. `GDK_BACKEND=wayland` for a native-Wayland session).
///
/// No-op everywhere else.
#[cfg(target_os = "linux")]
fn apply_linux_webkit_workarounds() {
    fn set_if_unset(key: &str, val: &str) {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, val);
        }
    }
    set_if_unset("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    set_if_unset("GDK_BACKEND", "x11");
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_webkit_workarounds() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_linux_webkit_workarounds();
    let state = AppState::new().expect("failed to initialize app state");

    let http_for_chat = state.http.clone();
    let users_for_chat = Arc::clone(&state.users);
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(window_state::state_flags())
                .build(),
        )
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
            let chat_mgr = ChatManager::new(
                app.handle().clone(),
                http_for_chat.clone(),
                users_for_chat.clone(),
            );
            app.manage(chat_mgr);
            let player_mgr = Arc::new(PlayerManager::new(app.handle().clone()));
            app.manage(player_mgr);
            let embed_mgr = embed::EmbedManager::new();
            app.manage(embed_mgr);
            // No focus-tracking hide: `transient_for(main)` makes the WM
            // stack the embed above the main window, so when the user
            // switches to another app it naturally goes behind. Manual
            // hide/show on focus loss triggered KWin's window-close
            // animation on every alt-tab.
            // Seed the main webview's cookie jar with stored YouTube cookies
            // so any /live_chat iframe in the React tree is signed in.
            if let Err(e) = auth::youtube::inject_into_main_webview(app.handle()) {
                log::warn!("youtube cookie injection failed: {e:#}");
            }
            tray::build(&app.handle())?;
            window_state::register(app)?;
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
            stop_stream,
            list_playing,
            open_in_browser,
            open_url,
            list_socials,
            chat_connect,
            chat_disconnect,
            chat_send,
            chat_open_popout,
            embed_mount,
            embed_position,
            embed_unmount,
            embed_set_visible,
            list_emotes,
            replay_chat_history,
            get_settings,
            update_settings,
            get_user_metadata,
            set_user_metadata,
            get_user_profile,
            get_user_messages,
            list_blocked_users,
            auth_status,
            twitch_login,
            twitch_logout,
            kick_login,
            kick_logout,
            youtube_login,
            youtube_login_paste,
            youtube_logout,
            youtube_detect_browsers,
            chaturbate_login,
            chaturbate_logout,
            import_twitch_follows,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
