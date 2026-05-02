use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{Emitter, Manager, State};

mod auth;
mod channels;
mod chat;
mod config;
mod embed;
mod login_popup;
mod notify;
mod platforms;
mod player;
mod refresh;
mod settings;
mod share_window;
mod spellcheck;
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
    twitch_anniversary_cache: platforms::twitch_anniversary::SharedCache,
    share_windows: share_window::SharedShareWindowState,
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
            twitch_anniversary_cache: Arc::new(platforms::twitch_anniversary::Cache::new()),
            share_windows: Arc::new(share_window::ShareWindowState::new()),
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

/// If the system clipboard contains a URL pointing at a supported
/// platform, return it so the Add Channel dialog can prefill on open.
/// Returns None for empty clipboards, non-URL strings (so a copied
/// bare handle like "shroud" doesn't auto-fill), oversized payloads,
/// and URLs that the existing parser doesn't recognise.
#[tauri::command]
fn clipboard_channel_url(app: tauri::AppHandle) -> Option<String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let raw = app.clipboard().read_text().ok()?;
    let trimmed = raw.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return None;
    }
    if trimmed.len() > 500 {
        return None;
    }
    parse_channel_input(trimmed).map(|_| trimmed.to_string())
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
    let yt_browser = state
        .settings
        .read()
        .general
        .youtube_cookies_browser
        .clone();
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
    // The React side sends the stream-level key. For YT multi-stream
    // that includes the :video_id suffix; we need the channel-level
    // key to find the Channel + the original key to find the specific
    // Livestream's video_id.
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
    let video_id = livestream.and_then(|ls| ls.video_id);
    player
        .launch(
            unique_key,
            channel.platform,
            &channel.channel_id,
            video_id.as_deref(),
            quality.as_deref().unwrap_or("best"),
            None,
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
fn chat_open_in_browser(
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

#[tauri::command]
async fn chat_detach(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    use tauri::WebviewUrl;

    let label = format!("chat-detach-{}", slugify(&unique_key));

    // Idempotent: focus existing window if already open.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    // URL fragment tells main.jsx to mount DetachedChatRoot.
    let path = format!(
        "index.html#chat-detach={}",
        urlencoding::encode(&unique_key),
    );
    let url = WebviewUrl::App(path.into());

    let mut builder = tauri::WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Chat — {unique_key}"))
        .inner_size(460.0, 700.0)
        .min_inner_size(320.0, 480.0)
        .decorations(false)
        .resizable(true)
        .visible(false)              // dark-first-paint discipline (PR #70 lesson)
        .background_color(tauri::webview::Color(0x09, 0x09, 0x0b, 0xff));

    // No transient_for(&main) — the detached chat is a peer top-level window,
    // not a subordinate dialog. Setting WM_TRANSIENT_FOR causes KWin (and most
    // X11 WMs) to stack/raise main alongside the popout whenever the popout
    // gets focus, which means clicking into the popout drags main forward too.
    // login_popup.rs intentionally keeps transient_for because those popups
    // *should* stay tied to main; chat detach is the opposite case.

    let window = builder.build().map_err(err_string)?;

    // Emit chat-detach:closed when the detached window is destroyed so the
    // main window can update its detachedKeys set.
    let app_for_close = app.clone();
    let key_for_close = unique_key.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = app_for_close.emit("chat-detach:closed", &key_for_close);
        }
    });

    window.show().map_err(err_string)?;
    Ok(())
}

