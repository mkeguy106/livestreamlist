# Embed rewrite — child webviews implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the parked-overlay-window chat embed (`src-tauri/src/embed.rs`'s `EmbedManager`) with real child webviews living inside the main window's surface — `gtk::Fixed` + wry `build_gtk` on Linux, Tauri's `WebviewWindow::add_child` on macOS / Windows. Multi-embed by construction (`HashMap<EmbedKey, ChildEmbed>`); per-platform `data_directory` and existing auth flows preserved unchanged.

**Architecture:** `EmbedHost` Rust singleton owns a HashMap of children. Linux only: a one-shot `GtkOverlay` reparent at startup creates a `gtk::Fixed` above the React webview. Each `ChildEmbed` wraps a platform-native handle (`wry::WebView` on Linux, `tauri::webview::Webview` on macOS / Windows). Frontend gains a global `<EmbedLayer>` registry that arbitrates which `<EmbedSlot>` is canonical for each `EmbedKey` and dispatches `embed_mount`/`embed_bounds`/`embed_set_visible`/`embed_unmount` IPC. The old per-component `<EmbeddedChat>` is deleted.

**Tech Stack:** Rust + Tauri 2.10.3, wry 0.54.4 (already pulled in transitively), `webkit2gtk` + `gtk` 0.18 (Linux), React 18, plain CSS.

**Spec:** `docs/superpowers/specs/2026-04-28-embed-rewrite-design.md`

**Branch:** Build on top of `docs/spec-embed-rewrite` or create a fresh `feat/embed-rewrite` from `main`. Final PR merges to `main`.

---

## Phase 1 — Pure-Rust scaffolding (cross-platform, TDD)

Goal: Define `Rect`, `EmbedKey`, and the `EmbedHost` HashMap registry with full unit-test coverage. No GTK / WebKit / Tauri APIs touched yet — this is the part that's actually unit-testable.

### Task 1.1: New types in `embed.rs`

**Files:**
- Modify: `src-tauri/src/embed.rs` (replacing the file body wholesale; keep the path)

- [ ] **Step 1: Save current embed.rs to a parking file for reference**

```bash
cp src-tauri/src/embed.rs /tmp/embed-old.rs
```

We'll consult this when porting CSS/JS injection (Phase 6) and auth-drift (Phase 6) but will not import any of it.

- [ ] **Step 2: Replace `embed.rs` with the new module skeleton**

Write `src-tauri/src/embed.rs`:

```rust
//! Multi-embed host for in-window YouTube / Chaturbate chats. See
//! docs/superpowers/specs/2026-04-28-embed-rewrite-design.md.
//!
//! Linux: child webviews live in a `gtk::Fixed` overlaid on top of the
//! React webview via a one-shot `GtkOverlay` reparent done at startup.
//! macOS / Windows: child webviews are created via Tauri's
//! `WebviewWindow::add_child`.

use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

use crate::platforms::Platform;

pub type EmbedKey = String;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            x,
            y,
            w: w.max(1.0),
            h: h.max(1.0),
        }
    }
}

pub struct EmbedHost {
    inner: Mutex<Inner>,
}

struct Inner {
    children: HashMap<EmbedKey, ChildEmbed>,
}

#[allow(dead_code)] // populated in Phase 3 / 4
pub(crate) struct ChildEmbed {
    pub(crate) platform: Platform,
    pub(crate) bounds: Rect,
    pub(crate) visible: bool,
}

impl EmbedHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                children: HashMap::new(),
            }),
        })
    }

    pub fn has(&self, key: &str) -> bool {
        self.inner.lock().children.contains_key(key)
    }

    pub fn keys(&self) -> Vec<EmbedKey> {
        self.inner.lock().children.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_clamps_zero_dims_to_one() {
        let r = Rect::new(10.0, 20.0, 0.0, -5.0);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.w, 1.0);
        assert_eq!(r.h, 1.0);
    }

    #[test]
    fn host_starts_empty() {
        let host = EmbedHost::new();
        assert!(!host.has("youtube:UC123"));
        assert!(host.keys().is_empty());
    }
}
```

- [ ] **Step 3: Run unit tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml embed::
```

Expected: PASS — both tests green.

- [ ] **Step 4: `cargo check` the whole crate**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: many errors about missing `EmbedManager`, `mount`, `position`, `unmount`, etc. — that's fine. We'll fix them in Phase 7. The new `embed.rs` itself should compile.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: skeleton EmbedHost + Rect types

First slice of the embed rewrite. Defines EmbedKey, Rect (with
1px clamp on degenerate dims), and an empty HashMap-backed
EmbedHost. No webview integration yet — that comes in Phase 3/4.
The crate does not compile yet (the old embed_mount handler etc.
still expect the old API); fixed up in Phase 7."
```

---

### Task 1.2: `EmbedHost` lifecycle invariants (still pure Rust)

We can't drive real webviews from a `cargo test`, but we CAN unit-test the HashMap arbitration logic by giving `ChildEmbed` a test-only constructor.

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Add test-only constructors and the lifecycle methods**

Append to `src-tauri/src/embed.rs` before the `#[cfg(test)]` block:

```rust
impl ChildEmbed {
    #[cfg(test)]
    fn fake(platform: Platform) -> Self {
        Self {
            platform,
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            visible: true,
        }
    }
}

impl EmbedHost {
    /// Mark a key as mounted with a fake child. Test-only — real mounts
    /// go through the platform-specific build path in Phase 3/4.
    #[cfg(test)]
    pub(crate) fn insert_fake(&self, key: &str, platform: Platform) {
        let mut g = self.inner.lock();
        g.children.insert(key.to_string(), ChildEmbed::fake(platform));
    }

    pub fn unmount(&self, key: &str) {
        self.inner.lock().children.remove(key);
    }

    pub fn unmount_platform(&self, platform: Platform) {
        let mut g = self.inner.lock();
        g.children.retain(|_, c| c.platform != platform);
    }

    pub fn keys_for_platform(&self, platform: Platform) -> Vec<EmbedKey> {
        self.inner
            .lock()
            .children
            .iter()
            .filter(|(_, c)| c.platform == platform)
            .map(|(k, _)| k.clone())
            .collect()
    }
}
```

- [ ] **Step 2: Add invariant tests to the `tests` module**

Append inside the existing `mod tests` in `embed.rs`:

```rust
#[test]
fn unmount_removes_only_target_key() {
    let host = EmbedHost::new();
    host.insert_fake("youtube:UC1", Platform::Youtube);
    host.insert_fake("youtube:UC2", Platform::Youtube);
    host.unmount("youtube:UC1");
    assert!(!host.has("youtube:UC1"));
    assert!(host.has("youtube:UC2"));
}

#[test]
fn unmount_unknown_key_is_noop() {
    let host = EmbedHost::new();
    host.insert_fake("youtube:UC1", Platform::Youtube);
    host.unmount("bogus");
    assert!(host.has("youtube:UC1"));
}

#[test]
fn unmount_platform_drops_all_of_platform() {
    let host = EmbedHost::new();
    host.insert_fake("youtube:UC1", Platform::Youtube);
    host.insert_fake("youtube:UC2", Platform::Youtube);
    host.insert_fake("chaturbate:bob", Platform::Chaturbate);
    host.unmount_platform(Platform::Youtube);
    assert!(!host.has("youtube:UC1"));
    assert!(!host.has("youtube:UC2"));
    assert!(host.has("chaturbate:bob"));
}

#[test]
fn keys_for_platform_filters() {
    let host = EmbedHost::new();
    host.insert_fake("youtube:UC1", Platform::Youtube);
    host.insert_fake("chaturbate:bob", Platform::Chaturbate);
    let yt = host.keys_for_platform(Platform::Youtube);
    let cb = host.keys_for_platform(Platform::Chaturbate);
    assert_eq!(yt, vec!["youtube:UC1".to_string()]);
    assert_eq!(cb, vec!["chaturbate:bob".to_string()]);
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml embed::
```

Expected: all 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: lifecycle invariants — unmount, unmount_platform, keys_for_platform"
```

---

## Phase 2 — Linux GtkOverlay sandwich

Goal: Once at app startup, reparent the React webview into a `GtkOverlay` so we can place a `gtk::Fixed` above it. No child webviews yet — just the container.

### Task 2.1: Verify `wry` is already available; add `gtk` re-export hooks

**Files:**
- Inspect: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`

- [ ] **Step 1: Confirm wry version**

```bash
cargo tree --manifest-path src-tauri/Cargo.toml | grep -E '^[│ ]*[├└]── wry'
```

Expected output should include `wry v0.54.4` (or compatible). If you see a different version, capture it and proceed — the `build_gtk` API exists from 0.35.2+.

- [ ] **Step 2: Confirm gtk + webkit2gtk are exposed**

```bash
cargo tree --manifest-path src-tauri/Cargo.toml | grep -E 'webkit2gtk|^[│ ]*[├└]── gtk '
```

Expected: both present (transitively via tauri / wry).

- [ ] **Step 3: Make wry a direct dependency for build_gtk access**

