# Sub-Anniversary PR 5 — Browser Cookie Auto-Scrape + Race Fix

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Achieve Qt parity by auto-scraping the Twitch web `auth-token` cookie from the user's browser at app launch (mirrors `gui/app.py:306-312`'s `extract_twitch_auth_token` flow). Plus fix the race-condition bug where the lazy `<TwitchWebConnectPrompt>` doesn't appear because the cookie-required event fires before the React listener attaches.

**Architecture:** New `auth::twitch_web::extract_from_browser()` using the [`rookie`](https://crates.io/crates/rookie) Rust crate (handles Firefox SQLite + Chromium encrypted SQLite + all major browsers in one API; mirrors how Tauri's YouTube auth uses yt-dlp `--cookies-from-browser`). Auto-scrape kicks off as a background task in `setup()` if OAuth is logged in but no web cookie cached. Race fix: change `twitch_anniversary_check` to return `{ info, cookieStatus }` synchronously instead of relying on event emission.

**Stacks on:** PRs 1-4 (all merged to main).

---

## Task 0: Add rookie dep + `extract_from_browser()` helper

**Files:**
- Modify `src-tauri/Cargo.toml` (add `rookie = "0.5"` or latest)
- Modify `src-tauri/src/auth/twitch_web.rs`

- [ ] **Step 1: Add rookie to Cargo.toml**

In `src-tauri/Cargo.toml` `[dependencies]`, add:
```toml
rookie = "0.5"
```
(If a newer version is on crates.io at execution time, prefer it.)

- [ ] **Step 2: Add `extract_from_browser` function**

Append to `src-tauri/src/auth/twitch_web.rs` (above `#[cfg(test)] mod tests`):

```rust
/// Try to scrape the Twitch web `auth-token` cookie from any installed
/// browser cookie database (Firefox SQLite + Chromium-family encrypted
/// SQLite + Safari + Edge + ...). Returns the cookie value if found
/// in any browser, None otherwise.
///
/// Mirrors the Qt app's `gui/youtube_login.py::extract_twitch_auth_token`
/// flow — gives users the cookie automatically without any login UI
/// when they're already signed into Twitch in their browser.
///
/// Returns None silently on any error (browser not detected, encrypted
/// store unreadable, no `.twitch.tv` cookies, etc.) — the lazy WebView
/// fallback in `login_via_webview` handles those cases.
pub fn extract_from_browser() -> Option<String> {
    let domains = vec!["twitch.tv".to_string()];
    let cookies = match rookie::load(Some(domains)) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("rookie load failed: {e}");
            return None;
        }
    };
    cookies
        .into_iter()
        .find(|c| c.name == "auth-token" && !c.value.is_empty())
        .map(|c| c.value)
}
```

(rookie's `Cookie` struct has `name`, `value`, `domain` fields — confirm at impl time. If the field names differ, adjust.)

- [ ] **Step 3: Verify**

```
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```
Both clean / 179 passing.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/auth/twitch_web.rs
git commit -m "feat(twitch-web): rookie-based browser cookie auto-scrape"
```

---

## Task 1: Auto-scrape on app launch

**Files:** Modify `src-tauri/src/lib.rs`.

In `setup()` (around line 1418-1485), spawn a background task: if Twitch OAuth identity is present AND no web cookie in keyring, try `extract_from_browser()` → `validate()` → `save_pair()`. Emit `twitch:web_status_changed` on success so the UI updates.

- [ ] **Step 1: Add the auto-scrape task**

In `src-tauri/src/lib.rs::setup()`, somewhere AFTER the `let chat_mgr = ...` block but BEFORE `Ok(())` (a good location is right after the existing YouTube cookie injection at line 1461-1463):

```rust
// Twitch web cookie auto-scrape (mirrors Qt's gui/app.py:306-312).
// If OAuth is logged in but no web cookie cached, try to scrape it
// from the user's browser. Async because validate() does a GQL ping.
if auth::twitch::stored_identity().is_some()
    && auth::twitch_web::stored_token().ok().flatten().is_none()
{
    let app_handle = app.handle().clone();
    let http_for_scrape = http_for_chat.clone();
    tauri::async_runtime::spawn(async move {
        let Some(token) = auth::twitch_web::extract_from_browser() else {
            log::debug!("twitch-web auto-scrape: no auth-token cookie found in any browser");
            return;
        };
        match auth::twitch_web::validate(&http_for_scrape, &token).await {
            Ok(identity) => {
                if let Err(e) = auth::twitch_web::save_pair(&token, &identity) {
                    log::warn!("twitch-web auto-scrape: save_pair failed: {e:#}");
                    return;
                }
                log::info!("twitch-web auto-scrape: captured cookie for @{}", identity.login);
                use tauri::Emitter;
                let _ = app_handle.emit(
                    "twitch:web_status_changed",
                    Some(identity),
                );
                broadcast_auth_changed(&app_handle);
            }
            Err(e) => {
                log::debug!("twitch-web auto-scrape: validate failed (cookie expired?): {e:#}");
            }
        }
    });
}
```

The references:
- `auth::twitch::stored_identity()` returns `Option<TwitchIdentity>` (PR 1)
- `auth::twitch_web::stored_token()` returns `Result<Option<String>>` (PR 1)
- `auth::twitch_web::extract_from_browser()` from Task 0
- `auth::twitch_web::validate()` from PR 1
- `auth::twitch_web::save_pair()` from PR 1 (note: it's `pub(crate)` — should be accessible from lib.rs since lib.rs is in the same crate)
- `broadcast_auth_changed()` exists in lib.rs already
- `tauri::Emitter` is the trait that provides `app.emit()` — the `use` line goes inside the closure to avoid a top-level import that would shadow the existing one (or hoist it; either is fine)

If `save_pair` is not crate-visible from lib.rs (sometimes `pub(crate)` and module boundaries surprise), make it `pub` instead — it's a deliberate API at this point.

- [ ] **Step 2: Verify**

```
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```
Clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/auth/twitch_web.rs
git commit -m "feat(twitch-web): auto-scrape cookie at app launch via rookie"
```

(If `save_pair` visibility was changed, the second file gets touched too — hence the add list.)

---

## Task 2: Race fix — `twitch_anniversary_check` returns cookie status

**Files:**
- Modify `src-tauri/src/platforms/twitch_anniversary.rs`
- Modify `src-tauri/src/lib.rs`
- Modify `src/hooks/useSubAnniversary.js`
- Modify `src/ipc.js` (mock)

The current `check()` returns `Option<SubAnniversaryInfo>` and emits the cookie-required event. The race: the React hook attaches the listener AFTER `twitch_anniversary_check` has already invoked the IPC, so the event fires before the listener exists. Fix: include the cookie status in the IPC response so the hook reads it synchronously.

- [ ] **Step 1: Add `CookieStatus` enum + `CheckResult` struct**

In `src-tauri/src/platforms/twitch_anniversary.rs`, near the top (after the constants):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieStatus {
    Ok,
    Missing,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub info: Option<SubAnniversaryInfo>,
    pub cookie_status: CookieStatus,
}
```

- [ ] **Step 2: Change `check()` signature + return type**

Change `pub async fn check(...) -> Option<SubAnniversaryInfo>` to `pub async fn check(...) -> CheckResult`. Adjust every return site:

- Cache hit returning Some → `CheckResult { info: Some(i), cookie_status: CookieStatus::Ok }`
- Cache hit returning None → `CheckResult { info: None, cookie_status: CookieStatus::Ok }` (we don't know from a cache hit whether the cookie is present, but if we cached a None it was probably "user not subbed" not "cookie missing" — the response status tells the truth)

Actually the cache wrinkle is subtle. A cached `None` could mean "user not subbed" OR "cookie was missing last time". The cache doesn't currently distinguish. Two options:
A. Store `(SubAnniversaryInfo, CookieStatus)` in cache — wider type, more accurate
B. On cache None hit, just return `CookieStatus::Ok` and trust that if the cookie is now missing the next *uncached* check will catch it

Go with **A** — the cache stores the cookie status alongside. Update the Cache:

```rust
pub struct Cache {
    inner: Mutex<HashMap<String, (Instant, CookieStatus, Option<SubAnniversaryInfo>)>>,
    ttl_some: Duration,
    ttl_none: Duration,
}

impl Cache {
    // ... existing constructors ...

    pub fn get(&self, channel_login: &str) -> Option<(CookieStatus, Option<SubAnniversaryInfo>)> {
        let inner = self.inner.lock();
        let (stored_at, status, value) = inner.get(channel_login)?;
        let ttl = if value.is_some() { self.ttl_some } else { self.ttl_none };
        if stored_at.elapsed() > ttl {
            return None;
        }
        Some((status.clone(), value.clone()))
    }

    pub fn set(&self, channel_login: &str, status: CookieStatus, value: Option<SubAnniversaryInfo>) {
        self.inner.lock().insert(
            channel_login.to_string(),
            (Instant::now(), status, value),
        );
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
    }
}
```

Update the cache tests to pass the status arg (use `CookieStatus::Ok` for the existing tests since they're testing TTL not status).

In `check()`, every `cache.set(channel_login, None)` becomes `cache.set(channel_login, CookieStatus::Missing or Expired, None)` depending on context, and `cache.set(channel_login, Some(info))` becomes `cache.set(channel_login, CookieStatus::Ok, Some(info))`.

Every `return None` becomes `return CheckResult { info: None, cookie_status: <appropriate> }`.

The return points in `check()`:
- Cookie missing → `CheckResult { info: None, cookie_status: CookieStatus::Missing }` (also still emits event for backward-compat with mid-session listeners)
- Network error → `CheckResult { info: None, cookie_status: CookieStatus::Ok }` (cookie was fine; just transient)
- 401/403 → `CheckResult { info: None, cookie_status: CookieStatus::Expired }`
- Other HTTP failure → `CheckResult { info: None, cookie_status: CookieStatus::Ok }`
- JSON parse failure → `CheckResult { info: None, cookie_status: CookieStatus::Ok }`
- `parse_response` returns None (not subbed) → `CheckResult { info: None, cookie_status: CookieStatus::Ok }`
- malformed renews_at → same
- `compute_window` returns None (window closed) → same
- success → `CheckResult { info: Some(info), cookie_status: CookieStatus::Ok }`

Cache hit returning the Some/None tuple from get → reconstruct CheckResult from it.

- [ ] **Step 3: Update IPC command return type**

In `src-tauri/src/lib.rs::twitch_anniversary_check`:

```rust
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

    // Dismissal check (only matters if Some — can't dismiss what's not there)
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
```

- [ ] **Step 4: Update React hook**

In `src/hooks/useSubAnniversary.js`:

Change the `refresh` function:

```js
const refresh = useCallback(async () => {
  if (!channelKey) {
    setInfo(null);
    infoRef.current = null;
    return;
  }
  try {
    const result = await twitchAnniversaryCheck(channelKey);
    // result is now { info, cookie_status: 'ok' | 'missing' | 'expired' }
    setInfo(result?.info ?? null);
    infoRef.current = result?.info ?? null;
    if ((result?.cookie_status === 'missing' || result?.cookie_status === 'expired')
        && !promptDismissedRef.current) {
      setConnectPromptVisible(true);
    }
  } catch (e) {
    setInfo(null);
    infoRef.current = null;
  }
}, [channelKey]);
```

The existing `twitch:web_cookie_required` event listener stays (it now serves only the mid-session cookie expiry case where the IPC was already in flight when the cookie expired).

- [ ] **Step 5: Update mock in `src/ipc.js`**

In the `mockInvoke` switch:

```js
    case 'twitch_anniversary_check':
      // Mock: return the new richer shape.
      return { info: null, cookie_status: 'ok' };
```

- [ ] **Step 6: Verify**

```
cargo test --manifest-path src-tauri/Cargo.toml   # cache tests will need updating
cargo check --manifest-path src-tauri/Cargo.toml
npm run build
```

The cache unit tests from PR 2 Task 3 will need updating to pass `CookieStatus::Ok` (or whatever) when calling `set()`. Adjust those tests as needed — the TTL semantics are unchanged, just the API takes one more arg.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/platforms/twitch_anniversary.rs src-tauri/src/lib.rs src/hooks/useSubAnniversary.js src/ipc.js
git commit -m "fix(sub-anniversary): return cookie status in check; eliminates listener race"
```

---

## Task 3: Docs + ship

- [ ] **Step 1: Update `CLAUDE.md` IPC table**

Update the `twitch_anniversary_check` row to reflect the new return type:

```
| `twitch_anniversary_check` | `uniqueKey` | `{ info: Option<SubAnniversaryInfo>, cookie_status: 'ok' \| 'missing' \| 'expired' }`. Cookie status read synchronously from response so React hook doesn't race the event emission |
```

- [ ] **Step 2: Update ROADMAP.md**

Add a sub-bullet under the umbrella sub-anniversary entry (which is already `[x]` from PR 4):

```
  - [x] PR 5: Browser cookie auto-scrape via `rookie` + race-fix (`extract_from_browser` at app launch + sync `cookie_status` in IPC response) — Qt parity, no manual login needed when user is already signed into Twitch in their browser (PR #N)
```

- [ ] **Step 3: Final verify + push + PR + merge**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run build
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: PR 5 — cookie auto-scrape + race fix"
git push -u origin feat/sub-anniversary-pr5-browser-scrape
gh pr create --title "Sub-anniversary PR 5 — browser cookie auto-scrape (Qt parity)" --body "..."
# After PR opens, replace #N + push fixup + gh pr merge --squash --delete-branch
```

---

## Self-review

- [x] Plan is short and focused.
- [x] Type/path consistency: `CheckResult`, `CookieStatus` used identically across Rust + IPC + React.
- [x] No placeholders.
- [x] Race fix is the actual fix; cache change is necessary side-effect of putting status in the response.
