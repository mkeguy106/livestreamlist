---
title: Embed rewrite — child webviews instead of parked overlay window
date: 2026-04-28
phase: 6
status: design
---

# Embed rewrite — child webviews instead of parked overlay window

## Background

Today's YouTube and Chaturbate chat embeds (`src-tauri/src/embed.rs`,
`src/components/EmbeddedChat.jsx`) work by spawning a separate borderless
top-level `WebviewWindow` and parking it over the React chat-pane region with
a stream of `set_position`/`set_size` IPC calls. The file's own opening
comment admits the lie: *"From the user's view the chat appears embedded;
the OS still treats it as its own window."*

This causes a long list of papercuts:

- **Floaty lag** when dragging or resizing the main window — every reflow
  is an IPC round-trip plus a WM `XMoveResizeWindow`.
- **Z-order fights** — modals must explicitly hide the embed; tooltips can
  be occluded; KWin Wayland animates the embed independently.
- **Single-embed ceiling** — `Option<CurrentEmbed>` in `embed.rs` permits
  only one embed at a time; the Columns layout silently shows zero YT/CB
  chats when a column-per-channel model is active.
- **Cross-platform-channel switches** trigger real WM window close+reopen
  animations because different platforms = different `data_directory` =
  can't `navigate()` the same window.
- **Per-platform compositor workarounds** keep accreting:
  `_NET_WM_BYPASS_COMPOSITOR=1` (X11-only), `transient_for(main)`,
  `skip_taskbar`, `always_on_top`-then-clear, deferred 200/600/1200ms
  re-positions.
- **Wayland constraint** — native Wayland clients can't read or set
  absolute window position, forcing the entire app onto Xwayland for the
  embed to track at all.

By contrast, the Qt predecessor (`livestream.list.qt`) embeds chats as
`QWebEngineView` instances inside the main `QMainWindow`'s widget tree.
Position is handled by Qt's layout engine, multi-embed is natural, modals
don't fight, and there's no separate OS window. We want to mirror this.

## Goal

Replace the parked-overlay-window architecture with **child webviews
living inside the main window's surface** — one webview per channel
embed, positioned via the platform's native child-view API. The on-disk
profile model and auth flows are unchanged; only the embedding mechanism
is replaced.

## Non-goals

- Chat-tabs feature (separate spec on `docs/spec-command-chat-tabs`).
  This rewrite is the foundation it needs; chat-tabs ships afterwards.
- Twitch / Kick chat — those use our native IRC / Pusher clients.
- Replacing login popups with iframes. Login flows stay as separate
  top-level `WebviewWindow`s sharing the per-platform `data_directory`.
- Native Wayland — nice-to-have unlocked by this work but kept out
  because `tauri-plugin-window-state` still needs Xwayland for the main
  window position.

## Investigation summary

Previous work documented in
`/home/joely/.cargo/registry/src/.../tauri-runtime-wry-2.10.1/src/lib.rs:5020-5031`
shows that Tauri's `WebviewWindow::add_child` on Linux unconditionally
parents the child webview into `window.default_vbox()` (a `gtk::Box`),
which ignores `bounds`/`set_position`/`set_size`. tauri#9611 (Apr 2025)
documents the bug; maintainer parked it.

Key finding: **wry itself supports `gtk::Fixed`-based positioning since
0.35.2** (Apr 2024). The escape hatch is to bypass Tauri's `add_child` on
Linux and call `wry::WebViewBuilder::build_gtk(&fixed)` directly,
positioning child webviews into a `gtk::Fixed` that we own. macOS and
Windows are unaffected — `add_child` works correctly there.

All `WebviewBuilder` features we depend on (`data_directory`,
`on_page_load`, `background_color`, `cookies_for_url` on the returned
handle) are supported by both APIs.

## Architecture

### Rust: `EmbedHost` and `ChildEmbed`

