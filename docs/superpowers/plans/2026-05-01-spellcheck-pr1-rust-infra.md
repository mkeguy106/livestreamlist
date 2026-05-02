# Spellcheck PR 1 — Rust Infrastructure + IPC

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the Rust-side spellcheck engine (hunspell binding, dict enumeration, personal dictionary, tokenizer) and expose four IPC commands plus three new `ChatSettings` fields. **No frontend UI in this PR** — verification is via cargo tests + manual `__TAURI_INTERNALS__` calls in devtools.

**Architecture:** New `src-tauri/src/spellcheck/` module with four files (`mod.rs`, `dict.rs`, `personal.rs`, `tokenize.rs`). `SpellChecker` lives in app state (`Arc<SpellChecker>`), holds a `parking_lot::Mutex<HashMap<String, Hunspell>>` for per-language dict caches plus an `RwLock<HashSet<String>>` for the personal dictionary. Settings get three new serde-defaulted fields on `ChatSettings`. IPC commands forward to `SpellChecker` methods.

**Tech Stack:** Rust 1.77+, Tauri 2, `hunspell-rs` (or `hunspell-sys` if the spike forces it), `parking_lot`, `serde`, `chrono` (already deps). System dep: `libhunspell-dev` on the build machine + `hunspell-en-us` (or equivalent) for runtime dicts on Linux. Bundled `en_US.aff/.dic` fallback shipped in `src-tauri/dictionaries/`.

**Spec:** `docs/superpowers/specs/2026-05-01-spellcheck-design.md`

---

## Task 0: Spike — verify hunspell builds

**Why this is task zero:** Per spec Risk 1, `hunspell-rs`'s C deps may fail on the build environment. A 30-min spike resolves which crate to use (or escalates to the user if both fail). Everything downstream assumes this works.

**Files:**
- Modify: `src-tauri/Cargo.toml`

**No TDD here** — this task is a build-verification spike, not a feature.

- [ ] **Step 1: Add `hunspell-rs` to Cargo.toml**

In `src-tauri/Cargo.toml`, under `[dependencies]`, add:

```toml
hunspell-rs = "0.4"
```

(If a newer version is on crates.io at execution time, prefer it.)

- [ ] **Step 2: Verify libhunspell-dev is installed on this machine**

Run: `pkg-config --exists hunspell && echo OK || echo MISSING`

If MISSING, install with the system package manager:
- Arch / CachyOS: `sudo pacman -S hunspell`
- Debian / Ubuntu: `sudo apt install libhunspell-dev`
- Fedora: `sudo dnf install hunspell-devel`
- macOS: `brew install hunspell`

- [ ] **Step 3: Try to build**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`

Expected: clean build (`Finished` line). If linker errors mentioning `hunspell`, libhunspell is not on the linker path — fix via Step 2.

- [ ] **Step 4: If `hunspell-rs` fails to compile or has an unusable API, fall back to `hunspell-sys`**

Replace the dependency with:

```toml
hunspell-sys = "0.4"
```

And every later task using `hunspell_rs::Hunspell` becomes `hunspell_sys::Hunhandle` plus thin Rust safe-wrappers. The plan's API surface is small enough (`new`, `add_dic`, `check`, `suggest`) that the swap is mechanical. Document the swap in the PR description.

If BOTH crates fail: stop and escalate to the user. Do not proceed with the rest of the plan — this means we need to revisit the architecture choice (drop back to JS-side `nspell`, which is the original architecture A from brainstorming).

- [ ] **Step 5: Commit the dependency add**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(spellcheck): add hunspell-rs dependency"
```

---

## Task 1: ChatSettings schema additions

**Files:**
- Modify: `src-tauri/src/settings.rs:111-152` (add fields + defaults + Default impl)
- Test: `src-tauri/src/settings.rs` (existing test module — extend `serde_round_trip` test)

- [ ] **Step 1: Write the failing serde round-trip extension**

In `src-tauri/src/settings.rs`, find the existing `let chat = ChatSettings { ... };` test setup (around line 218) and append the three new fields to its initialization, plus three new assertions after the round-trip:

```rust
let chat = ChatSettings {
    timestamp_24h: false,
    history_replay_count: 50,
    user_card_hover: false,
    user_card_hover_delay_ms: 800,
    show_badges: false,
    show_mod_badges: false,
    show_timestamps: false,
    spellcheck_enabled: false,
    autocorrect_enabled: false,
    spellcheck_language: "es_ES".to_string(),
};
let json = serde_json::to_string(&chat).unwrap();
let back: ChatSettings = serde_json::from_str(&json).unwrap();
assert_eq!(back.timestamp_24h, false);
// ... existing assertions ...
assert_eq!(back.spellcheck_enabled, false);
assert_eq!(back.autocorrect_enabled, false);
assert_eq!(back.spellcheck_language, "es_ES");
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests`

Expected: FAIL with "no field `spellcheck_enabled` on type `ChatSettings`".

- [ ] **Step 3: Add the three fields to ChatSettings**

In `src-tauri/src/settings.rs`, in the `ChatSettings` struct (line 111), add at the bottom of the field list:

```rust
    #[serde(default = "default_true")]
    pub spellcheck_enabled: bool,
    #[serde(default = "default_true")]
    pub autocorrect_enabled: bool,
    #[serde(default = "default_lang")]
    pub spellcheck_language: String,
```

Add the helper at the bottom of the existing default-fn block (around line 142):

```rust
fn default_lang() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| l.split('.').next().map(|s| s.to_string()))
        .filter(|s| !s.is_empty() && s != "C" && s != "POSIX")
        .unwrap_or_else(|| "en_US".to_string())
}
```

Update the `Default for ChatSettings` impl (line 144) to include the new fields:

```rust
impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            timestamp_24h: default_timestamp_24h(),
            history_replay_count: default_history_replay_count(),
            user_card_hover: default_user_card_hover(),
            user_card_hover_delay_ms: default_user_card_hover_delay_ms(),
            show_badges: default_true(),
            show_mod_badges: default_true(),
            show_timestamps: default_true(),
            spellcheck_enabled: default_true(),
            autocorrect_enabled: default_true(),
            spellcheck_language: default_lang(),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests`

Expected: PASS.

- [ ] **Step 5: Add a serde-default-on-missing test**

In `src-tauri/src/settings.rs`'s test module, add:

```rust
#[test]
fn chat_settings_defaults_for_missing_fields() {
    // Old config files without the new fields must still deserialize cleanly,
    // with the new fields taking their default-true / default-lang values.
    let json = r#"{"timestamp_24h":true,"history_replay_count":100,"user_card_hover":true,"user_card_hover_delay_ms":400,"show_badges":true,"show_mod_badges":true,"show_timestamps":true}"#;
    let chat: ChatSettings = serde_json::from_str(json).unwrap();
    assert_eq!(chat.spellcheck_enabled, true);
    assert_eq!(chat.autocorrect_enabled, true);
    assert!(!chat.spellcheck_language.is_empty());
}
```

- [ ] **Step 6: Run new test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests::chat_settings_defaults_for_missing_fields`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(spellcheck): add ChatSettings fields (spellcheck_enabled, autocorrect_enabled, spellcheck_language)"
```

---

## Task 2: tokenize.rs — pure tokenizer

**Files:**
- Create: `src-tauri/src/spellcheck/mod.rs` (initially just `pub mod tokenize;` and module declaration)
- Create: `src-tauri/src/spellcheck/tokenize.rs`
- Modify: `src-tauri/src/lib.rs` — add `mod spellcheck;` line near the other top-level `mod` declarations

- [ ] **Step 1: Wire the empty module into lib.rs**

In `src-tauri/src/lib.rs`, find the existing top-level `mod` declarations (e.g. `mod channels;`, `mod chat;`) and add:

```rust
mod spellcheck;
```

- [ ] **Step 2: Create the empty `mod.rs`**

Write `src-tauri/src/spellcheck/mod.rs`:

```rust
//! Spellcheck + autocorrect engine.
//!
//! Layout:
//! - `tokenize`  — pure tokenizer that classifies words / mentions / URLs /
//!   emote codes / all-caps shorthand. No external deps. Unit-testable.
//! - `personal`  — load/save the user's personal dictionary at
//!   `~/.config/livestreamlist/personal_dict.json`.
//! - `dict`      — enumerate installed hunspell dicts; bundled en_US fallback.
//! - (future)    — `SpellChecker` struct that wires these together against
//!   the hunspell crate. Lands in Task 5.

pub mod tokenize;
```

- [ ] **Step 3: Write the failing tokenize tests**

Create `src-tauri/src/spellcheck/tokenize.rs`:

```rust
//! Pure tokenizer. Splits a chat message into typed token ranges so the
//! caller can decide which tokens to spellcheck.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenClass {
    /// A regular word — should be spellchecked.
    Word,
    /// `@username` — skip.
    Mention,
    /// `https://…` or bare `domain.tld[/path]` — skip.
    Url,
    /// Emote code — either `:colon-form:` or an exact match against the
    /// per-channel emote list. Skip.
    Emote,
    /// 3+ uppercase letters with no internal punctuation (`LOL`, `LMAO`).
    /// Skip.
    AllCaps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRange {
    pub class: TokenClass,
    /// Byte offset (start, end) into the input text. UTF-8 safe.
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Split `text` into typed tokens. `channel_emotes` is the per-channel
/// emote-name list (used only by the `Emote` classifier — exact matches
/// against this list are tagged as emotes regardless of casing).
pub fn tokenize(text: &str, channel_emotes: &[String]) -> Vec<TokenRange> {
    // implementation pending
    let _ = (text, channel_emotes);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str, emotes: &[&str]) -> Vec<(TokenClass, &'static str)> {
        let owned: Vec<String> = emotes.iter().map(|s| s.to_string()).collect();
        tokenize(text, &owned)
            .into_iter()
            .map(|t| (t.class, leak_str(t.text)))
            .collect()
    }
    fn leak_str(s: String) -> &'static str { Box::leak(s.into_boxed_str()) }

    #[test]
    fn plain_words() {
        assert_eq!(
            classes("hello world", &[]),
            vec![(TokenClass::Word, "hello"), (TokenClass::Word, "world")],
        );
    }

    #[test]
    fn mentions_are_skipped() {
        assert_eq!(
            classes("hi @shroud how are you", &[]),
            vec![
                (TokenClass::Word, "hi"),
                (TokenClass::Mention, "@shroud"),
                (TokenClass::Word, "how"),
                (TokenClass::Word, "are"),
                (TokenClass::Word, "you"),
            ],
        );
    }

    #[test]
    fn urls_are_skipped() {
        // Both schemed and bare URLs.
        let r = classes("watch https://twitch.tv/shroud now", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::Url && *t == "https://twitch.tv/shroud"));
        let r = classes("twitch.tv/shroud is live", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::Url && *t == "twitch.tv/shroud"));
    }

    #[test]
    fn emote_list_match() {
        let r = classes("Kappa Kappa forsenE", &["Kappa", "forsenE"]);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|(c, _)| *c == TokenClass::Emote));
    }

    #[test]
    fn colon_form_emote() {
        let r = classes("hello :Kappa: world", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::Emote && *t == ":Kappa:"));
    }

    #[test]
    fn all_caps_shorthand() {
        let r = classes("LOL that was great LMAO", &[]);
        assert!(r.iter().any(|(c, t)| *c == TokenClass::AllCaps && *t == "LOL"));
        assert!(r.iter().any(|(c, t)| *c == TokenClass::AllCaps && *t == "LMAO"));
    }

    #[test]
    fn two_letter_caps_are_words_not_caps() {
        // Ambiguous pronouns ("OK", "AM"/"PM") are real words; the caps
        // skip-rule is 3+ chars to avoid false positives.
        let r = classes("OK fine", &[]);
        assert_eq!(r[0].0, TokenClass::Word);
    }

    #[test]
    fn unicode_words() {
        // Common Latin-1 + diacritics must tokenize as single words (no
        // splitting at ä / é).
        let r = classes("schöne grüße", &[]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, "schöne");
        assert_eq!(r[1].1, "grüße");
    }

    #[test]
    fn byte_offsets_are_correct() {
        // Caller relies on these to slice the original string.
        let text = "abc def";
        let toks = tokenize(text, &[]);
        assert_eq!(toks[0].start, 0);
        assert_eq!(toks[0].end, 3);
        assert_eq!(toks[1].start, 4);
        assert_eq!(toks[1].end, 7);
        assert_eq!(&text[toks[0].start..toks[0].end], "abc");
        assert_eq!(&text[toks[1].start..toks[1].end], "def");
    }

    #[test]
    fn mixed_punctuation_around_words() {
        // Trailing/leading punctuation must be stripped; the word inside
        // is what's tokenized.
        let r = classes("(hello), 'world'!", &[]);
        let words: Vec<&str> = r.iter().filter(|(c, _)| *c == TokenClass::Word).map(|(_, t)| *t).collect();
        assert_eq!(words, vec!["hello", "world"]);
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::tokenize::tests`

Expected: 10 FAILs (all tests, since `tokenize()` returns empty Vec).

- [ ] **Step 5: Implement tokenize**

Replace the placeholder `tokenize` body in `src-tauri/src/spellcheck/tokenize.rs`:

```rust
pub fn tokenize(text: &str, channel_emotes: &[String]) -> Vec<TokenRange> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() { break; }

        let token_start = i;
        // A token runs until whitespace. We then trim outer punctuation
        // and classify.
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let raw_end = i;
        let raw = &text[token_start..raw_end];

        // Trim leading/trailing punctuation (parens, quotes, commas, etc.)
        // that sit OUTSIDE the spellable word. We don't trim @, :, /, ., -
        // because those carry meaning (mention, emote, URL, hyphenated word).
        let trim_chars: &[char] = &['(', ')', ',', '!', '?', '"', '\'', ';', '`'];
        let trimmed = raw.trim_matches(trim_chars);
        if trimmed.is_empty() { continue; }
        let inner_start_offset = raw.find(trimmed).unwrap_or(0);
        let start = token_start + inner_start_offset;
        let end = start + trimmed.len();

        let class = classify(trimmed, channel_emotes);
        out.push(TokenRange {
            class,
            start,
            end,
            text: trimmed.to_string(),
        });
    }
    out
}

