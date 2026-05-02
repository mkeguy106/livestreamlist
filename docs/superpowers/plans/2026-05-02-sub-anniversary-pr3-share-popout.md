# Sub-Anniversary PR 3 — Share Popout Window

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the in-app transient `WebviewWindow` that opens Twitch's popout chat (`https://www.twitch.tv/popout/{login}/chat`) so the user can click Twitch's native Share button while signed in via the cookie captured by PR 1. **No banner UI in this PR** — verification is via devtools `__TAURI_INTERNALS__.invoke('twitch_share_resub_open', {uniqueKey: 'twitch:millyyy314'})`.

**Architecture:** New `src-tauri/src/share_window.rs` with `Mutex<HashMap<String, WebviewWindow>>` tracking open popouts. Window labels use `share-resub-{channel_login}` (NOT `unique_key` — Tauri labels can't contain `:`). Profile dir is `~/.local/share/livestreamlist/webviews/twitch_web/` (shared with `auth::twitch_web::login_via_webview` from PR 1, so the cookie is already present). Re-open is idempotent — focus existing window instead of creating a duplicate.

**Tech Stack:** Tauri 2 (`WebviewWindowBuilder`, `WebviewUrl::External`), `parking_lot::Mutex`, no new crates.

**Spec:** `docs/superpowers/specs/2026-05-02-sub-anniversary-banner-design.md`

**Stacks on:** PRs 1 + 2 (already merged to main).

---

## File Structure

**New:**
- `src-tauri/src/share_window.rs` — module
  - `pub struct ShareWindowState { inner: Mutex<HashMap<String, WebviewWindow>> }`
  - `pub fn open(app, channel_login, display_name, state)` 
  - `pub fn close(app, channel_login, state)`
  - `pub fn close_all(state)`

**Modified:**
- `src-tauri/src/lib.rs` — `mod share_window;`, `Arc<share_window::ShareWindowState>` in AppState, 2 new IPC commands
- `src/ipc.js` — wrappers + mock fallbacks
- `CLAUDE.md` — IPC table entries
- `docs/ROADMAP.md` — sub-bullet for PR 3

---

## Task 0: Module skeleton + state wiring

**Files:**
- Create `src-tauri/src/share_window.rs`
- Modify `src-tauri/src/lib.rs` (mod declaration + AppState field + constructor)

- [ ] **Step 1: Create `src-tauri/src/share_window.rs`**

```rust
//! Transient Twitch popout-chat windows for sharing sub anniversaries.
//!
//! When the user clicks "Share now" in the sub-anniversary banner
//! (PR 4), we open `https://www.twitch.tv/popout/{login}/chat` in a
//! native top-level WebviewWindow that shares the profile dir with
//! `auth::twitch_web::login_via_webview` (PR 1) — so the captured
//! `auth-token` cookie is already present and the user lands signed
//! in. They click Twitch's native Share button, the resub fires as a
//! USERNOTICE, and PR 4's auto-dismiss listener observes it on our
//! IRC stream and calls `close()` here.
//!
//! Window lifecycle:
//! - `open(login)` — focus existing if any, else build new + register
//! - `close(login)` — drop from registry, close the window
//! - `close_all()` — used when user toggles the feature off
//!
//! Window label is `share-resub-{channel_login}` (not `unique_key` —
//! Tauri labels disallow `:`).

use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::utils::config::Color;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub struct ShareWindowState {
    inner: Mutex<HashMap<String, WebviewWindow>>,
}

impl ShareWindowState {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl Default for ShareWindowState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedShareWindowState = Arc<ShareWindowState>;

fn label_for(channel_login: &str) -> String {
    format!("share-resub-{channel_login}")
}
```

- [ ] **Step 2: Register the module in `src-tauri/src/lib.rs`**

Find the `mod` declarations near the top (around lines 1-30 — look for `mod auth;`, `mod chat;`, etc.). Add `mod share_window;` in alphabetical order.

- [ ] **Step 3: Wire SharedShareWindowState into AppState**

In `AppState` struct (lines ~31-38), add a field:

```rust
share_windows: share_window::SharedShareWindowState,
```

In `AppState::new()`, add to the constructor:

```rust
share_windows: Arc::new(share_window::ShareWindowState::new()),
```

- [ ] **Step 4: Verify**

```
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected clean (warnings about unused state field, unused `label_for`, unused `WebviewWindowBuilder` etc. are fine — Tasks 1-2 consume them).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/share_window.rs src-tauri/src/lib.rs
git commit -m "feat(share-window): module skeleton + state wiring"
```

---

## Task 1: `open` + `close` + `close_all`

**Files:** Modify `src-tauri/src/share_window.rs`.

The reference for the WebviewWindow build pattern is `src-tauri/src/auth/twitch_web.rs::login_via_webview` (PR 1). Same profile dir, same builder pattern. Difference: a different URL, different size, different title.

- [ ] **Step 1: Add `open`, `close`, `close_all`**

Append to `src-tauri/src/share_window.rs`:

```rust
/// Open Twitch's popout chat for the given channel. If a window for
/// this channel is already open, focus it instead of creating a
/// duplicate.
pub fn open(
    app: &AppHandle,
    channel_login: &str,
    display_name: &str,
    state: &ShareWindowState,
) -> Result<()> {
    let label = label_for(channel_login);

    // Re-entry: focus existing
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        // Re-register in case our HashMap got out of sync (window
        // could have been opened by another path); this is idempotent.
        state.inner.lock().insert(label.clone(), existing);
        return Ok(());
    }

    let profile_dir = crate::auth::twitch_web::webview_profile_dir()
        .context("webview profile dir for share popout")?;
    let main = app.get_webview_window("main");
    let zinc_950 = Color(9, 9, 11, 255);

    let url = format!("https://www.twitch.tv/popout/{channel_login}/chat");
    let mut builder = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::External(url.parse().context("parsing popout URL")?),
    )
    .title(format!("Share your sub anniversary — {display_name}"))
    .inner_size(380.0, 720.0)
    .min_inner_size(320.0, 480.0)
    .data_directory(profile_dir)
    .visible(false)
    .background_color(zinc_950)
    .center()
    .devtools(cfg!(debug_assertions))
    .on_page_load(|w, payload| {
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            let _ = w.show();
        }
    });
    if let Some(main) = main.as_ref() {
        builder = builder
            .transient_for(main)
            .context("transient_for(main) on share popout window")?;
    }
    let window = builder.build().context("opening share popout window")?;

    state.inner.lock().insert(label, window);
    Ok(())
}

