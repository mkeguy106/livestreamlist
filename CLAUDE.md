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

### IPC — event topics

The Rust side emits events that React subscribes to via `listenEvent(name, handler)` (which wraps `@tauri-apps/api/event.listen`).

| Topic | Payload | Emitter |
|---|---|---|
| `chat:message:{uniqueKey}` | `ChatMessage` | `chat/twitch.rs` per PRIVMSG |
| `chat:status:{uniqueKey}` | `ChatStatusEvent` | `chat/twitch.rs` on connect/disconnect/error |

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

## Useful scripts

- See `package.json` for npm scripts
- `docs/ROADMAP.md` for phase-by-phase plan

## Out of scope (on purpose)

- No backend/server — everything lives in the client; platform APIs are hit directly
- No analytics, telemetry, crash reporting
- No auto-update framework yet (Phase 5)