Open `src-tauri/Cargo.toml` and find the existing `[dependencies]` block. After the `tauri = ...` line, add (under a Linux cfg if there isn't already one in the file):

```toml
[target.'cfg(target_os = "linux")'.dependencies]
wry = "0.54"
gtk = "0.18"
webkit2gtk = "2.0"
```

If a `[target.'cfg(target_os = "linux")'.dependencies]` block already exists with `gtk` (it does — used by `embed.rs`'s `set_bypass_compositor`), augment that block instead. Don't create duplicates.

- [ ] **Step 4: Verify**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: still failing on the `EmbedManager` references in `lib.rs`, but no new errors related to the dependency additions. The wry / gtk / webkit2gtk crates resolve.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "embed: depend on wry + webkit2gtk directly on Linux

Needed for build_gtk(&fixed) + WebView reparent in Phase 2/3."
```

---

### Task 2.2: Linux `setup_overlay` helper

This is GTK widget tree manipulation — not unit-testable. Test by manual smoke at the end of the phase.

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Add a Linux submodule with the reparent helper**

Append to `src-tauri/src/embed.rs`:

```rust
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::Context;
    use gtk::prelude::*;
    use gtk::{Box as GtkBox, Fixed, Orientation, Overlay};

    /// Wraps `gtk::Fixed` in a Send-marker so we can stash it inside
    /// `EmbedHost` (locked by parking_lot's Mutex). All GTK access is
    /// gated by `glib::MainContext::default().invoke` in real call
    /// sites, so the unsafe Send is sound — we never touch the widget
    /// off the main thread.
    pub(crate) struct FixedHandle(pub Fixed);
    unsafe impl Send for FixedHandle {}

    /// Build the `GtkOverlay` sandwich on top of the main React webview
    /// and return the `gtk::Fixed` we'll add child webviews into.
    ///
    /// Topology before:
    ///   GtkApplicationWindow > default_vbox(GtkBox) > [WebKitWebView]
    ///
    /// Topology after:
    ///   GtkApplicationWindow > default_vbox(GtkBox) > [Overlay]
    ///                                                  ├── (base) WebKitWebView
    ///                                                  └── (overlay) Fixed
    pub(crate) fn install_overlay(
        gtk_window: &gtk::ApplicationWindow,
    ) -> Result<FixedHandle> {
        let vbox: GtkBox = gtk_window
            .child()
            .and_then(|w| w.downcast::<GtkBox>().ok())
            .context("main window child is not a GtkBox")?;
        let webview = vbox
            .children()
            .into_iter()
            .find(|c| c.type_().name() == "WebKitWebView")
            .context("no WebKitWebView found in default_vbox")?;

        // Detach the React webview from the vbox, drop it into a new Overlay
        // as the base child, and pack the Overlay back into the vbox.
        vbox.remove(&webview);

        let overlay = Overlay::new();
        let fixed = Fixed::new();
        // base child — the React webview, fills the overlay
        overlay.add(&webview);
        // overlay child — our Fixed, also fills (children inside it are
        // positioned absolutely with `put`)
        overlay.add_overlay(&fixed);

        // Pack the overlay where the webview used to live. Greedy fill so
        // it fills the vbox exactly like the webview did.
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        // The overlay's overlay-child is `fixed`; ensure it's visible too
        // (show_all will have done it, but be explicit).
        fixed.set_visible(true);

        Ok(FixedHandle(fixed))
    }

    /// glib re-exports for the on_main_thread invoker used in lib.rs.
    pub(crate) use gtk::glib;
}

#[cfg(target_os = "linux")]
pub(crate) use linux::FixedHandle;
```

- [ ] **Step 2: Add the `fixed` field to `Inner` (Linux only)**

Locate `struct Inner` in `embed.rs` and modify:

```rust
struct Inner {
    children: HashMap<EmbedKey, ChildEmbed>,
    #[cfg(target_os = "linux")]
    fixed: Option<FixedHandle>,
}
```

Update `EmbedHost::new()`:

```rust
impl EmbedHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                children: HashMap::new(),
                #[cfg(target_os = "linux")]
                fixed: None,
            }),
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn install_fixed(&self, fixed: FixedHandle) {
        self.inner.lock().fixed = Some(fixed);
    }
}
```

- [ ] **Step 3: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: still failing on `lib.rs` references to old `EmbedManager`, but no new errors from `embed.rs`. The Linux module compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed(linux): GtkOverlay sandwich helper, FixedHandle"
```

---

### Task 2.3: Wire the overlay reparent into `Builder::setup`

**Files:**
- Modify: `src-tauri/src/lib.rs:1083` (where the old `EmbedManager` is constructed)

- [ ] **Step 1: Find the construction site**

```bash
grep -n 'EmbedManager::new' src-tauri/src/lib.rs
```

Expected: a single hit around line 1083 (`let embed_mgr = embed::EmbedManager::new();`).

- [ ] **Step 2: Replace with `EmbedHost::new()` and add the Linux setup**

Modify the surrounding `Builder::setup` closure. Find:

```rust
let embed_mgr = embed::EmbedManager::new();
app.manage(embed_mgr.clone());
```

Replace with:

```rust
let embed_host = embed::EmbedHost::new();
app.manage(embed_host.clone());

#[cfg(target_os = "linux")]
{
    use tauri::Manager as _;
    let main = app
        .get_webview_window("main")
        .expect("main window must exist by setup time");
    let host_for_setup = embed_host.clone();
    main.run_on_main_thread(move || {
        if let Ok(gtk_window) = main.gtk_window() {
            match embed::linux::install_overlay(&gtk_window) {
                Ok(fixed) => host_for_setup.install_fixed(fixed),
                Err(e) => log::error!("install_overlay failed: {e:#}"),
            }
        }
    })?;
}
```

Note: `embed::linux` needs to be `pub(crate)` (already is from the `mod linux` declaration if you wrote it as shown — confirm).

- [ ] **Step 3: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: errors are now only about `embed_mount`/`embed_position`/`embed_unmount`/`embed_set_visible` handlers and `unmount_platform` calls in `auth/chaturbate.rs`. Phase 7 + Phase 10 fix those. No errors about the setup or `EmbedHost`.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "lib: wire EmbedHost::new + linux::install_overlay into Builder::setup

App still doesn't compile — the embed IPC handlers and
auth::chaturbate::clear() still reference the old EmbedManager
API. Phase 7 / 10 fix those."
```

---

### Task 2.4: Manual smoke — overlay installs, no regressions

**Files:** none

- [ ] **Step 1: Stub out the broken IPC handlers temporarily so the app builds**

Open `src-tauri/src/lib.rs` and find each of `embed_mount`, `embed_position`, `embed_unmount`, `embed_set_visible`. Replace each body with `Ok(false)` / `Ok(())` and replace the parameter type `State<'_, Arc<embed::EmbedManager>>` with `State<'_, Arc<embed::EmbedHost>>`. Don't remove the handlers — we need the registrations in `generate_handler!` to stay valid until Phase 7.

For `embed_mount`:
```rust
#[tauri::command]
fn embed_mount(
    _app: tauri::AppHandle,
    _state: State<'_, AppState>,
    _embeds: State<'_, Arc<embed::EmbedHost>>,
    _unique_key: String,
    _x: f64, _y: f64, _width: f64, _height: f64,
) -> Result<bool, String> {
    Ok(false)  // Phase 5 fills this in.
}
```

For `embed_position`, `embed_unmount`, `embed_set_visible`: same pattern, no-op bodies, unused-prefixed args.

In `auth/chaturbate.rs::clear()`, find the call to `EmbedManager::unmount_platform` (was around line 305-ish in the old code) and update the import / type to `embed::EmbedHost`. The method name is the same.

- [ ] **Step 2: Build**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 3: Run dev**

```bash
npm run tauri:dev
```

Expected:
1. App launches, main window paints normally.
2. The React UI looks identical to before (overlay is invisible because the `gtk::Fixed` is empty).
3. Layouts, sidebar, channel list, modals all behave normally.
4. No log entries with `install_overlay failed`.
5. No console errors about missing IPC handlers (`embed_mount` returns `false` → React shows the offline placeholder, which is fine because we haven't verified an end-to-end embed yet).

If install_overlay logs an error: capture the message; the most likely cause is the assumption that `gtk_window.child()` is a `GtkBox`. Newer Tauri versions may wrap it differently — adjust `install_overlay` to walk the actual widget tree (use `gtk-inspector` to introspect: launch with `GTK_DEBUG=interactive npm run tauri:dev`).

- [ ] **Step 4: Manually inspect the widget tree**

In another terminal while the app is running:

```bash
GTK_DEBUG=interactive npm run tauri:dev
```

In gtk-inspector → Objects → Widgets, navigate to the main window. You should see:
```
GtkApplicationWindow
└── GtkBox (default_vbox)
    └── GtkOverlay
        ├── WebKitWebView
        └── GtkFixed
```

This is the success criterion for Phase 2.

- [ ] **Step 5: Commit (no code changes; just confirm)**

No commit needed if no edits. If you adjusted `install_overlay` to fix tree-walking, commit that:

```bash
git add src-tauri/src/embed.rs
git commit -m "embed(linux): adjust install_overlay tree-walking for actual main window child shape"
```

---

## Phase 3 — Linux ChildEmbed (real wry webview at fixed coords)

Goal: Build, position, hide, eval, and read cookies on a single Linux child webview. End-of-phase smoke: a hardcoded YouTube embed renders inside the chat pane.

### Task 3.1: `ChildEmbed::build_linux` skeleton

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Define `ChildInner` and Linux build path**

Append to `embed.rs`:

```rust
#[cfg(target_os = "linux")]
pub(crate) struct ChildInner(pub(crate) std::sync::Arc<wry::WebView>);

#[cfg(not(target_os = "linux"))]
pub(crate) struct ChildInner(pub(crate) tauri::webview::Webview);

// Linux-side construction
#[cfg(target_os = "linux")]
mod build_linux {
    use super::*;
    use anyhow::{bail, Context};
    use std::path::PathBuf;
    use wry::{Rect as WryRect, WebViewBuilder};
    use wry::dpi::{LogicalPosition, LogicalSize};

