# Roadmap

Phased plan to reach feature parity with `livestream.list.qt`, with each phase sized to be landable in one working session. Tick items as they ship; keep the order — later phases assume earlier infrastructure.

---

## Phase 1 — Desktop shell + Twitch live status  ✓ shipped (PR #1)

- [x] Tauri v2 app with custom titlebar (no native chrome)
- [x] Rust channel store (JSON persistence to XDG config, atomic writes)
- [x] URL autodetect for Twitch / YouTube / Kick / Chaturbate (URLs, bare handles, `t:/y:/k:/c:` prefix), unit-tested
- [x] Twitch GraphQL live-status client (unauthenticated public client id, batched ≤ 35 channels/request)
- [x] 60 s refresh loop
- [x] Add-channel dialog + persistence + empty state
- [x] Streamlink launch (detached, survives app close) + browser handoff
- [x] Three layouts (Command / Columns / Focus) driven by real data
- [x] Keyboard: `1`/`2`/`3` layout, `N` add, `R` refresh
- [x] `WEBKIT_DISABLE_DMABUF_RENDERER=1` baked in for NVIDIA + Wayland

---

## Phase 2a — Built-in Twitch chat  ✓ shipped (PR #2, stacked on #1)

- [x] Anonymous Twitch IRC over WebSocket with IRCv3 tag parsing
- [x] 7TV / BTTV / FFZ global emote pipeline with an overlaying word scan
- [x] Inline emote rendering with 1x/2x/4x srcset
- [x] `useChat` hook + `ChatView` component (IRC + compact variants) wired into all 3 layouts
- [x] Mock event bus for browser-only dev so chat UI works without Tauri

---

## Phase 2a follow-ups — live-status parity across platforms  ✓ shipped

Twitch is the hardest one and it's done; the others are straightforward ports.

