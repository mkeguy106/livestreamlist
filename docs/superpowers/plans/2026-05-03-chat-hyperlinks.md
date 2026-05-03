# Chat Hyperlinks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Detect HTTP/HTTPS URLs (schemed and bare-domain) in chat-message text and render them as clickable links that open in the user's external browser.

**Architecture:** New Rust module `chat/links.rs` produces `LinkRange[]` peer to the existing emote scan. Ranges flow on `ChatMessage.link_ranges` (`#[serde(default)]` for backward compat) and are interleaved with emote ranges by the existing `EmoteText.jsx` segment-merge loop. New segment type renders as `<a>` with click intercepted to call the existing `open_url` IPC.

**Tech Stack:** Rust (regex 1, url 2, std::sync::LazyLock), React (existing EmoteText component), CSS variables (existing tokens.css).

**Spec:** `docs/superpowers/specs/2026-05-03-chat-hyperlinks-design.md`

---

### Task 1: Add `regex` crate, create `chat/links.rs` skeleton, wire into `chat/mod.rs`

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/chat/links.rs`
- Modify: `src-tauri/src/chat/mod.rs`

- [ ] **Step 1: Add `regex = "1"` to Cargo.toml**

In `src-tauri/Cargo.toml`, find the `[dependencies]` block and add:

```toml
regex = "1"
```

Insert alphabetically (between existing `r…` entries if any, otherwise wherever fits the existing ordering style).

- [ ] **Step 2: Create `chat/links.rs` with empty stub**

Create `src-tauri/src/chat/links.rs`:

```rust
use crate::chat::models::{EmoteRange, LinkRange};

/// Scan `text` for clickable URLs. Skip ranges that overlap any of `existing`
/// (so emote codes that happen to look URL-shaped don't double-tokenize).
///
/// Returns ranges sorted by `start`.
pub fn scan_links(_text: &str, _existing: &[EmoteRange]) -> Vec<LinkRange> {
    Vec::new()
}
```

- [ ] **Step 3: Add `pub mod links;` to `chat/mod.rs`**

In `src-tauri/src/chat/mod.rs`, add `pub mod links;` next to the other `pub mod ...;` lines for chat submodules (alphabetical between `irc` and `models`, or wherever fits).

- [ ] **Step 4: Verify it builds**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: compiles. Will fail if `LinkRange` doesn't exist yet — that's Task 2.

- [ ] **Step 5: Stage but don't commit yet**

Combined commit with Task 2 once the type exists and compiles.

---

### Task 2: `LinkRange` struct + `ChatMessage.link_ranges` field

**Files:**
- Modify: `src-tauri/src/chat/models.rs`

- [ ] **Step 1: Add `LinkRange` struct**

In `src-tauri/src/chat/models.rs`, immediately after the existing `EmoteRange` struct (around line 82), add:

```rust
/// Byte-range (inclusive-exclusive end) in ChatMessage.text where a clickable
/// URL appears, plus the normalized click target. The click target may differ
/// from `text[start..end]` — bare-domain matches get `https://` prepended, and
/// zero-width chars are stripped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRange {
    pub start: usize,
    pub end: usize,
    pub url: String,
}
```

- [ ] **Step 2: Add `link_ranges` field on `ChatMessage`**

Find the `ChatMessage` struct (top of file, around line 9). Add a new field after `pub emote_ranges: Vec<EmoteRange>,`:

```rust
    #[serde(default)]
    pub link_ranges: Vec<LinkRange>,