    pub(crate) struct BuildSpec {
        pub url: String,
        pub profile_dir: PathBuf,
        pub bounds: Rect,
        pub init_script: Option<String>,
        pub on_page_load: Option<Box<dyn Fn(&wry::WebView) + Send + Sync>>,
        pub background: (u8, u8, u8, u8),
    }

    pub(crate) fn build_child(
        host_inner: &super::Inner,
        spec: BuildSpec,
    ) -> Result<wry::WebView> {
        let fixed = host_inner
            .fixed
            .as_ref()
            .context("install_overlay was not called yet — gtk::Fixed missing")?;

        let mut builder = WebViewBuilder::new()
            .with_url(&spec.url)
            .with_data_directory(spec.profile_dir)
            .with_background_color(spec.background)
            .with_visible(false) // shown on first PageLoadEvent::Finished
            .with_bounds(WryRect {
                position: LogicalPosition::new(spec.bounds.x, spec.bounds.y).into(),
                size: LogicalSize::new(spec.bounds.w, spec.bounds.h).into(),
            });
        if let Some(init) = spec.init_script {
            builder = builder.with_initialization_script(&init);
        }
        if let Some(on_load) = spec.on_page_load {
            // Wry's on_page_load fires for both Started and Finished. We only
            // care about Finished — guard inside the closure.
            builder = builder.with_on_page_load_handler(move |page_load_event, _url| {
                if matches!(page_load_event, wry::PageLoadEvent::Finished) {
                    // We don't have direct &WebView access in this signature;
                    // the wired-up handler in build_child below will set
                    // visibility + run the script via webview after build.
                    // Phase 6 wires this end-to-end.
                }
            });
            let _ = on_load; // hold the closure alive until Phase 6
        }

        let webview = builder
            .build_gtk(&fixed.0)
            .map_err(|e| anyhow::anyhow!("build_gtk failed: {e}"))?;

        Ok(webview)
    }

    /// Convert a physical-pixel Rect to a logical-pixel `wry::Rect`
    /// using the GTK scale factor.
    pub(crate) fn physical_to_logical(
        bounds: Rect,
        scale_factor: f64,
    ) -> WryRect {
        let s = scale_factor.max(1.0);
        WryRect {
            position: LogicalPosition::new(bounds.x / s, bounds.y / s).into(),
            size: LogicalSize::new((bounds.w / s).max(1.0), (bounds.h / s).max(1.0)).into(),
        }
    }
}
```

(The `on_page_load` closure threading is intentionally stubbed — Phase 6 fills it in. We're proving the build path here.)

- [ ] **Step 2: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean (modulo the stubs we put in lib.rs in Task 2.4).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed(linux): ChildInner type + build_child via wry build_gtk"
```

---

### Task 3.2: Hardcoded smoke embed

Verify a single child webview renders correctly inside our `gtk::Fixed`. This is throwaway code we delete at end of Task 3.6.

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add a temp smoke command**

Inside `lib.rs`, add near the other `#[tauri::command]` definitions:

```rust
#[cfg(target_os = "linux")]
#[tauri::command]
fn embed_smoke(
    _app: tauri::AppHandle,
    embeds: State<'_, Arc<embed::EmbedHost>>,
) -> Result<(), String> {
    let g = embeds.inner_for_smoke().lock();
    let spec = embed::build_linux::BuildSpec {
        url: "https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v=jfKfPfyJRdk"
            .to_string(),
        profile_dir: std::env::temp_dir().join("livestreamlist-smoke"),
        bounds: embed::Rect::new(400.0, 200.0, 400.0, 600.0),
        init_script: None,
        on_page_load: None,
        background: (9, 9, 11, 255),
    };
    let _wv = embed::build_linux::build_child(&g, spec).map_err(|e| e.to_string())?;
    // Leak intentionally; smoke only.
    std::mem::forget(_wv);
    Ok(())
}
```

For this to compile, expose a smoke-only accessor on `EmbedHost`. In `embed.rs`:

```rust
impl EmbedHost {
    #[cfg(target_os = "linux")]
    pub(crate) fn inner_for_smoke(&self) -> &Mutex<Inner> {
        &self.inner
    }
}
```

Add `embed_smoke` to `generate_handler!` (Linux only):

```rust
#[cfg(target_os = "linux")]
let smoke_handlers = tauri::generate_handler![embed_smoke];
// ... register alongside other handlers
```

Simpler: just unconditionally include the handler — gate the body, leave a stub on non-Linux:

```rust
#[cfg(not(target_os = "linux"))]
#[tauri::command]
fn embed_smoke() -> Result<(), String> { Ok(()) }
```

- [ ] **Step 2: Build + run**

```bash
npm run tauri:dev
```

In the app's dev console (Inspect → Console):

```js
window.__TAURI_INTERNALS__.invoke('embed_smoke')
```

Expected: a YouTube live-chat iframe-style page renders inside the main window at (400, 200) with size 400×600. Dark-themed.

If it renders but at wrong coords / wrong z-order: the `GtkOverlay` arrangement is off. Try `GTK_DEBUG=interactive` and confirm the Fixed is the overlay child (not the base), and that `add_overlay` was called rather than `add` for it.

If the webview is invisible: it may be hidden by `with_visible(false)`. For smoke only, change to `.with_visible(true)`.

If the webview renders but doesn't accept input: this is the GtkOverlay click-pass-through behavior. Set `overlay.set_overlay_pass_through(&fixed, false)` in `install_overlay`, and ensure individual child webviews each accept events. (Wry should do this by default; double-check via gtk-inspector that the webview's `can-focus` is true.)

- [ ] **Step 3: Drag and resize the main window**

Confirm the embed stays glued to its position (because GTK's layout reflows it automatically — no IPC needed). This is the moment of truth: no `onMoved` listener, no chase-the-cursor lag.

- [ ] **Step 4: Commit (smoke as a checkpoint)**

```bash
git add src-tauri/src/lib.rs src-tauri/src/embed.rs
git commit -m "embed(smoke): one-shot YT embed_smoke command for Phase 3 checkpoint

Removed in Phase 5 once the real mount path lands."
```

---

### Task 3.3: `ChildEmbed::set_bounds` on Linux

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Implement set_bounds**

```rust
impl ChildEmbed {
    pub(crate) fn set_bounds(&mut self, bounds: Rect, scale_factor: f64) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            let wry_rect = build_linux::physical_to_logical(bounds, scale_factor);
            self.inner.0.set_bounds(wry_rect)
                .map_err(|e| anyhow::anyhow!("set_bounds: {e}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            use tauri::{PhysicalPosition, PhysicalSize};
            self.inner
                .0
                .set_position(PhysicalPosition::new(bounds.x, bounds.y))
                .map_err(|e| anyhow::anyhow!("set_position: {e}"))?;
            self.inner
                .0
                .set_size(PhysicalSize::new(bounds.w as u32, bounds.h as u32))
                .map_err(|e| anyhow::anyhow!("set_size: {e}"))?;
            let _ = scale_factor;
        }
        self.bounds = bounds;
        Ok(())
    }
}
```

- [ ] **Step 2: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 3: Manual smoke**

Modify the `embed_smoke` command to additionally:
1. Sleep 2 seconds.
2. Call `set_bounds(Rect::new(100.0, 100.0, 600.0, 400.0))` on the smoke webview.

Run again. Expected: the embed pops to (400, 200, 400×600), then 2 seconds later moves to (100, 100, 600×400). No flicker.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: ChildEmbed::set_bounds for Linux + non-Linux"
```

---

### Task 3.4: `ChildEmbed::set_visible`

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Implement**

```rust
impl ChildEmbed {
    pub(crate) fn set_visible(&mut self, visible: bool) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.inner.0.set_visible(visible)
                .map_err(|e| anyhow::anyhow!("set_visible: {e}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            if visible {
                self.inner.0.show().map_err(|e| anyhow::anyhow!("show: {e}"))?;
            } else {
                self.inner.0.hide().map_err(|e| anyhow::anyhow!("hide: {e}"))?;
            }
        }
        self.visible = visible;
        Ok(())
    }
}
```

- [ ] **Step 2: Smoke**

In `embed_smoke`, add: 4 seconds after build, call `set_visible(false)`. Confirm the embed disappears. 6 seconds in, call `set_visible(true)`. Confirm reappear.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: ChildEmbed::set_visible for Linux + non-Linux"
```

---

### Task 3.5: `ChildEmbed::eval` and `cookies_for_url`

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Implement eval**

```rust
impl ChildEmbed {
    pub(crate) fn eval(&self, js: &str) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.inner.0.evaluate_script(js)
                .map_err(|e| anyhow::anyhow!("evaluate_script: {e}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.inner.0.eval(js).map_err(|e| anyhow::anyhow!("eval: {e}"))?;
        }
        Ok(())
    }
}
```

- [ ] **Step 2: Implement cookies_for_url**

```rust
use url::Url;

#[derive(Clone, Debug)]
pub struct CookieView {
    pub name: String,
    pub value: String,
}

impl ChildEmbed {
    pub(crate) fn cookies_for_url(&self, url: &Url) -> Result<Vec<CookieView>> {
        #[cfg(target_os = "linux")]
        {
            // wry 0.54 exposes cookies_for_url via WebView; if your wry version
            // is older, fall back to webkit2gtk::CookieManager directly. We
            // keep the same shape regardless.
            // Cross-check the wry version (cargo tree | grep wry) — adjust here.
            let cookies = self.inner.0.cookies_for_url(url.as_str())
                .map_err(|e| anyhow::anyhow!("cookies_for_url: {e}"))?;
            Ok(cookies.into_iter()
                .map(|c| CookieView {
                    name: c.name().to_string(),
                    value: c.value().to_string(),
                })
                .collect())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let cookies = self.inner.0.cookies_for_url(url.clone())
                .map_err(|e| anyhow::anyhow!("cookies_for_url: {e}"))?;
            Ok(cookies.into_iter()
                .map(|c| CookieView {
                    name: c.name().to_string(),
                    value: c.value().to_string(),
                })
                .collect())
        }
    }
}
```

