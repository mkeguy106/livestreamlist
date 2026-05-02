# Twitch Sub-Anniversary Banner — Design

**Date:** 2026-05-02
**Status:** approved (brainstorming complete)
**Goal:** port the Qt app's sub-anniversary feature to the Tauri rewrite, with two meaningful UX improvements over Qt: cookie capture via WebView (replacing manual paste) and one-click in-app share via a signed-in popout WebView (replacing "open the channel page in your default browser and hope you're logged in").

## Background

The Qt predecessor (`~/livestream.list.qt/`) displays a dismissible banner above the chat composer when the logged-in user has an unshared sub anniversary. Twitch's web UI surfaces a "Share" button when a sub renews; the share broadcasts a `USERNOTICE` (`msg-id=resub`) to the channel's chat with the user's optional message. The window is open for ~7 days after each renewal.

**Detection** — Qt queries `gql.twitch.tv` on chat-open with the user's browser `auth-token` cookie (Helix OAuth tokens are rejected by this endpoint):

```graphql
query SubAnniversary($login: String!) {
  user(login: $login) {
    self {
      subscriptionBenefit { tier renewsAt purchasedWithPrime gift { isGift } }
      subscriptionTenure(tenureMethod: CUMULATIVE) { months daysRemaining }
    }
  }
}
```

If `renewsAt - now ≥ 22 days`, the sub renewed within the last ~8 days → window is open. Returns months + tier + Prime/gift flags.

**Banner** — pinned above input: `⭐ It's your N months sub anniversary! — Share on Twitch ↗` linking to `https://twitch.tv/{login}` + `×` dismiss.

**Dismissal** — `dismissed_sub_anniversaries[channel_key] = renews_at_str` in settings; resets next cycle (since `renews_at` changes).

**Auto-dismiss** — watches USERNOTICE messages on the IRC stream; when one arrives with `user.name == own_login` and `system_text` contains "subscribed", treats it as a successful share and persistently dismisses.

**Setting** — `show_sub_anniversary_banner: bool = True` in `chat.builtin`.

### Roadmap entry

`docs/ROADMAP.md` line 356 (Phase 3): "Sub-anniversary banner — when the logged-in user's Twitch anniversary is detected via IRC, show a one-shot dismissible banner per billing cycle." (The "via IRC" wording is a roadmap artifact; the actual mechanism — established by Qt and adopted here — is GraphQL on chat-open. IRC alone can only detect a *just-shared* anniversary, not a *ready-to-share* one.)

### Improvements over Qt

| Qt | This design | Why it matters |
|---|---|---|
| Manual `auth-token` paste in Preferences | One-time WebView login that scrapes the cookie | Qt's worst UX surface — most users never figure out where to find the cookie |
| Banner link opens default browser | "Share now" button opens an in-app popout WebView signed in via the captured cookie | No cross-browser login dependency; works even if user's default browser has no Twitch login |
| User clicks Share in their browser; we don't know what happened | In-app popout auto-closes when we observe the resulting `USERNOTICE` on IRC | Tighter feedback loop; banner disappears the moment the share lands |
| Per-chat-open GQL fetch (no cache) | 6h LRU cache (Some) / 5min (None) per channel | Columns layout with N subbed channels visible doesn't fan out N concurrent GQL requests on every selection change |
| Single chat surface — one banner at a time | Each `ChatView` owns its own banner; multiple stack naturally in Columns | First-class multi-channel monitoring is the whole point of the Tauri app |
| Cookie missing → silent no-op | First Twitch chat-open with a missing cookie → lazy connect prompt explaining the feature | Discoverability without nagging |

## Goals