fn classify(token: &str, channel_emotes: &[String]) -> TokenClass {
    if token.starts_with('@') && token.len() > 1 {
        return TokenClass::Mention;
    }
    if is_url(token) {
        return TokenClass::Url;
    }
    if is_colon_emote(token) {
        return TokenClass::Emote;
    }
    if channel_emotes.iter().any(|e| e == token) {
        return TokenClass::Emote;
    }
    if is_all_caps_shorthand(token) {
        return TokenClass::AllCaps;
    }
    TokenClass::Word
}

fn is_url(token: &str) -> bool {
    if token.starts_with("https://") || token.starts_with("http://") {
        return true;
    }
    // Bare domain heuristic: contains a `.`, the part before the last `.`
    // is alphanumeric+hyphens, the part after is 2+ alpha (TLD-like).
    if let Some(dot_idx) = token.rfind('.') {
        let before = &token[..dot_idx];
        let after_dot = &token[dot_idx + 1..];
        let after_tld = after_dot.split('/').next().unwrap_or(after_dot);
        let tld_ok = after_tld.len() >= 2 && after_tld.chars().all(|c| c.is_ascii_alphabetic());
        let before_ok = !before.is_empty()
            && before.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if tld_ok && before_ok {
            return true;
        }
    }
    false
}

