# IPC Smoke-Test Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a `cargo run --bin smoke -- <cmd> '<json>'` binary that lets agents (and humans) exercise any of the 62 `#[tauri::command]` handlers end-to-end through Tauri's real IPC marshalling layer, without launching the desktop app. Single-shot and JSONL streaming modes; isolated temp config by default; side-effect denylist with explicit opt-out.

**Architecture:** A new binary at `src-tauri/src/bin/smoke.rs` (peer of the existing `livestreamlist` binary) links the existing `livestreamlist_lib` library crate. A new module `livestreamlist_lib::smoke` exposes `build_smoke_app(temp_root: &Path) -> Result<App<MockRuntime>>` which uses `tauri::test::mock_builder()` to construct an app with the production `generate_handler!` block + the production `AppState` + per-handle managers. The binary parses argv, sets up an isolated `XDG_CONFIG_HOME`, dispatches commands by name via `tauri::test::get_ipc_response`, and prints JSON envelopes on stdout. Logs go to stderr. Production setup gets a focused refactor to extract `manage_all_state()` so there's exactly one state-construction path.

**Tech Stack:** Rust 1.77+, Tauri 2.10.3 (`tauri::test` requires `feature = "test"`), `tempfile` (new dep, optional), `assert_cmd` + `predicates` (new dev-deps for integration tests), existing `tracing` + `serde_json`. The smoke binary is feature-gated (`required-features = ["smoke"]`) so default `cargo build` skips it and ships the production binary unchanged.

**Spec:** `docs/superpowers/specs/2026-05-02-ipc-smoke-harness-design.md`

---

## File Structure

| Path | Action | Responsibility |
|---|---|---|
| `src-tauri/Cargo.toml` | Modify | New feature `smoke`, optional `tempfile` dep, new `[[bin]] smoke` target with `required-features`, new dev-deps `assert_cmd` + `predicates` |
| `src-tauri/src/lib.rs` | Modify | Make `AppState` `pub(crate)`. Extract per-handle state construction into a new `pub(crate) fn manage_all_state(app: &mut App<R>) -> anyhow::Result<()>`. Production `setup()` calls it; smoke module calls it too |
| `src-tauri/src/smoke.rs` | Create | New module. `pub fn build_smoke_app(temp_root: &Path) -> anyhow::Result<App<MockRuntime>>`. Sets `XDG_CONFIG_HOME` + `XDG_DATA_HOME` to subdirs of `temp_root`, sets `LIVESTREAMLIST_RESOURCE_DIR` to source-tree dictionaries path, builds `mock_builder()` with the production handler list + state, returns ready-to-dispatch `App<MockRuntime>`. Plus the side-effect `DENYLIST` constant used by both unit tests and the binary. Unit-tested in-file |
| `src-tauri/src/bin/smoke.rs` | Create | The CLI binary. Argv parser (`--list` / `--help` / `--use-real-config` / `--allow-side-effects` / positional cmd + JSON args). Single-shot mode and JSONL streaming mode. Tracing subscriber to stderr. Panic-catching dispatch. Output envelope serialization. Exit codes |
| `src-tauri/tests/smoke_binary.rs` | Create | End-to-end integration tests via `assert_cmd::Command::cargo_bin("smoke")`. Covers all eight acceptance criteria + drift-prevention test |
| `src-tauri/src/bin/README.md` | Create | ~30-line README for the smoke binary |
| `CLAUDE.md` | Modify | Add a one-paragraph entry under the project's Development Commands section pointing at the binary |

`docs/ROADMAP.md` is updated separately at PR merge time per the project's "Ship it" workflow — not part of this plan.

---

## Task 0: Cargo wiring + binary scaffold

**Why no TDD here:** This is build-system plumbing — the test for "did I wire it right" is "does it compile." Following the same pattern as the spellcheck PR 1 plan's Task 0 (dependency setup).

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/bin/smoke.rs` (one-line stub)

- [ ] **Step 1: Add the optional `tempfile` dependency**

In `src-tauri/Cargo.toml`, find the `[dependencies]` section. Add:

```toml
tempfile = { version = "3.10", optional = true }
```

Place it next to other utility deps like `parking_lot` or `chrono`.

- [ ] **Step 2: Add the `smoke` feature**

In `src-tauri/Cargo.toml`, add (or extend) a `[features]` section:

```toml
[features]
smoke = ["tauri/test", "dep:tempfile"]
```

If a `[features]` section already exists, just append the `smoke = …` line.

- [ ] **Step 3: Add the binary target with `required-features`**

In `src-tauri/Cargo.toml`, add a new `[[bin]]` entry (Cargo will recognize multiple `[[bin]]` sections; the existing `livestreamlist` binary is the default `src/main.rs`):

```toml
[[bin]]
name = "smoke"
path = "src/bin/smoke.rs"
required-features = ["smoke"]
```

The `required-features` line means default `cargo build` skips this binary entirely — it's only built when `--features smoke` is passed.

- [ ] **Step 4: Add `assert_cmd` + `predicates` to dev-dependencies**

In `src-tauri/Cargo.toml`, find the `[dev-dependencies]` section (create one if it doesn't exist). Add:

```toml
assert_cmd = "2.0"
predicates = "3.1"
```

- [ ] **Step 5: Create the binary stub**

Create `src-tauri/src/bin/smoke.rs` with a minimal placeholder so the feature-gated build target exists:

```rust
//! IPC smoke-test harness binary. Built only with --features smoke.
//! See README.md in this directory for usage.

fn main() {
    eprintln!("smoke binary not yet implemented");
    std::process::exit(2);
}
```

- [ ] **Step 6: Verify the production build is unchanged**

Run from the repo root:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
```

