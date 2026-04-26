# YouTube multi-concurrent-stream support — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Match the Qt app's YouTube multi-concurrent-stream behaviour. Channels broadcasting N simultaneous livestreams (NASA-style) appear as N rows; auto-Shorts portrait variants of the same primary feed dedupe to the landscape one.

**Architecture:** `platforms/youtube.rs` becomes an orchestrator that combines yt-dlp primary detection with two HTML scrapes (`/streams` for live video IDs, `/watch?v=` for per-video metadata + portrait check). `Livestream` gains `video_id: Option<String>`; `unique_key()` adds the `:{video_id}` suffix for live YT. `channels.rs` gets a `channel_key_of()` helper to strip the suffix for per-channel ops, plus `replace_livestreams_for_channel()` to merge per-channel batches with `YOUTUBE_MISS_THRESHOLD = 2` tolerance for transient secondary-stream scrape misses.

**Tech Stack:** Rust 1.77+, Tauri 2, `reqwest`, `serde_json`, `regex` (likely already in tree), existing `yt-dlp` subprocess. No frontend changes.

**Spec:** [`docs/superpowers/specs/2026-04-26-youtube-multi-stream-design.md`](../specs/2026-04-26-youtube-multi-stream-design.md)

---

## File map

| File | Change |
|---|---|
| `src-tauri/src/channels.rs` | Add `Livestream.video_id`, change `Livestream::unique_key` generation, add `channel_key_of()` helper, add `youtube_miss_counts` field on `ChannelStore`, refactor `snapshot()` to return multiple Livestreams per YT channel, add `replace_livestreams_for_channel()` |
| `src-tauri/src/platforms/youtube.rs` | Major rewrite: change `fetch_live` to return `YouTubeLive { streams: Vec<YouTubeStream> }`, add `parse_streams_page`, `parse_player_response`, `is_portrait`, `extract_initial_data`, `fetch_streams_html`, `fetch_watch_html`, `find_landscape_alternative` |
| `src-tauri/src/refresh.rs` | Update `fetch_youtube_all` to take `&reqwest::Client`, change return type to flattened `Vec<Livestream>`, route through new merge primitive |
| `src-tauri/src/lib.rs` | Add `channel_key_of` calls in `set_favorite`, `remove_channel`, `set_dont_notify`, `set_auto_play`. Update `open_in_browser` for YT to use video_id when present. |
| `src-tauri/src/embed.rs` | Read `livestream.video_id` directly instead of parsing thumbnail URL; keep URL parse as fallback |
| `docs/ROADMAP.md` | Tick the YT multi-stream item with `(PR #N)` after merge |

---

## Task 1: Add `video_id` field to `Livestream` + update `unique_key`

**Files:**
- Modify: `src-tauri/src/channels.rs:39-58` (Livestream struct), `60-159` (Livestream impl + per-platform constructors)

This is the smallest possible data-model change. The new field is `None` everywhere until Task 7 populates it from the new YT scraper. `unique_key` is generated on save (matches existing pattern) — for now it stays the channel key for everything since `video_id` is still always `None`.

- [ ] **Step 1: Write the failing test**