If wry 0.54.4's `WebView::cookies_for_url` doesn't exist (verify via `cargo doc --open` on the wry crate, search for `cookies_for_url`), use the webkit2gtk fallback:

```rust
#[cfg(target_os = "linux")]
{
    use webkit2gtk::traits::{WebViewExt, WebContextExt, CookieManagerExt};
    let context = self.inner.0.context().context("no web context")?;
    let cookie_manager = context.cookie_manager().context("no cookie manager")?;
    // Use the synchronous cookies API; if only async is available, run it
    // via gtk::glib::MainContext::default().block_on(...).
    // ...
}
```

(If forced to use webkit2gtk's async API: park the implementation behind an `unimplemented!()` for now and revisit in Phase 6 when the auth-drift hook is wired. The auth verification path runs inside `on_page_load` callbacks which already have a webview reference — we have flexibility there.)

- [ ] **Step 3: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

If wry's `cookies_for_url` doesn't exist on Linux, mark it `unimplemented!()` and TODO-flag in a code comment to revisit in Phase 6 Task 6.3. Phase 6 will implement via webkit2gtk's CookieManager directly.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: ChildEmbed::eval + cookies_for_url

Linux cookies path may need a webkit2gtk::CookieManager
fallback if wry 0.54.4 doesn't expose cookies_for_url. Phase 6
auth-drift hook will iron out which path is used."
```

---

### Task 3.6: Tear down the smoke command

**Files:**
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/embed.rs`

- [ ] **Step 1: Remove `embed_smoke` and `inner_for_smoke`**

Delete:
- The `embed_smoke` command in `lib.rs` (both the Linux and non-Linux variants)
- Its registration in `generate_handler!`
- `EmbedHost::inner_for_smoke` in `embed.rs`

- [ ] **Step 2: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/embed.rs
git commit -m "embed: remove smoke command (Phase 3 checkpoint cleared)"
```

---

## Phase 4 — macOS / Windows ChildEmbed

Goal: Mirror Phase 3 on the non-Linux side via `WebviewWindow::add_child`. Since you don't have macOS / Windows local test environments described in CLAUDE.md, the smoke verification at the end of this phase is a `cargo check --target` cross-compile and a code-review pass — full runtime verification happens during the multi-platform manual smoke at the end of Phase 9.

### Task 4.1: Non-Linux build path

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Add a non-Linux build module**

```rust
#[cfg(not(target_os = "linux"))]
mod build_other {
    use super::*;
    use anyhow::Context;
    use std::path::PathBuf;
    use tauri::utils::config::Color;
    use tauri::webview::{Webview, WebviewBuilder, WebviewUrl, PageLoadEvent};
    use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

    pub(crate) struct BuildSpec {
        pub label: String,
        pub url: String,
        pub profile_dir: PathBuf,
        pub bounds: Rect,
        pub init_script: Option<String>,
        pub background: (u8, u8, u8, u8),
        pub on_page_load: Option<
            Box<dyn Fn(Webview, PageLoadEvent) + Send + Sync>,
        >,
    }

    pub(crate) fn build_child(app: &AppHandle, spec: BuildSpec) -> Result<Webview> {
        let main = app
            .get_webview_window("main")
            .context("main window unavailable")?;
        let bg = Color(spec.background.0, spec.background.1, spec.background.2, spec.background.3);
        let url = spec.url.parse::<url::Url>().context("parsing embed URL")?;
        let mut builder = WebviewBuilder::new(&spec.label, WebviewUrl::External(url))
            .data_directory(spec.profile_dir)
            .background_color(bg)
            .visible(false);
        if let Some(s) = spec.init_script {
            builder = builder.initialization_script(&s);
        }
        if let Some(handler) = spec.on_page_load {
            builder = builder.on_page_load(move |w, payload| {
                handler(w, payload.event());
            });
        }
        let position = PhysicalPosition::new(spec.bounds.x, spec.bounds.y);
        let size = PhysicalSize::new(spec.bounds.w as u32, spec.bounds.h as u32);
        let webview = main
            .add_child(builder, position, size)
            .map_err(|e| anyhow::anyhow!("add_child: {e}"))?;
        Ok(webview)
    }
}
```

- [ ] **Step 2: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean on Linux.

- [ ] **Step 3: Cross-compile for the other targets**

```bash
rustup target list --installed
```

If you don't have `aarch64-apple-darwin` or `x86_64-pc-windows-msvc` installed, install `x86_64-pc-windows-gnu` for a cheap sanity check:

```bash
rustup target add x86_64-pc-windows-gnu
cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-gnu
```

Expected: errors about missing native dependencies are fine (we can't fully build for Windows without MinGW); but Tauri / wry source-level errors should be zero. If you see compile errors in `embed.rs`'s non-Linux module, fix them before continuing.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed(other): build_child via Tauri add_child for macOS/Windows"
```

---

## Phase 5 — `EmbedHost::mount` end-to-end

Goal: Wire the live channel store + auth profile dirs + URL construction + per-platform build into a single `EmbedHost::mount` that returns a registered `ChildEmbed`. After this phase, `embed_mount` IPC works end-to-end (no auth/CSS yet — those land in Phase 6).

### Task 5.1: URL construction helpers

Pure-Rust, unit-testable.

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Port `yt_video_id` (lifted from /tmp/embed-old.rs)**

```rust
fn yt_video_id_from_thumb(thumbnail_url: &str) -> Option<String> {
    let trim = thumbnail_url.trim();
    let marker = "/vi/";
    let start = trim.find(marker)? + marker.len();
    let rest = &trim[start..];
    let end = rest.find('/').unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() { None } else { Some(id.to_string()) }
}
```

- [ ] **Step 2: URL construction by platform**

```rust
fn build_url_for(
    platform: Platform,
    channel_id: &str,
    livestream: Option<&crate::channels::Livestream>,
) -> Option<String> {
    match platform {
        Platform::Youtube => {
            let ls = livestream.filter(|l| l.is_live)?;
            let video_id = ls.video_id.clone()
                .or_else(|| ls.thumbnail_url.as_deref().and_then(yt_video_id_from_thumb))?;
            Some(format!(
                "https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v={video_id}"
            ))
        }
        Platform::Chaturbate => {
            Some(format!("https://chaturbate.com/{channel_id}/"))
        }
        Platform::Twitch | Platform::Kick => None,
    }
}
```

- [ ] **Step 3: Tests**

Append to `mod tests`:

```rust
#[test]
fn yt_video_id_from_thumb_extracts_id() {
    assert_eq!(
        yt_video_id_from_thumb("https://i.ytimg.com/vi/abc123/maxresdefault.jpg"),
        Some("abc123".to_string())
    );
    assert_eq!(yt_video_id_from_thumb(""), None);
    assert_eq!(yt_video_id_from_thumb("https://nope.example/"), None);
}

#[test]
fn build_url_youtube_uses_video_id() {
    use crate::channels::Livestream;
    let mut ls = Livestream::default();
    ls.is_live = true;
    ls.video_id = Some("abc123".to_string());
    let url = build_url_for(Platform::Youtube, "UC1", Some(&ls));
    assert_eq!(
        url,
        Some("https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v=abc123".to_string())
    );
}

#[test]
fn build_url_youtube_offline_returns_none() {
    use crate::channels::Livestream;
    let ls = Livestream::default();
    assert_eq!(build_url_for(Platform::Youtube, "UC1", Some(&ls)), None);
}

#[test]
fn build_url_chaturbate_uses_channel_id() {
    let url = build_url_for(Platform::Chaturbate, "alice", None);
    assert_eq!(url, Some("https://chaturbate.com/alice/".to_string()));
}

#[test]
fn build_url_twitch_kick_returns_none() {
    assert_eq!(build_url_for(Platform::Twitch, "anyone", None), None);
    assert_eq!(build_url_for(Platform::Kick, "anyone", None), None);
}
```

(`Livestream::default()` may not exist; if so, construct one explicitly with the minimal fields. Open `src-tauri/src/channels.rs` and check.)

- [ ] **Step 4: Run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml embed::
```

Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: URL builders + yt_video_id_from_thumb (unit-tested)"
```

---

### Task 5.2: `EmbedHost::mount` body

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Implement profile-dir lookup**

```rust
fn profile_dir(platform: Platform) -> Result<std::path::PathBuf> {
    match platform {
        Platform::Youtube => crate::auth::youtube::webview_profile_dir(),
        Platform::Chaturbate => crate::auth::chaturbate::webview_profile_dir(),
        Platform::Twitch | Platform::Kick => {
            anyhow::bail!("no webview profile dir for {:?}", platform)
        }
    }
}
```

- [ ] **Step 2: Implement `mount`**

```rust
use crate::channels::SharedStore;

const ZINC_950: (u8, u8, u8, u8) = (9, 9, 11, 255);

impl EmbedHost {
    pub fn mount(
        &self,
        app: &tauri::AppHandle,
        store: &SharedStore,
        unique_key: &str,
        bounds: Rect,
    ) -> Result<bool> {
        let scale_factor = {
            use tauri::Manager as _;
            app.get_webview_window("main")
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0)
        };

        // Resolve platform + URL
        let (channel, livestream) = {
            let g = store.lock();
            let channel_key = crate::channels::channel_key_of(unique_key);
            let ch = g.channels()
                .iter()
                .find(|c| c.unique_key() == channel_key)
                .cloned();
            let ls = g.snapshot()
                .into_iter()
                .find(|l| l.unique_key == unique_key);
            (ch, ls)
        };
        let Some(channel) = channel else {
            anyhow::bail!("unknown channel {unique_key}");
        };
        let Some(url) = build_url_for(channel.platform, &channel.channel_id, livestream.as_ref()) else {
            return Ok(false); // offline
        };

        // Idempotent: if already mounted, just resize.
        {
            let mut g = self.inner.lock();
            if let Some(existing) = g.children.get_mut(unique_key) {
                #[cfg(target_os = "linux")]
                {
                    let wry_rect = build_linux::physical_to_logical(bounds, scale_factor);
                    let _ = existing.inner.0.set_bounds(wry_rect);
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = existing.inner.0.set_position(tauri::PhysicalPosition::new(bounds.x, bounds.y));
                    let _ = existing.inner.0.set_size(tauri::PhysicalSize::new(bounds.w as u32, bounds.h as u32));
                }
                existing.bounds = bounds;
                return Ok(true);
            }
        }

        let pdir = profile_dir(channel.platform)?;

        #[cfg(target_os = "linux")]
        let inner = {
            let g = self.inner.lock();
            let spec = build_linux::BuildSpec {
                url,
                profile_dir: pdir,
                bounds,
                init_script: None, // Phase 6
                on_page_load: None, // Phase 6
                background: ZINC_950,
            };
            // Note: build_linux::physical_to_logical was applied via with_bounds inside
            // BuildSpec — but BuildSpec carries physical Rect. Update build_child to
            // convert before calling with_bounds:
            let webview = build_linux::build_child(&g, spec)?;
            ChildInner(std::sync::Arc::new(webview))
        };

        #[cfg(not(target_os = "linux"))]
        let inner = {
            let label = format!("embed-{}-{}", platform_label(channel.platform), slugify(unique_key));
            let spec = build_other::BuildSpec {
                label,
                url,
                profile_dir: pdir,
                bounds,
                init_script: None,
                background: ZINC_950,
                on_page_load: None,
            };
            ChildInner(build_other::build_child(app, spec)?)
        };

        let child = ChildEmbed {
            platform: channel.platform,
            bounds,
            visible: true,
            inner,
        };
        let mut g = self.inner.lock();
        g.children.insert(unique_key.to_string(), child);
        // The first show happens on PageLoadEvent::Finished — Phase 6 wires this.
        Ok(true)
    }
}

#[cfg(not(target_os = "linux"))]
fn platform_label(p: Platform) -> &'static str {
    match p {
        Platform::Youtube => "youtube",
        Platform::Chaturbate => "chaturbate",
        _ => "other",
    }
}

#[cfg(not(target_os = "linux"))]
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
```

- [ ] **Step 3: Update `BuildSpec` + `build_linux::build_child` to do the physical→logical conversion internally**

In `build_linux::build_child`, replace the `with_bounds(...)` line:

```rust
let scale_factor = fixed.0
    .toplevel()
    .map(|w| w.scale_factor() as f64)
    .unwrap_or(1.0);
let wry_rect = physical_to_logical(spec.bounds, scale_factor);
let mut builder = WebViewBuilder::new()
    .with_url(&spec.url)
    .with_data_directory(spec.profile_dir)
    .with_background_color(spec.background)
    .with_visible(false)
    .with_bounds(wry_rect);
```

- [ ] **Step 4: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: EmbedHost::mount end-to-end (no auth/CSS yet)

Idempotent — mounting an existing key just resizes. Offline
channels return Ok(false). Per-platform build path dispatched
inline; CSS injection + auth drift detection deferred to
Phase 6."
```

---

### Task 5.3: `EmbedHost::set_bounds` and `set_visible`

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Implement**

```rust
impl EmbedHost {
    pub fn set_bounds(&self, app: &tauri::AppHandle, key: &str, bounds: Rect) -> Result<()> {
        let scale_factor = {
            use tauri::Manager as _;
            app.get_webview_window("main")
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0)
        };
        let mut g = self.inner.lock();
        if let Some(child) = g.children.get_mut(key) {
            child.set_bounds(bounds, scale_factor)?;
        }
        Ok(())
    }

    pub fn set_visible(&self, key: &str, visible: bool) -> Result<()> {
        let mut g = self.inner.lock();
        if let Some(child) = g.children.get_mut(key) {
            child.set_visible(visible)?;
        }
        Ok(())
    }
}
```

- [ ] **Step 2: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: EmbedHost::set_bounds + set_visible"
```