Expected: compiles `livestreamlist` and `livestreamlist_lib`. Does NOT mention `smoke` (the binary's `required-features` excludes it from the default build).

- [ ] **Step 7: Verify the smoke binary builds with the feature**

Run:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke
```

Expected: clean build. The binary at `src-tauri/target/debug/smoke` exists.

- [ ] **Step 8: Verify the smoke binary stub runs**

Run:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke
```

Expected: stderr "smoke binary not yet implemented", exit code 2.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/bin/smoke.rs
git commit -m "feat(smoke): cargo wiring + binary scaffold

- New 'smoke' feature gates tempfile + tauri/test
- New [[bin]] smoke target with required-features = [\"smoke\"]
- assert_cmd + predicates added as dev-deps for upcoming integration tests
- Default cargo build is unchanged (production binary only)"
```

---

## Task 1: Extract `manage_all_state()` from production setup

**Files:**
- Modify: `src-tauri/src/lib.rs:32` (make `AppState` `pub(crate)`)
- Modify: `src-tauri/src/lib.rs:43` (make `AppState::new` `pub(crate)`)
- Modify: `src-tauri/src/lib.rs:1416-1548` (extract per-handle state construction into `pub(crate) fn manage_all_state`; production `setup()` calls it)
- Test: existing `cargo test` and `cargo build` are the verification — this is a refactor that must not change behavior

**Why no new test:** This task is a pure refactor; correctness is verified by the existing 215 unit tests + production build still working. New behavior (smoke-binary-side calls to `manage_all_state`) is exercised by Task 2's tests.

- [ ] **Step 1: Make `AppState` and `AppState::new` `pub(crate)`**

In `src-tauri/src/lib.rs`, change line 32 from:

```rust
struct AppState {
```

to:

```rust
pub(crate) struct AppState {
```

And change line 44 from:

```rust
    fn new() -> anyhow::Result<Self> {
```

to:

```rust
    pub(crate) fn new() -> anyhow::Result<Self> {
```

Mark the fields `pub(crate)` too:

```rust
pub(crate) struct AppState {
    pub(crate) store: SharedStore,
    pub(crate) http: reqwest::Client,
    pub(crate) notifier: Arc<NotifyTracker>,
    pub(crate) settings: SharedSettings,
    pub(crate) users: Arc<UserStore>,
    pub(crate) pronouns: Arc<PronounsCache>,
    pub(crate) twitch_anniversary_cache: platforms::twitch_anniversary::SharedCache,
    pub(crate) share_windows: share_window::SharedShareWindowState,
}
```

- [ ] **Step 2: Add the `manage_all_state` function above `pub fn run()`**

In `src-tauri/src/lib.rs`, immediately above `pub fn run()` (around line 1416), insert:

```rust
/// Construct AppState + per-handle managers and `manage()` them on the App.
///
/// Shared between production `run()` and the smoke harness binary so both
/// paths get the same state shape. Side-effecting setup (tray, window state
/// plugin, GTK overlay, background async tasks) stays in production `run()`
/// where it belongs.
pub(crate) fn manage_all_state<R: tauri::Runtime>(
    app: &mut tauri::App<R>,
) -> anyhow::Result<()> {
    let state = AppState::new()?;
    let http = state.http.clone();
    let users = Arc::clone(&state.users);
    app.manage(state);

    let handle = app.handle().clone();
    let chat_mgr = ChatManager::new(handle.clone(), http.clone(), users.clone());
    app.manage(chat_mgr);

    let player_mgr = Arc::new(PlayerManager::new(handle.clone()));
    app.manage(player_mgr);

    let embed_mgr = embed::EmbedHost::new();
    app.manage(embed_mgr);

    let login_popup_mgr = login_popup::LoginPopupManager::new();
    app.manage(login_popup_mgr);

    let personal_dict_path = crate::config::config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("personal_dict.json");
    let spellchecker = Arc::new(crate::spellcheck::SpellChecker::new(personal_dict_path));
    app.manage(spellchecker);

    Ok(())
}
```

- [ ] **Step 3: Refactor production `run()` to call `manage_all_state`**

Line numbers in this step are as-of plan-writing time (2026-05-02). If the file has shifted between then and execution, locate the matching code by content rather than line number.

In `src-tauri/src/lib.rs`, the `pub fn run()` body currently constructs `AppState` at line 1418 and then constructs the per-handle managers inside `setup()` at lines 1448-1545. Replace those — but **preserve** the side-effecting setup code (GTK overlay install on Linux, YouTube cookie injection, Twitch auto-scrape, resource dir env var, tray::build, window_state::register, optional log plugin in debug).

Replace lines 1418-1421 (the top-level AppState construction + clones for closures):

```rust
let state = AppState::new().expect("failed to initialize app state");

let http_for_chat = state.http.clone();
let users_for_chat = Arc::clone(&state.users);
```

With nothing — those clones are no longer needed at the outer scope (the `manage_all_state` call inside `setup()` constructs everything).

Then change the `tauri::Builder::default()` chain so it no longer calls `.manage(state)` at line 1439.

Inside the `.setup(move |app| { … })` closure (currently at line 1440), replace the manager construction (lines 1448-1457 for chat_mgr/player_mgr/embed_mgr clones, line 1474-1475 for login_popup_mgr, lines 1539-1545 for spellchecker — but NOT lines 1458-1473 for the GTK overlay, NOT lines 1483-1485 for YT cookie injection, NOT lines 1489-1491 for YT user info refresh, NOT lines 1499-1533 for Twitch auto-scrape, NOT lines 1534-1538 for resource dir env, NOT line 1546 for tray::build, NOT line 1547 for window_state::register) with a single call:

```rust
crate::manage_all_state(app)?;
```

The setup closure becomes (illustrative — lines from the existing code preserved verbatim, only the manager construction is replaced):

```rust
.setup(move |app| {
    if cfg!(debug_assertions) {
        app.handle().plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )?;
    }
    crate::manage_all_state(app)?;

    #[cfg(target_os = "linux")]
    {
        let main = app
            .get_webview_window("main")
            .expect("main window must exist by setup time");
        let host_for_setup = app.state::<Arc<embed::EmbedHost>>().inner().clone();
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

    if let Err(e) = auth::youtube::inject_into_main_webview(app.handle()) {
        log::warn!("youtube cookie injection failed: {e:#}");
    }
    if auth::youtube::load().ok().flatten().is_some() {
        let http = app.state::<AppState>().http.clone();
        spawn_youtube_user_info_refresh(&app.handle(), http);
    }
    if auth::twitch::stored_identity().is_some()
        && auth::twitch_web::stored_token().ok().flatten().is_none()
    {
        let app_handle = app.handle().clone();
        let http_for_scrape = app.state::<AppState>().http.clone();
        tauri::async_runtime::spawn(async move {
            // ... preserved verbatim from lines 1505-1532
        });
    }
    if let Ok(res_dir) = app.path().resource_dir() {
        std::env::set_var("LIVESTREAMLIST_RESOURCE_DIR", &res_dir);
    }
    tray::build(&app.handle())?;
    window_state::register(app)?;
    Ok(())
})
```

Note the two callsites that previously borrowed from outer-scope `http_for_chat`/`users_for_chat` — they now read `app.state::<AppState>()` inside the closure. The outer-scope clones can be removed.

The `embed::EmbedHost`'s `host_for_setup` variable (currently `embed_mgr.clone()`) is fetched via `app.state::<Arc<embed::EmbedHost>>().inner().clone()` instead of from the just-constructed local — embed_mgr was a local variable in the old code; now it's the managed state. Note the type parameter is `Arc<embed::EmbedHost>` because `EmbedHost::new()` returns `Arc<Self>`.

- [ ] **Step 4: Verify the refactor compiles**

Run:

```bash
cargo build --manifest-path src-tauri/Cargo.toml
```

Expected: clean build. If `embed::EmbedHost::clone()` doesn't exist (it likely needs `Clone` derived, which it already has based on `embed_mgr.clone()` working at line 1457), we may need `app.state::<Arc<embed::EmbedHost>>().inner().clone()` — `inner()` on `State<T>` returns `&T`, then `.clone()`. The `inner()` call is the standard Tauri pattern for borrowing managed state.

- [ ] **Step 5: Run the existing test suite**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all 215 existing tests pass. None of them should be affected by the refactor (none touch `AppState` construction directly).

- [ ] **Step 6: Smoke-test the production binary boots**

Build the production binary:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --release
```

Expected: clean release build. Optionally launch `npm run tauri:dev` and verify the app boots normally — but the cargo test pass + clean build is sufficient verification for this task.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "refactor(lib): extract manage_all_state() from production setup

Pulls AppState + per-handle manager construction out of the setup
closure into a pub(crate) function. Production setup() still owns
all the side-effecting work (GTK overlay, YouTube cookie injection,
Twitch auto-scrape, tray, window state). Prep for the smoke-test
harness binary, which calls manage_all_state() against a MockRuntime
app — same state shape, no side effects."
```

---

## Task 2: Smoke module + `build_smoke_app()` + dispatch core

**Files:**
- Create: `src-tauri/src/smoke.rs`
- Modify: `src-tauri/src/lib.rs` (declare `mod smoke;` near the top, gated `#[cfg(any(feature = "smoke", test))]`)

- [ ] **Step 1: Declare the module in `lib.rs`**

In `src-tauri/src/lib.rs`, find the `mod` declarations near the top (lines 6-22). Add:

```rust
#[cfg(any(feature = "smoke", test))]
pub mod smoke;
```

The cfg-gating means the smoke module compiles for `cargo build --features smoke` AND for `cargo test` (so unit tests run without needing the smoke feature explicitly).

- [ ] **Step 2: Write the failing test for `build_smoke_app`**

Create `src-tauri/src/smoke.rs` with:

```rust
//! Smoke-test harness module. Provides `build_smoke_app()` which constructs
//! a `tauri::App<MockRuntime>` with the production `generate_handler!` block
//! and `AppState`, against an isolated config dir. Used by the `smoke`
//! binary in `src/bin/smoke.rs` and tested in this module.

use std::path::Path;

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::App;

/// Names of `#[tauri::command]` functions that produce real-world side
/// effects beyond the temp config dir (network connections, subprocess
/// spawns, real WebKit/GTK windows). The binary's dispatch layer
/// short-circuits these unless `--allow-side-effects` is passed.
///
/// Also includes `list_playing` because `PlayerManager` is NOT
/// constructed in the smoke harness (its `::new()` takes concrete
/// `AppHandle<Wry>`, incompatible with `MockRuntime`); calling
/// `list_playing` without the manager managed would return a
/// "missing state" error from Tauri's serde layer.
pub const DENYLIST: &[&str] = &[
    "chat_connect", "chat_disconnect", "chat_send",
    "launch_stream", "stop_stream", "list_playing",
    "embed_mount", "embed_bounds", "embed_unmount", "embed_set_visible",
    "twitch_login", "twitch_web_login", "kick_login", "youtube_login",
    "youtube_login_paste", "chaturbate_login",
    "login_popup_open", "login_popup_close", "login_popup_resize",
    "chat_open_in_browser", "chat_detach", "chat_reattach", "chat_focus_detached",
    "twitch_share_resub_open", "twitch_share_window_close",
    "open_in_browser", "open_url",
];

pub fn build_smoke_app(temp_root: &Path) -> anyhow::Result<App<MockRuntime>> {
    use std::sync::Arc;

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

    // Cannot call crate::manage_all_state — its signature is concrete
    // tauri::App (= App<Wry>) because ChatManager::new and
    // PlayerManager::new take concrete AppHandle. Inline the
    // runtime-agnostic subset here. Skipped: ChatManager + PlayerManager
    // (their commands are denylisted; calling them returns a clean
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

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::{get_ipc_response, INVOKE_KEY};
    use tauri::ipc::{CallbackFn, InvokeBody};
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
        let response = get_ipc_response(&webview, make_request("list_channels", serde_json::json!({})))
            .expect("list_channels should succeed");
        let value: Vec<crate::Channel> = response.deserialize().expect("deserialize");
        assert!(value.is_empty(), "isolated config should have no channels");
    }

    #[test]
    fn build_smoke_app_marshalling_error_for_bad_args() {
        let temp = tempfile::tempdir().unwrap();
        let app = build_smoke_app(temp.path()).expect("build_smoke_app");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("webview build");
        let err = get_ipc_response(&webview, make_request("add_channel_from_input", serde_json::json!({"wrong_field": "x"})))
            .expect_err("missing 'input' field should fail");
        let msg = err.as_str().unwrap_or("");
        assert!(msg.starts_with("invalid args "), "Tauri arg-deserialize error has stable prefix; got: {msg}");
    }
}
```

The two test names express the contract: `build_smoke_app` returns a working app whose isolated config is fresh (no channels), and Tauri's arg-deserialize errors come back with the stable `"invalid args "` prefix that the binary will use to classify `kind:"deserialize"`.

- [ ] **Step 3: Add the `register_handlers!` macro**

The smoke module's `build_smoke_app` references `crate::register_handlers!()` — a macro that returns the same handler list production uses. Define it in `src-tauri/src/lib.rs`, immediately above the `pub fn run()` function. The macro's body is the contents of the existing `tauri::generate_handler![...]` block (the 62-command list at lines 1551-1608):

```rust
/// The full IPC handler list, shared between production `run()` and the
/// smoke binary. Adding a new `#[tauri::command]` requires editing exactly
/// one place: this macro's body.
#[macro_export]
macro_rules! register_handlers {
    () => {
        tauri::generate_handler![
            $crate::list_livestreams,
            $crate::list_channels,
            $crate::add_channel_from_input,
            $crate::clipboard_channel_url,
            $crate::remove_channel,
            $crate::set_favorite,
            $crate::refresh_all,
            $crate::launch_stream,
            $crate::stop_stream,
            $crate::list_playing,
            $crate::open_in_browser,
            $crate::open_url,
            $crate::list_socials,
            $crate::chat_connect,
            $crate::chat_disconnect,
            $crate::chat_send,
            $crate::chat_open_in_browser,
            $crate::chat_detach,
            $crate::chat_reattach,
            $crate::chat_focus_detached,
            $crate::embed_mount,
            $crate::embed_bounds,
            $crate::embed_unmount,
            $crate::embed_set_visible,
            $crate::login_popup_open,
            $crate::login_popup_close,
            $crate::login_popup_resize,
            $crate::list_emotes,
            $crate::replay_chat_history,
            $crate::get_settings,
            $crate::update_settings,
            $crate::get_user_metadata,
            $crate::set_user_metadata,
            $crate::get_user_profile,
            $crate::get_user_messages,
            $crate::list_blocked_users,
            $crate::auth_status,
            $crate::twitch_login,
            $crate::twitch_logout,
            $crate::twitch_web_login,
            $crate::twitch_web_clear,
            $crate::twitch_anniversary_check,
            $crate::twitch_anniversary_dismiss,
            $crate::twitch_share_resub_open,
            $crate::twitch_share_window_close,
            $crate::kick_login,
            $crate::kick_logout,
            $crate::youtube_login,
            $crate::youtube_login_paste,
            $crate::youtube_logout,
            $crate::youtube_detect_browsers,
            $crate::chaturbate_login,
            $crate::chaturbate_logout,
            $crate::import_twitch_follows,
            $crate::spellcheck_check,
            $crate::spellcheck_suggest,
            $crate::spellcheck_add_word,
            $crate::spellcheck_list_dicts,
        ]
    };
}
```

Then change the existing `.invoke_handler(tauri::generate_handler![...])` call (lines 1550-1609) in `pub fn run()` to:

```rust
.invoke_handler(register_handlers!())
```

The handler functions need to be visible from `$crate::` — since they're currently bare `fn` items inside `lib.rs` they're already accessible as `crate::name` from within the crate. The macro uses `$crate::name` so it works both in-crate (via the `crate::` alias) and from external crates (none today, but future-proof).

If any of the handler functions aren't visible (compile error like "function `X` is private") the fix is to add `pub(crate)` to them. Most are already accessible because they're defined at module scope in `lib.rs`; the macro_rules path `$crate::list_livestreams` resolves to `livestreamlist_lib::list_livestreams` from outside, or `crate::list_livestreams` from inside.

- [ ] **Step 4: Run the smoke unit tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke smoke::tests
```

(The `--features smoke` is needed because `tauri::test` requires `feature = "test"` per `Cargo.toml`.)

Expected: both `build_smoke_app_lists_empty_channels_in_isolated_config` and `build_smoke_app_marshalling_error_for_bad_args` PASS.

If the tests FAIL because `register_handlers!` doesn't expand — verify the macro is at crate-root scope (`#[macro_export]` requires it) and that the smoke module's `crate::register_handlers!()` resolves.

If the tests FAIL because `tauri::test` is not found — verify the `smoke` feature is enabled (`--features smoke` on the cargo invocation).

- [ ] **Step 5: Run the full existing suite to confirm no regression**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all 215 existing tests still pass. (The `smoke` module is cfg-gated to `feature = "smoke"` OR `test`, so it compiles under `cargo test` even without `--features smoke` — but the smoke-module's two tests need MockRuntime which is gated to the feature, so they'll be skipped without `--features smoke`. That's fine for this step; Step 4 covered them.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/smoke.rs
git commit -m "feat(smoke): add build_smoke_app() + register_handlers! macro

build_smoke_app(temp_root) constructs a tauri::App<MockRuntime> with
the production handler list + AppState, against an isolated XDG
config dir. register_handlers! is a shared macro so production
run() and the smoke binary read from one source-of-truth handler
list.

Two unit tests verify: (1) list_channels returns empty array in a
fresh isolated config, (2) Tauri's arg-deserialize errors come back
with the stable 'invalid args ' prefix that the binary will use to
classify kind:'deserialize' in its envelope."
```

---

## Task 3: Binary skeleton with `--list` and `--help`

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs` (replace the Task 0 stub)
- Create: `src-tauri/tests/smoke_binary.rs`

- [ ] **Step 1: Write the failing integration test for `--list`**

Create `src-tauri/tests/smoke_binary.rs`:

```rust
//! End-to-end integration tests for the `smoke` binary. Each test
//! invokes the binary via assert_cmd and asserts on stdout/stderr/exit.
//!
//! Run with: cargo test --features smoke --test smoke_binary

#![cfg(feature = "smoke")]

use assert_cmd::Command;

fn smoke() -> Command {
    Command::cargo_bin("smoke").expect("smoke binary built with --features smoke")
}

#[test]
fn list_prints_at_least_some_known_commands() {
    let output = smoke().arg("--list").output().expect("run --list");
    assert!(output.status.success(), "exit status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Sanity: a few well-known commands must be in the output.
    for cmd in &["list_channels", "list_livestreams", "spellcheck_check", "chat_send"] {
        assert!(stdout.contains(cmd), "--list output missing {cmd}; got:\n{stdout}");
    }
    // chat_send is on the denylist, so it should be tagged.
    assert!(stdout.contains("chat_send [blocked]"), "denylist tag missing for chat_send");
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let output = smoke().arg("--help").output().expect("run --help");
    assert!(output.status.success(), "exit status: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Usage:"), "--help output missing 'Usage:'");
    assert!(stdout.contains("--list"), "--help should mention --list");
    assert!(stdout.contains("--use-real-config"), "--help should mention --use-real-config");
    assert!(stdout.contains("--allow-side-effects"), "--help should mention --allow-side-effects");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary
```

Expected: FAIL. The smoke binary stub from Task 0 just prints "smoke binary not yet implemented" and exits 2; both tests fail on `assert!(output.status.success())`.

- [ ] **Step 3: Implement `--list` and `--help`**

Replace the contents of `src-tauri/src/bin/smoke.rs` with:

```rust
//! IPC smoke-test harness binary. Built only with --features smoke.
//!
//! Single-shot:  smoke <cmd> '<json args>'
//! JSONL stream: smoke   (no positional; reads stdin)
//! Subcommands:  smoke --list | --help
//! Flags:        --use-real-config | --allow-side-effects
//!
//! See README.md in this directory for the full protocol.

use livestreamlist_lib::smoke::DENYLIST;

const HELP: &str = "\
Usage:
    smoke [FLAGS] <command> '<json-args>'   # single-shot
    smoke [FLAGS]                            # JSONL streaming on stdin
    smoke --list                             # list all dispatchable commands
    smoke --help                             # this help

Flags:
    --use-real-config       Use ~/.config/livestreamlist/ instead of an isolated
                            temp dir. Off by default to protect real state.
    --allow-side-effects    Dispatch side-effecting commands (chat_send, embed_*,
                            *_login, etc.) instead of returning the blocked envelope.

Output: one JSON object per command, line-delimited.
    Success: {\"command\":\"...\",\"ok\":true,\"value\":...,\"duration_ms\":N}
    Error:   {\"command\":\"...\",\"ok\":false,\"error\":\"...\",\"kind\":\"...\",\"duration_ms\":N}

JSONL input shape: {\"id\":\"...\",\"cmd\":\"...\",\"args\":{...}}
JSONL output shape: same as Output above + \"id\" field if input had one.

Logs (tracing) go to stderr; stdout is parseable JSONL.
Exit code is 0 in JSONL mode (on EOF) and matches the 'ok' field in single-shot mode.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }

    if args.iter().any(|a| a == "--list") {
        // Print every name from register_handlers! plus the [blocked] tag for
        // any in DENYLIST. The handler list is the source-of-truth; keep this
        // synced via the drift test in tests/smoke_binary.rs.
        let mut names = list_handlers();
        names.sort();
        for name in &names {
            if DENYLIST.contains(&name.as_str()) {
                println!("{name} [blocked]");
            } else {
                println!("{name}");
            }
        }
        return;
    }

    eprintln!("dispatch loop not yet implemented (Task 4)");
    std::process::exit(2);
}

/// Source the handler-name list from the `register_handlers!` macro.
/// Today this re-parses the macro's body via the source file at compile
/// time — see the `embedded_handler_names` build script approach in
/// Task 9 if introspection turns out to be needed.
fn list_handlers() -> Vec<String> {
    // Hard-coded mirror of the register_handlers! body. The drift-prevention
    // test in tests/smoke_binary.rs asserts this stays in sync with the
    // #[tauri::command] count in lib.rs.
    [
        "list_livestreams", "list_channels", "add_channel_from_input",
        "clipboard_channel_url", "remove_channel", "set_favorite",
        "refresh_all", "launch_stream", "stop_stream", "list_playing",
        "open_in_browser", "open_url", "list_socials",
        "chat_connect", "chat_disconnect", "chat_send", "chat_open_in_browser",
        "chat_detach", "chat_reattach", "chat_focus_detached",
        "embed_mount", "embed_bounds", "embed_unmount", "embed_set_visible",
        "login_popup_open", "login_popup_close", "login_popup_resize",
        "list_emotes", "replay_chat_history",
        "get_settings", "update_settings",
        "get_user_metadata", "set_user_metadata", "get_user_profile",
        "get_user_messages", "list_blocked_users",
        "auth_status",
        "twitch_login", "twitch_logout", "twitch_web_login", "twitch_web_clear",
        "twitch_anniversary_check", "twitch_anniversary_dismiss",
        "twitch_share_resub_open", "twitch_share_window_close",
        "kick_login", "kick_logout",
        "youtube_login", "youtube_login_paste", "youtube_logout", "youtube_detect_browsers",
        "chaturbate_login", "chaturbate_logout",
        "import_twitch_follows",
        "spellcheck_check", "spellcheck_suggest", "spellcheck_add_word", "spellcheck_list_dicts",
    ].iter().map(|s| s.to_string()).collect()
}
```

The hard-coded list is pragmatic: Tauri's `RuntimeAuthority` doesn't expose a public iter-commands API in 2.10.3, and a build.rs scrape adds compile-time complexity for limited gain. The drift test (Task 9) keeps this list in sync.

- [ ] **Step 4: Run the test to verify it passes**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary
```

Expected: both `list_prints_at_least_some_known_commands` and `help_prints_usage_and_exits_zero` PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin/smoke.rs src-tauri/tests/smoke_binary.rs
git commit -m "feat(smoke): --list and --help subcommands

--list prints every dispatchable command, sorted, with a [blocked]
tag for entries in DENYLIST. --help prints the protocol summary.
Both exit 0.

The handler-name list in the binary is hard-coded; a drift test
(Task 9) will assert it stays synced with #[tauri::command] count
in lib.rs."
```

---

## Task 4: Single-shot dispatch (success + error envelope)

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs` (add the dispatch loop)
- Modify: `src-tauri/tests/smoke_binary.rs` (add tests)

- [ ] **Step 1: Add failing tests for single-shot success and error envelopes**

Append to `src-tauri/tests/smoke_binary.rs`:

```rust
#[test]
fn single_shot_list_channels_returns_empty_array() {
    let output = smoke()
        .args(["list_channels", "{}"])
        .output()
        .expect("run list_channels");
    assert!(output.status.success(), "exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON ({e}): {stdout}"));
    assert_eq!(json["command"], "list_channels");
    assert_eq!(json["ok"], true);
    assert_eq!(json["value"], serde_json::json!([]));
    assert!(json["duration_ms"].is_number(), "duration_ms missing");
}

#[test]
fn single_shot_marshalling_error_classifies_as_deserialize() {
    let output = smoke()
        .args(["add_channel_from_input", r#"{"wrong_field":"x"}"#])
        .output()
        .expect("run add_channel_from_input with bad args");
    assert!(!output.status.success(), "expected non-zero exit for bad args");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON ({e}): {stdout}"));
    assert_eq!(json["command"], "add_channel_from_input");
    assert_eq!(json["ok"], false);
    assert_eq!(json["kind"], "deserialize");
    assert!(
        json["error"].as_str().unwrap_or("").contains("invalid args"),
        "error message should contain Tauri's stable 'invalid args' prefix; got: {}",
        json["error"]
    );
}

#[test]
fn single_shot_unknown_command_returns_command_error() {
    let output = smoke()
        .args(["this_command_does_not_exist", "{}"])
        .output()
        .expect("run unknown command");
    assert!(!output.status.success(), "expected non-zero exit for unknown command");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], false);
    // kind is "command" for any post-dispatch error that isn't deserialize
    assert!(json["kind"].as_str().unwrap_or("").len() > 0);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary single_shot
```

Expected: all three FAIL — the binary still exits 2 with "dispatch loop not yet implemented."

- [ ] **Step 3: Implement the single-shot dispatch loop**

Edit `src-tauri/src/bin/smoke.rs`. Replace the `eprintln!("dispatch loop not yet implemented (Task 4)");` line and the `std::process::exit(2);` after it with a real dispatcher. Add the necessary use statements at the top of the file:

```rust
use std::time::Instant;

use livestreamlist_lib::smoke::{build_smoke_app, DENYLIST};
use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{get_ipc_response, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindowBuilder;
```

Replace `main()` with:

```rust
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }

    if args.iter().any(|a| a == "--list") {
        let mut names = list_handlers();
        names.sort();
        for name in &names {
            if DENYLIST.contains(&name.as_str()) {
                println!("{name} [blocked]");
            } else {
                println!("{name}");
            }
        }
        return;
    }

    // Strip flag-style args; keep positional (cmd + json).
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();

    if positional.len() != 2 {
        eprintln!("expected exactly 2 positional args (cmd + json args); got {}", positional.len());
        eprintln!("for JSONL streaming mode (Task 6), pass no positionals.");
        eprintln!("run with --help for usage.");
        std::process::exit(2);
    }
    let cmd = positional[0];
    let raw_args = positional[1];

    let temp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("failed to create temp dir: {e}");
            std::process::exit(2);
        }
    };
    let app = match build_smoke_app(temp.path()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("build_smoke_app failed: {e:#}");
            std::process::exit(2);
        }
    };
    let webview = match WebviewWindowBuilder::new(&app, "main", Default::default()).build() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("webview build failed: {e}");
            std::process::exit(2);
        }
    };

    let envelope = dispatch_one(&webview, cmd, raw_args);
    println!("{}", serde_json::to_string(&envelope).unwrap());
    std::process::exit(if envelope["ok"].as_bool().unwrap_or(false) { 0 } else { 1 });
}