#[tauri::command]
async fn chat_reattach(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    // Emit redock first so main has the channel back in tabKeys before the
    // window's :closed event fires (the :closed handler is idempotent and
    // tolerates either ordering).
    let _ = app.emit("chat-detach:redock", &unique_key);

    let label = format!("chat-detach-{}", slugify(&unique_key));
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
async fn chat_focus_detached(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    let label = format!("chat-detach-{}", slugify(&unique_key));
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    Ok(())
}

#[tauri::command]
async fn spellcheck_check(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
    text: String,
    language: String,
    channel_emotes: Vec<String>,
) -> Result<Vec<crate::spellcheck::MisspelledRange>, String> {
    Ok(state.check(&text, &language, &channel_emotes))
}

#[tauri::command]
async fn spellcheck_suggest(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
    word: String,
    language: String,
) -> Result<Vec<String>, String> {
    Ok(state.suggest(&word, &language))
}

#[tauri::command]
async fn spellcheck_add_word(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
    word: String,
) -> Result<bool, String> {
    state.add_to_personal(&word).map_err(err_string)
}

#[tauri::command]
async fn spellcheck_list_dicts(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
) -> Result<Vec<crate::spellcheck::dict::DictInfo>, String> {
    Ok(state.list_dicts())
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

#[cfg(test)]
mod chat_detach_tests {
    use super::*;

    #[test]
    fn slug_for_twitch_yields_valid_label() {
        let slug = slugify("twitch:shroud");
        let label = format!("chat-detach-{slug}");
        assert_eq!(label, "chat-detach-twitch-shroud");
        // Tauri labels must match ^[a-zA-Z0-9 _-]+$ — verify our slug does.
        assert!(label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' '));
    }

    #[test]
    fn slug_for_youtube_multi_stream_yields_valid_label() {
        let slug = slugify("youtube:UCnasa:isst1");
        let label = format!("chat-detach-{slug}");
        assert_eq!(label, "chat-detach-youtube-UCnasa-isst1");
    }

    #[test]
    fn slug_strips_non_alphanumeric() {
        let slug = slugify("kick:trainwrecks!");
        let label = format!("chat-detach-{slug}");
        assert_eq!(label, "chat-detach-kick-trainwrecks-");
    }
}

// Real handlers delegate to EmbedHost. EmbedHost::mount / set_bounds / set_visible
// are themselves cfg(not(test))-gated (they touch ChildEmbed::inner which only
// exists in real builds), so each handler exists in two cfg-gated variants.
#[cfg(not(test))]
#[tauri::command]
fn embed_mount(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<bool, String> {
    embeds
        .mount(
            &app,
            &state.store,
            &unique_key,
            embed::Rect::new(x, y, width, height),
        )
        .map_err(err_string)
}

#[cfg(test)]
#[tauri::command]
fn embed_mount(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
    _embeds: State<'_, Arc<embed::EmbedHost>>,
    _unique_key: String,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
) -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(test))]
#[tauri::command]
fn embed_bounds(
    app: tauri::AppHandle,
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    embeds
        .set_bounds(&app, &unique_key, embed::Rect::new(x, y, width, height))
        .map_err(err_string)
}

#[cfg(test)]
#[tauri::command]
fn embed_bounds(
    _app: tauri::AppHandle,
    _embeds: State<'_, Arc<embed::EmbedHost>>,
    _unique_key: String,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(test))]
#[tauri::command]
fn embed_set_visible(
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    visible: bool,
) -> Result<(), String> {
    embeds.set_visible(&unique_key, visible).map_err(err_string)
}

#[cfg(test)]
#[tauri::command]
fn embed_set_visible(
    _embeds: State<'_, Arc<embed::EmbedHost>>,
    _unique_key: String,
    _visible: bool,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(test))]
#[tauri::command]
fn embed_unmount(embeds: State<'_, Arc<embed::EmbedHost>>, unique_key: String) {
    embeds.unmount(&unique_key);
}

#[cfg(test)]
#[tauri::command]
fn embed_unmount(_embeds: State<'_, Arc<embed::EmbedHost>>, _unique_key: String) {
    // noop in test build
}

#[tauri::command]
fn login_popup_open(
    app: tauri::AppHandle,
    popup: State<'_, Arc<login_popup::LoginPopupManager>>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    popup.open(&app, x, y, width, height).map_err(err_string)
}

#[tauri::command]
fn login_popup_close(popup: State<'_, Arc<login_popup::LoginPopupManager>>) {
    popup.close();
}

#[tauri::command]
fn login_popup_resize(
    popup: State<'_, Arc<login_popup::LoginPopupManager>>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    popup.resize(width, height).map_err(err_string)
}

/// Broadcast a no-payload "auth state changed" event so every webview
/// (main + login popup) can re-pull `auth_status`. Logged but not fatal.
fn broadcast_auth_changed(app: &tauri::AppHandle) {
    if let Err(e) = app.emit("auth:changed", ()) {
        log::warn!("emit auth:changed: {e:#}");
    }
}

