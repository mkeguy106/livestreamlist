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

## Phase 2a follow-ups — live-status parity across platforms

Twitch is the hardest one and it's done; the others are straightforward ports.

- [ ] **YouTube live check** — subprocess `yt-dlp --dump-json --no-download https://youtube.com/@{handle}/live` (matches Qt app). Parse `is_live`, `title`, `concurrent_view_count`, `release_timestamp`, thumbnail. Batch size 5 to stay under yt-dlp throttling.
- [ ] **Kick live check** — `GET https://kick.com/api/v2/channels/{slug}`; map `livestream` object to `Livestream` struct. Use `start_time` for duration (UTC timezone required).
- [ ] **Chaturbate live check** — two-step: bulk `/api/ts/roomlist/room-list/?follow=true` (needs session cookie; Phase 2b) + individual `/api/chatvideocontext/{user}/` (public) to detect private/hidden/group rooms. For now support only individual.
- [ ] Wire each into `refresh.rs` alongside the existing Twitch branch; platform-specific concurrency caps

---

## Phase 2b — Chat sending + Kick chat + moderation

### Twitch auth + sending

- [ ] OAuth implicit flow — spawn system browser to `https://id.twitch.tv/oauth2/authorize?response_type=token&client_id=…&redirect_uri=http://localhost:65432&scope=chat:read+chat:edit+user:read:follows`
- [ ] Local loopback HTTP server on port 65432 captures the `#access_token=…` fragment (Tauri Rust can bind this; use `tiny_http` or raw `hyper`)
- [ ] Token stored via `keyring` crate (Secret Service / Keychain / Credential Manager)
- [ ] Twitch IRC connection switches from `justinfan*` + anonymous pass to authed nick/pass once token present
- [ ] `chat_send(uniqueKey, text)` invoke command; Twitch composer stops being a placeholder
- [ ] Moderation events: `CLEARCHAT`, `CLEARMSG`, `NOTICE msg-id=…` — `chat:moderation:{key}` event → frontend removes or greys matching messages
- [ ] Local echo handling (Twitch doesn't echo own PRIVMSG back; we fake it)

### Kick chat

- [ ] OAuth 2.1 + PKCE flow to `https://id.kick.com/oauth/authorize` (callback also on 65432, reuse the server)
- [ ] Pusher WebSocket: `wss://ws-us2.pusher.com/app/…`, subscribe to `chatrooms.{chatroom_id}.v2`
- [ ] Parse `App\Events\ChatMessageEvent` → `ChatMessage` with emote parsing (Kick uses `[emote:ID:name]` inline tokens, not tag-based)
- [ ] Kick does echo own messages via websocket → no local echo, skip-on-id-match
- [ ] Send via `POST https://api.kick.com/public/v1/chat` with `Authorization: Bearer {token}`, auto-refresh on 401

### Per-channel 3rd-party emotes

- [ ] Fetch 7TV channel emotes by Twitch user id (`GET /v3/users/twitch/{id}`)
- [ ] BTTV channel emotes by Twitch user id
- [ ] FFZ channel emotes by Twitch login
- [ ] Wire channel emote load into `chat_connect` so `EmoteCache::set_channel` is populated before message flow starts
- [ ] Twitch channel **sub emotes** — need auth; use Helix `GET /chat/emotes?broadcaster_id=…` once tokens are available

### YouTube + Chaturbate chat (embedded)

- [ ] Tauri's `webview` can create additional web-only windows — embed YouTube's native `/live_chat?…&is_popout=1` in an isolated webview, persisted cookie store
- [ ] Same pattern for Chaturbate's room page; inject the DOM-isolation CSS/JS from `livestream.list.qt`'s `chaturbate_web_chat.py`
- [ ] Sign-in flows: host a small `_YouTubeLoginWindow` / `_ChaturbateLoginWindow` that holds a cookie-tracking webview

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

---

## Phase 3 follow-ups — UX consistency + first-paint polish

These are small-scoped UX fixes that don't belong under a "phase" but should land together for a consistent polish pass.

- [ ] **Dark-mode first-paint** — the window currently flashes white on launch before the React bundle parses and paints. Set the Tauri window / `index.html` / `<body>` background to `--zinc-950` (our dark base) via a tiny inline `<style>` in the shell HTML and `"background_color"` in `tauri.conf.json` so the compositor maps the window on a dark surface from frame zero. No flash-bang at launch, especially on HDR / high-brightness monitors.
- [ ] **Global hover + focus audit** — sweep every interactive element and normalize hover / focus-visible styling against `tokens.css`. Known offenders: chat emotes, the account chip in the titlebar, the preferences icon, add-channel and refresh buttons, layout-switcher dots, column header controls. Currently several of them fall through to WebKit's native cursor / outline, breaking the visual identity.
- [ ] **Channel-list search** — a slim search input pinned at the top of the channel rail (Command layout) that filters the visible list live as the user types. Match by `display_name` and `channel_id`, case-insensitive. Use the existing `.rx-input` class so it lands without new design decisions.
- [ ] **Last-selected-channel memory** — persist `last_selected_channel_key` across runs. On launch: if that channel is live, select it and open its chat in the main pane. If it's offline, fall back to selecting the top entry of the (live-first, then favorites, then alpha) channel list.

---

## Phase 4 — Preferences / tray / notifications / single-instance

- [ ] **Preferences dialog** with the same 5 tabs as the Qt app: General / Playback / Chat / Appearance / Accounts
  - Tab layout as a second Tauri window (not modal) with hairline separators
  - Bind form state to `settings.json` in XDG config
- [ ] **Theme editor** — live color-picker for every zinc scale + accent; persist to `~/.config/livestreamlist/themes/*.json`
- [ ] **Desktop notifications** — Tauri's `tauri-plugin-notification`. Go-live events, whispers/DMs, quiet-hours window
- [ ] **System tray** — `tauri-plugin-tray`. Live count in tooltip, icon swap when anything live, right-click menu with refresh / show / quit
- [ ] **Single-instance guard** — `tauri-plugin-single-instance`. Second launch focuses existing window. CLI `--allow-multiple` bypass
- [ ] **Background mode** — close → hide to tray, don't quit
- [ ] Log viewer / "Open log directory"
- [ ] Import/export settings+channels to JSON (NOT tokens/cookies)
- [ ] **Developer tab + context-menu hardening** — the default WebKit right-click menu (back / forward / reload / Inspect Element) leaks development tooling to normal users. Disable it by default via `tauri.conf.json` + a `contextmenu` listener in the shell HTML. Add a new "Developer" tab in Preferences with two toggles (both default off): "Show developer context menu" (re-enables the native menu including Inspect Element) and "Verbose logging" (promotes `log::debug!` calls to stdout + writes a detailed trace log to `~/.local/share/livestreamlist/logs/debug.log` for issue reports).

---

## Phase 5 — Import follows / spellcheck / Turbo / release

- [ ] **Import follows from Twitch** — Helix `GET /channels/followed?user_id=…` (requires `user:read:follows`)
- [ ] **Import follows from Kick** — REST endpoint (auth required)
- [ ] **Import follows from Chaturbate** — bulk room-list endpoint with session cookies
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
