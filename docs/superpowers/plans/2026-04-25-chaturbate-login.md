# Chaturbate Login Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an in-app webview sign-in flow for Chaturbate so the chat embed (and Phase 5's follow-import) sees a populated cookie profile, plus the UI surface to drive it.

**Architecture:** Mirror `auth/youtube.rs` shape, slimmed down. New Rust module `auth/chaturbate.rs` writes a stamp file when `sessionid` is captured in a popup `chaturbate-login` WebviewWindow whose `data_directory` is the same profile dir the chat embed uses. Embed page-load extends to verify the cookie still exists, emitting `chat:auth:chaturbate` events that drive a chat-pane banner and the Preferences row.

**Tech Stack:** Rust (Tauri 2, `chrono`, `serde`, `parking_lot`), React 18, plain CSS. No new crates, no new npm packages.

**Spec:** [`docs/superpowers/specs/2026-04-25-chaturbate-login-design.md`](../specs/2026-04-25-chaturbate-login-design.md)

---

## Task 1: Auth module — types, persistence, helpers

**Files:**
- Create: `src-tauri/src/auth/chaturbate.rs`
- Modify: `src-tauri/src/auth/mod.rs:1-5`

This task lands the persistence layer (no webview yet). After it, `cargo test` covers the round-trip.

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/auth/chaturbate.rs` (file does not yet exist; create it with just this skeleton):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn round_trip_serialises_rfc3339() {
        let stamp = ChaturbateAuth {
            logged_in_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap(),
            last_verified_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 11, 30, 0).unwrap(),
        };
        let json = serde_json::to_string(&stamp).unwrap();
        // chrono serialises DateTime<Utc> as RFC3339 by default.
        assert!(json.contains("2026-04-25T10:00:00Z"));
        assert!(json.contains("2026-04-25T11:30:00Z"));
        let back: ChaturbateAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(back.logged_in_at, stamp.logged_in_at);
        assert_eq!(back.last_verified_at, stamp.last_verified_at);
    }
}
```

- [ ] **Step 2: Run tests, expect fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chaturbate::tests
```

Expected: compile error — `ChaturbateAuth` not defined.

- [ ] **Step 3: Implement the type, persistence, helpers**

Replace `src-tauri/src/auth/chaturbate.rs` with:

```rust
//! Chaturbate auth — in-app webview sign-in.
//!
//! Chaturbate has no real OAuth surface. Logged-in state is decided by
//! the `sessionid` cookie on chaturbate.com (see livestream.list.qt's
//! chat/chaturbate_web_chat.py for prior art). We open a popup
//! WebviewWindow at the login page with a persistent profile dir, poll
//! its cookie jar until `sessionid` appears, and write a small stamp
//! file marking the user as signed in. The cookies themselves live in
//! the webview profile dir (shared with the chat embed); the stamp is
//! a presence flag plus timestamps the UI uses to render hints.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::config;

const STAMP_FILENAME: &str = "chaturbate-auth.json";
const LOGIN_WINDOW_LABEL: &str = "chaturbate-login";
const LOGIN_URL: &str = "https://chaturbate.com/auth/login/";
const SITE_URL: &str = "https://chaturbate.com/";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300); // 5 min — generous for 2FA / age check

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaturbateAuth {
    pub logged_in_at: DateTime<Utc>,
    pub last_verified_at: DateTime<Utc>,
}

fn stamp_path() -> Result<PathBuf> {
    Ok(config::data_dir()?.join(STAMP_FILENAME))
}

pub fn webview_profile_dir() -> Result<PathBuf> {
    let dir = config::data_dir()?.join("webviews").join("chaturbate");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}

pub fn load() -> Result<Option<ChaturbateAuth>> {
    let path = stamp_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let stamp: ChaturbateAuth = serde_json::from_slice(&bytes)
        .context("parsing Chaturbate stamp file")?;
    Ok(Some(stamp))
}