fn is_colon_emote(token: &str) -> bool {
    token.len() >= 3
        && token.starts_with(':')
        && token.ends_with(':')
        && token[1..token.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_all_caps_shorthand(token: &str) -> bool {
    if token.chars().count() < 3 { return false; }
    token.chars().all(|c| c.is_ascii_uppercase())
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::tokenize::tests`

Expected: 10 PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/spellcheck/mod.rs src-tauri/src/spellcheck/tokenize.rs
git commit -m "feat(spellcheck): pure tokenizer (Word / Mention / Url / Emote / AllCaps)"
```

---

## Task 3: personal.rs — personal dictionary load/save

**Files:**
- Create: `src-tauri/src/spellcheck/personal.rs`
- Modify: `src-tauri/src/spellcheck/mod.rs` — add `pub mod personal;`

- [ ] **Step 1: Wire the new module**

In `src-tauri/src/spellcheck/mod.rs`, add:

```rust
pub mod personal;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/spellcheck/personal.rs`:

```rust
//! Personal dictionary — words the user has explicitly marked as "not a
//! misspelling" via the right-click menu's "Add to dictionary" item.
//!
//! Persisted at `~/.config/livestreamlist/personal_dict.json` via the
//! existing `atomic_write` helper from `config::atomic_write`. Lowercase
//! normalized; case-insensitive lookup. Not language-scoped.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersonalDictFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    words: Vec<String>,
}

fn default_version() -> u32 { SCHEMA_VERSION }

#[derive(Debug, Clone, Default)]
pub struct PersonalDict {
    /// Lowercase-normalized.
    set: HashSet<String>,
    path: PathBuf,
}

impl PersonalDict {
    /// Load from `path`. Missing or malformed file yields an empty dict
    /// (preserving the path for the next `save`).
    pub fn load(path: PathBuf) -> Self {
        let set = match std::fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<PersonalDictFile>(&s) {
                Ok(file) => file.words.into_iter().map(|w| w.to_lowercase()).collect(),
                Err(_) => HashSet::new(),
            },
            Err(_) => HashSet::new(),
        };
        Self { set, path }
    }

    pub fn contains(&self, word: &str) -> bool {
        self.set.contains(&word.to_lowercase())
    }

    /// Add a word. Returns `Ok(true)` if newly inserted, `Ok(false)` if
    /// already present. On `true` the file is rewritten; on `false` no
    /// disk I/O occurs.
    pub fn add(&mut self, word: &str) -> Result<bool> {
        let normalized = word.to_lowercase();
        if !self.set.insert(normalized) {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    pub fn len(&self) -> usize { self.set.len() }
    pub fn is_empty(&self) -> bool { self.set.is_empty() }

    fn save(&self) -> Result<()> {
        let mut words: Vec<String> = self.set.iter().cloned().collect();
        words.sort();
        let file = PersonalDictFile {
            version: SCHEMA_VERSION,
            words,
        };
        let json = serde_json::to_string_pretty(&file)
            .context("serializing personal dict")?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {:?}", parent))?;
        }
        crate::config::atomic_write(&self.path, json.as_bytes())
            .with_context(|| format!("writing personal dict to {:?}", self.path))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp(td: &TempDir) -> PathBuf {
        td.path().join("personal_dict.json")
    }

    #[test]
    fn empty_when_missing() {
        let td = TempDir::new().unwrap();
        let d = PersonalDict::load(tmp(&td));
        assert!(d.is_empty());
        assert!(!d.contains("anything"));
    }

    #[test]
    fn malformed_json_yields_empty() {
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        std::fs::write(&p, "not json {{").unwrap();
        let d = PersonalDict::load(p);
        assert!(d.is_empty());
    }

    #[test]
    fn add_persists_and_round_trips() {
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        let mut d = PersonalDict::load(p.clone());
        assert_eq!(d.add("Kappa").unwrap(), true);
        assert_eq!(d.add("Kappa").unwrap(), false); // duplicate
        assert_eq!(d.len(), 1);

        let d2 = PersonalDict::load(p);
        assert!(d2.contains("Kappa"));
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let td = TempDir::new().unwrap();
        let mut d = PersonalDict::load(tmp(&td));
        d.add("StreamerName").unwrap();
        assert!(d.contains("streamername"));
        assert!(d.contains("STREAMERNAME"));
        assert!(d.contains("StreamerName"));
    }

    #[test]
    fn load_existing_file() {
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        std::fs::write(
            &p,
            r#"{"version":1,"words":["alpha","BETA","Gamma"]}"#,
        ).unwrap();
        let d = PersonalDict::load(p);
        assert_eq!(d.len(), 3);
        assert!(d.contains("alpha"));
        assert!(d.contains("beta"));
        assert!(d.contains("gamma"));
    }

    #[test]
    fn duplicate_add_is_noop_no_io() {
        // Calling add() with an existing word must NOT touch the file
        // (we'd see this in the mtime). Important so that
        // spellcheck_check doesn't re-write the file on every flagged
        // word that the user has already added.
        let td = TempDir::new().unwrap();
        let p = tmp(&td);
        let mut d = PersonalDict::load(p.clone());
        d.add("hello").unwrap();
        let mtime1 = std::fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        d.add("hello").unwrap();
        let mtime2 = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2, "second add should not have rewritten the file");
    }
}
```

- [ ] **Step 3: Add `tempfile` to dev-dependencies if not already present**

In `src-tauri/Cargo.toml`, check `[dev-dependencies]`. If `tempfile` is missing, add:

```toml
[dev-dependencies]
tempfile = "3"
```

Verify: `grep tempfile src-tauri/Cargo.toml` returns a hit.

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::personal::tests`

Expected: compile errors (referencing `crate::config::atomic_write`).

- [ ] **Step 5: Confirm `atomic_write` is exported from `config`**

Run: `grep -n "pub fn atomic_write\|pub use.*atomic_write" src-tauri/src/config.rs`

Expected: a `pub fn atomic_write(...)` definition. If the function exists but isn't pub-exported, modify `config.rs` to make it `pub`.

- [ ] **Step 6: Run tests to verify they now pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::personal::tests`

Expected: 6 PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/spellcheck/mod.rs src-tauri/src/spellcheck/personal.rs
[ -f src-tauri/src/config.rs ] && git add src-tauri/src/config.rs
git commit -m "feat(spellcheck): personal dictionary load/save"
```

---

## Task 4: dict.rs — dictionary enumeration

**Files:**
- Create: `src-tauri/src/spellcheck/dict.rs`
- Modify: `src-tauri/src/spellcheck/mod.rs` — add `pub mod dict;`

- [ ] **Step 1: Wire the new module**

In `src-tauri/src/spellcheck/mod.rs`, add:

```rust
pub mod dict;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/spellcheck/dict.rs`:

```rust
//! Enumerate hunspell dictionaries available on the host.
//!
//! Linux: scans `/usr/share/hunspell`, `/usr/share/myspell/dicts`, and
//! the Flatpak host paths (`/run/host/usr/share/hunspell`). Pairs `.aff`
//! and `.dic` files by basename.
//!
//! macOS / Windows: returns the bundled `en_US` only (no system enchant
//! integration in this PR).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictInfo {
    /// Locale code, e.g. `"en_US"`, `"de_DE"`.
    pub code: String,
    /// Human-readable name for the dropdown, e.g. `"English (US)"`.
    pub name: String,
    /// Absolute path to the `.aff` file. The matching `.dic` is at the
    /// same path with `.dic` extension. Internal — not part of the IPC
    /// contract; frontend only needs `code` and `name`.
    #[serde(skip, default = "default_path")]
    pub aff_path: PathBuf,
}

fn default_path() -> PathBuf {
    PathBuf::new()
}

/// Discover dictionaries on the system, plus the bundled fallback.
/// Returns a deduplicated list keyed by `code` — system entries take
/// precedence over the bundle.
pub fn list_dicts() -> Vec<DictInfo> {
    let mut found: Vec<DictInfo> = Vec::new();
    for dir in search_paths() {
        scan_dir(&dir, &mut found);
    }
    if let Some(bundled) = bundled_en_us_path() {
        if !found.iter().any(|d| d.code == "en_US") {
            found.push(DictInfo {
                code: "en_US".to_string(),
                name: "English (US) — bundled".to_string(),
                aff_path: bundled,
            });
        }
    }
    found.sort_by(|a, b| a.code.cmp(&b.code));
    found
}

/// Where to look on the current OS. Public for testing.
pub fn search_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        vec![
            PathBuf::from("/usr/share/hunspell"),
            PathBuf::from("/usr/share/myspell/dicts"),
            PathBuf::from("/run/host/usr/share/hunspell"),
            PathBuf::from("/app/share/hunspell"),
        ]
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec![]
    }
}