Append to the existing test module at the bottom of `src-tauri/src/channels.rs`. If there's no `#[cfg(test)] mod tests` block yet, create one. Add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::Platform;

    fn test_channel(platform: Platform, channel_id: &str) -> Channel {
        Channel {
            platform,
            channel_id: channel_id.to_string(),
            display_name: channel_id.to_string(),
            favorite: false,
            dont_notify: false,
            auto_play: false,
            added_at: None,
        }
    }

    #[test]
    fn livestream_unique_key_no_video_id_matches_channel() {
        let ch = test_channel(Platform::Youtube, "UC123");
        let ls = Livestream::offline_for(&ch, None);
        assert_eq!(ls.unique_key, "youtube:UC123");
        assert!(ls.video_id.is_none());
    }

    #[test]
    fn livestream_unique_key_with_video_id_appends_suffix() {
        let ch = test_channel(Platform::Youtube, "UC123");
        let mut ls = Livestream::offline_for(&ch, None);
        ls.video_id = Some("vidABC".to_string());
        ls.recompute_unique_key();
        assert_eq!(ls.unique_key, "youtube:UC123:vidABC");
    }

    #[test]
    fn livestream_unique_key_video_id_only_affects_youtube() {
        let ch = test_channel(Platform::Twitch, "ninja");
        let mut ls = Livestream::offline_for(&ch, None);
        // Twitch doesn't have a video_id field semantically, but if some
        // future platform sets it, the suffix must NOT be appended.
        ls.video_id = Some("anything".to_string());
        ls.recompute_unique_key();
        assert_eq!(ls.unique_key, "twitch:ninja");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests -- --nocapture`

Expected: compile errors — `video_id` field doesn't exist on `Livestream`, `recompute_unique_key` method doesn't exist.

- [ ] **Step 3: Add the field**

Edit `src-tauri/src/channels.rs` — find the `Livestream` struct (around line 39) and add `video_id` after `error`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Livestream {
    pub unique_key: String,
    pub platform: Platform,
    pub channel_id: String,
    pub display_name: String,
    pub is_live: bool,
    pub title: Option<String>,
    pub game: Option<String>,
    pub game_slug: Option<String>,
    pub viewers: Option<i64>,
    pub started_at: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
    pub profile_image_url: Option<String>,
    pub last_checked: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// YouTube-only: the live video id. When present on a YouTube
    /// Livestream, `unique_key` includes a trailing `:{video_id}` segment
    /// to distinguish multiple concurrent streams from the same channel.
    /// `None` for non-YT platforms and for offline YT placeholders.
    #[serde(default)]
    pub video_id: Option<String>,
    /// Mirrored from Channel so the frontend can filter to favorites without
    /// an extra round-trip.
    #[serde(default)]
    pub favorite: bool,
}
```

- [ ] **Step 4: Add `recompute_unique_key` method**

In the same file, find the `impl Livestream` block (around line 60) and add this method at the top of the impl, before `offline_for`:

```rust
impl Livestream {
    /// Build the unique_key from current platform/channel_id/video_id.
    /// Call this whenever video_id changes after construction.
    pub fn recompute_unique_key(&mut self) {
        self.unique_key = format!("{}:{}", self.platform.as_str(), self.channel_id);
        if matches!(self.platform, Platform::Youtube) {
            if let Some(vid) = &self.video_id {
                self.unique_key.push(':');
                self.unique_key.push_str(vid);
            }
        }
    }

    pub fn offline_for(channel: &Channel, profile_image_url: Option<String>) -> Self {
        // ... existing body unchanged
    }
    // ... rest of impl unchanged
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests -- --nocapture`

Expected: 3 tests pass.

- [ ] **Step 6: Run the full test suite to verify nothing broke**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all existing tests still pass (the new `video_id: Option<String>` field defaults to `None`, no callers touch it).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/channels.rs
git commit -m "feat(channels): add video_id field + recompute_unique_key on Livestream

Pre-req for YouTube multi-stream: live YT streams need a unique_key
suffixed with the video_id so multiple concurrent streams from the
same channel can coexist as separate entries. Field defaults to None
everywhere until the YT scraper starts populating it.

recompute_unique_key is a deliberate explicit call rather than a
getter or auto-derive — Livestream is serialized to/from JSON via
serde and we don't want the key to silently desync from the field
state."
```

---

## Task 2: `channel_key_of` helper

**Files:**
- Modify: `src-tauri/src/channels.rs` — add free function near `Channel::unique_key`

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `src-tauri/src/channels.rs`:

```rust
    #[test]
    fn channel_key_of_strips_yt_video_suffix() {
        assert_eq!(channel_key_of("youtube:UC123:vidABC"), "youtube:UC123");
    }

    #[test]
    fn channel_key_of_passthrough_for_yt_without_suffix() {
        assert_eq!(channel_key_of("youtube:UC123"), "youtube:UC123");
    }

    #[test]
    fn channel_key_of_passthrough_for_other_platforms() {
        assert_eq!(channel_key_of("twitch:ninja"), "twitch:ninja");
        assert_eq!(channel_key_of("kick:adin"), "kick:adin");
        assert_eq!(channel_key_of("chaturbate:user"), "chaturbate:user");
    }

    #[test]
    fn channel_key_of_handles_at_handle_yt_id() {
        assert_eq!(channel_key_of("youtube:@nasa"), "youtube:@nasa");
        assert_eq!(channel_key_of("youtube:@nasa:vid1"), "youtube:@nasa");
    }

    #[test]
    fn channel_key_of_returns_input_for_malformed() {
        // Empty / no colons / unknown platform — pass through, don't panic.
        assert_eq!(channel_key_of(""), "");
        assert_eq!(channel_key_of("not_a_key"), "not_a_key");
        assert_eq!(channel_key_of("unknownplatform:foo:bar"), "unknownplatform:foo:bar");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests::channel_key_of -- --nocapture`

Expected: compile errors — `channel_key_of` not defined.

- [ ] **Step 3: Implement the helper**

Add to `src-tauri/src/channels.rs` just before the `Livestream` struct definition (around line 37):

```rust
/// Given a stream-level unique_key (which may include a `:{video_id}`
/// suffix for live YouTube streams), return the channel-level unique_key.
///
/// For non-YouTube platforms and offline YouTube channels (those with no
/// suffix), returns the input unchanged. Used by per-channel IPC handlers
/// (set_favorite, remove_channel, set_dont_notify, set_auto_play) which
/// need to look up the Channel even when given a stream-level key.
pub fn channel_key_of(stream_key: &str) -> &str {
    if !stream_key.starts_with("youtube:") {
        return stream_key;
    }
    let mut parts = stream_key.splitn(3, ':');
    let plat = parts.next();
    let chan = parts.next();
    if plat.is_some() && chan.is_some() && parts.next().is_some() {
        let len = plat.unwrap().len() + 1 + chan.unwrap().len();
        return &stream_key[..len];
    }
    stream_key
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests::channel_key_of -- --nocapture`

Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/channels.rs
git commit -m "feat(channels): channel_key_of helper

Strips the :{video_id} suffix from a YouTube stream-level unique_key
to recover the channel-level key. Pass-through for everything else.

IPC routing helper for the upcoming multi-stream YouTube support:
per-channel handlers (favorite, remove, dont_notify, auto_play) get
the full stream key from the React side and need to look up the
Channel — this is the one-line conversion they call."
```

---

## Task 3: `is_portrait` helper

**Files:**
- Modify: `src-tauri/src/platforms/youtube.rs` — add free function

- [ ] **Step 1: Write the failing test**

Append a `#[cfg(test)] mod tests` block to `src-tauri/src/platforms/youtube.rs` (or extend if one exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_portrait_true_when_height_exceeds_width() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [{ "width": 720, "height": 1280 }]
            }
        });
        assert!(is_portrait(&response));
    }

    #[test]
    fn is_portrait_false_for_landscape() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [{ "width": 1920, "height": 1080 }]
            }
        });
        assert!(!is_portrait(&response));
    }

    #[test]
    fn is_portrait_falls_back_to_formats_when_adaptive_empty() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [],
                "formats": [{ "width": 480, "height": 1080 }]
            }
        });
        assert!(is_portrait(&response));
    }

    #[test]
    fn is_portrait_skips_format_entries_without_dimensions() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [
                    { "mimeType": "audio/mp4" },
                    { "width": 1920, "height": 1080 }
                ]
            }
        });
        assert!(!is_portrait(&response));
    }

    #[test]
    fn is_portrait_false_when_streaming_data_missing() {
        assert!(!is_portrait(&json!({})));
        assert!(!is_portrait(&json!({ "videoDetails": {} })));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::is_portrait -- --nocapture`

Expected: compile errors — `is_portrait` not defined.

- [ ] **Step 3: Implement**

Add to `src-tauri/src/platforms/youtube.rs` (anywhere outside an existing impl, before the `#[cfg(test)]` block):

```rust
/// True when the first format with a width and height has width < height.
/// Skips audio-only formats (which have no dimensions). False if no
/// dimensioned format is present.
fn is_portrait(player_response: &Value) -> bool {
    let streaming_data = match player_response.get("streamingData") {
        Some(v) => v,
        None => return false,
    };
    for fmts_key in &["adaptiveFormats", "formats"] {
        if let Some(fmts) = streaming_data.get(fmts_key).and_then(|v| v.as_array()) {
            for fmt in fmts {
                let w = fmt.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
                let h = fmt.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
                if w > 0 && h > 0 {
                    return w < h;
                }
            }
        }
    }
    false
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::is_portrait -- --nocapture`

Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/platforms/youtube.rs
git commit -m "feat(youtube): is_portrait helper for ytInitialPlayerResponse

Reads streamingData.adaptiveFormats[0].width/height (falls back to
formats[]) and returns width < height. Used to detect when YouTube's
/live URL handed us a portrait Shorts livestream that has a landscape
alternative on the same channel — the multi-stream orchestrator
swaps to the landscape one in that case."
```

---

## Task 4: `parse_streams_page` parser

**Files:**
- Modify: `src-tauri/src/platforms/youtube.rs` — add parse function

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `src-tauri/src/platforms/youtube.rs`:

```rust
    fn streams_page_fixture() -> Value {
        json!({
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "tabs": [
                        {
                            "tabRenderer": {
                                "title": "Streams",
                                "content": {
                                    "richGridRenderer": {
                                        "contents": [
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "live_v1",
                                                "thumbnailOverlays": [
                                                    { "thumbnailOverlayTimeStatusRenderer": { "style": "LIVE" } }
                                                ]
                                            }}}},
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "live_v2",
                                                "badges": [
                                                    { "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } }
                                                ]
                                            }}}},
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "vod_v3",
                                                "thumbnailOverlays": [
                                                    { "thumbnailOverlayTimeStatusRenderer": { "style": "DEFAULT" } }
                                                ]
                                            }}}},
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "live_v4",
                                                "badges": [{ "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } }],
                                                "thumbnailOverlays": [{ "thumbnailOverlayTimeStatusRenderer": { "style": "LIVE" } }]
                                            }}}}
                                        ]
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        })
    }

    #[test]
    fn parse_streams_page_extracts_live_video_ids() {
        let ids = parse_streams_page(&streams_page_fixture());
        assert_eq!(ids, vec!["live_v1", "live_v2", "live_v4"]);
    }

    #[test]
    fn parse_streams_page_returns_empty_for_unexpected_shape() {
        assert!(parse_streams_page(&json!({})).is_empty());
        assert!(parse_streams_page(&json!({ "contents": "wrong type" })).is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::parse_streams_page -- --nocapture`

Expected: compile error — `parse_streams_page` not defined.

- [ ] **Step 3: Implement**

Add to `src-tauri/src/platforms/youtube.rs` (outside any impl, before `#[cfg(test)]`):

```rust
/// Parse the `ytInitialData` JSON from a YouTube channel's `/streams` page
/// into a list of currently-live video IDs. Live status is detected via
/// either the `BADGE_STYLE_TYPE_LIVE_NOW` badge or the `LIVE` thumbnail-
/// overlay style — matches Qt's heuristic.
///
/// Returns an empty vec on any unexpected shape (missing fields, wrong
/// types). Better to underreport than to panic on a YouTube DOM change.
fn parse_streams_page(initial_data: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    let tabs = initial_data
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(|v| v.as_array());
    let Some(tabs) = tabs else { return ids };

    for tab in tabs {
        let contents = tab.pointer("/tabRenderer/content/richGridRenderer/contents");
        let Some(contents) = contents.and_then(|v| v.as_array()) else { continue };
        for item in contents {
            let renderer = item.pointer("/richItemRenderer/content/videoRenderer");
            let Some(renderer) = renderer else { continue };
            let badge_live = renderer
                .get("badges")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|b| {
                        b.pointer("/metadataBadgeRenderer/style")
                            .and_then(|v| v.as_str())
                            == Some("BADGE_STYLE_TYPE_LIVE_NOW")
                    })
                })
                .unwrap_or(false);
            let overlay_live = renderer
                .get("thumbnailOverlays")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|o| {
                        o.pointer("/thumbnailOverlayTimeStatusRenderer/style")
                            .and_then(|v| v.as_str())
                            == Some("LIVE")
                    })
                })
                .unwrap_or(false);
            if !(badge_live || overlay_live) {
                continue;
            }
            if let Some(vid) = renderer.get("videoId").and_then(|v| v.as_str()) {
                ids.push(vid.to_string());
            }
        }
    }
    ids
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::parse_streams_page -- --nocapture`

Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/platforms/youtube.rs
git commit -m "feat(youtube): parse_streams_page extracts live video IDs

Walks ytInitialData → contents.twoColumnBrowseResultsRenderer.tabs[]
.tabRenderer.content.richGridRenderer.contents[].richItemRenderer
.content.videoRenderer and returns the videoId for any item flagged
LIVE via either the BADGE_STYLE_TYPE_LIVE_NOW badge or the LIVE
thumbnail overlay. Matches the detection logic in Qt's
api/youtube.py::_fetch_concurrent_live_video_ids.

Tolerant of unexpected JSON shape — returns empty vec instead of
panicking. YouTube's DOM changes occasionally; better to underreport
than to crash a refresh tick."
```

---

## Task 5: `parse_player_response` parser

**Files:**
- Modify: `src-tauri/src/platforms/youtube.rs` — add parse function

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
    fn watch_player_response_fixture(video_id: &str, title: &str, viewers: i64) -> Value {
        json!({
            "videoDetails": {
                "videoId": video_id,
                "title": title,
                "isLive": true,
                "isLiveContent": true,
                "thumbnail": {
                    "thumbnails": [
                        { "url": "https://i.ytimg.com/vi/x/lo.jpg", "width": 168, "height": 94 },
                        { "url": "https://i.ytimg.com/vi/x/hi.jpg", "width": 1280, "height": 720 }
                    ]
                }
            },
            "streamingData": {
                "adaptiveFormats": [{ "width": 1920, "height": 1080 }]
            },
            "microformat": {
                "playerMicroformatRenderer": {
                    "liveBroadcastDetails": {
                        "isLiveNow": true,
                        "startTimestamp": "2026-04-26T12:00:00+00:00"
                    },
                    "viewCount": viewers.to_string()
                }
            }
        })
    }

    #[test]
    fn parse_player_response_extracts_metadata() {
        let stream = parse_player_response(
            &watch_player_response_fixture("vidXYZ", "ISS Earth View", 1234),
        ).expect("should parse");
        assert_eq!(stream.video_id, "vidXYZ");
        assert_eq!(stream.title, "ISS Earth View");
        assert_eq!(stream.viewers, Some(1234));
        assert_eq!(stream.thumbnail_url.as_deref(), Some("https://i.ytimg.com/vi/x/hi.jpg"));
        assert!(stream.started_at.is_some());
    }

    #[test]
    fn parse_player_response_returns_none_when_not_live() {
        let mut data = watch_player_response_fixture("vidXYZ", "title", 0);
        data["videoDetails"]["isLive"] = json!(false);
        data["microformat"]["playerMicroformatRenderer"]["liveBroadcastDetails"]["isLiveNow"] = json!(false);
        assert!(parse_player_response(&data).is_none());
    }

    #[test]
    fn parse_player_response_returns_none_for_unexpected_shape() {
        assert!(parse_player_response(&json!({})).is_none());
        assert!(parse_player_response(&json!({ "videoDetails": "wrong" })).is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::parse_player_response -- --nocapture`

Expected: compile error — `parse_player_response` not defined.

- [ ] **Step 3: Implement**

Add to `src-tauri/src/platforms/youtube.rs` (next to `parse_streams_page`):

```rust
/// Parse a `ytInitialPlayerResponse` JSON object into a `YouTubeStream`.
/// Returns `None` if the video isn't currently live or if any required
/// field is missing.
fn parse_player_response(player_response: &Value) -> Option<YouTubeStream> {
    let details = player_response.get("videoDetails")?;
    let video_id = details.get("videoId").and_then(|v| v.as_str())?.to_string();
    let title = details
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let is_live = details
        .get("isLive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || player_response
            .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if !is_live {
        return None;
    }

    // Thumbnail: pick the largest-width entry available.
    let thumbnail_url = details
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0))
                .and_then(|t| t.get("url").and_then(|u| u.as_str()))
                .map(String::from)
        });

    // Viewers: microformat.playerMicroformatRenderer.viewCount is a string.
    let viewers = player_response
        .pointer("/microformat/playerMicroformatRenderer/viewCount")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok());

    // started_at from liveBroadcastDetails.startTimestamp (RFC3339).
    let started_at = player_response
        .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/startTimestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    Some(YouTubeStream {
        video_id,
        title,
        viewers,
        game: None,
        started_at,
        thumbnail_url,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::parse_player_response -- --nocapture`

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/platforms/youtube.rs
git commit -m "feat(youtube): parse_player_response extracts per-video metadata

Reads videoDetails (id/title/thumbnail), microformat
.playerMicroformatRenderer.liveBroadcastDetails (isLiveNow,
startTimestamp), and microformat.viewCount. Returns None unless the
video is currently live. Tolerant of missing fields.

Used by the multi-stream orchestrator to fetch metadata for secondary
concurrent streams (the primary still comes from yt-dlp)."
```

---

## Task 6: HTML scraper helpers + initial-data extraction

**Files:**
- Modify: `src-tauri/src/platforms/youtube.rs` — add `extract_initial_data`, `fetch_streams_html`, `fetch_watch_html`, `SCRAPE_HEADERS` constant

- [ ] **Step 1: Write the failing test**

Append to the `tests` module:

```rust
    #[test]
    fn extract_initial_data_finds_var_assignment() {
        let html = r#"
            <html><head><script>var ytInitialData = {"key":"value"};</script></head></html>
        "#;
        let data = extract_initial_data(html, "ytInitialData").unwrap();
        assert_eq!(data["key"], "value");
    }

    #[test]
    fn extract_initial_data_handles_window_assignment() {
        let html = r#"<script>window["ytInitialPlayerResponse"] = {"a":1};</script>"#;
        let data = extract_initial_data(html, "ytInitialPlayerResponse").unwrap();
        assert_eq!(data["a"], 1);
    }

    #[test]
    fn extract_initial_data_returns_none_when_absent() {
        assert!(extract_initial_data("<html></html>", "ytInitialData").is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::extract_initial_data -- --nocapture`

Expected: compile error — `extract_initial_data` not defined.

- [ ] **Step 3: Implement extraction + headers + scrapers**

Add at the top of `src-tauri/src/platforms/youtube.rs` (after the existing `use` lines), and a new function block:

```rust
const SCRAPE_HEADERS: &[(&str, &str)] = &[
    ("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                    (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36"),
    ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    ("Accept-Language", "en-US,en;q=0.9"),
];

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Pull a `var Name = {...};` or `window["Name"] = {...};` JSON blob out
/// of an HTML page. The two patterns YouTube uses are accepted. Returns
/// None if the assignment isn't present or the JSON is malformed.
///
/// We deliberately do this with manual scanning instead of a full regex
/// because the embedded JSON contains arbitrary `;` and `}` characters
/// — we have to brace-balance to find the end. Same approach Qt's
/// `_parse_initial_data` and `_parse_player_response` take.
fn extract_initial_data(html: &str, var_name: &str) -> Option<Value> {
    // Try `var NAME = ` first.
    let start_marker_var = format!("var {var_name} = ");
    let start_marker_window = format!("window[\"{var_name}\"] = ");
    let candidates = [
        html.find(&start_marker_var).map(|i| i + start_marker_var.len()),
        html.find(&start_marker_window).map(|i| i + start_marker_window.len()),
    ];
    let json_start = candidates.iter().filter_map(|x| *x).next()?;
    let bytes = html[json_start..].as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    // Brace-balance to find the end of the JSON object.
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escape = false;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let json_slice = &html[json_start..json_start + end];
    serde_json::from_str(json_slice).ok()
}

/// HTTP GET the channel's `/streams` page and extract `ytInitialData`.
async fn fetch_streams_html(http: &reqwest::Client, channel_id: &str) -> Result<Option<Value>> {
    let url = if channel_id.starts_with("UC") && channel_id.len() == 24 {
        format!("https://www.youtube.com/channel/{channel_id}/streams")
    } else if channel_id.starts_with('@') {
        format!("https://www.youtube.com{channel_id}/streams")
    } else {
        format!("https://www.youtube.com/@{channel_id}/streams")
    };
    let mut req = http.get(&url).timeout(HTTP_TIMEOUT);
    for (k, v) in SCRAPE_HEADERS {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        log::debug!("YT /streams {channel_id}: HTTP {}", resp.status());
        return Ok(None);
    }
    let html = resp.text().await.context("reading /streams body")?;
    Ok(extract_initial_data(&html, "ytInitialData"))
}

/// HTTP GET `youtube.com/watch?v={id}` and extract `ytInitialPlayerResponse`.
async fn fetch_watch_html(http: &reqwest::Client, video_id: &str) -> Result<Option<Value>> {
    let url = format!("https://www.youtube.com/watch?v={video_id}");
    let mut req = http.get(&url).timeout(HTTP_TIMEOUT);
    for (k, v) in SCRAPE_HEADERS {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        log::debug!("YT /watch {video_id}: HTTP {}", resp.status());
        return Ok(None);
    }
    let html = resp.text().await.context("reading /watch body")?;
    Ok(extract_initial_data(&html, "ytInitialPlayerResponse"))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::extract_initial_data -- --nocapture`

Expected: 3 tests pass.

- [ ] **Step 5: Verify the whole module compiles + all existing tests still pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/platforms/youtube.rs
git commit -m "feat(youtube): HTML scraper helpers + ytInitialData extraction

extract_initial_data brace-balances JSON out of YouTube's HTML
shell. Two assignment patterns covered: \`var Name = {...}\` and
\`window[\"Name\"] = {...}\`. Manual scanning instead of regex
because embedded JSON contains arbitrary \`;\` and \`}\` characters.

fetch_streams_html / fetch_watch_html wrap reqwest with a Chrome-
flavoured user-agent and a 15 s timeout. The UA isn't a fingerprint
spoof — same caveat as the Chaturbate login window: spoofing on
WebKit triggers Cloudflare bot-checks. We're a real reqwest client
here, so a Chrome UA is just \"look like a normal browser\" without
fingerprint mismatch. Headers match Qt's SCRAPE_HEADERS."
```

---

## Task 7: Refactor `youtube::fetch_live` for multi-stream

**Files:**
- Modify: `src-tauri/src/platforms/youtube.rs` — change `fetch_live` signature, add `find_landscape_alternative`, update `YouTubeLive` struct

- [ ] **Step 1: Write the failing test**

Pure-function tests for the orchestration logic are tricky because it does I/O. Add an integration-style test that exercises the wire-up against synthetic data via a helper. Append to the `tests` module:

```rust
    #[test]
    fn fetch_live_signature_returns_vec_of_streams() {
        // Compile-time check: the new YouTubeLive struct must expose `streams: Vec<...>`.
        let live = YouTubeLive {
            channel_id: "x".into(),
            display_name: "x".into(),
            streams: vec![],
        };
        assert!(live.streams.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests::fetch_live_signature -- --nocapture`

Expected: compile error — `streams` field doesn't exist; `stream` does.

- [ ] **Step 3: Update `YouTubeLive` struct**

Edit `src-tauri/src/platforms/youtube.rs`. Find the `YouTubeLive` struct (around line 19) and replace `stream: Option<YouTubeStream>` with `streams: Vec<YouTubeStream>`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeLive {
    pub channel_id: String,
    pub display_name: String,
    /// Empty when the channel is offline. Length 1 for typical single-
    /// stream channels. Length 2+ for NASA-style multi-concurrent.
    pub streams: Vec<YouTubeStream>,
}
```

- [ ] **Step 4: Update existing `parse_live` and the `Ok(None)` early-return paths**

Find `parse_live` (around line 97) and update it to populate `streams`:

```rust
fn parse_live(root: &Value, channel_fallback: &str) -> YouTubeLive {
    let channel_id = root
        .get("channel_id")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_fallback)
        .to_string();
    let display_name = root
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or(&channel_id)
        .to_string();

    let is_live = root
        .get("is_live")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            root.get("live_status")
                .and_then(|v| v.as_str())
                .map(|s| s == "is_live")
        })
        .unwrap_or(false);

    let streams = if is_live {
        vec![parse_stream(root)]
    } else {
        Vec::new()
    };

    YouTubeLive {
        channel_id,
        display_name,
        streams,
    }
}
```

Find the `if stderr.contains("does not exist") || stderr.contains("is not currently live")` block (around line 82-88) and update its `YouTubeLive` literal to use `streams: Vec::new()` instead of `stream: None`.

- [ ] **Step 5: Update `fetch_live` to take `&reqwest::Client` and run the orchestration**

Replace the existing `fetch_live` (around lines 47-95) with:

```rust
/// Fetch the current set of live streams for a YouTube channel. Returns
/// an empty `streams` vec for offline channels. For the typical single-
/// stream live channel, returns `streams.len() == 1`. For NASA-style
/// multi-concurrent channels, returns `streams.len() >= 2`.
///
/// Calls yt-dlp for the primary stream + scrapes `/streams` for the
/// concurrent-list + scrapes `/watch?v=` per non-primary video for
/// metadata. Portrait dedupe (auto-Shorts variant of the same primary
/// feed): when the primary is portrait AND there's a landscape
/// alternative, swap to the landscape one.
pub async fn fetch_live(
    channel_id: &str,
    cookies_browser: Option<&str>,
    http: &reqwest::Client,
) -> Result<YouTubeLive> {
    // Step 1: primary via yt-dlp (existing path, factored out below).
    let primary = fetch_primary_via_ytdlp(channel_id, cookies_browser).await?;

    // Offline → empty streams.
    let primary_stream = match primary.streams.first() {
        Some(s) => s.clone(),
        None => return Ok(primary),
    };

    // Step 2: concurrent-list scrape.
    let live_ids = match fetch_streams_html(http, channel_id).await {
        Ok(Some(data)) => parse_streams_page(&data),
        Ok(None) => Vec::new(),
        Err(e) => {
            log::warn!("YT /streams scrape failed for {channel_id}: {e:#}");
            Vec::new()
        }
    };

    // Single live id (or scrape failed) — return primary as-is.
    if live_ids.len() <= 1 {
        return Ok(primary);
    }

    // Step 3: portrait dedupe on primary — only when there's something to swap to.
    let mut primary_resolved = primary_stream.clone();
    let primary_player = match fetch_watch_html(http, &primary_stream.video_id).await {
        Ok(Some(p)) => Some(p),
        Ok(None) => None,
        Err(e) => {
            log::debug!("YT /watch primary scrape failed: {e:#}");
            None
        }
    };
    if primary_player.as_ref().map(is_portrait).unwrap_or(false) {
        if let Some(landscape) =
            find_landscape_alternative(http, &live_ids, &primary_stream.video_id).await
        {
            primary_resolved = landscape;
        }
    }

    // Step 4: assemble the final stream list.
    let mut streams = vec![primary_resolved.clone()];
    for vid in live_ids.iter() {
        if vid == &primary_resolved.video_id {
            continue;
        }
        match fetch_watch_html(http, vid).await {
            Ok(Some(player_response)) => {
                if let Some(stream) = parse_player_response(&player_response) {
                    streams.push(stream);
                }
            }
            Ok(None) => log::debug!("YT /watch {vid}: no player response"),
            Err(e) => log::warn!("YT /watch {vid} failed: {e:#}"),
        }
    }

    Ok(YouTubeLive {
        channel_id: primary.channel_id,
        display_name: primary.display_name,
        streams,
    })
}

/// Returns the first non-portrait `YouTubeStream` from the candidate
/// list (excluding `current_video_id`). Used to swap an auto-Shorts
/// primary for the matching landscape variant on the same channel.
async fn find_landscape_alternative(
    http: &reqwest::Client,
    candidates: &[String],
    current_video_id: &str,
) -> Option<YouTubeStream> {
    for vid in candidates {
        if vid == current_video_id {
            continue;
        }
        let player_response = match fetch_watch_html(http, vid).await {
            Ok(Some(p)) => p,
            _ => continue,
        };
        if !is_portrait(&player_response) {
            return parse_player_response(&player_response);
        }
    }
    None
}
```

- [ ] **Step 6: Extract the existing yt-dlp body into `fetch_primary_via_ytdlp`**

The body of the OLD `fetch_live` (everything after the function signature, ending with `Ok(Some(parse_live(&data, channel_id)))`) becomes a new helper:

```rust
async fn fetch_primary_via_ytdlp(
    channel_id: &str,
    cookies_browser: Option<&str>,
) -> Result<YouTubeLive> {
    let url = live_url(channel_id);
    let mut cmd = Command::new("yt-dlp");
    cmd.arg("--dump-single-json")
        .arg("--no-download")
        .arg("--no-warnings")
        .arg("--skip-download")
        .arg("--no-playlist");
    for arg in crate::auth::youtube::yt_dlp_cookie_args(cookies_browser) {
        cmd.arg(arg);
    }
    let run = cmd.arg(&url).kill_on_drop(true).output();

    let out = match timeout(TIMEOUT, run).await {
        Err(_) => anyhow::bail!("yt-dlp timed out for {url}"),
        Ok(Err(e)) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::bail!("yt-dlp not found on PATH — install it to use YouTube channels");
            }
            return Err(e).context("spawning yt-dlp");
        }
        Ok(Ok(o)) => o,
    };

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("does not exist") || stderr.contains("is not currently live") {
            return Ok(YouTubeLive {
                channel_id: channel_id.to_string(),
                display_name: channel_id.to_string(),
                streams: Vec::new(),
            });
        }
        anyhow::bail!("yt-dlp failed: {stderr}");
    }

    let data: Value = serde_json::from_slice(&out.stdout).context("parsing yt-dlp JSON")?;
    Ok(parse_live(&data, channel_id))
}
```

- [ ] **Step 7: Run all tests + cargo check**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: compilation succeeds for `youtube.rs`. Will FAIL elsewhere because `refresh.rs` still calls `fetch_live` with the old signature (`Option<YouTubeLive>` return, no `http` arg) and the old `Ok(Some(...))` pattern. Those callers get fixed in Task 9.

`cargo test --manifest-path src-tauri/Cargo.toml platforms::youtube::tests` should still pass — `fetch_live_signature_returns_vec_of_streams` now compiles and passes.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/platforms/youtube.rs
git commit -m "feat(youtube): refactor fetch_live for multi-stream orchestration

Returns YouTubeLive { streams: Vec<YouTubeStream> } instead of
Option<YouTubeStream>. Empty streams = offline. Length 1 = typical
single-stream live channel. Length 2+ = NASA-style multi-concurrent.

Orchestration:
1. Primary via yt-dlp (factored into fetch_primary_via_ytdlp)
2. Scrape /streams for concurrent live video IDs
3. If only one ID, return primary as-is (no orientation probe)
4. If multiple, /watch?v= the primary to check orientation
5. If primary is portrait AND a landscape alternative exists,
   swap (find_landscape_alternative)
6. /watch?v= each non-primary video for metadata, append

Build is intentionally broken at refresh.rs after this commit —
fixed in the next task."
```

