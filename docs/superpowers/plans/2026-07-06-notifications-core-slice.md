# Notifications Core Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-channel notification mute UI, notification sound (rodio, bundled default + custom file), per-platform filter, quiet hours, and a test button — per the approved spec `docs/superpowers/specs/2026-07-06-notifications-core-slice-design.md`.

**Architecture:** `notify.rs` becomes a `notify/` module: the existing `NotifyTracker` (untouched edge logic) consults a new pure `gate::should_notify` for every suppression decision, then `sound::play` after the visual notification succeeds. New `NotificationSettings` on `Settings` absorbs `general.notify_on_live`. Two new IPC commands. Frontend: context-menu mute + bell-slash glyph + a new Preferences → Notifications tab.

**Tech Stack:** Rust (Tauri 2, chrono, parking_lot, rodio), React 18, `@tauri-apps/plugin-dialog` (new) for the sound file picker.

## Global Constraints

- Never commit to `main`; PR 1 on branch `feat/notifications-backend`, PR 2 on `feat/notifications-frontend` (stacked on PR 1's branch until it merges, or created after PR 1 merges).
- Commit messages must NEVER reference AI, Claude, or automated generation.
- `cargo check` must stay at **0 warnings**; CI's fmt + clippy (`--all-targets -D warnings`) gates are **blocking** — run `cargo fmt --manifest-path src-tauri/Cargo.toml` before every commit (if the local `cargo fmt` shim errors, use `/usr/bin/rustfmt --edition 2021 <changed .rs files>` directly).
- Never use native `title=""` attributes — wrap in `<Tooltip text="…">` from `src/components/Tooltip.jsx` with a matching `aria-label`.
- Any element with explicit width + padding needs `box-sizing: border-box` (no global reset in this codebase).
- New `#[tauri::command]`s MUST be added to BOTH `register_handlers!` in `src-tauri/src/lib.rs` AND `list_handlers()` in `src-tauri/src/smoke_harness/smoke.rs` — the test `list_count_matches_register_handlers_macro_body` (runs under `cargo test --features smoke`) enforces the count.
- Full verification before each PR: `cargo test`, `cargo test --features smoke`, `cargo clippy --all-targets -- -D warnings`, `npm run build` — all from repo root with `--manifest-path src-tauri/Cargo.toml` on the cargo commands.

---

# PR 1 — Backend (branch `feat/notifications-backend`)

### Task 1: `NotificationSettings` struct + `notify_on_live` migration

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Test: same file, `mod tests`

**Interfaces:**
- Produces: `Settings.notifications: NotificationSettings` with fields `enabled: bool`, `sound_enabled: bool`, `custom_sound_path: String`, `platform_filter: PlatformFilter { twitch, youtube, kick, chaturbate: bool }`, `quiet_hours_enabled: bool`, `quiet_start: String`, `quiet_end: String`. Also `Settings::load()` migration: when the JSON has no `notifications` key but has `general.notify_on_live: false`, the loaded `notifications.enabled` must be `false`.
- Consumes: existing `Settings` struct (`settings.rs:15`), existing test patterns (`settings.rs` `mod tests`).

- [ ] **Step 1: Write the failing tests** (append to the existing `mod tests` in `settings.rs`)

```rust
#[test]
fn notification_settings_defaults_when_missing() {
    let s: Settings = serde_json::from_str("{}").unwrap();
    let n = &s.notifications;
    assert!(n.enabled);
    assert!(n.sound_enabled);
    assert_eq!(n.custom_sound_path, "");
    assert!(n.platform_filter.twitch);
    assert!(n.platform_filter.youtube);
    assert!(n.platform_filter.kick);
    assert!(n.platform_filter.chaturbate);
    assert!(!n.quiet_hours_enabled);
    assert_eq!(n.quiet_start, "23:00");
    assert_eq!(n.quiet_end, "08:00");
}

#[test]
fn notification_settings_round_trip() {
    let mut s = Settings::default();
    s.notifications.enabled = false;
    s.notifications.custom_sound_path = "/tmp/ding.ogg".into();
    s.notifications.platform_filter.kick = false;
    s.notifications.quiet_hours_enabled = true;
    let json = serde_json::to_string(&s).unwrap();
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert!(!back.notifications.enabled);
    assert_eq!(back.notifications.custom_sound_path, "/tmp/ding.ogg");
    assert!(!back.notifications.platform_filter.kick);
    assert!(back.notifications.quiet_hours_enabled);
}

/// Old configs carry `general.notify_on_live`; a missing `notifications`
/// block must seed `enabled` from it exactly once at load.
#[test]
fn migrates_notify_on_live_false_into_enabled() {
    let json = r#"{"general":{"notify_on_live":false}}"#;
    let s = Settings::from_json_with_migrations(json).unwrap();
    assert!(!s.notifications.enabled);
}

/// If the `notifications` block IS present, it wins over the legacy field.
#[test]
fn present_notifications_block_beats_legacy_field() {
    let json =
        r#"{"general":{"notify_on_live":false},"notifications":{"enabled":true}}"#;
    let s = Settings::from_json_with_migrations(json).unwrap();
    assert!(s.notifications.enabled);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings:: 2>&1 | tail -5`
Expected: compile error — `notifications` field and `from_json_with_migrations` don't exist.

- [ ] **Step 3: Implement**

In `settings.rs`, add to `Settings` (after `pub chat`):

```rust
    #[serde(default)]
    pub notifications: NotificationSettings,
```

Add the structs (follow the file's existing `Default`-impl style):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformFilter {
    #[serde(default = "default_true")]
    pub twitch: bool,
    #[serde(default = "default_true")]
    pub youtube: bool,
    #[serde(default = "default_true")]
    pub kick: bool,
    #[serde(default = "default_true")]
    pub chaturbate: bool,
}

impl Default for PlatformFilter {
    fn default() -> Self {
        Self { twitch: true, youtube: true, kick: true, chaturbate: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    #[serde(default)]
    pub custom_sound_path: String,
    #[serde(default)]
    pub platform_filter: PlatformFilter,
    #[serde(default)]
    pub quiet_hours_enabled: bool,
    #[serde(default = "default_quiet_start")]
    pub quiet_start: String,
    #[serde(default = "default_quiet_end")]
    pub quiet_end: String,
}

fn default_quiet_start() -> String { "23:00".into() }
fn default_quiet_end() -> String { "08:00".into() }

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sound_enabled: true,
            custom_sound_path: String::new(),
            platform_filter: PlatformFilter::default(),
            quiet_hours_enabled: false,
            quiet_start: default_quiet_start(),
            quiet_end: default_quiet_end(),
        }
    }
}
```

(If the file doesn't already have a `fn default_true() -> bool { true }` helper, add one; grep first — `grep -n "fn default_true" src-tauri/src/settings.rs`.)

Migration: add a parse entry point and route `Settings::load()` through it. Find where `load()` currently does `serde_json::from_slice`/`from_str` and refactor to:

```rust
/// Parse settings JSON applying one-time migrations. Public for tests.
pub fn from_json_with_migrations(json: &str) -> Result<Settings, serde_json::Error> {
    let raw: serde_json::Value = serde_json::from_str(json)?;
    let has_notifications_block = raw.get("notifications").is_some();
    let legacy_notify_on_live = raw
        .get("general")
        .and_then(|g| g.get("notify_on_live"))
        .and_then(|v| v.as_bool());
    let mut s: Settings = serde_json::from_value(raw)?;
    // Migration: absorb general.notify_on_live into notifications.enabled
    // when the new block is absent. The legacy field stays tolerated on
    // GeneralSettings so old JSON parses; it is no longer written as the
    // source of truth.
    if !has_notifications_block {
        if let Some(legacy) = legacy_notify_on_live {
            s.notifications.enabled = legacy;
        }
    }
    Ok(s)
}
```

Note: `from_json_with_migrations` is a free function in the `settings` module (call as `settings::from_json_with_migrations` or adjust the test to `use super::*` — the tests above assume `Settings::from_json_with_migrations`; make it an associated fn `impl Settings { pub fn from_json_with_migrations(...) }` to match). Keep `general.notify_on_live` field in `GeneralSettings` (serde-tolerated) — do NOT delete it; the Preferences General tab row is removed in PR 2.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings:: 2>&1 | tail -3`
Expected: all settings tests PASS (existing + 4 new).

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/settings.rs
git commit -m "feat(notify): NotificationSettings struct + notify_on_live migration"
```

---

### Task 2: `notify/` module split + pure gate

**Files:**
- Create: `src-tauri/src/notify/gate.rs`
- Move: `src-tauri/src/notify.rs` → `src-tauri/src/notify/mod.rs` (`git mv`)
- Test: `src-tauri/src/notify/gate.rs` `mod tests`

**Interfaces:**
- Consumes: `crate::settings::NotificationSettings` (Task 1), `crate::platforms::Platform`.
- Produces: `notify::gate::{should_notify, DenyReason}`:
  ```rust
  pub enum DenyReason { Disabled, ChannelMuted, PlatformFiltered, QuietHours }
  pub fn should_notify(
      s: &NotificationSettings,
      platform: Platform,
      dont_notify: bool,
      local_now: chrono::NaiveTime,
  ) -> Result<(), DenyReason>
  ```

- [ ] **Step 1: Move the module**

```bash
mkdir -p src-tauri/src/notify
git mv src-tauri/src/notify.rs src-tauri/src/notify/mod.rs
```

Add at the top of the new `mod.rs` (below the module doc comment): `pub mod gate;`
Run `cargo check --manifest-path src-tauri/Cargo.toml` — expected: fails only on missing `gate.rs` (create an empty file to confirm the move itself is clean, then continue).

- [ ] **Step 2: Write the failing tests** in `src-tauri/src/notify/gate.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::Platform;
    use crate::settings::NotificationSettings;
    use chrono::NaiveTime;

    fn t(hm: &str) -> NaiveTime {
        NaiveTime::parse_from_str(hm, "%H:%M").unwrap()
    }
    fn base() -> NotificationSettings {
        NotificationSettings::default()
    }

    #[test]
    fn allows_by_default() {
        assert!(should_notify(&base(), Platform::Twitch, false, t("12:00")).is_ok());
    }

    #[test]
    fn denies_when_disabled() {
        let mut s = base();
        s.enabled = false;
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("12:00")),
            Err(DenyReason::Disabled)
        ));
    }

    #[test]
    fn channel_mute_beats_platform_filter() {
        let mut s = base();
        s.platform_filter.twitch = false; // both suppressors active
        assert!(matches!(
            should_notify(&s, Platform::Twitch, true, t("12:00")),
            Err(DenyReason::ChannelMuted)
        ));
    }

    #[test]
    fn denies_filtered_platform() {
        let mut s = base();
        s.platform_filter.kick = false;
        assert!(matches!(
            should_notify(&s, Platform::Kick, false, t("12:00")),
            Err(DenyReason::PlatformFiltered)
        ));
        // other platforms unaffected
        assert!(should_notify(&s, Platform::Twitch, false, t("12:00")).is_ok());
    }

    #[test]
    fn quiet_hours_non_wrapping_window() {
        let mut s = base();
        s.quiet_hours_enabled = true;
        s.quiet_start = "09:00".into();
        s.quiet_end = "17:00".into();
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("12:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(should_notify(&s, Platform::Twitch, false, t("08:59")).is_ok());
        // boundary: start inclusive, end exclusive
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("09:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(should_notify(&s, Platform::Twitch, false, t("17:00")).is_ok());
    }

    #[test]
    fn quiet_hours_wraps_midnight() {
        let mut s = base();
        s.quiet_hours_enabled = true; // defaults 23:00 -> 08:00
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("23:30")),
            Err(DenyReason::QuietHours)
        ));
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("03:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("23:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(should_notify(&s, Platform::Twitch, false, t("08:00")).is_ok());
        assert!(should_notify(&s, Platform::Twitch, false, t("12:00")).is_ok());
    }

    #[test]
    fn quiet_hours_disabled_never_suppresses() {
        let s = base(); // quiet_hours_enabled = false
        assert!(should_notify(&s, Platform::Twitch, false, t("23:30")).is_ok());
    }

    /// Malformed HH:MM must FAIL OPEN — never silently eat notifications.
    #[test]
    fn malformed_quiet_times_fail_open() {
        let mut s = base();
        s.quiet_hours_enabled = true;
        s.quiet_start = "25:99".into();
        assert!(should_notify(&s, Platform::Twitch, false, t("23:30")).is_ok());
        s.quiet_start = "not a time".into();
        assert!(should_notify(&s, Platform::Twitch, false, t("23:30")).is_ok());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml notify::gate 2>&1 | tail -5`
Expected: compile error — `should_notify` / `DenyReason` undefined.

- [ ] **Step 4: Implement** (top of `gate.rs`):

```rust
//! Pure notification-suppression decisions. ALL reasons a go-live event may
//! be silenced live here so they're unit-testable without a Tauri app handle.

use chrono::NaiveTime;

use crate::platforms::Platform;
use crate::settings::NotificationSettings;

#[derive(Debug, PartialEq, Eq)]
pub enum DenyReason {
    Disabled,
    ChannelMuted,
    PlatformFiltered,
    QuietHours,
}

/// Evaluation order: Disabled → ChannelMuted → PlatformFiltered → QuietHours.
pub fn should_notify(
    s: &NotificationSettings,
    platform: Platform,
    dont_notify: bool,
    local_now: NaiveTime,
) -> Result<(), DenyReason> {
    if !s.enabled {
        return Err(DenyReason::Disabled);
    }
    if dont_notify {
        return Err(DenyReason::ChannelMuted);
    }
    let platform_on = match platform {
        Platform::Twitch => s.platform_filter.twitch,
        Platform::YouTube => s.platform_filter.youtube,
        Platform::Kick => s.platform_filter.kick,
        Platform::Chaturbate => s.platform_filter.chaturbate,
    };
    if !platform_on {
        return Err(DenyReason::PlatformFiltered);
    }
    if s.quiet_hours_enabled && in_quiet_window(&s.quiet_start, &s.quiet_end, local_now) {
        return Err(DenyReason::QuietHours);
    }
    Ok(())
}

/// `[start, end)`; `start > end` wraps midnight. Malformed times fail open.
fn in_quiet_window(start: &str, end: &str, now: NaiveTime) -> bool {
    let (Ok(start), Ok(end)) = (
        NaiveTime::parse_from_str(start, "%H:%M"),
        NaiveTime::parse_from_str(end, "%H:%M"),
    ) else {
        log::warn!("quiet hours: malformed time (start={start:?} end={end:?}); not suppressing");
        return false;
    };
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}
```

(Check the exact `Platform` variant names with `grep -n "pub enum Platform" -A6 src-tauri/src/platforms/mod.rs` and match them — the codebase may use `Youtube` instead of `YouTube`.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml notify:: 2>&1 | tail -3`
Expected: gate tests + the 6 pre-existing `is_go_live` tests all PASS. Also `cargo check` → 0 warnings.

- [ ] **Step 6: fmt + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/notify src-tauri/src/notify.rs 2>/dev/null; git add -A src-tauri/src
git commit -m "feat(notify): split notify module; pure suppression gate with tests"
```

---

### Task 3: sound playback (`notify/sound.rs`) + bundled default

**Files:**
- Create: `src-tauri/src/notify/sound.rs`, `src-tauri/sounds/notify.ogg`, `src-tauri/sounds/README.md`
- Modify: `src-tauri/Cargo.toml` (rodio dep), `src-tauri/src/notify/mod.rs` (`pub mod sound;`)

**Interfaces:**
- Consumes: `NotificationSettings` (Task 1).
- Produces: `notify::sound::play(settings: &NotificationSettings)` — non-blocking, never panics, no return value. Also `notify::sound::play_path_or_default(custom_path: &str)` used by `play` and directly by the Preferences ▶ preview later (`notify_test` covers preview in this slice — see Task 5).

- [ ] **Step 1: Add the dependency**

In `src-tauri/Cargo.toml` under `[dependencies]`:

```toml
rodio = { version = "0.20", default-features = false, features = ["symphonia-vorbis", "symphonia-flac", "symphonia-mp3", "symphonia-wav"] }
```

Run `cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -3`. If that feature list doesn't resolve (rodio feature names shift between versions), fall back to `rodio = "0.20"` (default features) and note it in the commit body. Expected: compiles.

- [ ] **Step 2: Generate the bundled default sound** (CC0, generated — no licensing risk)

```bash
mkdir -p src-tauri/sounds
ffmpeg -f lavfi -i "sine=frequency=880:duration=0.09" -f lavfi -i "sine=frequency=1320:duration=0.18" \
  -filter_complex "[0][1]concat=n=2:v=0:a=1,afade=t=in:d=0.01,afade=t=out:st=0.12:d=0.15,volume=0.4" \
  -c:a libvorbis -q:a 3 src-tauri/sounds/notify.ogg
```

(If `ffmpeg` is missing: `sox -n src-tauri/sounds/notify.ogg synth 0.09 sine 880 : synth 0.18 sine 1320 fade 0.01 0.27 0.15 vol 0.4`.) Listen to it once (`mpv src-tauri/sounds/notify.ogg`) — it should be a soft, short two-tone chime, not a harsh beep. Create `src-tauri/sounds/README.md`:

```markdown
# sounds/

`notify.ogg` — default go-live notification sound. Generated with ffmpeg
(two-tone sine chime, see plan doc); public domain / CC0. Replaced at
runtime when `settings.notifications.custom_sound_path` is set.
```

- [ ] **Step 3: Implement `sound.rs`** (no unit tests — audio device I/O; the pure gate carries the logic coverage. Compile-only + manual verification):

```rust
//! Notification sound playback via rodio. Fire-and-forget: failures are
//! logged and never block the visual notification.

use crate::settings::NotificationSettings;

static DEFAULT_SOUND: &[u8] = include_bytes!("../../sounds/notify.ogg");

/// Honors `sound_enabled`; resolves custom file vs bundled default.
pub fn play(settings: &NotificationSettings) {
    if !settings.sound_enabled {
        return;
    }
    play_path_or_default(&settings.custom_sound_path);
}

/// Play `custom_path` if non-empty and readable, else the bundled default.
/// Detached thread: rodio's OutputStream must outlive playback and must not
/// block the caller (refresh loop / IPC).
pub fn play_path_or_default(custom_path: &str) {
    let custom = custom_path.trim().to_string();
    std::thread::Builder::new()
        .name("notify-sound".into())
        .spawn(move || {
            let bytes: std::borrow::Cow<'static, [u8]> = if custom.is_empty() {
                std::borrow::Cow::Borrowed(DEFAULT_SOUND)
            } else {
                match std::fs::read(&custom) {
                    Ok(b) => std::borrow::Cow::Owned(b),
                    Err(e) => {
                        log::warn!("custom notify sound {custom:?} unreadable ({e}); using default");
                        std::borrow::Cow::Borrowed(DEFAULT_SOUND)
                    }
                }
            };
            if let Err(e) = play_bytes(&bytes) {
                log::warn!("notification sound playback failed: {e:#}");
            }
        })
        .ok();
}

