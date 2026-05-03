//! Smoke-test harness module. Provides `build_smoke_app()` which constructs
//! a `tauri::App<MockRuntime>` with the production `generate_handler!` block
//! and the runtime-agnostic subset of `AppState`, against an isolated config
//! dir. Used by the `smoke` binary in `src/bin/smoke.rs` and tested in this
//! module.

/// Names of `#[tauri::command]` functions that produce real-world side
/// effects beyond the temp config dir (network connections, subprocess
/// spawns, real WebKit/GTK windows). The binary's dispatch layer
/// short-circuits these unless `--allow-side-effects` is passed.
///
/// Also includes commands that require ChatManager or PlayerManager state,
/// neither of which is constructed in the smoke harness — calling them
/// under `--allow-side-effects` would panic with `kind:"panic"` instead of
/// returning a clean `kind:"blocked"` envelope.
pub const DENYLIST: &[&str] = &[
    "chat_connect",
    "chat_disconnect",
    "chat_send",
    "launch_stream",
    "stop_stream",
    "list_playing",
    // Requires ChatManager or PlayerManager state not managed in smoke harness
    "kick_logout",
    "twitch_logout",
    "list_emotes",
    "embed_mount",
    "embed_bounds",
    "embed_unmount",
    "embed_set_visible",
    "twitch_login",
    "twitch_web_login",
    "kick_login",
    "youtube_login",
    "youtube_login_paste",
    "chaturbate_login",
    "login_popup_open",
    "login_popup_close",
    "login_popup_resize",
    "chat_open_in_browser",
    "chat_detach",
    "chat_reattach",
    "chat_focus_detached",
    "twitch_share_resub_open",
    "twitch_share_window_close",
    "open_in_browser",
    "open_url",
];

// `build_smoke_app` and its tests require `tauri/test` (the MockRuntime),
// which is only compiled when the `smoke` feature is enabled.
#[cfg(feature = "smoke")]
pub use smoke_impl::{build_real_config_app, build_smoke_app};

#[cfg(feature = "smoke")]
mod smoke_impl {
    use std::path::Path;
    use std::sync::Arc;

    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri::{App, Manager};

    pub fn build_smoke_app(temp_root: &Path) -> anyhow::Result<App<MockRuntime>> {
        // Set XDG paths to subdirs of temp_root BEFORE constructing AppState.
        // The `dirs` crate (used by config::config_dir) reads these env vars
        // on each call, so the override is effective immediately.
        let config_root = temp_root.join("config");
        let data_root = temp_root.join("data");
        std::fs::create_dir_all(&config_root)?;
        std::fs::create_dir_all(&data_root)?;
        std::env::set_var("XDG_CONFIG_HOME", &config_root);
        std::env::set_var("XDG_DATA_HOME", &data_root);

        // Point spellcheck::dict::bundled_en_us_path at the source tree.
        // In production, lib.rs::run sets this from app.path().resource_dir();
        // for the smoke binary the equivalent is the dictionaries/ directory
        // next to Cargo.toml at compile time.
        let resource_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::env::set_var("LIVESTREAMLIST_RESOURCE_DIR", resource_dir);

        // Construct the mock app with the same handler list as production.
        let mut app = mock_builder()
            .invoke_handler(crate::register_handlers!())
            .build(mock_context(noop_assets()))?;

        // CANNOT call crate::manage_all_state — its signature is concrete
        // tauri::App (= App<Wry>) because ChatManager::new and
        // PlayerManager::new take concrete AppHandle. Inline the
        // runtime-agnostic subset here. Skipped: ChatManager + PlayerManager
        // (their commands are in DENYLIST; calling them returns a clean
        // 'missing state' error which is acceptable signal for agents).
        let state = crate::AppState::new()?;
        app.manage(state);

        let embed_mgr = crate::embed::EmbedHost::new();
        app.manage(embed_mgr);

        let login_popup_mgr = crate::login_popup::LoginPopupManager::new();
        app.manage(login_popup_mgr);

        let personal_dict_path = crate::config::config_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("personal_dict.json");
        let spellchecker = Arc::new(crate::spellcheck::SpellChecker::new(personal_dict_path));
        app.manage(spellchecker);

        Ok(app)
    }

    /// Build the smoke app against the user's real ~/.config/livestreamlist/
    /// (and ~/.local/share/...). For agent debugging cases that need to
    /// inspect actual state. Don't write through this — the safety story
    /// for the smoke binary is "isolated by default."
    pub fn build_real_config_app() -> anyhow::Result<App<MockRuntime>> {
        let resource_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::env::set_var("LIVESTREAMLIST_RESOURCE_DIR", resource_dir);

        let mut app = mock_builder()
            .invoke_handler(crate::register_handlers!())
            .build(mock_context(noop_assets()))?;

        let state = crate::AppState::new()?;
        app.manage(state);

        let embed_mgr = crate::embed::EmbedHost::new();
        app.manage(embed_mgr);

        let login_popup_mgr = crate::login_popup::LoginPopupManager::new();
        app.manage(login_popup_mgr);

        let personal_dict_path = crate::config::config_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("personal_dict.json");
        let spellchecker = Arc::new(crate::spellcheck::SpellChecker::new(personal_dict_path));
        app.manage(spellchecker);

        Ok(app)
    }
}

#[cfg(all(test, feature = "smoke"))]
mod tests {
    use super::build_smoke_app;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{get_ipc_response, INVOKE_KEY};
    use tauri::webview::InvokeRequest;
    use tauri::WebviewWindowBuilder;

    fn make_request(cmd: &str, args: serde_json::Value) -> InvokeRequest {
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        }
    }

    #[test]
    fn build_smoke_app_lists_empty_channels_in_isolated_config() {
        let temp = tempfile::tempdir().unwrap();
        let app = build_smoke_app(temp.path()).expect("build_smoke_app");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview build");
        let response = get_ipc_response(
            &webview,
            make_request("list_channels", serde_json::json!({})),
        )
        .expect("list_channels should succeed");
        let value: Vec<crate::channels::Channel> =
            response.deserialize().expect("deserialize");
        assert!(value.is_empty(), "isolated config should have no channels");
    }

    #[test]
    fn build_smoke_app_marshalling_error_for_bad_args() {
        let temp = tempfile::tempdir().unwrap();
        let app = build_smoke_app(temp.path()).expect("build_smoke_app");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview build");
        let err = get_ipc_response(
            &webview,
            make_request(
                "add_channel_from_input",
                serde_json::json!({"wrong_field": "x"}),
            ),
        )
        .expect_err("missing 'input' field should fail");
        let msg = err.as_str().unwrap_or("");
        assert!(
            msg.starts_with("invalid args "),
            "Tauri arg-deserialize error has stable prefix; got: {msg}"
        );
    }
}