- [x] **YouTube live check** — subprocess `yt-dlp --dump-single-json --no-download` (matches Qt app). Parses `is_live`/`live_status`, `title`, `concurrent_view_count`, `release_timestamp`, thumbnail.
- [x] **Kick live check** — `GET https://kick.com/api/v2/channels/{slug}`; maps `livestream` object to `Livestream` struct. `start_time` parsed as UTC (handles both naive `YYYY-MM-DD HH:MM:SS` and RFC3339).
- [x] **Chaturbate live check** — individual `/api/chatvideocontext/{user}/` (public). Surfaces private/hidden/group as a non-error status so the UI can dim. Bulk follow endpoint deferred to Phase 2b (needs cookies).
- [x] Wired into `refresh.rs` alongside Twitch with `YT_CONCURRENCY = 2` cap (originally 5; lowered in PR #27 after observing YouTube rate-limiting); Kick/Chaturbate are cheap REST and run unbounded `join_all`.
- [x] **YouTube rate-limit cooldown** (PR #27) — yt-dlp's "rate-limited for up to an hour" notice was firing on the 60 s refresh; added `--sleep-requests 1` to space each invocation's internal request burst, plus a process-global `RATE_LIMITED_UNTIL` (`OnceLock<Mutex<Option<Instant>>>` in `platforms/youtube.rs`) that flips for 30 min when stderr matches "rate-limit" / "rate limit". `fetch_youtube_all` checks `is_rate_limited()` first and short-circuits with a single warn log so an active throttle isn't deepened by retries. Cooldown is in-process; relaunching resets it.
- [x] **UC-URL channel display-name backfill** (PR #31) — channels added via `youtube.com/channel/UC.../` had `display_name == channel_id` and stayed that way (the refresh path never propagated yt-dlp's friendly name to either the persisted `Channel` or the per-cycle `Livestream`). Added `ChannelStore::update_channel_display_name` + a `youtube::is_uc_id` heuristic; refresh now backfills the persisted name when the current value matches `UC...24chars`, and overrides `Livestream.display_name` for both live and offline branches so the rail renders correctly the same cycle.
- [x] **Login dropdown shows YouTube @handle and Chaturbate username** (PR #41) — `auth::youtube::fetch_user_info(http)` GETs `/account` with keyring cookies, scrapes `vanityChannelUrl` (fallback `channelHandleText`), persists to keyring as `youtube_user_info`. Triggered after login flows and on app boot when keyring cookies exist. CB equivalent runs inline from `login_via_webview` once the `sessionid` poll captures the WebView jar — `data-username` / `"username":"..."` patterns. `ChaturbateAuth` gained `Option<String> username` (`#[serde(default)]`). Surfaced via `auth_status` and rendered ahead of the existing "Signed in" / "Cookies from chrome" fallbacks. Diagnostic logging on the YT side (status, final URL, HTML length on parse failure) so DOM-marker drift is visible without forcing the user to re-login.
- [x] **YouTube live-status: scrape-first (Qt parity)** (PR #43) — `fetch_live` now runs `fetch_primary_via_scrape` first (single GET on `/live`, extracts `ytInitialPlayerResponse` and reuses the existing `parse_player_response`); yt-dlp moves to the fallback arm and only runs when the scrape returns no parseable player response. Multi-stream `/streams` + per-video `/watch` scrapes and the portrait-dedupe step run unchanged on top of either path. Channel `display_name` comes from `videoDetails.author` on scrape path vs `channel` from yt-dlp on fallback (same field internally, so PR #31's UC-ID backfill stays compatible). PR #27's 30-min rate-limit cooldown stays in place as defence-in-depth.
- [x] **YouTube concurrent viewer count** (PR #45) — `parse_player_response` was reading viewers from `microformat.playerMicroformatRenderer.viewCount` (all-time total, typically empty on live broadcasts), so small live streamers showed no viewer count at all. Now reads `videoDetails.viewCount` first (concurrent count for live, matches Qt) and falls back to the microformat field when missing. Affects both the scrape-first primary path and the multi-stream per-video `/watch` scrapes.
- [ ] **CB username on-boot retry** — PR #41 only populates the CB username at login time because the cookies live in the WebView profile (not keyring). Existing CB stamps from prior logins keep `username = None` until the user logs out and back in. Fix: at boot, if a stamp exists with no username, open a hidden short-lived WebViewWindow against the CB profile, eval JS to capture the username via custom event, save and close. Or: parse the WebKit cookie SQLite database in the profile dir (more fragile — varies by WebKit version).

---

## Phase 2b — Chat sending + Kick chat + moderation  ✓ shipped

All Phase 2b items shipped across multiple PRs (Twitch auth + sending, Kick chat, per-channel emotes, YT/CB embedded chat, YouTube cookie login, Chaturbate login + chat embed + playback in #23, YouTube multi-concurrent-stream support in #25).

### Twitch auth + sending  ✓ shipped

- [x] OAuth implicit flow — spawn system browser to `https://id.twitch.tv/oauth2/authorize?response_type=token&client_id=…&redirect_uri=http://localhost:65432&scope=chat:read+chat:edit+user:read:follows`
- [x] Local loopback HTTP server on port 65432 captures the `#access_token=…` fragment (Tauri Rust can bind this; use `tiny_http` or raw `hyper`)
- [x] Token stored via `keyring` crate (Secret Service / Keychain / Credential Manager)
- [x] Twitch IRC connection switches from `justinfan*` + anonymous pass to authed nick/pass once token present
- [x] `chat_send(uniqueKey, text)` invoke command; Twitch composer stops being a placeholder
- [x] Moderation events: `CLEARCHAT`, `CLEARMSG`, `NOTICE msg-id=…` — `chat:moderation:{key}` event → frontend removes or greys matching messages
- [x] Local echo handling (Twitch doesn't echo own PRIVMSG back; we fake it). PR #49 fixed the synthesized echo rendering as the lowercase IRC nick (`angeloftheodd`) instead of the cased `display-name` (`AngelOfTheOdd`) — `TwitchChatConfig` now captures `display-name` from `GLOBALUSERSTATE`/`USERSTATE` alongside `own_badges`, and `build_self_echo` prefers it over `auth.login`.

### Kick chat  ✓ shipped

- [x] OAuth 2.1 + PKCE flow to `https://id.kick.com/oauth/authorize` (callback also on 65432, reuse the server)
- [x] Pusher WebSocket: `wss://ws-us2.pusher.com/app/…`, subscribe to `chatrooms.{chatroom_id}.v2`
- [x] Parse `App\Events\ChatMessageEvent` → `ChatMessage` with emote parsing (Kick uses `[emote:ID:name]` inline tokens, not tag-based)
- [x] Kick does echo own messages via websocket → no local echo, skip-on-id-match
- [x] Send via `POST https://api.kick.com/public/v1/chat` with `Authorization: Bearer {token}`, auto-refresh on 401

### Per-channel 3rd-party emotes  ✓ shipped

- [x] Fetch 7TV channel emotes by Twitch user id (`GET /v3/users/twitch/{id}`)
- [x] BTTV channel emotes by Twitch user id
- [x] FFZ channel emotes by Twitch login
- [x] Wire channel emote load into `chat_connect` so `EmoteCache::set_channel` is populated before message flow starts
- [x] Twitch channel **sub emotes** — need auth; use Helix `GET /chat/emotes?broadcaster_id=…` once tokens are available

### YouTube + Chaturbate chat (embedded)

- [x] **YouTube embedded chat** — top-level borderless `WebviewWindow` snap-aligned over the React chat-pane region, parented via `transient_for(main)` so KWin keeps it stacked on top of the main window without `always_on_top`. Loads `https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v={video_id}`. Persistent profile at `~/.local/share/livestreamlist/webviews/youtube/` is shared with the auth-capture window so cookies persist across sign-in → embed. Reused via `Webview::navigate()` on same-platform channel switches (no close/recreate animation), recreated only when crossing platform boundaries. CSS injection on first paint matches the YouTube widget to our zinc theme. `_NET_WM_BYPASS_COMPOSITOR=1` is set on the X11 window so KWin skips Wobbly Windows + minimize/restore animations on the embed (the trade-off being that on systems with Wobbly enabled the embed stays rigid while the main window wobbles — an unavoidable side effect of the separate-surface architecture). Notes: Tauri's `add_child` was tried first but Linux puts child webviews into `gtk::Box` which auto-positions and ignores `set_position`; an `<iframe>` was tried second but YouTube's `X-Frame-Options: SAMEORIGIN` blocks it.
- [x] **Chaturbate embedded chat** — same `transient_for` top-level borderless window pattern as YouTube. Loads `https://chaturbate.com/{slug}/`. Injects the DOM-isolation script (ported from `livestream.list.qt`'s `chaturbate_web_chat.py::_ISOLATE_CHAT_JS`) on every page load, retrying every 250 ms until the chat container shows up — hides the video player, sidebar, ads, and header so only the chat column is visible. Persistent cookie profile at `~/.local/share/livestreamlist/webviews/chaturbate/`.
- [x] **YouTube cookie login** — YouTube's authenticated endpoints (send chat, read subscriptions) need the five Google session cookies: `SID`, `HSID`, `SSID`, `APISID`, `SAPISID`. Shipped two auth paths:
  - (a) **In-app sign-in** — opens a `youtube-login` WebviewWindow at `https://accounts.google.com/signin` with a persistent profile dir (`~/.local/share/livestreamlist/webviews/youtube/`). Polls `cookies_for_url(google.com)` every 750 ms; once all five target cookies are present, saves to the keyring (entry: `youtube_cookies`) + a Netscape `youtube-cookies.txt` for `yt-dlp --cookies` and closes the window. Cookies are also injected into the main webview at startup via `set_cookie` so the embedded `/live_chat` window sees us as signed in.
  - (b) **Manual paste fallback** — Preferences → Accounts → YouTube → "Paste cookies…" textarea. Accepts cookie-header (`SID=…; HSID=…`), one-per-line, or Netscape `cookies.txt` format. Required for Flatpak sandboxed builds where the in-app webview can't persist cookies.
  - (c) **Browser-cookie picker** — auto-detected installed browsers (Chrome, Firefox, Brave, Edge, Opera, Vivaldi, LibreWolf) shown as buttons under Preferences → Accounts → YouTube → "Other ways to sign in". Saves the choice to `settings.general.youtube_cookies_browser`; `yt-dlp` calls then pass `--cookies-from-browser <name>` instead of `--cookies <file>`. Mirrors the Qt app's primary auth path without us reimplementing the SQLite/decryption logic in Rust.
- [x] **Chaturbate login** (PR #23) — `chaturbate-login` WebviewWindow at `https://chaturbate.com/auth/login/` with custom zinc-950 chrome (drag + close via scoped capability granting `start_dragging` + `close` to chaturbate.com only). Poll for `sessionid` cookie via event-driven close detection + close-check-before-cookies-poll loop. Persistent profile shared with the chat embed (no re-injection). Stamp file (`chaturbate-auth.json`) with `last_verified_at` timestamp; embed-side verification on every page-load emits `chat:auth:chaturbate { signed_in, reason }` events that drive the Accounts row + chat-pane banner. Drift detection clears stamp only (not profile dir, embed window may be live). Playback uses `yt-dlp -g` → mpv (matches Qt's `LaunchMethod.YT_DLP` for CB).
- [x] **CB embed isolation re-injects on navigation** (PR #35) — `CB_ISOLATE_JS` (and the YouTube theme CSS) was `eval()`'d once at window creation, so the same-platform channel-switch path that reuses the embed via `navigate()` left the post-navigate page un-isolated (Chaturbate's full site — model video + chat — instead of just chat). Moved the eval into the `on_page_load(Finished)` handler so it runs on every page load, initial and navigations.
- [x] **YouTube multi-concurrent-stream channels** (PR #25) — NASA-style channels with 2+ simultaneous live streams render as N rows. `Livestream` gains `video_id: Option<String>`; `unique_key()` adds `:{video_id}` suffix for live YT (e.g. `youtube:UCnasa:isst1`). `platforms/youtube.rs` orchestrates yt-dlp primary detection + `youtube.com/channel/{id}/streams` HTML scrape (via `BADGE_STYLE_TYPE_LIVE_NOW` / `LIVE` overlay heuristic) + `youtube.com/watch?v=` per-video metadata scrape. Portrait dedupe (auto-Shorts variant of the primary feed) swaps to landscape via `find_landscape_alternative`. `channels::channel_key_of` strips the suffix for per-channel ops; per-stream ops carry the full key through to `embed.rs` + `player.rs` so chat/play/browser-open hit the SPECIFIC video. `YOUTUBE_MISS_THRESHOLD = 2` gives secondaries one cycle of grace before reaping (avoids flap on transient `/streams` partial returns).

---

## Phase 3 — Chat polish

- [ ] Reply threading — Twitch `@reply-parent-msg-id` IRC tag + Kick `reply_to_original_message_id` field → `reply_to` already on `ChatMessage`, but UI needs a reply-context row that word-wraps
- [ ] Conversation dialog — click a reply context to see the full @-mention conversation between two users
- [ ] `USERNOTICE` handling: sub/resub/raid/subgift/mystery-gift banners — promoted DismissibleBanner at top of chat
- [ ] Hype train + Pinned Message ("hype chat") banner
- [ ] Socials banner — Twitch GraphQL channel socials, YouTube `/about` scrape (remember `UC…` IDs need `/channel/UC…/about`), Kick REST
- [ ] Title banner with clickable game/category link (Twitch/Kick). Mind that Qt's `linkActivated` doesn't fire inside styled spans after `<br>` — anchor tags must sit outside opacity wrappers
- [ ] Per-channel chat logs — JSONL + plaintext rollups to `~/.local/share/livestreamlist/logs/{platform}/{login}/YYYY-MM-DD.jsonl`, LRU-prune to a configured disk budget
- [ ] Mention highlight row background + orange left accent bar
- [ ] Emote picker popup (search, category tabs, viewport culling)
- [ ] Tab completion for emotes (`:`) and mentions (`@`)
- [ ] **Third-party Twitch chat preload** — on chat connect, fetch the last N messages from a public history service (e.g. `recent-messages.robotty.de/api/v2/recent-messages/{login}`) and replay them before the live IRC stream starts. Matches the Qt app's pre-load behavior so a freshly opened channel isn't an empty box for the first 30 seconds. Configurable message cap (default 100) and respect the service's TTL / rate limits; gracefully skip when the service is unreachable.
- [ ] **Twitch whispers (DMs)** — send and receive one-to-one messages with other Twitch users. IRC whispers were deprecated in 2023; the current path is Helix `POST /helix/whispers?from_user_id=X&to_user_id=Y` for sending and EventSub `user.whisper.message` for receiving. Needs scope `user:manage:whispers`. UI: a separate "Whispers" tab list alongside channel tabs, one conversation per partner. Per-partner history persisted as JSONL under `~/.local/share/livestreamlist/whispers/{user_id}.jsonl` (mirror of Qt's `chat/whisper_store.py`). Rate limits to respect: 40/sec to known recipients, 100/day to new recipients for verified phone numbers.

---

## Phase 3 follow-ups — UX consistency + first-paint polish

These are small-scoped UX fixes that don't belong under a "phase" but should land together for a consistent polish pass.

- [ ] **Dark-mode first-paint** — the window currently flashes white on launch before the React bundle parses and paints. Set the Tauri window / `index.html` / `<body>` background to `--zinc-950` (our dark base) via a tiny inline `<style>` in the shell HTML and `"background_color"` in `tauri.conf.json` so the compositor maps the window on a dark surface from frame zero. No flash-bang at launch, especially on HDR / high-brightness monitors.
- [x] **Global hover audit — themed tooltip everywhere** (PR #39, PR #47) — extracted `src/components/Tooltip.jsx` from the inline popover that lived in `Command.jsx`'s `IconBtn`. PR #39 covered titlebar + sidebar icons; PR #47 closed the rest: channel rail rows ("Double-click to play"), `Composer` popout button, `PreferencesDialog` browser-cookie buttons, `UserHistoryDialog` rows, `Focus.jsx` channel title and play button, `ChatView` reply-context row, `SocialsBanner` URL tooltips, `UserBadges` / `UserCard` badges, `EmoteText` emote names. `Tooltip` grew `placement` (top/bottom), `wrap` (multi-line + maxWidth 320), `block` (full-width wrapper), `align` (left/center/right anchor — `right` keeps near-edge tooltips like Popout from overflowing the window), and `wrapperStyle` (vertical-align escape hatch for emote baselines). `TitleBanner.jsx`'s tooltip was removed rather than ported because the title text is already fully visible in the banner. Focus-visible parity is intentional follow-up below.
- [ ] **Focus-visible parity for tooltips** — `Tooltip` triggers on `mouseenter`/`mouseleave` only; keyboard users still get `aria-label` but no popover. Add `focus`/`blur` handlers + `:focus-visible` outline normalization across `tokens.css` so keyboard-only navigation has the same affordance as mouse hover.
- [x] **Channel-list search** (PR #29) — slim `.rx-input` between the filter/sort toolbar and the rail; case-insensitive substring match against `display_name + channel_id`. Selection resolves from the full `livestreams` list (not the filtered one) so typing only changes what's visible in the rail, never the chat panel — chat switches only on explicit click. Esc clears the query while focused; state is ephemeral by design.
- [x] **Inline favorite star + search clear button** (PR #50) — filled star icon renders to the left of the channel name in the Command sidebar when `ch.favorite` is true; click it to unfavorite. Unfavorited rows show no star to keep the dense list visually quiet — entry to favoriting stays on the right-click context menu, which is now a real toggle (`Pin as favorite` / `Unpin from favorites`) rather than a one-way set-true. Search input gains a small × button at the right edge that appears once there's text; click clears and refocuses the input. Existing Esc-to-clear preserved.
- [x] **Clipboard URL autopaste in Add Channel** (PR #52) — Qt-app parity: when the clipboard contains a URL pointing at a supported platform (Twitch / YouTube / Kick / Chaturbate), the Add Channel dialog prefills the input on open. Adds `tauri-plugin-clipboard-manager` and a `clipboard_channel_url` IPC that reuses the existing `parse_channel_input` parser so URL recognition stays a single source of truth. Required `http(s)://` prefix prevents bare-handle clipboard contents from auto-filling; 500-byte cap guards against giant clipboards. Prefilled value is select-all'd so a single keystroke replaces it. No clipboard polling — only checks on dialog open.
- [x] **Silence /channels/followers 401 log spam** (PR #56) — Helix `/channels/followers` requires `moderator:read:followers` scope + caller-is-mod status, neither of which the app's token has for most channels. Every user-card open re-fired the call and re-warned. The 401 was already swallowed gracefully (card just skipped follower rows), so this was purely log noise. `platforms/twitch_users.rs` now keeps a session-scope `AtomicBool` latch: first 401 logs once at info level with an explanation; subsequent `fetch_follow` calls short-circuit before the network round-trip. Latch is cleared on login/logout so a re-authenticated session can retry (in case the token gains the scope or the user becomes a mod).
- [x] **Sidebar refresh button** (PR #33) — two-arrow loop icon to the right of the live/total chiclet in the Command sidebar header; calls `ctx.refresh()` on click and spins via the new `rx-spin` keyframes (next to `rx-pulse` in `tokens.css`) while `ctx.loading` is true. Click is gated on `!loading` so rapid clicks don't fan out overlapping refreshes. The keyboard shortcuts (F5 / Cmd-R) already in `App.jsx` keep working — this is purely a visible affordance.
- [x] **Login chiclet tone-down** (PR #35) — closed-state titlebar chiclet (T/Y/K/C with status dots) was reading too bright for a passive affordance. Letter opacity 0.7 → 0.45; status dots Tailwind green-700/red-800 → green-900/red-900 (`#14532d` / `#7f1d1d`). Dropdown contents unchanged.
- [x] **Titlebar right-cluster reorder + gear bump** (PR #37) — gear button moved right of the login chiclet (closer to the platform-account state it adjusts via Preferences → Accounts) and resized from `fontSize: 10` → `14` so the ⚙ reads at a glance instead of disappearing into the chiclet's hairline border.
- [x] **Last-selected-channel memory** (PR #54) — persist `selectedKey` to `localStorage['livestreamlist.lastChannel']` on every change. On launch: initialize from localStorage, then a one-shot validator checks the channel against fresh refresh data — kept if still live, cleared otherwise so the existing default-selection effect picks the first live channel. Both effects gate on `loading=false` because the cached `list_livestreams` snapshot returns all channels with `is_live=false` on a fresh launch (live state is transient — not persisted across runs); without the gate the validator would always see stale-offline and fall back. Applies to Command and Focus layouts; Columns is unaffected.
- [ ] **Command layout options (A screen)** — Preferences → Appearance → Layout group, applying to the Command layout only (Columns and Focus have their own structure). Today the A screen is a fixed two-column grid: channel rail on the left, selected-channel pane on the right. User-selectable options:
  - **Sidebar position**: Left (default) / Right. Primary use case is right-handed users who want the list adjacent to where their mouse naturally sits, plus RTL-language preference. Implementation: a `data-sidebar-position="right"` attribute on `.rx-root` swaps grid-area assignments; no component restructure required.
  - **Sidebar width**: persistent drag-to-resize handle between the rail and the main pane. Clamp to `min-width: 220px` / `max-width: 520px` (outside that, the rail either hides channel names or consumes too much chat real estate). Persist the pixel value in `settings.appearance.command_sidebar_width`.
  - **Sidebar collapse toggle**: a small chevron in the rail header collapses the list to a 48 px icon-only rail (first letter of each channel's display name inside a platform-accent chip, live-dot to its left). Click any icon to re-expand on hover, or hit the chevron again to pin-open. Persist as `settings.appearance.command_sidebar_collapsed`.
  - **Sidebar density**: Comfortable (current) / Compact — swaps row height between 40 px (comfortable) and 28 px (compact) and the font size between `--t-13` and `--t-12`. Useful when the user has a long channel list and wants to see more on screen without using a smaller overall UI scale.
  - All four settings live as CSS custom properties on `:root` so Command.jsx doesn't have to branch — the grid template and spacing read from them.
- [x] **Login dropdown as separate WebviewWindow** (PR #27) — the inline titlebar dropdown was occluded by the YouTube/Chaturbate chat embed, which is itself a top-level `transient_for` window stacked above main. Replaced with a sibling borderless `transient_for` `WebviewWindow` (`login-popup` label, `WindowTypeHint::DropdownMenu` for KWin to stack it above the embed's `Utility`) loaded with the same React bundle; `LoginPopupRoot` renders the rows. Self-sizes via `ResizeObserver` → `login_popup_resize` IPC so the OS window tracks content height (rows + busy/error banners + future platforms). Cross-window auth state syncs via a global `auth:changed` event broadcast from each auth IPC. Dismissal is two-layered: the popup polls `document.hasFocus()` (300 ms grace, then 100 ms interval) AND the chiclet toggles via `openRef` so rapid double/triple-clicks can't strand a popup that never received focus. `resizable(true)` on the builder is required — KWin silently ignores `set_size` on non-resizable windows.
- [ ] **UI scale setting (accessibility)** — slider under Preferences → Appearance → UI Scale, range 75% – 200% in 5% steps, default 100%. Needed for users with low vision and for 4K-at-small-physical-size setups where the default sizing is too tight. Implementation options to evaluate:
  - (a) `WebviewWindow::set_zoom()` — Tauri 2 exposes WebKit/Chromium's page zoom. Best typography quality; scales the whole webview uniformly including fonts, images, and layout.
  - (b) CSS-variable approach — add `:root { font-size: calc(16px * var(--ui-scale)); }` and express every sizing token in `rem`. More invasive (needs a sweep of `tokens.css` and the inline-styled components), but gives us pixel-perfect control and survives future webview runtime changes.
  - (c) `document.body.style.zoom` — quick, cross-browser, but known to break fixed-position overlays and has subpixel-layout artifacts.
  Pick after a prototype comparison on 4K + 1440p + 1080p. Persist as `settings.appearance.ui_scale`. Apply *before* first paint so the app doesn't reflow after the React tree mounts.

---

## Phase 4 — Preferences / tray / notifications / single-instance

- [ ] **Preferences dialog** with the same 5 tabs as the Qt app: General / Playback / Chat / Appearance / Accounts
  - Tab layout as a second Tauri window (not modal) with hairline separators
  - Bind form state to `settings.json` in XDG config
- [ ] **Theme editor** — live color-picker for every zinc scale + accent; persist to `~/.config/livestreamlist/themes/*.json`
- [ ] **Desktop notifications** — Tauri's `tauri-plugin-notification`. Go-live events, whispers/DMs, quiet-hours window
- [ ] **System tray icon** — `tauri-plugin-tray` (currently partially shipped in `src-tauri/src/tray.rs` with a live-count tooltip and basic Show / Hide / Refresh / Quit menu). Bring to Qt parity:
  - **Dynamic icon state** — the tray icon visually reflects whether anything is live: a red live-dot variant when ≥ 1 channel is live, a greyed / monochrome variant otherwise. Source from an SVG, rasterise at 22 px (Linux AppIndicator), 16 px (Windows shell), 18 px (macOS menu bar). Refresh the icon from `refresh_all`'s result each poll.
  - **Click behaviour** — left-click toggles the main window (show → focus → hide cycle; matches Qt's `_on_activated` for `Trigger` and `DoubleClick`). Right-click opens the menu. Middle-click reserved for "refresh now" as a Linux convention.
- [ ] **Tray menu expansion** — current menu is Show / Hide / Refresh / Quit. Grow to match the Qt app:
  - **Open Livestream List** — always restores + focuses the main window, whether hidden, minimized, or on another workspace.
  - **Notifications** (checkable) — mirrors `settings.general.notify_on_live` so the user can toggle go-live alerts without opening preferences.
  - **Recent Notifications…** — opens a small, non-modal history dialog showing the last ~30 go-live notifications with channel display name, platform accent, timestamp, and a click-to-play / click-to-open action. The notification log itself is a ring buffer maintained in-memory (plus a JSONL rollover in `~/.local/share/livestreamlist/notifications.jsonl` for persistence across restarts).
  - **Refresh now** — triggers `refresh_all` invoke; tooltip updates on completion.
  - **Preferences…** — opens the preferences window (Phase 4's dialog).
  - **Quit** — ends the process (distinct from the Close button if close-to-tray is enabled).
- [ ] **Close-to-tray behaviour + first-close prompt** — `settings.general.close_to_tray` (already declared on the Settings struct, unused). When on, pressing the window's close button calls `hide()` instead of `close()`; the app keeps running in the tray, refresh continues, notifications fire. When off (default), close quits. **First-close prompt**: the very first time the user hits close, show a one-shot modal — "Keep Livestream List running in the tray to receive go-live notifications?" with **Yes / No / Always quit (don't ask)** options. Persist the chosen answer to `settings.general.close_to_tray` + `close_to_tray_asked`; mirror Qt's first-launch-prompt flow.
- [ ] **Start minimized** — `settings.general.start_minimized` toggle (plus a `--start-minimized` CLI flag for the autostart entry to pass). When on, the app launches directly to the tray with no visible window; the window is only shown when the user clicks the tray icon. Pairs with autostart for a login-to-tray experience. On Wayland sessions the "invisible until first show" sequence already works because of the deferred-show startup sequence we shipped in Phase 2.5.
- [ ] **Autostart at login** — Preferences → General → "Launch at login" toggle, writing platform-native autostart entries:
  - **Linux** — XDG `.desktop` file at `~/.config/autostart/com.mkeguy106.livestreamlist.desktop` with `Exec=livestreamlist --start-minimized`, `X-GNOME-Autostart-enabled=true`, and the `Hidden=true` flag added/removed as the toggle flips.
  - **Windows** — registry value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, path to the installed `.exe` + `--start-minimized`.
  - **macOS** — a LaunchAgent plist under `~/Library/LaunchAgents/com.mkeguy106.livestreamlist.plist` with `RunAtLoad=true` and the bundle identifier as the label.
  - Detect Flatpak builds (`FLATPAK_ID` env var set) and wrap the exec with `flatpak run app.livestreamlist.livestreamlist` instead of the raw binary path — mirrors Qt's `IS_FLATPAK` branch in `core/autostart.py`.
- [ ] **Linux desktop-environment support matrix** — tray works out-of-the-box on KDE, XFCE, Cinnamon, Budgie, LXQt, MATE, and Pantheon; on GNOME the user needs the AppIndicator shell extension. At startup, detect whether a StatusNotifierHost is present on D-Bus (`org.kde.StatusNotifierWatcher` or `org.ayatana.StatusNotifierWatcher`); if not, surface a non-intrusive "Tray icon unavailable — install the GNOME AppIndicator extension" notice in Preferences → General instead of failing silently.
- [ ] **Single-instance guard** — `tauri-plugin-single-instance` (already wired in `src-tauri/src/lib.rs::run`). A second launch reuses the existing instance: show + focus the main window through the same code path the tray click uses. `--allow-multiple` / `-m` CLI flag bypass for power users (matches Qt's flag name).
- [ ] Log viewer / "Open log directory"
- [ ] **Import/export settings + channels** — single-file JSON bundle containing `channels.json`, `settings.json`, saved theme files, and Phase 6's saved column groups. Top-level `version` string matching the app's `Cargo.toml` package version; on import, refuse files from a newer version, offer a best-effort migration from older versions. **Never includes credentials** (OAuth tokens live in the system keyring; YouTube/Chaturbate cookies live in webview profile directories). On import, offer `replace` vs `merge` semantics for channels (show a diff: adds / removes / updates; user picks per-row or bulk); settings replace wholesale. Emit a format compatible with Qt's `livestream-list-export-YYYY-MM-DD.json` where reasonable so users can migrate between the two apps.
- [ ] **Developer tab + context-menu hardening** — the default WebKit right-click menu (back / forward / reload / Inspect Element) leaks development tooling to normal users. Disable it by default via `tauri.conf.json` + a `contextmenu` listener in the shell HTML. Add a new "Developer" tab in Preferences with two toggles (both default off): "Show developer context menu" (re-enables the native menu including Inspect Element) and "Verbose logging" (promotes `log::debug!` calls to stdout + writes a detailed trace log to `~/.local/share/livestreamlist/logs/debug.log` for issue reports).

---

## Phase 5 — Import follows / spellcheck / Turbo / release

- [ ] **Import follows from Twitch** — Helix `GET /channels/followed?user_id=…` (requires `user:read:follows`). Paginate until cursor exhausted; dedupe against existing channels in the store.
- [ ] **Import follows from Kick** — authenticated REST endpoint (`GET /api/internal/v1/channels/followed` or the public-API equivalent once Kick stabilises it). Requires the Kick OAuth token from Phase 2b.
- [ ] **Import follows from Chaturbate** — bulk room-list endpoint (`/api/ts/roomlist/room-list/?follow=true`) with the logged-in session cookies from Phase 2b's Chaturbate sign-in. Paginate through all followed rooms; add each as a Chaturbate channel.
- [ ] **Import subscriptions from YouTube** — gated behind YouTube cookie login (Phase 2b). Request the authenticated subscriptions page (`https://www.youtube.com/feed/channels`) or the authenticated internal API (`youtubei/v1/browse` with `browseId=FEsubscriptions`); parse the subscription list for channel IDs, handles, and display names. Include a checkbox **"Only import channels that do livestreams"** (checked by default — matches Qt's `YouTubeImportDialog.filter_checkbox`); when on, hit each candidate channel's `/live` URL and drop channels that redirect to `/videos` (i.e., never-live VOD-only channels). Progress bar during the filter pass — typical subscription list of a few hundred channels takes 30–60 s with 10-way concurrency.
- [ ] **Spellcheck / autocorrect** — ship `hunspell` on Linux via the `hunspell` crate or subprocess; fall back to a pure-Rust `symspell` on Windows/macOS. Skip rules for emotes, URLs, mentions, all-caps. Red wavy underlines in composer; green underline on auto-correction for 3 s. Distance-1 Damerau-Levenshtein for auto; distance-≤ 2 for manual suggestions
- [ ] Adult word list (`data/adult.txt`) bundled to suppress false positives on chat slang
- [ ] **Twitch Turbo auth** — must use browser `auth-token` cookie (not the OAuth token — it's client-ID-bound). `Authorization: OAuth {cookie}` when passed to streamlink. Offer a login button that scrapes the cookie from a local WebView session. Once wired, subscribers get their subscribed quality tiers and Turbo users get ad-free playback automatically — same behavior as the Qt app
- [ ] **External-player picker + args** — Preferences → Playback tab lets the user pick their player (mpv / vlc / iina / custom path), set extra streamlink flags, and set extra player flags (passed via `-a '…'`). Match the Qt app's "Streamlink + additional arguments" form: validate the binary is on PATH or at the given absolute path, validate args don't contain shell metacharacters, and persist per-field
- [ ] Streamlink additional-args validator (must allow non-flag values like `debug` after `--loglevel`)
- [ ] Record-and-play support via `--record PATH`
- [ ] **Release pipeline** — GitHub Actions: build AppImage + `.deb` on Ubuntu, `.dmg` on macOS, Inno Setup `.exe` on Windows. Tag-driven (`v*`). One release at a time (GitHub Actions storage quota)
- [ ] Autoupdater — `tauri-plugin-updater` checking a manifest

---

## Phase 6 — Columns layout redesign + in-app playback

Today the Columns layout auto-populates from every live channel in the store. The target is a user-curated workspace: the user builds a column set, saves it as a named group, switches between groups, and actually watches the streams inline instead of shelling out to an external player.

- [x] **Pause auto-populate Columns ahead of redesign** (PR #27) — replaced the live-column rendering with a "redesign in progress" placeholder so no `<ChatView>` instances mount in this layout. Clears the deck for the new group-management + inline-playback work without ripping the layout switcher out. Restore happens as part of the items below.

### Group management

- [ ] **Empty by default** — Columns layout starts empty on first load; no auto-populate from live channels. A prominent empty state invites the user to add columns from their channel list.
- [ ] **Add-column picker** — next to the existing Add / Refresh buttons, an "Add column" button opens a picker of every channel in the store (live first, then offline alpha) with a checkbox selection model. User picks any subset — offline channels included — and they become columns in the current group.
- [ ] **Named groups + switcher dropdown** — groups persist to `settings.json`. A dropdown next to Add / Refresh lists saved groups and lets the user switch between them. Groups auto-save on any change (add / remove / reorder). The user can rename any group inline (double-click name, or a small edit-pencil in the dropdown).
- [ ] **Clear all columns** — a button next to Refresh that wipes the current group to empty. Confirmation dialog if the group has ≥ 3 columns to guard against accidents.
- [ ] **Per-column remove** — a minus (×) button on each column header removes it from the current group (auto-saved).
- [ ] **Drag-to-reorder columns** — HTML5 drag-and-drop (or a lightweight dnd-kit if needed) between columns; new order auto-saves to the group. Drop target is the column header region, not the chat body, to avoid drag-hijacking during chat selection.

### In-column video playback

- [ ] **Inline video** — render the live stream *inside* each column instead of launching streamlink to an external player. Use the HLS stream URL from Twitch / Kick / YouTube and play it in a `<video>` element via `hls.js` or Tauri's native video widget (prototype both; pick the lower-resource option on the target hardware).
- [ ] **Per-channel volume** — a slider on each column's video control bar; value persists to `settings.json` per `unique_key` so muting a specific channel survives restart.
- [ ] **Popout to external player** — a popout button on each column's video control bar that hands off to the user's configured external player (see Phase 5's player-picker). The inline video pauses while the external player is active.
- [ ] **Resume on external-player close** — track the external player PID; when it exits (detected by `Child::try_wait` polling or `wait_with_output` on a background task), automatically resume the inline video in the column. Audio setting is preserved through the handoff. If the user wants the popout to become "sticky," a settings toggle disables the auto-resume behavior.

### Not in scope for this phase

- DVR / scrubbing controls — inline video is live-only.
- Picture-in-picture across columns (would require a separate always-on-top child window per column; reassess after the basic inline-play ships).
- Recording while playing inline — use the popout external player for that; see Phase 5's `--record PATH` item.

---

## Deferred / out of scope

- Marketing website (intentionally: the Qt app doesn't have one either)
- Mobile (Android/iOS) — Tauri supports it but the feature set doesn't justify the port yet
- Telemetry / analytics / crash reporting — user-local utility, no server component
- Multi-user / profile support — one user, one machine
- Built-in stream recording beyond what streamlink gives us
- Clip creation, VOD playback, channel points UI, bits UI

## Known risks / open questions

| Risk | Mitigation |
|---|---|
| WebKitGTK on Wayland bugginess (already hit one crash; NVIDIA + Chrome devs warn of more) | DMABUF disabled; fallback path to `GDK_BACKEND=x11` documented in README |
| `yt-dlp` subprocess per YouTube channel is slow and rate-limited | Batch of 5; cache live/offline state with TTL; consider switching to raw `innertube` API later |
| Tauri cannot embed arbitrary cross-origin iframes (YT/Chaturbate chat) without a real webview window | Phase 2b will use `WebviewWindow` per embed; cookie persistence via named webview profiles |
| OAuth redirect hijacking — binding 65432 on a multi-user box | Use `127.0.0.1:65432` (loopback only); kill the listener after one request; one-time state token validation |
| Keyring plugin differs across distros (Secret Service vs libsecret vs KWallet) | Use the `keyring` crate which abstracts this; graceful fallback to encrypted file in XDG data if no keyring is available |
| Emote load latency on slow networks (first chat connect feels empty for 3-5 s) | Phase 3: disk-cache emote sets; serve from cache, revalidate in background (stale-while-revalidate) |
| Large channel lists → N WebSockets eat memory (each chat connection is ~200 KB resident) | Column layout already auto-disconnects off-screen columns; Phase 3 will add a configurable max-connected-chats |

## Sequencing notes

- Phase 2a follow-ups (YouTube/Kick/Chaturbate live) can ship independently; don't block Phase 2b
- Phase 2b *should* ship OAuth before Kick chat — Kick is the harder one and wants the auth infrastructure in place
- Phase 3 depends on Phase 2b's full chat plumbing
- Phase 4 depends on nothing phase-specific, but preferences wiring is much more useful after Phase 3 (more things to configure)
- Phase 5 could ship in any order; the release pipeline is the only item with hard external dependencies

---

## Dev infrastructure

Cross-cutting tooling work that doesn't fit a user-facing phase but unblocks higher-quality work in all of them.

- [ ] **Headless WebKit test harness for webview-lifecycle code** — today the auth flows (`auth/youtube.rs`, the upcoming `auth/chaturbate.rs`) and the chat-embed window manager (`embed.rs`) are exercised only by manual integration tests because Tauri's `WebviewWindow` needs a real WebKit2GTK runtime. Stand up a CI-runnable harness — Xvfb + WebKitGTK in a container, or `wry` directly with a virtual display — so we can write unit tests that cover: login-window URL-change detection, post-load JS injection, cookie persistence across "restarts", embed window create/navigate/teardown, transient-for parenting. Pre-req for confidently refactoring the embed/auth code; current "test by clicking through the UI" loop is the bottleneck on those modules.

---

## Proposed for roadmap review (2026-04-25)

Gap analysis against the Qt app (`~/livestream.list.qt/`) — docs (`README.md`, `CHANGELOG.md`, `docs/youtube-cookies.md`) and source tree. Items below are **candidates** to triage into the phased plan; none are committed. Each is tagged with a suggested fit (`→ Ph N` for existing phase, `→ new` for candidate follow-up, `→ shipped?` where we may already have it partially and need to verify before re-adding).

### Channels — list management and filtering

- [ ] **Trash bin / soft-delete** — deleted channels move to a `trash.json` rather than being permanently removed; dedicated Trash dialog with restore / permanent-delete / empty-all actions. → new (Ph 4 pref dialog is a natural home)
- [ ] **Selection mode** — multi-select with shift-click range, Select All / Deselect All / Delete Selected; toolbar toggle or Edit menu. → Ph 3 follow-ups
- [ ] **Sort modes** — expand beyond the implied default: Name (alpha), Viewers (desc), Playing (currently-launched first), Last Seen (offline-by-recency), Time Live (uptime asc). → new
- [ ] **Platform filter dropdown** — All / Twitch / YouTube / Kick / Chaturbate in the channel rail header. → Ph 3 follow-ups (lives next to the channel-list search already there)
- [ ] **Favorites-only filter** — `favorite` field is already on `Channel`; the filter UI is not. → Ph 3 follow-ups
- [ ] **Hide offline toggle** — one-click filter to show only currently-live channels. → Ph 3 follow-ups
- [ ] **Last-seen timestamp** — `channels.json` records the last-live UTC timestamp per channel and the row shows "2h ago" / "3d ago" when offline. → new (data model change + row rendering)
- [ ] **Auto-launch / auto-play channel flag + filter** — per-channel "A" toggle; filter to show only auto-launch channels; toolbar global auto-play master switch. → Ph 5 (pairs with external-player picker)
- [ ] **Per-channel "don't notify" UI** — the `dont_notify` field exists on `Channel` but has no UI toggle today. → Ph 4 (preferences / per-row)
- [ ] **Per-row icon visibility toggles** — independent show/hide for platform icon, play, stop, favorite, chat, browser icons in the channel row. → Ph 4 Appearance tab
- [ ] **Per-row info toggles** — independent show/hide for live-duration and viewer-count on each row. → Ph 4
- [ ] **Right-click context menu on channel rows** — Play / Close Channel / Open Chat / Open in Browser / Favorite. → Ph 3 follow-ups
- [ ] **Platform colors toggle** — toggle whether channel names are tinted by platform accent. → Ph 4 Appearance tab (`--twitch` / `--youtube` / `--kick` / `--cb` accents already defined in `tokens.css`)
- [ ] **Stream list font-size zoom** — `Ctrl+=` / `Ctrl+-` shortcuts that target the channel rail only, independent of the global UI scale. → Ph 3 follow-ups

### Live status refresh

- [ ] **User-configurable refresh interval** — expose the current hard-coded 60 s as a 10–300 s preference. → Ph 4 General tab
- [ ] **Per-platform concurrency caps** — YouTube and Kick `Semaphore` limits exposed in Preferences → Performance (matches Qt's `PerformanceSettings.youtube_concurrency` / `kick_concurrency`). → Ph 4
- [ ] **Debounced on-disk writes for channel flag changes** — favorite / auto-launch / dont-notify flips should debounce ~2 s before hitting disk to avoid I/O thrash on bulk operations. → new (minor; sits in `channels.rs`)
- [ ] **Error state on rows** — when a specific platform client errored, surface `error_message` in the row (small red dot + tooltip) instead of silently showing offline. → Ph 2a follow-ups
- [ ] **`YOUTUBE_MISS_THRESHOLD` tolerance** — Qt allows a transient scrape miss (count = 2) before declaring a secondary concurrent YT stream offline; avoids flapping in the multi-stream case. → Ph 2b (fold into the multi-stream item)

### Chat — core message handling

- [ ] **ROOMSTATE → chat-mode banners** — parse `slow`, `subs-only`, `emote-only`, `followers-only` (minutes; `-1` = off), `r9k` and surface as a dismissible banner row above the message list. → Ph 3
- [ ] **`/me` action messages** — detect `\x01ACTION …\x01` payload and render in italic with the username coloured like the body. → Ph 3
- [ ] **First-message (`first-msg=1`) highlight** — shipped? (check current chat rendering); if not, add a subtle left accent / tinted row. → shipped?
- [ ] **Sub-anniversary banner** — when the logged-in user's Twitch anniversary is detected via IRC, show a one-shot dismissible banner per billing cycle. → Ph 3
- [ ] **Custom highlight keywords** — user list of words that trigger the mention highlight style + optional notification. → Ph 3
- [ ] **Local echo for sent Twitch messages** — Twitch does not echo own PRIVMSGs; synthesise a local echo using `USERSTATE` tags so the user sees their own send immediately. → Ph 2b (sending)
- [ ] **Prediction badge tooltips** — parse the `predictions` badge version (`blue-1` etc.) and render a descriptive tooltip ("Predicted: Blue"). → Ph 3

### Chat — emotes

- [ ] **Animated emote rendering** — GIF/WebP frame extraction and per-frame-delay animation in both chat and emote picker, driven by a shared timer (don't start one timer per emote). → Ph 3
- [ ] **Provider toggles** — per-user toggles for 7TV / BTTV / FFZ independently (matches Qt's `emote_providers` setting). → Ph 4 Chat tab
- [ ] **Emote disk cache budget** — the current memory LRU + 500 MB disk cap from CLAUDE.md is shipped; expose the disk cap as a user-configurable setting (50–5000 MB). → Ph 4 General tab
- [ ] **Prefetch-on-picker-open** — when the emote picker opens, auto-download any missing channel emotes (prioritised over globals). → Ph 3
- [ ] **Channel vs Global separation in picker** — subsection header within each provider tab so users can tell sub-emotes from globals. → Ph 3 (fold into emote-picker item)
- [ ] **Twitch sub emotes via Helix** — `GET /chat/emotes/user?user_id=…` (paginated) lists every emote the logged-in user can use across their subscriptions. → Ph 2b (fold into "Twitch channel sub emotes" bullet)
- [ ] **7TV EventAPI live emote updates** — subscribe to add/remove events for the connected channel so the picker reflects upstream changes without a manual refresh. → Ph 3

### Chat — UX

- [ ] **In-chat search with predicates** — `Ctrl+F`; predicates: `from:username`, `has:link`, `has:emote`, `is:sub`, `is:mod`, `is:first`, `is:system`, `is:action`, `is:hype`; prev/next navigation with match counter. → Ph 3
- [ ] **Scroll pause on user scroll-up** — lock auto-scroll when the user scrolls away from the bottom; show a "paused — click to resume" indicator; auto-resume when the user scrolls back to bottom. → Ph 3
- [ ] **Link tooltip previews** — hover a URL to fetch the page `<title>` (first ~16 KB) in a background task and show it in a tooltip; 200-entry LRU. → Ph 3
- [ ] **Slow-mode countdown in composer** — when ROOMSTATE has `slow > 0`, render the countdown inside the send button / placeholder text. → Ph 3
- [ ] **Character counter in composer** — live remaining-characters display; limits: Twitch/Kick/Chaturbate 500, YouTube 200. → Ph 3
- [ ] **Up/Down message history cycling** — per-channel ring of recently-sent messages; arrow keys in an empty composer pop them back for edit+resend. → Ph 3
- [ ] **@mention autocomplete** — separate from general tab-completion; triggered by `@`, scoped to users seen in the current session. → Ph 3 (sub-bullet of "Tab completion for emotes + mentions")
- [ ] **Autocorrect** — the existing spellcheck bullet should also call out distance-1 Damerau-Levenshtein auto-correct with a brief 3 s green-underline display. → Ph 5 (fold into spellcheck item)
- [ ] **Adult-word dictionary** — bundle a `data/adult.txt` so common chat slang doesn't get false-positive spellcheck hits. → Ph 5 (fold into spellcheck)
- [ ] **Deleted-message display modes** — strikethrough / truncated / hidden; configurable per user. → Ph 3
- [ ] **Alternating row colors** — optional even/odd row background tinting in the chat list; separate tokens for dark + light themes; alpha-enabled. → Ph 4 Appearance tab
- [ ] **Per-channel chat tab colors** — user can assign a custom tint to each chat tab for visual channel recognition. → Ph 4
- [ ] **Configurable chat font size** — independent of the global UI scale (4–24 pt). → Ph 4 Chat tab
- [ ] **Configurable line spacing** — 0–20 px between messages. → Ph 4 Chat tab
- [ ] **Scrollback buffer cap** — max messages held in memory per channel (100–50 000, default 1 000). → Ph 4 Chat tab
- [ ] **Platform name colors toggle** — use Twitch's assigned user colour from IRC; let users opt out for colour-neutral reading. → Ph 4 Chat tab

### Chat — windows + persistence

- [ ] **Plain-text log format option** — alternate to JSONL; `[YYYY-MM-DD HH:MM:SS] Username: message` per line. → Ph 3 (fold into chat-logs item)
- [ ] **History-on-open** — when opening a channel's chat, load the last N lines from the disk JSONL log (configurable 10–1000, default 100) before the live stream resumes. → Ph 3 (distinct from third-party preload: local history persists across restarts, third-party only covers the last ~30 s)
- [ ] **Chat-log export to text file** — one-click export of the current buffer to a plain-text file. → Ph 3
- [ ] **Split view (two chats side-by-side)** — the Columns redesign in Ph 6 covers N columns; Qt's split view is specifically for two built-in chat tabs in one window. → Ph 6 (could fold in as a 2-column preset)
- [ ] **Chat window always-on-top** — independent of main window always-on-top; for popout chat windows. → Ph 4 / existing popout code
- [ ] **Whisper tab + banner** — whisper-specific chat tab list separate from channel tabs; main-window banner that flashes on inbound whisper (1 Hz for 60 s). → Ph 3 (extend the whispers bullet)

### Playback

- [ ] **yt-dlp launch method** — alternative to streamlink for YouTube and Chaturbate (some URLs don't resolve via streamlink). → Ph 5 (sibling to external-player picker)
- [ ] **Per-platform launch method picker** — Twitch → streamlink, YouTube → yt-dlp, Kick → streamlink, Chaturbate → yt-dlp (each independently configurable). → Ph 5
- [ ] **Stream quality picker per launch** — Source / 1080p / 720p / 480p / 360p / Audio-only; default configurable; also selectable per-launch via right-click. → Ph 5
- [ ] **Global + per-channel auto-play toggles** — global toolbar button ("A") + per-channel `auto_launch` flag; channels fire `launch_stream` automatically on offline→live transition. → Ph 5
- [ ] **"Playing" row indicator + stop control** — stream rows show a prominent playing indicator and a stop button while the `PlayerManager` has an active process for that key. → Ph 5
- [ ] **Streamlink console window** — optional in-app terminal window showing live stdout/stderr from the streamlink subprocess, with an "auto-close when process exits" option. → Ph 5
- [ ] **Record-while-watching** — already in Ph 5 as `--record PATH`; add the `record_directory` setting + a filename template (`{channel}_{timestamp}.ts`). → Ph 5 (fold)
- [ ] **Low-latency defaults per platform** — ship default `additional_args` of `--twitch-low-latency --kick-low-latency`; user can override. → Ph 5
- [ ] **Shell-metacharacter validation** for streamlink/player args — reject arguments containing `;`, `&&`, backticks, etc. in the Preferences form so the detached subprocess can never execute an attacker-crafted string. → Ph 5
- [ ] **Video preview on hover** — 320×180 HLS preview that plays when hovering a live row for > 400 ms; optional audio; 60 s preview-URL cache. Needs a lightweight in-app HLS player — possibly the same one we'd use for Ph 6 inline playback. → new (Ph 6-adjacent)

### Notifications

- [ ] **Smart no-flurry on startup** — suppress go-live notifications during the first refresh after launch; channels that were already live at startup are silently noted, only future transitions fire. → Ph 4 (fold into Desktop Notifications bullet)
- [ ] **Urgency levels** — Low / Normal / Critical (maps to notify-send priority on Linux / criticality level on other OSes). → Ph 4
- [ ] **Custom notification sound** — file picker for WAV / OGG / MP3 / FLAC / Opus; plays via `paplay`/`aplay`/`ffplay` on Linux, `winsound` on Windows, `NSSound` on macOS. → Ph 4
- [ ] **Notification timeout (0–60 s)** — configurable dismiss time, 0 = system default. → Ph 4
- [ ] **Per-platform notification filter** — independent toggles for Twitch / YouTube / Kick / Chaturbate. → Ph 4
- [ ] **Raid notifications** — desktop notification fires on Twitch USERNOTICE `msg-id=raid` on any connected chat, independent of go-live events. → Ph 3 / Ph 4
- [ ] **Mention notifications + distinct sound** — fire when `@{our_name}` appears in any connected chat; separate sound from go-live. → Ph 3 / Ph 4
- [ ] **"Watch" action button on notifications** — click to directly `launch_stream` the target channel without focusing the window. → Ph 4
- [ ] **Show-game / show-title toggles** — suppress game and/or title in the notification body for users who want the minimal "X is live!" form. → Ph 4
- [ ] **Notification backend selector** — auto / `desktop-notifier` D-Bus / `notify-send` subprocess (Linux); a manual fallback when the auto-detect picks the wrong one. → Ph 4
- [ ] **Flatpak-safe backend** — use `flatpak-spawn --host notify-send` when `FLATPAK_ID` is set so the sandbox doesn't swallow notifications. → Ph 4
- [ ] **Test-notification buttons** in Preferences (test live sound + test mention sound). → Ph 4

### Themes + appearance

- [ ] **Built-in theme presets** — beyond our Linear/Vercel mono default: High Contrast, Nord Dark, Monokai, Solarized Dark, Light. Each is a JSON under `~/.config/livestreamlist/themes/` loaded at startup. → Ph 4
- [ ] **Theme mode selector** — Auto (follows system `prefers-color-scheme`) / Light / Dark / High Contrast / Custom. → Ph 4
- [ ] **Theme cycle button** — small button in the titlebar that rotates through all available themes (matches Qt's toolbar button). → Ph 4
- [ ] **Theme JSON export/import** — separate from the full settings bundle; one-click share a theme file. → Ph 4 (fold into theme-editor item)
- [ ] **Main window always-on-top toggle** — View menu entry, persists across restarts. → Ph 4
- [ ] **UI Styles density cycle** — app-wide density modes (not just the sidebar): Default / Compact 1 / Compact 2 / Compact 3 scaling the toolbar, dialogs, and list rows. Separate from the per-sidebar density toggle proposed in Ph 3 follow-ups. → Ph 4

### Accounts + auth

- [ ] **Browser cookie import for YouTube** — primary auth path in the Qt app: read cookies directly from Chrome / Chromium / Brave / Firefox / Opera / Vivaldi / LibreWolf cookie stores. In Rust this is a `rookie`-style crate call. Flatpak builds can't reach browser data; document that manual paste is the only path there. → Ph 2b (fold into YouTube cookie-login bullet as path "(c)")
- [ ] **Expired-cookie auto-refresh prompt** — detect when YouTube cookies have expired (HTTP response signals); offer an in-app re-sign-in dialog. → Ph 2b (fold)
- [ ] **Multi-account switcher** — quick account switcher popup for managing multiple Twitch / Kick / YouTube logins per platform; stores multiple keyring entries keyed by account name. → new (Ph 4-adjacent, noted as not-yet-shipped in Qt's own roadmap)
- [ ] **Keyring graceful fallback** — if no keyring is available (headless machines, CI), store secrets in a 0600-mode JSON file under the config dir and log a clear warning. → Ph 2b

### Accessibility + comfort

- [ ] **Built-in High Contrast theme** — separate from the generic theme editor; ship as a preset with WCAG-AA contrast ratios and pure-black/pure-white backgrounds. → Ph 4 (fold into theme presets)
- [ ] **Streamer mode** — auto-detect a running OBS / other streaming software (PID / window-title scan) and mask usernames, whispers, and notification content until the user toggles back. Qt calls this out on their own roadmap but hasn't shipped it. → new
- [ ] **Chat scroll "paused" affordance** — accessibility value is making the pause state visible when the user has lost context on why new messages stopped arriving. → Ph 3 (fold with scroll-pause)

### Keyboard shortcuts

Current: `1`/`2`/`3` layout, `N` add, `R` refresh (Phase 1 shipped). Qt has many more. Proposed additions (Ph 3 follow-ups):

- [ ] `Ctrl+N` add channel (supersede `N` or keep both)
- [ ] `Ctrl+R` / `F5` refresh
- [ ] `Ctrl+,` preferences
- [ ] `Ctrl+Q` quit (with close-to-tray still honored if enabled)
- [ ] `Ctrl+F` in-chat search
- [ ] `Ctrl+E` emote picker
- [ ] `Ctrl+Shift+E` refresh emotes for current channel
- [ ] `Ctrl+W` new whisper
- [ ] `Ctrl+=` / `Ctrl+-` zoom channel-list font
- [ ] `Escape` cancel reply / close popup / exit selection mode
- [ ] `Delete` delete selected channel(s) in selection mode
- [ ] `Tab` emote completion in composer (scoped to composer focus)
- [ ] `@` start mention autocomplete
- [ ] `Ctrl+C` copy selected chat message

### Other — small polish items

- [ ] **Chaturbate private/hidden/group room detection** — `api/chatvideocontext` returns a room status beyond "online/offline"; dim the row + tooltip when the room is not in a public show. → Ph 2a follow-ups (fold into Chaturbate live-check)
- [ ] **Clickable category/game chip** in the title banner — opens the platform's category browse page. Watch out for Qt's `linkActivated` bug in rich text (anchor tags outside opacity wrappers). → Ph 3 (fold into title banner bullet)
- [ ] **Browser chat URL types** — Popout / Embedded / Default for Twitch when using the browser-chat fallback. → Ph 4 (fold into a new browser-chat subsection, lower priority)
- [ ] **Browser selection for external chat** — System Default / Chrome / Chromium / Edge / Firefox when the user opts for browser chat instead of built-in. → Ph 4
- [ ] **YouTube `/live` URL suffix** when opening a channel in the browser so it lands on the currently-active stream rather than the channel homepage. → Ph 1 (tiny fix to `open_in_browser`)
- [ ] **About dialog** — version string, license link, GitHub link, acknowledgements. → Ph 4
- [ ] **Reset-to-defaults button** in Preferences → General. → Ph 4
- [ ] **"Open log directory"** button that spawns `xdg-open`/`open`/`explorer` on the logs folder. → Ph 4 (fold into existing item)
- [ ] **App-level file logging** — rotating log file at `~/.local/share/livestreamlist/logs/` separate from chat logs; configurable level (INFO / DEBUG); toggleable; openable from Preferences. → Ph 4 (fold into the Developer tab item)

### Confirmed shipped (audited 2026-04-25)

- [x] **User cards** — `src/components/UserCard.jsx`: hover + click popover with pronouns, bio, follow age, badges, plus separate `UserHistoryDialog`, right-click context menu, nickname + note edit dialogs.
- [x] **Blocked users list + unblock action** — `src/components/PreferencesDialog.jsx` Chat tab; `src/ipc.js::list_blocked_users`. Purge-on-block-event hooked up in chat handlers.
- [x] **Badge cache + mod classification** — `src-tauri/src/chat/badges.rs::BadgeCache` with `classify_mod_*` helpers. Hover tooltip text (e.g. "6-Month Subscriber") still to verify against the Qt app's coverage.
- [x] **Chat visibility toggles** (badges, mod badges, timestamps) — exposed in `PreferencesDialog.jsx` Chat tab. (Per-mod-badge-subtype toggles not in scope today; punt to Ph 4 if wanted.)
- [x] **Timestamp 12h / 24h format** — `timestamp_24h` UI toggle wired in `PreferencesDialog.jsx`; rendering branches in `ChatView.jsx`.
- [x] **Favorites star in channel rail** — `Command.jsx` "Pin as favorite" context-menu entry, calls `set_favorite`.
- [x] **First-message (`first-msg=1`) highlight** — `chat/twitch.rs` parses `first-msg=1` into `is_first_message`; rendered with a visual treatment in `ChatView.jsx`.
- [x] **Emote 2x/4x srcset** — `EmoteText.jsx` builds 1x/2x/4x srcset; browser picks per device-pixel-ratio.