fn play_bytes(bytes: &[u8]) -> anyhow::Result<()> {
    use rodio::{Decoder, OutputStream, Sink};
    let (_stream, handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&handle)?;
    let cursor = std::io::Cursor::new(bytes.to_vec());
    sink.append(Decoder::new(cursor)?);
    sink.sleep_until_end();
    Ok(())
}
```

(rodio 0.20 API check: if `OutputStream::try_default` / `Sink::try_new` moved (0.21 renamed some of these to `OutputStreamBuilder`/`Sink::connect_new`), adapt to the version that resolved in Step 1 — the shape stays: open default output, decode the byte cursor, block the detached thread until the clip ends.)

Add `pub mod sound;` to `notify/mod.rs`.

- [ ] **Step 4: Verify compile + manual play**

Run: `cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -2` → 0 warnings.
Manual: temporarily add a `sound::play_path_or_default("")` call in `lib.rs::setup`, `npm run tauri:dev`, confirm the chime plays at launch, REMOVE the temp call.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/sounds src-tauri/src/notify
git commit -m "feat(notify): rodio sound playback with bundled default chime"
```

---

### Task 4: wire gate + sound into the go-live path

**Files:**
- Modify: `src-tauri/src/notify/mod.rs` (`detect_and_notify`, `send_go_live`)
- Modify: `src-tauri/src/lib.rs` + `src-tauri/src/refresh.rs` — wherever `detect_and_notify` is invoked (find with `grep -rn "detect_and_notify\|notify_on_live" src-tauri/src/lib.rs src-tauri/src/refresh.rs`)

