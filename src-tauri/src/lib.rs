use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{Manager, State};

mod channels;
mod chat;
mod config;
mod notify;
mod platforms;
mod refresh;
mod streamlink;
mod tray;

use channels::{Channel, ChannelStore, Livestream, SharedStore};
use chat::ChatManager;
use notify::NotifyTracker;
use platforms::{parse_channel_input, Platform};

struct AppState {
    store: SharedStore,
    http: reqwest::Client,
    notifier: Arc<NotifyTracker>,
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
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            http,
            notifier: Arc::new(NotifyTracker::new()),
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
    notifier.detect_and_notify(&app, &channels, &snapshot);
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
            replay_chat_history,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
