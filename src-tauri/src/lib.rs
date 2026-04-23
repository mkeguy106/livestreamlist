use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{Manager, State};

mod channels;
mod chat;
mod config;
mod platforms;
mod refresh;
mod streamlink;

use channels::{Channel, ChannelStore, Livestream, SharedStore};
use chat::ChatManager;
use platforms::{parse_channel_input, Platform};

struct AppState {
    store: SharedStore,
    http: reqwest::Client,
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
async fn refresh_all(state: State<'_, AppState>) -> Result<Vec<Livestream>, String> {
    let store = Arc::clone(&state.store);
    let client = state.http.clone();
    refresh::refresh_all(store, client).await.map_err(err_string)
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
            chat_connect,
            chat_disconnect,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