pub fn save(stamp: &ChaturbateAuth) -> Result<()> {
    let path = stamp_path()?;
    let bytes = serde_json::to_vec(stamp).context("serialising Chaturbate stamp")?;
    config::atomic_write(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Bumps `last_verified_at` if the stamp exists. No-op when not signed in.
pub fn touch_verified() -> Result<()> {
    let Some(mut stamp) = load()? else {
        return Ok(());
    };
    stamp.last_verified_at = Utc::now();
    save(&stamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn round_trip_serialises_rfc3339() {
        let stamp = ChaturbateAuth {
            logged_in_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap(),
            last_verified_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 11, 30, 0).unwrap(),
        };
        let json = serde_json::to_string(&stamp).unwrap();
        assert!(json.contains("2026-04-25T10:00:00Z"));
        assert!(json.contains("2026-04-25T11:30:00Z"));
        let back: ChaturbateAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(back.logged_in_at, stamp.logged_in_at);
        assert_eq!(back.last_verified_at, stamp.last_verified_at);
    }
}
```

The `WebviewUrl`, `WebviewWindowBuilder`, `AppHandle`, `Manager` imports are reserved for the next task; keep them now to avoid a churn-y diff. Suppress unused warnings with `#[allow(unused_imports)]` if cargo complains.

Actually — to keep this task self-contained, drop the webview imports for now and add them in Task 2:

```rust
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config;
```

Plus the `STAMP_FILENAME` constant and the rest above. Drop `LOGIN_*`, `SITE_URL`, `POLL_INTERVAL`, `LOGIN_TIMEOUT`, `anyhow!`, `Duration` until Task 2.

- [ ] **Step 4: Register the module**

Modify `src-tauri/src/auth/mod.rs` (current contents shown for reference):

```rust
mod callback_server;
pub mod chaturbate;
pub mod kick;
mod tokens;
pub mod twitch;
pub mod youtube;
```

(Add the `pub mod chaturbate;` line; keep alphabetical order with `kick`.)

- [ ] **Step 5: Run tests, expect pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chaturbate::tests
```

Expected: 1 test, all pass. Also run `cargo check --manifest-path src-tauri/Cargo.toml` and confirm zero warnings introduced.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/auth/chaturbate.rs src-tauri/src/auth/mod.rs
git commit -m "feat(chaturbate): add auth stamp module"
```

---

## Task 2: Login webview flow

**Files:**
- Modify: `src-tauri/src/auth/chaturbate.rs` (add `clear`, `login_via_webview`)

The login flow opens a child webview, polls for the `sessionid` cookie, and saves the stamp on success. Webview-driven flow is exercised manually; no automated test.

- [ ] **Step 1: Add the imports + login constants**

At the top of `src-tauri/src/auth/chaturbate.rs`, replace the import block with:

```rust
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::config;
```

And add these constants below the existing `STAMP_FILENAME`:

```rust
const LOGIN_WINDOW_LABEL: &str = "chaturbate-login";
const LOGIN_URL: &str = "https://chaturbate.com/auth/login/";
const SITE_URL: &str = "https://chaturbate.com/";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
```

- [ ] **Step 2: Add `clear()` and `login_via_webview()`**

Append below `touch_verified` (before `#[cfg(test)]`):

```rust
pub fn clear() -> Result<()> {
    if let Ok(path) = stamp_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(dir) = webview_profile_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

/// Open a child WebviewWindow at the Chaturbate login page, poll its
/// cookie jar until `sessionid` appears, then save + close. Bubbles a
/// clear error if the user closes the window or the timeout (5 min)
/// expires.
pub async fn login_via_webview(app: AppHandle) -> Result<ChaturbateAuth> {
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.close();
    }
    let profile_dir = webview_profile_dir()?;
    let window = WebviewWindowBuilder::new(
        &app,
        LOGIN_WINDOW_LABEL,
        WebviewUrl::External(LOGIN_URL.parse()?),
    )
    .title("Sign in to Chaturbate")
    .inner_size(480.0, 720.0)
    .min_inner_size(400.0, 600.0)
    .data_directory(profile_dir)
    .build()
    .context("opening Chaturbate login window")?;

    let site: url::Url = SITE_URL.parse()?;
    let started = std::time::Instant::now();

    loop {
        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            anyhow::bail!("Chaturbate login timed out after 5 minutes");
        }
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            anyhow::bail!("login window closed before sign-in completed");
        }

        match window.cookies_for_url(site.clone()) {
            Ok(jar) => {
                let signed_in = jar
                    .iter()
                    .any(|c| c.name() == "sessionid" && !c.value().is_empty());
                if signed_in {
                    let now = Utc::now();
                    let stamp = ChaturbateAuth {
                        logged_in_at: now,
                        last_verified_at: now,
                    };
                    save(&stamp)?;
                    let _ = window.close();
                    return Ok(stamp);
                }
            }
            Err(e) => log::debug!("cookies_for_url(chaturbate.com): {e}"),
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
```

The unused `anyhow!` import is consumed by `anyhow::bail!`. Leave the `Manager` import — `app.get_webview_window` requires the trait in scope.

- [ ] **Step 3: Verify it builds**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean build, no warnings about unused imports.

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chaturbate::tests
```

Expected: existing test still passes.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/auth/chaturbate.rs
git commit -m "feat(chaturbate): webview-driven login + clear"
```

---

## Task 3: Profile-dir consolidation in `embed.rs`

**Files:**
- Modify: `src-tauri/src/embed.rs:349-359`

The embed currently constructs the chaturbate profile dir inline. Reroute it through `auth::chaturbate::webview_profile_dir` so the login window and the embed agree on a single source of truth. The YouTube branch already does this (delegates to `auth::youtube::webview_profile_dir`).

- [ ] **Step 1: Replace `profile_dir` body**

In `src-tauri/src/embed.rs`, the existing function (lines 349–359) reads:

```rust
fn profile_dir(platform: Platform) -> Result<PathBuf> {
    let name = match platform {
        Platform::Youtube => "youtube",
        Platform::Chaturbate => "chaturbate",
        Platform::Twitch | Platform::Kick => "_unused",
    };
    let dir = config::data_dir()?.join("webviews").join(name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}
```

Replace with:

```rust
fn profile_dir(platform: Platform) -> Result<PathBuf> {
    match platform {
        Platform::Youtube => crate::auth::youtube::webview_profile_dir(),
        Platform::Chaturbate => crate::auth::chaturbate::webview_profile_dir(),
        Platform::Twitch | Platform::Kick => {
            anyhow::bail!("no webview profile dir for {:?}", platform)
        }
    }
}
```

The unreachable `_unused` case becomes an error: nothing in the codebase calls `profile_dir(Twitch | Kick)` today, so flipping it to a hard error is safer than silently routing them to a stub directory.

- [ ] **Step 2: Verify imports**

`crate::auth::youtube` is already used elsewhere in the file (check the top of `embed.rs`); `crate::auth::chaturbate` is reachable now that Task 1 registered it. The existing `config` import is no longer used inside `profile_dir`, but `config` is referenced elsewhere in the file. If Rust warns about unused imports after this change, leave it alone — it isn't unused at the file level.

- [ ] **Step 3: Build**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "refactor(embed): delegate profile_dir to auth modules"
```

---

## Task 4: `EmbedManager::unmount_platform` helper

**Files:**
- Modify: `src-tauri/src/embed.rs` (add method on `EmbedManager`)

Logout needs to close any active Chaturbate embed before nuking the profile dir on disk. Add a tiny method that closes `current` if its platform matches.

- [ ] **Step 1: Add the method**

Inside `impl EmbedManager` (after the existing `unmount` method, around line 336), add:

```rust
/// Close the current embed if it belongs to `platform`. Idempotent.
/// Used by auth flows that need to release the profile dir before
/// removing it on disk.
pub fn unmount_platform(&self, platform: Platform) {
    let mut g = self.inner.lock();
    if let Some(cur) = &g.current {
        if cur.platform != platform {
            return;
        }
    } else {
        return;
    }
    if let Some(prev) = g.current.take() {
        let _ = prev.window.close();
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "feat(embed): unmount_platform helper"
```

---

## Task 5: Embed page-load auth verification

**Files:**
- Modify: `src-tauri/src/embed.rs` (extend `on_page_load` closure, emit auth event)

When a Chaturbate embed finishes loading, peek at its cookie store to confirm `sessionid` is still present. Emit `chat:auth:chaturbate` so the UI banner and Preferences row stay accurate.

- [ ] **Step 1: Add the verification helper at the bottom of `embed.rs`**

Below the existing `set_bypass_compositor` function (and outside any `#[cfg(target_os)]` block):

```rust
#[derive(Clone, serde::Serialize)]
struct ChaturbateAuthEvent {
    signed_in: bool,
    /// "ok" | "session_expired" | "not_logged_in"
    reason: &'static str,
}

/// On a Chaturbate embed page-load-finished, decide auth status from the
/// embed's own cookie store and broadcast it. Stamp file gets touched
/// or cleared so `auth_status` stays in sync.
fn verify_chaturbate_auth(window: &WebviewWindow, app: &AppHandle) {
    let site: url::Url = match "https://chaturbate.com/".parse() {
        Ok(u) => u,
        Err(_) => return,
    };
    let signed_in = match window.cookies_for_url(site) {
        Ok(jar) => jar
            .iter()
            .any(|c| c.name() == "sessionid" && !c.value().is_empty()),
        Err(e) => {
            log::warn!("verify_chaturbate_auth cookies_for_url: {e:#}");
            return; // transient — don't flap the UI
        }
    };
    let stamp_present = matches!(crate::auth::chaturbate::load(), Ok(Some(_)));
    let reason = if signed_in {
        if let Err(e) = crate::auth::chaturbate::touch_verified() {
            log::warn!("touch_verified failed: {e:#}");
        }
        "ok"
    } else if stamp_present {
        if let Err(e) = crate::auth::chaturbate::clear() {
            log::warn!("clear (drift) failed: {e:#}");
        }
        "session_expired"
    } else {
        "not_logged_in"
    };
    let payload = ChaturbateAuthEvent { signed_in, reason };
    if let Err(e) = app.emit("chat:auth:chaturbate", payload) {
        log::warn!("emit chat:auth:chaturbate: {e:#}");
    }
}
```

`Emitter` trait import for `app.emit(...)`: add `use tauri::Emitter;` at the top of `embed.rs` (next to `use tauri::{...}`); the rest of the file may already use `Manager` but not `Emitter`.

- [ ] **Step 2: Wire it into the existing `on_page_load`**

Find the existing `.on_page_load(...)` block in `EmbedManager::mount` (around line 233):

```rust
.on_page_load(move |w: WebviewWindow, payload: PageLoadPayload<'_>| {
    if matches!(payload.event(), PageLoadEvent::Finished) {
        let _ = w.show();
    }
})
```

Replace with:

```rust
.on_page_load({
    let app_for_load = app.clone();
    let platform_for_load = channel.platform;
    move |w: WebviewWindow, payload: PageLoadPayload<'_>| {
        if matches!(payload.event(), PageLoadEvent::Finished) {
            let _ = w.show();
            if platform_for_load == Platform::Chaturbate {
                verify_chaturbate_auth(&w, &app_for_load);
            }
        }
    }
})
```

`app` is the `&AppHandle` parameter of `mount`; `channel.platform` is the value already in scope. Both must be cloned/copied (Platform is `Copy` — see `platforms/mod.rs`; `AppHandle` is `Clone`).

- [ ] **Step 3: Build**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean. If there's an `Emitter` trait error, confirm the `use tauri::Emitter;` line is present.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "feat(chaturbate): verify auth on embed page-load"
```

---

## Task 6: IPC commands + `auth_status` extension

**Files:**
- Modify: `src-tauri/src/lib.rs` (extend `AuthStatus`, add `chaturbate_login` / `chaturbate_logout`, register handlers)

Wire the Rust auth module to invoke commands and surface the status alongside Twitch/Kick/YouTube.

- [ ] **Step 1: Extend the AuthStatus types**

Find the `AuthStatus` and `YoutubeAuthStatus` structs in `src-tauri/src/lib.rs` (around lines 641–654). Add a sibling struct and field:

```rust
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
```

- [ ] **Step 2: Populate the field in `auth_status`**

Find the `auth_status` command (around line 657). Replace the body with:

```rust
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
```

- [ ] **Step 3: Add the login / logout commands**

Below the existing `youtube_logout` command (around line 748), append:

```rust
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
```

The `Platform` import is already in scope at the top of `lib.rs` (used by `open_in_browser`); the `Arc<embed::EmbedManager>` state is registered via `app.manage(embed_mgr)` in `setup`.

- [ ] **Step 4: Register the handlers**

In `tauri::generate_handler![...]` (around line 935), add the two commands. Find the existing block:

```rust
youtube_login,
youtube_login_paste,
youtube_logout,
youtube_detect_browsers,
```

Insert below:

```rust
chaturbate_login,
chaturbate_logout,
```

- [ ] **Step 5: Build**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean. If `chrono` `to_rfc3339` is missing, the `chrono` crate is already in the workspace (used by `auth/youtube.rs::write_netscape_file`); no Cargo.toml edits needed.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(chaturbate): IPC commands + auth_status field"
```

---

## Task 7: Frontend IPC wrappers + mock auth

**Files:**
- Modify: `src/ipc.js` (add invoke wrappers, extend mock state)

- [ ] **Step 1: Add the invoke wrappers**

Find the existing block in `src/ipc.js` (around lines 40–47):

```js
export const twitchLogin = () => invoke('twitch_login');
export const twitchLogout = () => invoke('twitch_logout');
export const kickLogin = () => invoke('kick_login');
export const kickLogout = () => invoke('kick_logout');
export const youtubeLogin = () => invoke('youtube_login');
export const youtubeLoginPaste = (text) => invoke('youtube_login_paste', { text });
export const youtubeLogout = () => invoke('youtube_logout');
export const youtubeDetectBrowsers = () => invoke('youtube_detect_browsers');
```

Below it, add:

```js
export const chaturbateLogin = () => invoke('chaturbate_login');
export const chaturbateLogout = () => invoke('chaturbate_logout');
```

- [ ] **Step 2: Extend the mock auth state**

In the same file, find the in-memory `mockAuth` initial value (search the file for `mockAuth = {`). It currently has shape:

```js
let mockAuth = {
  twitch: null,
  kick: null,
  // youtube state already mocked elsewhere
};
```

If `chaturbate` isn't already there, add `chaturbate: null` to the initial value. Then in the `switch` that handles invoke commands in browser mode (around the existing `case 'twitch_login':` block, ~line 220), add:

```js
case 'chaturbate_login':
  mockAuth = {
    ...mockAuth,
    chaturbate: { signed_in: true, last_verified_at: new Date().toISOString() },
  };
  return true;
case 'chaturbate_logout':
  mockAuth = { ...mockAuth, chaturbate: null };
  return undefined;
```

And in the mock `auth_status` case (search for `case 'auth_status'` in the same switch), include the chaturbate field in the returned object:

```js
case 'auth_status':
  return {
    twitch: mockAuth.twitch,
    kick: mockAuth.kick,
    youtube: { browser: null, has_paste: false },
    chaturbate: mockAuth.chaturbate
      ? mockAuth.chaturbate
      : { signed_in: false, last_verified_at: null },
  };
```

If the existing `auth_status` mock case doesn't already return a youtube field, add it too — match what the real backend returns to keep the React side honest.

- [ ] **Step 3: Smoke test in browser mode**

```bash
npm run dev
```

Open the dev URL in a browser (not Tauri). Open Preferences → Accounts. The page must render without console errors after this task even though the new buttons aren't wired yet — useAuth should already see the `chaturbate` field in the mock.

Stop the dev server (`Ctrl+C`).

- [ ] **Step 4: Commit**

```bash
git add src/ipc.js
git commit -m "feat(chaturbate): frontend invoke wrappers + mocks"
```

---

## Task 8: `useAuth` — state, actions, event subscription

**Files:**
- Modify: `src/hooks/useAuth.jsx`

- [ ] **Step 1: Update imports**

Replace the existing import block at the top of `src/hooks/useAuth.jsx`:

```jsx
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import {
  authStatus,
  chaturbateLogin,
  chaturbateLogout,
  kickLogin,
  kickLogout,
  twitchLogin,
  twitchLogout,
  youtubeLogin,
  youtubeLoginPaste,
  youtubeLogout,
} from '../ipc.js';
import { listenEvent } from '../ipc.js';
```

If `listenEvent` is exported from `../ipc.js` already, fold it into the same import block. (Project pattern: yes, both `invoke` wrappers and `listenEvent` come from `ipc.js` — see `useChat.js` for an example.)

- [ ] **Step 2: Extend initial state**

Find the `useState({ ... })` initialiser (around line 22) and add the chaturbate field:

```jsx
const [state, setState] = useState({
  loading: true,
  twitch: null,
  kick: null,
  youtube: { browser: null, has_paste: false },
  chaturbate: { signed_in: false, last_verified_at: null },
  error: null,
});
```

- [ ] **Step 3: Carry it through `refresh`**

In the existing `refresh` callback (around line 30):

```jsx
const refresh = useCallback(async () => {
  try {
    const data = await authStatus();
    setState({
      loading: false,
      twitch: data?.twitch ?? null,
      kick: data?.kick ?? null,
      youtube: data?.youtube ?? { browser: null, has_paste: false },
      chaturbate: data?.chaturbate ?? { signed_in: false, last_verified_at: null },
      error: null,
    });
  } catch (e) {
    setState((s) => ({ ...s, loading: false, error: String(e?.message ?? e) }));
  }
}, []);
```

- [ ] **Step 4: Add `'chaturbate'` cases to `login` and `logout`**

In `login` (around line 49):

```jsx
const login = useCallback(async (platform) => {
  try {
    if (platform === 'youtube') {
      await youtubeLogin();
      await refresh();
      return;
    }
    if (platform === 'chaturbate') {
      await chaturbateLogin();
      await refresh();
      return;
    }
    const id = platform === 'kick' ? await kickLogin() : await twitchLogin();
    setState((s) => ({ ...s, [platform]: id, error: null }));
  } catch (e) {
    setState((s) => ({ ...s, error: String(e?.message ?? e) }));
    throw e;
  }
}, [refresh]);
```

In `logout` (around line 64):

```jsx
const logout = useCallback(async (platform) => {
  try {
    if (platform === 'kick') await kickLogout();
    else if (platform === 'youtube') {
      await youtubeLogout();
      await refresh();
      return;
    } else if (platform === 'chaturbate') {
      await chaturbateLogout();
      await refresh();
      return;
    } else {
      await twitchLogout();
    }
    setState((s) => ({ ...s, [platform]: null, error: null }));
  } catch (e) {
    setState((s) => ({ ...s, error: String(e?.message ?? e) }));
    throw e;
  }
}, [refresh]);
```

- [ ] **Step 5: Subscribe to the auth event**

Below the existing `useEffect(() => { refresh(); }, [refresh]);` block, add:

```jsx
useEffect(() => {
  let unlisten = null;
  let cancelled = false;
  listenEvent('chat:auth:chaturbate', (payload) => {
    setState((s) => ({
      ...s,
      chaturbate: {
        ...s.chaturbate,
        signed_in: !!payload?.signed_in,
        // last_verified_at is owned by the stamp file; refresh on next
        // auth_status pull picks it up. Don't synthesise here.
      },
    }));
  })
    .then((u) => {
      if (cancelled) {
        u?.();
      } else {
        unlisten = u;
      }
    })
    .catch(() => {});
  return () => {
    cancelled = true;
    if (unlisten) unlisten();
  };
}, []);
```

- [ ] **Step 6: Smoke test**

```bash
npm run dev
```

Open the browser dev URL. In the JS console, type `localStorage` (just to confirm the page is alive). The Accounts tab should still render without errors.

Stop the dev server.

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useAuth.jsx
git commit -m "feat(chaturbate): useAuth state + actions + event sub"
```

---

## Task 9: `formatRelative` utility

**Files:**
- Modify: `src/utils/format.js`

A small helper that turns an RFC3339 timestamp into `"5m ago"`, `"3h ago"`, `"2d ago"`. Used by the Preferences row hint.

- [ ] **Step 1: Add the helper**

Append to `src/utils/format.js`:

```js
/**
 * Turn an RFC3339 timestamp (or anything Date.parse() handles) into a
 * coarse relative string: "just now", "5m ago", "3h ago", "2d ago".
 * Returns the original string on parse failure.
 */
export function formatRelative(ts) {
  if (!ts) return '';
  const ms = Date.parse(ts);
  if (Number.isNaN(ms)) return ts;
  const diff = Date.now() - ms;
  if (diff < 0 || diff < 30_000) return 'just now';
  const m = Math.floor(diff / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
}
```

- [ ] **Step 2: No automated test (file has no existing tests)**

The function is small and pure; manual verification in the Preferences row is enough.

- [ ] **Step 3: Commit**

```bash
git add src/utils/format.js
git commit -m "feat(format): formatRelative helper"
```

---

## Task 10: Preferences AccountsTab — Chaturbate row

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` (extend `AccountsTab`)

- [ ] **Step 1: Update imports**

At the top of `src/components/PreferencesDialog.jsx`, ensure `formatRelative` is imported:

```jsx
import { formatRelative } from '../utils/format.js';
```

(If `format.js` is already imported elsewhere in the file, add `formatRelative` to the existing import list.)

- [ ] **Step 2: Pull `chaturbate` from `useAuth`**

Find the `AccountsTab` body (around line 125):

```jsx
function AccountsTab() {
  const { twitch, kick, youtube, login, logout, loginYoutubePaste, refresh } = useAuth();
```

Update the destructure:

```jsx
function AccountsTab() {
  const { twitch, kick, youtube, chaturbate, login, logout, loginYoutubePaste, refresh } = useAuth();
  const [cbLoginRunning, setCbLoginRunning] = useState(false);
  const [cbError, setCbError] = useState(null);

  const runChaturbateLogin = async () => {
    setCbError(null);
    setCbLoginRunning(true);
    try {
      await login('chaturbate');
    } catch (e) {
      setCbError(String(e?.message ?? e));
    } finally {
      setCbLoginRunning(false);
    }
  };
```

(Keep the existing `ytLoginRunning`, `ytPasteOpen`, etc. state; just add the two `cb*` lines and the handler.)

- [ ] **Step 3: Add the row**

Find the existing YouTube row in `AccountsTab` (the `<Row label="YouTube" ...>` block, around line 212). After the closing `</Row>` of the YouTube block — and before any subsequent block (e.g. `<Row label="Import Twitch follows" …>`) — insert:

```jsx
<Row
  label="Chaturbate"
  hint={
    chaturbate?.signed_in
      ? `Signed in · verified ${formatRelative(chaturbate.last_verified_at)}`
      : 'Sign in to chat as yourself'
  }
>
  {chaturbate?.signed_in ? (
    <div style={{ display: 'flex', gap: 6 }}>
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={runChaturbateLogin}
        disabled={cbLoginRunning}
      >
        {cbLoginRunning ? 'Signing in…' : 'Sign in again'}
      </button>
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={() => logout('chaturbate')}
      >
        Log out
      </button>
    </div>
  ) : (
    <button
      type="button"
      className="rx-btn"
      onClick={runChaturbateLogin}
      disabled={cbLoginRunning}
    >
      {cbLoginRunning ? 'Waiting on Chaturbate…' : 'Sign in to Chaturbate'}
    </button>
  )}
  {cbError && (
    <div style={{ marginTop: 6, fontSize: 'var(--t-11)', color: 'var(--live)' }}>
      {cbError}
    </div>
  )}
</Row>
```

The `<Row>` component, `rx-btn` classes, and `--live` / `--t-11` tokens are all already defined; no new styles needed.

- [ ] **Step 4: Smoke test the UI**

```bash
npm run dev
```

Open Preferences → Accounts in the browser. Confirm:
- A "Chaturbate" row appears below the YouTube row.
- Hint reads *"Sign in to chat as yourself"*.
- Button reads *"Sign in to Chaturbate"*.

In the browser console, simulate sign-in by running:
```js
window.dispatchEvent(new CustomEvent('mock-cb-login'))
```

That won't actually do anything — quickest manual trick: stop the dev server, change the mock initial value of `mockAuth.chaturbate` in `ipc.js` to `{ signed_in: true, last_verified_at: new Date().toISOString() }`, restart, confirm the row flips to two buttons + the *"verified just now"* hint. Revert the change before committing.

- [ ] **Step 5: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(chaturbate): Preferences Accounts row"
```

---

## Task 11: ChaturbateAuthBanner component

**Files:**
- Create: `src/components/ChaturbateAuthBanner.jsx`

A compact banner that renders above the chat embed when the user is on a Chaturbate channel and not signed in.

- [ ] **Step 1: Create the file**

`src/components/ChaturbateAuthBanner.jsx`:

```jsx
import { useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';

/**
 * Renders inside ChatView's embed branch above <EmbeddedChat>. Shows a
 * thin banner when the user is on a Chaturbate channel and not signed in,
 * with a one-click recovery path. Returns null when signed in.
 */
export default function ChaturbateAuthBanner() {
  const { chaturbate, login } = useAuth();
  const [running, setRunning] = useState(false);
  const [error, setError] = useState(null);

  if (chaturbate?.signed_in) return null;

  const onSignIn = async () => {
    setError(null);
    setRunning(true);
    try {
      await login('chaturbate');
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div
      role="status"
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
        padding: '6px 10px',
        background: 'var(--zinc-900)',
        borderBottom: 'var(--hair)',
        fontSize: 'var(--t-11)',
        color: 'var(--zinc-300)',
      }}
    >
      <span>
        {error
          ? `Sign-in failed: ${error}`
          : 'Signed out of Chaturbate — chat is read-only.'}
      </span>
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={onSignIn}
        disabled={running}
      >
        {running ? 'Signing in…' : 'Sign in'}
      </button>
    </div>
  );
}
```

- [ ] **Step 2: No standalone smoke test**

Mount happens in Task 12; we'll verify there.

- [ ] **Step 3: Commit**

```bash
git add src/components/ChaturbateAuthBanner.jsx
git commit -m "feat(chaturbate): auth banner component"
```

---

## Task 12: Mount the banner in `ChatView`

**Files:**
- Modify: `src/components/ChatView.jsx` (~line 41–60)

- [ ] **Step 1: Import the banner**

Add to the existing imports at the top of `src/components/ChatView.jsx`:

```jsx
import ChaturbateAuthBanner from './ChaturbateAuthBanner.jsx';
```

- [ ] **Step 2: Render the banner inside the embed branch**

Find the existing branch (around line 41):

```jsx
const platform = channelKey?.split(':')[0];
if (platform === 'youtube' || platform === 'chaturbate') {
  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        overflow: 'hidden',
      }}
    >
      {header}
      <div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
        <EmbeddedChat
          channelKey={channelKey}
          isLive={isLive}
          placeholderText="Channel isn't live — chat will appear here when it goes live."
        />
      </div>
    </div>
  );
}
```

Insert the banner between `{header}` and the inner div, conditional on platform:

```jsx
const platform = channelKey?.split(':')[0];
if (platform === 'youtube' || platform === 'chaturbate') {
  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        overflow: 'hidden',
      }}
    >
      {header}
      {platform === 'chaturbate' && <ChaturbateAuthBanner />}
      <div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
        <EmbeddedChat
          channelKey={channelKey}
          isLive={isLive}
          placeholderText="Channel isn't live — chat will appear here when it goes live."
        />
      </div>
    </div>
  );
}
```

The banner returns null when signed-in, so the layout doesn't change in the happy path.

- [ ] **Step 3: Smoke test**

```bash
npm run dev
```

In the browser, add a Chaturbate channel (mock auto-detect handles `chaturbate.com/{slug}`), select it, confirm the banner appears above the chat-embed placeholder. Click *Sign in*; the mock flips to signed-in and the banner disappears.

Stop the dev server.

- [ ] **Step 4: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chaturbate): mount auth banner in ChatView"
```