A single `Arc<EmbedHost>` is registered in app state at startup. It owns
a `HashMap<EmbedKey, ChildEmbed>` plus, on Linux, a `gtk::Fixed` overlaid
on top of the React webview.

```rust
// src-tauri/src/embed.rs

pub type EmbedKey = String;          // unique_key (with optional :video_id)

pub struct EmbedHost {
    inner: Mutex<Inner>,
}

struct Inner {
    children: HashMap<EmbedKey, ChildEmbed>,
    #[cfg(target_os = "linux")]
    fixed: SendWrapper<gtk::Fixed>,
}

struct ChildEmbed {
    platform: Platform,
    bounds: Rect,
    visible: bool,
    inner: ChildInner,
}

#[cfg(target_os = "linux")]
struct ChildInner(SendWrapper<wry::WebView>);

#[cfg(not(target_os = "linux"))]
struct ChildInner(tauri::webview::Webview);

impl ChildEmbed {
    fn set_bounds(&mut self, r: Rect) -> Result<()>;
    fn set_visible(&mut self, v: bool) -> Result<()>;
    fn eval(&self, js: &str) -> Result<()>;
    fn cookies_for_url(&self, url: &Url) -> Result<Vec<Cookie>>;
}
```

Public methods on `EmbedHost`:

| Method | Purpose |
|---|---|
| `mount(app, store, key, bounds) -> Result<bool>` | Idempotent. If `key` exists, just `set_bounds`. Otherwise create. Returns `false` if the channel is offline so React shows a placeholder. |
| `set_bounds(key, bounds)` | Reflow on React layout change. |
| `set_visible(key, bool)` | GTK widget hide/show (modals; future tab-inactive). |
| `unmount(key)` | Destroy a single child — drop the wry::WebView / Tauri Webview, remove from HashMap. Profile dir on disk is untouched. |
| `unmount_platform(platform)` | Drop every child of `platform`. Used by `auth::*::clear()` before `remove_dir_all` on the profile dir. |

No more `position(...)` separate from `mount(...)` (collapsed into `set_bounds`),
no more `set_visible_all(bool)` (replaced by per-key calls from the
modal-hook in App.jsx).

### Linux container topology

Tauri's main window today is roughly:

```
GtkApplicationWindow
└── default_vbox (gtk::Box)
    └── WebKitWebView                      ← React app
```

After `Builder::setup`, on the GTK main thread via `with_webview`, we
sandwich a `GtkOverlay`:

```
GtkApplicationWindow
└── default_vbox (gtk::Box)
    └── GtkOverlay
        ├── (base) WebKitWebView           ← React app (full overlay area)
        └── (overlay) gtk::Fixed           ← embed_host.fixed, holds children
```

Reasons:
- `GtkOverlay` puts the `gtk::Fixed` (and its child webviews) **above**
  the React webview at the GTK widget tree level — same visual layering
  as today's parked window.
- `gtk::Fixed` accepts arbitrary `(x, y, w, h)` coords for child widgets,
  which is what wry's `set_bounds` requires.
- The base child of `GtkOverlay` (the React webview) renders normally and
  fills the overlay area. The overlay child (`gtk::Fixed`) is invisible
  except where we `put` child webviews.
- Reparent operation is one-shot at startup. We never re-parent again.

Reparent uses Wry's `InnerWebView::reparent(container)`, which exists in
the version we're already pulling in (wry 0.54.4).

### macOS / Windows

Use Tauri's `WebviewWindow::add_child(WebviewBuilder, label, position, size)`
directly. The returned `Webview` handle exposes `set_size`,
`set_position`, `show`, `hide`, `eval`, `cookies_for_url`.

No setup hook needed. `EmbedHost::new()` on these platforms is just
`Self { inner: Mutex::new(Inner { children: HashMap::new() }) }`.

### Per-platform `ChildEmbed` impl

Only four methods diverge — the rest of `EmbedHost` (HashMap, lifecycle,
IPC, auth hook) is shared.