---

## Phase 6 — CSS injection + auth drift

Goal: First-paint discipline, dark theme injection on YouTube, DOM isolation on Chaturbate, and auth drift detection on Chaturbate's `PageLoadEvent::Finished`. After this phase, the embed visually matches today's behavior.

### Task 6.1: Port the static CSS / JS scripts

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Lift constants from /tmp/embed-old.rs**

Append to `embed.rs`:

```rust
const YT_THEME_CSS: &str = r#"
html, body { background: #09090b !important; }
yt-live-chat-renderer, yt-live-chat-app { background: #09090b !important; }
yt-live-chat-header-renderer { background: #09090b !important; border: 0 !important; }
::-webkit-scrollbar { width: 8px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: #27272a; border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: #3f3f46; }
"#;

const CB_ISOLATE_JS: &str = r#"
(function() {
  function apply() {
    try {
      var oldStyle = document.getElementById('lsl-chat-iso');
      if (oldStyle) oldStyle.remove();
      var prio = ['#ChatTabContainer', '#defchat'];
      var fall = ['.chat-holder', '#chat-box', '.chat-container'];
      var chatEl = null;
      for (var i = 0; i < prio.length && !chatEl; i++) chatEl = document.querySelector(prio[i]);
      if (!chatEl) {
        for (var i = 0; i < fall.length && !chatEl; i++) {
          var el = document.querySelector(fall[i]);
          if (el && el.offsetHeight > 50) chatEl = el;
        }
      }
      if (!chatEl) return false;
      var anc = chatEl.parentElement;
      while (anc && anc !== document.documentElement) {
        anc.setAttribute('data-lsl-path', '');
        anc = anc.parentElement;
      }
      chatEl.setAttribute('data-lsl-chat', '');
      var s = document.createElement('style');
      s.id = 'lsl-chat-iso';
      s.textContent = [
        'html,body{margin:0!important;padding:0!important;overflow:hidden!important;background:#09090b!important}',
        'body>*:not([data-lsl-path]):not([data-lsl-chat]){display:none!important}',
        '[data-lsl-path]>*:not([data-lsl-path]):not([data-lsl-chat]){display:none!important}',
        '[data-lsl-path]{display:block!important;position:static!important;margin:0!important;padding:0!important;width:100%!important;height:100%!important}',
        '[data-lsl-chat]{display:flex!important;flex-direction:column!important;position:fixed!important;top:0!important;left:0!important;right:0!important;bottom:0!important;width:100%!important;height:100%!important;z-index:1!important}',
      ].join('');
      document.head.appendChild(s);
      return true;
    } catch (e) { return false; }
  }
  if (apply()) return;
  var tries = 0;
  var iv = setInterval(function() { tries++; if (apply() || tries > 80) clearInterval(iv); }, 250);
})();
"#;

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "''".to_string())
}

/// JS the embed should run after every page load.
fn injection_for(platform: Platform) -> Option<String> {
    match platform {
        Platform::Youtube => Some(format!(
            "(function(){{var s=document.createElement('style');s.textContent={};document.head.appendChild(s);}})();",
            json_string(YT_THEME_CSS),
        )),
        Platform::Chaturbate => Some(CB_ISOLATE_JS.to_string()),
        _ => None,
    }
}
```

- [ ] **Step 2: `cargo check` + `cargo test`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml embed::
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: port YT_THEME_CSS + CB_ISOLATE_JS + injection_for"
```

---

### Task 6.2: Wire `on_page_load` + first-paint show

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Linux on_page_load**

In `build_linux::build_child`, after creating the WebViewBuilder, attach a page-load handler that captures the platform + an `Arc<EmbedHost>` to look up itself:

```rust
// Build a separate handler that can mutate the webview after build.
// wry's on_page_load fires for both Started and Finished — guard inside.
let platform = spec.platform;
let webview_arc: std::sync::Arc<std::sync::OnceLock<wry::WebView>> =
    std::sync::Arc::new(std::sync::OnceLock::new());
let arc_for_handler = webview_arc.clone();
let builder = builder.with_on_page_load_handler(move |event, _url| {
    if !matches!(event, wry::PageLoadEvent::Finished) { return; }
    let Some(wv) = arc_for_handler.get() else { return; };
    let _ = wv.set_visible(true);
    if let Some(js) = injection_for(platform) {
        let _ = wv.evaluate_script(&js);
    }
    if platform == Platform::Chaturbate {
        verify_chaturbate_auth_linux(wv);
    }
});

let webview = builder
    .build_gtk(&fixed.0)
    .map_err(|e| anyhow::anyhow!("build_gtk failed: {e}"))?;