**Interfaces:**
- Consumes: `gate::should_notify` (Task 2), `sound::play` (Task 3), `Settings.notifications` (Task 1).
- Produces: `NotifyTracker::detect_and_notify` gains a `settings: &NotificationSettings` parameter; the old caller-side `notify_on_live` gating is REPLACED by the gate (callers stop reading `general.notify_on_live` — the seed-instead-of-notify branch keys off `settings.notifications.enabled` now).

- [ ] **Step 1: Update `detect_and_notify`** — new signature and body changes:

```rust
    pub fn detect_and_notify<R: Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        channels: &[Channel],
        snapshot: &[Livestream],
        settings: &crate::settings::NotificationSettings,
    ) {
```

Replace the per-channel `dont_notify` short-circuit with the gate (the gate now owns that check):

```rust
        let now = chrono::Local::now().time();
        for ls in snapshot {
            let was = prev.insert(ls.unique_key.clone(), ls.is_live);
            if is_go_live(was, ls.is_live) {
                let ch = cfg_map.get(ls.channel_id.as_str()).copied();
                let dont_notify = ch.map(|c| c.dont_notify).unwrap_or(false);
                match crate::notify::gate::should_notify(settings, ls.platform, dont_notify, now)
                {
                    Ok(()) => send_go_live(app, ls, settings),
                    Err(reason) => {
                        log::debug!("go-live for {} suppressed: {reason:?}", ls.unique_key)
                    }
                }
            }
        }
```