- Detect ready-to-share Twitch anniversaries on chat-open
- Show a dismissible banner pinned above the composer
- One-click share via in-app popout WebView (Twitch's own Share modal handles the optional message field)
- Auto-dismiss when we observe the resulting `USERNOTICE` on IRC
- Per-cycle persistent dismissal (resets on next billing cycle naturally)
- Lazy "connect web session" prompt the first time the feature would benefit the user
- Preference toggle (default on)
- Multi-channel: N subbed channels visible in Columns → N independent banners

## Non-goals (explicit)

- **Kick parity.** Spike confirmed Kick has no equivalent share affordance (no public `subscriptionTenure` endpoint; no "share resub" UI surface; no documented chat event for ready-to-share state). The `channel.subscription.renewal` Pusher webhook requires a public webhook URL and has [a documented bug where it doesn't fire if the subscriber attaches a message](https://github.com/KickEngineering/KickDevDocs/issues/189). If Kick ships an analogous feature, revisit as a follow-up.
- **YouTube parity.** No equivalent surface.
- **Programmatic share** (calling Twitch's GQL `ShareResubNotification` mutation directly). Possible but requires replicating Twitch's `Client-Integrity` (KPSDK) anti-bot header, which is fragile and risks account flags. The popout approach achieves the same UX with zero ToS exposure since the user performs the action inside Twitch's native UI.
- **Rich anniversary stats UI.** Just months + ready-to-share signal. No charts, no history, no celebration animation.
- **Manage-dismissed-anniversaries UI.** Edit `settings.json` by hand if you really need to.

## Architecture

```
React (ChatView)
  ├── <SubAnniversaryBanner channelKey={k} />
  │     ↳ useSubAnniversary(k) → invoke('twitch_anniversary_check')
  │     ↳ "Share now" → invoke('twitch_share_resub_open')
  │     ↳ "×" → invoke('twitch_anniversary_dismiss')
  └── <TwitchWebConnectPrompt /> (one-time, lazy-triggered)
        ↳ "Connect" → invoke('twitch_web_login')

Rust
  ├── auth/twitch_web.rs (NEW) — cookie-capture WebView popup +
  │     keyring store of `auth-token`; validate on launch
  ├── platforms/twitch_anniversary.rs (NEW) — GQL query against
  │     gql.twitch.tv with the captured cookie; 6h cache; pure
  │     window-math function (renewsAt + now → is_active)
  ├── share_window.rs (NEW) — transient WebviewWindow loader
  │     for popout chat, shared profile dir with cookie store
  └── chat/twitch.rs (EXISTING, +small change) — when build_usernotice
        sees own-login + msg-id ∈ {resub, sub}, emit chat:resub_self:{k}
        (subgift/anonsubgift are someone gifting TO others, not own share)

Persistence
  ├── ~/.config/livestreamlist/settings.json
  │     ├── chat.show_sub_anniversary_banner: bool (default true)
  │     └── chat.dismissed_sub_anniversaries: HashMap<channel_key, renews_at_str>
  ├── system keyring
  │     └── twitch_browser_auth_token (string)
  │     └── twitch_web_identity (json: {login, expires_at})
  └── ~/.local/share/livestreamlist/webviews/twitch_web/
        — shared WebContext profile dir for both cookie-capture
          window AND share popouts (so the share popout sees the
          captured cookie automatically)
```

### Why the share popout is a top-level `WebviewWindow`, not a child embed

The CB/YT chat embeds (`embed.rs`) live inside the main window's `gtk::Fixed` overlay because the user wants them as part of their persistent chat layout. The share popout is fundamentally different:

- Transient: open → user clicks Share → close (auto or manual). Lifetime measured in seconds.
- Needs its own native window context: Twitch's Share modal is a `position: fixed` overlay; embedding it inside our main window's Fixed makes the modal occlude our own UI in confusing ways.
- Top-level windows on Linux honor `set_size` correctly via Tauri's standard window builder (the `add_child` Linux pitfall doesn't apply here).

We register popouts in `Mutex<HashMap<channel_key, WebviewWindow>>` so we can close on auto-dismiss or on user re-trigger (focus existing instead of opening a duplicate).

### Why auto-dismiss is a Rust-side concern

Qt's auto-dismiss runs in the manager: each batch of incoming `ChatMessage`s is scanned for own-login + system text containing "subscribed". This means every chat surface that would care must repeat the same scan.

We move the detection one layer down: `build_usernotice` already knows it's processing a USERNOTICE and already has the `msg-id` and `login`. Adding a typed Rust-side check + a separate emit (`chat:resub_self:{k}`) is ~5 lines and keeps React's `useSubAnniversary` from having to filter the entire message stream. The USERNOTICE itself still flows through the normal `chat:message:{k}` path so it's visible in chat as expected.

## IPC surface

### Commands

| Command | Args | Returns | Purpose |
|---|---|---|---|
| `twitch_anniversary_check` | `unique_key: String` | `Option<SubAnniversaryInfo>` | Cookie-gated GQL query w/ 6h cache; returns Some only if window active + not dismissed + setting on |
| `twitch_anniversary_dismiss` | `unique_key, renews_at: String` | `()` | Persist `{channel_key: renews_at}` in settings; emit `settings:changed` |
| `twitch_web_login` | — | `bool` | Open cookie-capture WebView; resolves true on success, false on cancel/timeout |
| `twitch_web_status` | — | `Option<TwitchWebIdentity>` | `{login, expires_at}` if cookie cached + recently validated |
| `twitch_web_clear` | — | `()` | Wipe keyring entry + close any open share popouts |
| `twitch_share_resub_open` | `unique_key` | `()` | Open transient WebviewWindow loading `https://www.twitch.tv/popout/{login}/chat` w/ shared profile |
| `twitch_share_window_close` | `unique_key` | `()` | Close the popout window for that channel (idempotent) |

### Events

| Topic | Payload | When |
|---|---|---|
| `chat:resub_self:{unique_key}` | `{ months: u32, login: String }` | `build_usernotice` sees own-login + msg-id ∈ {resub, sub} (subgift/anonsubgift are someone gifting TO others, not the user's own share) |
| `twitch:web_cookie_required` | `{ reason: "missing" \| "expired" }` | `twitch_anniversary_check` finds no cookie OR GQL returns 401/403 |
| `twitch:web_status_changed` | `Option<TwitchWebIdentity>` | After login or clear |

### Data shapes

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAnniversaryInfo {
    pub months: u32,
    pub days_remaining_in_window: u32, // 22 days until renewal = 8 days into share window
    pub tier: String,                  // "1000" | "2000" | "3000"
    pub is_prime: bool,
    pub is_gift: bool,
    pub channel_login: String,
    pub channel_display_name: String,
    pub renews_at: String,             // ISO 8601, used as dismissal cycle key
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchWebIdentity {
    pub login: String,
    pub expires_at: Option<String>,    // ISO 8601, from Twitch cookie expires-at if available
}
```

## Data flow

### On chat-open (per `ChatView` mount)

1. `useSubAnniversary(channelKey)` invokes `twitch_anniversary_check(channelKey)`.
2. Rust sequence:
   - bail if `channel.platform != Twitch` → `None`
   - bail if `settings.chat.show_sub_anniversary_banner == false` → `None`
   - look in 6h LRU cache (keyed by `channel_login`); cache hit → return cached `Option`
   - load `auth-token` from keyring; missing → emit `twitch:web_cookie_required {reason: "missing"}`, return `None`
   - POST to `gql.twitch.tv` with `Client-Id: kimne78kx3ncx6brgo4mv6wki5h1ko`, `Authorization: OAuth {auth-token}`, the verbatim `SubAnniversary` query
   - 401/403 → clear keyring, emit `twitch:web_cookie_required {reason: "expired"}`, return `None`
   - other HTTP/JSON failure → log warn, cache `None` for **5min**, return `None`
   - parse → if no `subscriptionBenefit` → cache `None`, return `None`
   - compute `days_until_renewal = (renews_at - now).total_seconds() / 86400`
   - if `< 22` → cache `None`, return `None`
   - check `settings.chat.dismissed_sub_anniversaries[channel_key] == renews_at_str` → return `None`
   - return `Some(SubAnniversaryInfo)`; cache it (6h TTL)
3. React: if `Some` → mount `<SubAnniversaryBanner>`. If event `twitch:web_cookie_required` arrives → mount `<TwitchWebConnectPrompt>` instead (one-shot per app session, dismissible).

### On Share Now click

1. React invokes `twitch_share_resub_open(channelKey)`.
2. Rust looks up channel login from store; calls `WebviewWindow::builder("share-resub-{channel_login}")` (using `channel_login`, not `unique_key`, because Tauri window labels must match `^[a-zA-Z0-9_-]+$` and `unique_key` contains a `:` separator) with size 380×720, title `"Share your sub anniversary — {display_name}"`, URL `https://www.twitch.tv/popout/{login}/chat`, WebContext using the shared `~/.local/share/livestreamlist/webviews/twitch_web/` profile dir.
3. Window registered in `Mutex<HashMap<channel_key, WebviewWindow>>`.
4. If a popout for that key already exists → focus it instead of opening a duplicate.
5. User clicks Twitch's native Share button inside the popout → fills out optional message → submits → Twitch broadcasts USERNOTICE to their IRC.

### On observed own-resub

1. Our IRC client receives USERNOTICE; `build_usernotice` constructs the `ChatMessage` as it does today.
2. **NEW**: after build, if `login == cfg.identity_login` AND `msg-id ∈ {resub, sub}` → emit `chat:resub_self:{channel_key}` with `{months, login}`. (`subgift`/`anonsubgift`/`giftpaidupgrade` are someone gifting TO others, not the user's own share, so they don't count.)
3. React's `useSubAnniversary` listener fires:
   - invokes `twitch_anniversary_dismiss(channelKey, currentInfo.renews_at)` — persists dismissal
   - invokes `twitch_share_window_close(channelKey)` — closes the popout
   - clears local `info` state → banner unmounts
4. The USERNOTICE itself still flows through the normal chat-message path, so the user sees their resub fanfare in the message stream right where they sent it from.

### On × dismiss

Invoke `twitch_anniversary_dismiss(channelKey, info.renews_at)`; banner unmounts; popout left alone (user is using it).

### On Preferences toggle off

React effect on `show_sub_anniversary_banner` change clears local banner state across all mounted ChatViews. Rust `twitch_anniversary_check` short-circuits to `None` going forward. Open share popouts are closed via `share_window::close_all()`.

### On launch

`twitch_web_status` invoked once at app start. If keyring has a cookie, do a cheap GQL ping (e.g. `query CurrentUser`) to validate. If 401/403 → clear silently; user gets the lazy connect prompt next time they open a Twitch chat.

## Components — file map

### New files

```
src-tauri/src/auth/twitch_web.rs              ~250 lines
  • login_via_webview(app: AppHandle) → Result<TwitchWebIdentity>
    - opens WebviewWindow at https://www.twitch.tv/login w/ shared profile dir
    - on_navigation handler polls cookies_for_url("https://twitch.tv/")
    - when auth-token present AND URL no longer /login → capture, validate, close
    - 5min timeout → return Err
  • status() → Option<TwitchWebIdentity>     (cookie present + recently validated)
  • clear() → unmount any share popouts; remove keyring entry
  • validate(client, &cookie) → Result<TwitchWebIdentity>
    - cheap GQL request (e.g. CurrentUser query) confirming cookie still works
  • Stored in keyring as "twitch_browser_auth_token" + "twitch_web_identity"

src-tauri/src/platforms/twitch_anniversary.rs  ~200 lines
  • check(channel_login, cookie, cache) → Result<Option<SubAnniversaryInfo>>
  • compute_window(renews_at, now) → Option<u32>   (pure; unit-tested)
    - returns Some(days_remaining_in_window) iff days_until_renewal >= 22
  • parse_response(json) → Option<SubAnniversaryInfo>   (pure; unit-tested)
  • Cache: parking_lot Mutex<HashMap<String, (Instant, Option<SubAnniversaryInfo>)>>
    6h TTL for Some, 5min TTL for None (so transient errors retry quickly)

src-tauri/src/share_window.rs                 ~120 lines
  • open(app, channel_key, channel_login, display_name) → Result<()>
  • close(app, channel_key) → ()
  • close_all() → ()
  • State: Mutex<HashMap<String, WebviewWindow>>; on close, remove + drop

src/components/SubAnniversaryBanner.jsx       ~80 lines
  • Props: channelKey, info, onShare, onDismiss
  • Reuses .rx-chiclet style; ⭐ + months text + [Share now] (.rx-btn-primary) + ×
  • Subtitle below: "Twitch will let you add a message"

src/components/TwitchWebConnectPrompt.jsx     ~60 lines
  • Lazy-mounted by ChatView when twitch:web_cookie_required fires
  • One-shot per session (not persisted); same look as banner
  • Text: "We can detect your Twitch sub anniversaries. Sign in once to enable."
  • [Connect] [Not now]

src/hooks/useSubAnniversary.js                ~120 lines
  • Inputs: channelKey
  • Outputs: { info, connectPromptVisible, share, dismiss, dismissPrompt }
  • Effects:
    - on mount + channelKey change: invoke twitch_anniversary_check
    - listenEvent('chat:resub_self:' + channelKey, () => auto-dismiss)
    - listenEvent('twitch:web_cookie_required', () => connectPromptVisible = true)
    - listenEvent('twitch:web_status_changed', () => re-check)
```

### Modified files

```
src-tauri/src/lib.rs
  • register 7 new commands in tauri::generate_handler!
  • init twitch_anniversary cache + share_window state in setup()

src-tauri/src/settings.rs
  • ChatSettings:
    + show_sub_anniversary_banner: bool (default true)
    + dismissed_sub_anniversaries: HashMap<String, String> (default empty)

src-tauri/src/chat/twitch.rs
  • In build_usernotice (after constructing ChatMessage):
    if msg.tags.get("msg-id") matches {resub, sub, subgift, anonsubgift}
       && login == cfg.identity_login (NEW field on TwitchChatConfig)
       → emit chat:resub_self:{channel_key} with {months, login}
  • TwitchChatConfig gets identity_login: Arc<RwLock<Option<String>>>
    populated from auth::twitch::current_identity()

src/components/ChatView.jsx
  • Just above the composer (or just above the dim-block-divider for backfill):
      <SubAnniversaryBanner ... />
      <TwitchWebConnectPrompt ... />

src/components/PreferencesDialog.jsx
  • Chat tab, new section after Spellcheck:
    "Sub anniversary banner" toggle (chat.show_sub_anniversary_banner)
    + secondary row: "Twitch web session: <connected as @login> [Disconnect]"
                     OR "Not connected [Connect]"
    + tooltip explaining what the connection is for

src/ipc.js
  • Wrap the 7 new commands; mock fallbacks for browser-only dev
```

## Error handling & edge cases

| Failure | Behavior |
|---|---|
| No `auth-token` cookie cached | `check` returns `None`. First Twitch chat-open in session → emit `twitch:web_cookie_required {reason: "missing"}` → `<TwitchWebConnectPrompt>` mounts. Subsequent chat-opens silent (one prompt per app session). |
| Cookie expired (GQL 401/403) | Clear keyring entry, `share_window::close_all()`, emit `twitch:web_cookie_required {reason: "expired"}` and `twitch:web_status_changed: null`. Prompt re-mounts with text "Twitch session expired — sign in again." |
| GQL 5xx / network error | Log warn, cache `None` for **5min only** (so a transient outage doesn't suppress the banner for 6h). Banner stays hidden. |
| Malformed GQL response | Same as above — `parse_response` returns `None`, cache 5min. |
| User opens Share popout, closes window without sharing | No `chat:resub_self` ever fires → banner stays. They can re-open from the same Share button. Re-opening when window already exists for that key: focus existing window instead of creating a new one. |
| User shares but IRC connection drops before USERNOTICE arrives | Auto-dismiss won't fire on the share itself, but `compute_window` eventually returns `None` once `days_until_renewal < 22` and the banner naturally disappears. Worst case: banner shows for up to ~8 more days. User can × dismiss manually. **Documented limitation.** |
| User shares from a different client (phone, web, another desktop) while our chat is connected | Same-channel IRC USERNOTICE still arrives → auto-dismiss fires correctly. ✓ |
| User shares while we're disconnected entirely | Same as the IRC-drop case above: banner naturally clears when the share window closes (~8 days), or earlier via × dismiss. **Documented limitation: cross-client share without our IRC running means no immediate auto-dismiss.** |
| `WebviewWindow::builder` fails (rare; Linux GTK init issue) | Toast: "Couldn't open share popout. Open in browser?" → fallback `xdg-open https://www.twitch.tv/popout/{login}/chat`. Same caveat as Qt: relies on browser login. |
| User toggles `show_sub_anniversary_banner` off while popout is open | `share_window::close_all()`; clear all banners. |
| User deletes channel from rail while popout is open | `share_window::close(channel_key)` for that key. |
| Multiple favorites are sub-anniversary-active simultaneously (e.g. 3 channels in Columns) | Each ChatView has its own banner. Each Share Now opens its own WebviewWindow with `share-resub-{channel_key}` window label. Multiple popouts open simultaneously is fine. |
| User's `twitch_identity` (OAuth login) and `twitch_browser_auth_token` (web login) are different accounts | Detected at login: `login_via_webview` returns `TwitchWebIdentity`; we compare to `auth::twitch::current_identity()`. Mismatch → toast "Web login is @X but app is logged in as @Y. The anniversary feature requires both to match." → don't store cookie. |
| User logs out of OAuth | Cookie remains usable (independent), but the auto-dismiss path needs `identity_login` — gracefully degrade: cookie-based detection still works, just no auto-dismiss; user must × manually. |

## Testing

### Rust unit tests

```
platforms::twitch_anniversary::compute_window
  • renewsAt 30 days out → Some(8)        // 30 - 22 = 8 days into share window
  • renewsAt 22 days out → Some(0)        // edge: just inside threshold
  • renewsAt 21 days, 23h out → None      // edge: just outside
  • renewsAt in past → None               // already renewed past window
  • renewsAt 1 year out → None            // sanity / annual sub edge

platforms::twitch_anniversary::parse_response
  • full response w/ subscriptionBenefit + tenure → Some(...)
  • response w/ no `self` → None          // user not subbed
  • response w/ self.subscriptionBenefit == null → None
  • response w/ tenure == null → handles gracefully
  • response w/ malformed renewsAt → None
  • response w/ purchasedWithPrime=true → is_prime=true
  • response w/ gift.isGift=true → is_gift=true

platforms::twitch_anniversary::cache (LRU + TTL)
  • Some cached for 6h, then re-fetched
  • None cached for 5min, then re-fetched
  • clear() empties

auth::twitch_web::extract_auth_token
  • cookies vec containing auth-token → Some(value)
  • cookies vec without auth-token → None
  • cookies w/ wrong domain → None
```

### React tests

None planned. Vitest harness doesn't exist in this repo. Visual feature; hand-tested in dev.

### Manual test plan

1. **Cold start, no cookie** → open Twitch chat for any channel → connect prompt appears → click Connect → WebView opens → log in → window closes → re-check happens automatically → banner appears (if you're inside an active window for that channel).
2. **Click Share now** → popout opens → click Twitch's Share button → modal appears w/ message field → submit → popout auto-closes → banner disappears → resub fanfare visible in chat stream.
3. **Click ×** → banner disappears → restart app → reopen same chat → banner stays dismissed (per-cycle key).
4. **Wait until next billing cycle** → banner re-appears (different `renewsAt`).
5. **Open multiple Columns of subbed channels** → all banners visible simultaneously → Share each → each popout independent.
6. **Toggle Preferences off** → all banners disappear → toggle on → re-fetch → banners reappear.
7. **Clear `auth-token` cookie in keyring manually** → next Twitch chat-open → expired prompt fires.
8. **Force GQL endpoint to 500** (e.g. block in `/etc/hosts`) → no banner, no error toast, normal behavior preserved.

## Open questions / future work

- **Programmatic share fallback.** If users in the wild report the popout flow being clunky (Twitch UI changes, slow page load, Share button not where expected), we have the auth-token cookie already — direct GQL `ShareResubNotification` mutation becomes feasible, modulo `Client-Integrity` header research.
- **Tier badge in banner.** `info.tier` and `info.is_prime` are already returned but not surfaced. A `Tier 2 ⭐` chiclet in the banner would be a small follow-up.
- **Notification on detection.** A native OS notification when an anniversary is freshly detected (not just a banner) would improve discoverability for users who don't open a given chat often. Out of scope for v1.
- **Cookie-capture WebView UA pinning.** If Twitch starts treating WebKitGTK's UA differently from Chromium's, we may need to spoof. Defer until observed.