let _ = webview_arc.set(webview);
let webview = webview_arc.get().expect("set above").clone();
```

This is where the `BuildSpec` needs a `platform: Platform` field. Add it.

(`wry::WebView` may not implement `Clone` — if not, drop the OnceLock dance and instead emit page-load events as a tauri Emitter signal, then wire the handler in `lib.rs` — see Step 5 for a fallback.)

- [ ] **Step 2: Non-Linux on_page_load**

In `build_other::build_child`:

```rust
let platform = spec.platform; // add this field to BuildSpec
let app_handle = app.clone();
let on_load = move |w: tauri::webview::Webview, event: PageLoadEvent| {
    if !matches!(event, PageLoadEvent::Finished) { return; }
    let _ = w.show();
    if let Some(js) = injection_for(platform) {
        let _ = w.eval(&js);
    }
    if platform == Platform::Chaturbate {
        verify_chaturbate_auth_other(&w, &app_handle);
    }
};
let mut builder = builder.on_page_load(move |w, payload| on_load(w, payload.event()));
```

- [ ] **Step 3: Add `platform` to both `BuildSpec`s and pass it from `mount`**

Update both `BuildSpec` structs to include `pub platform: Platform`. In `EmbedHost::mount`, set `platform: channel.platform` when constructing the spec.

- [ ] **Step 4: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: errors about `verify_chaturbate_auth_linux` / `verify_chaturbate_auth_other` not existing yet — that's Task 6.3.

- [ ] **Step 5: Fallback — if `wry::WebView` is not `Clone`**

If the Linux build fails because `wry::WebView` can't be cloned/captured, replace the on-page-load + visibility/script approach with a tauri-event-based handler:

In `build_linux::build_child`, emit a tauri event from the closure:

```rust
let app_handle_for_handler = /* clone of AppHandle */;
let key_for_handler = unique_key.to_string();
let builder = builder.with_on_page_load_handler(move |event, _url| {
    if !matches!(event, wry::PageLoadEvent::Finished) { return; }
    let _ = app_handle_for_handler.emit(
        "embed:page-load:finished",
        serde_json::json!({ "key": key_for_handler, "platform": format!("{:?}", platform) }),
    );
});
```

In `lib.rs::run`'s setup, listen for `embed:page-load:finished` and look up the child by key in the EmbedHost, calling `child.set_visible(true) + child.eval(&injection_for(platform))`. Re-route the chaturbate auth verification through this same listener.

This is more event-driven but keeps the code clean. Pick whichever path matches your wry version's actual API.

Update build_linux::BuildSpec to take `app_handle` and `unique_key` if going this route.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: on_page_load wires first-paint show + per-platform JS injection"
```

---

### Task 6.3: `verify_chaturbate_auth` for both platforms

**Files:**
- Modify: `src-tauri/src/embed.rs`

- [ ] **Step 1: Port the auth-drift verifier**

Append to `embed.rs`:

```rust
#[derive(Clone, serde::Serialize)]
struct ChaturbateAuthEvent {
    signed_in: bool,
    reason: &'static str, // "ok" | "session_expired" | "not_logged_in"
}

fn classify_chaturbate_auth(signed_in: bool, stamp_present: bool) -> &'static str {
    if signed_in {
        "ok"
    } else if stamp_present {
        "session_expired"
    } else {
        "not_logged_in"
    }
}

fn handle_chaturbate_auth_outcome(app: &tauri::AppHandle, signed_in: bool) {
    use tauri::Emitter as _;
    let stamp_present = matches!(crate::auth::chaturbate::load(), Ok(Some(_)));
    let reason = classify_chaturbate_auth(signed_in, stamp_present);
    if signed_in {
        if let Err(e) = crate::auth::chaturbate::touch_verified() {
            log::warn!("touch_verified failed: {e:#}");
        }
    } else if stamp_present {
        if let Err(e) = crate::auth::chaturbate::clear_stamp_only() {
            log::warn!("clear_stamp_only (drift) failed: {e:#}");
        }
    }
    let payload = ChaturbateAuthEvent { signed_in, reason };
    if let Err(e) = app.emit("chat:auth:chaturbate", payload) {
        log::warn!("emit chat:auth:chaturbate: {e:#}");
    }
}

#[cfg(test)]
mod auth_classify_tests {
    use super::*;
    #[test]
    fn ok_when_signed_in() {
        assert_eq!(classify_chaturbate_auth(true, true), "ok");
        assert_eq!(classify_chaturbate_auth(true, false), "ok");
    }
    #[test]
    fn session_expired_when_stamp_lies() {
        assert_eq!(classify_chaturbate_auth(false, true), "session_expired");
    }
    #[test]
    fn not_logged_in_when_neither() {
        assert_eq!(classify_chaturbate_auth(false, false), "not_logged_in");
    }
}
```

- [ ] **Step 2: Linux verifier — read cookies via webkit2gtk if needed**

If `wry::WebView::cookies_for_url` works (Task 3.5), use it. Otherwise:

```rust
#[cfg(target_os = "linux")]
fn verify_chaturbate_auth_linux(webview: &wry::WebView, app: &tauri::AppHandle) {
    let site: url::Url = match "https://chaturbate.com/".parse() {
        Ok(u) => u,
        Err(_) => return,
    };
    let signed_in = match webview.cookies_for_url(site.as_str()) {
        Ok(jar) => jar.iter().any(|c| c.name() == "sessionid" && !c.value().is_empty()),
        Err(e) => {
            log::warn!("verify_chaturbate_auth cookies_for_url: {e:#}");
            return; // transient, don't flap
        }
    };
    handle_chaturbate_auth_outcome(app, signed_in);
}
```

Fallback if wry's cookies API is async-only — use webkit2gtk's CookieManager:

```rust
#[cfg(target_os = "linux")]
fn verify_chaturbate_auth_linux(webview: &wry::WebView, app: &tauri::AppHandle) {
    use webkit2gtk::traits::{WebViewExt, WebContextExt, CookieManagerExt};
    let context = match webview.context() { Some(c) => c, None => return };
    let cookie_manager = match context.cookie_manager() { Some(c) => c, None => return };
    let app_handle = app.clone();
    cookie_manager.cookies("https://chaturbate.com/", gtk::gio::Cancellable::NONE, move |result| {
        let signed_in = match result {
            Ok(jar) => jar.iter().any(|c| c.name().map(|n| n.as_str() == "sessionid").unwrap_or(false)
                && !c.value().map(|v| v.is_empty()).unwrap_or(true)),
            Err(_) => return,
        };
        handle_chaturbate_auth_outcome(&app_handle, signed_in);
    });
}
```

- [ ] **Step 3: Non-Linux verifier**

```rust
#[cfg(not(target_os = "linux"))]
fn verify_chaturbate_auth_other(webview: &tauri::webview::Webview, app: &tauri::AppHandle) {
    let site: url::Url = match "https://chaturbate.com/".parse() {
        Ok(u) => u,
        Err(_) => return,
    };
    let signed_in = match webview.cookies_for_url(site) {
        Ok(jar) => jar.iter().any(|c| c.name() == "sessionid" && !c.value().is_empty()),
        Err(e) => {
            log::warn!("verify_chaturbate_auth cookies_for_url: {e:#}");
            return;
        }
    };
    handle_chaturbate_auth_outcome(app, signed_in);
}
```

- [ ] **Step 4: Pass `AppHandle` into both `BuildSpec`s**

Add `pub app: tauri::AppHandle` to both `BuildSpec`s. Pass `app.clone()` in `mount`. Capture into the on_page_load closure.

- [ ] **Step 5: `cargo check` + `cargo test`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml embed::
```

Expected: clean. The 3 `auth_classify_tests` pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "embed: chaturbate auth-drift verification on page-load-finished

Mirrors the old verify_chaturbate_auth: reads sessionid cookie
from the embed's profile, emits chat:auth:chaturbate, and
calls clear_stamp_only on drift. classify_chaturbate_auth is
a pure helper with unit tests."
```

---

## Phase 7 — IPC commands

Goal: Replace the stubbed handlers from Task 2.4 with real implementations that delegate to `EmbedHost`.

### Task 7.1: Real `embed_mount` + `embed_bounds` + `embed_set_visible` + `embed_unmount`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Replace stubbed handlers**

Find the stubbed `embed_mount`, `embed_position`, `embed_set_visible`, `embed_unmount` from Task 2.4 and replace with:

```rust
#[tauri::command]
fn embed_mount(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<bool, String> {
    embeds
        .mount(&app, &state.store, &unique_key, embed::Rect::new(x, y, width, height))
        .map_err(err_string)
}

#[tauri::command]
fn embed_bounds(
    app: tauri::AppHandle,
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    embeds
        .set_bounds(&app, &unique_key, embed::Rect::new(x, y, width, height))
        .map_err(err_string)
}

#[tauri::command]
fn embed_set_visible(
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    visible: bool,
) -> Result<(), String> {
    embeds.set_visible(&unique_key, visible).map_err(err_string)
}

#[tauri::command]
fn embed_unmount(embeds: State<'_, Arc<embed::EmbedHost>>, unique_key: String) {
    embeds.unmount(&unique_key);
}
```

- [ ] **Step 2: Update `generate_handler!`**

Find the call to `tauri::generate_handler![...]` in `lib.rs::run` and replace `embed_position` with `embed_bounds`. The other names (`embed_mount`, `embed_set_visible`, `embed_unmount`) stay.

- [ ] **Step 3: `cargo check`**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "lib: real embed_* handlers delegating to EmbedHost"
```

---

## Phase 8 — Frontend `EmbedLayer` + `EmbedSlot`

Goal: A global registry component arbitrates which slot is canonical for each `EmbedKey` and dispatches IPC. The old `EmbeddedChat.jsx` is replaced by a slim `EmbedSlot.jsx`.

### Task 8.1: `ipc.js` wrappers

**Files:**
- Modify: `src/ipc.js`

- [ ] **Step 1: Update wrappers**

Find the existing `embedMount`, `embedPosition`, `embedUnmount` in `ipc.js` and replace with:

```js
export const embedMount = (uniqueKey, x, y, width, height) =>
  invoke('embed_mount', { uniqueKey, x, y, width, height });