```

- [ ] **Step 3: Verify cargo check passes**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: compiles. Every `ChatMessage { ... }` literal in the codebase that didn't include `link_ranges` will now error — fix each by adding `link_ranges: Vec::new(),` (alphabetical position doesn't matter; Rust allows omitted fields with `..` only via update syntax). Locations to expect:

- `src-tauri/src/chat/twitch.rs::build_privmsg` (around line 446)
- `src-tauri/src/chat/twitch.rs::build_self_echo` (around line 547)
- `src-tauri/src/chat/twitch.rs::build_usernotice` (around line 638)
- `src-tauri/src/chat/kick.rs` (around line 328)

Add `link_ranges: Vec::new(),` to each. Re-run `cargo check` until clean.

- [ ] **Step 4: Commit Task 1 + Task 2 together**

```bash
git add src-tauri/Cargo.toml src-tauri/src/chat/links.rs src-tauri/src/chat/mod.rs src-tauri/src/chat/models.rs src-tauri/src/chat/twitch.rs src-tauri/src/chat/kick.rs
git commit -m "feat(chat): add LinkRange model + links module skeleton"
```

---

### Task 3: Schemed-URL detection (TDD core path)

**Files:**
- Modify: `src-tauri/src/chat/links.rs`

- [ ] **Step 1: Write a failing test for schemed URLs**

Append to `chat/links.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: usize, end: usize, url: &str) -> LinkRange {
        LinkRange { start, end, url: url.to_string() }
    }

    #[test]
    fn schemed_https_basic() {
        let got = scan_links("check https://example.com", &[]);
        assert_eq!(got, vec![r(6, 25, "https://example.com")]);
    }

    #[test]
    fn schemed_http_basic() {
        let got = scan_links("see http://example.com here", &[]);
        assert_eq!(got, vec![r(4, 22, "http://example.com")]);
    }

    #[test]
    fn no_match_plain_text() {
        let got = scan_links("hello world", &[]);
        assert!(got.is_empty());
    }
}
```

`PartialEq` isn't yet on `LinkRange`. Add `#[derive(PartialEq, Eq)]` to the struct in `models.rs` for tests:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRange {
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: `FAILED. 3 passed; 3 failed` (the no_match passes by default since `Vec::new()` is empty; the two scan_links tests fail).

Actually all three of the new tests are in the new module — `no_match_plain_text` will pass trivially (empty `Vec` matches empty expectation). The two URL tests fail with "left: [], right: [...]".

- [ ] **Step 3: Implement schemed-URL detection**

Replace the body of `scan_links` in `chat/links.rs`:

```rust
use std::sync::LazyLock;
use regex::Regex;
use crate::chat::models::{EmoteRange, LinkRange};

static SCHEMED_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bhttps?://[^\s<>"]+"#).expect("schemed URL regex compiles")
});

pub fn scan_links(text: &str, existing: &[EmoteRange]) -> Vec<LinkRange> {
    let mut out = Vec::new();
    for m in SCHEMED_URL_RE.find_iter(text) {
        let start = m.start();
        let end = m.end();
        // Skip overlap with existing emotes (defense-in-depth; will be
        // exercised in a later test).
        if existing.iter().any(|r| start < r.end && end > r.start) {
            continue;
        }
        let raw = &text[start..end];
        if let Ok(parsed) = url::Url::parse(raw) {
            out.push(LinkRange { start, end, url: parsed.to_string() });
        }
    }
    out.sort_by_key(|r| r.start);
    out
}
```

- [ ] **Step 4: Re-run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: 3 passed, 0 failed.

Note: `url::Url::parse` may produce `https://example.com/` (with trailing slash) — check the test expected value. If it does, adjust the test expected to `"https://example.com/"`. Use `assert_eq!` with whatever `Url::parse` actually produces (run once, observe, lock in).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/links.rs src-tauri/src/chat/models.rs
git commit -m "feat(chat/links): detect schemed http(s) URLs"
```

---

### Task 4: Trailing punctuation strip + paren balancing

**Files:**
- Modify: `src-tauri/src/chat/links.rs`

- [ ] **Step 1: Write failing tests for trailing punctuation + parens**

Append to the `tests` module in `chat/links.rs`:

```rust
    #[test]
    fn strip_trailing_period() {
        let got = scan_links("see https://example.com.", &[]);
        assert_eq!(got, vec![r(4, 23, "https://example.com/")]);
    }

    #[test]
    fn strip_trailing_punctuation_set() {
        // Each of .,;:!?'"*_ should not become part of the URL.
        let got = scan_links("hi https://example.com! bye", &[]);
        assert_eq!(got, vec![r(3, 22, "https://example.com/")]);
    }

    #[test]
    fn parens_unbalanced_outer() {
        // (https://example.com) — outer parens belong to the surrounding text.
        let got = scan_links("(https://example.com) bye", &[]);
        assert_eq!(got, vec![r(1, 20, "https://example.com/")]);
    }

    #[test]
    fn parens_balanced_inside_url() {
        // Wikipedia URL with balanced ( ) inside — keep the closing paren.
        let got = scan_links("see https://en.wikipedia.org/wiki/Foo_(bar) end", &[]);
        assert_eq!(
            got,
            vec![r(4, 42, "https://en.wikipedia.org/wiki/Foo_(bar)")]
        );
    }
