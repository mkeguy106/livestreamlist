# Accounts Panel Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Accounts tab content in the Preferences dialog with the card-based layout from `Accounts Panel.dc.html`, and add real YouTube subscriptions import.

**Architecture:** Frontend rewrite of `AccountsTab` into per-platform cards (identity row + import zone), reusing the existing dialog shell and existing Twitch/Chaturbate import IPC. One new Rust backend: `import_youtube_subscriptions`, which fetches the signed-in user's subscriptions via YouTube's InnerTube `browse` API (authenticated with stored Google cookies + a computed `SAPISIDHASH` header) and parses them with a pure, unit-tested function.

**Tech Stack:** Rust (Tauri 2, `reqwest`, `serde_json`, `sha1`), React 18, plain CSS variables.

## Global Constraints

- Never reference AI/Claude/automated generation in commit messages.
- Never use the native `title=""` attribute for hover text — wrap with `<Tooltip>` (`src/components/Tooltip.jsx`) and mirror with `aria-label`.
- Platform accent colors come from existing tokens: `--twitch #a78bfa`, `--youtube #f87171`, `--kick #4ade80`, `--cb #fb923c`; `--ok #22c55e` for connected status.
- Bulk-imported channels use `dont_notify: true` (monitoring list, not alert list).
- YouTube `channel_id` convention (must match `platforms::parse_channel_input`): handle without `@` from a `/@handle` canonical URL, else the `UC…` id from `/channel/UC…`.
- `ImportResult` shape is `{ added: u32, skipped: u32, total_seen: u32 }` (serialized fields `added`, `skipped`, `total_seen`).
- Rust ≥ 1.77. Use `tauri::async_runtime::spawn` (never raw `tokio::spawn`) for any setup-context background task (not needed in this plan, but the rule stands).

---

### Task 1: YouTube subscriptions parser (pure)

Pure function that walks an InnerTube `browse` (FEchannels) JSON response and extracts subscribed channels + the next continuation token. No network — fully unit-tested.

**Files:**
- Modify: `src-tauri/src/auth/youtube.rs` (add types + parser near the bottom, before `#[cfg(test)] mod tests`; add tests inside that module)