export const embedBounds = (uniqueKey, x, y, width, height) =>
  invoke('embed_bounds', { uniqueKey, x, y, width, height });

export const embedSetVisible = (uniqueKey, visible) =>
  invoke('embed_set_visible', { uniqueKey, visible });

export const embedUnmount = (uniqueKey) =>
  invoke('embed_unmount', { uniqueKey });
```

Update mock fallbacks (search for `embed_mount` in the same file's mock block) to match.

- [ ] **Step 2: Search for callers using the old names**

```bash
grep -rn 'embedPosition\|setVisibleAll' src/
```

Expected: only inside the old `EmbeddedChat.jsx` (which we're about to replace) and possibly App.jsx's modal hook. Note these for Task 8.5.

- [ ] **Step 3: Commit**

```bash
git add src/ipc.js
git commit -m "ipc: rename embedPosition→embedBounds; embedSetVisible takes a key"
```

---

### Task 8.2: `EmbedLayer` component + context

**Files:**
- Create: `src/components/EmbedLayer.jsx`

- [ ] **Step 1: Write the component**

```jsx
import { createContext, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { embedMount, embedBounds, embedSetVisible, embedUnmount } from '../ipc.js';

export const EmbedLayerContext = createContext(null);

/**
 * Global registry that arbitrates which <EmbedSlot> is canonical per
 * EmbedKey and dispatches embed_* IPC accordingly. One slot per key
 * may be active at a time; multiple slots with the same key are
 * allowed only if at most one is `active`.
 */
export default function EmbedLayer({ children, modalOpen }) {
    // key → { activeSlotId, refs: Map<slotId, { ref, active }> }
    const registry = useRef(new Map());
    // mounted keys (in Rust) — used for cleanup
    const mountedKeys = useRef(new Set());

    const reflowKey = useCallback((key) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        const active = [...entry.refs.values()].find((s) => s.active);
        if (!active || !active.ref.current) {
            // No active slot for this key — hide if mounted
            if (mountedKeys.current.has(key)) {
                embedSetVisible(key, false).catch(() => {});
            }
            return;
        }
        const r = active.ref.current.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        const x = r.left * dpr;
        const y = r.top * dpr;
        const w = Math.max(1, r.width) * dpr;
        const h = Math.max(1, r.height) * dpr;

        if (!mountedKeys.current.has(key)) {
            embedMount(key, x, y, w, h).then((ok) => {
                if (ok) {
                    mountedKeys.current.add(key);
                    if (modalOpen) embedSetVisible(key, false).catch(() => {});
                }
            }).catch(() => {});
        } else {
            embedBounds(key, x, y, w, h).catch(() => {});
            embedSetVisible(key, !modalOpen).catch(() => {});
        }
    }, [modalOpen]);

    const register = useCallback((key, slotId, ref, active) => {
        let entry = registry.current.get(key);
        if (!entry) {
            entry = { refs: new Map() };
            registry.current.set(key, entry);
        }
        entry.refs.set(slotId, { ref, active });
        // Defer to next frame so the placeholder's bounding rect is real.
        requestAnimationFrame(() => reflowKey(key));
    }, [reflowKey]);

    const unregister = useCallback((key, slotId) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        entry.refs.delete(slotId);
        if (entry.refs.size === 0) {
            registry.current.delete(key);
            if (mountedKeys.current.has(key)) {
                embedUnmount(key).catch(() => {});
                mountedKeys.current.delete(key);
            }
        } else {
            reflowKey(key);
        }
    }, [reflowKey]);

    const updateActive = useCallback((key, slotId, active) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        const slot = entry.refs.get(slotId);
        if (!slot || slot.active === active) return;
        slot.active = active;
        reflowKey(key);
    }, [reflowKey]);

    // Reflow all keys on viewport changes
    useEffect(() => {
        const onResize = () => {
            for (const key of registry.current.keys()) reflowKey(key);
        };
        window.addEventListener('resize', onResize);
        return () => window.removeEventListener('resize', onResize);
    }, [reflowKey]);

    // Re-apply visibility when modalOpen toggles
    useEffect(() => {
        for (const key of mountedKeys.current) {
            embedSetVisible(key, !modalOpen).catch(() => {});
        }
    }, [modalOpen]);

    const ctx = useMemo(() => ({
        register, unregister, updateActive, reflowKey,
    }), [register, unregister, updateActive, reflowKey]);

    return (
        <EmbedLayerContext.Provider value={ctx}>
            {children}
        </EmbedLayerContext.Provider>
    );
}
```

- [ ] **Step 2: `npm run build`**

```bash
npm run build
```

Expected: build succeeds (no TypeScript / JSX syntax errors).

- [ ] **Step 3: Commit**

```bash
git add src/components/EmbedLayer.jsx
git commit -m "frontend: EmbedLayer registry + context (no callers yet)"
```

---

### Task 8.3: `EmbedSlot` component

**Files:**
- Create: `src/components/EmbedSlot.jsx`

- [ ] **Step 1: Write**

```jsx
import { useContext, useEffect, useRef } from 'react';
import { EmbedLayerContext } from './EmbedLayer.jsx';

let nextSlotId = 1;
function generateSlotId() {
    return `slot-${nextSlotId++}`;
}

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * Reserves a rectangle for a chat embed. Reports its rect + active state
 * to the global EmbedLayer; the layer dispatches embed_* IPC.
 *
 * Outside Tauri (browser dev): renders a placeholder hint.
 */
export default function EmbedSlot({ channelKey, isLive, active, placeholderText }) {
    const ref = useRef(null);
    const slotIdRef = useRef(null);
    const layer = useContext(EmbedLayerContext);

    useEffect(() => {
        if (!layer || !inTauri) return;
        if (!isLive || !channelKey) return;
        if (slotIdRef.current === null) slotIdRef.current = generateSlotId();
        layer.register(channelKey, slotIdRef.current, ref, active);
        return () => {
            if (slotIdRef.current !== null) {
                layer.unregister(channelKey, slotIdRef.current);
            }
        };
        // The ref is stable; channelKey + isLive + active are the deps.
    }, [channelKey, isLive, active, layer]);

    useEffect(() => {
        if (!layer || !inTauri) return;
        if (slotIdRef.current === null) return;
        layer.updateActive(channelKey, slotIdRef.current, active);
    }, [active, channelKey, layer]);

    // Resize observer chain
    useEffect(() => {
        if (!layer || !inTauri) return;
        const el = ref.current;
        if (!el) return;
        const ro = new ResizeObserver(() => layer.reflowKey(channelKey));
        ro.observe(el);
        for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
            ro.observe(p);
        }
        return () => ro.disconnect();
    }, [channelKey, layer]);

    return (
        <div
            ref={ref}
            style={{
                width: '100%',
                height: '100%',
                position: 'relative',
                overflow: 'hidden',
            }}
        >
            {!isLive ? (
                <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                    {placeholderText ?? 'Channel offline.'}
                </div>
            ) : !inTauri ? (
                <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                    Embedded chat is only available in the desktop app.
                </div>
            ) : null}
        </div>
    );
}
```

- [ ] **Step 2: Build**

```bash
npm run build
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/components/EmbedSlot.jsx
git commit -m "frontend: EmbedSlot — placeholder + register/unregister with EmbedLayer"
```

---

### Task 8.4: Mount `<EmbedLayer>` in App.jsx and wire modal state

**Files:**
- Modify: `src/App.jsx`

- [ ] **Step 1: Find the modal-state in App.jsx**

```bash
grep -n -E 'setVisibleAll|isModalOpen|showModal|modal' src/App.jsx | head -20
```

Note where modal-open state lives (or where today's `embedSetVisible` setter is called from).

- [ ] **Step 2: Wrap the existing tree in `<EmbedLayer>`**

Find the top-level `return` in `App.jsx`. The existing structure is roughly:

```jsx
return (
  <>
    <Titlebar ... />
    <Direction layout={layout} ... />
  </>
);
```

Wrap it:

```jsx
import EmbedLayer from './components/EmbedLayer.jsx';

return (
  <EmbedLayer modalOpen={isAnyModalOpen}>
    <Titlebar ... />
    <Direction layout={layout} ... />
  </EmbedLayer>
);
```

`isAnyModalOpen` is already tracked in App.jsx today (the value passed to the existing `setVisibleAll(false)` call). Use whichever boolean / setter is currently in use; there's no need to invent new state.

- [ ] **Step 3: Remove the old `embedSetVisibleAll` call**

Search for the old call site:

```bash
grep -n 'embedSetVisible\|setVisibleAll' src/
```

Delete every direct call from App.jsx — the new EmbedLayer handles it via the `modalOpen` prop.

- [ ] **Step 4: Build**

```bash
npm run build
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/App.jsx src/ipc.js
git commit -m "App: mount EmbedLayer; remove direct embedSetVisible calls"
```

---

### Task 8.5: Replace `<EmbeddedChat>` with `<EmbedSlot>` in ChatView

**Files:**
- Modify: `src/components/ChatView.jsx`

- [ ] **Step 1: Find the import + usage**

```bash
grep -n 'EmbeddedChat\|<EmbedSlot' src/components/ChatView.jsx
```

Expected: import on line 10, usage around line 58 (per the earlier grep).

- [ ] **Step 2: Update import + usage**

In `src/components/ChatView.jsx`:

```jsx
// Line 10:
import EmbedSlot from './EmbedSlot.jsx';