(`Livestream` must expose its platform — check with `grep -n "platform" src-tauri/src/channels.rs | head`; it does (`ls.platform` is used in the existing `send_go_live` body). Note the gate's `Disabled` arm makes the whole loop a no-op notification-wise but the tracker still advances `prev` — which is exactly the current `seed()` semantics; keep the callers' existing enabled/disabled branching ONLY if removing it would break the seed flow — read the call sites first. Preferred end-state: callers always call `detect_and_notify` and the gate decides; delete the separate `seed()` call path if it becomes dead, or keep `seed()` for the disabled case if the call sites rely on it — document whichever you do in the commit body.)

- [ ] **Step 2: `send_go_live` plays sound after showing:**

```rust
fn send_go_live<R: Runtime>(
    app: &tauri::AppHandle<R>,
    ls: &Livestream,
    settings: &crate::settings::NotificationSettings,
) {
    // …existing title/body construction unchanged…
    match app.notification().builder().title(title).body(body).show() {
        Ok(()) => crate::notify::sound::play(settings),
        Err(e) => log::warn!("notification failed for {}: {e:#}", ls.unique_key),
    }
}
```

- [ ] **Step 3: Update call sites** in `lib.rs`/`refresh.rs` to pass `&settings.notifications` (they already read `Settings` for `notify_on_live` — swap that read).

