# CLAUDE.md

Guidance for Claude Code when working in this repository.

## Project Overview

Cross-platform desktop livestream monitor. Successor to [`livestream.list.qt`](https://github.com/mkeguy106/livestream.list.qt) (PySide6/Qt6), rewritten on **Tauri v2 + React + Rust**.

The single product surface is a desktop window with three switchable layouts (the top-left titlebar dots pick them): **Command** (sidebar rail + details pane), **Columns** (TweetDeck-style live columns), **Focus** (single featured stream + tab strip).

Visual identity is the Linear/Vercel mono aesthetic from the design bundle — zinc near-black, red live dots, Inter + JetBrains Mono, hairline 1 px borders on `rgba(255,255,255,.06)`, density 9. Platform accents (twitch/youtube/kick/chaturbate) are pale-desaturated, used only to mark provenance.

## Tech Stack

- **Frontend**: React 18, Vite 5, plain CSS variables in `src/tokens.css`
- **Backend**: Rust (stable, ≥ 1.77), Tauri 2, `reqwest` (rustls), `tokio-tungstenite` (WebSocket), `parking_lot`, `chrono`
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
    │   └── twitch.rs        # GraphQL live-status client (batched ≤ 35 / request)
    ├── refresh.rs           # Orchestrates refresh_all across platforms
    ├── streamlink.rs        # Detached subprocess spawn + browser handoff
    └── chat/
        ├── mod.rs           # ChatManager — one task per channel
        ├── models.rs        # ChatMessage, ChatUser, EmoteRange, ChatBadge
        ├── irc.rs           # IRCv3 parser + Twitch emote-tag parser
        ├── twitch.rs        # Anonymous WebSocket IRC client
        └── emotes.rs        # 7TV / BTTV / FFZ loaders + EmoteCache
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
| `open_in_browser` | `uniqueKey` | `xdg-open` / `open` / `start` on the channel URL |
| `chat_connect` | `uniqueKey` | Start per-channel chat task |
| `chat_disconnect` | `uniqueKey` | Abort that task |
| `embed_mount` | `uniqueKey, x, y, width, height` | Mount a YT/CB child webview at given physical-pixel rect; idempotent (resize if already mounted). Returns `false` if channel offline |
| `embed_bounds` | `uniqueKey, x, y, width, height` | Reflow an existing embed (e.g. on layout change) |
| `embed_set_visible` | `uniqueKey, visible: bool` | Hide/show one embed (used for modal occlusion) |
| `embed_unmount` | `uniqueKey` | Drop the child webview; the underlying GtkWidget destroys via `wry::WebView::Drop` |
| `chaturbate_login` / `chaturbate_logout` | — | Open the CB login popup / wipe profile dir + clear stamp |

### IPC — event topics

The Rust side emits events that React subscribes to via `listenEvent(name, handler)` (which wraps `@tauri-apps/api/event.listen`).

| Topic | Payload | Emitter |
|---|---|---|
| `chat:message:{uniqueKey}` | `ChatMessage` | `chat/twitch.rs` per PRIVMSG |
| `chat:status:{uniqueKey}` | `ChatStatusEvent` | `chat/twitch.rs` on connect/disconnect/error |
| `chat:auth:chaturbate` | `{ signed_in, reason }` | `embed.rs::handle_chaturbate_auth_outcome` on every CB embed page-load — broadcasts auth-drift status |

Events are strictly one-way (Rust → UI). UI never emits events; it calls invoke commands.

### Data flow

0. `ChannelStore::load()` reads `~/.config/livestreamlist/channels.json` into memory.
1. `App` mounts → `useLivestreams` invokes `list_livestreams` (instant cache), then `refresh_all` (network).
2. `refresh_all` runs each platform client in parallel (currently only Twitch GraphQL), merges into the store's `livestreams` map, returns a fresh snapshot.
3. `useLivestreams` re-polls `refresh_all` every 60 s.
4. When a layout mounts `ChatView channelKey={k}`, `useChat(k)` invokes `chat_connect` and subscribes to `chat:message:{k}`.
5. `ChatManager::connect` spawns a task. `twitch::run` opens a WebSocket, sends `CAP REQ + NICK justinfan… + JOIN #…`, reads frames, parses IRC, emits events per PRIVMSG.
6. `EmoteCache` has globals loaded on app start (7TV + BTTV + FFZ); Twitch emote IDs come from the IRC `emotes=` tag and map directly to Twitch CDN URLs.

### Channel store (`src-tauri/src/channels.rs`)

- `Channel` — persisted: platform, channel_id, display_name, favorite, dont_notify, auto_play, added_at
- `Livestream` — transient: live/off, title, game, viewers, started_at, thumbnail_url, last_checked, error
- `unique_key` = `"{platform}:{channel_id}"` (the identifier used everywhere: storage, IPC, event topics, React keys)
- `ChannelStore` is held in `Arc<Mutex<…>>` (parking_lot) — no async locks; the store is memory-fast

### Chat architecture

`ChatManager` owns an `Arc<EmoteCache>` + a `Mutex<HashMap<unique_key, JoinHandle>>`. One task per connected channel. Abort is idempotent.

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
- `ChildInner` — Linux: `Arc<wry::WebView>`. Non-Linux: `tauri::webview::Webview`. `unsafe impl Send + Sync` on the Linux variant — GTK access is gated to the main thread by call sites; the Mutex serializes lookups.
- `EmbedKey` = the same `unique_key` flowing through chat IPC (with the optional YT `:video_id` suffix from the multi-stream scraper).

**Construction (Linux)** — `build_linux::build_child`:
- Per-platform `data_directory` via `wry::WebContext::new(Some(profile_dir))` then `WebViewBuilder::new_with_web_context(ctx)`. `wry 0.54` moved data_directory off the builder onto WebContext — this is non-obvious. The `WebContext` is `Box::leak`'d for the lifetime of the main window (the on-disk profile dir is the persistence; the in-memory WebContext leaking is a small constant per mount).
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

### The three layouts

- **Command** — selected-channel workflow. Sidebar rail shows all channels (live first, then offline alpha). Main pane shows the selected channel's header + chat.
- **Columns** — parallel-monitoring workflow. One compact column per **live** channel, each with its own chat. "Add column" opens the add-channel dialog.
- **Focus** — single-stream reader mode. Tab strip of all channels across the top; split 60/40 with video placeholder / chat.

All three share the same data hook (`useLivestreams`). Each has its own chat binding: Command/Focus use one `ChatView` for the selected/featured channel; Columns mounts one `ChatView` per visible column.

Selection state (`selectedKey`) lives in `App`. Layout choice persists to `localStorage` under `livestreamlist.layout`.

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

## Configuration

Data dir (XDG):
- Linux: `~/.config/livestreamlist/`
- macOS: `~/Library/Application Support/livestreamlist/`
- Windows: `%APPDATA%\livestreamlist\`

Files:
- `channels.json` — persistent channel list
- `settings.json` — reserved for Phase 4 (preferences)
- Chat logs, emote disk cache, auth tokens — reserved for Phase 3+

## Known Pitfalls

| Issue | Fix |
|---|---|
| `tokio::spawn` inside `Builder::setup()` panics: *"no reactor running"* | Use `tauri::async_runtime::spawn` everywhere. Raw tokio works inside `#[tauri::command] async fn` but not in setup |
| WebKitGTK crashes with `Error 71 (Protocol error)` on NVIDIA + KDE Wayland | Baked `WEBKIT_DISABLE_DMABUF_RENDERER=1` into `lib.rs::apply_linux_webkit_workarounds`. If other WebKit weirdness hits, try `GDK_BACKEND=x11` |
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
| `wry 0.54` moved `data_directory` from `WebViewBuilder` to `WebContext` | `WebViewBuilder::with_data_directory(...)` no longer compiles. Use `WebContext::new(Some(profile_dir))` then `WebViewBuilder::new_with_web_context(&mut ctx)`. WebContext borrows for the builder's lifetime — `Box::leak(Box::new(ctx))` is the simple fix when the WebView outlives the builder anyway |
| `webview.destroy()` requires the wry strong count to actually reach zero | `InnerWebView::Drop` (`wry-0.54.4/src/webkitgtk/mod.rs:96-99`) is what removes the GtkWidget from its parent — there's no public API to do it manually short of reaching for the underlying webkit2gtk widget. So any structural change that introduces an Arc cycle on a `wry::WebView` will manifest as "the embed visually stays after I call unmount." Audit Arc/Weak relationships carefully when wiring callback closures around WebViews |
| **HTML5 drag-and-drop is broken in WebKitGTK** | `dragstart` fires (it's WebKit-internal) but `dragenter`/`dragover`/`drop` are never delivered to JS — GTK's drag-drop machinery captures events before they reach the webview. Standard workarounds don't help: `text/plain` shim alongside the custom MIME, container-level event delegation via `closest('[data-tab-key]')`, and `dragDropEnabled: false` in `tauri.conf.json` all leave dragover dead. **For drag UX, use mouse events instead of HTML5 dnd.** See `src/components/TabStrip.jsx::TabStrip` for the canonical pattern: `onMouseDown` arms a drag with source key + start coords; document-level `mousemove`/`mouseup` listeners (added via `useEffect` while a drag is armed) track the cursor and use `document.elementFromPoint(...).closest('[data-tab-key]')` for drop-target identification; a movement threshold distinguishes click from drag; `mousedown` calls `e.preventDefault()` to suppress text-selection initiation; body cursor + userSelect are locked while active to prevent visual bleed onto neighboring UI |
| `EmbedSlot`'s register-effect must NOT include `active` in its dep array | The `active` prop on `<EmbedSlot active={isActiveTab}>` flows through TWO `useEffect`s: a `register` effect (registers the slot with `EmbedLayer`) and a separate `updateActive` effect (calls `layer.updateActive(...)` on changes). If `active` is in the register effect's deps, every change runs cleanup → setup, which calls `unregister` then `register`. With the chat-tab system having exactly one slot per channelKey, `unregister` hits the `entry.refs.size === 0` branch in `EmbedLayer` and fires `embedUnmount`, destroying the wry `WebView` via `wry::WebView::Drop`. The subsequent `register` triggers a fresh `embedMount` — the user sees the YT/CB chat reload on every tab switch. The register effect's deps must be `[channelKey, isLive, layer]` only; the `active` flag's actual change is handled by the separate `updateActive` effect, which doesn't unregister. (Fixed in PR #80; documented inline at `src/components/EmbedSlot.jsx`.) |

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

## Out of scope (on purpose)

- No backend/server — everything lives in the client; platform APIs are hit directly
- No analytics, telemetry, crash reporting
- No auto-update framework yet (Phase 5)