**Interfaces:**
- Produces:
  - `pub struct SubChannel { pub channel_id: String, pub title: String }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub struct SubsPage { pub channels: Vec<SubChannel>, pub continuation: Option<String> }` (derives `Debug, Default`)
  - `pub fn parse_subscriptions(v: &serde_json::Value) -> SubsPage`

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests { ... }` block in `src-tauri/src/auth/youtube.rs`:

```rust
    #[test]
    fn parse_subscriptions_extracts_handle_and_title() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"a":{"channelRenderer":{
                "channelId":"UCabc",
                "title":{"simpleText":"Ludwig"},
                "navigationEndpoint":{"browseEndpoint":{
                    "browseId":"UCabc","canonicalBaseUrl":"/@LudwigAhgren"}}}}}"#,
        )
        .unwrap();
        let page = parse_subscriptions(&v);
        assert_eq!(page.channels.len(), 1);
        assert_eq!(page.channels[0].channel_id, "LudwigAhgren");
        assert_eq!(page.channels[0].title, "Ludwig");
        assert_eq!(page.continuation, None);
    }

    #[test]
    fn parse_subscriptions_falls_back_to_channel_id_url() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"channelRenderer":{
                "channelId":"UCxyz",
                "title":{"runs":[{"text":"Some Channel"}]},
                "navigationEndpoint":{"browseEndpoint":{"canonicalBaseUrl":"/channel/UCxyz"}}}}"#,
        )
        .unwrap();
        let page = parse_subscriptions(&v);
        assert_eq!(page.channels.len(), 1);
        assert_eq!(page.channels[0].channel_id, "UCxyz");
        assert_eq!(page.channels[0].title, "Some Channel");
    }

    #[test]
    fn parse_subscriptions_reads_continuation_token() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"items":[
                {"continuationItemRenderer":{"continuationEndpoint":{
                    "continuationCommand":{"token":"TOKEN123"}}}}]}"#,
        )
        .unwrap();
        let page = parse_subscriptions(&v);
        assert!(page.channels.is_empty());
        assert_eq!(page.continuation.as_deref(), Some("TOKEN123"));
    }

    #[test]
    fn parse_subscriptions_dedupes_within_page() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"list":[
                {"channelRenderer":{"channelId":"UC1","title":{"simpleText":"A"},
                    "navigationEndpoint":{"browseEndpoint":{"canonicalBaseUrl":"/@a"}}}},
                {"channelRenderer":{"channelId":"UC1","title":{"simpleText":"A"},
                    "navigationEndpoint":{"browseEndpoint":{"canonicalBaseUrl":"/@a"}}}}]}"#,
        )
        .unwrap();
        let page = parse_subscriptions(&v);
        assert_eq!(page.channels.len(), 1);
        assert_eq!(page.channels[0].channel_id, "a");
    }

    #[test]
    fn parse_subscriptions_empty_and_garbage_are_safe() {
        let empty: serde_json::Value = serde_json::from_str("{}").unwrap();
        assert!(parse_subscriptions(&empty).channels.is_empty());
        let arr: serde_json::Value = serde_json::from_str("[1,2,3,\"x\"]").unwrap();
        let page = parse_subscriptions(&arr);
        assert!(page.channels.is_empty());
        assert_eq!(page.continuation, None);
    }

    #[test]
    fn parse_subscriptions_skips_renderer_without_id() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"channelRenderer":{"title":{"simpleText":"No Id"}}}"#,
        )
        .unwrap();
        assert!(parse_subscriptions(&v).channels.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml parse_subscriptions`
Expected: FAIL — `cannot find function parse_subscriptions in this scope`.

- [ ] **Step 3: Write the parser**

Add immediately above the `#[cfg(test)] mod tests {` line in `src-tauri/src/auth/youtube.rs`:

```rust
/// One subscribed channel from the YouTube subscriptions feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubChannel {
    /// Handle without `@` (from `/@handle`) or the `UC…` id (from
    /// `/channel/UC…`) — matches `platforms::parse_channel_input` so imported
    /// channels dedupe against manually-added ones.
    pub channel_id: String,
    pub title: String,
}

/// One page of the InnerTube `browse` (FEchannels) response.
#[derive(Debug, Default)]
pub struct SubsPage {
    pub channels: Vec<SubChannel>,
    pub continuation: Option<String>,
}

/// Recursively walk an InnerTube `browse` (FEchannels) response, collecting
/// every `channelRenderer` and the next continuation token. Walking the tree
/// rather than indexing fixed paths keeps this robust to YouTube's frequent
/// layout churn. Pure — unit-tested.
pub fn parse_subscriptions(v: &serde_json::Value) -> SubsPage {
    let mut page = SubsPage::default();
    let mut seen = std::collections::HashSet::new();
    walk_subs(v, &mut page, &mut seen);
    page
}

fn walk_subs(
    v: &serde_json::Value,
    page: &mut SubsPage,
    seen: &mut std::collections::HashSet<String>,
) {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(renderer) = map.get("channelRenderer") {
                if let Some(ch) = extract_channel(renderer) {
                    if seen.insert(ch.channel_id.clone()) {
                        page.channels.push(ch);
                    }
                }
            }
            if let Some(cmd) = map.get("continuationCommand") {
                if let Some(tok) = cmd.get("token").and_then(|t| t.as_str()) {
                    if !tok.is_empty() {
                        page.continuation = Some(tok.to_string());
                    }
                }
            }
            for child in map.values() {
                walk_subs(child, page, seen);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr {
                walk_subs(child, page, seen);
            }
        }
        _ => {}
    }
}

fn extract_channel(r: &serde_json::Value) -> Option<SubChannel> {
    let title = r
        .get("title")
        .and_then(|t| {
            t.get("simpleText")
                .and_then(|s| s.as_str())
                .map(String::from)
                .or_else(|| {
                    t.get("runs")
                        .and_then(|runs| runs.get(0))
                        .and_then(|r0| r0.get("text"))
                        .and_then(|s| s.as_str())
                        .map(String::from)
                })
        })
        .unwrap_or_default();

    let canonical = r
        .get("navigationEndpoint")
        .and_then(|n| n.get("browseEndpoint"))
        .and_then(|b| b.get("canonicalBaseUrl"))
        .and_then(|s| s.as_str());

    let channel_id = match canonical {
        Some(url) if url.starts_with("/@") => url.trim_start_matches("/@").to_string(),
        Some(url) if url.starts_with("/channel/") => {
            url.trim_start_matches("/channel/").to_string()
        }
        _ => r
            .get("channelId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
    };

    if channel_id.is_empty() {
        return None;
    }
    Some(SubChannel { channel_id, title })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml parse_subscriptions`
Expected: PASS — 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/auth/youtube.rs
git commit -m "feat(youtube): pure parser for InnerTube subscriptions feed"
```

---

### Task 2: YouTube subscriptions fetch + import command

Wire the parser to the network: compute the `SAPISIDHASH` auth header, page through InnerTube `browse`, and expose an `import_youtube_subscriptions` IPC command that adds the channels to the store.

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `sha1`)
- Modify: `src-tauri/src/auth/youtube.rs` (add `fetch_subscriptions` + `sapisid_hash`)
- Modify: `src-tauri/src/lib.rs` (add command + register in `generate_handler!`)
- Modify: `src/ipc.js` (export + mock)

**Interfaces:**
- Consumes: `parse_subscriptions`, `SubChannel`, `YouTubeCookies::entries`, `load` (Task 1 + existing `auth/youtube.rs`); `add_imported_channels`, `ImportResult`, `AppState` (existing `lib.rs`).
- Produces:
  - `pub async fn fetch_subscriptions(http: &reqwest::Client) -> anyhow::Result<Vec<SubChannel>>`
  - IPC command `import_youtube_subscriptions` → `ImportResult`
  - `ipc.js`: `export const importYoutubeSubscriptions = () => invoke('import_youtube_subscriptions')`

- [ ] **Step 1: Add the `sha1` dependency**

In `src-tauri/Cargo.toml`, directly below the existing `sha2 = "0.10"` line:

```toml
sha1 = "0.10"
```

- [ ] **Step 2: Add the fetch + SAPISIDHASH helper**

In `src-tauri/src/auth/youtube.rs`, add these constants near the other `const` declarations at the top of the file (after `const POLL_INTERVAL`):

```rust
/// Public InnerTube web key (ships in every youtube.com page; not secret).
const INNERTUBE_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
/// InnerTube web client version. YouTube is lenient about the exact value.
const INNERTUBE_CLIENT_VERSION: &str = "2.20240101.00.00";
const SUBS_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36";
/// Safety cap so a malformed continuation loop can't page forever.
const SUBS_MAX_PAGES: usize = 50;
```

Add this function above the `parse_subscriptions` definition added in Task 1:

```rust
/// Compute the `Authorization: SAPISIDHASH …` header YouTube's authenticated
/// InnerTube endpoints require: `SHA1("{ts} {SAPISID} {origin}")`.
fn sapisid_hash(sapisid: &str) -> String {
    use sha1::{Digest, Sha1};
    let ts = chrono::Utc::now().timestamp();
    let origin = "https://www.youtube.com";
    let digest = Sha1::digest(format!("{ts} {sapisid} {origin}").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("SAPISIDHASH {ts}_{hex}")
}

/// Fetch the signed-in user's subscriptions via InnerTube `browse`
/// (`browseId: FEchannels`), paging through continuation tokens. Requires
/// keyring cookies (in-app Google sign-in or pasted cookies) — returns a
/// clear error when none are stored.
pub async fn fetch_subscriptions(http: &reqwest::Client) -> Result<Vec<SubChannel>> {
    let cookies = load()?.ok_or_else(|| {
        anyhow!("Sign in to YouTube (or paste cookies) to import your subscriptions")
    })?;
    let cookie_header = cookies
        .entries()
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let auth = sapisid_hash(&cookies.sapisid);

    let context = serde_json::json!({
        "client": {
            "clientName": "WEB",
            "clientVersion": INNERTUBE_CLIENT_VERSION,
            "hl": "en",
            "gl": "US"
        }
    });

    let url =
        format!("https://www.youtube.com/youtubei/v1/browse?key={INNERTUBE_KEY}&prettyPrint=false");
    let mut body = serde_json::json!({ "context": context, "browseId": "FEchannels" });

    let mut all: Vec<SubChannel> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for _ in 0..SUBS_MAX_PAGES {
        let resp = http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Cookie", &cookie_header)
            .header("Authorization", &auth)
            .header("Origin", "https://www.youtube.com")
            .header("X-Origin", "https://www.youtube.com")
            .header("User-Agent", SUBS_UA)
            .json(&body)
            .send()
            .await
            .context("POST youtubei/v1/browse")?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "YouTube subscriptions request failed: HTTP {}",
                resp.status()
            );
        }
        let v: serde_json::Value = resp.json().await.context("parsing browse response")?;
        let page = parse_subscriptions(&v);
        for ch in page.channels {
            if seen.insert(ch.channel_id.clone()) {
                all.push(ch);
            }
        }
        match page.continuation {
            Some(token) => {
                body = serde_json::json!({ "context": context, "continuation": token });
            }
            None => break,
        }
    }

    log::info!("YouTube import: scraped {} subscription(s)", all.len());
    Ok(all)
}
```

Note: `anyhow!`, `Context`, `Result` are already imported at the top of this file (`use anyhow::{anyhow, Context, Result};`).

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: compiles (warnings about unused `fetch_subscriptions` are fine until Step 4 wires it).

- [ ] **Step 4: Add the import command**

In `src-tauri/src/lib.rs`, add this command immediately after the `import_twitch_follows` function (ends at line ~344, before the `import_chaturbate_follows` doc comment):

```rust
#[tauri::command]
async fn import_youtube_subscriptions(state: State<'_, AppState>) -> Result<ImportResult, String> {
    let subs = auth::youtube::fetch_subscriptions(&state.http)
        .await
        .map_err(err_string)?;

    let channels = subs
        .into_iter()
        .map(|s| Channel {
            platform: Platform::Youtube,
            channel_id: s.channel_id.clone(),
            display_name: if s.title.is_empty() {
                s.channel_id
            } else {
                s.title
            },
            favorite: false,
            // Bulk import = monitoring list, not an alert list — default to no
            // go-live notification so importing a large subscription list
            // doesn't flood the desktop. Re-enable per channel.
            dont_notify: true,
            auto_play: false,
            added_at: Some(Utc::now()),
        })
        .collect();

    Ok(add_imported_channels(&state.store, channels))
}
```

- [ ] **Step 5: Register the command**

In `src-tauri/src/lib.rs`, in the `tauri::generate_handler![ … ]` list, add a line directly below `$crate::import_twitch_follows,`:

```rust
            $crate::import_youtube_subscriptions,