/// Close the share popout for `channel_login` if open. No-op otherwise.
pub fn close(app: &AppHandle, channel_login: &str, state: &ShareWindowState) {
    let label = label_for(channel_login);
    let window = state.inner.lock().remove(&label);
    if let Some(w) = window {
        let _ = w.close();
        return;
    }
    // Window might have been closed by user (Tauri removes from manager
    // but our HashMap doesn't auto-clear). Best-effort lookup + close.
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.close();
    }
}

/// Close every registered popout. Used when the user toggles the
/// sub-anniversary feature off, or on Twitch web logout.
pub fn close_all(state: &ShareWindowState) {
    let windows: Vec<WebviewWindow> = state.inner.lock().drain().map(|(_, w)| w).collect();
    for w in windows {
        let _ = w.close();
    }
}
```

- [ ] **Step 2: Verify compile**

```
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected clean (warnings about unused `open`/`close`/`close_all` go away in Task 2 when the IPC commands consume them).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/share_window.rs
git commit -m "feat(share-window): open + close + close_all"
```

---

## Task 2: IPC commands

**Files:** Modify `src-tauri/src/lib.rs`.

Two commands: `twitch_share_resub_open(unique_key)` and `twitch_share_window_close(unique_key)`. The frontend passes `unique_key` (e.g. `twitch:millyyy314`); we derive `channel_login` from the resolved Channel.

- [ ] **Step 1: Add the commands**

Insert below `twitch_anniversary_dismiss` (PR 2's last command) in `src-tauri/src/lib.rs`:

```rust
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
        return Err(format!("share popout only supported for Twitch; got {:?}", channel.platform));
    }
    share_window::open(
        &app,
        &channel.channel_id,
        &channel.display_name,
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
```

- [ ] **Step 2: Register in `generate_handler!`**

Find `twitch_anniversary_dismiss,` in the `generate_handler!` macro. Add directly after:

```rust
            twitch_anniversary_check,
            twitch_anniversary_dismiss,
            twitch_share_resub_open,
            twitch_share_window_close,
```

- [ ] **Step 3: Verify**

```
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```
Both must be clean / 179 passing.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(share-window): twitch_share_resub_open + close IPC"
```

---

## Task 3: Frontend ipc.js wrappers

**Files:** Modify `src/ipc.js`.

- [ ] **Step 1: Add wrappers**

After `twitchAnniversaryDismiss` (added in PR 2):

```js
export const twitchShareResubOpen = (uniqueKey) =>
  invoke('twitch_share_resub_open', { uniqueKey });
export const twitchShareWindowClose = (uniqueKey) =>
  invoke('twitch_share_window_close', { uniqueKey });
```

- [ ] **Step 2: Add mock cases**

In the `mockInvoke` switch:

```js
    case 'twitch_share_resub_open':
      // Mock: noop in browser-only dev (no native window).
      return null;
    case 'twitch_share_window_close':
      return null;
```

- [ ] **Step 3: Verify**

`npm run build` clean.

- [ ] **Step 4: Commit**

```bash
git add src/ipc.js
git commit -m "feat(share-window): ipc.js wrappers + mock fallbacks"
```

---

## Task 4: Docs + roadmap + ship

- [ ] **Step 1: CLAUDE.md IPC table**

After the `twitch_anniversary_dismiss` row in the IPC table:

```
| `twitch_share_resub_open` | `uniqueKey` | Open transient WebviewWindow at `twitch.tv/popout/{login}/chat` with shared web-cookie profile so user can click Twitch's native Share button. Idempotent (focus existing) |
| `twitch_share_window_close` | `uniqueKey` | Close the popout window for that channel (idempotent) |
```

- [ ] **Step 2: ROADMAP.md sub-bullet**

After the existing PR 2 sub-bullet:

```
  - [x] PR 3: Share popout window (`share_window.rs` + 2 IPC commands) — opens twitch.tv/popout/{login}/chat in a transient signed-in WebviewWindow (PR #N)
```

- [ ] **Step 3: Final verify**

```
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run build
```
All green.

- [ ] **Step 4: Commit, push, PR, merge**

```bash
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: note share_window IPC + PR 3 in roadmap"
git push -u origin feat/sub-anniversary-pr3-share-popout
gh pr create --title "Sub-anniversary PR 3 — share popout window" --body "..."
# After PR opens, replace #N with actual number, push fixup, merge --squash --delete-branch
```

---

## Self-review

- [x] Spec coverage — all spec items in "Share popout window" section: `share_window.rs::open/close/close_all`, IPC commands, shared profile dir, idempotent re-entry, channel_login (not unique_key) for window label.
- [x] Type consistency — `SharedShareWindowState` consistent across module + AppState + commands.
- [x] No placeholders.