```

(Adjust expected `url` strings if `Url::parse` adds/removes trailing slashes — run once, lock in.)

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: 4 new tests fail (URL captures `.`, `!`, `)`, etc. as part of the URL).

- [ ] **Step 3: Implement `strip_trailing_punct_and_balance_parens`**

Replace `scan_links` with the punctuation-aware version. Keep the regex; post-process each match:

```rust
pub fn scan_links(text: &str, existing: &[EmoteRange]) -> Vec<LinkRange> {
    let mut out = Vec::new();
    for m in SCHEMED_URL_RE.find_iter(text) {
        let mut start = m.start();
        let mut end = m.end();
        // Trim trailing punctuation + balance parens/brackets.
        end = trim_url_end(&text[start..end]) + start;
        if end <= start {
            continue;
        }
        if existing.iter().any(|r| start < r.end && end > r.start) {
            continue;
        }
        let _ = &mut start; // start is fixed; suppress lint
        let raw = &text[start..end];
        if let Ok(parsed) = url::Url::parse(raw) {
            out.push(LinkRange { start, end, url: parsed.to_string() });
        }
    }
    out.sort_by_key(|r| r.start);
    out
}

/// Returns the new candidate length (relative to the start of `s`) after
/// stripping trailing sentence punctuation and balancing parens/brackets.
fn trim_url_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut len = bytes.len();
    loop {
        let prev = len;
        // Strip trailing punctuation.
        while len > 0 {
            let c = bytes[len - 1];
            if matches!(c, b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'"' | b'*' | b'_') {
                len -= 1;
            } else {
                break;
            }
        }
        // Strip unbalanced trailing `)` or `]`.
        if len > 0 {
            let last = bytes[len - 1];
            if last == b')' || last == b']' {
                let (open, close) = if last == b')' { (b'(', b')') } else { (b'[', b']') };
                let mut opens = 0i32;
                let mut closes = 0i32;
                for &c in &bytes[..len] {
                    if c == open { opens += 1; }
                    else if c == close { closes += 1; }
                }
                if closes > opens {
                    len -= 1;
                    continue; // run punctuation pass again
                }
            }
        }
        if len == prev {
            break;
        }
    }
    len
}
```

- [ ] **Step 4: Re-run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: 7 passed, 0 failed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/links.rs
git commit -m "feat(chat/links): strip trailing punctuation + balance parens"
```

---

### Task 5: Bare-domain detection (TLD allowlist)

**Files:**
- Modify: `src-tauri/src/chat/links.rs`

- [ ] **Step 1: Write failing tests for bare domains**

Append to the `tests` module:

```rust
    #[test]
    fn bare_domain_allowlisted() {
        let got = scan_links("yo youtube.com/watch?v=abc end", &[]);
        assert_eq!(
            got,
            vec![r(3, 26, "https://youtube.com/watch?v=abc")]
        );
    }

    #[test]
    fn bare_domain_no_path() {
        let got = scan_links("visit example.com later", &[]);
        assert_eq!(got, vec![r(6, 17, "https://example.com/")]);
    }

    #[test]
    fn bare_domain_tld_not_in_allowlist() {
        // `.story` is not on the allowlist; treat as plain text.
        let got = scan_links("hey cool.story bro", &[]);
        assert!(got.is_empty(), "expected no link, got {:?}", got);
    }

    #[test]
    fn bare_domain_subdomain() {
        let got = scan_links("watch live.twitch.tv/shroud now", &[]);
        assert_eq!(
            got,
            vec![r(6, 27, "https://live.twitch.tv/shroud")]
        );
    }
```