---

## Task 8: Update refresh.rs for the new fetch_live signature + flatten Vec

**Files:**
- Modify: `src-tauri/src/refresh.rs:11-155` — pass http client through, change return shape, flatten

- [ ] **Step 1: Update `fetch_youtube_all` signature and return type**

Find the current `fetch_youtube_all` (around line 127). Replace it with:

```rust
/// Run yt-dlp + concurrent-list scraping in batches of YT_CONCURRENCY.
/// Returns one entry per channel id. Each entry's `streams` vec is empty
/// for offline channels, length 1 for single-stream live channels, and
/// length >= 2 for multi-concurrent channels.
async fn fetch_youtube_all(
    ids: &[String],
    cookies_browser: Option<&str>,
    http: &reqwest::Client,
) -> HashMap<String, youtube::YouTubeLive> {
    let mut out = HashMap::new();
    for chunk in ids.chunks(YT_CONCURRENCY) {
        let futs: Vec<_> = chunk
            .iter()
            .map(|id| async move {
                (id.clone(), youtube::fetch_live(id, cookies_browser, http).await)
            })
            .collect();
        let results = join_all(futs).await;
        for (id, res) in results {
            match res {
                Ok(live) => {
                    out.insert(id, live);
                }
                Err(e) => log::warn!("YouTube refresh failed for {id}: {e:#}"),
            }
        }
    }
    out
}
```

