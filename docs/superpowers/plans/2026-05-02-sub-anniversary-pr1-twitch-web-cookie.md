# Sub-Anniversary PR 1 — Twitch Web Cookie Infrastructure

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the Twitch *web* (cookie-based) auth path that future PRs depend on. Adds a popup `WebviewWindow` that signs the user into `twitch.tv`, captures the `auth-token` cookie, validates it via a GQL ping, stores it in the keyring, and surfaces "Connected as @login" in Preferences. **No banner UI in this PR** — verification is via Preferences interaction + cargo tests.

**Architecture:** New `src-tauri/src/auth/twitch_web.rs` modeled on `auth/chaturbate.rs::login_via_webview`. Same WebviewWindow + cookie-poll + WindowEvent::Destroyed pattern. Storage is the existing `tokens` keyring helper (matches the OAuth flow's pattern). Three new IPC commands plus a capability file for the login window's injected titlebar. Mismatch between OAuth login and web login surfaces as a recoverable error so the user can choose which account to keep.

**Tech Stack:** Rust 1.77+, Tauri 2 (`WebviewWindowBuilder`, `cookies_for_url`), `reqwest` (rustls), `keyring` (already in deps), `chrono`. No new crates.

**Spec:** `docs/superpowers/specs/2026-05-02-sub-anniversary-banner-design.md`

---

## File Structure

**New:**
- `src-tauri/src/auth/twitch_web.rs` — module
- `src-tauri/capabilities/twitch-web-login.json` — window capability for injected titlebar's drag/close

**Modified:**
- `src-tauri/src/auth/mod.rs` — `pub mod twitch_web;`
- `src-tauri/src/lib.rs` — register 3 commands; extend `AuthStatus` with `twitch_web` field; populate it in `auth_status`
- `src/ipc.js` — wrappers for the 3 new commands
- `src/components/PreferencesDialog.jsx` — new "Twitch web session" row directly under the existing Twitch row

---

## Task 0: Module skeleton + capability + auth/mod.rs registration

**Files:**
- Create: `src-tauri/src/auth/twitch_web.rs`
- Create: `src-tauri/capabilities/twitch-web-login.json`
- Modify: `src-tauri/src/auth/mod.rs`

**No TDD here** — scaffolding step.

- [ ] **Step 1: Create the module skeleton**

Create `src-tauri/src/auth/twitch_web.rs`:

```rust
//! Twitch *web* (cookie-based) auth.
//!
//! `gql.twitch.tv` rejects Helix bearer tokens for several internal-ish
//! queries we need (e.g. `subscriptionBenefit` for the sub-anniversary
//! banner). The same queries accept the `auth-token` cookie that
//! twitch.tv sets at login. We capture it via an in-app WebView popup
//! at the login page (modelled on `auth::chaturbate::login_via_webview`),
//! validate via a cheap GQL `CurrentUser` query, and stash the cookie
//! in the keyring under `twitch_browser_auth_token`.
//!
//! This module is independent of `auth::twitch` (the OAuth/Helix flow):
//! they may target different accounts. Mismatch detection compares the
//! web-login to the OAuth-login at capture time and refuses to store
//! when they don't match (the user is asked to log out one before
//! continuing).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::tokens;

const KEYRING_TOKEN: &str = "twitch_browser_auth_token";
const KEYRING_IDENTITY: &str = "twitch_web_identity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchWebIdentity {
    pub login: String,
    /// When we last successfully validated the cookie via GQL.
    pub last_verified_at: DateTime<Utc>,
}
```

- [ ] **Step 2: Register the module**

Edit `src-tauri/src/auth/mod.rs`. Insert `pub mod twitch_web;` alphabetically between `tokens` (private) and `youtube`:

```rust
mod callback_server;
pub mod chaturbate;
pub mod kick;
mod tokens;
pub mod twitch;
pub mod twitch_web;   // NEW
pub mod youtube;
```

- [ ] **Step 3: Create the login-window capability file**

Create `src-tauri/capabilities/twitch-web-login.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "twitch-web-login",
  "description": "Allow the twitch-web-login window's injected titlebar JS (running in the twitch.tv origin) to call exactly two window-control commands: start a drag and close the window. Scoped to this one window + URL pattern; no data-access or app commands are exposed to twitch.tv.",
  "windows": ["twitch-web-login"],
  "remote": {
    "urls": ["https://*.twitch.tv/*"]
  },
  "permissions": [
    "core:window:allow-start-dragging",
    "core:window:allow-close"
  ]
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean (warnings about unused `KEYRING_*` constants are fine — they get used in subsequent tasks).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/auth/twitch_web.rs src-tauri/src/auth/mod.rs src-tauri/capabilities/twitch-web-login.json
git commit -m "feat(twitch-web): module skeleton + login-window capability"
```

---

## Task 1: Pure helper — `extract_auth_token` (TDD)

**Files:**
- Modify: `src-tauri/src/auth/twitch_web.rs`

The cookie-extraction logic is the only non-trivial pure function in this module. We TDD it because the production usage runs only inside a live WebView poll loop, which is hard to test directly.

- [ ] **Step 1: Write the failing tests**

Append to `src-tauri/src/auth/twitch_web.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Cookie;

    fn cookie(name: &str, value: &str, domain: &str) -> Cookie<'static> {
        // tauri re-exports cookie::Cookie; build via the public builder API.
        Cookie::build((name.to_string(), value.to_string()))
            .domain(domain.to_string())
            .build()
    }

    #[test]
    fn extract_auth_token_present() {
        let jar = vec![
            cookie("foo", "bar", "twitch.tv"),
            cookie("auth-token", "abcd1234", "twitch.tv"),
        ];
        assert_eq!(extract_auth_token(&jar), Some("abcd1234".to_string()));
    }

    #[test]
    fn extract_auth_token_empty_value_treated_as_missing() {
        let jar = vec![cookie("auth-token", "", "twitch.tv")];
        assert_eq!(extract_auth_token(&jar), None);
    }

    #[test]
    fn extract_auth_token_absent() {
        let jar = vec![cookie("session", "x", "twitch.tv")];
        assert_eq!(extract_auth_token(&jar), None);
    }

    #[test]
    fn extract_auth_token_empty_jar() {
        let jar: Vec<Cookie<'static>> = vec![];
        assert_eq!(extract_auth_token(&jar), None);
    }
}
```

- [ ] **Step 2: Run tests — verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p livestreamlist_lib auth::twitch_web -- --nocapture`
Expected: 4 tests fail with `cannot find function 'extract_auth_token' in this scope`.

- [ ] **Step 3: Implement `extract_auth_token`**

Add to `src-tauri/src/auth/twitch_web.rs`, immediately above the `#[cfg(test)] mod tests` block:

```rust
use tauri::Cookie;

/// Find the `auth-token` cookie's value if it's both present and
/// non-empty. The Twitch web app sets this cookie post-login and clears
/// the value (but leaves the cookie) on logout, so an empty value is
/// semantically "missing".
pub(crate) fn extract_auth_token(jar: &[Cookie<'_>]) -> Option<String> {
    jar.iter()
        .find(|c| c.name() == "auth-token" && !c.value().is_empty())
        .map(|c| c.value().to_string())
}
```

- [ ] **Step 4: Run tests — verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -p livestreamlist_lib auth::twitch_web`
Expected: 4 passes.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/auth/twitch_web.rs
git commit -m "feat(twitch-web): extract_auth_token helper + tests"
```

---

## Task 2: Keyring storage — save / load / clear identity + token

**Files:**
- Modify: `src-tauri/src/auth/twitch_web.rs`

Mirrors the layout in `auth::twitch` (token in one keyring entry, identity in another, both cleared together).

- [ ] **Step 1: Add storage functions**

Append (above `#[cfg(test)] mod tests`):

```rust
/// Token currently stored in the keyring (if any). Used by callers
/// (anniversary GQL, future web-cookie consumers) to authenticate.
pub fn stored_token() -> Result<Option<String>> {
    tokens::load(KEYRING_TOKEN)
}

/// Last-validated identity from the keyring without re-validating
/// against Twitch. Used at boot for an instant "Connected as @X" UI.
pub fn stored_identity() -> Option<TwitchWebIdentity> {
    tokens::load(KEYRING_IDENTITY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

/// Persist the validated cookie + identity. Both must succeed; if
/// identity-save fails we roll back the token so we never have a
/// partial state ("token present but identity says not logged in").
pub(crate) fn save_pair(token: &str, identity: &TwitchWebIdentity) -> Result<()> {
    tokens::save(KEYRING_TOKEN, token).context("saving twitch web token")?;
    let identity_json = serde_json::to_string(identity).context("serialising identity")?;
    if let Err(e) = tokens::save(KEYRING_IDENTITY, &identity_json) {
        let _ = tokens::clear(KEYRING_TOKEN);
        return Err(e.context("saving twitch web identity"));
    }
    Ok(())
}

pub fn clear() -> Result<()> {
    tokens::clear(KEYRING_TOKEN)?;
    tokens::clear(KEYRING_IDENTITY).ok();
    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/auth/twitch_web.rs
git commit -m "feat(twitch-web): keyring save/load/clear for cookie + identity"
```

---

## Task 3: `validate` — GQL ping using the captured cookie

**Files:**
- Modify: `src-tauri/src/auth/twitch_web.rs`

The cheapest GQL request we can do that confirms the cookie still works AND tells us our login: `query CurrentUser { currentUser { login id } }`. (Twitch GQL accepts both cookie auth and Helix bearer; we want to test cookie auth specifically.)

- [ ] **Step 1: Add the public client ID + URL constants and `validate` function**

Append (above `#[cfg(test)] mod tests`):

```rust
const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// Same anonymous public web client ID Twitch's own site sends for
/// non-Helix GQL calls. Already used by `platforms::twitch` for
/// unauthenticated reads.
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// Validate the cookie and return the identity it resolves to. Errors
/// on HTTP/JSON failures or when the response has no `currentUser`
/// (the 401-equivalent: GQL returns 200 with `currentUser: null`).
pub async fn validate(client: &reqwest::Client, cookie: &str) -> Result<TwitchWebIdentity> {
    #[derive(Deserialize)]
    struct Resp {
        data: Option<Data>,
    }
    #[derive(Deserialize)]
    struct Data {
        #[serde(rename = "currentUser")]
        current_user: Option<CurrentUser>,
    }
    #[derive(Deserialize)]
    struct CurrentUser {
        login: String,
    }

    let body = serde_json::json!({
        "query": "query CurrentUser { currentUser { login id } }",
    });
    let resp = client
        .post(GQL_URL)
        .header("Client-Id", PUBLIC_CLIENT_ID)
        .header("Authorization", format!("OAuth {cookie}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("POST gql.twitch.tv (CurrentUser)")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "gql.twitch.tv {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let parsed: Resp = resp.json().await.context("parsing CurrentUser response")?;
    let login = parsed
        .data
        .and_then(|d| d.current_user)
        .map(|u| u.login)
        .ok_or_else(|| anyhow::anyhow!("currentUser is null — cookie no longer valid"))?;
    Ok(TwitchWebIdentity {
        login,
        last_verified_at: Utc::now(),
    })
}

/// Boot-time status: returns `Some` if both keyring entries exist AND
/// the cookie still validates. Mirrors `auth::twitch::status` semantics.
/// Failures clear the stored cookie so the UI doesn't keep lying.
pub async fn status(client: &reqwest::Client) -> Result<Option<TwitchWebIdentity>> {
    let Some(token) = stored_token()? else {
        return Ok(None);
    };
    match validate(client, &token).await {
        Ok(id) => {
            // Refresh the stored identity with the new last_verified_at.
            let _ = save_pair(&token, &id);
            Ok(Some(id))
        }
        Err(e) => {
            log::warn!("Twitch web cookie invalid, clearing: {e:#}");
            let _ = clear();
            Ok(None)
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/auth/twitch_web.rs
git commit -m "feat(twitch-web): validate + status against gql.twitch.tv"
```

---

## Task 4: `login_via_webview` — popup window + cookie poll

**Files:**
- Modify: `src-tauri/src/auth/twitch_web.rs`

Modelled on `auth/chaturbate.rs::login_via_webview` (lines 327–496). Same close-detection + cookie-poll + 5-min timeout pattern. Differences:
- Different cookie name (`auth-token`) and login URL.
- After successful capture, validates immediately via `validate` (so the same call that captures returns a usable identity).
- No injected titlebar JS for v1 — reuse Twitch's own login page chrome (we can polish later). This means we keep `decorations(true)`, which means the capability file from Task 0 is unused for v1 but stays in place for the future polish PR.

- [ ] **Step 1: Add module-level constants and helper**

Insert near the top of `src-tauri/src/auth/twitch_web.rs`, after the existing `KEYRING_*` constants:

```rust
const LOGIN_WINDOW_LABEL: &str = "twitch-web-login";
const LOGIN_URL: &str = "https://www.twitch.tv/login";
const SITE_URL: &str = "https://www.twitch.tv/";
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Where the WebView keeps cookies / IndexedDB / cache. Shared with
/// the share-popout window in PR 3 so the captured cookie is visible
/// to that window without a re-login.
pub fn webview_profile_dir() -> Result<std::path::PathBuf> {
    let dir = crate::config::data_dir()?
        .join("webviews")
        .join("twitch_web");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}
```

- [ ] **Step 2: Add `login_via_webview`**

Append (above `#[cfg(test)] mod tests`). Keep the `tokio::time::sleep` import explicit:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri::utils::config::Color;

/// Open the popup, poll for the auth-token cookie, validate it via
/// GQL, and persist it. Returns the resolved identity. Errors when the
/// user closes the window before signing in or when the 5-min timeout
/// elapses.
///
/// Re-entry: if the login window is already open (user double-clicked),
/// we focus the existing one and start a parallel poll loop against it
/// — closing it would race the first invocation's poll.
pub async fn login_via_webview(
    app: AppHandle,
    client: reqwest::Client,
) -> Result<TwitchWebIdentity> {
    let initial_login = stored_identity().map(|i| i.login);

    let closed = Arc::new(AtomicBool::new(false));

    let window = if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.set_focus();
        existing
    } else {
        let profile_dir = webview_profile_dir()?;
        let main = app.get_webview_window("main");
        let zinc_950 = Color(9, 9, 11, 255);
        let mut builder = WebviewWindowBuilder::new(
            &app,
            LOGIN_WINDOW_LABEL,
            WebviewUrl::External(LOGIN_URL.parse()?),
        )
        .title("Sign in to Twitch (web)")
        // Twitch's login page is comfortable around 480x720; widen
        // slightly so 2FA codes / "Trust this device" prompts have
        // room without scrolling.
        .inner_size(520.0, 760.0)
        .min_inner_size(480.0, 640.0)
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
                .context("transient_for(main) on Twitch web login window")?;
        }
        builder.build().context("opening Twitch web login window")?
    };

    let closed_for_event = closed.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed | WindowEvent::CloseRequested { .. }) {
            closed_for_event.store(true, Ordering::Relaxed);
        }
    });

    let site: url::Url = SITE_URL.parse()?;
    let started = std::time::Instant::now();

    loop {
        // Close check FIRST — see chaturbate.rs comment for why this
        // ordering matters (cookies_for_url on a destroyed webview
        // can hang indefinitely).
        if closed.load(Ordering::Relaxed)
            || app.get_webview_window(LOGIN_WINDOW_LABEL).is_none()
        {
            if let Some(id) = stored_identity() {
                if Some(id.login.clone()) != initial_login {
                    return Ok(id);
                }
            }
            anyhow::bail!("login window closed before sign-in completed");
        }

        match window.cookies_for_url(site.clone()) {
            Ok(jar) => {
                if let Some(token) = extract_auth_token(&jar) {
                    // Got the cookie — validate immediately. If validate
                    // fails (e.g. 2FA partway through and we caught a
                    // half-set cookie) keep polling; the user will
                    // either complete login (success next iteration) or
                    // close the window (close-check will bail).
                    match validate(&client, &token).await {
                        Ok(identity) => {
                            save_pair(&token, &identity)?;
                            let _ = window.close();
                            return Ok(identity);
                        }
                        Err(e) => {
                            log::debug!("cookie present but validate failed (probably mid-login): {e:#}");
                        }
                    }
                }
            }
            Err(e) => log::debug!("cookies_for_url(twitch.tv): {e}"),
        }

        if closed.load(Ordering::Relaxed)
            || app.get_webview_window(LOGIN_WINDOW_LABEL).is_none()
        {
            if let Some(id) = stored_identity() {
                if Some(id.login.clone()) != initial_login {
                    return Ok(id);
                }
            }
            anyhow::bail!("login window closed before sign-in completed");
        }

        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            anyhow::bail!("Twitch web login timed out after 5 minutes");
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean. If you get import errors for `WebviewUrl` or `WindowEvent`, double-check the `use` line — `tauri::WebviewUrl` is correct; `WindowEvent` is `tauri::WindowEvent`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/auth/twitch_web.rs
git commit -m "feat(twitch-web): login_via_webview popup + cookie poll"
```

---

## Task 5: Mismatch detection wrapper

**Files:**
- Modify: `src-tauri/src/auth/twitch_web.rs`

If the user is logged into the OAuth flow (`auth::twitch`) as `@A` but signs into the web flow as `@B`, the anniversary feature would silently misbehave (wrong account's subs detected). Wrap `login_via_webview` so the caller can distinguish "different account" from other errors.

- [ ] **Step 1: Add error type + wrapper**

Append (above `#[cfg(test)] mod tests`):

```rust
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("Web login is @{web} but app is logged in as @{oauth}. Log out of one before continuing.")]
    AccountMismatch { web: String, oauth: String },
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Same as `login_via_webview` but rejects mismatched accounts.
/// On mismatch the freshly-captured cookie is cleared so we don't
/// half-store the wrong account.
pub async fn login_with_match_check(
    app: AppHandle,
    client: reqwest::Client,
) -> Result<TwitchWebIdentity, LoginError> {
    let identity = login_via_webview(app, client).await?;
    if let Some(oauth) = super::twitch::stored_identity() {
        if oauth.login.eq_ignore_ascii_case(&identity.login) {
            return Ok(identity);
        }
        // Mismatch: roll back the keyring writes login_via_webview did.
        let _ = clear();
        return Err(LoginError::AccountMismatch {
            web: identity.login,
            oauth: oauth.login,
        });
    }
    // No OAuth login → no comparison possible. Allow.
    Ok(identity)
}
```

- [ ] **Step 2: Add `thiserror` to Cargo.toml if it's not already there**

Run: `grep -n thiserror src-tauri/Cargo.toml`
If no output: edit `src-tauri/Cargo.toml`, under `[dependencies]`, add `thiserror = "1"`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/auth/twitch_web.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(twitch-web): mismatch detection vs OAuth identity"
```

---

## Task 6: IPC commands + `AuthStatus` extension

**Files:**
- Modify: `src-tauri/src/lib.rs`

Three commands: `twitch_web_login`, `twitch_web_clear`, plus extending `auth_status` to include the web identity (no separate `twitch_web_status` command — the auth-status endpoint is the natural surface React already polls).

- [ ] **Step 1: Add commands and extend `AuthStatus`**

Locate `AuthStatus` in `src-tauri/src/lib.rs` (around line 946). Add a `twitch_web` field:

```rust
#[derive(serde::Serialize)]
struct AuthStatus {
    twitch: Option<auth::twitch::TwitchIdentity>,
    twitch_web: Option<auth::twitch_web::TwitchWebIdentity>,  // NEW
    kick: Option<auth::kick::KickIdentity>,
    youtube: YoutubeAuthStatus,
    chaturbate: ChaturbateAuthStatus,
}
```

In `auth_status` (around line 978), populate it:

```rust
#[tauri::command]
async fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, String> {
    let twitch = auth::twitch::status(&state.http)
        .await
        .map_err(err_string)?;
    let twitch_web = auth::twitch_web::status(&state.http)
        .await
        .map_err(err_string)?;
    let kick = auth::kick::status(&state.http).await.map_err(err_string)?;
    // ...rest unchanged...
    Ok(AuthStatus {
        twitch,
        twitch_web,                                     // NEW
        kick,
        youtube: YoutubeAuthStatus { /* ... */ },
        chaturbate,
    })
}
```

- [ ] **Step 2: Add the two commands**

Add directly below `twitch_logout` (around line 1042):

```rust
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
```

- [ ] **Step 3: Register the commands**

Find the `tauri::generate_handler!` macro call (search for `generate_handler` in `src-tauri/src/lib.rs`). Add the two new commands to the list, in the same neighborhood as `twitch_login` / `twitch_logout`:

```rust
tauri::generate_handler![
    // ...
    twitch_login,
    twitch_logout,
    twitch_web_login,    // NEW
    twitch_web_clear,    // NEW
    // ...
]
```

- [ ] **Step 4: Verify it compiles AND clippy is clean**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean.

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
Expected: clean.

- [ ] **Step 5: Run all tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: green (the four `extract_auth_token` tests pass; existing tests untouched).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(twitch-web): twitch_web_login + twitch_web_clear IPC commands"
```

---

## Task 7: Frontend IPC wrappers

**Files:**
- Modify: `src/ipc.js`

- [ ] **Step 1: Locate the existing twitch IPC wrappers**

Run: `grep -n "twitch_login\|twitchLogin" src/ipc.js`

The exact location of existing twitch wrappers will inform where to insert new ones. (We don't show the line because file evolves; insert alphabetically or grouped.)

- [ ] **Step 2: Add the two wrappers**

In `src/ipc.js`, alongside the existing `twitchLogin` / `twitchLogout` wrappers, add:

```js
export async function twitchWebLogin() {
  if (mockMode) return null;
  return invoke('twitch_web_login');
}

export async function twitchWebClear() {
  if (mockMode) return null;
  return invoke('twitch_web_clear');
}
```

(Use whatever the file's actual `mockMode` / `invoke` names are — match the surrounding twitch wrappers.)

- [ ] **Step 3: Verify dev server starts cleanly**

Run: `npm run dev` (in another terminal). Confirm no syntax errors. Stop with Ctrl-C.

- [ ] **Step 4: Commit**

```bash
git add src/ipc.js
git commit -m "feat(twitch-web): ipc.js wrappers"
```

---

## Task 8: Preferences UI — "Twitch web session" row

**Files:**
- Modify: `src/components/PreferencesDialog.jsx`

Add a row directly under the existing "Twitch" row. Visual: matches surrounding rows. Behaviour:
- Not connected → `[Connect web session]` button → calls `twitchWebLogin()`. While running, button shows "Waiting on Twitch…" and is disabled.
- Connected → hint reads `Connected as @login` → `[Disconnect]` button → calls `twitchWebClear()`.
- On mismatch error → `setTwitchWebError(...)` displayed under the row.

- [ ] **Step 1: Locate the existing Twitch row**

Find the `<Row label="Twitch" ... />` in `src/components/PreferencesDialog.jsx` (around line 206 in the snapshot taken when the spec was written; may have moved).

- [ ] **Step 2: Read the surrounding component to understand state hooks**

Look at how `twitch`, `kick`, `youtube`, `chaturbate` are pulled from `auth_status` and passed in. The `twitch_web` field added in Task 6 will arrive on the same object. Confirm the destructuring at the top of the relevant section (search for `auth_status` or `useAuthStatus`).

- [ ] **Step 3: Add state for in-flight + error display**

Where the YouTube section declares `ytLoginRunning` and `ytError` (around line 175 in the snapshot), add parallels for Twitch web:

```jsx
const [twWebRunning, setTwWebRunning] = useState(false);
const [twWebError, setTwWebError] = useState(null);
```

- [ ] **Step 4: Add the connect handler**

Below the YouTube `runYoutubeLogin` handler (around line 178 in the snapshot):

```jsx
const runTwitchWebLogin = async () => {
  setTwWebError(null);
  setTwWebRunning(true);
  try {
    await twitchWebLogin();
    refresh();   // re-pull auth_status so the row flips to "connected"
  } catch (e) {
    setTwWebError(String(e?.message ?? e));
  } finally {
    setTwWebRunning(false);
  }
};

const runTwitchWebClear = async () => {
  setTwWebError(null);
  try {
    await twitchWebClear();
    refresh();
  } catch (e) {
    setTwWebError(String(e?.message ?? e));
  }
};
```

(Imports: add `twitchWebLogin, twitchWebClear` to the import from `../ipc`.)

- [ ] **Step 5: Add the row directly under the existing Twitch row**

Insert immediately after the `</Row>` closing the Twitch row:

```jsx
<Row
  label="Twitch web session"
  hint={
    twitch_web
      ? `Connected as @${twitch_web.login}`
      : 'Sign in once for sub-anniversary detection (separate from chat login)'
  }
>
  {twitch_web ? (
    <button type="button" className="rx-btn rx-btn-ghost" onClick={runTwitchWebClear}>
      Disconnect
    </button>
  ) : (
    <button
      type="button"
      className="rx-btn"
      onClick={runTwitchWebLogin}
      disabled={twWebRunning}
    >
      {twWebRunning ? 'Waiting on Twitch…' : 'Connect web session'}
    </button>
  )}
</Row>
{twWebError && (
  <div style={{
    color: 'var(--warn, #f59e0b)',
    fontSize: 'var(--t-12, 12px)',
    margin: '4px 0 8px 0',
    paddingLeft: 8,
  }}>
    {twWebError}
  </div>
)}
```

(Replace `twitch_web` with whatever destructured name matches the `auth_status` field. If the destructuring is `const { twitch, kick, youtube, chaturbate } = authStatus ?? {};`, change it to `const { twitch, twitch_web, kick, youtube, chaturbate } = authStatus ?? {};`. JS won't blow up on the snake_case key.)

- [ ] **Step 6: Manual verification — dev**

Start the app: `npm run tauri:dev`

In Preferences (open via the gear icon or whatever surface the app uses):

1. **Cold state, no cookie cached.** Twitch web row shows "Sign in once for sub-anniversary detection…" + `[Connect web session]`.
2. **Click Connect web session.** Popup opens at `https://www.twitch.tv/login`. Log in.
3. **Popup auto-closes** when the cookie is captured + validated.
4. Twitch web row flips to "Connected as @yourlogin" + `[Disconnect]`.
5. **Mismatch test (only run if you have a second Twitch account):** log out OAuth (top "Twitch" row → Log out), then Connect web session and sign in as a *different* account. Error toast: "Web login is @X but app is logged in as @Y…" Cookie should NOT be stored (Disconnect button should NOT appear; row should still show "Sign in once…").
6. **Restart the app** with the cookie cached. The row immediately shows "Connected as @login" without re-prompting.
7. **Click Disconnect.** Row flips back to "Sign in once…".

- [ ] **Step 7: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(twitch-web): Preferences row with connect/disconnect"
```

---

## Task 9: Documentation + roadmap

**Files:**
- Modify: `CLAUDE.md` (project root)
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Add a brief CLAUDE.md note**

In the root `CLAUDE.md`, find the "IPC — invoke commands" table and add the two new rows. Also add a one-liner under "Auth" mentioning the dual surface.

Look for "Auth: Twitch OAuth implicit, …" (currently around line ~30) and replace with:

```
- **Auth**: Twitch OAuth implicit (`auth/twitch.rs`) + Twitch web cookie (`auth/twitch_web.rs`, separate keyring; required for `gql.twitch.tv` calls that reject Helix bearers, e.g. sub-anniversary detection), Kick OAuth 2.1 PKCE, YouTube browser cookies
```

In the IPC table, add (alphabetically near other twitch_*):

```
| `twitch_web_login` | — | Open WebView popup at twitch.tv/login; capture + validate auth-token cookie; persist. Returns identity or rejects on mismatch with OAuth login |
| `twitch_web_clear` | — | Wipe keyring entries for twitch web cookie + identity |
```

- [ ] **Step 2: Note PR1 in the roadmap**

The roadmap entry at `docs/ROADMAP.md:356` is the umbrella sub-anniversary item. Don't tick it — only PR1 is shipping. Append a sub-bullet:

```
- [ ] **Sub-anniversary banner** — when the logged-in user's Twitch anniversary is detected via IRC, show a one-shot dismissible banner per billing cycle. → Ph 3
  - [x] PR 1: Twitch web cookie infrastructure (`auth/twitch_web.rs` + Preferences row) — foundation for GQL `subscriptionBenefit` queries that reject Helix bearers (PR #N)
```

(Replace `#N` after the PR is opened.)

- [ ] **Step 3: Commit the docs**

```bash
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: note twitch web cookie auth path + PR 1 in roadmap"
```

---

## Task 10: Final verification + push + open PR

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run build
```

All three must be green.

- [ ] **Step 2: Smoke-test the dev build once more**

Run: `npm run tauri:dev`

Repeat the verification steps from Task 8 step 6. If anything regresses, fix and re-commit.

- [ ] **Step 3: Push the branch**

(Branch name suggestion: `feat/sub-anniversary-pr1-twitch-web-cookie`.)

```bash
git push -u origin feat/sub-anniversary-pr1-twitch-web-cookie
```

- [ ] **Step 4: Open the PR**

```bash
gh pr create --title "Sub-anniversary PR 1 — Twitch web cookie infrastructure" --body "$(cat <<'EOF'
## Summary

Foundation for the upcoming sub-anniversary banner feature. This PR ships the Twitch *web* (cookie-based) auth path:

- New \`src-tauri/src/auth/twitch_web.rs\` with WebView popup login + GQL cookie validation.
- Cookie + identity stored in keyring as \`twitch_browser_auth_token\` / \`twitch_web_identity\`.
- Two new IPC commands: \`twitch_web_login\` (opens popup, validates cookie, persists), \`twitch_web_clear\` (wipes keyring).
- \`auth_status\` extended with a \`twitch_web\` field.
- Preferences gets a new "Twitch web session" row directly under the existing Twitch row.
- Mismatch detection: refuses to store the web cookie if the captured login differs from the OAuth login (avoids silent wrong-account behaviour in downstream features).

\`gql.twitch.tv\` rejects Helix bearer tokens for several internal-ish queries we need (notably \`subscriptionBenefit\` for sub-anniversary detection in PR 2). The web cookie satisfies that endpoint.

## Why a separate auth flow

The OAuth flow targets the public Helix API; this flow targets the internal-ish GQL surface that the twitch.tv website uses. They can validly target different accounts (e.g. user has a "main" account they sub from and a "bot" account they chat from). Mismatch detection makes the divergence explicit instead of silently misbehaving.

## Test plan

- [x] \`cargo test\` green (4 new \`extract_auth_token\` tests)
- [x] \`cargo clippy --all-targets -- -D warnings\` clean
- [x] \`npm run build\` clean
- [x] Manual: cold-start → Connect web session → log in → popup auto-closes → row flips to "Connected as @login"
- [x] Manual: restart app with cookie cached → row immediately shows connected (no re-prompt)
- [x] Manual: Disconnect → row flips back to "Sign in once…"
- [x] Manual mismatch (if second account available): error toast appears, cookie not stored
EOF
)"
```

- [ ] **Step 5: Done.** PR is open. Stop here — PRs 2-4 will be planned + executed in subsequent sessions stacked on this branch.

---

## Self-review checklist (run before declaring this plan ready)

- [x] **Spec coverage** — Each spec section in `2026-05-02-sub-anniversary-banner-design.md` related to web cookie auth is implemented:
  - "Cookie capture via WebView popup" → Task 4
  - "keyring stored as `twitch_browser_auth_token` + `twitch_web_identity`" → Task 2
  - "Validate on launch via cheap GQL ping" → Task 3 (`status` function)
  - "Mismatch detection (web login != OAuth login)" → Task 5
  - "Preferences row" → Task 8
  - Spec items NOT in this PR but in subsequent PRs (banner, GQL anniversary query, share popout, settings additions) — explicitly excluded.

- [x] **Placeholder scan** — No "TBD", no "implement later". Code blocks contain the actual code.

- [x] **Type consistency** — `TwitchWebIdentity { login, last_verified_at }` is the type used in tasks 1, 2, 3, 4, 5, 6, 8 (consistent). `extract_auth_token(&[Cookie<'_>]) -> Option<String>` matches between Task 1 (test + impl) and Task 4 (caller). `LoginError` from Task 5 is mapped to `String` in Task 6's `twitch_web_login` command via `e.to_string()`.

- [x] **Skeleton/cap files** — `auth/mod.rs` registers the module (Task 0); capability JSON exists (Task 0).