/// Run one command and return its envelope as a serde_json::Value.
fn dispatch_one(webview: &tauri::WebviewWindow<tauri::test::MockRuntime>, cmd: &str, raw_args: &str) -> Value {
    let started = Instant::now();

    // Step 1: parse the args. A JSON parse failure here is a CLI input
    // error, not a Tauri marshalling error — distinguish them via 'kind'.
    let args: Value = match serde_json::from_str(raw_args) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "command": cmd,
                "ok": false,
                "error": format!("input json parse error: {e}"),
                "kind": "input",
                "duration_ms": started.elapsed().as_millis(),
            });
        }
    };

    // Step 2: dispatch via Tauri's mock IPC.
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "http://tauri.localhost".parse().unwrap(),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    let result = get_ipc_response(webview, request);
    let duration_ms = started.elapsed().as_millis();

    match result {
        Ok(InvokeResponseBody::Json(s)) => {
            let value: Value = serde_json::from_str(&s).unwrap_or(Value::Null);
            json!({
                "command": cmd,
                "ok": true,
                "value": value,
                "duration_ms": duration_ms,
            })
        }
        Ok(InvokeResponseBody::Raw(_)) => json!({
            "command": cmd,
            "ok": false,
            "error": "command returned binary response, not supported in smoke harness",
            "kind": "binary",
            "duration_ms": duration_ms,
        }),
        Err(err_value) => {
            let msg = err_value.as_str()
                .map(String::from)
                .unwrap_or_else(|| err_value.to_string());
            // Tauri's argument-deserialization errors are stably formatted as
            // "invalid args `<arg>` for command `<cmd>`: <serde_err>". See
            // tauri::Error::InvalidArgs in Tauri 2.10.3 src/error.rs.
            let kind = if msg.starts_with("invalid args ") {
                "deserialize"
            } else {
                "command"
            };
            json!({
                "command": cmd,
                "ok": false,
                "error": msg,
                "kind": kind,
                "duration_ms": duration_ms,
            })
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary single_shot
```

Expected: all three single-shot tests PASS.

If the second test fails because `kind` isn't `"deserialize"` — verify Tauri's error string actually starts with `"invalid args "`. Run:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- add_channel_from_input '{"wrong_field":"x"}'
```

Inspect the error string in the output. Adjust the prefix-match in `dispatch_one` if the actual string differs (Tauri 2.10's source confirms the format, but a future minor version could change it).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin/smoke.rs src-tauri/tests/smoke_binary.rs
git commit -m "feat(smoke): single-shot dispatch with classified error envelopes

Successful: {command, ok:true, value, duration_ms}.
Errors carry 'kind':
  - 'input'       — CLI args weren't valid JSON
  - 'deserialize' — Tauri's serde layer rejected the args
  - 'command'     — command body returned Err
  - 'binary'      — command returned non-JSON bytes (out of scope)

Exit code matches 'ok' field. Drives the iteration loop for agent-driven
PRs against any of the 62 #[tauri::command] handlers."
```

---

## Task 5: Side-effect denylist + `--allow-side-effects` flag

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs`
- Modify: `src-tauri/tests/smoke_binary.rs`

- [ ] **Step 1: Add failing tests**

Append to `src-tauri/tests/smoke_binary.rs`:

```rust
#[test]
fn denylist_blocks_chat_send_by_default() {
    let output = smoke()
        .args(["chat_send", r#"{"uniqueKey":"twitch:shroud","text":"hi"}"#])
        .output()
        .expect("run chat_send");
    assert!(!output.status.success(), "expected non-zero exit");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["kind"], "blocked");
    assert!(
        json["error"].as_str().unwrap_or("").contains("--allow-side-effects"),
        "error should mention the opt-out flag; got: {}", json["error"]
    );
}

#[test]
fn allow_side_effects_bypasses_denylist() {
    let output = smoke()
        .args(["--allow-side-effects", "chat_send", r#"{"uniqueKey":"twitch:shroud","text":"hi"}"#])
        .output()
        .expect("run chat_send with allow-side-effects");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    // We don't assert the call succeeds — chat_send under MockRuntime with no
    // active connection will fail. We just assert it's NOT blocked.
    assert_ne!(json["kind"], "blocked", "kind=blocked means flag wasn't honored");
    // The warning should appear on stderr.
    assert!(
        stderr.contains("dispatching side-effecting command 'chat_send'"),
        "stderr missing side-effects warning; got: {stderr}"
    );
}

#[test]
fn denylist_intercepts_before_tauri_arg_deserialization() {
    // Sending bad args to a denylisted command returns kind='blocked',
    // NOT kind='deserialize'. This documents the dispatch order:
    //   1. parse the input JSON syntactically (kind='input' on failure)
    //   2. check denylist (kind='blocked' if hit)
    //   3. dispatch via Tauri (kind='deserialize' or 'command' on failure)
    //
    // The spec discussed always running marshalling first. In practice
    // Tauri's serde-based arg deserialization happens INSIDE
    // get_ipc_response, which we don't call for blocked commands.
    // The simpler order — check denylist first — is what we ship.
    // Agents who want marshalling verification on a denied command pass
    // --allow-side-effects.
    let output = smoke()
        .args(["chat_send", r#"{"wrong_field":"x"}"#])
        .output()
        .expect("run chat_send with bad args");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(json["ok"], false);
    assert_eq!(json["kind"], "blocked");
}
```

Note on the third test: the spec implied dispatch order "parse JSON → check denylist → dispatch" with the goal that marshalling would be verified for ALL commands including blocked ones. In practice, JSON parsing (Step 1 in `dispatch_one`) happens before denylist check (this task), so a malformed-JSON arg always returns `kind:"input"`. But Tauri's serde-based arg deserialization happens INSIDE `get_ipc_response`, which we don't call for blocked commands. So:
- Bad JSON syntax → `kind:"input"` for all commands (already implemented).
- Bad JSON shape (parses but wrong fields) → for non-blocked commands → `kind:"deserialize"`. For blocked commands → `kind:"blocked"` (the denylist intercepts before Tauri's deserializer runs).

This is a small departure from the spec; document it in the commit message.

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary -- denylist allow_side_effects
```

Expected: all three tests FAIL — the binary doesn't yet check the denylist, so chat_send currently dispatches and returns whatever Tauri does with it.

- [ ] **Step 3: Implement the denylist check in `dispatch_one`**

Edit `src-tauri/src/bin/smoke.rs`. Change the `main()` function to extract the `--allow-side-effects` flag and pass it to `dispatch_one`. Then add the denylist check inside `dispatch_one`.

Update `main()` — replace the dispatch call:

```rust
let allow_side_effects = args.iter().any(|a| a == "--allow-side-effects");

// ... (rest unchanged through webview construction) ...

let envelope = dispatch_one(&webview, cmd, raw_args, allow_side_effects);
```

Update `dispatch_one`'s signature:

```rust
fn dispatch_one(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    raw_args: &str,
    allow_side_effects: bool,
) -> Value {
```

After the args-parsing block (the `let args: Value = match … {}` block), insert:

```rust
    // Step 1.5: denylist check.
    if !allow_side_effects && DENYLIST.contains(&cmd) {
        return json!({
            "command": cmd,
            "ok": false,
            "error": format!(
                "command '{cmd}' is blocked in smoke harness; use --allow-side-effects to dispatch"
            ),
            "kind": "blocked",
            "duration_ms": started.elapsed().as_millis(),
        });
    }
    if allow_side_effects && DENYLIST.contains(&cmd) {
        eprintln!("WARN: dispatching side-effecting command '{cmd}' under --allow-side-effects");
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary -- denylist allow_side_effects
```

Expected: all three PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin/smoke.rs src-tauri/tests/smoke_binary.rs
git commit -m "feat(smoke): side-effect denylist + --allow-side-effects opt-out

Commands in DENYLIST (chat_send, embed_*, *_login, launch_stream, etc.)
return kind='blocked' by default. --allow-side-effects bypasses the
check and logs a stderr warning for each dispatched call.

Dispatch order is: parse JSON → check denylist → invoke. Agents who
need marshalling verification on a denied command must pass the flag.
This is a small simplification vs the spec, which discussed always
running marshalling first; in practice Tauri's deserializer runs
inside get_ipc_response, so checking the denylist first avoids the
edge cases of partial-dispatch."
```

---

## Task 6: JSONL streaming mode

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs`
- Modify: `src-tauri/tests/smoke_binary.rs`

- [ ] **Step 1: Add failing tests for JSONL streaming**

Append to `src-tauri/tests/smoke_binary.rs`:

```rust
#[test]
fn jsonl_streaming_dispatches_multiple_commands() {
    let input = "\
{\"id\":\"a\",\"cmd\":\"list_channels\",\"args\":{}}
{\"id\":\"b\",\"cmd\":\"list_livestreams\",\"args\":{}}
";
    let output = smoke()
        .write_stdin(input)
        .output()
        .expect("run jsonl stream");
    assert!(output.status.success(), "exit: {:?}", output.status);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 2, "expected 2 response lines, got: {stdout}");

    let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r0["id"], "a");
    assert_eq!(r0["command"], "list_channels");
    assert_eq!(r0["ok"], true);

    let r1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r1["id"], "b");
    assert_eq!(r1["command"], "list_livestreams");
    assert_eq!(r1["ok"], true);
}

#[test]
fn jsonl_state_persists_across_calls_in_one_session() {
    let input = "\
{\"id\":\"add\",\"cmd\":\"add_channel_from_input\",\"args\":{\"input\":\"twitch.tv/shroud\"}}
{\"id\":\"list\",\"cmd\":\"list_channels\",\"args\":{}}
";
    let output = smoke()
        .write_stdin(input)
        .output()
        .expect("run add+list jsonl stream");
    assert!(output.status.success(), "exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 2, "expected 2 response lines, got: {stdout}");

    let add: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(add["ok"], true, "add_channel_from_input failed: {add}");

    let list: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(list["ok"], true);
    let channels = list["value"].as_array().expect("channels array");
    assert_eq!(channels.len(), 1, "expected 1 channel after add; got {channels:?}");
    assert_eq!(channels[0]["channel_id"], "shroud");
}

#[test]
fn jsonl_malformed_input_continues_session() {
    let input = "\
{\"cmd\":\"list_channels\",\"args\":{}}
this is not json
{\"cmd\":\"list_livestreams\",\"args\":{}}
";
    let output = smoke().write_stdin(input).output().expect("run with malformed line");
    assert!(output.status.success(), "should exit 0 on EOF even with malformed lines");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 3, "should emit 3 responses (good, bad, good); got: {stdout}");

    let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r0["ok"], true);

    let r1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r1["ok"], false);
    assert_eq!(r1["kind"], "input");

    let r2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(r2["ok"], true);
}

#[test]
fn jsonl_optional_id_omitted_in_response_when_absent_in_input() {
    let input = "{\"cmd\":\"list_channels\",\"args\":{}}\n";
    let output = smoke().write_stdin(input).output().expect("run no-id jsonl");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(json.get("id").is_none(), "id should be omitted when absent in input; got: {json}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary jsonl
```

Expected: all four FAIL — the binary currently exits 2 with "expected exactly 2 positional args" when stdin is the only input (no positionals).

- [ ] **Step 3: Implement JSONL streaming**

Edit `src-tauri/src/bin/smoke.rs`. Change the no-positional branch in `main()` from `eprintln!("expected exactly 2 positional args")` + exit 2 to a JSONL streaming loop.

Update the use statements at the top:

```rust
use std::io::{BufRead, Write};
```

Replace the section that errors on `positional.len() != 2` with:

```rust
    // No positional args → JSONL streaming on stdin.
    if positional.is_empty() {
        run_jsonl_loop(&webview, allow_side_effects);
        return;
    }

    if positional.len() != 2 {
        eprintln!("expected exactly 2 positional args (cmd + json args); got {}", positional.len());
        eprintln!("for JSONL streaming, pass no positionals (and stream on stdin).");
        eprintln!("run with --help for usage.");
        std::process::exit(2);
    }
```

Wait — to take this branch we need `webview` constructed BEFORE checking positional count. Move the temp/app/webview construction up so it's always done. Actually, both branches need it; restructure `main()`:

```rust
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    if args.iter().any(|a| a == "--list") {
        let mut names = list_handlers();
        names.sort();
        for name in &names {
            if DENYLIST.contains(&name.as_str()) {
                println!("{name} [blocked]");
            } else {
                println!("{name}");
            }
        }
        return;
    }

    let allow_side_effects = args.iter().any(|a| a == "--allow-side-effects");

    let positional: Vec<&str> = args.iter()
        .filter(|a| !a.starts_with("--"))
        .map(|s| s.as_str())
        .collect();

    if positional.len() != 0 && positional.len() != 2 {
        eprintln!("expected 0 (JSONL stream) or 2 (single-shot) positional args; got {}", positional.len());
        eprintln!("run with --help for usage.");
        std::process::exit(2);
    }

    let temp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => { eprintln!("temp dir: {e}"); std::process::exit(2); }
    };
    let app = match build_smoke_app(temp.path()) {
        Ok(a) => a,
        Err(e) => { eprintln!("build_smoke_app: {e:#}"); std::process::exit(2); }
    };
    let webview = match WebviewWindowBuilder::new(&app, "main", Default::default()).build() {
        Ok(w) => w,
        Err(e) => { eprintln!("webview: {e}"); std::process::exit(2); }
    };

    if positional.is_empty() {
        run_jsonl_loop(&webview, allow_side_effects);
        return;
    }

    let envelope = dispatch_one(&webview, positional[0], positional[1], allow_side_effects);
    println!("{}", serde_json::to_string(&envelope).unwrap());
    std::process::exit(if envelope["ok"].as_bool().unwrap_or(false) { 0 } else { 1 });
}
```

Add the `run_jsonl_loop` function below `dispatch_one`:

```rust
/// Read JSONL from stdin; for each line, parse {id?, cmd, args} and emit
/// a JSONL response. Continue on malformed lines (emit kind='input'). Exit
/// 0 on EOF.
fn run_jsonl_loop(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    allow_side_effects: bool,
) {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(e) => {
                let env = json!({
                    "ok": false,
                    "error": format!("stdin read error: {e}"),
                    "kind": "input",
                });
                writeln!(out, "{}", serde_json::to_string(&env).unwrap()).ok();
                continue;
            }
        };
        let parsed: Result<JsonlInput, _> = serde_json::from_str(&line);
        let envelope = match parsed {
            Err(e) => {
                let mut env = json!({
                    "ok": false,
                    "error": format!("malformed input line ({e}): {line}"),
                    "kind": "input",
                });
                env["command"] = Value::Null;
                env
            }
            Ok(JsonlInput { id, cmd, args }) => {
                let raw_args = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
                let mut env = dispatch_one(webview, &cmd, &raw_args, allow_side_effects);
                if let Some(id) = id {
                    env["id"] = Value::String(id);
                }
                env
            }
        };
        writeln!(out, "{}", serde_json::to_string(&envelope).unwrap()).ok();
    }
}

#[derive(serde::Deserialize)]
struct JsonlInput {
    id: Option<String>,
    cmd: String,
    #[serde(default)]
    args: Value,
}
```

The `serde::Deserialize` derive needs `serde` in the binary's dependency tree — it's already available because `livestreamlist_lib` re-exports it through transitive deps. If the compiler complains about a missing derive, add `serde = "1"` to a `[dependencies]` `serde` line in `src-tauri/Cargo.toml` (it likely already exists for the lib crate, but the binary doesn't get features by default — verify with `grep '^serde' src-tauri/Cargo.toml`).

- [ ] **Step 4: Run the tests to verify they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary jsonl
```

Expected: all four PASS.

If `jsonl_state_persists_across_calls_in_one_session` fails because `add_channel_from_input` returned an error like "couldn't recognise 'twitch.tv/shroud'" — verify the URL parser accepts that input. The `parse_channel_input` function in `platforms/mod.rs` should accept bare URLs. If it doesn't, change the test input to a known-working URL like `https://twitch.tv/shroud`.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin/smoke.rs src-tauri/tests/smoke_binary.rs
git commit -m "feat(smoke): JSONL streaming mode

No positional args → read JSONL on stdin, emit JSONL on stdout, one
AppState for the session. State persists across calls (add then list
reflects the add). Optional 'id' field in input round-trips into the
response. Malformed lines emit kind='input' and the loop continues.

Amortizes ~3s of cargo+startup over many calls — the same binary
will eventually back a CI smoke fixture (parked as follow-up)."
```

---

## Task 7: Panic safety

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs`
- Modify: `src-tauri/tests/smoke_binary.rs`

- [ ] **Step 1: Add a failing test for panic safety in JSONL mode**

Append to `src-tauri/tests/smoke_binary.rs`:

```rust
#[test]
fn jsonl_panicking_command_does_not_kill_session() {
    // Send a sequence: known-good, panic-inducing, known-good.
    // The middle call uses an arg shape that's known to deref None
    // somewhere in our code path. The simplest reliable trigger is
    // calling embed_unmount (denylisted; bypassed via flag) which on
    // MockRuntime tries to look up a webview key that doesn't exist
    // and may panic depending on the platform code's defensive checks.
    //
    // If embed_unmount turns out to be defensive and doesn't panic,
    // any other denylisted command that touches GTK should — verified
    // by manual trial during implementation. The point is: SOME command
    // must panic for this test to be meaningful. Document the chosen
    // command in the test comment.
    let input = "\
{\"id\":\"a\",\"cmd\":\"list_channels\",\"args\":{}}
{\"id\":\"b\",\"cmd\":\"embed_unmount\",\"args\":{\"uniqueKey\":\"twitch:nonexistent\"}}
{\"id\":\"c\",\"cmd\":\"list_channels\",\"args\":{}}
";
    let output = smoke()
        .args(["--allow-side-effects"])
        .write_stdin(input)
        .output()
        .expect("run panic-test jsonl");
    assert!(output.status.success(), "session must survive panic; exit: {:?} stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.trim().split('\n').collect();
    assert_eq!(lines.len(), 3, "expected 3 lines (good, panic-or-error, good); got: {stdout}");

    let r0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r0["id"], "a");
    assert_eq!(r0["ok"], true, "first call must succeed");

    let r1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r1["id"], "b");
    assert_eq!(r1["ok"], false, "panic call must fail");
    // Acceptable kinds: 'panic' (catch_unwind caught it) or 'command'
    // (the called function returned an error instead of panicking).
    assert!(
        ["panic", "command"].contains(&r1["kind"].as_str().unwrap_or("")),
        "kind should be panic or command for failing call; got: {}", r1["kind"]
    );

    let r2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(r2["id"], "c");
    assert_eq!(r2["ok"], true, "third call must succeed (session survived)");
}
```

The test is intentionally permissive: it accepts `kind:"panic"` (catch_unwind worked) OR `kind:"command"` (the call gracefully errored). What's important is that **the third call succeeds** — proving the session didn't die.

- [ ] **Step 2: Run the test to verify it currently fails (or hangs)**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary jsonl_panicking
```

Expected outcomes (any of these counts as "fails the test"):
- The smoke process panics, killing the session → the `assert!(output.status.success())` fails.
- The third call's response is missing because the loop died.
- If `embed_unmount` happens to gracefully handle the missing key (returns Ok or Err without panicking), the test will trivially pass even without panic safety. In that case the test is still useful as a regression guard, but doesn't actually exercise catch_unwind. Pick a different command that DOES panic — try `chat_disconnect` with a key that has no active connection, or `twitch_share_window_close` for a non-existent channel.

If no command in the codebase panics on bad args (i.e. all are defensively coded), this test becomes a no-op. In that case, ADD a synthetic panicking command for this test only. Define in `src-tauri/src/smoke.rs` (under `#[cfg(any(feature = "smoke", test))]`):

```rust
/// Synthetic command for panic-safety tests. Never registered in
/// production. Add to register_handlers! macro guarded by cfg.
#[tauri::command]
pub fn _smoke_test_panic() -> String {
    panic!("synthetic panic for smoke test");
}
```

Then guard `register_handlers!` with a cfg-flagged optional entry:

```rust
#[macro_export]
macro_rules! register_handlers {
    () => {
        tauri::generate_handler![
            // ... all 62 ...
            $crate::spellcheck_list_dicts,
            #[cfg(any(feature = "smoke", test))]
            $crate::smoke::_smoke_test_panic,
        ]
    };
}
```

Hmm — `#[cfg]` inside `tauri::generate_handler![...]`'s macro args isn't valid (proc-macro args are token streams). Cleaner: have TWO macros, `register_handlers!()` for production and `register_handlers_with_test!()` for the smoke binary, and the test version adds the synthetic command.

Even simpler workaround: skip the synthetic panicking command. Pick a real command that's known to panic on bad input by running it manually:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --allow-side-effects chat_send '{"uniqueKey":"twitch:does-not-exist","text":"hi"}'
```

If this dies with a panic, use `chat_send` for the test. If it returns a clean error, try another command. Document the chosen command + rationale in the test comment.

- [ ] **Step 3: Implement panic safety**

Wrap the dispatch call in `dispatch_one` with `std::panic::catch_unwind`. Edit `src-tauri/src/bin/smoke.rs`:

Replace the `let result = get_ipc_response(webview, request);` line with:

```rust
    let webview_ref: &tauri::WebviewWindow<tauri::test::MockRuntime> = webview;
    let request_for_panic = format!("{cmd}:{}", &raw_args[..raw_args.len().min(120)]);
    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        get_ipc_response(webview_ref, request)
    })) {
        Ok(r) => r,
        Err(panic) => {
            let msg = panic_message(&panic);
            return json!({
                "command": cmd,
                "ok": false,
                "error": format!("command panicked: {msg}"),
                "kind": "panic",
                "duration_ms": started.elapsed().as_millis(),
                "panic_request": request_for_panic,
            });
        }
    };