```

- [ ] **Step 6: Add the ipc.js binding + mock**

In `src/ipc.js`, add directly below the `importTwitchFollows` export (line ~72):

```js
export const importYoutubeSubscriptions = () => invoke('import_youtube_subscriptions');
```

And in the mock `switch` (directly below the `case 'import_twitch_follows':` block, line ~323):

```js
    case 'import_youtube_subscriptions':
      return { added: 0, skipped: 0, total_seen: 0 };
```

- [ ] **Step 7: Verify build**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all existing + Task 1 tests; no new tests here).
Run: `npm run build`
Expected: succeeds.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/auth/youtube.rs src-tauri/src/lib.rs src/ipc.js
git commit -m "feat(youtube): import subscriptions via InnerTube browse"
```

---

### Task 3: Accounts tab card redesign (frontend)

Rewrite `AccountsTab` into the card layout: header with "Import all follows", one `PlatformCard` per platform, import zones with idle/running/done states, the YouTube sign-in disclosure, the Chaturbate sign-in-again affordance, and a slim Twitch-web-session row. Add the keyframes and the connected-count badge on the Accounts nav item.

**Files:**
- Modify: `src/tokens.css` (add `acc-pop` + `acc-indeterminate` keyframes)
- Modify: `src/components/PreferencesDialog.jsx` (import binding; nav badge; rewrite `AccountsTab`; add `PlatformCard`, `ImportControl`, helpers)