/// Path to the bundled fallback `en_US.aff`. `None` if not present.
pub fn bundled_en_us_path() -> Option<PathBuf> {
    // At runtime, the bundled dict ships alongside the executable.
    // Tauri places resources under the resource_dir; for dev we look
    // relative to the manifest dir.
    let candidates = [
        // Dev: walk up from CARGO_MANIFEST_DIR
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dictionaries/en_US.aff"),
        // Production bundles will copy here via tauri.conf.json resources.
        // Resolved at runtime by SpellChecker::new — see Task 6.
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Scan a directory for `.aff` files, pair them with their matching
/// `.dic`, and append `DictInfo` entries to `out`. Public for testing
/// (so unit tests can hand a tempdir).
pub fn scan_dir(dir: &Path, out: &mut Vec<DictInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("aff") {
            continue;
        }
        let stem = match p.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let dic = p.with_extension("dic");
        if !dic.exists() {
            continue;
        }
        if out.iter().any(|d| d.code == stem) {
            continue; // earlier path took precedence
        }
        out.push(DictInfo {
            code: stem.clone(),
            name: pretty_name(&stem),
            aff_path: p,
        });
    }
}

/// Map `en_US` → `English (US)`, `de_DE` → `German (Germany)`, etc.
/// Falls back to the raw code when unknown.
fn pretty_name(code: &str) -> String {
    let (lang, region) = match code.split_once('_') {
        Some((l, r)) => (l, Some(r)),
        None => (code, None),
    };
    let lang_name = match lang {
        "en" => "English",
        "es" => "Spanish",
        "de" => "German",
        "fr" => "French",
        "it" => "Italian",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "pl" => "Polish",
        "ru" => "Russian",
        "sv" => "Swedish",
        "no" => "Norwegian",
        "da" => "Danish",
        "fi" => "Finnish",
        "cs" => "Czech",
        "hu" => "Hungarian",
        "tr" => "Turkish",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh" => "Chinese",
        _ => return code.to_string(),
    };
    let region_name = region.and_then(|r| match r {
        "US" => Some("US"),
        "GB" | "UK" => Some("UK"),
        "CA" => Some("Canada"),
        "AU" => Some("Australia"),
        "DE" => Some("Germany"),
        "AT" => Some("Austria"),
        "CH" => Some("Switzerland"),
        "ES" => Some("Spain"),
        "MX" => Some("Mexico"),
        "FR" => Some("France"),
        "BR" => Some("Brazil"),
        "PT" => Some("Portugal"),
        _ => None,
    });
    match region_name {
        Some(r) => format!("{} ({})", lang_name, r),
        None => lang_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, "").unwrap();
    }

    #[test]
    fn scan_pairs_aff_and_dic() {
        let td = TempDir::new().unwrap();
        touch(&td.path().join("en_US.aff"));
        touch(&td.path().join("en_US.dic"));
        touch(&td.path().join("de_DE.aff"));
        touch(&td.path().join("de_DE.dic"));
        let mut out = Vec::new();
        scan_dir(td.path(), &mut out);
        out.sort_by(|a, b| a.code.cmp(&b.code));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].code, "de_DE");
        assert_eq!(out[1].code, "en_US");
    }

    #[test]
    fn scan_skips_aff_without_matching_dic() {
        let td = TempDir::new().unwrap();
        touch(&td.path().join("en_US.aff")); // no .dic
        touch(&td.path().join("de_DE.aff"));
        touch(&td.path().join("de_DE.dic"));
        let mut out = Vec::new();
        scan_dir(td.path(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "de_DE");
    }

    #[test]
    fn scan_dedupes_within_one_call() {
        let td = TempDir::new().unwrap();
        touch(&td.path().join("en_US.aff"));
        touch(&td.path().join("en_US.dic"));
        let mut out = Vec::new();
        scan_dir(td.path(), &mut out);
        // A second scan over the same dir must NOT add a duplicate.
        scan_dir(td.path(), &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn scan_missing_dir_is_noop() {
        let mut out = Vec::new();
        scan_dir(Path::new("/nonexistent/path/that/does/not/exist"), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn pretty_name_known_locales() {
        assert_eq!(pretty_name("en_US"), "English (US)");
        assert_eq!(pretty_name("de_DE"), "German (Germany)");
        assert_eq!(pretty_name("fr"), "French");
    }

    #[test]
    fn pretty_name_unknown_falls_back() {
        assert_eq!(pretty_name("xx_YY"), "xx_YY");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::dict::tests`

Expected: compile error (because the file is created and the code is complete; tests should actually pass on the first compile if the impl is correct). If they fail because of the bundled dict (`bundled_en_us_path` requires the file to exist), don't worry — those tests don't reference it.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::dict::tests`

Expected: 6 PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/spellcheck/mod.rs src-tauri/src/spellcheck/dict.rs
git commit -m "feat(spellcheck): dictionary enumeration with pretty-name mapping"
```

---

## Task 5: Bundled en_US fallback

**Why:** macOS / Windows users typically don't have hunspell dicts installed system-wide. Bundle `en_US.aff/.dic` so spellcheck works out of the box on first launch. Linux users with system hunspell-en-us installed get the system dict (newer / better-curated); the bundle is the floor.

**Files:**
- Create: `src-tauri/dictionaries/en_US.aff` (downloaded)
- Create: `src-tauri/dictionaries/en_US.dic` (downloaded)
- Modify: `src-tauri/tauri.conf.json` — add `bundle.resources` entry so the dicts ship in production builds
- Modify: `src-tauri/src/spellcheck/dict.rs::bundled_en_us_path` — also check the runtime resource directory

- [ ] **Step 1: Download the en_US dictionaries**

The `LibreOffice/dictionaries` GitHub repo has authoritative en_US files licensed permissively.

```bash
mkdir -p src-tauri/dictionaries
curl -fLo src-tauri/dictionaries/en_US.aff \
  https://raw.githubusercontent.com/LibreOffice/dictionaries/master/en/en_US.aff
curl -fLo src-tauri/dictionaries/en_US.dic \
  https://raw.githubusercontent.com/LibreOffice/dictionaries/master/en/en_US.dic
```

Verify both files exist and are non-empty:
```bash
wc -l src-tauri/dictionaries/en_US.aff src-tauri/dictionaries/en_US.dic
```
Expected: `.aff` is a few hundred lines; `.dic` is ~50,000 lines.

- [ ] **Step 2: Verify the dev-mode fallback works**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored bundled_en_us_path_resolves`

(This test doesn't exist yet — write it next.)

- [ ] **Step 3: Add an integration test that resolves the bundled path**

Append to `src-tauri/src/spellcheck/dict.rs`'s test module:

```rust
    #[test]
    fn bundled_en_us_path_resolves_in_dev() {
        // After Task 5, the bundled dict files exist under
        // CARGO_MANIFEST_DIR/dictionaries/. This test verifies dev-mode
        // fallback. (Production resource resolution is tested by
        // SpellChecker::new in Task 6.)
        let path = bundled_en_us_path()
            .expect("bundled en_US.aff should be present after Task 5");
        assert!(path.ends_with("en_US.aff"));
        assert!(path.exists());
        let dic = path.with_extension("dic");
        assert!(dic.exists(), "matching .dic should also exist");
    }
```

- [ ] **Step 4: Run the new test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::dict::tests::bundled_en_us_path_resolves_in_dev`

Expected: PASS.

- [ ] **Step 5: Wire the bundle into tauri.conf.json**

Open `src-tauri/tauri.conf.json` and find the `bundle` object. Add `resources` to it (creating the field if absent):

```json
"bundle": {
  …existing fields…
  "resources": [
    "dictionaries/*"
  ]
}
```

If `resources` already exists, append `"dictionaries/*"` to the array.

- [ ] **Step 6: Update bundled_en_us_path to also check the runtime resource dir**

In `src-tauri/src/spellcheck/dict.rs`, replace `bundled_en_us_path` with:

```rust
pub fn bundled_en_us_path() -> Option<PathBuf> {
    // Production: Tauri exposes resources via the resource_dir at runtime.
    // We can't reach AppHandle from this pure function, so we consult an
    // env var that's set by SpellChecker::new (which DOES have AppHandle).
    if let Ok(resolved) = std::env::var("LIVESTREAMLIST_RESOURCE_DIR") {
        let p = PathBuf::from(resolved).join("dictionaries/en_US.aff");
        if p.exists() {
            return Some(p);
        }
    }
    // Dev: walk up from CARGO_MANIFEST_DIR.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("dictionaries/en_US.aff");
    if dev.exists() { Some(dev) } else { None }
}
```

(`SpellChecker::new` in Task 6 will set `LIVESTREAMLIST_RESOURCE_DIR` from `app.path().resource_dir()`. Env-var indirection avoids threading `AppHandle` through this pure function.)

- [ ] **Step 7: Re-run dict tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::dict::tests`

Expected: all 7 PASS.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/dictionaries src-tauri/src/spellcheck/dict.rs src-tauri/tauri.conf.json
git commit -m "feat(spellcheck): bundle en_US.aff/.dic fallback for non-Linux platforms"
```

---

## Task 6: spellcheck/mod.rs — SpellChecker core (with hunspell)

**Files:**
- Modify: `src-tauri/src/spellcheck/mod.rs` — add the `SpellChecker` struct and methods

- [ ] **Step 1: Write the failing integration test**

Append to `src-tauri/src/spellcheck/mod.rs`:

```rust
use crate::spellcheck::dict::DictInfo;
use crate::spellcheck::personal::PersonalDict;
use crate::spellcheck::tokenize::{tokenize, TokenClass};
use anyhow::{anyhow, Context, Result};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// One misspelled word with its byte range in the input text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MisspelledRange {
    pub start: usize,
    pub end: usize,
    pub word: String,
}

/// App-wide spellcheck state. Held in `Arc<SpellChecker>` via
/// `tauri::Manager::manage`. Per-language `Hunspell` instances are
/// loaded lazily on first use.
pub struct SpellChecker {
    /// language code (`"en_US"`) → loaded Hunspell instance.
    /// Lazy: populated on first `check`/`suggest` for that language.
    by_lang: Mutex<HashMap<String, Arc<Mutex<hunspell_rs::Hunspell>>>>,
    /// All dicts discovered + bundled, for `list_dicts` and lazy load.
    available: Vec<DictInfo>,
    personal: RwLock<PersonalDict>,
}

impl SpellChecker {
    /// Construct from app-state context. `personal_dict_path` is
    /// usually `~/.config/livestreamlist/personal_dict.json` (computed
    /// by the caller using `config::data_dir()`).
    pub fn new(personal_dict_path: PathBuf) -> Self {
        let available = dict::list_dicts();
        let personal = PersonalDict::load(personal_dict_path);
        Self {
            by_lang: Mutex::new(HashMap::new()),
            available,
            personal: RwLock::new(personal),
        }
    }

    pub fn list_dicts(&self) -> Vec<DictInfo> {
        self.available.clone()
    }

    /// Load (or return cached) Hunspell instance for `code`. Returns
    /// `None` if no dict matches.
    fn dict_for(&self, code: &str) -> Option<Arc<Mutex<hunspell_rs::Hunspell>>> {
        let mut map = self.by_lang.lock();
        if let Some(d) = map.get(code) {
            return Some(d.clone());
        }
        let info = self.available.iter().find(|d| d.code == code)?;
        let aff = info.aff_path.to_string_lossy().to_string();
        let dic = info.aff_path.with_extension("dic").to_string_lossy().to_string();
        let h = hunspell_rs::Hunspell::new(&aff, &dic);
        let arc = Arc::new(Mutex::new(h));
        map.insert(code.to_string(), arc.clone());
        Some(arc)
    }

    /// Run spellcheck on `text` for `language`. Tokenizes, filters out
    /// non-Word tokens (mentions, URLs, emotes, all-caps shorthand),
    /// looks up each Word in hunspell, and skips anything in the
    /// personal dict. Returns ranges suitable for the React overlay.
    pub fn check(
        &self,
        text: &str,
        language: &str,
        channel_emotes: &[String],
    ) -> Vec<MisspelledRange> {
        let dict = match self.dict_for(language) {
            Some(d) => d,
            None => return Vec::new(),
        };
        let dict = dict.lock();
        let personal = self.personal.read();
        let mut out = Vec::new();
        for tok in tokenize(text, channel_emotes) {
            if tok.class != TokenClass::Word { continue; }
            if personal.contains(&tok.text) { continue; }
            if !dict.check(&tok.text) {
                out.push(MisspelledRange {
                    start: tok.start,
                    end: tok.end,
                    word: tok.text,
                });
            }
        }
        out
    }

    pub fn suggest(&self, word: &str, language: &str) -> Vec<String> {
        let Some(dict) = self.dict_for(language) else { return Vec::new(); };
        let dict = dict.lock();
        let mut s = dict.suggest(word);
        s.truncate(5);
        s
    }

    pub fn add_to_personal(&self, word: &str) -> Result<bool> {
        let mut p = self.personal.write();
        p.add(word)
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;

    fn checker_with_bundled() -> SpellChecker {
        let td = TempDir::new().unwrap();
        let dict_path = td.path().join("personal.json");
        // Leak td so the dir survives the test (avoids race with the
        // checker holding the path).
        std::mem::forget(td);
        SpellChecker::new(dict_path)
    }

    #[test]
    fn check_detects_obvious_misspelling() {
        // Requires Task 5's bundled en_US to be present.
        let c = checker_with_bundled();
        let result = c.check("hello wnoderful world", "en_US", &[]);
        let words: Vec<&str> = result.iter().map(|r| r.word.as_str()).collect();
        assert!(words.contains(&"wnoderful"), "expected wnoderful flagged, got {:?}", words);
        // Correct words must NOT be flagged.
        assert!(!words.contains(&"hello"));
        assert!(!words.contains(&"world"));
    }

    #[test]
    fn check_skips_mentions_urls_emotes() {
        let c = checker_with_bundled();
        let r = c.check("hi @shroud check twitch.tv/shroud Kappa LMAO",
                        "en_US",
                        &["Kappa".to_string()]);
        // None of the skip-tokens should appear as misspellings.
        for word in r.iter().map(|m| &m.word) {
            assert_ne!(word, "shroud");
            assert_ne!(word, "twitch.tv/shroud");
            assert_ne!(word, "Kappa");
            assert_ne!(word, "LMAO");
        }
    }

    #[test]
    fn personal_dict_suppresses_flag() {
        let c = checker_with_bundled();
        // First confirm "wnoderful" IS flagged…
        assert!(!c.check("wnoderful test", "en_US", &[]).is_empty());
        // …then add it and confirm it's NOT.
        c.add_to_personal("wnoderful").unwrap();
        let r = c.check("wnoderful test", "en_US", &[]);
        assert!(r.is_empty(), "personal-dict word should not be flagged: {:?}", r);
    }

    #[test]
    fn suggest_returns_corrections() {
        let c = checker_with_bundled();
        let s = c.suggest("teh", "en_US");
        assert!(!s.is_empty(), "expected at least one suggestion for 'teh'");
        assert!(s.iter().any(|s| s == "the"), "'the' should be in suggestions: {:?}", s);
    }

    #[test]
    fn unknown_language_returns_empty() {
        let c = checker_with_bundled();
        let r = c.check("anything", "xx_YY", &[]);
        assert!(r.is_empty());
        let s = c.suggest("teh", "xx_YY");
        assert!(s.is_empty());
    }
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml spellcheck::integration_tests`

Expected: 5 PASS. If `wnoderful` is not flagged or `teh` returns no suggestions, the bundled `en_US.dic` may be malformed — re-download in Task 5. If `Hunspell::new` panics, libhunspell is missing at runtime — install per Task 0 Step 2.

- [ ] **Step 3: If `hunspell_rs` API differs from what's used (`new(aff, dic)` / `check(word) -> bool` / `suggest(word) -> Vec<String>`)**

Check the actual crate API:
```bash
cargo doc --manifest-path src-tauri/Cargo.toml --no-deps -p hunspell-rs --open
```

If method names or return types differ, adjust `dict_for`, `check`, and `suggest` accordingly. The semantic contract (load aff+dic, ask `is this word valid?`, ask `suggestions for this word`) is universal across hunspell wrappers.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/spellcheck/mod.rs
git commit -m "feat(spellcheck): SpellChecker with lazy per-language dict load + integration tests"
```

---

## Task 7: IPC commands in lib.rs

**Files:**
- Modify: `src-tauri/src/lib.rs` — register `SpellChecker` state, add 4 commands, list in `generate_handler`

- [ ] **Step 1: Register `SpellChecker` in app state**

In `src-tauri/src/lib.rs`, find the `setup()` block (the closure passed to `tauri::Builder::default().setup(...)`). Near the other `app.manage(...)` calls (e.g. `app.manage(login_popup_mgr)`), add:

```rust
            // Set the resource dir env var so spellcheck::dict::bundled_en_us_path
            // can resolve it without needing AppHandle.
            if let Ok(res_dir) = app.path().resource_dir() {
                std::env::set_var("LIVESTREAMLIST_RESOURCE_DIR", &res_dir);
            }
            let personal_dict_path = crate::config::data_dir()
                .join("personal_dict.json");
            let spellchecker = std::sync::Arc::new(
                crate::spellcheck::SpellChecker::new(personal_dict_path),
            );
            app.manage(spellchecker);
```

(If `crate::config::data_dir` doesn't exist with that exact signature, find the existing helper that yields `~/.config/livestreamlist/` — likely `config::config_dir()` or similar — and use it.)

- [ ] **Step 2: Write the four IPC commands**

In `src-tauri/src/lib.rs`, near the other `#[tauri::command]` functions, add:

```rust
#[tauri::command]
async fn spellcheck_check(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
    text: String,
    language: String,
    channel_emotes: Vec<String>,
) -> Result<Vec<crate::spellcheck::MisspelledRange>, String> {
    Ok(state.check(&text, &language, &channel_emotes))
}

#[tauri::command]
async fn spellcheck_suggest(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
    word: String,
    language: String,
) -> Result<Vec<String>, String> {
    Ok(state.suggest(&word, &language))
}

#[tauri::command]
async fn spellcheck_add_word(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
    word: String,
) -> Result<bool, String> {
    state.add_to_personal(&word).map_err(err_string)
}

#[tauri::command]
async fn spellcheck_list_dicts(
    state: tauri::State<'_, std::sync::Arc<crate::spellcheck::SpellChecker>>,
) -> Result<Vec<crate::spellcheck::dict::DictInfo>, String> {
    Ok(state.list_dicts())
}
```

- [ ] **Step 3: Add the four commands to `generate_handler!`**

Find `tauri::generate_handler![...]` in `src-tauri/src/lib.rs` and append:

```rust
            spellcheck_check,
            spellcheck_suggest,
            spellcheck_add_word,
            spellcheck_list_dicts,
```

- [ ] **Step 4: Verify the build**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`

Expected: clean build. If errors complain about `MisspelledRange` not being `pub`, ensure `pub use` or `pub` qualifiers exist on the type in `spellcheck/mod.rs`.

- [ ] **Step 5: Run all tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all PASS (settings tests + tokenize tests + personal tests + dict tests + spellcheck integration tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(spellcheck): IPC commands (check, suggest, add_word, list_dicts)"
```

---

## Task 8: Frontend IPC wrappers

**Files:**
- Modify: `src/ipc.js` — add four wrapper functions

- [ ] **Step 1: Add the wrappers**

In `src/ipc.js`, near the other `invoke()` wrappers (search for `chatSend`, `listEmotes`, etc. for the existing pattern), add:

```js
// Spellcheck (PR 1 — engine + IPC only; UI lands in PR 2+)

export async function spellcheckCheck(text, language, channelEmotes) {
  return invoke('spellcheck_check', {
    text,
    language,
    channelEmotes: channelEmotes ?? [],
  });
}

export async function spellcheckSuggest(word, language) {
  return invoke('spellcheck_suggest', { word, language });
}

export async function spellcheckAddWord(word) {
  return invoke('spellcheck_add_word', { word });
}

export async function spellcheckListDicts() {
  return invoke('spellcheck_list_dicts');
}
```

(Look at how `chatSend` etc. handle the in-memory mock fallback for `npm run dev`. If there's a mock layer at the top of the file, add stubs that return reasonable empty values: `spellcheckCheck` → `[]`, `spellcheckSuggest` → `[]`, `spellcheckAddWord` → `true`, `spellcheckListDicts` → `[{ code: 'en_US', name: 'English (US)' }]`. This keeps the browser-only dev mode functional after PR 2 wires them up.)

- [ ] **Step 2: Quick syntax check**

Run: `npm run build`

Expected: clean build (no React/JSX errors).

- [ ] **Step 3: Commit**

```bash
git add src/ipc.js
git commit -m "feat(spellcheck): IPC wrappers (spellcheckCheck/Suggest/AddWord/ListDicts)"
```

---

## Task 9: Manual smoke test via devtools

This task has no code; it's verification.

- [ ] **Step 1: Launch the app**

Run: `npm run tauri:dev`

Expected: app opens normally; no spellcheck-related crashes in the terminal.

- [ ] **Step 2: Open devtools and invoke each command**

Right-click in the app → Inspect Element → Console. Run:

```js
const { invoke } = window.__TAURI_INTERNALS__;

// Should return e.g. [{ start: 6, end: 15, word: "wnoderful" }]
await invoke('spellcheck_check', {
  text: 'hello wnoderful world',
  language: 'en_US',
  channelEmotes: [],
});

// Should return at least ['the', ...] for 'teh'.
await invoke('spellcheck_suggest', { word: 'teh', language: 'en_US' });

// Should return true the first time, false on re-call.
await invoke('spellcheck_add_word', { word: 'wnoderful' });

// Should now return [] (wnoderful is in personal dict).
await invoke('spellcheck_check', {
  text: 'hello wnoderful world',
  language: 'en_US',
  channelEmotes: [],
});

// Should list at least en_US (system or bundled).
await invoke('spellcheck_list_dicts');
```

Verify each returns a sensible value. If any throws, check the terminal for Rust panic backtraces.

- [ ] **Step 3: Verify the personal dict file landed on disk**

```bash
cat ~/.config/livestreamlist/personal_dict.json
```

Expected: `{"version":1,"words":["wnoderful"]}` (formatted with newlines).

- [ ] **Step 4: Clean up the test entry**

```bash
rm ~/.config/livestreamlist/personal_dict.json
```

(Or edit out `"wnoderful"` if you want to keep other entries.)

- [ ] **Step 5: No commit needed** — this is verification only.

---

## Task 10: CLAUDE.md updates

**Files:**
- Modify: `CLAUDE.md` — IPC table, Module structure, Configuration files

- [ ] **Step 1: Add the four new IPC commands to the table**

In `CLAUDE.md`'s `### IPC — invoke commands` table (around line 115), append rows:

```
| `spellcheck_check` | `text, language, channelEmotes` | Tokenize input + return `[{ start, end, word }, ...]` for misspellings (skips `@mentions`, URLs, emote codes, all-caps shorthand, personal-dict words) |
| `spellcheck_suggest` | `word, language` | Top 5 hunspell suggestions for a word |
| `spellcheck_add_word` | `word` | Append to `personal_dict.json`; returns `true` if newly added |
| `spellcheck_list_dicts` | — | Enumerate available dicts (`{ code, name }`) for the Preferences language dropdown |
```

- [ ] **Step 2: Add the spellcheck module to the module structure**

In `CLAUDE.md`'s `### Module structure` section's Rust tree, under `src-tauri/src/`, add:

```
    ├── spellcheck/
    │   ├── mod.rs           # SpellChecker — per-language Hunspell cache, personal dict
    │   ├── tokenize.rs      # Pure tokenizer: Word / Mention / Url / Emote / AllCaps
    │   ├── personal.rs      # ~/.config/livestreamlist/personal_dict.json load/save
    │   └── dict.rs          # Enumerate /usr/share/hunspell etc. + bundled en_US fallback
```

(Insert in alphabetical-ish order; place after `refresh.rs` and before `streamlink.rs`.)

- [ ] **Step 3: Update the Configuration files list**

In `CLAUDE.md`'s `## Configuration` section, add to the Files list:

```
- `personal_dict.json` — user-added words for spellcheck (lowercase-normalized; `{ "version": 1, "words": [...] }`)
```

- [ ] **Step 4: Note the bundled dict in the dev-deps section**

In `CLAUDE.md` under `## Tech Stack`, append to the Backend bullet:

```
, `hunspell-rs` (system libhunspell) with bundled en_US.aff/.dic fallback under src-tauri/dictionaries/
```

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): document spellcheck module + IPC + personal_dict.json"
```

---

## Final verification

- [ ] **Step 1: All tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: every test green.

- [ ] **Step 2: Frontend builds**

Run: `npm run build`
Expected: clean.

- [ ] **Step 3: Branch summary**

Run: `git log --oneline main..HEAD`

Expected: ~9 commits, one per task. The diffstat (`git diff --stat main..HEAD`) should show roughly:
- 4 new Rust files in `src-tauri/src/spellcheck/`
- 2 new dict files in `src-tauri/dictionaries/`
- modified `Cargo.toml` (+2 deps), `Cargo.lock`, `lib.rs`, `settings.rs`, `tauri.conf.json`
- modified `src/ipc.js`
- modified `CLAUDE.md`

- [ ] **Step 4: Stop here.**

PR 1 is complete. Do NOT push, open the PR, or merge — wait for explicit "ship it" from the user, per the project's "Don't ship without explicit ask" rule.

---

## Notes for the implementer

- **The user follows a "ship it" workflow** that runs the full push → PR → merge → roadmap mark → docs PR → cleanup pipeline. Until they say "ship it", finished work stays on the branch locally.
- **Never commit directly to main**; this plan assumes the branch is `feat/spellcheck-1-rust-infra` (or similar) created off main before Task 0.
- **No emojis** in commit messages or code (per `CLAUDE.md` `## Git Commits`).
- **No reference to AI / Claude** in commit messages.
- The `hunspell-rs` API surface in this plan is the most-common one (`Hunspell::new(aff, dic) -> Self`, `check(&str) -> bool`, `suggest(&str) -> Vec<String>`). If the actual crate version differs, adjust call sites in Task 6 only — the rest of the plan is API-agnostic.
- **PR 2** (frontend overlay + red squiggles) is the natural next plan and will be drafted once PR 1 lands so it can build against the actual landed APIs.