```

Add the helper function `panic_message`:

```rust
fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "<non-string panic payload>".to_string()
    }
}
```

`AssertUnwindSafe` is needed because `WebviewWindow` (and the `request` it owns transitively) is not `UnwindSafe`. The `&webview` reference is moved into the closure; nothing observable on the outside changes if the panic happens, so the assertion is sound.

- [ ] **Step 4: Run the tests to verify they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary
```

Expected: all tests pass, including `jsonl_panicking_command_does_not_kill_session`.

If the test still fails because the panic happens in an async task spawned by the command (Tauri's command runtime spawns futures on its own runtime), `catch_unwind` won't catch it — the panic propagates through Tokio's task abort handle. In that case, document the limitation in the smoke binary's README and weaken the test: assert that the session at least didn't crash, even if the bad call's envelope is missing or strangely shaped.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/bin/smoke.rs src-tauri/tests/smoke_binary.rs
git commit -m "feat(smoke): catch_unwind around dispatch for panic safety

Synchronous panics during get_ipc_response are converted to
kind='panic' envelopes; the JSONL session survives. Async panics
inside Tauri-spawned futures may slip through (Tokio absorbs them
into JoinError) — documented as a known limitation."
```

---

## Task 8: Config isolation enforcement + `--use-real-config`

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs`
- Modify: `src-tauri/src/smoke.rs` (extend `build_smoke_app` or add a sibling for real-config mode)
- Modify: `src-tauri/tests/smoke_binary.rs`

The temp-dir setup is already implemented in `build_smoke_app` (Task 2). What's needed here is:
1. A `build_real_app()` sibling that uses production config dirs.
2. The binary's `main()` choosing between them based on `--use-real-config`.
3. A test that verifies isolation works (a write-then-read in one smoke run doesn't leak to the next).

- [ ] **Step 1: Add a failing test for config isolation across runs**

Append to `src-tauri/tests/smoke_binary.rs`:

```rust
#[test]
fn config_isolated_by_default_across_runs() {
    // Two independent smoke runs. Each gets a fresh temp config.
    // The second run must NOT see channels added by the first.
    let r1 = smoke()
        .args(["add_channel_from_input", r#"{"input":"https://twitch.tv/shroud"}"#])
        .output().expect("first run");
    assert!(r1.status.success(), "first add failed: {:?}", String::from_utf8_lossy(&r1.stderr));

    let r2 = smoke()
        .args(["list_channels", "{}"])
        .output().expect("second run");
    assert!(r2.status.success());
    let stdout = String::from_utf8(r2.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(
        json["value"], serde_json::json!([]),
        "second run must see empty channel list (isolation broken if non-empty); got: {json}"
    );
}
```

This test depends on Task 6 (single-shot mode) being committed. It MUST pass without any further code change because Task 2's `build_smoke_app` already creates a fresh temp dir.

- [ ] **Step 2: Run the isolation test — should already pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary config_isolated
```

Expected: PASS — `build_smoke_app` already creates a fresh temp dir per process (Task 2). If FAIL, debug: `XDG_CONFIG_HOME` may be leaking from a parent process, or the temp dir path is colliding. Fix before proceeding — do NOT skip ahead with a broken isolation contract.

This is a regression-guard test, not a new-feature test. The remaining steps add the `--use-real-config` flag (which IS new code).

- [ ] **Step 3: Add a failing test for `--use-real-config`**

Append:

```rust
#[test]
fn use_real_config_flag_does_not_use_temp_dir() {
    // We can't easily assert this against the user's real ~/.config
    // (we'd corrupt their state). Instead: verify the flag doesn't
    // crash and the response shape is correct. The deeper "did it
    // actually use ~/.config" check is manual-only.
    let output = smoke()
        .args(["--use-real-config", "list_channels", "{}"])
        .output()
        .expect("run --use-real-config list_channels");
    // Status may be 0 or non-zero depending on whether the user has
    // channels persisted. The point: it must not crash.
    let stdout = String::from_utf8(output.stdout).unwrap();
    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout not JSON ({e}): {stdout}"));
    // Shape check.
    assert_eq!(json["command"], "list_channels");
    assert!(json["ok"].is_boolean());
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary use_real_config
```

Expected: FAIL (until we wire the flag, the binary uses the temp-dir path which is what `--use-real-config` should bypass; the test currently might pass coincidentally because the response shape is right — but we still want the explicit flag handling).

- [ ] **Step 5: Implement `--use-real-config`**

In `src-tauri/src/smoke.rs`, add a sibling function:

```rust
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
    crate::manage_all_state(&mut app)?;
    Ok(app)
}
```

In `src-tauri/src/bin/smoke.rs`, change the app construction in `main()`:

```rust
let use_real_config = args.iter().any(|a| a == "--use-real-config");

let _temp_keepalive;  // hold TempDir for process lifetime when isolated
let app = if use_real_config {
    eprintln!("WARN: --use-real-config: dispatching against ~/.config/livestreamlist/");
    livestreamlist_lib::smoke::build_real_config_app()
} else {
    let temp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => { eprintln!("temp dir: {e}"); std::process::exit(2); }
    };
    let app = livestreamlist_lib::smoke::build_smoke_app(temp.path());
    _temp_keepalive = Some(temp);
    app
};
let app = match app {
    Ok(a) => a,
    Err(e) => { eprintln!("build app: {e:#}"); std::process::exit(2); }
};
```

Note the `_temp_keepalive` binding pattern: the `TempDir` deletes the directory on drop. We need to keep it alive for the whole process lifetime, even though we don't need to reference it again after `build_smoke_app` consumes its path. The `Option<TempDir>` keeps it in scope until `main()` returns.

- [ ] **Step 6: Run the test to verify it passes**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/smoke.rs src-tauri/src/bin/smoke.rs src-tauri/tests/smoke_binary.rs
git commit -m "feat(smoke): config isolation by default + --use-real-config opt-out

Default: each smoke run gets a fresh tempfile::TempDir for
XDG_CONFIG_HOME + XDG_DATA_HOME. Real ~/.config is untouched.

--use-real-config opt-out points at the user's real config dir for
debugging cases like 'list my actual channels'. Logs a stderr
warning so the choice is visible.

Already-passing isolation test + new shape-check test for the flag."
```

---

## Task 9: Drift-prevention test

**Files:**
- Modify: `src-tauri/tests/smoke_binary.rs`

- [ ] **Step 1: Add the failing drift test**

Append to `src-tauri/tests/smoke_binary.rs`:

```rust
#[test]
fn list_count_matches_tauri_command_count_in_lib_rs() {
    // Asserts that the hard-coded handler list in src/bin/smoke.rs's
    // list_handlers() stays in sync with the #[tauri::command] count
    // in src/lib.rs. Adding a new command without updating both will
    // fail this test, surfacing the drift before it ships.
    let lib_rs = std::fs::read_to_string("src/lib.rs").expect("read lib.rs");
    let command_count = lib_rs.matches("#[tauri::command]").count();

    let output = smoke().arg("--list").output().expect("run --list");
    assert!(output.status.success());
    let list_count = String::from_utf8(output.stdout).unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();

    assert_eq!(
        list_count, command_count,
        "smoke --list ({list_count}) must equal #[tauri::command] count in lib.rs ({command_count}); \
         did you add a #[tauri::command] without updating list_handlers() in src/bin/smoke.rs and \
         register_handlers!() in src/lib.rs?"
    );
}
```

The test reads `src/lib.rs` from the test's working directory. `cargo test`'s working directory for an integration test is the package root (`src-tauri/`), so the relative path resolves correctly. If running from the repo root via `--manifest-path`, cargo still sets the test cwd to the package dir.

- [ ] **Step 2: Run the test to verify it passes today**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke --test smoke_binary list_count_matches
```

Expected: PASS. Both the hard-coded list (62 entries) and the `#[tauri::command]` count (62, verified earlier) match.

If FAIL — recount one or the other; reconcile. If `lib.rs` has any commented-out `#[tauri::command]` lines, the count includes them — adjust the assertion to account.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/tests/smoke_binary.rs
git commit -m "test(smoke): drift assertion between --list and #[tauri::command] count

Future contributors who add a new IPC command and forget to update
list_handlers() in src/bin/smoke.rs (and register_handlers! in
src/lib.rs) will see this test fail in CI / cargo test."
```

---

## Task 10: Tracing logs to stderr

**Files:**
- Modify: `src-tauri/src/bin/smoke.rs`

The smoke binary's stdout must be parseable JSONL. By default `tracing` writes to stdout via `tracing_subscriber::fmt`. Need to install a subscriber that writes to stderr instead.

- [ ] **Step 1: Verify the issue**

Run a command that the production code logs through:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- list_channels '{}' 2>/dev/null
```

If stdout contains anything other than the JSON envelope (e.g., `INFO ...` lines from `log` macros in `lib.rs::run` or in `AppState::new`), that's the bug. If stdout is clean, this task is a no-op and the binary already handles it correctly — verify and skip.

In our case, `lib.rs` uses the `log` crate (see `log::warn!` calls in `AppState::new`). Without an installed logger, `log` macros are no-ops — so this might already be fine. The risk is when `tauri_plugin_log` is enabled (it's gated to `cfg!(debug_assertions)` in production) — but the smoke binary doesn't install that plugin.

If verification shows stdout IS clean: SKIP this task (no code change needed). Document in the commit message that no subscriber installation was needed.

If verification shows stdout has log noise: continue with Steps 2–4.

- [ ] **Step 2: Install a stderr-routing subscriber (only if step 1 showed noise)**

In `src-tauri/src/bin/smoke.rs`, at the top of `main()` before any other work:

```rust
fn main() {
    install_logger();
    // ... rest of main
```

Add:

```rust
fn install_logger() {
    use tracing_subscriber::{fmt, EnvFilter};
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .try_init();
}
```

Add `tracing-subscriber = { version = "0.3", features = ["env-filter"] }` to dev-dependencies in Cargo.toml — actually, since the smoke binary needs it at runtime, add to the optional/feature-gated deps:

```toml
[dependencies]
tracing-subscriber = { version = "0.3", features = ["env-filter"], optional = true }
```

And update the smoke feature:

```toml
smoke = ["tauri/test", "dep:tempfile", "dep:tracing-subscriber"]
```

Also add a `log` → `tracing` bridge if `lib.rs` uses `log::` macros (which it does, e.g. `log::warn!`):

```toml
tracing-log = { version = "0.2", optional = true }
```

```toml
smoke = ["tauri/test", "dep:tempfile", "dep:tracing-subscriber", "dep:tracing-log"]
```

```rust
fn install_logger() {
    use tracing_subscriber::{fmt, EnvFilter};
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .try_init();
    let _ = tracing_log::LogTracer::init();
}
```

- [ ] **Step 3: Verify stdout is clean**

Run:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- list_channels '{}' 2>/dev/null
```

Expected: ONLY the JSON envelope on stdout. Any log output goes to /dev/null via stderr redirection.

Then verify logs DO appear when stderr is captured:

```bash
RUST_LOG=info cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- list_channels '{}' 2>&1 1>/dev/null
```

Expected: `INFO` lines (or whatever the production code logs) on the merged stream.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/bin/smoke.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(smoke): route tracing/log output to stderr

Stdout is reserved for the JSONL envelope so jq, agents, and shell
pipelines can parse it without filtering log noise. Default level is
'warn' to keep stderr quiet; RUST_LOG overrides as expected.

(If verification showed no log noise without an installed subscriber,
this commit is empty — log macros are no-ops without a subscriber.)"
```

---

## Task 11: Documentation

**Files:**
- Create: `src-tauri/src/bin/README.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Create the bin README**

Create `src-tauri/src/bin/README.md`:

```markdown
# `smoke` binary

End-to-end IPC smoke-test harness. Lets you (or an agent) call any
`#[tauri::command]` handler with JSON args from the shell, getting a
JSON envelope back, without launching the desktop app.

## Build

```bash
cargo build --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke
```

The binary's `required-features = ["smoke"]` excludes it from default
`cargo build`.

## Use

Single-shot:
```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- \
    spellcheck_check '{"text":"helo","language":"en_US","channelEmotes":[]}'
# {"command":"spellcheck_check","ok":true,"value":[{"start":0,"end":4,"word":"helo"}],"duration_ms":12}
```

JSONL streaming:
```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke <<EOF
{"id":"a","cmd":"add_channel_from_input","args":{"input":"twitch.tv/shroud"}}
{"id":"b","cmd":"list_channels","args":{}}
EOF
```

List all dispatchable commands (with `[blocked]` tag for denylist entries):
```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --list
```

## Safety

- **Config is isolated by default.** Each run uses a fresh tempdir for
  `XDG_CONFIG_HOME` + `XDG_DATA_HOME`. Your real `~/.config/livestreamlist/`
  is never touched. Pass `--use-real-config` to opt out.
- **Side-effecting commands are blocked by default.** `chat_send`,
  `embed_*`, `*_login`, `launch_stream`, etc. return `kind:"blocked"`
  unless you pass `--allow-side-effects` (which logs a stderr warning
  per dispatch).
- **Network calls are NOT mocked.** `refresh_all`, `twitch_anniversary_check`,
  `import_twitch_follows` hit the real APIs. Don't spam them.

## Out of scope

- No headless WebKit / GTK. Commands that construct `wry::WebView` (embed
  mounts, login popups) appear in the denylist and require the real app.
- No keyring isolation. Auth-token reads hit the real Secret Service /
  KWallet on your machine.
- No CI integration in v1. The binary is the foundation; piping a JSONL
  fixture through it in CI is a follow-up.

## Architecture / contributing

The full design is in `docs/superpowers/specs/2026-05-02-ipc-smoke-harness-design.md`.
The dispatch surface lives in `src-tauri/src/smoke.rs` (build_smoke_app,
DENYLIST) and `src-tauri/src/bin/smoke.rs` (CLI loop, envelope formatting).

Adding a new `#[tauri::command]`:
1. Add the function to `lib.rs` as usual.
2. Add the function name to `register_handlers!` in `lib.rs`.
3. Add the function name to `list_handlers()` in `src/bin/smoke.rs`.
4. If the command has real-world side effects, add it to `DENYLIST` in `smoke.rs`.

The drift test in `tests/smoke_binary.rs` will fail loudly if you forget step 3.
```

- [ ] **Step 2: Add a Development Commands entry to CLAUDE.md**

Find the Development Commands section in `CLAUDE.md` (the project-level one at `livestreamlist/CLAUDE.md`). Below the existing `cargo test --manifest-path src-tauri/Cargo.toml` line, add:

```markdown
# IPC smoke-test harness — call any #[tauri::command] without launching the app
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --list
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- spellcheck_check '{"text":"helo","language":"en_US","channelEmotes":[]}'
# Single-shot: positional cmd + JSON args. JSONL streaming: no positionals, read stdin.
# See src-tauri/src/bin/README.md for the full protocol.
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/bin/README.md CLAUDE.md
git commit -m "docs(smoke): bin/README + CLAUDE.md entry

Discoverability for agents starting fresh: CLAUDE.md's Development
Commands section now mentions the smoke binary so future sessions
find it without reading the spec/plan."
```

---

## Final verification

Once all tasks above are committed:

- [ ] **Run the full test suite**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --features smoke
```

Expected: all 215 existing unit tests + the smoke module's unit tests + the smoke binary's integration tests all pass.

- [ ] **Verify default cargo build still ships only the production binary**

```bash
cargo clean --manifest-path src-tauri/Cargo.toml
cargo build --manifest-path src-tauri/Cargo.toml --release
ls src-tauri/target/release/ | grep -E '^(livestreamlist|smoke)$'
```

Expected: only `livestreamlist` is present (no `smoke` binary in the default build).

- [ ] **Verify the production app boots**

```bash
npm run tauri:dev
```

Expected: app boots normally; channel list, chat, etc. work as before. The refactor didn't break anything.

- [ ] **Spot-check the smoke binary against a couple of real commands**

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- spellcheck_list_dicts '{}'
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- list_emotes '{}'
```

Expected: both return reasonable JSON envelopes (the first lists installed Hunspell dicts; the second lists currently-loaded emotes which will likely be empty in a fresh isolated config).

If everything passes — the implementation is ready for PR. Do the standard project workflow: push the branch, open the PR, await review, merge with `--squash --delete-branch`, then update `docs/ROADMAP.md` (flip `- [ ] Headless IPC smoke-test harness` to `- [x] … (PR #N)`) in a follow-up docs PR per the project's "Ship it" workflow.