/// Best-effort YouTube user-info refresh — runs the keyring-cookie
/// fetch in the background, persists the resulting `@handle` to the
/// keyring on success, and emits `auth:changed` so the login chiclet
/// dropdown updates without the user re-opening it. Fire-and-forget;
/// every failure path logs and swallows.
fn spawn_youtube_user_info_refresh(app: &tauri::AppHandle, http: reqwest::Client) {
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        match auth::youtube::fetch_user_info(&http).await {
            Ok(Some(info)) => {
                if let Err(e) = auth::youtube::save_user_info(&info) {
                    log::warn!("save YouTube user info: {e:#}");
                    return;
                }
                log::info!("YouTube handle detected: @{}", info.handle);
                broadcast_auth_changed(&app_for_task);
            }
            Ok(None) => {
                log::debug!("YouTube user info not detectable (no cookies or no marker match)");
            }
            Err(e) => log::warn!("fetch YouTube user info: {e:#}"),
        }
    });
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
    // Per-stream IPC; strip the :video_id suffix to find the Channel.
    // For YT/CB the chat is embed-based and chat.connect is a no-op,
    // so the per-channel key is fine to pass through.
    let channel_key = channels::channel_key_of(&unique_key).to_string();
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == channel_key)
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
    twitch_web: Option<auth::twitch_web::TwitchWebIdentity>,
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
    /// Logged-in user's `@handle`, if we were able to scrape it from
    /// an authenticated /account fetch. None when using browser-cookie
    /// auth (no keyring cookies), when YouTube's DOM doesn't match any
    /// of our markers, or simply before the background fetch completes
    /// on first launch.
    handle: Option<String>,
}

#[derive(serde::Serialize)]
struct ChaturbateAuthStatus {
    signed_in: bool,
    last_verified_at: Option<String>, // RFC3339, None when not signed in
    /// Logged-in CB username, scraped from chaturbate.com at login
    /// time. None for stamps written before this field was added or
    /// when the scrape couldn't find the marker.
    username: Option<String>,
}

#[tauri::command]
async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, String> {
    let twitch = auth::twitch::status(&state.http)
        .await
        .map_err(err_string)?;
    let twitch_web = auth::twitch_web::status(&state.http)
        .await
        .map_err(err_string)?;
    let kick = auth::kick::status(&state.http).await.map_err(err_string)?;
    let browser = state
        .settings
        .read()
        .general
        .youtube_cookies_browser
        .clone();
    let has_paste = auth::youtube::cookies_file_present();
    let yt_handle = auth::youtube::load_user_info()
        .map_err(err_string)?
        .map(|i| i.handle);
    let chaturbate = match auth::chaturbate::load().map_err(err_string)? {
        Some(stamp) => ChaturbateAuthStatus {
            signed_in: true,
            last_verified_at: Some(stamp.last_verified_at.to_rfc3339()),
            username: stamp.username,
        },
        None => ChaturbateAuthStatus {
            signed_in: false,
            last_verified_at: None,
            username: None,
        },
    };
    Ok(AuthStatus {
        twitch,
        twitch_web,
        kick,
        youtube: YoutubeAuthStatus {
            browser,
            has_paste,
            handle: yt_handle,
        },
        chaturbate,
    })
}

#[tauri::command]
async fn twitch_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<auth::twitch::TwitchIdentity, String> {
    let identity = auth::twitch::login(&state.http).await.map_err(err_string)?;
    // Auth state changed — reconnect any live Twitch chat tasks so they
    // pick up the new credentials.
    chat.reconnect_platform(Platform::Twitch, &state.store);
    broadcast_auth_changed(&app);
    Ok(identity)
}

#[tauri::command]
fn twitch_logout(
    app: tauri::AppHandle,
    chat: State<'_, Arc<ChatManager>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    auth::twitch::logout().map_err(err_string)?;
    chat.reconnect_platform(Platform::Twitch, &state.store);
    broadcast_auth_changed(&app);
    Ok(())
}

#[tauri::command]
async fn twitch_web_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<auth::twitch_web::TwitchWebIdentity, String> {
    let identity = auth::twitch_web::login_with_match_check(app.clone(), state.http.clone())
        .await
        .map_err(|e| e.to_string())?;
    broadcast_auth_changed(&app);
    Ok(identity)
}

#[tauri::command]
fn twitch_web_clear(app: tauri::AppHandle) -> Result<(), String> {
    auth::twitch_web::clear().map_err(err_string)?;
    broadcast_auth_changed(&app);
    Ok(())
}

#[tauri::command]
async fn twitch_anniversary_check(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    unique_key: String,
) -> Result<platforms::twitch_anniversary::CheckResult, String> {
    use platforms::twitch_anniversary::{CheckResult, CookieStatus};

    // Setting check
    let enabled = state.settings.read().chat.show_sub_anniversary_banner;
    if !enabled {
        return Ok(CheckResult { info: None, cookie_status: CookieStatus::Ok });
    }

    // Resolve channel — may have :video_id suffix from YT multi-stream;
    // canonical key for store lookup is via channel_key_of.
    let channel_key = channels::channel_key_of(&unique_key).to_string();
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == channel_key)
        .cloned();

    let Some(channel) = channel else {
        return Ok(CheckResult { info: None, cookie_status: CookieStatus::Ok });
    };

    if channel.platform != Platform::Twitch {
        return Ok(CheckResult { info: None, cookie_status: CookieStatus::Ok });
    }

    let mut result = platforms::twitch_anniversary::check(
        &state.http,
        &channel.channel_id,
        &state.twitch_anniversary_cache,
        &app,
    )
    .await;

    // Dismissal check — keyed by unique_key (NOT channel_key) since the
    // dismissal map's keys come from frontend invokes which use the
    // unique_key directly, including any :video_id suffix.
    if let Some(ref i) = result.info {
        let settings = state.settings.read();
        if let Some(dismissed_renews) = settings.chat.dismissed_sub_anniversaries.get(&unique_key) {
            if dismissed_renews == &i.renews_at {
                result.info = None;
            }
        }
    }

    Ok(result)
}

