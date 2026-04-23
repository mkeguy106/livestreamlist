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

---

## Phase 5 — Import follows / spellcheck / Turbo / release

- [ ] **Import follows from Twitch** — Helix `GET /channels/followed?user_id=…` (requires `user:read:follows`)
- [ ] **Import follows from Kick** — REST endpoint (auth required)
- [ ] **Import follows from Chaturbate** — bulk room-list endpoint with session cookies
- [ ] **Spellcheck / autocorrect** — ship `hunspell` on Linux via the `hunspell` crate or subprocess; fall back to a pure-Rust `symspell` on Windows/macOS. Skip rules for emotes, URLs, mentions, all-caps. Red wavy underlines in composer; green underline on auto-correction for 3 s. Distance-1 Damerau-Levenshtein for auto; distance-≤ 2 for manual suggestions
- [ ] Adult word list (`data/adult.txt`) bundled to suppress false positives on chat slang
- [ ] **Twitch Turbo auth** — must use browser `auth-token` cookie (not the OAuth token — it's client-ID-bound). `Authorization: OAuth {cookie}` when passed to streamlink. Offer a login button that scrapes the cookie from a local WebView session
- [ ] Streamlink additional-args validator (must allow non-flag values like `debug` after `--loglevel`)
- [ ] Record-and-play support via `--record PATH`
- [ ] **Release pipeline** — GitHub Actions: build AppImage + `.deb` on Ubuntu, `.dmg` on macOS, Inno Setup `.exe` on Windows. Tag-driven (`v*`). One release at a time (GitHub Actions storage quota)
- [ ] Autoupdater — `tauri-plugin-updater` checking a manifest

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