| Concern | Linux | macOS / Windows |
|---|---|---|
| Build | `wry::WebViewBuilder::new(url).with_*().build_gtk(&host.fixed)` | `WebviewWindow::add_child(builder, label, position, size)` |
| `set_bounds` | `wry_webview.set_bounds(Rect{x,y,w,h})` | `webview.set_position(...)` + `webview.set_size(...)` |
| `set_visible` | `webview.gtk_widget().set_visible(bool)` | `webview.show()` / `webview.hide()` |
| `cookies_for_url` | `wry_webview.cookies_for_url(url)` (verify in 0.54.4) | `webview.cookies_for_url(url)` (used today) |

### Coordinates

Wire format: **physical pixels** for `embed_mount`/`embed_bounds`. Same
as today.

- macOS / Windows: pass through to Tauri APIs as `PhysicalPosition` /
  `PhysicalSize`. (No change from today.)
- Linux: GTK uses logical coordinates. The Rust side divides each
  incoming `(x, y, w, h)` by `main_window.scale_factor()` exactly once
  before `set_bounds`. (Frontend keeps multiplying by `devicePixelRatio`
  as it does today; net change is one DPR conversion site instead of
  three.)

### IPC surface

Registered in `lib.rs::generate_handler!`:

| Command | Args | Returns | Notes |
|---|---|---|---|
| `embed_mount` | `key, x, y, w, h` | `bool` | Idempotent. `false` = channel offline. |
| `embed_bounds` | `key, x, y, w, h` | `()` | Was `embed_position`; renamed. |
| `embed_set_visible` | `key, visible` | `()` | Replaces `set_visible_all`; per-key. |
| `embed_unmount` | `key` | `()` | Drop a single child. |

`unmount_platform(platform)` is a Rust-internal API used by
`auth::*::clear()`; not an IPC command.