- [ ] **Step 4: Run everything**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -2`
Expected: all green (existing notify tests unchanged). `cargo check` → 0 warnings.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add -A src-tauri/src
git commit -m "feat(notify): route go-live notifications through the gate + sound"
```

---

### Task 5: IPC — `set_channel_notify` + `notify_test`

**Files:**
- Modify: `src-tauri/src/channels.rs` (new `set_dont_notify` method + test), `src-tauri/src/lib.rs` (2 commands + registration), `src-tauri/src/smoke_harness/smoke.rs` (`list_handlers` + 2 entries)

**Interfaces:**
- Consumes: `channels::persist` (existing off-lock persistence), `sound::play`, notification builder.
- Produces:
  - `ChannelStore::set_dont_notify(&mut self, unique_key: &str, mute: bool) -> bool` (pure in-memory, mirrors `set_favorite` at `channels.rs:319`)
  - `#[tauri::command] fn set_channel_notify(unique_key: String, mute: bool, state) -> Result<bool, String>`
  - `#[tauri::command] fn notify_test(app, state) -> Result<(), String>` — fires "Test notification" through the notification builder + `sound::play`; honors `enabled`/`sound_enabled` but BYPASSES quiet hours + platform filter + channel mute (spec: the button must always demonstrate something when notifications are on; returns `Err("notifications are disabled")` when `enabled` is false so the UI can hint)