(Lock in expected URL forms after first run.)

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: 4 new failures (no bare-domain matches yet).

- [ ] **Step 3: Add bare-domain regex + allowlist + integrate**

In `chat/links.rs`, add:

```rust
const TLD_ALLOWLIST: &[&str] = &[
    "com", "net", "org", "io", "gg", "tv", "edu", "gov",
    "co", "uk", "us", "me", "ly", "app", "dev", "fm",
    "live", "stream", "video", "art", "news", "to", "cc",
    "so", "ai", "xyz", "info", "sh",
];

static BARE_DOMAIN_RE: LazyLock<Regex> = LazyLock::new(|| {
    let tlds = TLD_ALLOWLIST.join("|");
    let pat = format!(
        r#"(?i)\b(?:[a-z0-9-]+\.)+(?:{tlds})\b(?:/[^\s<>"]*)?"#
    );
    Regex::new(&pat).expect("bare domain regex compiles")
});
```

Update `scan_links` to merge schemed + bare matches before processing. Schemed URLs win on overlap (a regex over `https://example.com` would match both patterns; we want the schemed one).

```rust
pub fn scan_links(text: &str, existing: &[EmoteRange]) -> Vec<LinkRange> {
    // Collect all candidate (start, end, has_scheme) spans.
    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    for m in SCHEMED_URL_RE.find_iter(text) {
        spans.push((m.start(), m.end(), true));
    }
    for m in BARE_DOMAIN_RE.find_iter(text) {
        // Skip if overlaps a schemed match (schemed already covers it).
        let s = m.start();
        let e = m.end();
        if spans.iter().any(|(ss, ee, _)| s < *ee && e > *ss) {
            continue;
        }
        spans.push((s, e, false));
    }
    spans.sort_by_key(|(s, _, _)| *s);

    let mut out = Vec::new();
    for (start, raw_end, has_scheme) in spans {
        let end = trim_url_end(&text[start..raw_end]) + start;
        if end <= start {
            continue;
        }
        if existing.iter().any(|r| start < r.end && end > r.start) {
            continue;
        }
        let raw = &text[start..end];
        let candidate = if has_scheme {
            raw.to_string()
        } else {
            format!("https://{raw}")
        };
        if let Ok(parsed) = url::Url::parse(&candidate) {
            out.push(LinkRange { start, end, url: parsed.to_string() });
        }
    }
    out
}
```