**Interfaces:**
- Consumes: `useAuth()` (`twitch, twitch_web, kick, youtube, chaturbate, login, logout, loginYoutubePaste, refresh`), `usePreferences()`, `importTwitchFollows`, `importYoutubeSubscriptions` (Task 2), `importChaturbateFollows`, `twitchWebLogin`, `twitchWebClear`, `youtubeDetectBrowsers`, `formatRelative`, `Tooltip`, `YoutubePasteDialog` (existing in same file).
- Produces: module-level `PLATFORMS`, `IMPORT_RUNNERS`, `platformConnected(id, auth)`, `importCapable(id, auth)`; components `PlatformCard`, `ImportControl`.

- [ ] **Step 1: Add keyframes to tokens.css**

In `src/tokens.css`, directly after the existing `@keyframes rx-spin …` line (line ~100):

```css
@keyframes acc-pop { from { opacity: 0; transform: translateY(3px); } to { opacity: 1; transform: none; } }
@keyframes acc-indeterminate { 0% { left: -40%; } 100% { left: 100%; } }
```

- [ ] **Step 2: Add the new import binding**

In `src/components/PreferencesDialog.jsx`, add `importYoutubeSubscriptions,` to the import block from `../ipc.js` (lines 7-16), alphabetically near `importTwitchFollows`:

```js
import {
  importTwitchFollows,
  importYoutubeSubscriptions,
  importChaturbateFollows,
  listBlockedUsers,
  setUserMetadata,
  spellcheckListDicts,
  twitchWebClear,
  twitchWebLogin,
  youtubeDetectBrowsers,
} from '../ipc.js';
```

- [ ] **Step 3: Add module-level config + helpers**

In `src/components/PreferencesDialog.jsx`, directly below the `TABS` array (after line 23):

```js
const PLATFORMS = [
  {
    id: 'twitch', name: 'Twitch', letter: 'T', tag: 'TTV',
    accent: 'var(--twitch)', monoBg: 'rgba(167,139,250,.12)', monoBorder: 'rgba(167,139,250,.22)',
    importTitle: 'Import follows',
    importDesc: 'Adds every channel you follow on Twitch. Existing entries are skipped.',
  },
  {
    id: 'youtube', name: 'YouTube', letter: 'Y', tag: 'YT',
    accent: 'var(--youtube)', monoBg: 'rgba(248,113,113,.12)', monoBorder: 'rgba(248,113,113,.22)',
    importTitle: 'Import subscriptions',
    importDesc: 'Adds every channel you’re subscribed to on YouTube. Existing entries are skipped.',
  },
  {
    id: 'kick', name: 'Kick', letter: 'K', tag: 'KICK',
    accent: 'var(--kick)', monoBg: 'rgba(74,222,128,.12)', monoBorder: 'rgba(74,222,128,.22)',
    importTitle: 'Import follows', importDesc: '',
  },
  {
    id: 'chaturbate', name: 'Chaturbate', letter: 'C', tag: 'CB',
    accent: 'var(--cb)', monoBg: 'rgba(251,146,60,.12)', monoBorder: 'rgba(251,146,60,.22)',
    importTitle: 'Import follows',
    importDesc: 'Adds every model you follow on Chaturbate. Existing entries are skipped.',
  },
];

const IMPORT_RUNNERS = {
  twitch: importTwitchFollows,
  youtube: importYoutubeSubscriptions,
  chaturbate: importChaturbateFollows,
};

function platformConnected(id, auth) {
  switch (id) {
    case 'twitch': return !!auth.twitch;
    case 'youtube': return !!(auth.youtube?.has_paste || auth.youtube?.browser);
    case 'kick': return !!auth.kick;
    case 'chaturbate': return !!auth.chaturbate?.signed_in;
    default: return false;
  }
}

// Import is possible only when connected AND we actually have a working path.
// Kick has no follows API; YouTube needs keyring cookies (not browser-cookie).
function importCapable(id, auth) {
  switch (id) {
    case 'twitch': return !!auth.twitch;
    case 'youtube': return !!auth.youtube?.has_paste;
    case 'chaturbate': return !!auth.chaturbate?.signed_in;
    default: return false;
  }
}
```