// Around line 58 — replace <EmbeddedChat ...>
<EmbedSlot
  channelKey={channelKey}
  isLive={isLive}
  active={true /* ChatView only renders for the active channel today */}
  placeholderText={...same as before}
/>
```

The `active={true}` literal is correct for today's app: ChatView is only rendered for one channel at a time per layout (Command/Focus = active selection, Columns = each visible column). When chat-tabs lands, ChatView gets an `active` prop that maps to "is this the focused tab"; the `active={true}` here becomes `active={isActiveTab}`.

- [ ] **Step 3: Verify no other callers of EmbeddedChat**

```bash
grep -rn 'EmbeddedChat\b' src/
```

Expected: only `EmbedLayer.jsx` (no — none) and the file `src/components/EmbeddedChat.jsx` itself. If anywhere else references it, update those callers similarly.

- [ ] **Step 4: Build**

```bash
npm run build
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "ChatView: use <EmbedSlot> instead of <EmbeddedChat>"
```

---

### Task 8.6: Delete the old `EmbeddedChat.jsx`

**Files:**
- Delete: `src/components/EmbeddedChat.jsx`

- [ ] **Step 1: Confirm no remaining imports**

```bash
grep -rn 'EmbeddedChat' src/
```

Expected: empty (or just the file itself).

- [ ] **Step 2: Delete + commit**

```bash
git rm src/components/EmbeddedChat.jsx
git commit -m "frontend: remove EmbeddedChat — superseded by EmbedSlot + EmbedLayer"
```

- [ ] **Step 3: Build sanity**

```bash
npm run build
```

Expected: clean.

---

## Phase 9 — End-to-end smoke

Goal: Verify every behavior the spec promises actually works on Linux. macOS / Windows verification deferred to PR review (no local environments).

### Task 9.1: Linux smoke matrix

**Files:** none

- [ ] **Step 1: Run dev**

```bash
npm run tauri:dev
```

- [ ] **Step 2: Single-embed sanity (YouTube)**

1. Add a known-live YouTube channel.
2. Switch to Command layout. Click the channel.
3. Confirm the chat renders inside the chat pane (dark themed).
4. Drag the main window across the screen — embed stays glued, no lag.
5. Resize the main window — embed reflows smoothly.

- [ ] **Step 3: Multi-embed sanity (the original motivating bug)**

1. Add 3 live YouTube channels (or 2 YT + 1 CB).
2. Switch to Columns layout.
3. **Confirm all three chats render simultaneously.** This is the test that fails on `main` today (only one renders).
4. Drag the window — all three reflow together.

- [ ] **Step 4: Cross-platform channel switch (no animation)**

1. In Command layout, select a YouTube channel — embed renders.
2. Click a Chaturbate channel.
3. Confirm: previous YT embed disappears, CB embed appears, **no WM open/close window animation**.

- [ ] **Step 5: Modal occlusion**

1. With an embed visible, open Preferences (or Add Channel).
2. Confirm embed hides instantly (GTK widget hide; should be imperceptible).
3. Close the modal.
4. Confirm embed reappears.

- [ ] **Step 6: Auth drift (Chaturbate)**

1. Sign in to Chaturbate via Preferences → Accounts.
2. Open a CB channel — embed loads, banner says nothing (signed in).
3. Manually clear the `sessionid` cookie via `gtk-inspector` (or wait for natural expiry).
4. Switch away then back to that channel — embed reloads, banner appears: "Signed out of Chaturbate — Sign in".

- [ ] **Step 7: Logout doesn't yank the rug**

1. Sign in to Chaturbate.
2. Open a CB channel embed.
3. While the embed is mounted and active, click "Log out" in Preferences.
4. Confirm: embed unmounts cleanly first; THEN profile dir is wiped. No WebKit error in the log.

- [ ] **Step 8: Capture any failures**

If any step fails, file a single short note in your shell scratchpad with: which step, what symptom, what log lines. Don't fix yet — accumulate. After the matrix, prioritize and fix in a final pass.

- [ ] **Step 9: Commit fixes if any**

For each fix:

```bash
git add <files>
git commit -m "fix: <symptom> — <fix summary>"
```

---

### Task 9.2: `cargo test` + `cargo clippy` final pass

**Files:** none

- [ ] **Step 1: Run all Rust tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: PASS — all existing tests + the new embed tests.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: zero warnings on the new code. If clippy complains about the existing `verify_chaturbate_auth` etc., review case-by-case.

- [ ] **Step 3: Format**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
```

If anything was reformatted:

```bash
git diff --stat
git add -u
git commit -m "fmt: cargo fmt"
```

---

## Phase 10 — Roadmap update + final review

### Task 10.1: Roadmap entry

**Files:**
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Read the file's current end**

```bash
tail -40 docs/ROADMAP.md
```

Identify the right phase — likely a new "Phase 6 — Workspace polish" section if it doesn't exist, or appended to an existing late-phase section.

- [ ] **Step 2: Add the entry**

Append (adjust phase header if Phase 6 already exists):

```markdown
## Phase 6 — Workspace polish

- [x] **Embed rewrite — child webviews instead of parked overlay window** —
  Replaces the borderless top-level `WebviewWindow` parked over the React
  chat pane with real child webviews painted into the main window's surface.
  Linux uses a `GtkOverlay` + `gtk::Fixed` sandwich and bypasses Tauri's
  broken `add_child` via `wry::WebViewBuilder::build_gtk`; macOS / Windows
  use Tauri's `add_child` directly. Multi-embed becomes a first-class
  capability (`HashMap<EmbedKey, ChildEmbed>`), unblocking the Columns
  layout and the upcoming chat-tabs work. Per-platform `data_directory`
  and existing auth flows preserved unchanged. Frontend gains a global
  `<EmbedLayer>` registry + `<EmbedSlot>` placeholder; the old
  `<EmbeddedChat>` chasing-the-main-window dance is removed. (PR #N)
```

(Substitute `#N` once the PR opens — the user's "ship it" workflow handles this in the post-merge docs PR.)

- [ ] **Step 3: Commit**

```bash
git add docs/ROADMAP.md
git commit -m "docs(roadmap): add embed rewrite under Phase 6"
```

---

### Task 10.2: Open PR

- [ ] **Step 1: Push the branch**

```bash
git push -u origin feat/embed-rewrite
```

(Or whatever branch name — `docs/spec-embed-rewrite` if you stayed on the spec branch and renamed it; `git branch -m feat/embed-rewrite` first if so.)

- [ ] **Step 2: Open the PR**

```bash
gh pr create --title "embed: child webviews instead of parked overlay window" --body "$(cat <<'EOF'
## Summary

- Replaces the borderless-WebviewWindow-parked-over-react-pane embed with real child webviews painted into the main window's surface (Linux: gtk::Fixed via wry build_gtk; macOS/Windows: Tauri add_child).
- Multi-embed by construction (HashMap<EmbedKey, ChildEmbed>) — Columns layout now actually shows N concurrent YT/CB chats.
- Per-platform profile dirs and existing auth flows preserved unchanged.
- Frontend gains a global EmbedLayer registry + EmbedSlot placeholder; EmbeddedChat deleted.

Spec: `docs/superpowers/specs/2026-04-28-embed-rewrite-design.md`
Plan: `docs/superpowers/plans/2026-04-28-embed-rewrite.md`

## Test plan

- [x] `cargo test` — all green
- [x] `cargo clippy` — zero warnings on new code
- [x] Linux smoke matrix per Phase 9 (single-embed, multi-embed, cross-platform switch, modal occlusion, auth drift, logout cleanup)
- [ ] macOS smoke (deferred — no local env)
- [ ] Windows smoke (deferred — no local env)
EOF
)"
```

- [ ] **Step 3: Wait for review.** The user's "ship it" workflow handles merge + roadmap finalization.

---

## Self-review notes

Spec coverage:
- §1 goal/non-goals — Phase 1-9 cover the goal; non-goals (chat-tabs, login iframes, native Wayland) deliberately not implemented.
- §2.a module structure — Phase 1, 8 (frontend split).
- §2.b GTK topology — Phase 2.
- §2.c EmbedHost/ChildEmbed — Phase 1, 3, 4, 5.
- §2.d IPC surface — Phase 7.
- §2.e frontend — Phase 8.
- §2.f lifecycle — Phase 5 + 8 (covered by behavior).
- §2.g per-platform branches — Phase 3, 4.
- §2.h auth integration unchanged — Phase 6.3 + 9.1.7 (the no-rug-pull test).
- §2.i first-paint — Phase 6.2.
- §2.j cross-platform parity — Phase 4 + 9 (mac/Win deferred to PR review).
- §3 migration — entire plan.
- §4 risks — covered by Phase 2.4 manual smoke (GtkOverlay layering check) and Phase 4 cross-compile sanity.
- §5 phasing — mapped to Phases 1-9.

Placeholder scan: the plan has explicit code in every step. The one place that hand-waves is the wry `cookies_for_url` fallback in Task 3.5 / 6.3 (real fallback path provided, with both the wry-direct and webkit2gtk-CookieManager versions written out). No "TODO", "fill in", "similar to Task N" anywhere.

Type consistency:
- `Rect::new(x, y, w, h)` — used uniformly.
- `EmbedKey = String` — used in HashMap, IPC handlers, all React layer calls.
- `embed_bounds` (renamed from embed_position) — IPC name and `embedBounds` JS wrapper both consistent.
- `embedSetVisible(key, bool)` — signature change from old (no key) consistently rolled out: lib.rs handler, ipc.js wrapper, EmbedLayer caller.
- `BuildSpec` — both Linux and non-Linux variants take `platform`, `app`, and `unique_key` after Task 6.3.

Done.
