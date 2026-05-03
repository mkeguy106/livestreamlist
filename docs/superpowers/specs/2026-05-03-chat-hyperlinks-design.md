# Clickable URLs in chat messages — design

**Status:** approved (brainstorm 2026-05-03)
**Roadmap reference:** Phase 3 — Chat polish, "Hyperlink support in chat" bullet.

## Goal

Detect `http://` / `https://` URLs and bare `domain.tld/path` forms in chat-message text and render them as clickable links that open in the user's external browser. Mirrors the affordance every other chat client provides; right now URLs are dead text.

## Non-goals

Carried verbatim from the roadmap bullet:

- Link previews / OG-tag fetching / embedded thumbnails
- Internal app navigation (clicking `twitch.tv/shroud` opening the channel inside the app)
- Non-HTTP schemes (`mailto:`, `ftp:`, etc.)
- Markdown-style `[text](url)` syntax
- Detecting Twitch clips / YouTube videos / etc. for inline playback
- Autolinking `@mentions` to user cards (separate roadmap item)

## User-visible behavior

- A URL anywhere in a chat message renders as a link in `var(--zinc-300)` with no underline.
- Hover bumps to `var(--zinc-100)` and adds `text-decoration: underline`.
- Left-click opens the URL in the user's external browser via the existing `open_url` IPC. The chat webview does not navigate.
- `href` is set on the anchor so middle-click and "Copy link address" right-click work via WebKit defaults.
- Bare-domain links (`youtube.com/watch?v=abc`) display as the user typed them; the click target has `https://` prepended.
- Trailing sentence punctuation is stripped (`.,;:!?'"*_`); parens and brackets are balanced (Wikipedia URLs containing `(bar)` are kept whole).
- Links inside known emote codes are not detected (emote scan wins).

## Architecture

### New module: `src-tauri/src/chat/links.rs`

Peer to `chat/emotes.rs`. Public API:

```rust
pub fn scan_links(text: &str, existing: &[EmoteRange]) -> Vec<LinkRange>;
```

Pure function. Idempotent. Compiled regexes held in `std::sync::LazyLock<Regex>` (Rust ≥ 1.77 per `CLAUDE.md`; matches the codebase's stdlib-`OnceLock` style — see `chat/twitch.rs::SELF_ECHO_PREFIX`). Per-message scan is O(n) over message length.

Adds `regex = "1"` to `src-tauri/Cargo.toml` — first use of the crate in the project; small enough dep that pulling it in for this feature is fine.

### New struct: `LinkRange` in `chat/models.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkRange {
    /// Byte offset (inclusive) into ChatMessage.text.
    pub start: usize,
    /// Byte offset (exclusive) into ChatMessage.text.
    pub end: usize,
    /// Click target. Has `https://` prepended for bare-domain matches and
    /// zero-width characters stripped. Used by the frontend `open_url` call.
    pub url: String,
}
```

`ChatMessage` gains:

```rust
#[serde(default)]
pub link_ranges: Vec<LinkRange>,
```

`#[serde(default)]` keeps existing JSONL chat logs deserializing cleanly. Old logs replay without linkified URLs; we do not backfill on replay (not worth the runtime cost for historical messages).

### Detection algorithm

1. **Permissive match** with two alternatives OR'd together:
   - **Schemed**: `(?i)\bhttps?://[^\s<>"]+`
   - **Bare**: `(?i)\b(?:[a-z0-9-]+\.)+(?:com|net|org|io|gg|tv|edu|gov|co|uk|us|me|ly|app|dev|fm|live|stream|video|art|news|to|cc|so|ai|xyz|info|sh)\b(?:/[^\s<>"]*)?`

   The bare-domain pattern uses a curated TLD allowlist (~24 TLDs covering every commonly-shared platform plus general-purpose suffixes). Avoids false positives like `cool.story` or `1.5` while catching everything users actually paste. The list lives in a single `const TLD_ALLOWLIST: &[&str]` so additions are one-line.

2. **Trailing-punctuation strip with paren balancing** (the GFM autolink / linkify-it algorithm):
   - Trim trailing chars from `.,;:!?'"*_` while present.
   - For each trailing `)` (and `]`), count `(` minus `)` *inside* the candidate URL. If negative, the trailing `)` is unbalanced — strip it. Repeat until stable. Same for `[...]`.
   - This is the bit that makes `(https://example.com)` → outer parens stripped, but `https://en.wikipedia.org/wiki/Foo_(bar)` → kept whole.

3. **Validate**: prepend `https://` if scheme missing, then `url::Url::parse(...)`. On parse failure, drop the candidate.

