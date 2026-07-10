# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project Overview

Cross-platform desktop livestream monitor. Successor to [`livestream.list.qt`](https://github.com/mkeguy106/livestream.list.qt) (PySide6/Qt6), rewritten on **Tauri v2 + React + Rust**.

The single product surface is a desktop window with three switchable layouts (the top-left titlebar dots pick them): **Command** (sidebar rail + details pane), **Columns** (TweetDeck-style live columns), **Focus** (single featured stream + tab strip).

Visual identity is the Linear/Vercel mono aesthetic from the design bundle — zinc near-black, red live dots, Inter + JetBrains Mono, hairline 1 px borders on `rgba(255,255,255,.06)`, density 9. Platform accents (twitch/youtube/kick/chaturbate) are pale-desaturated, used only to mark provenance.

## Tech Stack

- **Frontend**: React 18, Vite 5, plain CSS variables in `src/tokens.css`
- **Backend**: Rust (stable, ≥ 1.77), Tauri 2, `reqwest` (rustls), `tokio-tungstenite` (WebSocket), `parking_lot`, `chrono`, `hunspell-rs` (system libhunspell) with bundled en_US.aff/.dic fallback under src-tauri/dictionaries/
- **Runtime**: Tauri's own async runtime wrapping Tokio — use `tauri::async_runtime::spawn`, never raw `tokio::spawn` from setup
- **IPC**: `invoke` commands (request/response) and `emit` events (push, topic-addressed)
- **Persistence**: JSON under XDG config

## Development Commands

```bash
# Install (one-time)
npm install

# Dev loop — hot-reloads frontend; Rust changes auto-rebuild
npm run tauri:dev

# Frontend-only dev (browser; IPC falls back to in-memory mocks)
npm run dev

# Frontend build
npm run build

# Production app build (produces AppImage / .deb / .rpm / .dmg / .exe installer)
npm run tauri:build
# → src-tauri/target/release/livestreamlist
# → src-tauri/target/release/bundle/*

# Rust tests (URL parse, IRC parse, emote scan)
cargo test --manifest-path src-tauri/Cargo.toml

# Rust check (fast) / clippy / fmt
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml

# IPC smoke-test harness — call any #[tauri::command] without launching the app
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --list
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- spellcheck_check '{"text":"helo","language":"en_US","channelEmotes":[]}'
# Single-shot: positional cmd + JSON args. JSONL streaming: no positionals, read stdin.
# Isolated config by default (--use-real-config to opt out); side-effects denied (--allow-side-effects to opt in).
# See src-tauri/src/smoke_harness/README.md for the full protocol.
```

Relaunch during Rust-side dev (Tauri watches `src-tauri/`; no explicit kill usually needed):

```bash
pkill -f "target/debug/livestreamlist"
pkill -f "tauri dev"
pkill -f "/bin/vite"
npm run tauri:dev
```

## Architecture

### Module structure

```
src/                         # React frontend
├── App.jsx                  # Titlebar + layout switcher + Add dialog + empty state
├── main.jsx                 # ReactDOM bootstrap
├── ipc.js                   # Tauri invoke + listenEvent wrappers with mock fallbacks
├── tokens.css               # Design tokens (zinc scale, platform colors, hairlines, utilities)
├── directions/              # The three top-level layouts
│   ├── Command.jsx
│   ├── Columns.jsx
│   └── Focus.jsx
├── components/              # Shared widgets
│   ├── AddChannelDialog.jsx
│   ├── WindowControls.jsx   # minimize/maximize/close (custom titlebar)
│   ├── ChatView.jsx         # message list + autoscroll + composer placeholder
│   └── EmoteText.jsx        # text + emote range → img substitution
├── hooks/
│   ├── useLivestreams.js    # 60s poll over refresh_all + initial snapshot
│   └── useChat.js           # chat_connect + chat:message:{key} subscription
└── utils/
    └── format.js            # viewers, uptime, platform letter

src-tauri/                   # Rust backend
├── Cargo.toml
├── tauri.conf.json          # decorations:false, identifier, window defaults
├── capabilities/default.json
├── icons/                   # auto-generated; regenerate via `cargo tauri icon <path>`
└── src/
    ├── main.rs              # Thin entrypoint → livestreamlist_lib::run()
    ├── lib.rs               # tauri::Builder wiring + invoke handlers + WebKit workaround
    ├── config.rs            # XDG paths, atomic_write helper
    ├── channels.rs          # Channel, Livestream, ChannelStore (disk + in-memory)
    ├── platforms/
    │   ├── mod.rs           # Platform enum + URL autodetect parser (unit-tested)
    │   └── twitch.rs        # GraphQL live-status client (batched ≤ 35 / request; chunks run concurrently, capped at 4 in-flight; 5-min 429 cooldown)
    ├── refresh.rs           # Orchestrates refresh_all across platforms
    ├── spellcheck/
    │   ├── mod.rs           # SpellChecker — per-language Hunspell cache, personal dict
    │   ├── tokenize.rs      # Pure tokenizer: Word / Mention / Url / Emote / AllCaps
    │   ├── personal.rs      # ~/.config/livestreamlist/personal_dict.json load/save
    │   └── dict.rs          # Enumerate /usr/share/hunspell etc. + bundled en_US fallback
    ├── streamlink.rs        # Detached subprocess spawn + browser handoff
    ├── mpv.rs               # MpvProcess spawn (--wid --vo=x11 --hwdec=auto-copy), JSON IPC control, monitor task
    ├── chat/
    │   ├── mod.rs           # ChatManager — one task per channel
    │   ├── models.rs        # ChatMessage, ChatUser, EmoteRange, ChatBadge
    │   ├── irc.rs           # IRCv3 parser + Twitch emote-tag parser
    │   ├── twitch.rs        # Anonymous WebSocket IRC client
    │   └── emotes.rs        # 7TV / BTTV / FFZ loaders + EmoteCache
    └── video/
        ├── mod.rs           # VideoManager — session map, spawn/stop/linger/reap, video:status emits; start_direct() hands the mpv backend (Linux) the raw streamlink URL, bypassing the CORS passthrough
        ├── session.rs       # VideoSession state machine (Starting/Serving/Lingering), generation-guarded
        ├── spawn.rs         # streamlink argv builder + free-port allocation (pure, unit-tested)
        └── passthrough.rs   # Per-session localhost CORS passthrough — one listener per session (own port) proxying streamlink's MPEG-TS to the webview, injects ACAO
```

### Async model (critical)

Tauri v2 has its own runtime abstraction (`tauri::async_runtime`) that wraps Tokio. The Tokio runtime is **not available inside `Builder::setup()`** — raw `tokio::spawn` calls from that context panic with `"there is no reactor running"`.

**Rule**: always use `tauri::async_runtime::spawn` for background tasks kicked off from Rust. It's runtime-agnostic and works from both `setup()` and invoke handlers.

Invoke commands marked `async fn` run on the Tauri runtime automatically — use of `reqwest` / `tokio-tungstenite` inside them is fine.

### IPC — invoke commands

Declared in `src-tauri/src/lib.rs` via `#[tauri::command]`, registered in `tauri::generate_handler![...]`. Called from the frontend via `invoke('name', { args })` in `src/ipc.js`.

| Command | Args | Purpose |
|---|---|---|
| `list_livestreams` | — | Cached snapshot of latest refresh |
| `list_channels` | — | Raw channel list |
| `add_channel_from_input` | `input: String` | Parse URL/handle, insert Channel, persist |
| `remove_channel` | `uniqueKey` | Remove + persist |
| `set_favorite` | `uniqueKey, favorite: bool` | Toggle + persist |
| `refresh_all` | — | Poll all platform clients, update store, return snapshot |
| `launch_stream` | `uniqueKey, quality?` | Detached `streamlink` subprocess with mpv |
| `video_start` | `uniqueKey, quality?` | Start (or resume / quality-switch) an inline-video session; returns `{ url }` — the localhost CORS-passthrough URL — once the streamlink child is ready. Errors are plain strings; a soft-cap rejection is prefixed `"cap:"` so the frontend renders an in-panel message instead of a generic error |
| `video_stop` | `uniqueKey` | Explicit stop (the ✕ control) — bypasses linger, kills the session and its streamlink child immediately |
| `video_backend` | — | Which inline-video backend this build/platform uses: `"mpv"` on Linux, `"mpegts"` elsewhere (Windows flips to mpv in a later slice). A hardcoded `cfg!(target_os = "linux")` check, not a user setting. Cached module-wide by `useVideoBackend`; `VideoPanel` renders `<MpvVideo>` or `<InlineVideo>` accordingly |
| `mpv_mount` | `uniqueKey, x, y, width, height, quality?, muted, volume` | Start (or resume) an mpv-backed session and mount its `GtkDrawingArea` surface at the given physical-pixel rect; idempotent on an already-mounted key (**resize-only** — will NOT restart mpv against a new URL, so quality switches must `mpv_unmount` first). Emits `mpv:status:{uniqueKey}` itself for the `starting`/`cap`/`error` states reached before the monitor task takes over. Linux-only; stubbed to an `Err` on other targets and in smoke/test builds |
| `mpv_bounds` | `uniqueKey, x, y, width, height` | Reflow an existing mpv surface (e.g. on layout change) |
| `mpv_set_visible` | `uniqueKey, visible: bool` | Hide/show one mpv surface (used for hover-occlusion and modal occlusion) — composes with mpv-confirmed readiness so a surface mpv hasn't painted yet is never shown |
| `mpv_unmount` | `uniqueKey` | Drop the mpv child; `MpvChild::Drop` SIGKILLs the process and destroys the `GtkDrawingArea`. The monitor task's socket-EOF then reports `consumer_dropped` to `VideoManager`, starting linger |
| `mpv_set_volume` | `uniqueKey, volume: f64` | Live volume (0.0–1.0 UI scale, converted to mpv's 0–100) over the mpv JSON-IPC socket — no pipeline restart |
| `mpv_set_muted` | `uniqueKey, muted: bool` | Live mute over the mpv JSON-IPC socket — no pipeline restart |
| `frontend_log` | `level, message` | Bridge a frontend diagnostic line into the Rust `log` crate (`error`/`warn`/`info`, prefixed `[frontend]`) so it reaches the `tauri:dev` terminal — WebKit console output is otherwise invisible. Used by `InlineVideo.jsx`'s perf watchdog |
| `open_in_browser` | `uniqueKey` | `xdg-open` / `open` / `start` on the channel URL |
| `chat_connect` | `uniqueKey` | Start per-channel chat task |
| `chat_disconnect` | `uniqueKey` | Abort that task |
| `embed_mount` | `uniqueKey, x, y, width, height` | Mount a YT/CB child webview at given physical-pixel rect; idempotent (resize if already mounted). Returns `false` if channel offline |
| `embed_bounds` | `uniqueKey, x, y, width, height` | Reflow an existing embed (e.g. on layout change) |
| `embed_set_visible` | `uniqueKey, visible: bool` | Hide/show one embed (used for modal occlusion) |
| `embed_unmount` | `uniqueKey` | Drop the child webview; the underlying GtkWidget destroys via `wry::WebView::Drop` |
| `chaturbate_login` / `chaturbate_logout` | — | Open the CB login popup / wipe profile dir + clear stamp |
| `twitch_web_login` | — | Open WebView popup at twitch.tv/login; capture + validate auth-token cookie; persist. Returns identity or rejects on mismatch with OAuth login |
| `twitch_web_clear` | — | Wipe keyring entries for twitch web cookie + identity |
| `twitch_anniversary_check` | `uniqueKey` | Returns `{ info: Option<SubAnniversaryInfo>, cookie_status: 'ok' \| 'missing' \| 'expired' }` so React reads cookie status synchronously without racing the event listener. GQL `subscriptionBenefit` query if share window open + not dismissed + setting on. Cached 6h (Some) / 5min (None). Cookie via `auth::twitch_web::stored_token` (auto-scraped at app launch via `rookie` from any installed browser when OAuth is logged in but no web cookie cached). Still emits `twitch:web_cookie_required` for mid-session expiry |
| `twitch_anniversary_dismiss` | `uniqueKey, renewsAt` | Persist `{channel: renewsAt}` in `chat.dismissed_sub_anniversaries`; resets next billing cycle when `renewsAt` changes |
| `twitch_share_resub_open` | `uniqueKey` | Open transient WebviewWindow at `twitch.tv/popout/{login}/chat` with shared web-cookie profile so user can click Twitch's native Share button. Idempotent (focus existing) |
| `twitch_share_window_close` | `uniqueKey` | Close the popout window for that channel (idempotent) |
| `spellcheck_check` | `text, language, channelEmotes` | Tokenize input + return `[{ start, end, word }, ...]` for misspellings (skips `@mentions`, URLs, emote codes, all-caps shorthand, personal-dict words) |
| `spellcheck_suggest` | `word, language` | Top 5 hunspell suggestions for a word |
| `spellcheck_add_word` | `word` | Append to `personal_dict.json`; returns `true` if newly added |
| `spellcheck_list_dicts` | — | Enumerate available dicts (`{ code, name }`) for the Preferences language dropdown |

### IPC — event topics

The Rust side emits events that React subscribes to via `listenEvent(name, handler)` (which wraps `@tauri-apps/api/event.listen`).

| Topic | Payload | Emitter |
|---|---|---|
| `livestreams:updated` | `Vec<Livestream>` (full snapshot, same shape as the `refresh_all` return) | `lib.rs::perform_refresh_all` (background scheduler + manual `refresh_all`) and the tail of `refresh_channel`; consumed by `useLivestreams` |
| `chat:message:{uniqueKey}` | `ChatMessage` | `chat/twitch.rs` per PRIVMSG |
| `chat:status:{uniqueKey}` | `ChatStatusEvent` | `chat/twitch.rs` on connect/disconnect/error |
| `chat:auth:chaturbate` | `{ signed_in, reason }` | `embed.rs::handle_chaturbate_auth_outcome` on every CB embed page-load — broadcasts auth-drift status |
| `video:status:{uniqueKey}` | `{ state: "starting" \| "serving" \| "ended" \| "error", message? }` | `video/mod.rs` — emitted on session start, readiness, linger-reap, and child-death/timeout |
| `mpv:status:{uniqueKey}` | `{ state: "starting" \| "playing" \| "cap" \| "ended" \| "error", message? }` | `lib.rs::mpv_mount` (emits `starting` before the streamlink spawn; `cap`/`error` if `start_direct` or the GTK/mpv spawn fails) + `mpv.rs::spawn_monitor` (emits `playing` on the first `playback-restart`/`file-loaded` IPC event, `ended` on clean EOF after real playback, `error` on an mpv playback error or a startup exit) |
| `chat:resub_self:{uniqueKey}` | `{ months, login }` | `chat/twitch.rs::build_usernotice` when own login broadcasts a `msg-id=resub` or `sub` USERNOTICE; consumed by `useSubAnniversary` for auto-dismiss |
| `twitch:web_cookie_required` | `{ reason: "missing" \| "expired" }` | `platforms/twitch_anniversary.rs::check` when the cookie is absent or rejected by GQL; consumed by `useSubAnniversary` to mount `<TwitchWebConnectPrompt>` |
| `twitch:web_status_changed` | `Option<TwitchWebIdentity>` | After Twitch web login or clear (`auth/twitch_web.rs`); consumed by `useAuth` and `useSubAnniversary` |

Events are strictly one-way (Rust → UI). UI never emits events; it calls invoke commands.

### Data flow

0. `ChannelStore::load()` reads `~/.config/livestreamlist/channels.json` into memory.
1. `App` mounts → `useLivestreams` invokes `list_livestreams` (instant cache), then `refresh_all` (network).
2. `refresh_all` runs each platform client in parallel (currently only Twitch GraphQL), merges into the store's `livestreams` map, returns a fresh snapshot.
3. The refresh **loop lives in Rust**: `lib.rs::spawn_refresh_scheduler` (started in `setup()` via `tauri::async_runtime::spawn`) loops `perform_refresh_all` → sleep `settings.general.refresh_interval_seconds` (re-read each cycle so Preferences changes apply without restart; clamped to a 10 s floor via `clamp_refresh_interval`). Every full-store refresh path — the scheduler, the manual `refresh_all` IPC, and the tail of `refresh_channel` — emits `livestreams:updated` with the snapshot. `useLivestreams` no longer polls; it seeds from `list_livestreams`, kicks off one `refresh_all` on mount, exposes a manual `refresh()`, and subscribes to `livestreams:updated`, funneling every snapshot through the pure `mergeSnapshots(prev, next)` helper (`src/utils/mergeSnapshots.js`) which reuses row/array references for unchanged channels so the Command sidebar doesn't re-sort/reconcile when nothing displayed changed. The scheduler and manual refresh share a `tokio::sync::Mutex` (`AppState::refresh_lock`): manual `lock().await`s (always runs, waits out an in-flight cycle), the scheduler `try_lock()`s (skips its tick when a refresh is already running). Tray "Refresh now" emits `tray:refresh-requested` → `App.jsx` calls `refresh()` → one `livestreams:updated` emit.
4. When a layout mounts `ChatView channelKey={k}`, `useChat(k)` invokes `chat_connect` and subscribes to `chat:message:{k}`.
5. `ChatManager::connect` spawns a task. `twitch::run` opens a WebSocket, sends `CAP REQ + NICK justinfan… + JOIN #…`, reads frames, parses IRC, emits events per PRIVMSG.
6. `EmoteCache` has globals loaded on app start (7TV + BTTV + FFZ); Twitch emote IDs come from the IRC `emotes=` tag and map directly to Twitch CDN URLs.

### Channel store (`src-tauri/src/channels.rs`)

- `Channel` — persisted: platform, channel_id, display_name, favorite, dont_notify, auto_play, added_at
- `Livestream` — transient: live/off, title, game, viewers, started_at, thumbnail_url, last_checked, error
- `unique_key` = `"{platform}:{channel_id}"` (the identifier used everywhere: storage, IPC, event topics, React keys)
- `ChannelStore` is held in `Arc<Mutex<…>>` (parking_lot) — no async locks; the store is memory-fast. The mutating methods (`add` / `add_many` / `remove` / `set_favorite` / `update_channel_display_name`) are **pure in-memory** — they no longer touch disk. Persistence happens **outside the lock** via the free `channels::persist(&store)` function: it serializes the channel list under the lock (a cheap in-memory clone via `serialize_channels`), drops the guard, then runs the blocking `atomic_write` to `channels.json`. Every mutation call site (in `lib.rs` and `refresh.rs`) calls `persist` after its mutation returns. Bulk imports use `add_many` (one lock acquisition, one `persist`) instead of an O(n) add-and-save loop.

### Chat architecture

`ChatManager` owns an `Arc<EmoteCache>` + a `Mutex<HashMap<unique_key, ConnectionHandle>>`. One tracked task per connected channel (the IRC/WebSocket read loop); abort is idempotent. The short-lived auxiliary tasks a connection fans out — the 3rd-party emote loader (`emote_loader::load_twitch_for_channel`, which now runs the user-emote stale-refresh sequentially at its tail rather than as a nested spawn) and the Twitch badge prefetches (global on connect, per-channel on ROOMSTATE) — are **tracked per connection** in `AuxTasks` (a cloneable `Arc<Mutex<Vec<JoinHandle>>>` shared between the `ConnectionHandle` and the connection task via `TwitchChatConfig.aux`), and `disconnect` aborts them alongside the main task, so rapid connect/disconnect churn no longer accumulates orphaned in-flight fetches. Aborting mid-fetch is safe: the loaders only write into the shared caches after a successful fetch completes.

Per-channel flow:
1. Frontend calls `chat_connect(uniqueKey)` when `ChatView` mounts
2. `ChatManager::connect` looks up the channel, spawns a task running `twitch::run`
3. Task emits `chat:status:Connecting` → connects WebSocket → emits `Connected` → reads lines
4. Each `PRIVMSG` → `build_privmsg` → `ChatMessage` with `emote_ranges` populated from both Twitch tags and 3rd-party word scanning → emit
5. On disconnect, task emits `Closed`. Frontend tears down on unmount.

### Embed architecture (YouTube / Chaturbate chat)

Twitch and Kick have native IRC / Pusher clients (`chat/`). YouTube and Chaturbate don't expose a usable real-time API to anonymous clients, so their chats are **embedded as third-party webviews living inside the main window's surface** — like Qt's `QWebEngineView` as a child widget.

The pre-rewrite approach (parked borderless top-level `WebviewWindow` chased with `set_position` IPC) is gone — see commit history if you need it. Today's model:

**Linux topology** (`src-tauri/src/embed.rs::linux::install_overlay`, runs once at startup):

```
GtkApplicationWindow
└── default_vbox (gtk::Box)
    └── GtkOverlay  (set_overlay_pass_through(fixed, true))
        ├── (base) WebKitWebView           ← React app, fills the overlay
        └── (overlay) gtk::Fixed           ← embed_host.fixed — child webviews go here
```

The pass-through bit is critical: without `set_overlay_pass_through(&fixed, true)`, the empty `gtk::Fixed` (sized to fill the overlay) intercepts every mouse event and the React UI stops accepting clicks. Pass-through forwards events on the Fixed itself to the React webview underneath; webviews placed *inside* the Fixed still capture their own input via their own GdkWindow.

**macOS / Windows topology**: Tauri's `WebviewWindow::add_child` (specifically `Window::add_child` — the method is on `Window`, not `WebviewWindow`) just works — no overlay setup needed.

**Rust types** (`src-tauri/src/embed.rs`):

- `EmbedHost` — singleton in app state, owns `Mutex<HashMap<EmbedKey, ChildEmbed>>` plus (Linux only) the `gtk::Fixed` from `install_overlay`. Public methods: `mount`, `set_bounds`, `set_visible`, `unmount`, `unmount_platform`.
- `ChildEmbed` — per-key entry. Fields: `platform, bounds, visible`, plus `inner: ChildInner` (gated `#[cfg(not(test))]` so HashMap-arbitration unit tests don't need GTK). Methods that touch `inner` are `#[cfg(not(test))]`.
- `ChildInner` — Linux: `(Arc<wry::WebView>, Box<wry::WebContext>)` (context owned so it drops with the embed; webview field first so it drops first). Non-Linux: `tauri::webview::Webview`. `unsafe impl Send + Sync` on the Linux variant — GTK access is gated to the main thread by call sites; the Mutex serializes lookups.
- `EmbedKey` = the same `unique_key` flowing through chat IPC (with the optional YT `:video_id` suffix from the multi-stream scraper).

**Construction (Linux)** — `build_linux::build_child`:
- Per-platform `data_directory` via `wry::WebContext::new(Some(profile_dir))` then `WebViewBuilder::new_with_web_context(&mut ctx)`. `wry 0.54` moved data_directory off the builder onto WebContext — this is non-obvious. The `WebContext` is **owned, not leaked**: `build_child` / `build_import_child` return `(Arc<wry::WebView>, Box<WebContext>)` and the Box is stored as field 1 of `ChildInner`, so the context drops when the embed unmounts. Tuple-field order in `ChildInner` is load-bearing — the WebView (field 0) must drop before the context (field 1); Rust drops fields in declaration order, so the `webview.destroy()` in `InnerWebView::Drop` runs while the context is still alive. Dropping the Rust `WebContext` wrapper is safe even though the WebView keeps using the profile: `WebKitWebView` holds its own GObject ref on the underlying `WebKitWebContext`, and we register no custom URI schemes (the one thing wry's docs say needs the `WebContext` kept alive).
- `with_visible(false)` on the builder; the on_page_load handler shows on `PageLoadEvent::Finished`. Same dark-first-paint discipline as the rest of the app (PR #70 lesson).
- `with_background_color((9, 9, 11, 255))` — zinc-950, so any in-flight repaint stays dark.
- `WebViewBuilderExtUnix::build_gtk(&fixed)` is the Linux-only build path. Tauri's own `add_child` is broken on Linux (it parents into `default_vbox()`, a `gtk::Box`, which ignores `set_position`/`set_size` — see [tauri#9611](https://github.com/tauri-apps/tauri/issues/9611)) so we go around it.

**On-page-load wiring**:

The wry 0.54 `with_on_page_load_handler` callback signature is `Fn(PageLoadEvent, String) + Send + Sync` — it does NOT receive the WebView reference. To call methods on the webview from inside the callback (show, eval CSS injection, run auth-drift verifier), we thread a `Weak<wry::WebView>` through an `Arc<OnceLock<...>>`:

```rust
let cell: Arc<OnceLock<Weak<wry::WebView>>> = Arc::new(OnceLock::new());
let cell_for_handler = cell.clone();
let handler = move |event, _url| {
    if !matches!(event, PageLoadEvent::Finished) { return; }
    let Some(weak) = cell_for_handler.get() else { return; };
    let Some(wv) = weak.upgrade() else { return; };
    let _ = wv.set_visible(true);
    if let Some(js) = injection_for(platform) { let _ = wv.evaluate_script(&js); }
    if platform == Platform::Chaturbate { verify_chaturbate_auth_linux(&wv, &app); }
};
let webview = builder.with_on_page_load_handler(handler).build_gtk(&fixed.0)?;
let webview_arc = Arc::new(webview);
let _ = cell.set(Arc::downgrade(&webview_arc));
```

**The Weak is critical**. A strong `Arc<OnceLock<Arc<wry::WebView>>>` would create a cycle (`WebView → callback registry → closure → Arc → same WebView`) — strong count would never reach zero on unmount, `InnerWebView::drop`'s `webview.destroy()` (wry's `webkitgtk/mod.rs:96-99`, which detaches the GtkWidget from the Fixed) would never run, and the embed would visually persist after the React side correctly called `embed_unmount`.

**CSS / DOM injection** (`embed.rs::injection_for`):
- YouTube: `YT_THEME_CSS` — a `<style>` tag forcing zinc-950 backgrounds + custom scrollbar styling on `yt-live-chat-renderer` etc.
- Chaturbate: `CB_ISOLATE_JS` — finds the chat container (`#ChatTabContainer` / `#defchat` / fallback `.chat-holder`), tags ancestors `data-lsl-path`, injects a CSS rule that hides every `body>*` except the chat path. Re-injected on every page load (navigations wipe the JS context).

**Chaturbate auth-drift** (`verify_chaturbate_auth_linux` / `_other`):

After every Chaturbate embed `PageLoadEvent::Finished`, read the `sessionid` cookie via `WebView::cookies_for_url("https://chaturbate.com/")`. Three outcomes:
- Cookie present → `auth::chaturbate::touch_verified()`, emit `chat:auth:chaturbate { signed_in: true, reason: "ok" }`
- Missing + stamp present → drift; `clear_stamp_only()` (NOT full `clear()` — the embed window is mid-load against the profile dir, full wipe would `remove_dir_all` under WebKit's feet). Emit `{ signed_in: false, reason: "session_expired" }`
- Missing + no stamp → emit `{ signed_in: false, reason: "not_logged_in" }`

`classify_chaturbate_auth` is the pure helper; it's the only part with unit tests (`embed.rs::classify_*` tests).

**Frontend** (`src/components/EmbedLayer.jsx` + `src/components/EmbedSlot.jsx`):

- `<EmbedLayer modalOpen={anyDialogOpen}>` is mounted once at App.jsx scope. It's the **only** component that calls `embed_*` IPC. Owns a `Map<EmbedKey, { refs: Map<slotId, {ref, active}> }>` registry.
- `<EmbedSlot channelKey isLive active>` is mounted by `ChatView` for YT/CB platforms. Renders a placeholder `<div>` with `position: relative; overflow: hidden`, registers itself with the layer via `EmbedLayerContext`. A `ResizeObserver` chain on the placeholder's ancestors triggers reflow on layout changes.
- The layer arbitrates: for each `EmbedKey`, it picks the active slot's `getBoundingClientRect()` as canonical and dispatches `embed_mount` / `embed_bounds`. When no slot for a key is active, `embed_set_visible(key, false)`. When the last slot for a key unregisters, `embed_unmount(key)`.
- App-level modal state (`addOpen || prefsOpen || nickDlg.open || ...`, OR-ed into one `anyDialogOpen` boolean) flows in via the `modalOpen` prop. The layer's `useEffect(modalOpen)` flips visibility on every mounted embed. This replaces the old singleton `embedSetVisibleAll(false)` API.

**Per-platform profile isolation** is preserved verbatim from the Qt predecessor:
- `~/.local/share/livestreamlist/webviews/youtube/` for the YT profile (cookies, cache, IndexedDB)
- `~/.local/share/livestreamlist/webviews/chaturbate/` for the CB profile
- Login popups (`auth/youtube.rs::login_via_webview`, `auth/chaturbate.rs::login_via_webview`) use the same `data_directory` as the embeds → cookies persist on disk and the embed picks them up automatically.
- Logout (`auth::*::clear()`) calls `EmbedHost::unmount_platform(platform)` first, THEN `remove_dir_all(profile_dir)`. This ordering matters — wiping the dir while an embed is still loading against it crashes WebKit. The Chaturbate flow has a `clear_stamp_only()` variant for the auth-drift case where the embed is mid-load and we just want to flip the stamp without touching the profile dir.

**Multi-embed**: the HashMap-keyed model means N concurrent embeds is a first-class feature, not a workaround. The Columns layout shows one embed per visible YT/CB column, all rendering simultaneously. The pre-rewrite single-`Option<CurrentEmbed>` ceiling is gone.

### Inline video (Phase 6 slice 2)

Twitch-only inline playback: one streamlink child per playing channel serving MPEG-TS over a localhost port, bridged to the webview by a **per-session** CORS passthrough listener (`video/passthrough.rs` — one listener/port per session so streams never share a libsoup connection pool, round 6); `InlineVideo.jsx` is the only mpegts.js consumer. See the module tree (`video/`) and the video pitfall rows (incl. the libsoup 6-connection starvation row).

**Settings** (`settings.rs::VideoSettings`, all serde-defaulted): `default_quality` (default **`"best"`**; options `best` / `1080p60` / `720p60` / `720p` / `480p`; governs the Focus layout's featured stream and anywhere without a more specific default), `column_quality` (default **`"720p60"`**, round 5 — Columns-layout videos; see quality resolution below), `max_concurrent` (6), `linger_seconds` (60), `use_twitch_auth` (true), `autoplay_columns` (**true** — live Twitch columns auto-start on group-open / column-add; the header ⏹ / `InlineVideo` onClose stops one for that mount only via a local `sessionStopped` flag, never touching the persisted `channels[key].on` flag), `autoplay_unmuted` (**true** — autoplayed columns start unmuted unless a per-channel persisted `muted` wins), `dmabuf_renderer` (**false** as of round 7 — set `WEBKIT_DISABLE_DMABUF_RENDERER=1` by default; the round 4 default-true experiment produced a black window under real app load on NVIDIA/KDE, see the pitfalls row). `channels: HashMap<unique_key, ChannelVideoState { on, volume, muted, quality }>`.

**Quality resolution** (round 5): the Rust side (`video/mod.rs::start`) always resolves `quality_override.or(per_channel_quality).unwrap_or(default_quality)` — it has no notion of "column" vs "Focus". The variant-specific default lives entirely in `InlineVideo.jsx::resolveDefaultQuality`: the Focus variant passes `null` as the override (so Rust's own per-channel → `default_quality` resolution applies, "best" out of the box), while the column variant explicitly resolves `chan.quality ?? settings.video.column_quality ?? '720p60'` and passes THAT as the override on every `startSession` call site (mount, transient-startup auto-retry, manual retry — `pickQuality`'s explicit user pick is unaffected). This was a bandwidth fix: live telemetry at 4+ concurrent column streams showed delivery starvation (near-zero decode, latency collapsed to ~0, download speed collapsed) — a bandwidth cliff from each stream pulling ~6 Mbps at "best" in a 240–600 px column. The perf heartbeat's WARN/INFO lines (below) append `q=<quality>` so terminal telemetry confirms what each stream is actually pulling.

**Playback profiles** (`InlineVideo.jsx`, round 4): three cushioned mpegts.js configs, chosen ONCE at player-creation time from `variant` + `muted` — Focus → `FOCUS_CONFIG` (maxLatency 3, minRemain 1); muted column → `COLUMN_MUTED_CONFIG` (6 / 2); audible column → `COLUMN_UNMUTED_CONFIG` (4 / 1.5). All share `BASE_CONFIG`: `enableStashBuffer: true`, `liveBufferLatencyChasing: true`, and `CLEANUP_CONFIG` (`autoCleanupMaxBackwardDuration: 12 / Min: 6`, round 7). Jank correlation drove the stash-on cushion: the prior round's zero-stash / 0.5 s-min-remain profile starved into a latency-chase stutter because WebKitGTK blocks MSE appends while `SourceBuffer.remove()` runs. Round 4's 60/30 cleanup window turned out to be the deeper bug, not a fix: WebKitGTK enforces a per-process MSE memory quota shared across every player, and a 40-55 s retained back-buffer was eating that quota until appends silently stalled (telemetry: `span=38-57s latency≈0.1s` while download held full bitrate) — worse with more concurrent streams dividing the same budget. Round 7 shrank the window to 12/6 (live monitoring never seeks backward, so minimal back-buffer is safe) to keep quota available for forward appends. Deliberate tradeoff: ~1–3 s more end-to-end latency from the stash cushion, unrelated to the cleanup window.

**Seamless mute** (round 4): `toggleMute` only flips `element.muted` + state + persists — **no pipeline swap**. A prior round recreated the player on mute-change to switch profiles, which made the stream visibly stop/resume; the owner disliked it. Consequence: a stream created muted keeps the lazier muted profile (≤6 s latency) after unmuting until its next natural recreation (channel switch, quality change, watchdog rebuild).

**Perf diagnostics** (`InlineVideo.jsx` → `frontend_log` IPC): the perf watchdog samples dropped/decoded frames every 10 s while playing and routes two lines into the Rust `log` crate (WebKit console output is invisible in `tauri:dev`): a throttled WARN (>5% window drop, ≤1/30 s) and a 60 s INFO heartbeat (`dropped=d/total span=Ns ranges=N latency=Ns q=<quality>`, latency = `buffered.end − currentTime`, round 5; `ranges=video.buffered.length` added round 7 — buffered-range fragmentation is the other signature of the WebKit MSE quota starving appends). `q=` — and the overlay quality-menu label — read `sessionQualityRef`, **frozen inside `startSession` at each real session start**, not the live settings resolution: editing "Column quality"/"Default quality" in Preferences mid-playback must not relabel a session still pulling the old quality. This is how the next choppiness report becomes diagnosable from the terminal — including whether a stutter correlates with a stream still pulling "best".

**Readiness probe**: `start()` waits for the streamlink port before returning the URL. On Linux the probe is **passive** (`video/mod.rs::port_is_listening` reads `/proc/net/tcp` for a `0100007F:<port>` LISTEN row) — never a `TcpStream::connect`, because streamlink's single-threaded HTTP server (listen backlog 1) would accept the probe as a client and race the real fetch into a refused window (the transient `networkError/HttpStatusCodeInvalid` on the 2nd+ video). `InlineVideo.jsx` also auto-retries (500/1000/2000 ms, ≤3) a NetworkError that arrives before any frame is decoded.

**Popout hand-off**: `InlineVideo.popout()` stops the inline session and launches mpv, holding a `popout` poster ("Starting external player…") until `usePlayerState` reports the external player live — then the column unmounts and Focus shows a `popped` resting state with a "Play inline" button.

**mpv backend (slice A)**: on Linux, inline video is played by an embedded `mpv` process rather than mpegts.js-over-MSE — see `mpv.rs` and the `embed.rs` `ChildInner::Mpv` variant. Backend selection is a build-time/OS check (`video_backend` IPC, `cfg!(target_os = "linux")`); `VideoPanel.jsx` reads it via `useVideoBackend()` and renders `<MpvVideo>` (mpv) or the pre-existing `<InlineVideo>` (mpegts.js) accordingly, so Windows/macOS and the smoke/test builds are unaffected. `EmbedLayer.jsx` branches every mount/bounds/visibility/unmount call on the slot's `backend` prop (`mpv_*` IPC vs the existing `embed_*` IPC) and adds two mpv-only mechanisms: **`occludeKey`**, a per-key hide (distinct from the global `useEmbedOcclusion`) so hovering one column's video to reveal its DOM control strip (mute/volume/quality/pop-out/stop) doesn't blank every other video or chat embed — only the global modal path still hides everything; and **`remountKey`**, an explicit unmount-then-reflow used for both quality switches (mount is idempotent-resize-only, so changing quality requires killing and respawning mpv against a new streamlink URL) and the Retry button after a stream death (the monitor task's socket-EOF path unmounts the mpv child on the Rust side, but `EmbedLayer`'s client-side `mountedKeys` set is unaware, so a plain reflow would take the "already mounted" branch and silently no-op forever — `remountKey` unmounts first, a safe no-op Rust-side, before reflowing into a genuine fresh `mpv_mount`). mpv rides the same `VideoManager` linger/cap/generation machinery as the mpegts backend: `embed::mount_mpv` calls `consumer_connected` before the monitor task can observe an exit, and the monitor's socket-EOF handler calls `consumer_dropped`, so an mpv session lingers (streamlink stays warm) exactly like a browser-passthrough one. App-exit reap kills both process families: `RunEvent::Exit` calls `VideoManager::stop_all()` (streamlink) and, on Linux, `EmbedHost::stop_all_mpv()` (mpv) — `MpvProcess` also sets `PR_SET_PDEATHSIG=SIGKILL` at spawn so an abrupt parent death (crash, `SIGKILL`) can't orphan mpv even when neither `Drop` nor `RunEvent::Exit` runs.

### The three layouts

- **Command** — selected-channel workflow. Sidebar rail shows all channels (live first, then offline alpha). Main pane shows the selected channel's header + chat.
- **Columns** — parallel-monitoring workspace (Phase 6 slice 1). User-curated **groups only** (named, switchable, persisted in `settings.columns`) hold channel sets incl. offline channels; no group selected → a lightweight chooser with inline group creation (the transient "Live now" dynamic group was removed post-ship — mounting a chat per live channel was overwhelming at scale). Per-column resize (240–600 px, persisted per channel), add-column picker, per-column × remove, header-drag reorder (mouse pattern), clear-all. Each column is a full compact-variant `ChatView`; YT/CB columns mount embeds via `EmbedSlot`. In-React popups over embed regions must call `useEmbedOcclusion(open)` (EmbedLayer.jsx) — native embeds render ABOVE the React surface and hide anything beneath them. **Auto-fit width** (`settings.columns.auto_fit_width`, default true): when the visible column SET changes (group open/switch, add, remove — NOT during a width drag or manual resize; the effect's dep is the joined key list, not the widths), the window resizes (debounced 300 ms) to `Σ widthFor(k) + 2px/col`, clamped to `[720, monitor.logicalWidth × 0.95]`, only acting when the delta exceeds 24 px. Skipped when not in Tauri, disabled by setting, no group active, or the window is maximized. Preferences → Appearance → "Columns layout".
- **Focus** — single-stream reader mode. Tab strip of all channels across the top; split 60/40 with video placeholder / chat.

All three share the same data hook (`useLivestreams`). Each has its own chat binding: Command/Focus use one `ChatView` for the selected/featured channel; Columns mounts one `ChatView` per visible column.

Selection state (`selectedKey`) lives in `App`. Layout choice persists to `localStorage` under `livestreamlist.layout`.

### Command layout — variable-driven sidebar (PR #82)

The Command layout's rail responds to four user-tunable settings driven by a CSS-variable + data-attribute contract on `<html>`. **No JSX branches on collapsed/density/position state** — all four affordances are either CSS-attribute-selected or rendered conditionally based on a single `settings?.appearance?.command_sidebar_collapsed` read.

**Settings shape** (in `src-tauri/src/settings.rs::AppearanceSettings`, all serde-defaulted):

| Field | Type | Default | Range |
|---|---|---|---|
| `command_sidebar_position`  | `String` | `"left"`        | `"left"` \| `"right"` |
| `command_sidebar_width`     | `u32`    | `240`           | clamped to 220–520 on read in JS |
| `command_sidebar_collapsed` | `bool`   | `false`         | — |
| `command_sidebar_density`   | `String` | `"comfortable"` | `"comfortable"` \| `"compact"` |

**Settings → DOM bridge** (a `useEffect` in `App.jsx` after `usePreferences()`'s destructure): writes three `data-sidebar-*` attributes and one `--cmd-sidebar-w` CSS variable on `document.documentElement`. The bridge is the **single writer** — `tokens.css` reads, never writes.

```js
root.dataset.sidebarPosition  = a.command_sidebar_position === 'right' ? 'right' : 'left';
root.dataset.sidebarCollapsed = a.command_sidebar_collapsed ? 'true' : '';
root.dataset.sidebarDensity   = a.command_sidebar_density === 'compact' ? 'compact' : 'comfortable';
const w = a.command_sidebar_collapsed
  ? 40
  : Math.max(220, Math.min(520, Number(a.command_sidebar_width) || 240));
root.style.setProperty('--cmd-sidebar-w', `${w}px`);
```

Two non-obvious bits:

1. `--cmd-sidebar-w` is declared on **`:root`**, not `.rx-root`. The latter would shadow the variable for all descendants because `<div class="rx-root">` is a *descendant* of `<html>`, and CSS-var inheritance gives a descendant's own declaration precedence over any value flowing in from `:root`. The collapsed-width override (`:root[data-sidebar-collapsed="true"] { --cmd-sidebar-w: 40px }`) and the drag-handle's `style.setProperty` would both be dead. Caught in code review during PR #82 and worth re-checking if anyone moves the var declaration.
2. When `command_sidebar_collapsed` is true, the bridge writes **40 px directly** rather than relying on the CSS rule. Inline style on `:root` always beats class selectors targeting the same element (specificity), so the bridge has to write the right value itself when it's the source of width. The collapsed-state CSS rule is now redundant-but-harmless documentation of intent.

**CSS Grid layout** (`tokens.css` — at the bottom under "Command layout (A) — variable-driven sidebar"):

```css
.cmd-row {
  display: grid;
  grid-template-columns: var(--cmd-sidebar-w) minmax(0, 1fr);
  grid-template-areas: "sidebar main";
}
:root[data-sidebar-position="right"] .cmd-row {
  grid-template-columns: minmax(0, 1fr) var(--cmd-sidebar-w);
  grid-template-areas: "main sidebar";
}
```

Sidebar/main get `grid-area` assignments. Position swap = template-areas swap. Border side flips via the same data-attribute selector. Active-row indicator on `.cmd-row-item.active` flips from `border-left` to `border-right` so the 2 px solid bar always sits on the **outer edge** of the row (the side touching the main pane).

**Class names on Command markup** (in `src/directions/Command.jsx`):

| Class | Purpose | Hidden when collapsed? |
|---|---|---|
| `.cmd-row` | grid wrapper | — |
| `.cmd-sidebar` | rail (`grid-area: sidebar`, `position: relative` for resize anchor) | — |
| `.cmd-main` | chat pane (`grid-area: main`) | — |
| `.cmd-rail-header` | rail's top header (Channels chiclet / density toggle / live count / refresh / chevron) | — but contents JSX-conditional |
| `.cmd-toolbar` | filter / sort / hide-offline icons row | yes |
| `.cmd-search` | channel search input | yes |
| `.cmd-row-item` | individual channel button (3-col grid: `10px 1fr auto`) | yes |
| `.cmd-row-item.active` | active-row state (background + outer-edge border) | n/a |
| `.cmd-row-text` | center column wrapper (name row + meta line) | yes |
| `.cmd-row-meta` | game/offline line + viewers cluster | yes (also hidden by compact density) |
| `.cmd-add` | "Add channel" button | yes |
| `.cmd-resize-handle` | drag-resize strip on the rail's inner edge | yes |
| `.cmd-collapse-chevron` | the chevron itself | always visible |

The collapsed-mode hide rules use `display: none !important` because several of those elements have inline `style={{ display: 'flex', ... }}` that beats class selectors. Important is the standard fix when the override target is an inline-styled element.

**Drag-resize handle** (`DragResizeHandle` inline in `Command.jsx`): mouse-event-based per the `TabStrip.jsx::TabStrip` canonical pattern. **Never use HTML5 dnd** — `dragenter`/`dragover` are unreliable on WebKitGTK (existing pitfall, see Pitfalls section). State is `useState`, listeners are managed via `useEffect([drag])` so the drag survives Alt-Tab / focus loss (cleanup runs when the component unmounts or `drag` resets to null), and Esc cancels the drag (restores start width without persisting). Body cursor + `userSelect` are saved-and-restored so concurrent drags from `TabStrip` aren't clobbered. Sign-flip handles right-mode: `next = startW + (isRight ? -dx : dx)`. Persists on mouseup via `usePreferences().patch`.

**Collapse chevron** (`CollapseChevron`): reads `collapsed` and `isRight` from `settings.appearance.*` (NOT from `document.documentElement.dataset` — the dataset is updated by the bridge **after** render commits, which would leave the chevron's glyph + tooltip visually stale by one frame on every toggle). Glyph rotates per a 4-state table (left-mode + expanded → points left, etc.). Tooltip uses `align="right"` so the popup extends leftward into the rail rather than off-screen. The chevron's CSS class has `order: 99` (left-mode) / `order: -1` (right-mode) so it floats to the rail's inner edge — but in collapsed mode the rail header's other items are JSX-conditionally not rendered (Tooltip wrapping made the previous `> *:not(.cmd-collapse-chevron)` selector brittle), and `:root[data-sidebar-collapsed="true"] .cmd-rail-header { justify-content: center; }` centers the lone chevron.

**Density toggle**: same setting (`command_sidebar_density`) flipped from two places — the Sidebar density row in Preferences (a segmented Comfortable / Compact control), and a small `DensityIconBtn` next to the "Channels" chiclet in the rail header (icon shows two horizontal lines for Comfortable, three for Compact; active state when compact). Both call `patch()` against the same field.

**Collapsed-state UX cuts** made during PR #82's smoke testing (worth knowing if reverting / extending):

- Original spec proposed a 48 px icon-rail with platform-letter chips next to live dots. In person the dot+chip column read as a barcode (too many marks competing for attention without the channel name to disambiguate). Shipped as a 40 px chevron-only strip with the channel list completely hidden — standard "minimize sidebar" pattern.
- `--cmd-row-h` and `--cmd-row-fs` variables were declared and overridden but no rule consumed them (density actually works through a `:root[data-sidebar-density="compact"] .cmd-row-meta { display: none }` + tightened padding override). Variables removed during code review per YAGNI.
- The dual-chip pattern (`.cmd-row-chip-collapsed` rendering a second chip outside `cmd-row-text` so it stayed visible when `cmd-row-text` hid in collapsed mode) was removed when the chip-in-collapsed UX was cut. If reintroducing icon-rail, the dual-chip pattern is the right approach — keeps the chip's expanded-mode position unchanged.

**ContextMenu viewport-clamp** (`src/components/ContextMenu.jsx`): the right-click menu auto-flips off the right/bottom edge of the viewport. Position is `useState`-owned (NOT imperative `el.style.left/top`) and computed in **`useLayoutEffect`** so the clamp runs synchronously after DOM mutation but before paint — eliminates the visible flicker the previous `useEffect` + imperative-style approach had, and prevents React's reconciler from clobbering the fix on subsequent re-renders. Edge buffer is 8 px. Right-click on a channel row no longer auto-opens its chat tab; "Open chat" lives in the menu instead (PR #82).

### Design tokens (`src/tokens.css`)

Everything that's colorful or sized is a CSS var. Categories:

- Zinc scale (11 stops, `--zinc-950` through `--zinc-100`) — all chrome
- `--live` (`#ef4444`) for live dots; `--ok`, `--warn` for status
- Platform accents: `--twitch`, `--youtube`, `--kick`, `--cb`
- Typography: `--font-sans` (Inter + system fallback), `--font-mono` (JetBrains Mono + system fallback)
- Type scale: `--t-9` through `--t-16`
- Radii: `--r-1` (3 px), `--r-2` (4 px, "radius of the app"), `--r-3` (6 px)
- Hairlines: `var(--hair)` = `1px solid rgba(255,255,255,.06)`
- Reusable classes: `.rx-root`, `.rx-titlebar`, `.rx-btn`, `.rx-btn-primary`, `.rx-btn-ghost`, `.rx-input`, `.rx-chiclet`, `.rx-kbd`, `.rx-mono`, `.rx-plat.{t,y,k,c}`, `.rx-live-dot`, `.rx-status-dot`

Inline styles are used liberally for one-off layout — consistent with the prototype designs. If a pattern recurs, promote it to a class.

### Hover-discoverable text — always use the themed `Tooltip`

**Never use the native `title=""` HTML attribute** for hover-discoverable text on buttons, icons, links, or any other interactive element. WebKitGTK renders native tooltips with the system's white-on-light styling, which breaks the zinc-950 dark design. We've re-fixed this multiple times across PRs (#86, #90 — Re-dock + Find toolbar) and want it to stop recurring.

**Always wrap interactive elements with `<Tooltip text="…">`** (`src/components/Tooltip.jsx`) — zinc-925 background, mono font, themed to the design system. Set `aria-label` on the element to mirror the tooltip text for screen readers.

Canonical pattern (`Command.jsx::IconBtn`):
```jsx
<Tooltip text={title}>
  <button type="button" aria-label={title} onClick={onClick}>…</button>
</Tooltip>
```

For elements near the right edge of the viewport (rightmost icon in a titlebar, last item in a column), pass `align="right"` so the popover anchors its right edge to the trigger and doesn't overflow off-screen. `align="left"` is the mirror for elements near the left edge.

When reviewing or writing any new feature that has buttons, icons, or hover-text affordances, audit for `title=""` and replace with `Tooltip` — and check `aria-label`-only elements for whether they should also have a themed tooltip for visual discoverability.

### Spellcheck overlay (PR 2 — `src/components/SpellcheckOverlay.jsx`)

The chat Composer's red squiggles use a transparent-text overlay layered on top of the existing `<input>`. The input keeps all its existing behavior (typing, autocomplete popup, caret, paste, undo); the overlay renders the same text in `color: transparent` with `<span class="spellcheck-misspelled">` wrapping each misspelled range. CSS `text-decoration: underline wavy` survives `color: transparent` (decorations are styled independently of color per the CSS spec), so the squiggles are visible even though the overlay's text is invisible. The input's actual text shows through from the layer below.

**Sync mechanics**:
- The overlay's font / padding / line-height / letter-spacing are copied from the input via `getComputedStyle` in `useLayoutEffect` (synchronous before paint, no flash of misalignment).
- A `ResizeObserver` re-copies on input resize (the composer flexes to fill the row; system fonts settle late after first paint).
- A `scroll` event listener on the input mirrors `scrollLeft` so when text overflows and the input scrolls horizontally, the overlay's squiggles track with it. Applied via `transform: translateX(-${scrollLeft}px)` on the overlay (transform is GPU-cheap; preserves subpixel precision).
- Overlay has `pointer-events: none` so right-clicks, drags, and selections all reach the input below.

**Why not contenteditable**: would require rewriting the autocomplete popup, caret tracking (`input.selectionStart`), and `onChange` handling. Contenteditable is also notoriously buggy (caret jump on programmatic edits, paste sanitization, IME composition). The overlay pattern is the standard "highlight while typing" approach (Slack, Linear, etc).

**Hook contract** (`src/hooks/useSpellcheck.js`):
- Inputs: `text, enabled, language, channelEmotes`
- Output: `{ misspellings: Array<{ start, end, word }> }`
- 150 ms debounce (matches Qt). Cleared on every text change and unmount.
- Stale-response guard: each check kickoff increments a `requestIdRef`; in-flight responses compare against the current value before applying — so a slow IPC return for old text never overwrites a fresh result.
- `enabled === false` (preference off, or channel not authed) clears `misspellings` immediately and skips the IPC call.

**Composer wiring**: `Composer.jsx` wraps the `<input>` in `<div style={{ position: 'relative', flex: 1, minWidth: 0 }}>`. The `minWidth: 0` is critical so the flex child can shrink below content size — without it the @me chiclet and Browser button get pushed out of the row at narrow widths. The overlay only renders when `spellcheckEnabled && authed`, so the disabled (logged-out) state shows no squiggles.

### Spellcheck autocorrect (PR 3 — `src/utils/autocorrect.js`, hook extension)

Autocorrect logic is a **pure decision function** in `src/utils/autocorrect.js`. The function `shouldAutocorrect({ word, suggestions, isPast, caretInside, alreadyCorrected, personalDict })` returns the replacement string (e.g. `"the"` for `"teh"`) or `null` if the conditions aren't all met. Conditions are ported verbatim from the Qt app's `chat/spellcheck/checker.py::_run_check`:

1. **Caret not inside the word** — the cursor-position guard, NEW in this port, fixes the Qt bug where editing a previously-corrected word would re-fire autocorrect on every keystroke. `caretInside === true` → `null`.
2. **`isPast === true`** — the next char after the word is space + alpha (user moved on).
3. **`!alreadyCorrected.has(lowercased word)`** — per-Composer-session memory of words we've already auto-corrected.
4. **`!personalDict.has(lowercased word)`** — same for the persistent personal dict.
5. **Confidence**: apostrophe expansion (`dont→don't`), single hunspell suggestion, OR top suggestion within Damerau-Levenshtein ≤ 1.

Module-scope DEV asserts (matching the `commandTabs.js` pattern) cover every condition + the bug regression: `te` with caret inside should NOT fire even when `te` looks like a confident misspelling. These run on import in `npm run dev` / `npm run tauri:dev`.

**Hook extension** (`useSpellcheck.js`):
- `recentCorrections: Map<positionKey, { start, end, word, originalWord }>` — for the green pill overlay. Auto-pruned 3.1 s after each correction.
- `alreadyCorrected: Set<string>` — lowercased session memory.
- `recordCorrection({ originalWord, replacementWord, position })` — Composer calls this when it applies a rewrite.
- `undoLast(): { originalWord, replacementWord, position } | null` — Esc handler. Only returns non-null if (a) there's a recorded correction, (b) within 5 s, (c) no keystrokes since the correction (`keystrokesSinceCorrectionRef === 0`).
- `clearRecent()` — Composer calls on `channelKey` change.

**Green pill** (`.spellcheck-corrected` in `tokens.css`):
- `rgba(60, 200, 60, 0.12)` translucent fill + `rgba(60, 200, 60, 0.6)` 1 px border + 3 px border-radius (mockup D from the brainstorm).
- `@keyframes spellcheck-corrected-fade` holds at full opacity for 80% of the 3 s animation, then fades to transparent over the last 20%. CSS-only; no JS timer needed for the visual. The hook's 3.1 s setTimeout removes the span entirely after the animation completes (3 s + 100 ms safety).

**Composer wiring** (`Composer.jsx`):
- New `caret` useState, updated in `onChange` / `onKeyUp` / `onClick`.
- `useEffect` on `[text, misspellings, alreadyCorrected, recordCorrection]` looks for a misspelled word that meets `shouldAutocorrect`'s conditions. The cursor-position guard is `rangeAtCaret(misspellings, caret)` — that range is skipped.
- When a correction fires, `runAutocorrectFor` (top-level helper) awaits `spellcheckSuggest` IPC, re-confirms conditions against `inputRef.current.value` (text may have changed during the await), applies the rewrite via `setText` + `setCaret` + `requestAnimationFrame(() => el.setSelectionRange(...))`, and calls `recordCorrection`.
- One correction per pass — break out of the loop after the first. The next render's misspellings naturally re-evaluate.
- Esc keydown (when popup is closed) calls `undoLast()`; if it returns a restoration, Composer rewrites text to put `originalWord` back at `position`.

### Spellcheck right-click menu (PR 4 — `src/components/SpellcheckContextMenu.jsx`)

Right-click on a misspelled word OR a green-pill (recently-corrected) word in the chat composer pops the themed `ContextMenu` (the same one used by the channel rail's right-click menu, viewport-clamping per PR #82).

**Hit-test pattern**: Composer's outer `<form>` has `onContextMenu={onContextMenu}`. The handler calls `document.elementsFromPoint(x, y)` and looks for an element with `class="spellcheck-misspelled"` or `class="spellcheck-corrected"`. Both classes carry `data-word` (and `corrected` also carries `data-original`). Composer matches the word back to its range via `misspellings` or `recentCorrections` (first-match semantics — multiple instances of the same word in a single message resolve to the first occurrence).

**Menu contents** (`SpellcheckContextMenu`):
- `misspelled`: top-5 hunspell suggestions (fetched async via `spellcheck_suggest` IPC; "Loading…" placeholder while in flight) + separator + `Add "word" to dictionary` + `Ignore in this message`.
- `corrected`: `Undo correction (revert to "originalWord")`.

**Per-message ignore set** (`useSpellcheck.markIgnored` / `clearIgnored`): Composer-session-scoped `Set<string>` (lowercased). Words in the set are filtered out of `misspellings` BEFORE the array is exposed to the overlay or autocorrect. The set is cleared on (a) successful message send (after `chatSend` + `setText('')`), (b) channel switch (alongside `clearRecent`). Not persisted; not language-scoped.

**"Add to dictionary"** calls `spellcheck_add_word` IPC (PR 1). The Rust side appends to `~/.config/livestreamlist/personal_dict.json` and updates the in-memory `PersonalDict`. The next debounced `spellcheck_check` (within 150 ms) naturally drops the word from `misspellings` because Rust's `SpellChecker::check` applies the personal dict server-side. No client-side mirror of the dict is needed.

**Manual suggestion-apply**: clicking a suggestion item rewrites text via `setText` + `setCaret` + `requestAnimationFrame(setSelectionRange)` (matching the autocorrect rewrite pattern). Also calls `recordCorrection` so the word shows the green pill briefly — manually-chosen corrections are visually equivalent to autocorrected ones.

**`undoCorrection(positionKey)`** is distinct from `undoLast()`: undoLast only undoes the most recent autocorrect (Esc handler); undoCorrection takes a specific position key (the same key used by `recentCorrections.set()`) and undoes that specific entry. Used by the right-click "Undo correction" item which can target any visible green pill, not just the most recent.

### Spellcheck Preferences (PR 5 — `PreferencesDialog.jsx::SpellcheckSection`)

Three rows at the top of the Chat tab in Preferences:
- **Enable spellcheck** — `settings.chat.spellcheck_enabled` (default `true`). When off, the SpellcheckOverlay unmounts entirely (Composer's conditional render); the hook clears `recentCorrections` + `alreadyCorrected` so pills/squiggles disappear immediately.
- **Auto-correct misspelled words** — `settings.chat.autocorrect_enabled` (default `true`). **Chained-disable**: when spellcheck is off, this toggle is `disabled` and shown greyed; the hint text changes to "Requires spellcheck to be enabled." When spellcheck is on but autocorrect is off, squiggles still render but Composer's autocorrect effect bails before any rewrite.
- **Language** — `settings.chat.spellcheck_language` (default = system locale via `default_lang()` in `settings.rs`, falls back to `en_US`). Dropdown options fetched on mount via `spellcheck_list_dicts` IPC; cached in component-local state. Disabled when spellcheck is off OR while the IPC is in flight.

**On language change**: `useSpellcheck`'s reset effect (deps `[language, enabled]`) clears `recentCorrections` + `alreadyCorrected`. The next debounced `spellcheck_check` (within 150 ms) re-evaluates against the new dictionary, so misspelled-vs-correct flags update naturally.

### Sub-anniversary detection + share (PRs #104–#109)

Detects when a logged-in Twitch user has a sub anniversary in the ~8-day "share window" and surfaces a banner above the chat composer. Click "Share now" → an in-app signed-in popout of Twitch's native popout chat → user clicks Twitch's Share button → Twitch fires a `USERNOTICE` → our IRC client observes it → banner auto-dismisses + popout auto-closes. Achieves Qt parity with two meaningful UX improvements: cookie capture is automatic (no manual login when user is already signed in to Twitch in any major browser) and the share popout is in-app (no cross-browser-login dependency).

**Why a separate web cookie**: `gql.twitch.tv/subscriptionBenefit` rejects Helix OAuth bearer tokens; only the web `auth-token` cookie that twitch.tv sets at login authenticates that GQL query. Same constraint Qt has. Tauri's existing OAuth flow (`auth::twitch`) uses Helix and remains the chat-IRC auth — these are separate accounts in principle and mismatch detection makes that explicit.

**Backend modules**:

| Module | Responsibility |
|---|---|
| `auth/twitch_web.rs` | Web cookie capture via `rookie` browser-scrape (`extract_from_browser`) at app launch, OR via WebView popup login (`login_via_webview`) as fallback. Validates via cheap GQL `CurrentUser` query; persists token + identity to keyring (`twitch_browser_auth_token` / `twitch_web_identity`). Mismatch detection compares web-login to OAuth-login at capture time. |
| `platforms/twitch_anniversary.rs` | GQL `subscriptionBenefit` query + 6h/5min TTL cache + pure `compute_window` (renews_at + now → days remaining in share window) + pure `parse_response`. Returns `CheckResult { info: Option<SubAnniversaryInfo>, cookie_status: 'ok' \| 'missing' \| 'expired' }` so the React hook reads cookie status synchronously without racing the event listener. |
| `share_window.rs` | Transient `WebviewWindow` opening `https://www.twitch.tv/popout/{login}/chat` with shared profile dir at `~/.local/share/livestreamlist/webviews/twitch_web/`. `Mutex<HashMap<channel_login, WebviewWindow>>` registry. On open, an init script injects the captured cookie into the WebView's cookie jar via `document.cookie` + `window.location.reload()` (no-op if profile dir already has the cookie — happens after first share or after a manual `login_via_webview`). |
| `chat/twitch.rs::build_usernotice` | When a USERNOTICE arrives with `msg-id ∈ {resub, sub}` AND `login.eq_ignore_ascii_case(&own.login)`, emits `chat:resub_self:{channel_key}` event with `{months, login}` payload. Drives the auto-dismiss path on the React side. |

**App-launch auto-scrape** (`lib.rs::setup`): if Twitch OAuth identity is present AND no web cookie cached, spawn a background task via `tauri::async_runtime::spawn` that calls `extract_from_browser()` (rookie) → `validate()` (GQL ping) → `save_pair()`. Silent on failure — lazy WebView fallback handles Flatpak / unsupported browsers. Mirrors Qt's `gui/app.py:306-312::extract_twitch_auth_token` flow.

**IPC surface** (registered in `lib.rs`): `twitch_web_login`/`twitch_web_clear` (PR 1), `twitch_anniversary_check`/`twitch_anniversary_dismiss` (PR 2), `twitch_share_resub_open`/`twitch_share_window_close` (PR 3). The check command returns a `CheckResult` struct with both info AND `cookie_status` so React renders the right UI state synchronously off a single IPC response.

**Events emitted to React**:
- `chat:resub_self:{unique_key}` `{ months, login }` — own-resub USERNOTICE observed; consumed by `useSubAnniversary` for auto-dismiss
- `twitch:web_cookie_required` `{ reason: "missing" \| "expired" }` — emitted by `check()` for mid-session expiry (initial check is handled synchronously via `cookie_status` in the IPC response; this event remains for the case where the cookie expires AFTER `useSubAnniversary` has been mounted)
- `twitch:web_status_changed` `Option<TwitchWebIdentity>` — emitted on auto-scrape success, manual login, and clear

**Frontend** (`src/`):

- `hooks/useSubAnniversary.js` — owns `info` + `connectPromptVisible` state; three event listeners; reads `cookie_status` from IPC response + sets `connectPromptVisible` synchronously (race-free); uses an `infoRef` to avoid stale-closure issues in the `chat:resub_self` listener.
- `components/SubAnniversaryBanner.jsx` — pinned-above-composer banner: ⭐ + months + display name + `Share now` (.rx-btn-primary) + × dismiss. Subtle purple tint (Twitch sub color).
- `components/TwitchWebConnectPrompt.jsx` — lazy-mounted alternative when `cookie_status` is missing/expired AND the user hasn't connected via Preferences yet. One-shot per app session (Connect / Not now). Calls `twitchWebLogin()` (PR 1's WebView popup).
- `components/ChatView.jsx` — mounts `<SubAnniversaryBanner>` and `<TwitchWebConnectPrompt>` immediately above the composer. Banner takes priority over connect prompt if both could be visible.
- `components/PreferencesDialog.jsx` — Chat tab toggle for `chat.show_sub_anniversary_banner`; Accounts tab "Twitch web session" row showing the captured identity + Disconnect button.

**Settings** (`settings.rs::ChatSettings`):
- `show_sub_anniversary_banner: bool` (default `true`)
- `dismissed_sub_anniversaries: HashMap<String, String>` — keyed by `unique_key`, value is `renews_at` string. Naturally resets next billing cycle because `renews_at` changes.

**Auto-dismiss flow** (the satisfying bit):
1. User clicks `Share now` → popout opens, signed in via captured cookie
2. User clicks Twitch's native Share button + types optional message + submits
3. Twitch broadcasts USERNOTICE on IRC (`msg-id=resub`)
4. Our chat task is already connected (per-channel via `useChat`); `build_usernotice` emits `chat:resub_self:{channel_key}`
5. React listener fires: persists per-cycle dismissal via `twitch_anniversary_dismiss`, closes popout via `twitch_share_window_close`, clears local state — banner unmounts
6. The USERNOTICE itself still flows through the normal chat-message path so the user sees their resub fanfare in the chat stream

**Caveats / known limitations** (documented in spec):
- Twitch only (Kick has no equivalent share affordance — confirmed via spike during brainstorming)
- 30-day cycle assumption — annual subs return a meaningless `days_remaining_in_window` (no crash, just nonsensical math)
- Cross-client share (sharing on phone while our IRC isn't connected) → banner naturally clears when window closes (~8 days) or via × dismiss; no immediate auto-dismiss

## Configuration

Data dir (XDG):
- Linux: `~/.config/livestreamlist/`
- macOS: `~/Library/Application Support/livestreamlist/`
- Windows: `%APPDATA%\livestreamlist\`

Files:
- `channels.json` — persistent channel list
- `settings.json` — reserved for Phase 4 (preferences)
- `personal_dict.json` — user-added words for spellcheck (lowercase-normalized; `{ "version": 1, "words": [...] }`)
- Chat logs, emote disk cache, auth tokens — reserved for Phase 3+

## Known Pitfalls

| Issue | Fix |
|---|---|
| `tokio::spawn` inside `Builder::setup()` panics: *"no reactor running"* | Use `tauri::async_runtime::spawn` everywhere. Raw tokio works inside `#[tauri::command] async fn` but not in setup |
| WebKitGTK crashes to a fully BLACK WINDOW on NVIDIA + KDE Wayland when the dmabuf renderer is on | `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set by default in `lib.rs::apply_linux_webkit_workarounds` (`video.dmabuf_renderer` defaults **false**). Round 4 briefly flipped this default to true off a bare-window spike (`docs/superpowers/spikes/2026-07-07-inline-video-playback-spike.md`) that measured ~4x cheaper video painting and no crash on WebKit 2.52 — but round 7's live telemetry under real app load reproduced the black window twice on the owner's NVIDIA/KDE box; forcing `WEBKIT_DISABLE_DMABUF_RENDERER=1` restored rendering. **The workaround is CONFIRMED still required by default** — the spike's bare-window test was not representative of the full app's WebKit surface. `video.dmabuf_renderer = true` (Preferences → Video → "Use GPU dmabuf renderer") remains available as an opt-in experiment for systems that don't hit the crash; a user export of the env var always wins either way (`set_if_unset` never overrides). If other WebKit weirdness hits, try `GDK_BACKEND=x11` |
| Vite silently switches port from 5173 → 5174 when 5173 is busy; Tauri's `devUrl` then points at nothing and WebKit shows a blank error | `strictPort: true` in `vite.config.js` so Vite fails loud |
| Twitch `emotes=` tag indices are **char** (Unicode scalar), not bytes | Convert char → byte in `chat/twitch.rs::char_range_to_bytes` before slicing the UTF-8 message |
| Tauri v2 drag regions don't honor CSS `-webkit-app-region: drag`, and the `data-tauri-drag-region` attribute's injected listener is unreliable on Linux/WebKitGTK | `src/hooks/useDragRegion.js::useDragHandler` — manual `mousedown` handler calling `getCurrentWindow().startDragging()`. Skips drags when `closest('button, input, …')` matches. Double-click → `toggleMaximize()` |
| `decorations: false` removes the native titlebar — window controls are gone too | Custom buttons in `WindowControls.jsx` call `getCurrentWindow().minimize/toggleMaximize/close` |
| Environment detection: no `window.__TAURI__` in v2 | Check `window.__TAURI_INTERNALS__` instead |
| `anyhow::Error` is not `Serialize` — can't return directly from `#[tauri::command]` | Map to `String` via `err_string` helper |
| `#[derive(Default)]` on a Rust enum requires `#[default]` on the chosen variant | Platform enum marks `Twitch` as default (arbitrary but overwritten everywhere it matters) |
| App launched from a long-running terminal session may not raise on KDE Wayland | `lib.rs::run` stages `set_always_on_top(true)` before `show()` and clears it via a deferred (~150 ms) tokio task in `window_state::raise_to_front_deferred`. Maps the window in the topmost layer, beating focus-stealing prevention. If a launch still loads behind, set "Focus Stealing Prevention: None" in KWin settings |
| Native Wayland clients cannot read or set absolute window position | The protocol does not expose global coordinates to clients, so `outer_position` always returns `(0, 0)` and `set_position` is ignored. `tauri-plugin-window-state` cannot persist or restore position on a native Wayland session. `lib.rs::apply_linux_webkit_workarounds` sets `GDK_BACKEND=x11` (if the user hasn't already overridden it) so the app runs on Xwayland, where position persistence works correctly. To run native-Wayland anyway, set `GDK_BACKEND=wayland` — but accept that window position resets to compositor-chosen placement on every launch |
| Tauri's `WebviewWindow::add_child` is broken on Linux | Parents into `default_vbox()` (a `gtk::Box` that ignores `set_position` / `set_size` / `bounds`). Maintainer parked the issue ([tauri#9611](https://github.com/tauri-apps/tauri/issues/9611), Apr 2025). On Linux we bypass `add_child` entirely and use wry directly — `WebViewBuilderExtUnix::build_gtk(&fixed)` into a `gtk::Fixed` we own (see `embed.rs::install_overlay`). macOS / Windows `add_child` works; only Linux needs the workaround. Note `add_child` is on `Window<R>`, not `WebviewWindow<R>` — `app.get_window("main")` returns the right type |
| Empty `gtk::Fixed` overlaid on the React webview swallows all input | `gtk::Fixed` (sized by the parent `GtkOverlay` to fill the overlay area) intercepts every mouse event, breaking right-click, the custom titlebar drag region, and every UI click. Fix: `overlay.set_overlay_pass_through(&fixed, true)` — events landing on the empty Fixed forward to the React webview. Webviews placed *inside* the Fixed still get input via their own GdkWindow |
| Strong `Arc<wry::WebView>` captured by `with_on_page_load_handler` closure creates an Arc cycle | The closure is held by the WebView's internal callback registry. `WebView → closure → Arc → same WebView` keeps strong count at ≥1 forever; `InnerWebView::Drop` (which calls `webview.destroy()` and detaches the GtkWidget from the Fixed) never runs. Symptom: switching from a YT/CB channel to Twitch leaves the old embed visible. Fix: store `Weak<wry::WebView>` in the OnceLock and `Arc::downgrade(&webview_arc)` after `build_gtk`; upgrade inside the callback for the brief lifetime of the call |
| `wry 0.54` moved `data_directory` from `WebViewBuilder` to `WebContext` | `WebViewBuilder::with_data_directory(...)` no longer compiles. Use `WebContext::new(Some(profile_dir))` then `WebViewBuilder::new_with_web_context(&mut ctx)`. WebContext borrows only for the builder's lifetime — `build_gtk` consumes the builder and ends the borrow, so keep an owned `Box<WebContext>` and store it next to the WebView (see `ChildInner`). Don't `Box::leak` it — that leaks one context per mount |
| `webview.destroy()` requires the wry strong count to actually reach zero | `InnerWebView::Drop` (`wry-0.54.4/src/webkitgtk/mod.rs:96-99`) is what removes the GtkWidget from its parent — there's no public API to do it manually short of reaching for the underlying webkit2gtk widget. So any structural change that introduces an Arc cycle on a `wry::WebView` will manifest as "the embed visually stays after I call unmount." Audit Arc/Weak relationships carefully when wiring callback closures around WebViews |
| **HTML5 drag-and-drop is broken in WebKitGTK** | `dragstart` fires (it's WebKit-internal) but `dragenter`/`dragover`/`drop` are never delivered to JS — GTK's drag-drop machinery captures events before they reach the webview. Standard workarounds don't help: `text/plain` shim alongside the custom MIME, container-level event delegation via `closest('[data-tab-key]')`, and `dragDropEnabled: false` in `tauri.conf.json` all leave dragover dead. **For drag UX, use mouse events instead of HTML5 dnd.** See `src/components/TabStrip.jsx::TabStrip` for the canonical pattern: `onMouseDown` arms a drag with source key + start coords; document-level `mousemove`/`mouseup` listeners (added via `useEffect` while a drag is armed) track the cursor and use `document.elementFromPoint(...).closest('[data-tab-key]')` for drop-target identification; a movement threshold distinguishes click from drag; `mousedown` calls `e.preventDefault()` to suppress text-selection initiation; body cursor + userSelect are locked while active to prevent visual bleed onto neighboring UI |
| `EmbedSlot`'s register-effect must NOT include `active` in its dep array | The `active` prop on `<EmbedSlot active={isActiveTab}>` flows through TWO `useEffect`s: a `register` effect (registers the slot with `EmbedLayer`) and a separate `updateActive` effect (calls `layer.updateActive(...)` on changes). If `active` is in the register effect's deps, every change runs cleanup → setup, which calls `unregister` then `register`. With the chat-tab system having exactly one slot per channelKey, `unregister` hits the `entry.refs.size === 0` branch in `EmbedLayer` and fires `embedUnmount`, destroying the wry `WebView` via `wry::WebView::Drop`. The subsequent `register` triggers a fresh `embedMount` — the user sees the YT/CB chat reload on every tab switch. The register effect's deps must be `[channelKey, isLive, layer]` only; the `active` flag's actual change is handled by the separate `updateActive` effect, which doesn't unregister. (Fixed in PR #80; documented inline at `src/components/EmbedSlot.jsx`.) |
| Locally-generated chat IDs that get persisted to disk MUST include a per-process nonce | `chat/twitch.rs::SELF_ECHO_SEQ: AtomicU64` is process-global and resets to 0 on every app launch. Self-echo `ChatMessage`s persist to the chat log file, so on next session `replay_chat_history` can load `self-0` from the previous run into React's `bufferRef`. The new session's first send → echo gets fresh `self-0` → `useChat`'s id-dedup drops the new one as an apparent duplicate → user's first message after restart silently fails to render, even though it sent successfully to Twitch. Fix (PR #110): `SELF_ECHO_PREFIX: OnceLock<String>` initialized to Unix-millis hex on first call; IDs become `self-{prefix}-{N}`. Same pattern needed for any future locally-generated message IDs that flow through both the chat log and the dedup buffer |
| WebKitGTK reliably wedges one of several MSE pipelines created **simultaneously** (readyState 4, buffer full, zero frames decoded — see the inline-video spike) | Every pipeline creation — including watchdog rebuilds — flows through `src/utils/videoQueue.js::enqueuePipelineCreation`, a module-scope queue that spaces creations ~400 ms apart app-wide, so at most one is ever in flight. `InlineVideo.jsx`'s wedge watchdog keys on `getVideoPlaybackQuality().totalVideoFrames` frozen across 2 ticks (1.5 s each) while `readyState >= 3 && !paused` — **never `currentTime`**, because `liveBufferLatencyChasing` keeps nudging `currentTime` even on a wedged pipeline, masking the freeze |
| streamlink's `--player-external-http` server sends no `Access-Control-Allow-Origin` header, so the webview can't fetch it directly | `src-tauri/src/video/passthrough.rs` proxies `GET /video/{unique_key}` to the session's streamlink port and injects `Access-Control-Allow-Origin: *`. streamlink must stay in its **default continuous mode** — it's the only mode verified to reconnect cleanly after a watchdog-triggered pipeline rebuild. Stream-end is detected **client-side** (mpegts.js `LOADING_COMPLETE`, which calls `video_stop`), not from the child process — an unexpected streamlink exit is treated as `error`, not as normal end-of-stream |
| Six concurrent inline videos sharing ONE passthrough origin starve on rebuild ("playback pipeline stalled repeatedly") | WebKitGTK's libsoup caps concurrent connections at **~6 per (scheme, host, port)**. When every stream fetched from the same `http://127.0.0.1:{shared}/video/{key}` origin, six live streaming fetches pinned the whole pool; a watchdog rebuild's new fetch then queued behind a dying connection and starved. Fix (round 6): **one passthrough listener per session** — `passthrough::serve_session` is spawned per session on its own `127.0.0.1:0` port (stored as `VideoSession::public_port`; the URL becomes `http://127.0.0.1:{public_port}/video/{key}`), so no two streams ever share a connection pool. The listener is bound in `start()` right after the streamlink spawn (outside the sessions lock — never hold parking_lot across an async bind) and its `JoinHandle` lives in `VideoSession::listener_task`; `kill()` aborts it, so every session-removal path (stop / sweep reap / sweep died / quality switch / readiness failure / Drop / stop_all) tears the listener down structurally. There is no port map anymore |
| mpv's default `--vo=gpu` presents BLACK when rendered into an embedded child window (`--wid`) on the target NVIDIA/KDE box | Same GL-present failure family as WebKit's dmabuf-renderer black window (see the WebKitGTK row above) — GPU-path presentation into a foreign X11 window fails silently on this driver/compositor combo. The verified recipe (`mpv.rs::build_mpv_args`) is `--vo=x11 --hwdec=auto-copy`: `x11` blits reliably into the child window while `auto-copy` keeps decode on nvdec, so hardware decode is preserved even though presentation is software-blitted. Pointer pass-through for the DOM hover-controls needs **both** halves of the trick, not just one: mpv's own `--input-cursor-passthrough` flag (so mpv's internal input handling doesn't eat events) **and** an empty input region on the GTK side — `embed.rs::mount_mpv` calls `area.input_shape_combine_region(Some(&gtk::cairo::Region::create()))` on the `GtkDrawingArea` right after `realize()` — so the X11 window itself has no hit-test area and events fall through to the React webview underneath. Missing either half leaves the surface eating hover/click events instead of the DOM control strip |

## Git workflow

Branch protection is on `main`. **Never commit directly to `main`** — always branch off. Stacked branches are fine (`feat/tauri-phase-2-chat` is stacked on `feat/tauri-phase-1`).

**Commit messages** — do not include any reference to AI, Claude, or automated generation. Conventional-style subjects ("Phase 2a: …", "fix: …", "docs: …") are fine.

**Releases** — tag-driven (Phase 5 work). `git tag vX.Y.Z && git push --tags` will (eventually) fire a CI workflow producing AppImage / .deb / .dmg / .exe artifacts.

## Roadmap maintenance

`docs/ROADMAP.md` is the source of truth for what's planned vs shipped. **Whenever a feature from the roadmap ships, the roadmap must be updated in the same PR (or a follow-up docs PR if the feature PR was already merged):**

- Flip the leading `- [ ]` to `- [x]` on the relevant bullet
- Append `(PR #N)` after the title for traceability
- If implementation diverged meaningfully from the original description (different storage path, different API endpoint, additional sub-features), edit the bullet to reflect what actually shipped — not what was originally proposed
- For phase headers, when ALL items in a phase or sub-phase are checked, mark the header `## Phase X — title  ✓ shipped (PR #N)` so a glance shows the phase status without expanding the bullets

This keeps the roadmap accurate so future planning isn't done against stale assumptions, and so the gap between "planned" and "actually built" is always visible. If you discover during a session that previously-shipped work isn't reflected, fix it before doing new work — never plan on top of a known-stale roadmap.

## "Ship it" — what the user means

When the user says **"ship it"** about a finished feature branch, do the entire integration sequence end-to-end without further prompting. Each step is non-negotiable:

1. **Verify clean state** — `cargo test` + `npm run build` green, no uncommitted changes that don't belong in the PR
2. **Push the branch** — `git push -u origin <branch>`
3. **Open the PR** — `gh pr create` with a substantive title (under 70 chars) and a body covering Summary + key tradeoffs + Test plan
4. **Merge the PR** — `gh pr merge <N> --squash --delete-branch` (squash is the repo convention; never use merge or rebase merge unless the user asks)
5. **Mark the roadmap** — per the section above. If the shipped feature is **not** on the roadmap at all (a one-off fix or a small UX improvement), add it to the appropriate phase as a checked item with `(PR #N)` so the phase still tells a complete story. If the shipped feature was the LAST unshipped item in a phase, also mark the phase header `✓ shipped`.
6. **Land the roadmap update** — small follow-up docs PR (the feature PR is already merged by step 4); push, `gh pr create`, `gh pr merge --squash --delete-branch`
7. **Local cleanup** — pull main, delete the local feature branch, remove any worktree

Don't stop after step 4 thinking the feature is "shipped" — the roadmap mark is part of shipping. If the workflow is being applied to a branch that already merged via someone else's hand, just do step 5-7.

## Useful scripts

- See `package.json` for npm scripts
- `docs/ROADMAP.md` for phase-by-phase plan

## Documentation

A high-level summary of this project also lives in the Obsidian vault at
`~/GoogleDrive/obsidian/notes/Livestreamlist Rust.md`. Update it alongside
`docs/ROADMAP.md` and this file when the project's status or architecture
changes meaningfully — it's a snapshot for cross-project reference, not a
substitute for the roadmap's detail.

## Out of scope (on purpose)

- No backend/server — everything lives in the client; platform APIs are hit directly
- No analytics, telemetry, crash reporting
- No auto-update framework yet (Phase 5)