- [ ] **Step 2: Update the caller of `fetch_youtube_all`**

Find the line `let youtube_fut = fetch_youtube_all(&youtube_ids, youtube_cookies_browser.as_deref());` (around line 39). Update to pass the HTTP client:

```rust
    let youtube_fut = fetch_youtube_all(&youtube_ids, youtube_cookies_browser.as_deref(), http);
```

The surrounding function (`refresh_all` or similar) already has access to `&AppState` which carries `http: reqwest::Client`. If not visible at this line, thread it in via the function signature. (Look at the function header to confirm.)

- [ ] **Step 3: Update the YT result → Livestream conversion**

Find where YouTube results are merged into the store. The current code likely calls `store.upsert_livestream(...)` once per channel using `live.stream` (`Option<YouTubeStream>`). Locate it and replace with a per-stream loop:

```rust
    // YouTube — flatten Vec<YouTubeStream> into individual Livestream entries.
    // Per-channel batch update happens via replace_livestreams_for_channel
    // (added in Task 10) so secondary streams that vanish get the miss-
    // threshold treatment instead of immediate offline fire.
    for channel in &youtube_channels {
        let live = match youtube_results.get(&channel.channel_id) {
            Some(l) => l,
            None => continue, // refresh failed for this channel
        };
        let mut streams: Vec<channels::Livestream> = live
            .streams
            .iter()
            .map(|s| channels::Livestream::from_youtube(channel, s))
            .collect();
        if streams.is_empty() {
            // offline placeholder
            streams.push(channels::Livestream::offline_for(channel, None));
        }
        store.lock().replace_livestreams_for_channel(&channel.unique_key(), streams);
    }
```

