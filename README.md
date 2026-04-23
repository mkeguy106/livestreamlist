# livestreamlist

Cross-platform desktop livestream monitor — successor to `livestream.list.qt`, built on **Tauri + React + Rust**.

Three switchable UI layouts, Linear/Vercel mono aesthetic, density 9. Click one of the three dots in the top-left corner of the custom titlebar to switch:

| Dot | Letter | Layout  | Shortcut |
| :-: | :----: | ------- | -------- |
| 1   | A      | Command | `1`      |
| 2   | B      | Columns | `2`      |
| 3   | C      | Focus   | `3`      |

- **Command** — sidebar rail of all channels + details pane for the selected one.
- **Columns** — TweetDeck-style horizontal columns, one per live channel.
- **Focus** — single featured stream with a tab strip of all channels along the top.

The selected layout persists to `localStorage`. `N` opens the add-channel dialog; `R` forces a refresh.

## Status

**Phase 1 — shipped**
- Tauri v2 shell, custom titlebar (no native chrome)
- Rust backend: channel store (JSON persistence), URL autodetect parser
- Twitch GraphQL live-status client (batched ≤ 35 channels/request, unauthenticated)
- Real channels wired into all 3 layouts with 60 s poll
- Add channel from URL / handle / `t:login` syntax
- Launch stream via `streamlink` (detached, survives app close)
- Open channel in default browser

**Phase 2a — in progress**
- [x] Twitch IRC over WebSocket (anonymous read) with emote tag parsing
- [x] Global emote pipeline: 7TV + BTTV + FFZ + Twitch CDN
- [x] `ChatView` renderer with inline emote images
- [x] Chat wired into Command (selected), Columns (per column), Focus (featured)
- [ ] YouTube / Kick / Chaturbate live-status clients
- [ ] Per-channel 3rd-party emotes (7TV/BTTV/FFZ by Twitch user id / login)

**Phase 2b** — Kick Pusher WS + OAuth 2.1 PKCE. Twitch OAuth implicit + chat sending. YouTube + Chaturbate web-embed chat. Moderation events (CLEARCHAT / CLEARMSG).

**Phase 3** — Chat polish: reply threading, hype/raid banners, socials banner, per-channel chat logs.

**Phase 4** — Preferences UI, theme editor, desktop notifications, system tray, single-instance guard.

**Phase 5** — Import follows, spellcheck/autocorrect, Twitch Turbo auth.

## Requirements

**Runtime:** `streamlink` and `mpv` on `PATH`. (Plus a web browser for the "Open in browser" action.)

**Build:**
- Node ≥ 20
- Rust ≥ 1.77 (stable)
- Linux system deps: `webkit2gtk-4.1`, `libayatana-appindicator`, `librsvg`, `base-devel`

## Develop

```bash
npm install
npm run tauri:dev         # launches the desktop window with HMR on the frontend
npm run tauri:build       # production build → src-tauri/target/release/
npm run build             # frontend only
cargo test --manifest-path src-tauri/Cargo.toml
```

The frontend alone is also runnable in a plain browser (`npm run dev`) — it falls back to mock data when the Tauri IPC isn't present, which is useful for layout iteration.

## Configuration

Channel list persists to XDG config:

- Linux: `~/.config/livestreamlist/channels.json`
- macOS: `~/Library/Application Support/livestreamlist/channels.json`
- Windows: `%APPDATA%\livestreamlist\channels.json`

## Project structure

```
src/                         # React frontend
├── App.jsx                  # Titlebar + layout switcher + add-channel dialog
├── ipc.js                   # Tauri invoke wrappers (with browser-dev fallbacks)
├── directions/              # Command / Columns / Focus layouts
├── components/              # AddChannelDialog, WindowControls
├── hooks/                   # useLivestreams
├── utils/                   # format helpers (viewers, uptime)
└── tokens.css               # Design tokens (zinc scale, platform colors, hairlines)

src-tauri/                   # Rust backend
├── Cargo.toml
├── tauri.conf.json
├── capabilities/
└── src/
    ├── lib.rs               # Tauri builder + invoke handlers
    ├── main.rs
    ├── config.rs            # XDG paths, atomic writes
    ├── channels.rs          # Channel + Livestream + ChannelStore
    ├── platforms/
    │   ├── mod.rs           # Platform enum + URL autodetect
    │   └── twitch.rs        # GraphQL live-status client
    ├── refresh.rs           # Orchestrates refresh_all
    └── streamlink.rs        # Detached process spawn for streamlink + xdg-open
```

## Linux pitfall

WebKitGTK crashes with `Error 71 (Protocol error) dispatching to Wayland display` on NVIDIA + KDE Plasma Wayland when the DMABUF renderer is enabled. The binary sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` at startup to avoid this. If you hit other WebKit Wayland weirdness, try `GDK_BACKEND=x11` (forces XWayland).