4. **Zero-width normalization**: ZW-space (`​`), ZW-non-joiner (`‌`), ZW-joiner (`‍`), and BOM (`﻿`) are common Twitch link-mod evasion chars. The regex matches them as part of the URL (so byte offsets stay accurate against the original text); the stored `url` field has them stripped before `Url::parse`.

5. **Skip overlaps with existing emote ranges**: if the candidate's `[start, end)` overlaps any `EmoteRange` in `existing`, drop it. Emotes scan first, so they win.

### Wiring into chat producers

Three call sites, one new line each (after the existing emote-scan call):

- `chat/twitch.rs::build_privmsg` — incoming Twitch PRIVMSG
- `chat/twitch.rs::build_self_echo` — locally-synthesized echo of own messages (Twitch doesn't echo PRIVMSG)
- `chat/kick.rs` — incoming Kick chat message (after `extract_kick_emotes`)

Backfill (robotty) flows through `build_privmsg` already, so it inherits naturally. YouTube / Chaturbate chat is rendered by their embedded webviews — N/A for this feature.

### Frontend rendering

`src/components/EmoteText.jsx` takes a new `links` prop alongside `ranges`. Internally, the existing segment-merge loop generalizes: emote ranges + link ranges merge into one sorted list, overlap-skipped (link scan already overlap-skips emotes server-side, but client-side defense matches the existing pattern).

New segment type `'link'` renders:

```jsx
<a
  href={seg.url}
  onClick={(e) => {
    e.preventDefault();
    invoke('open_url', { url: seg.url });
  }}
  className="rx-chat-link"
>
  {displayText}
</a>
```

- `href` set for middle-click / "Copy link address" right-click via WebKit defaults.
- `onClick` `preventDefault` blocks the chat webview from navigating; routes through `open_url` IPC instead.
- `displayText` is `text.slice(start, end)` — the user-typed form, including any zero-width chars (invisible) and whatever scheme they used.

CSS added to `src/tokens.css`:

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

### Call sites that pass `links` through

`EmoteText` consumers in `ChatView.jsx` (`IrcRow`, `IrcRow` `/me` body, `CompactRow`, plus reply-context preview when threading lands) need to pass `m.link_ranges` alongside `m.emote_ranges`. Single grep pass.

## Testing

### Rust — `chat/links.rs` test module, table-driven

| Input | Expected |
|---|---|
| `"check https://example.com"` | one link `https://example.com` at byte offsets 6..25 |
| `"see https://example.com."` | link `https://example.com`, trailing `.` plaintext |
| `"(https://example.com)"` | link `https://example.com`, both parens plaintext |
| `"see https://en.wikipedia.org/wiki/Foo_(bar)"` | link includes `(bar)` (balanced parens kept) |
| `"yo youtube.com/watch?v=abc"` | bare-domain link, normalized `url = "https://youtube.com/watch?v=abc"` |
| `"hey cool.story bro"` | no link (TLD `.story` not in allowlist) |
| `"https://twitch.tv/​shroud"` | link with ZW char in display range, ZW stripped from `url` |
| `"Kappa"` with `Kappa` as a known emote in `existing` | no link (overlap-skipped) |
| `"mailto:foo@bar.com"` | no link (only http/https schemed) |
| `"hello"` | empty |

### Frontend smoke test (manual)

`EmoteText.jsx` is untested today; not adding test infra here. Smoke checklist after implementation:

- Type message containing a URL in the composer, send → renders as link, click opens browser.
- Bare-domain message → renders, click opens with `https://` prepended.
- Wikipedia URL with `(bar)` → kept whole, opens correctly.
- Mixed emote + link in same message → both render correctly.
- Paste a URL with zero-width sneak (from Twitch's link-modded chat) → still clickable, opens clean URL.
- Self-echo: type a URL, send → own message renders the link.
- Kick: same smoke checklist on a live Kick channel.

## Implementation notes

- **PR sizing**: one PR. Rust module + tests + `LinkRange` struct + `ChatMessage` field + 3 producer-site wirings + frontend interleave + CSS. Manageable in one review.
- **Backward compat**: `link_ranges` is `#[serde(default)]`. Existing JSONL chat logs replay cleanly without linkified URLs.
- **Performance**: regex compiled once via `once_cell::sync::Lazy<Regex>`. Per-message scan is O(n) over message length. Negligible at chat-message scale.
- **TLD allowlist evolution**: the constant lives in `chat/links.rs`. Adding TLDs is a one-line change. If false-positive complaints come in for `.story`-style suffixes, easier to extend than to switch to the full IANA list.