- [ ] **Step 4: Add the connected-count badge on the Accounts nav item**

In `src/components/PreferencesDialog.jsx`, the top-level `PreferencesDialog` component (line 25) needs auth. Add directly below `const { settings, error, patch } = usePreferences();` (line 27):

```js
  const auth = useAuth();
  const connectedCount = PLATFORMS.filter((p) => platformConnected(p.id, auth)).length;
```

Then replace the `{TABS.map((t) => ( … ))}` block (lines 78-97) with this version, which renders the badge on the Accounts item:

```jsx
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id)}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
                textAlign: 'left',
                padding: '6px 10px',
                border: 'none',
                borderRadius: 4,
                background: t.id === tab ? 'var(--zinc-900)' : 'transparent',
                color: t.id === tab ? 'var(--zinc-100)' : 'var(--zinc-400)',
                fontSize: 'var(--t-12)',
                fontFamily: 'inherit',
                cursor: 'pointer',
              }}
            >
              <span>{t.label}</span>
              {t.id === 'accounts' && (
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                  {connectedCount}/4
                </span>
              )}
            </button>
          ))}
```

- [ ] **Step 5: Replace the AccountsTab body**

In `src/components/PreferencesDialog.jsx`, replace the entire `AccountsTab` function (the `return (…)` JSX block, lines 243-524 — from `return (` through the closing `);` and the function's final `}`) with the markup below. Keep the state hooks and handlers at the top of `AccountsTab` (lines 132-241) intact **except** remove the now-unused `importState`/`cbImportState` hooks and the `runImport`/`runCbImport` handlers (replaced by the unified `imports` state below). Concretely:

1. Delete these lines from the top of `AccountsTab`:
   - `const [importState, setImportState] = useState(null); // {running, result, error}` (line 135)
   - `const [cbImportState, setCbImportState] = useState(null); // {running, result, error}` (line 136)
   - the `runImport` function (lines 196-204)
   - the `runCbImport` function (lines 206-214)

2. Add this unified import state + handlers directly below the remaining `useState` hooks at the top of `AccountsTab` (e.g. after the `browsers` state, line ~180):

```js
  const auth = useAuth();
  const [imports, setImports] = useState({}); // id -> { status, result, error }

  const runImport = useCallback(async (id) => {
    const runner = IMPORT_RUNNERS[id];
    if (!runner) return;
    setImports((s) => ({ ...s, [id]: { status: 'running' } }));
    try {
      const result = await runner();
      setImports((s) => ({ ...s, [id]: { status: 'done', result } }));
    } catch (e) {
      setImports((s) => ({ ...s, [id]: { status: 'error', error: String(e?.message ?? e) } }));
    }
  }, []);

  const anyImportRunning = Object.values(imports).some((x) => x?.status === 'running');

  const importAll = useCallback(() => {
    for (const p of PLATFORMS) {
      if (importCapable(p.id, auth) && imports[p.id]?.status !== 'running') {
        runImport(p.id);
      }
    }
  }, [auth, imports, runImport]);

  const anyCapable = PLATFORMS.some((p) => importCapable(p.id, auth));
```

   Note: `AccountsTab`'s existing first line already destructures `useAuth()` fields (line 133). Keep that line, and ALSO keep the `const auth = useAuth();` added above — both calls return the same context value; `auth` is used for the helper checks while the destructured names are used by the existing handlers. (If preferred, replace the destructure usages with `auth.twitch` etc., but that is optional cleanup, not required.)

3. Replace the `return ( … );` block with:

```jsx
  const detailFor = (id) => {
    switch (id) {
      case 'twitch':
        return auth.twitch ? `@${auth.twitch.login}` : 'Not logged in';
      case 'youtube':
        return ytBrowser
          ? `Using cookies from ${ytLabelFor(ytBrowser)}`
          : auth.youtube?.has_paste
          ? 'Signed in via Google'
          : 'Not signed in';
      case 'kick':
        return auth.kick ? `@${auth.kick.login}` : 'Not logged in';
      case 'chaturbate':
        return auth.chaturbate?.signed_in
          ? `Signed in · verified ${formatRelative(auth.chaturbate.last_verified_at)}`
          : 'Not signed in';
      default:
        return '';
    }
  };

  const authButtonFor = (id) => {
    const connected = platformConnected(id, auth);
    switch (id) {
      case 'twitch':
        return connected ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('twitch')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={() => login('twitch')}>
            Connect
          </button>
        );
      case 'youtube':
        return connected ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('youtube')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={runYoutubeLogin} disabled={ytLoginRunning}>
            {ytLoginRunning ? 'Waiting on Google…' : 'Connect'}
          </button>
        );
      case 'kick':
        return connected ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('kick')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={() => login('kick')}>
            Connect
          </button>
        );
      case 'chaturbate':
        return connected ? (
          <div style={{ display: 'flex', gap: 6 }}>
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={runChaturbateLogin}
              disabled={cbLoginRunning}
            >
              {cbLoginRunning ? 'Signing in…' : 'Sign in again'}
            </button>
            <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('chaturbate')}>
              Log out
            </button>
          </div>
        ) : (
          <button type="button" className="rx-btn" onClick={runChaturbateLogin} disabled={cbLoginRunning}>
            {cbLoginRunning ? 'Waiting on Chaturbate…' : 'Connect'}
          </button>
        );
      default:
        return null;
    }
  };

  const importZoneFor = (p) => {
    const connected = platformConnected(p.id, auth);
    if (p.id === 'kick') {
      return <ImportNote>Kick doesn’t expose your follows to apps yet.</ImportNote>;
    }
    if (!connected) {
      return <ImportNote>Connect {p.name} to import the channels you follow.</ImportNote>;
    }
    if (p.id === 'youtube' && !auth.youtube?.has_paste) {
      return (
        <ImportNote>Sign in with Google or paste cookies to enable subscription import.</ImportNote>
      );
    }
    return (
      <ImportControl
        title={p.importTitle}
        desc={p.importDesc}
        accent={p.accent}
        state={imports[p.id]}
        onRun={() => runImport(p.id)}
      />
    );
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, margin: '-20px -24px', height: 'calc(100% + 40px)' }}>
      {/* Header */}
      <div
        style={{
          flexShrink: 0,
          padding: '4px 24px 14px',
          borderBottom: 'var(--hair)',
          display: 'flex',
          alignItems: 'flex-end',
          justifyContent: 'space-between',
          gap: 16,
        }}
      >
        <div>
          <div style={{ fontSize: 'var(--t-16)', fontWeight: 600, color: 'var(--zinc-100)', letterSpacing: '-.01em' }}>
            Accounts
          </div>
          <div style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-500)', marginTop: 3 }}>
            Connect a platform, then pull in everyone you already follow.
          </div>
        </div>
        <Tooltip text={anyCapable ? 'Import follows from every connected platform' : 'Connect a platform first'}>
          <button
            type="button"
            className="rx-btn"
            aria-label="Import all follows"
            onClick={importAll}
            disabled={!anyCapable || anyImportRunning}
            style={{ flexShrink: 0 }}
          >
            <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--ok)' }} />
            Import all follows
          </button>
        </Tooltip>
      </div>

      {/* Cards */}
      <div style={{ flex: 1, overflow: 'auto', padding: '0 24px 22px', display: 'flex', flexDirection: 'column', gap: 12 }}>
        {PLATFORMS.map((p) => (
          <PlatformCard
            key={p.id}
            cfg={p}
            connected={platformConnected(p.id, auth)}
            detail={detailFor(p.id)}
            authButton={authButtonFor(p.id)}
            importZone={importZoneFor(p)}
            error={p.id === 'chaturbate' ? cbError : null}
            disclosure={
              p.id === 'youtube' && !platformConnected('youtube', auth) ? (
                <YoutubeSignInExtras
                  browsers={browsers}
                  ytBrowser={ytBrowser}
                  setYtBrowser={setYtBrowser}
                  ytAdvanced={ytAdvanced}
                  setYtAdvanced={setYtAdvanced}
                  onPaste={() => setYtPasteOpen(true)}
                  ytError={ytError}
                />
              ) : null
            }
          />
        ))}

        {/* Twitch web session — secondary, de-emphasized */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            padding: '10px 13px',
            border: 'var(--hair)',
            borderRadius: 6,
            background: 'var(--zinc-925)',
          }}
        >
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-300)', fontWeight: 500 }}>
              Twitch web session
            </div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginTop: 2 }}>
              {twitch_web
                ? `Connected as @${twitch_web.login}`
                : 'Sign in once for sub-anniversary detection (separate from chat login)'}
            </div>
          </div>
          {twitch_web ? (
            <button type="button" className="rx-btn rx-btn-ghost" onClick={runTwitchWebClear}>
              Disconnect
            </button>
          ) : (
            <button type="button" className="rx-btn rx-btn-ghost" onClick={runTwitchWebLogin} disabled={twWebRunning}>
              {twWebRunning ? 'Waiting on Twitch…' : 'Connect'}
            </button>
          )}
        </div>
        {twWebError && (
          <div style={{ color: 'var(--warn, #f59e0b)', fontSize: 'var(--t-11)', paddingLeft: 4 }}>{twWebError}</div>
        )}
      </div>

      <YoutubePasteDialog
        open={ytPasteOpen}
        onClose={() => setYtPasteOpen(false)}
        onSubmit={async (text) => {
          await loginYoutubePaste(text);
          setYtPasteOpen(false);
        }}
      />
    </div>
  );
}
```

- [ ] **Step 6: Add the PlatformCard, ImportControl, ImportNote, and YoutubeSignInExtras components**

In `src/components/PreferencesDialog.jsx`, add these components directly after the `AccountsTab` function's closing brace (before `function YoutubePasteDialog`):

```jsx
function PlatformCard({ cfg, connected, detail, authButton, importZone, disclosure, error }) {
  return (
    <div
      style={{
        flexShrink: 0,
        border: '1px solid var(--zinc-800)',
        borderRadius: 8,
        background: 'var(--zinc-900)',
        overflow: 'hidden',
      }}
    >
      {/* Identity row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '13px 14px' }}>
        <div
          style={{
            width: 36, height: 36, flexShrink: 0, borderRadius: 9,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontFamily: 'var(--font-mono)', fontWeight: 600, fontSize: 16,
            background: cfg.monoBg, color: cfg.accent, border: `1px solid ${cfg.monoBorder}`,
          }}
        >
          {cfg.letter}
        </div>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 'var(--t-13)', fontWeight: 600, color: 'var(--zinc-100)', display: 'flex', alignItems: 'center', gap: 7 }}>
            {cfg.name}
            <span className="rx-mono" style={{ fontSize: 9, letterSpacing: '.07em', textTransform: 'uppercase', color: cfg.accent }}>
              {cfg.tag}
            </span>
          </div>
          <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginTop: 3, display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
            <span style={{ width: 6, height: 6, borderRadius: '50%', flexShrink: 0, background: connected ? 'var(--ok)' : 'var(--zinc-600)' }} />
            <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{detail}</span>
          </div>
        </div>
        <div style={{ flexShrink: 0 }}>{authButton}</div>
      </div>

      {/* Import zone */}
      <div style={{ borderTop: '1px solid rgba(255,255,255,.05)', background: 'var(--zinc-925)', padding: '12px 14px 13px' }}>
        {disclosure}
        {importZone}
        {error && (
          <div style={{ marginTop: 8, fontSize: 'var(--t-11)', color: 'var(--live)' }}>{error}</div>
        )}
      </div>
    </div>
  );
}

function ImportNote({ children }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
      <span style={{ display: 'inline-flex', width: 14, height: 14, alignItems: 'center', justifyContent: 'center' }}>⌗</span>
      {children}
    </div>
  );
}

function ImportControl({ title, desc, accent, state, onRun }) {
  const status = state?.status ?? 'idle';
  const running = status === 'running';
  const label = running ? 'Importing' : status === 'done' ? 'Import again' : 'Import now';
  const btnClass = status === 'done' ? 'rx-btn rx-btn-ghost' : 'rx-btn';
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 14 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-300)', fontWeight: 500 }}>{title}</div>
          <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginTop: 2, lineHeight: 1.45 }}>{desc}</div>
        </div>
        <button
          type="button"
          className={btnClass}
          onClick={onRun}
          disabled={running}
          style={{ flexShrink: 0, minWidth: 96, justifyContent: 'center' }}
        >
          {running && (
            <span
              style={{
                width: 11, height: 11, borderRadius: '50%',
                border: '1.5px solid currentColor', borderTopColor: 'transparent',
                display: 'inline-block', animation: 'rx-spin .7s linear infinite',
              }}
            />
          )}
          {label}
        </button>
      </div>

      {running && (
        <div style={{ marginTop: 11 }}>
          <div style={{ position: 'relative', height: 4, borderRadius: 3, background: 'var(--zinc-800)', overflow: 'hidden' }}>
            <div
              style={{
                position: 'absolute', top: 0, height: '100%', width: '40%',
                borderRadius: 3, background: accent,
                animation: 'acc-indeterminate 1.1s ease-in-out infinite',
              }}
            />
          </div>
          <div className="rx-mono" style={{ fontSize: 10.5, color: 'var(--zinc-400)', marginTop: 6 }}>
            Importing your follows…
          </div>
        </div>
      )}

      {status === 'done' && state?.result && (
        <div style={{ marginTop: 10, display: 'flex', alignItems: 'center', gap: 8, animation: 'acc-pop .2s ease' }}>
          <span style={{ display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: 15, height: 15, borderRadius: '50%', background: 'rgba(34,197,94,.16)', color: 'var(--ok)', fontSize: 10 }}>✓</span>
          <span className="rx-mono" style={{ fontSize: 10.5, color: 'var(--zinc-300)' }}>
            Added {state.result.added} · skipped {state.result.skipped} · {state.result.total_seen} seen
          </span>
        </div>
      )}

      {status === 'error' && (
        <div style={{ marginTop: 8, fontSize: 'var(--t-11)', color: '#f87171' }}>{state.error}</div>
      )}
    </div>
  );
}

function YoutubeSignInExtras({ browsers, ytBrowser, setYtBrowser, ytAdvanced, setYtAdvanced, onPaste, ytError }) {
  return (
    <div style={{ marginBottom: 10 }}>
      <button
        type="button"
        onClick={() => setYtAdvanced((v) => !v)}
        style={{ all: 'unset', cursor: 'pointer', fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}
      >
        {ytAdvanced ? '▾ Other ways to sign in' : '▸ Other ways to sign in'}
      </button>
      {ytAdvanced && (
        <div style={{ marginTop: 8, padding: '8px 10px', border: '1px solid var(--zinc-800)', borderRadius: 4, display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-300)', marginBottom: 4, fontWeight: 500 }}>
              Use cookies from a browser
            </div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginBottom: 6 }}>
              Reuses an existing browser session — no extra sign-in needed.
            </div>
            {browsers === null ? (
              <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>Detecting…</div>
            ) : browsers.length === 0 ? (
              <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>No supported browsers found on this system.</div>
            ) : (
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {browsers.map((b) => {
                  const active = ytBrowser === b.id;
                  return (
                    <Tooltip key={b.id} text={active ? `Stop using ${b.label} cookies` : `Use ${b.label} cookies`}>
                      <button
                        type="button"
                        className={active ? 'rx-btn' : 'rx-btn rx-btn-ghost'}
                        onClick={() => setYtBrowser(active ? null : b.id)}
                      >
                        {active ? `✓ ${b.label}` : b.label}
                      </button>
                    </Tooltip>
                  );
                })}
              </div>
            )}
          </div>
          <div style={{ borderTop: 'var(--hair)', paddingTop: 8 }}>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-300)', marginBottom: 4, fontWeight: 500 }}>Paste cookies</div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginBottom: 6 }}>
              For sandboxed builds (Flatpak) where the app can’t reach a browser cookie store.
            </div>
            <button type="button" className="rx-btn rx-btn-ghost" onClick={onPaste}>Paste cookies…</button>
          </div>
          {ytError && <div style={{ fontSize: 'var(--t-11)', color: '#f87171' }}>{ytError}</div>}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 7: Verify the frontend builds and the old Row-based code is gone**

Run: `npm run build`
Expected: succeeds with no "unused" errors. If the build complains that `Row` is now unused inside `AccountsTab` only, that's fine — `Row` is still used by the other tabs.

Manually confirm in `npm run dev` (browser, mock IPC): open Preferences → Accounts. Expect four cards, the header "Import all follows" button, the `0/4`→`N/4` badge updating as you connect mock accounts, the Kick note, and the Twitch web session row at the bottom. Clicking "Import now" on Twitch shows the running bar briefly then "Added 0 · skipped 0 · 0 seen" (mock).

- [ ] **Step 8: Commit**

```bash
git add src/tokens.css src/components/PreferencesDialog.jsx
git commit -m "feat(accounts): card-based Accounts panel redesign"
```

---

### Task 4: Final verification + roadmap

Run the full gate and update the roadmap if the feature is tracked there.

**Files:**
- Modify: `docs/ROADMAP.md` (only if an Accounts/import item exists there)

- [ ] **Step 1: Rust gate**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS.
Run: `cargo clippy --manifest-path src-tauri/Cargo.toml`
Expected: no new warnings from `auth/youtube.rs` or `lib.rs`.

- [ ] **Step 2: Frontend gate**

Run: `npm run build`
Expected: succeeds.

- [ ] **Step 3: Roadmap**

Open `docs/ROADMAP.md`. If there's an item covering accounts/auth/import, flip `- [ ]` → `- [x]` and append the PR number once known. If YouTube subscriptions import or an "Accounts panel" item isn't listed, add it as a checked bullet under the appropriate phase. If nothing relevant exists, skip.

```bash
git add docs/ROADMAP.md
git commit -m "docs(roadmap): note Accounts panel redesign + YouTube import"
```

(Skip the commit if `docs/ROADMAP.md` was not changed.)

---

## Notes for the implementer

- The InnerTube `fetch_subscriptions` path can only be exercised end-to-end with a real signed-in YouTube account; CI/unit coverage is the pure `parse_subscriptions` parser. If a manual test returns zero channels, log the raw response and check whether YouTube changed the `channelRenderer` shape — the recursive walk is intentionally tolerant, so a zero result usually means an auth failure (SAPISIDHASH/cookies) rather than a parse miss.
- `import_youtube_subscriptions` works on all platforms (pure HTTP) — unlike `import_chaturbate_follows`, it is **not** Linux-gated.
- The prototype's simulated live count is intentionally replaced by an indeterminate bar; do not add a fake counter.