- [ ] **Step 4: Re-run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: all 11 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/links.rs
git commit -m "feat(chat/links): bare-domain detection with TLD allowlist"
```

---

### Task 6: Zero-width sneak handling + emote overlap test

**Files:**
- Modify: `src-tauri/src/chat/links.rs`

- [ ] **Step 1: Write failing tests for ZW chars + emote overlap**

Append:

```rust
    #[test]
    fn zero_width_chars_in_url() {
        // Twitch link-mod sometimes inserts ZW-space (U+200B) inside a URL.
        // The display range should keep them (so byte offsets match the raw
        // text); the click target should have them stripped.
        let text = "go to https://twitch.tv/\u{200B}shroud now";
        let got = scan_links(text, &[]);
        assert_eq!(got.len(), 1, "expected 1 link, got {got:?}");
        let link = &got[0];
        // Display range still spans the ZW char.
        assert_eq!(&text[link.start..link.end], "https://twitch.tv/\u{200B}shroud");
        // Click target has ZW stripped.
        assert_eq!(link.url, "https://twitch.tv/shroud");
    }

    #[test]
    fn skips_overlap_with_existing_emote() {
        // Existing emote covers byte range 0..7 (e.g., "Kappa12"). A bare-
        // domain match at 0..N should be dropped.
        let existing = vec![EmoteRange {
            start: 0,
            end: 11, // covers "twitch.tv/x"
            name: "FakeEmote".to_string(),
            url_1x: String::new(),
            url_2x: None,
            url_4x: None,
            animated: false,
        }];
        let got = scan_links("twitch.tv/x rest", &existing);
        assert!(got.is_empty(), "expected emote-overlap to drop link, got {got:?}");
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: emote-overlap test passes already (`existing.iter().any(...)` skip is in place); ZW test fails (current regex `[^\s<>"]+` includes ZW chars in the URL but doesn't strip them from the `url` field).

- [ ] **Step 3: Strip ZW chars from `url` field**

In `scan_links`, after building `candidate` and before `Url::parse`, strip ZW chars:

```rust
        let candidate_clean = strip_zero_width(&candidate);
        if let Ok(parsed) = url::Url::parse(&candidate_clean) {
            out.push(LinkRange { start, end, url: parsed.to_string() });
        }
```

Add helper:

```rust
fn strip_zero_width(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'))
        .collect()
}
```

- [ ] **Step 4: Re-run tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::links
```

Expected: all 13 passed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/links.rs
git commit -m "feat(chat/links): strip zero-width chars from click target"
```

---

### Task 7: Wire `scan_links` into Twitch chat producers

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs`

- [ ] **Step 1: Add use statement**

At the top of `chat/twitch.rs`, find the existing `use super::models::...` line and extend it to include `LinkRange`:

```rust
use super::models::{ChatBadge, ChatMessage, ChatRoomState, ChatRoomStateEvent, ChatStatus, ChatStatusEvent, ChatUser, EmoteRange, LinkRange, ReplyInfo, SystemEvent};
```

(Adjust list to match what's actually imported there — only add `LinkRange`.)

Add:

```rust
use super::links::scan_links;
```

- [ ] **Step 2: Wire into `build_privmsg`**

Find the line `emote_ranges.sort_by_key(|r| r.start);` near line 426 in `build_privmsg`. Immediately after it, add:

```rust
    let link_ranges = scan_links(&text, &emote_ranges);
```

In the `ChatMessage { ... }` literal at the end of the function, replace `link_ranges: Vec::new(),` (added in Task 2) with `link_ranges,`.

- [ ] **Step 3: Wire into `build_self_echo`**

Find the line `emote_ranges.sort_by_key(|r| r.start);` near line 545 in `build_self_echo`. After it, add:

```rust
    let link_ranges = scan_links(&clean_text, &emote_ranges);
```

Replace `link_ranges: Vec::new(),` with `link_ranges,` in the struct literal.

- [ ] **Step 4: Wire into `build_usernotice`**

Find the analogous spot in `build_usernotice` (around line 624, after `emote_ranges.sort_by_key(...)`). Add:

```rust
    let link_ranges = scan_links(&text, &emote_ranges);
```

Replace `link_ranges: Vec::new(),` with `link_ranges,` in the struct literal.

- [ ] **Step 5: Verify build + tests**

```bash
cargo build --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml chat::
```

Expected: clean build, all chat tests pass (existing IRC parse tests + new links tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/chat/twitch.rs
git commit -m "feat(chat/twitch): scan links in privmsg, self-echo, usernotice"
```

---

### Task 8: Wire `scan_links` into Kick chat producer

**Files:**
- Modify: `src-tauri/src/chat/kick.rs`

- [ ] **Step 1: Add use statement**

In `chat/kick.rs`, find `use super::models::{...}` (around line 28). Extend to include `LinkRange`:

```rust
use super::models::{ChatBadge, ChatMessage, ChatRoomState, ChatRoomStateEvent, ChatStatus, ChatStatusEvent, ChatUser, EmoteRange, LinkRange};
```

(Match actual import list, only add `LinkRange`.)

Add:

```rust
use super::links::scan_links;
```

- [ ] **Step 2: Compute link ranges**

After `extract_kick_emotes` is called and `emote_ranges` is populated (around line 272 — `let (stripped, emote_ranges) = extract_kick_emotes(&content);`), add:

```rust
    let link_ranges = scan_links(&stripped, &emote_ranges);
```

In the `ChatMessage { ... }` literal at the end of the parser (around line 328+), replace `link_ranges: Vec::new(),` with `link_ranges,`.

- [ ] **Step 3: Verify build**

```bash
cargo build --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: clean build, all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat/kick): scan links in incoming messages"
```

---

### Task 9: Frontend — interleave links in `EmoteText.jsx` + CSS

**Files:**
- Modify: `src/components/EmoteText.jsx`
- Modify: `src/tokens.css`

- [ ] **Step 1: Add `.rx-chat-link` CSS**

In `src/tokens.css`, append (under the existing reusable-class block — search for `.rx-mono` or `.rx-chiclet` for the right neighborhood):

```css
.rx-chat-link {
  color: var(--zinc-300);
  text-decoration: none;
  cursor: pointer;
}
.rx-chat-link:hover {
  color: var(--zinc-100);
  text-decoration: underline;
}
```

- [ ] **Step 2: Update `EmoteText.jsx` to accept `links` prop and render link segments**

Replace the entire body of `src/components/EmoteText.jsx` with:

```jsx
import { invoke } from '../ipc.js';
import Tooltip from './Tooltip.jsx';

/**
 * Render chat text with emote and link byte-ranges substituted for <img> /
 * <a> elements. Ranges are byte offsets (as emitted by the Rust IRC parser).
 * We slice the UTF-8 string with a TextEncoder to preserve character
 * boundaries when a range happens to sit next to a multi-byte unicode
 * codepoint.
 */
export default function EmoteText({ text, ranges, links, size = 20 }) {
  if (!text) return null;
  const emoteRanges = ranges ?? [];
  const linkRanges = links ?? [];
  if (emoteRanges.length === 0 && linkRanges.length === 0) {
    return <span>{text}</span>;
  }

  // Merge both arrays into one sorted list. Emotes win on overlap (link scan
  // already overlap-skips emotes server-side; defensive client-side too).
  const all = [
    ...emoteRanges.map((r) => ({ kind: 'emote', range: r })),
    ...linkRanges.map((r) => ({ kind: 'link', range: r })),
  ].sort((a, b) => a.range.start - b.range.start);

  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  const segments = [];
  let cursor = 0;

  const pushText = (s, e) => {
    if (e > s) segments.push({ type: 'text', text: decoder.decode(bytes.slice(s, e)) });
  };

  for (const item of all) {
    const { kind, range } = item;
    if (range.start < cursor) continue; // overlapping range; skip
    pushText(cursor, range.start);
    if (kind === 'emote') {
      segments.push({ type: 'emote', range });
    } else {
      const display = decoder.decode(bytes.slice(range.start, range.end));
      segments.push({ type: 'link', range, display });
    }
    cursor = range.end;
  }
  pushText(cursor, bytes.length);

  return (
    <span style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
      {segments.map((seg, i) => {
        if (seg.type === 'text') {
          return <span key={i}>{seg.text}</span>;
        }
        if (seg.type === 'emote') {
          return (
            <Tooltip
              key={i}
              placement="top"
              text={seg.range.name}
              wrapperStyle={{
                verticalAlign: -Math.round(size * 0.25),
                margin: '0 1px',
              }}
            >
              <img
                src={seg.range.url_1x}
                srcSet={
                  seg.range.url_2x
                    ? `${seg.range.url_1x} 1x, ${seg.range.url_2x} 2x${seg.range.url_4x ? `, ${seg.range.url_4x} 4x` : ''}`
                    : undefined
                }
                alt={seg.range.name}
                loading="lazy"
                style={{
                  height: size,
                  width: 'auto',
                }}
              />
            </Tooltip>
          );
        }
        // type === 'link'
        return (
          <a
            key={i}
            href={seg.range.url}
            className="rx-chat-link"
            onClick={(e) => {
              e.preventDefault();
              invoke('open_url', { url: seg.range.url });
            }}
          >
            {seg.display}
          </a>
        );
      })}
    </span>
  );
}
```

- [ ] **Step 3: Verify the frontend builds**

```bash
npm run build
```

Expected: clean Vite build.

- [ ] **Step 4: Commit**

```bash
git add src/components/EmoteText.jsx src/tokens.css
git commit -m "feat(chat): interleave clickable URL ranges in EmoteText"
```

---

### Task 10: Pass `link_ranges` through all `EmoteText` call sites

**Files:**
- Modify: `src/components/ChatView.jsx`

- [ ] **Step 1: Update three call sites in `ChatView.jsx`**

Three `<EmoteText ...>` invocations exist in `src/components/ChatView.jsx` (around lines 639, 712, 963). Each currently looks like:

```jsx
<EmoteText text={m.text} ranges={m.emote_ranges} size={20} />
```

Add a `links={m.link_ranges}` prop to each. After the change:

```jsx
<EmoteText text={m.text} ranges={m.emote_ranges} links={m.link_ranges} size={20} />
```

(Sizes 20, 18, and `compact ? 18 : 20` respectively — preserve existing.)

Verify all three sites are updated:

```bash
grep -n "EmoteText" src/components/ChatView.jsx
```

Each result should now include `links={m.link_ranges}`.

- [ ] **Step 2: Verify build**

```bash
npm run build
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chat): wire link_ranges through ChatView render sites"
```

---

### Task 11: End-to-end smoke test

**Files:** none (manual verification)

- [ ] **Step 1: Launch dev mode**

```bash
npm run tauri:dev
```

Wait for the window to appear and a Twitch channel to load.

- [ ] **Step 2: Type a URL in the composer and send**

In a Twitch channel where you're authenticated, type `check this out https://example.com/foo` and press Enter.

Expected: own message renders with `https://example.com/foo` styled in zinc-300, hover shows underline + zinc-100, click opens the URL in your default browser.

- [ ] **Step 3: Bare-domain test**

Send `youtube.com/watch?v=dQw4w9WgXcQ`.

Expected: link renders as typed (no `https://` shown), click opens `https://youtube.com/...` in browser.

- [ ] **Step 4: Wikipedia paren test**

Send `see https://en.wikipedia.org/wiki/Foo_(bar)`.

Expected: full URL including `(bar)` is the link target.

- [ ] **Step 5: Mixed emote + link test**

Send `Kappa https://example.com Kappa` (in a channel where Kappa is a known emote).

Expected: both Kappa images render and the URL is clickable.

- [ ] **Step 6: Inbound message test**

Wait for someone else in the channel to post a URL. Verify it renders identically.

- [ ] **Step 7: Kick smoke test**

Switch to a Kick channel that's live. Have a URL post (or send one yourself). Verify it renders and clicks correctly.

- [ ] **Step 8: Trailing punctuation visual confirmation**

Send `did you see https://example.com?` — expected: `https://example.com` is the link, the `?` is plaintext after.

- [ ] **Step 9: Update ROADMAP entry**

Modify `docs/ROADMAP.md`:

Find the line at line 101 starting with `- [ ] **Hyperlink support in chat — clickable URLs open in browser**` and flip the checkbox + append `(PR #N)` (replace `N` with the actual PR number once opened):

```markdown
- [x] **Hyperlink support in chat — clickable URLs open in browser** (PR #N) — [keep the existing description]
```

Commit:

```bash
git add docs/ROADMAP.md
git commit -m "docs(roadmap): mark chat hyperlinks shipped (PR #N)"
```

(Alternatively, do the roadmap update as a follow-up docs PR after the feature merges, per the "ship it" workflow in `CLAUDE.md`.)

- [ ] **Step 10: Stop dev mode**

`Ctrl+C` in the terminal running `npm run tauri:dev`.

---

## Self-review notes

- Spec coverage: ✓ all sections of the spec map to a task. Detection algorithm → tasks 3–6. Wiring → 7–8. Frontend → 9–10. Tests → embedded in 3–6 + manual smoke in 11.
- No placeholders. Every code block is concrete.
- Type names consistent: `LinkRange`, `link_ranges`, `scan_links` used identically in every task.
- Commit cadence: ~7 commits across the feature, one logical change each.