#[tauri::command]
fn twitch_anniversary_dismiss(
    state: State<'_, AppState>,
    unique_key: String,
    renews_at: String,
) -> Result<(), String> {
    {
        let mut s = state.settings.write();
        s.chat.dismissed_sub_anniversaries.insert(unique_key, renews_at);
    }
    let snapshot = state.settings.read().clone();
    settings::save(&snapshot).map_err(err_string)?;
    Ok(())
}

#[tauri::command]
fn twitch_share_resub_open(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    unique_key: String,
) -> Result<(), String> {
    let channel_key = channels::channel_key_of(&unique_key).to_string();
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == channel_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;
    if channel.platform != Platform::Twitch {
        return Err(format!(
            "share popout only supported for Twitch; got {:?}",
            channel.platform
        ));
    }
    // Pull the captured cookie (if any) so the popout's profile dir
    // gets it via init-script injection on first load. PR 1's manual
    // login flow naturally seeds the profile dir; PR 5's browser
    // auto-scrape goes straight to the keyring without touching the
    // profile dir, leaving the popout anonymous without this bridge.
    let cookie = auth::twitch_web::stored_token().ok().flatten();
    share_window::open(
        &app,
        &channel.channel_id,
        &channel.display_name,
        cookie.as_deref(),
        &state.share_windows,
    )
    .map_err(err_string)
}

#[tauri::command]
fn twitch_share_window_close(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    unique_key: String,
) -> Result<(), String> {
    let channel_key = channels::channel_key_of(&unique_key).to_string();
    let channel_login = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == channel_key)
        .map(|c| c.channel_id.clone());
    if let Some(login) = channel_login {
        share_window::close(&app, &login, &state.share_windows);
    }
    Ok(())
}

#[tauri::command]
async fn kick_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<auth::kick::KickIdentity, String> {
    let identity = auth::kick::login(&state.http).await.map_err(err_string)?;
    chat.reconnect_platform(Platform::Kick, &state.store);
    broadcast_auth_changed(&app);
    Ok(identity)
}

#[tauri::command]
fn kick_logout(
    app: tauri::AppHandle,
    chat: State<'_, Arc<ChatManager>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    auth::kick::logout().map_err(err_string)?;
    chat.reconnect_platform(Platform::Kick, &state.store);
    broadcast_auth_changed(&app);
    Ok(())
}

#[tauri::command]
async fn youtube_login(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    auth::youtube::login_via_webview(app.clone())
        .await
        .map_err(err_string)?;
    clear_youtube_browser_pref(&state);
    if let Err(e) = auth::youtube::inject_into_main_webview(&app) {
        log::warn!("post-login cookie injection failed: {e:#}");
    }
    broadcast_auth_changed(&app);
    spawn_youtube_user_info_refresh(&app, state.http.clone());
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
    broadcast_auth_changed(&app);
    spawn_youtube_user_info_refresh(&app, state.http.clone());
    Ok(true)
}

#[tauri::command]
fn youtube_logout(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    auth::youtube::clear().map_err(err_string)?;
    clear_youtube_browser_pref(&state);
    broadcast_auth_changed(&app);
    Ok(())
}

#[tauri::command]
async fn chaturbate_login(app: tauri::AppHandle) -> Result<bool, String> {
    auth::chaturbate::login_via_webview(app.clone())
        .await
        .map_err(err_string)?;
    broadcast_auth_changed(&app);
    Ok(true)
}