Wire and serialization unchanged (same as today's `embed_mount`).

### Auth integration

**No changes to `auth/youtube.rs`, `auth/chaturbate.rs`, or
`docs/superpowers/specs/2026-04-25-chaturbate-login-design.md`.**

The auth modules' contracts are preserved verbatim:

- `webview_profile_dir()` paths unchanged
  (`~/.local/share/livestreamlist/webviews/{youtube,chaturbate}/`).
- Login popups remain separate top-level `WebviewWindow`s built with the
  same `data_directory` as the embeds. Cookies persist on disk; embed
  child webviews + login window share the on-disk store, exactly like
  today.
- `verify_chaturbate_auth` runs from the embed's `on_page_load`
  `PageLoadEvent::Finished` handler. The new `ChildEmbed::cookies_for_url`
  exposes the same API the function already calls.
- `clear()` continues to call `EmbedHost::unmount_platform(platform)`
  before `remove_dir_all(profile_dir)`. Now drops every child of that
  platform instead of just `Option<CurrentEmbed>`.
- `clear_stamp_only` (the drift case where the embed is mid-load) still
  applies — same code path, no semantic change.

### Init scripts and dark-first-paint

Preserved verbatim:
- `background_color(zinc_950)` on the wry / Tauri builder
  (`Color(9, 9, 11, 255)`).
- show-after-`PageLoadEvent::Finished` hook — the same pattern as today,
  registered on the wry::WebView (Linux) or Tauri Webview (mac/Win).
- Per-platform CSS / JS injection in the `on_page_load` handler:
  `YT_THEME_CSS` for YouTube, `CB_ISOLATE_JS` for Chaturbate. Re-injected
  on every page load because `navigate()` (when reused, future
  optimization) wipes the JS context.

### Frontend: global `EmbedLayer` + `EmbedSlot`

Today every `ChatView` for a YT/CB channel renders an `<EmbeddedChat>`
that owns mount/bounds/unmount IPC for that channel. This conflates
"where in the layout the embed should appear" with "is the embed alive."
For chat-tabs (where multiple ChatViews mount with `display:none`) this
breaks — the inactive tabs would each report 0×0 bounds, fighting for
the same `EmbedKey`.

New shape:

- One global `<EmbedLayer>` mounted at App.jsx scope. It owns a registry
  (`Map<EmbedKey, EmbedSlot>`) and is the *only* component that calls
  `embed_mount`/`embed_bounds`/`embed_set_visible`/`embed_unmount` IPC.
- `<EmbedSlot channelKey={k} active={bool}>` is mounted by ChatView
  in place of today's `<EmbeddedChat>`. It registers itself with the
  layer via context, exposes its `getBoundingClientRect`, and reports
  active/inactive state.
- The `EmbedLayer` arbitrates: for each `EmbedKey`, it picks the active
  slot's rect as canonical and calls `embed_bounds`. When no slot for a
  key is active, it calls `embed_set_visible(key, false)`. When a key
  is no longer claimed by any slot at all (last channel switched away,
  channel removed), it calls `embed_unmount(key)`.
- App-level modal state hooks into `EmbedLayer` to call
  `embed_set_visible(key, false)` for the active embed when modals are
  open. (Today's `set_visible_all` hook moves here.)

For the Single-instance-per-channel rule: the `EmbedKey` in the registry
matches today's `unique_key` (with optional YT `:video_id` suffix). Two
slots with the same key cannot both be `active=true`; the layer asserts
this. (For chat-tabs in the future, this is enforced by the tab strip:
the same channel can be a tab once, and only the active tab marks its
slot active.)

#### `EmbedSlot` — minimal placeholder

Replaces today's 196-line `EmbeddedChat.jsx` with ~30 lines. No more
`outerPosition` cache, no `onMoved`/`onResized` listeners on the main
window, no scroll listener, no 200/600/1200ms timed re-positions. Just:

```jsx
function EmbedSlot({ channelKey, active }) {
  const ref = useRef(null);
  const layer = useContext(EmbedLayerContext);

  useEffect(() => {
    layer.register(channelKey, ref, { active });
    return () => layer.unregister(channelKey, ref);
  }, [channelKey, active]);

  return <div ref={ref} className="embed-placeholder" />;
}
```

The `EmbedLayer` owns a `ResizeObserver` chain on each registered slot's
ref + a `window.resize` listener, and pushes new bounds via
`embed_bounds(key, ...)`. No more chasing main-window movement — the
embed lives in the same OS surface.

### Lifecycle table

| Event | Today | New |
|---|---|---|
| User selects a YT/CB channel in Command | `embed_mount` opens (or navigates) the parked window | `EmbedLayer.register(key, ...)` → `embed_mount` if first-active for key |
| User selects a different channel (same platform) | Reuses parked window via `navigate()` | Old slot unregisters; old key `embed_unmount`. New slot registers; new key `embed_mount`. |
| User selects a different channel (cross-platform) | Closes + reopens parked window (WM animation) | Same as above, no WM animation. |
| User opens a modal | `set_visible_all(false)` on parked window | `embed_set_visible(active_key, false)` on the active child. |
| Channel removed from list | `embed_unmount` closes parked window | `embed_unmount` drops the child from the HashMap. |
| App quits | Parked window closes | Children drop with the main window's GTK / Cocoa / HWND tree. |
| Multiple YT chats simultaneously visible (Columns) | **Broken today** — `Option<CurrentEmbed>` shows only one | Each column registers its slot; layer mounts N children. |
| Future: chat-tabs inactive tab | n/a | Slot registered with `active=false` → `embed_set_visible(key, false)`. Webview process stays alive, scroll position preserved. |
| Future: chat-tabs popout | n/a | Popout window has its own `EmbedHost` → `embed_unmount` from main, `embed_mount` in popout. |

## Migration

Single PR (full cross-platform parity, per user direction):

1. Delete the body of `src-tauri/src/embed.rs`. Keep the file path.
2. Write the new `EmbedHost` + `ChildEmbed` + per-platform branches.
3. `Builder::setup` in `lib.rs::run`:
   - All platforms: `app.manage(Arc::new(EmbedHost::new()))`.
   - Linux: additionally, the GtkOverlay reparent on the GTK main thread
     via `with_webview`. Stash the `gtk::Fixed` in the `EmbedHost`.
4. IPC handlers: `embed_mount`, `embed_bounds`, `embed_set_visible`,
   `embed_unmount`. Drop `embed_position` (renamed to `embed_bounds`).
5. Frontend:
   - New `src/components/EmbedLayer.jsx` (component + context).
   - Rewrite `src/components/EmbeddedChat.jsx` as `EmbedSlot.jsx`
     (~30 lines).
   - Update every ChatView mount site (`src/components/ChatView.jsx`)
     to use `<EmbedSlot>` and pass an `active` prop derived from
     "is this ChatView currently the focused one." For Command/Focus
     today, `active = true` always. For Columns, `active = true` for
     every visible column. For chat-tabs (future), `active = (this tab is the
     active tab for its key)`.
   - Mount `<EmbedLayer>` once at the top of `App.jsx`.
   - App-level modal state wires into `EmbedLayer` via the context.
6. `auth/chaturbate.rs::clear()` still calls
   `EmbedHost::unmount_platform(Platform::Chaturbate)` before
   `remove_dir_all`. Same call site, broader effect (all CB children
   instead of just one). No spec change.
7. Roadmap update: add a Phase 6 entry capturing "embed rewrite — child
   webviews instead of parked overlay."

## Risks and unknowns

1. **GtkOverlay layering composes correctly with WebKitGTK rendering.**
   The most important Day-1 question. Could manifest as: child webview
   doesn't render, doesn't receive input, or compositor artifacts at the
   overlay boundary. Mitigation: prove with a single hardcoded YT embed
   before writing any of the host/IPC code. Fallback if it breaks: a
   side-by-side `gtk::Fixed` (no overlay) and forbid React from
   rendering in the chat-pane region — viable, uglier.

2. **`add_child` builder feature parity on macOS/Windows.** The wry path
   on Linux is verified by the research pass; the Tauri `add_child`
   path needs a per-platform smoke that `data_directory`,
   `on_page_load`, `background_color`, and `cookies_for_url` all work
   on add_child children (vs only on top-level WebviewWindows).
   Mitigation: Day 2 spike per platform; if any fails, that platform
   falls back to today's parked-window approach in this PR and a
   follow-up addresses it.

3. **WebKitWebProcess RAM** scales linearly with child count. With
   chat-tabs's 15-tab soft cap landing later, this will be 15+ webview
   processes. Same situation as Qt today; document and accept.

4. **DPR / scale-factor handling differs per platform.** Mitigation:
   single conversion site in Rust on Linux (`/ scale_factor`); pass
   through to Tauri PhysicalPosition/PhysicalSize on mac/Win. Frontend
   keeps `× devicePixelRatio` unchanged.

5. **Wry direct-build webviews aren't in Tauri's manager.** No Tauri
   IPC commands or events into them. For our case this is fine —
   embeds are opaque content surfaces; we never need to send IPC into
   them. Documented as an explicit boundary.

6. **Login window occlusion.** Login popups remain separate top-level
   windows; they're always WM-focused and short-lived. Today's
   `set_visible_all` hide-on-modal doesn't apply since logins aren't
   React modals — and they shouldn't be hidden anyway. No regression.

## Phasing within the single PR

Even though it ships as one PR, internally the work is ordered to keep
each step verifiable:

1. **Day 1** — Linux spike: `EmbedHost::new()`, GtkOverlay sandwich,
   build a single hardcoded YT embed at fixed coords. Verify input +
   render + no compositor artifacts.
2. **Day 2** — macOS + Windows smoke of `add_child` with
   `data_directory`, `on_page_load`, `cookies_for_url`,
   `background_color`. (~30 min per platform.)
3. **Day 3** — HashMap-driven lifecycle, IPC surface, full Linux
   multi-embed (Columns layout with 3 YT channels rendering
   simultaneously).
4. **Day 4** — macOS + Windows wired through `EmbedHost` (assuming
   Day 2 was clean).
5. **Day 5** — Frontend: `EmbedLayer` + `EmbedSlot` rewrite, modal
   occlusion via `embed_set_visible`, parity smoke on all three
   platforms.
6. **Day 6** — Auth drift hook on Chaturbate child webview, manual test
   of login popup → embed → drift detection. Roadmap update. Polish.
7. **Day 7** — Buffer for Risk #1 if it bites.

## Testing

### Unit tests

- `EmbedHost` lifecycle: mount → mount-same-key → bounds → set_visible
  → unmount. Idempotency. HashMap-only path; no GTK / NSView /
  WebView2 dependency. Cross-platform.
- `EmbedKey` round-trip: `unique_key` with and without YT `:video_id`
  suffix. (`channel_key_of` helper from the multi-stream spec already
  unit-tested; just confirm the embed module uses it correctly.)

### Manual smoke (PR test plan)

- Add a YT channel; open Command layout; verify embed renders inside
  the chat pane, drag the main window, confirm zero lag.
- Switch to Columns layout with 3 live YT channels; confirm all 3
  embeds visible simultaneously (impossible today).
- Open a modal (Preferences, Add Channel); confirm embed hides; close
  modal; confirm embed reappears.
- Switch from a YT channel to a Chaturbate channel; confirm no WM
  open/close animation; confirm Chaturbate page-load CSS isolation
  applies.
- Sign out of Chaturbate via Preferences → Accounts; confirm the active
  CB embed unmounts before the profile dir wipes (no WebKit-rug-pull
  errors in the log).
- Resize the main window; confirm embed reflows smoothly with no
  trailing rectangle.
- Open Find in chat (existing feature); confirm find UI doesn't render
  underneath the embed.
- Per-platform smoke: repeat the multi-embed test on macOS and Windows
  to confirm `add_child` parity.

### Out of scope for tests (this spec)

- Native Wayland — keep on Xwayland for now (`tauri-plugin-window-state`
  still requires it for main window position).
- Stress test of >15 simultaneous embeds (no real-world use case until
  chat-tabs and even then capped).

## File-by-file summary

| File | Change |
|---|---|
| `src-tauri/src/embed.rs` | Rewritten. New `EmbedHost`, `ChildEmbed`, per-platform `ChildInner`. |
| `src-tauri/src/lib.rs` | Replace `embed_position` with `embed_bounds` and `embed_set_visible` in `generate_handler!`. Linux: GtkOverlay reparent in `Builder::setup`. |
| `src-tauri/src/auth/chaturbate.rs` | No change. Continues to call `EmbedHost::unmount_platform` before `remove_dir_all`. |
| `src-tauri/src/auth/youtube.rs` | No change. |
| `src/components/EmbeddedChat.jsx` | Replaced by `EmbedSlot.jsx` (~30 lines). |
| `src/components/EmbedLayer.jsx` | New. Owns the registry + IPC. ~150 lines. |
| `src/components/ChatView.jsx` | Replace `<EmbeddedChat>` mount with `<EmbedSlot>`; pass `active` prop. |
| `src/App.jsx` | Mount `<EmbedLayer>` at top scope; wire modal state into its context. |
| `src/ipc.js` | `embedMount`, `embedBounds` (renamed), `embedSetVisible` (new), `embedUnmount`. Mock fallbacks updated. |
| `docs/ROADMAP.md` | Add Phase 6 entry "Embed rewrite — child webviews instead of parked overlay." |

No changes to `channels.rs`, `refresh.rs`, `chat/`, `platforms/`,
`auth/youtube.rs`, `auth/chaturbate.rs`, `auth/twitch.rs`, `auth/kick.rs`,
or any of the existing chat / platform / refresh logic.