The exact merge code in your refresh.rs may differ — adapt to the existing pattern, but the shape (loop per channel, build Vec, call `replace_livestreams_for_channel`) is the same.

`channels::Livestream::from_youtube` is added in Task 9; for now this won't compile, that's expected.

- [ ] **Step 4: Don't run the build yet**

The build will fail because `from_youtube`, `replace_livestreams_for_channel` don't exist yet — wired up in Tasks 9 and 10. Skip to commit.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/refresh.rs
git commit -m "feat(refresh): thread http client + Vec<Stream> through YouTube path

Updates the YouTube branch of the refresh pipeline to match the new
fetch_live signature (takes &reqwest::Client) and result shape (Vec
of streams instead of Option). Per-channel results flow through
replace_livestreams_for_channel (added in the next two commits) so
the miss-threshold logic can intercept secondary-stream disappearance.

Build is intentionally broken until the channels.rs additions land.\""
```

---

## Task 9: Add `Livestream::from_youtube`

**Files:**
- Modify: `src-tauri/src/channels.rs` — add constructor

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `src-tauri/src/channels.rs`:

```rust
    #[test]
    fn from_youtube_populates_video_id_and_unique_key() {
        let ch = test_channel(Platform::Youtube, "UCnasa");
        let stream = crate::platforms::youtube::YouTubeStream {
            video_id: "isst1".to_string(),
            title: "ISS Earth View".to_string(),
            viewers: Some(1234),
            game: None,
            started_at: None,
            thumbnail_url: Some("https://i.ytimg.com/vi/isst1/hi.jpg".to_string()),
        };
        let ls = Livestream::from_youtube(&ch, &stream);
        assert!(ls.is_live);
        assert_eq!(ls.video_id.as_deref(), Some("isst1"));
        assert_eq!(ls.title.as_deref(), Some("ISS Earth View"));
        assert_eq!(ls.viewers, Some(1234));
        assert_eq!(ls.unique_key, "youtube:UCnasa:isst1");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests::from_youtube -- --nocapture`

Expected: compile error — `from_youtube` not defined.

- [ ] **Step 3: Implement**

In `src-tauri/src/channels.rs`, find the `from_kick` constructor (around line 84) and add a new method right after it:

```rust
    pub fn from_youtube(
        channel: &Channel,
        stream: &crate::platforms::youtube::YouTubeStream,
    ) -> Self {
        let mut ls = Self::offline_for(channel, None);
        ls.is_live = true;
        ls.title = Some(stream.title.clone()).filter(|s| !s.is_empty());
        ls.game = stream.game.clone();
        ls.viewers = stream.viewers;
        ls.started_at = stream.started_at;
        ls.thumbnail_url = stream.thumbnail_url.clone();
        ls.video_id = Some(stream.video_id.clone());
        ls.recompute_unique_key();
        ls
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests::from_youtube -- --nocapture`

Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/channels.rs
git commit -m "feat(channels): Livestream::from_youtube constructor

Builds a live YT Livestream with video_id populated and unique_key
recomputed to include the :{video_id} suffix. Mirrors the existing
from_twitch / from_kick / from_chaturbate constructor pattern."
```

---

## Task 10: `replace_livestreams_for_channel` + `youtube_miss_counts` + miss-threshold

**Files:**
- Modify: `src-tauri/src/channels.rs` — add field on `ChannelStore`, add merge primitive, refactor `snapshot()`

This is the densest task. Splits into smaller TDD cycles internally.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:

```rust
    fn live_yt_stream(channel: &Channel, video_id: &str) -> Livestream {
        Livestream::from_youtube(
            channel,
            &crate::platforms::youtube::YouTubeStream {
                video_id: video_id.to_string(),
                title: format!("Title {video_id}"),
                viewers: Some(100),
                game: None,
                started_at: None,
                thumbnail_url: None,
            },
        )
    }

    #[test]
    fn replace_livestreams_for_channel_inserts_initial_streams() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        let new_streams = vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")];
        store.replace_livestreams_for_channel(&ch.unique_key(), new_streams);

        assert_eq!(store.livestreams.len(), 2);
        assert!(store.livestreams.contains_key("youtube:UCnasa:v1"));
        assert!(store.livestreams.contains_key("youtube:UCnasa:v2"));
    }

    #[test]
    fn replace_livestreams_keeps_secondary_for_one_miss() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();

        // Cycle 1: v1, v2, v3 all live.
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2"), live_yt_stream(&ch, "v3")],
        );
        assert_eq!(store.livestreams.len(), 3);

        // Cycle 2: v3 missing — should still be present (miss = 1).
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        assert!(
            store.livestreams.contains_key("youtube:UCnasa:v3"),
            "v3 should survive 1 miss",
        );
        assert_eq!(store.youtube_miss_counts.get("youtube:UCnasa:v3"), Some(&1));
    }

    #[test]
    fn replace_livestreams_reaps_secondary_after_two_misses() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2"), live_yt_stream(&ch, "v3")],
        );

        // Two consecutive misses for v3 → reap.
        for _ in 0..2 {
            store.replace_livestreams_for_channel(
                &ch.unique_key(),
                vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
            );
        }
        assert!(!store.livestreams.contains_key("youtube:UCnasa:v3"));
        assert!(!store.youtube_miss_counts.contains_key("youtube:UCnasa:v3"));
    }

    #[test]
    fn replace_livestreams_resets_miss_count_when_stream_returns() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        // Miss v2.
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1")],
        );
        assert_eq!(store.youtube_miss_counts.get("youtube:UCnasa:v2"), Some(&1));
        // v2 returns.
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        assert!(store.youtube_miss_counts.get("youtube:UCnasa:v2").is_none());
    }

    #[test]
    fn replace_livestreams_offline_clears_all_secondary_streams_immediately() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        // Channel went fully offline — the new vec is just the offline placeholder.
        let offline = Livestream::offline_for(&ch, None);
        store.replace_livestreams_for_channel(&ch.unique_key(), vec![offline]);
        assert_eq!(store.livestreams.len(), 1);
        assert!(store.livestreams.contains_key("youtube:UCnasa"));
        assert!(!store.livestreams.contains_key("youtube:UCnasa:v1"));
        assert!(!store.livestreams.contains_key("youtube:UCnasa:v2"));
        assert!(store.youtube_miss_counts.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests::replace -- --nocapture`

Expected: compile errors — `youtube_miss_counts` field doesn't exist; `replace_livestreams_for_channel` doesn't exist.

- [ ] **Step 3: Add the field**

Find the `ChannelStore` struct (around line 162) and add:

```rust
pub struct ChannelStore {
    channels: Vec<Channel>,
    livestreams: HashMap<String, Livestream>,
    /// YouTube secondary-stream miss tolerance — counts consecutive
    /// refresh cycles where a stream key was expected but missing.
    /// Stream is reaped when the count hits YOUTUBE_MISS_THRESHOLD = 2.
    /// Avoids spurious offline events when /streams scrape transiently
    /// returns a partial list. Only applies to keys with a `:{video_id}`
    /// suffix (channel-level keys go through the immediate path).
    youtube_miss_counts: HashMap<String, u32>,
}
```

Update the two `Self { ... }` literals in `load()` (around lines 171 and 179) to include `youtube_miss_counts: HashMap::new()`:

```rust
        if !path.exists() {
            return Ok(Self {
                channels: Vec::new(),
                livestreams: HashMap::new(),
                youtube_miss_counts: HashMap::new(),
            });
        }
        // ...
        Ok(Self {
            channels: p.channels,
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        })
```

- [ ] **Step 4: Add the constant + the method**

Add a constant near the top of `src-tauri/src/channels.rs`:

```rust
/// Number of consecutive refresh cycles a YouTube secondary stream can
/// be missing from `/streams` before it's removed from the store. Avoids
/// flap-spam when a transient scrape failure drops one stream of a
/// multi-stream channel.
const YOUTUBE_MISS_THRESHOLD: u32 = 2;
```

Add this method to the `impl ChannelStore` block (after `upsert_livestream`):

```rust
    /// Atomic batch update of all livestreams associated with one channel.
    ///
    /// `channel_key` is the channel-level unique_key (no `:{video_id}` suffix).
    /// `new_streams` is the complete current set — pass an empty vec to mark
    /// the channel as offline (callers typically pass a single offline
    /// placeholder via `Livestream::offline_for` for that case).
    ///
    /// For YouTube channels with multiple streams returned over time, the
    /// `YOUTUBE_MISS_THRESHOLD` mechanism gives secondary streams (`key !=
    /// channel_key`) one cycle of grace — they survive 1 missing cycle,
    /// only get reaped on the 2nd consecutive miss. This avoids spurious
    /// offline events when YouTube's `/streams` scrape transiently returns
    /// a partial list.
    pub fn replace_livestreams_for_channel(
        &mut self,
        channel_key: &str,
        new_streams: Vec<Livestream>,
    ) {
        let new_keys: HashSet<String> = new_streams.iter().map(|s| s.unique_key.clone()).collect();
        let prefix = format!("{channel_key}:");

        // Determine if the channel is OFFLINE this cycle (only the bare
        // channel_key offline placeholder, no live streams).
        let channel_is_offline =
            new_streams.iter().all(|s| !s.is_live && s.unique_key == channel_key);

        // Walk existing entries belonging to this channel.
        let existing_keys: Vec<String> = self
            .livestreams
            .keys()
            .filter(|k| k.as_str() == channel_key || k.starts_with(&prefix))
            .cloned()
            .collect();

        for key in existing_keys {
            if new_keys.contains(&key) {
                // Will be overwritten below; reset miss counter just in case.
                self.youtube_miss_counts.remove(&key);
                continue;
            }
            // Missing this cycle.
            if key == channel_key {
                // Bare channel key (offline placeholder OR primary live key
                // for a single-stream channel). Always immediate-update.
                self.livestreams.remove(&key);
                self.youtube_miss_counts.remove(&key);
                continue;
            }
            // Secondary stream key (with :video_id suffix).
            if channel_is_offline {
                // Whole channel went dark — clear all secondaries immediately.
                self.livestreams.remove(&key);
                self.youtube_miss_counts.remove(&key);
            } else {
                // Channel still live; apply miss threshold.
                let n = self.youtube_miss_counts.entry(key.clone()).or_insert(0);
                *n += 1;
                if *n >= YOUTUBE_MISS_THRESHOLD {
                    self.livestreams.remove(&key);
                    self.youtube_miss_counts.remove(&key);
                }
            }
        }

        // Insert / overwrite the new streams.
        for ls in new_streams {
            self.livestreams.insert(ls.unique_key.clone(), ls);
        }
    }
```

You'll need to add `HashSet` to the use list at the top of the file:

```rust
use std::collections::{HashMap, HashSet};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests::replace -- --nocapture`

Expected: 5 tests pass.

- [ ] **Step 6: Refactor `snapshot()` to return multiple Livestreams per channel**

Find `snapshot()` (around line 241). Replace with:

```rust
    pub fn snapshot(&self) -> Vec<Livestream> {
        let mut out = Vec::new();
        for c in &self.channels {
            let channel_key = c.unique_key();
            let prefix = format!("{channel_key}:");
            let mut entries: Vec<&Livestream> = self
                .livestreams
                .values()
                .filter(|ls| ls.unique_key == channel_key || ls.unique_key.starts_with(&prefix))
                .collect();
            if entries.is_empty() {
                out.push(Livestream::offline_for(c, None));
                continue;
            }
            // Sort: live first, then by video_id for stable ordering.
            entries.sort_by(|a, b| {
                b.is_live.cmp(&a.is_live)
                    .then_with(|| a.video_id.cmp(&b.video_id))
                    .then_with(|| a.unique_key.cmp(&b.unique_key))
            });
            for ls in entries {
                out.push(ls.clone());
            }
        }
        out
    }
```

Add a snapshot test:

```rust
    #[test]
    fn snapshot_returns_one_offline_for_unrefreshed_channel() {
        let store = ChannelStore {
            channels: vec![test_channel(Platform::Twitch, "ninja")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let snap = store.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(!snap[0].is_live);
    }

    #[test]
    fn snapshot_returns_one_per_yt_concurrent_stream() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
    }
```

- [ ] **Step 7: Run all tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml channels::tests`

Expected: all tests pass (the new snapshot tests + the existing ones).

- [ ] **Step 8: Run the whole crate test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: passes (the broken refresh.rs from Task 8 was already in this state, but now `replace_livestreams_for_channel` exists so the call site compiles).

Actually it WILL still fail at the `from_youtube` import in refresh.rs if you didn't already merge that change. If `cargo check` errors here, look at what's missing and verify Tasks 8 and 9's edits all landed.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/channels.rs
git commit -m "feat(channels): replace_livestreams_for_channel + miss-threshold

Per-channel batch update primitive for the refresh pipeline. Replaces
the upsert_livestream loop for YouTube where one channel can produce
multiple Livestream entries.

- channel_key bare-key (offline placeholder OR single-stream primary):
  immediate update; old entry removed if not in new set.
- secondary keys (:video_id suffix) on a channel that is still live:
  YOUTUBE_MISS_THRESHOLD = 2. One cycle of grace; reap on second
  consecutive miss. Avoids flap when /streams scrape transiently
  returns a partial list.
- channel went fully offline: clear all secondary keys immediately,
  drop their miss counters too. The whole-channel-dark signal is more
  authoritative than the per-stream presence in /streams.

snapshot() refactored to return all Livestream entries per channel
(was: one entry per channel keyed by channel.unique_key()). Sorts
within a channel: live first, then by video_id, then unique_key —
stable ordering across refreshes."
```

---

## Task 11: Update IPC handlers to use `channel_key_of`

**Files:**
- Modify: `src-tauri/src/lib.rs` — `set_favorite` (line 114), `remove_channel` (line 109), and the `set_dont_notify` / `set_auto_play` handlers if they exist

- [ ] **Step 1: Locate and update each handler**

For each per-channel handler, prepend a `let unique_key = channels::channel_key_of(&unique_key).to_string();` near the top of the function body. Example for `set_favorite`:

```rust
#[tauri::command]
fn set_favorite(
    unique_key: String,
    favorite: bool,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let unique_key = channels::channel_key_of(&unique_key).to_string();
    state
        .store
        .lock()
        .set_favorite(&unique_key, favorite)
        .map_err(err_string)
}
```

Same pattern for `remove_channel`. If `set_dont_notify` / `set_auto_play` exist (search for them; they may or may not be wired yet), update those too.

- [ ] **Step 2: Verify the build**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: clean build.

- [ ] **Step 3: Verify all tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(ipc): route per-channel handlers through channel_key_of

set_favorite and remove_channel (plus set_dont_notify / set_auto_play
where present) now strip the :{video_id} suffix from the incoming
unique_key before calling into the store. Lets the React side use
the same key for every operation — per-stream calls (launch_stream,
chat_connect, embed_mount) use the full key as-is, per-channel calls
get the channel-level key automatically."
```

---

## Task 12: Update embed.rs to read `livestream.video_id` directly

**Files:**
- Modify: `src-tauri/src/embed.rs` — locate the `yt_video_id` thumbnail-URL parse + replace with field read

- [ ] **Step 1: Locate the existing parse**

Search the file for `yt_video_id`:

Run: `grep -nE "yt_video_id" src-tauri/src/embed.rs`

The current path likely calls `yt_video_id(&livestream.thumbnail_url.unwrap_or_default())` to extract the video id from the YouTube thumbnail URL pattern.

- [ ] **Step 2: Replace with field read + URL fallback**

Find the line and change it to read `livestream.video_id` first, falling back to the URL parse only if the field is None (defensive — for any livestream entry that hasn't been refreshed since the new field was added):

```rust
let video_id = livestream
    .video_id
    .clone()
    .or_else(|| livestream.thumbnail_url.as_deref().and_then(yt_video_id));
```

(Adjust the surrounding context to match what's there — the variable name and ownership may differ.)

- [ ] **Step 3: Build + test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`

Expected: passes.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "refactor(embed): read livestream.video_id directly

The new field on Livestream is the canonical source. Falls back to
the existing thumbnail-URL parse for any livestream entry that
predates the field landing — defensive only; the YT scraper now
populates the field on every refresh."
```

---

## Task 13: Update `open_in_browser` for per-video YT URL

**Files:**
- Modify: `src-tauri/src/lib.rs:821` — `open_in_browser` handler

- [ ] **Step 1: Locate the existing YouTube branch**

Read the function around line 821. The YouTube branch likely does something like:

```rust
Platform::Youtube => format!("https://www.youtube.com/channel/{}", channel.channel_id),
```

- [ ] **Step 2: Update to use video_id when present**

Change the YouTube branch to look up the Livestream first and prefer the per-video URL when a video_id is available:

```rust
Platform::Youtube => {
    let livestream = state
        .store
        .lock()
        .snapshot()
        .into_iter()
        .find(|ls| ls.unique_key == unique_key);
    if let Some(vid) = livestream.and_then(|ls| ls.video_id) {
        format!("https://www.youtube.com/watch?v={vid}")
    } else if channel.channel_id.starts_with("UC") && channel.channel_id.len() == 24 {
        format!("https://www.youtube.com/channel/{}/live", channel.channel_id)
    } else {
        format!("https://www.youtube.com/@{}/live", channel.channel_id)
    }
}
```

This handler currently takes `unique_key: String` — that's the stream-level key, which is what we want (matches the React side). The channel lookup at the top of the function should already use `channel_key_of` after Task 11; if not, add it:

```rust
let channel_key = channels::channel_key_of(&unique_key).to_string();
let channel = state.store.lock().channels().iter().find(|c| c.unique_key() == channel_key)...
```

- [ ] **Step 3: Build + test**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(ipc): per-video URL for YouTube open_in_browser

When the unique_key includes a video_id suffix and the corresponding
Livestream is in the snapshot, open youtube.com/watch?v={video_id}
so the user lands on the specific stream. Falls back to /live for
single-stream channels and offline channels (matches existing
behaviour). Mirrors Qt's per-stream open behaviour."
```

---

## Task 14: Manual integration smoke + tag the roadmap

- [ ] **Step 1: Manual smoke test**

Run `npm run tauri:dev` and verify:

1. **Single-stream YT channel** — add `youtube.com/@LinusTechTips` (or any single-stream channel known to be live). Refresh. Confirm one row.
2. **NASA-style** — add `youtube.com/@NASA` (or another channel known to broadcast multiple concurrent live streams). Refresh. Confirm 2+ rows with distinct titles.
3. **Per-row chat embed** — click each row's chat indicator (or the Command layout's main pane). Each embed should load a different `live_chat?v=` page.
4. **Per-row play** — right-click → Play on each. Each should launch its own stream via mpv.
5. **Per-row open in browser** — right-click → Open in Browser on each. Each should land on the specific `watch?v=` page, not the channel root.
6. **Channel-level favorite** — toggle favorite on one row. Restart the app. Favorite persists (the channel-level state should not be per-stream).
7. **Miss tolerance** — wait through 1-2 refresh cycles where one of the multi-stream channel's secondaries is briefly missing from the scrape. Confirm it persists for one cycle (state shows it as still live in your last-known view) and disappears on the second consecutive miss.

- [ ] **Step 2: Wait for PR review + merge, then tag the roadmap**

After the PR merges (let's call it `#N`), open `docs/ROADMAP.md`. Find the line:

```
- [ ] **YouTube multi-concurrent-stream channels** (NASA-style) — ...
```

Replace with:

```
- [x] **YouTube multi-concurrent-stream channels** (PR #N) — ...
```

Optionally rewrite the bullet to reflect what actually shipped. Per the project's `## Roadmap maintenance` rule (in `CLAUDE.md`), when this is the LAST unchecked item in Phase 2b, also update the Phase 2b header to ` ✓ shipped`.

Commit on a small follow-up docs branch:

```bash
git checkout main && git pull
git checkout -b docs/yt-multistream-shipped
# edit docs/ROADMAP.md
git add docs/ROADMAP.md
git commit -m "docs(roadmap): mark YT multi-concurrent-stream shipped (PR #N)"
git push -u origin docs/yt-multistream-shipped
gh pr create --title "docs: mark YT multi-stream shipped" --body "Per the roadmap-maintenance rule. Phase 2b is now complete."
```