---

## Task 13: End-to-end manual verification

**Files:** none (validation only)

The previous tasks each ran a focused smoke test in browser mode. This task runs the real Tauri build and walks the full happy + drift paths.

- [ ] **Step 1: Build & run the desktop app**

```bash
npm run tauri:dev
```

Wait for the app window to appear.

- [ ] **Step 2: Happy path**

1. Open the titlebar gear → Preferences → Accounts.
2. Confirm the **Chaturbate** row reads *"Sign in to chat as yourself"*.
3. Click *"Sign in to Chaturbate"*. A second window opens at `https://chaturbate.com/auth/login/`.
4. Sign in (or use an existing session). Within a few seconds of `sessionid` appearing, the popup closes on its own.
5. The row flips to *"Signed in · verified just now"* with **Sign in again** + **Log out**.
6. Add a known-live Chaturbate channel via the Add dialog (or `c:somelivechannel` quick-input).
7. Select that channel in the Command layout. The chat embed should load with the user authenticated (the *"Sign in to chat"* CB prompt does NOT appear; an input box does).
8. The banner does NOT appear above the embed.

- [ ] **Step 3: Drift path**

1. With the app still running and signed in, open `~/.local/share/livestreamlist/webviews/chaturbate/Cookies` (Chromium cookie store) and delete it (or `rm -rf` the whole `chaturbate` profile dir while no embed is active).
   - Easier alternative: in the active embed window, open dev tools (right-click → Inspect if enabled, or use Tauri's debug build), Application tab → Cookies → delete `sessionid`.
2. Switch off the channel and back on (or hit *R* refresh / re-mount).
3. On the next Chaturbate page-load-finished, the embed verifies and broadcasts `signed_in: false`.
4. The chat-pane banner appears: *"Signed out of Chaturbate — chat is read-only. [Sign in]"*.
5. The Preferences row updates to *"Sign in to chat as yourself"* (refresh the dialog if it's already open).
6. Click the banner's **Sign in** button → login flow runs → on success, banner disappears, embed reloads with auth.

- [ ] **Step 4: Logout path**

1. From Preferences → Accounts → **Log out** (Chaturbate row).
2. Stamp file disappears: `ls ~/.local/share/livestreamlist/chaturbate-auth.json` returns "No such file".
3. Profile dir is wiped: `ls ~/.local/share/livestreamlist/webviews/chaturbate/` empty (or directory absent).
4. Row resets to *"Sign in to chat as yourself"*.
5. Reopening the Chaturbate channel re-creates an empty profile dir; banner appears (no auth).

- [ ] **Step 5: Commit (no code changes; document the validation)**

If the verification surfaces any bugs, fix them in additional task entries; otherwise nothing to commit. The implementation tasks are done.

---

## Self-review notes (carried into the plan above)

- Spec coverage:
  - Goal / non-goals — Task plan honours both (no username detection, no paste, no follow-import).
  - Module surface (`load`, `save`, `clear`, `webview_profile_dir`, `touch_verified`, `login_via_webview`) — Tasks 1+2.
  - Stamp file format — Task 1, validated by round-trip test.
  - Profile-dir consolidation — Task 3.
  - Login / logout flow — Tasks 2 + 6.
  - Embed-side validation + event topic — Task 5.
  - IPC commands + AuthStatus extension — Task 6.
  - Frontend wrappers + mocks — Task 7.
  - useAuth state, actions, subscription — Task 8.
  - formatRelative helper — Task 9.
  - Preferences row — Task 10.
  - Chat-pane banner + mount — Tasks 11 + 12.
  - Manual testing matrix — Task 13.
- Type / name consistency: `ChaturbateAuth`, `ChaturbateAuthStatus`, `ChaturbateAuthEvent`, `chat:auth:chaturbate`, `chaturbate_login`, `chaturbate_logout` — used identically across tasks.
- No placeholders.