#[tauri::command]
fn chaturbate_logout(
    app: tauri::AppHandle,
    embeds: State<'_, Arc<embed::EmbedHost>>,
) -> Result<(), String> {
    embeds.unmount_platform(Platform::Chaturbate);
    auth::chaturbate::clear().map_err(err_string)?;
    broadcast_auth_changed(&app);
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
    let mut msgs =
        chat::log_store::read_recent(channel.platform, &channel.channel_id, limit.min(1000))
            .map_err(err_string)?;
    // Transient marker — applied after deserialize, never persisted back.
    // Lets the frontend dim log-replayed messages alongside robotty
    // backfill so all pre-live history shares one visual treatment.
    for m in &mut msgs {
        m.is_log_replay = true;
    }
    Ok(msgs)
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
        .plugin(tauri_plugin_clipboard_manager::init())
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
            let embed_mgr = embed::EmbedHost::new();
            app.manage(embed_mgr.clone());
            #[cfg(target_os = "linux")]
            {
                let main = app
                    .get_webview_window("main")
                    .expect("main window must exist by setup time");
                let host_for_setup = embed_mgr.clone();
                let main_for_closure = main.clone();
                main.run_on_main_thread(move || {
                    if let Ok(gtk_window) = main_for_closure.gtk_window() {
                        match embed::linux::install_overlay(&gtk_window) {
                            Ok(fixed) => host_for_setup.install_fixed(fixed),
                            Err(e) => log::error!("install_overlay failed: {e:#}"),
                        }
                    }
                })?;
            }
            let login_popup_mgr = login_popup::LoginPopupManager::new();
            app.manage(login_popup_mgr);
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
            // Refresh the cached YouTube @handle in the background. Cheap
            // single GET; populates `auth_status.youtube.handle` so the
            // login chiclet shows it without waiting for a re-login.
            if auth::youtube::load().ok().flatten().is_some() {
                spawn_youtube_user_info_refresh(&app.handle(), http_for_chat.clone());
            }
            // Twitch web cookie auto-scrape (Qt parity, mirrors
            // gui/app.py:306-312::extract_twitch_auth_token). If OAuth
            // is logged in but no web cookie cached, try to scrape it
            // from the user's browser. Async because validate() does a
            // GQL ping. Silent on failure — lazy WebView fallback in
            // auth::twitch_web::login_via_webview handles unsupported
            // browsers / Flatpak isolation.
            if auth::twitch::stored_identity().is_some()
                && auth::twitch_web::stored_token().ok().flatten().is_none()
            {
                let app_handle = app.handle().clone();
                let http_for_scrape = http_for_chat.clone();
                tauri::async_runtime::spawn(async move {
                    let Some(token) = auth::twitch_web::extract_from_browser() else {
                        log::debug!(
                            "twitch-web auto-scrape: no auth-token cookie found in any browser"
                        );
                        return;
                    };
                    match auth::twitch_web::validate(&http_for_scrape, &token).await {
                        Ok(identity) => {
                            if let Err(e) = auth::twitch_web::save_pair(&token, &identity) {
                                log::warn!("twitch-web auto-scrape: save_pair failed: {e:#}");
                                return;
                            }
                            log::info!(
                                "twitch-web auto-scrape: captured cookie for @{}",
                                identity.login
                            );
                            use tauri::Emitter;
                            let _ = app_handle
                                .emit("twitch:web_status_changed", Some(identity));
                            broadcast_auth_changed(&app_handle);
                        }
                        Err(e) => {
                            log::debug!(
                                "twitch-web auto-scrape: validate failed (cookie expired?): {e:#}"
                            );
                        }
                    }
                });
            }
            // Set the resource dir env var so spellcheck::dict::bundled_en_us_path
            // can resolve it without needing AppHandle.
            if let Ok(res_dir) = app.path().resource_dir() {
                std::env::set_var("LIVESTREAMLIST_RESOURCE_DIR", &res_dir);
            }
            let personal_dict_path = crate::config::config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("personal_dict.json");
            let spellchecker = std::sync::Arc::new(
                crate::spellcheck::SpellChecker::new(personal_dict_path),
            );
            app.manage(spellchecker);
            tray::build(&app.handle())?;
            window_state::register(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_livestreams,
            list_channels,
            add_channel_from_input,
            clipboard_channel_url,
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
            chat_open_in_browser,
            chat_detach,
            chat_reattach,
            chat_focus_detached,
            embed_mount,
            embed_bounds,
            embed_unmount,
            embed_set_visible,
            login_popup_open,
            login_popup_close,
            login_popup_resize,
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
            twitch_web_login,
            twitch_web_clear,
            twitch_anniversary_check,
            twitch_anniversary_dismiss,
            twitch_share_resub_open,
            twitch_share_window_close,
            kick_login,
            kick_logout,
            youtube_login,
            youtube_login_paste,
            youtube_logout,
            youtube_detect_browsers,
            chaturbate_login,
            chaturbate_logout,
            import_twitch_follows,
            spellcheck_check,
            spellcheck_suggest,
            spellcheck_add_word,
            spellcheck_list_dicts,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