- [ ] **Step 1: Failing test for the store method** (append to `channels.rs` `mod tests`, following the `set_favorite` test pattern there):

```rust
#[test]
fn set_dont_notify_toggles_and_reports_touched() {
    let mut store = /* construct exactly like the neighboring set_favorite test does */;
    // copy the test-channel setup from the set_favorite test in this module
    assert!(store.set_dont_notify("twitch:somechan", true));
    assert!(store.channels().iter().any(|c| c.dont_notify));
    assert!(store.set_dont_notify("twitch:somechan", false));
    assert!(!store.set_dont_notify("twitch:nosuch", true));
}
```

(Open the existing `set_favorite`/`add_many` tests in `channels.rs` `mod tests` and copy their store-construction helper verbatim — do not invent a new fixture.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels:: 2>&1 | tail -3`
Expected: compile error, `set_dont_notify` undefined.

- [ ] **Step 3: Implement store method** (next to `set_favorite`, same doc-comment style):

```rust
    /// Toggle a channel's notification mute. Pure in-memory — caller persists
    /// after unlocking. Returns whether the channel was found and set.
    pub fn set_dont_notify(&mut self, unique_key: &str, mute: bool) -> bool {
        for c in &mut self.channels {
            if c.unique_key() == unique_key {
                c.dont_notify = mute;
                return true;
            }
        }
        false
    }
```

- [ ] **Step 4: Commands in `lib.rs`** (model on `set_favorite`, `lib.rs:156`):

```rust
#[tauri::command]
fn set_channel_notify(
    unique_key: String,
    mute: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let unique_key = channels::channel_key_of(&unique_key).to_string();
    let touched = state.store.lock().set_dont_notify(&unique_key, mute);
    if touched {
        channels::persist(&state.store).map_err(err_string)?;
    }
    Ok(touched)
}

#[tauri::command]
fn notify_test<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let n = state.settings.read().notifications.clone();
    if !n.enabled {
        return Err("notifications are disabled".into());
    }
    app.notification()
        .builder()
        .title("Test notification")
        .body("This is what a go-live notification looks like.")
        .show()
        .map_err(err_string)?;
    notify::sound::play(&n);
    Ok(())
}
```

(Match how other commands read settings — `grep -n "state.settings.read()" src-tauri/src/lib.rs | head -3` — and whether commands here take `AppHandle<R>` generically or concrete `tauri::AppHandle` (check a neighbor like the tray or notification-using commands; use the same form). Register BOTH in `register_handlers!` AND add `"set_channel_notify"`, `"notify_test"` to `list_handlers()` in `src-tauri/src/smoke_harness/smoke.rs` — alphabetically consistent with how that list is grouped.)

- [ ] **Step 5: Run the full gate**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -2
cargo test --manifest-path src-tauri/Cargo.toml --features smoke 2>&1 | grep -E "test result|FAILED" | head -3
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -2
```
Expected: all green; the smoke count test passes (both lists grew by 2).

- [ ] **Step 6: Smoke-harness live check**

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- set_channel_notify '{"uniqueKey":"twitch:example","mute":true}'
```
Expected: `Ok(false)` shape (isolated config has no such channel) — proves marshalling.

- [ ] **Step 7: fmt + commit + PR**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add -A src-tauri
git commit -m "feat(notify): set_channel_notify + notify_test IPC"
npm run build   # confirm frontend untouched-but-green
git push -u origin feat/notifications-backend
gh pr create --title "feat(notify): notification gate, sound, mute + test IPC (backend)" --body "<summary per repo convention: Summary / tradeoffs / Test plan; reference the spec path>"
```
STOP — orchestrator reviews and merges before PR 2 starts (PR 2 branches off updated main).

---

# PR 2 — Frontend (branch `feat/notifications-frontend`, off main AFTER PR 1 merges)

### Task 6: IPC wrappers + mocks + dialog plugin

**Files:**
- Modify: `src/ipc.js`, `package.json`, `src-tauri/Cargo.toml`, `src-tauri/src/lib.rs` (plugin registration), `src-tauri/capabilities/default.json`

**Interfaces:**
- Produces: `setChannelNotify(uniqueKey, mute) -> Promise<bool>`, `notifyTest() -> Promise<void>` exported from `src/ipc.js`; `@tauri-apps/plugin-dialog`'s `open()` importable in components.
- Consumes: PR 1's commands.

- [ ] **Step 1: Wrappers in `ipc.js`** (copy the style of the existing thin wrappers at the top of the file):

```js
export const setChannelNotify = (uniqueKey, mute) =>
  invoke('set_channel_notify', { uniqueKey, mute });
export const notifyTest = () => invoke('notify_test');
```

- [ ] **Step 2: Mocks** — in `mockInvoke`'s switch add:

```js
    case 'set_channel_notify': {
      const ch = mockChannels.find(c => `${c.platform}:${c.channel_id}` === args.uniqueKey);
      if (ch) ch.dont_notify = args.mute;
      return !!ch;
    }
    case 'notify_test':
      console.log('[mock] notify_test — would fire a desktop notification + sound');
      return null;
```

(Match the file's actual mock-channel array name and key derivation — read the neighboring `set_favorite` mock case and mirror it exactly.)

- [ ] **Step 3: Dialog plugin** (sound-file Browse…):

```bash
npm install @tauri-apps/plugin-dialog
cargo add tauri-plugin-dialog --manifest-path src-tauri/Cargo.toml
```

In `lib.rs` where other `.plugin(...)` calls chain on the Builder (grep `plugin(tauri_plugin_notification`), add `.plugin(tauri_plugin_dialog::init())`. In `src-tauri/capabilities/default.json`, add `"dialog:allow-open"` to the permissions array (read the file; follow its existing permission-string format).

- [ ] **Step 4: Verify**

Run: `npm run build && cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -2`
Expected: both green, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add src/ipc.js package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/capabilities/default.json
git commit -m "feat(notify): frontend IPC wrappers, mocks, dialog plugin"
```

---

### Task 7: context-menu mute + bell-slash row glyph

**Files:**
- Modify: `src/directions/Command.jsx` (context menu ~line 500 + row meta cluster), `src/App.jsx` (thread `setChannelNotify` through `ctx` if channel actions flow that way — read how `setFavorite` reaches the menu first and mirror it)

**Interfaces:**
- Consumes: `setChannelNotify` (Task 6), channel objects carrying `dont_notify` (already in `list_channels` payloads).
- Produces: right-click menu item + glyph; optimistic UI refresh via the same path `setFavorite` uses (find it: `grep -n "setFavorite" src/App.jsx src/directions/Command.jsx`).

- [ ] **Step 1: Menu item** — in Command.jsx's channel-row `ContextMenu`, after the Favorite item (~line 509), add:

```jsx
<ContextMenu.Item
  onClick={() => {
    onToggleNotify?.(channel.unique_key, !channel.dont_notify);
  }}
>
  {channel.dont_notify ? 'Unmute notifications' : 'Mute notifications'}
</ContextMenu.Item>
```

Thread `onToggleNotify` from wherever `onFavorite` originates (same prop chain; in App.jsx's ctx actions add `toggleNotify: (key, mute) => setChannelNotify(key, mute).then(refreshChannels).catch(console.error)` — match the exact refresh-after-mutation call the favorite path uses; grep it, don't guess).

- [ ] **Step 2: Bell-slash glyph** — in the row meta cluster (`.cmd-row-meta` area), render when `channel.dont_notify`:

```jsx
{channel.dont_notify && (
  <Tooltip text="Notifications muted">
    <span aria-label="Notifications muted" style={{ color: 'var(--zinc-600)', fontSize: 10 }}>
      🔕
    </span>
  </Tooltip>
)}
```

Use an inline SVG bell-slash (12px, `stroke: currentColor`) instead of the emoji if the row already uses SVG glyphs — inspect the neighboring viewers/favorite glyphs and match the medium. Keep it inside `.cmd-row-meta` so collapsed mode hides it for free.

- [ ] **Step 3: Verify in dev**

Run `npm run dev`, right-click a mock channel → Mute notifications → glyph appears, menu label flips. `npm run build` green.

- [ ] **Step 4: Commit**

```bash
git add src/App.jsx src/directions/Command.jsx
git commit -m "feat(notify): per-channel mute via context menu + row glyph"
```

---

### Task 8: Preferences → Notifications tab

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` (`TABS` const line 19, tab render ~line 210, new `NotificationsTab` component; REMOVE the notify row from `GeneralTab`)

**Interfaces:**
- Consumes: `usePreferences().patch` (existing settings patch flow — read `GeneralTab` for the exact `patch((prev) => ({...prev, general: {...}}))` shape and mirror with `notifications`), `notifyTest`, `setChannelNotify`, `listChannels` (existing ipc export — verify name with grep), `open` from `@tauri-apps/plugin-dialog`, existing `Row`/`Toggle` components in the file.
- Produces: the tab per spec §Frontend.

- [ ] **Step 1: Tab registration**

```js
const TABS = [
  { id: 'general', label: 'General' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'chat', label: 'Chat' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'accounts', label: 'Accounts' },
];
```

Add `{settings && tab === 'notifications' && <NotificationsTab settings={settings} patch={patch} />}` beside the other tab renders (~line 210).

- [ ] **Step 2: Build `NotificationsTab`** in the same file, using the file's existing `Row` + `Toggle` primitives (do not invent new ones). Structure — a `n = settings?.notifications || {}` accessor and a `patchN = (fields) => patch((prev) => ({ ...prev, notifications: { ...prev.notifications, ...fields } }))` helper, then:

1. Row "Enable notifications" → `patchN({ enabled: v })`. Everything below uses `disabled={!n.enabled}` + the spellcheck section's chained-disable styling (grep `Requires spellcheck` in this file to copy the exact disabled pattern).
2. Row "Play sound" → toggle `sound_enabled`; below it a sub-row showing `n.custom_sound_path || 'Default chime'` +
   - Browse… button: `const { open } = await import('@tauri-apps/plugin-dialog'); const f = await open({ filters: [{ name: 'Audio', extensions: ['ogg','wav','mp3','flac'] }] }); if (f) patchN({ custom_sound_path: f });`
   - "Reset" ghost button (visible only when custom path set): `patchN({ custom_sound_path: '' })`
   - "▶ Test" ghost button: `notifyTest().catch(console.error)` (one button covers both sound-preview and notification-test; hint text: "Fires a real notification. Ignores quiet hours.")
3. Row "Platforms" → four inline checkboxes (Twitch / YouTube / Kick / Chaturbate) each patching `platform_filter.{p}` — spread carefully: `patchN({ platform_filter: { ...n.platform_filter, twitch: v } })`. Label each with the `.rx-plat` chip classes used elsewhere (grep `rx-plat` usage).
4. Row "Quiet hours" → toggle `quiet_hours_enabled` + two `<input className="rx-input" style={{width: 64, boxSizing: 'border-box'}}>` for start/end. Validate on blur with `/^([01]\d|2[0-3]):[0-5]\d$/`; invalid → keep focus styling red (`border: '1px solid var(--live)'`) and DON'T patch. Hint: "Notifications are suppressed between these times (may wrap midnight)."
5. Row "Muted channels" → on tab mount, `listChannels()` into state; render `channels.filter(c => c.dont_notify)` as rows: display_name + platform chip + "Unmute" ghost button (`setChannelNotify(key, false)` then refetch) + an "Unmute all" button (confirm via the existing `ConfirmDialog` component — grep its props) iterating sequentially. Empty state: `No muted channels.`
6. Test-notification row is covered by the ▶ Test button in row 2 — do NOT duplicate it as a separate row (deviation from spec's 6-row sketch, noted here deliberately: one test affordance, not two).

- [ ] **Step 3: Remove the old General-tab row** — delete the "Notify when a channel goes live" Row from `GeneralTab` (grep `notify_on_live` in the file). Leave `close_to_tray` and the rest untouched. (Backend still tolerates the legacy field; UI stops writing it.)

- [ ] **Step 4: Verify in dev + build**

`npm run dev`: walk every row, flip everything, reload — persistence via mock settings; muted list shows mock channels muted in Task 7's testing. `npm run build` green.

- [ ] **Step 5: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(notify): Notifications preferences tab"
```

---

### Task 9: verification, roadmap, PR

**Files:**
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Full gate**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -2
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -1
npm run build
```
All green.

- [ ] **Step 2: Live smoke** (`npm run tauri:dev`): mute a channel via context menu → glyph; Preferences → Notifications: Test button fires notification + chime; set quiet hours spanning now + confirm a `notify_test` still fires (bypass) but debug logs show gate suppression on the next real go-live if one occurs; unmute-all with imported channels.

- [ ] **Step 3: Roadmap** — in `docs/ROADMAP.md`: under Phase 4, the `- [ ] **Rich notification events + settings**` bullet: split it — check a new bullet `- [x] **Notification core settings** (PR #N)` describing what shipped (per-channel mute UI + list, rodio sound w/ custom file, platform filter, quiet hours, test button, Notifications tab, notify_on_live migration) and leave `- [ ] Rich notification events — whispers/mentions/raids, recent-notifications log, urgency/timeout` as the remainder. Also check the matching items in the "### Notifications" proposed-backlog section (sound, quiet hours, per-platform filter, test button bullets) with `(PR #N)`.

- [ ] **Step 4: PR**

```bash
git push -u origin feat/notifications-frontend
gh pr create --title "feat(notify): mute UI, sound settings, quiet hours — Notifications tab (frontend)" --body "<Summary / tradeoffs / Test plan; reference spec>"
```
STOP — orchestrator reviews/merges.

---

## Self-review notes (already applied)

- Spec coverage: settings ✓(T1) gate ✓(T2) sound ✓(T3) wiring ✓(T4) IPC+smoke ✓(T5) wrappers/mocks/dialog ✓(T6) menu+glyph ✓(T7) tab ✓(T8) roadmap+verify ✓(T9). Migration ✓(T1). Error policies: sound fail-open (T3 code), malformed quiet times fail open (T2 tests), unknown-key IPC error (T5 returns Ok(false)/Err).
- Deliberate deviations from spec, both minor: (a) single ▶ Test button instead of separate sound-preview + test-notification rows (T8 step 2 note); (b) `notify_test` returns Err when disabled instead of silently no-op, so the UI can explain.
- Type consistency: `set_channel_notify(uniqueKey, mute)` ↔ `setChannelNotify(uniqueKey, mute)` ↔ store `set_dont_notify(key, mute)` — consistent throughout.
