---
title: YouTube multi-concurrent-stream support
date: 2026-04-26
phase: 2b
status: design
---

# YouTube multi-concurrent-stream support

## Background

Closes the last unshipped item in Phase 2b. Some YouTube channels broadcast 2+ simultaneous live streams from one channel id (NASA Space Station's ISS Earth-view + Mission Control + Crew Cam, news outlets running parallel feeds, event channels). Today our YouTube refresh assumes one Livestream per Channel, so secondary streams are invisible.

Goal: parity with the Qt app's behaviour (`api/youtube.py::_get_all_concurrent_streams` + `_fetch_concurrent_live_video_ids` + `_find_landscape_alternative`):

- **Same content in multiple aspect ratios** (e.g., a primary landscape feed + an auto-generated portrait Shorts variant of the same content) → render only the landscape one. The Qt heuristic that triggers this is "primary returned a portrait stream from `/live`" — when YouTube's `/live` URL prefers a Shorts livestream because both exist, Qt swaps to the landscape alternative from `/streams`. We match this asymmetric behaviour exactly.
- **Genuinely different streams** (NASA-style: distinct content per video) → render one row per video. Each row shows the per-video title for disambiguation.

Out of scope: switching primary detection from `yt-dlp` to HTML scraping (Qt does both; we keep `yt-dlp` for the primary because it works and handles cookie-gated streams cleanly), and any UI grouping (rows are flat, matching Qt).

## What ships

1. New scraping logic in `platforms/youtube.rs`: `/streams` page parser + `/watch?v=` player-response parser + portrait detection
2. `Livestream.video_id: Option<String>` field
3. `Livestream.unique_key()` gains the `:{video_id}` suffix when `video_id` is `Some` (matches Qt's `stream_key`)
4. `channels.rs::channel_key_of` helper that strips the suffix for per-channel operations
5. `refresh.rs` flattens `Vec<YouTubeStream>` per channel into the store + applies `YOUTUBE_MISS_THRESHOLD = 2` to secondary streams
6. IPC handlers split into per-stream (use full key) vs per-channel (strip first via helper)
7. `embed.rs` reads `livestream.video_id` directly when constructing the `live_chat?v=` URL (replaces the current thumbnail-URL parse fallback)

## Architecture

### Module evolution: `platforms/youtube.rs`

Today: thin wrapper around `yt-dlp --dump-single-json --no-download` returning `Result<Option<YouTubeLive>>` with at most one stream.

After: orchestrator that combines three sources:

| Source | Method | Purpose |
|---|---|---|
| `yt-dlp /live` (existing) | subprocess | Primary stream metadata + `is_live` |
| `youtube.com/channel/{id}/streams` (new) | HTTP GET | List of all live video IDs on the channel |
| `youtube.com/watch?v={id}` (new) | HTTP GET | `ytInitialPlayerResponse` for portrait check + per-video metadata |

Public API change:

```rust
// Old
pub async fn fetch_live(channel_id: &str, cookies_browser: Option<&str>)
    -> Result<Option<YouTubeLive>>;

// New
pub async fn fetch_live(channel_id: &str, cookies_browser: Option<&str>, http: &reqwest::Client)
    -> Result<YouTubeLive>;

pub struct YouTubeLive {
    pub channel_id: String,
    pub display_name: String,
    pub streams: Vec<YouTubeStream>,  // empty = offline
}
```

`YouTubeStream` keeps its existing fields (`video_id`, `title`, `viewers`, `started_at`, `thumbnail_url`).

### Per-channel orchestration

```
fetch_live(channel_id, cookies, http) -> Vec<YouTubeStream>:
    primary = fetch_primary_via_ytdlp(channel_id, cookies)  # existing
    if primary is None or not live:
        return []                    # offline → empty vec

    live_ids = scrape_concurrent_video_ids(channel_id, http)

    if live_ids.is_empty() or live_ids.len() == 1:
        # No swap target available — either scrape failed or there's
        # only the primary to begin with. Return primary as-is; skip
        # the orientation check since we have nothing to swap with.
        return [primary]

    # Portrait dedupe on primary — only worth the /watch scrape on
    # primary when there's at least one alternative to swap to.
    primary_resolved = primary
    if is_portrait_stream_via_watch(primary.video_id, http):
        if landscape := find_landscape_alternative(channel_id, primary.video_id, http):
            primary_resolved = landscape

    # Multi-stream: append other concurrent streams as their own entries.
    results = [primary_resolved]
    for vid in live_ids:
        if vid == primary_resolved.video_id:
            continue
        if let Ok(stream) = scrape_player_response(vid, http):
            results.push(stream)

    return results
```

The early-return on `live_ids.len() <= 1` is the important optimisation. Without it, every refresh of every live YT channel would do an extra `/watch?v=` scrape just to learn the primary's orientation — even when there's no swap alternative. With the early return, the typical single-stream YT channel costs `1 yt-dlp + 1 /streams scrape` per refresh; only multi-stream channels pay for the orientation probe.

`find_landscape_alternative(channel_id, current_video_id, http)`:
- Re-scrape `/streams` (cheap; a few hundred ms)
- For each candidate `vid != current_video_id`, fetch `/watch?v=vid`'s player response
- Return the first non-portrait one
- Returns `None` if no landscape alternative found

`is_portrait_stream(stream_or_player_response)`:
- Read `streamingData.adaptiveFormats[0].width/height` (or `formats[0]` as fallback)
- Return `width < height`
- Returns `false` on missing fields

For yt-dlp-derived streams, we don't have direct access to `streamingData`. Going with one extra `/watch?v=` scrape on the primary's video_id — but only when `live_ids.len() > 1` (see orchestration above). For single-stream channels (the common case) we skip the scrape entirely. Alternative would be `yt-dlp --print '%(formats.0.width)s,%(formats.0.height)s'` but that fights yt-dlp's flag surface and fails for live streams that don't yet have a fully-resolved manifest; the conditional /watch scrape is simpler and pays nothing in the common case.

### HTTP client + headers

The existing `AppState.http: reqwest::Client` is reused. Both scrapes send these headers:

```rust
const SCRAPE_HEADERS: &[(&str, &str)] = &[
    ("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                    (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36"),
    ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    ("Accept-Language", "en-US,en;q=0.9"),
];
```

Match Qt's `SCRAPE_HEADERS` to land on the same DOM YouTube serves Qt. Since our HTTP client (not WebKit) does the scrape, we don't have the WebKit-vs-Chrome fingerprint mismatch problem we hit with Cloudflare on Chaturbate.

### Cookie reuse

Anonymous-friendly for public channels, which is the common case. We do **not** pipe Google session cookies into the HTML scrapes in v1 — saves the complexity of mixing cookie sources, matches what Qt does for these specific endpoints (Qt only sends cookies to its yt-dlp call). Member-only / sub-only multi-streams aren't a real-world thing.

### Concurrency

Existing `YT_CONCURRENCY = 5` semaphore in `refresh.rs` wraps the entire `fetch_live` call. Multi-stream channels do their internal `/streams` + N×`/watch` fetches inside that single permit — back-pressure preserved, no risk of fanning out and triggering YouTube rate limits.

### Data model: `Livestream` + `unique_key`

```rust
// channels.rs
pub struct Livestream {
    // existing fields ...
    pub video_id: Option<String>,   // populated for live YT only
}

impl Livestream {
    pub fn unique_key(&self) -> String {
        let base = format!("{}:{}", self.channel.platform, self.channel.channel_id);
        if matches!(self.channel.platform, Platform::Youtube) {
            if let Some(vid) = &self.video_id {
                return format!("{base}:{vid}");
            }
        }
        base
    }
}
```

`Channel.unique_key()` is unchanged. Other platforms unchanged.

### IPC routing helper

```rust
// channels.rs
/// Given a stream-level unique_key (which may include a `:{video_id}`
/// suffix for live YouTube), return the channel-level unique_key.
/// For non-YT platforms and offline YT channels, returns the input
/// slice unchanged.
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

### IPC handler split

| Handler | Key type | Routing |
|---|---|---|
| `launch_stream` | stream_key | Direct lookup of Livestream; uses `livestream.video_id` for the URL |
| `chat_connect` | stream_key | Direct lookup; embed uses `livestream.video_id` |
| `chat_disconnect` | stream_key | Direct lookup |
| `embed_mount` / `embed_unmount` / `embed_position` | stream_key | Direct lookup |
| `open_in_browser` | stream_key | YT: `watch?v={video_id}`; others: existing URL |
| `set_favorite` | channel_key | `let key = channel_key_of(&input); ...` |
| `remove_channel` | channel_key | Same |
| `set_dont_notify` | channel_key | Same |
| `set_auto_play` | channel_key | Same |

### Refresh + miss-threshold

`refresh.rs`'s YouTube path:

1. Run `fetch_live` per channel under the existing semaphore
2. Each call returns `Vec<YouTubeStream>` (empty = offline)
3. Convert each `YouTubeStream` → `Livestream` (carrying `video_id` and `channel`)
4. Hand the flattened `Vec<Livestream>` to the store-merge logic

Miss-threshold (matches Qt's `monitor.py::_youtube_miss_counts`, threshold = 2). State lives on `ChannelStore` alongside the existing `livestreams` map (refresh is stateless across calls; the store is the natural persistence point):

```rust
// channels.rs additions
pub struct ChannelStore {
    // existing fields ...
    youtube_miss_counts: HashMap<String, u32>,  // stream_key → consecutive misses
}
```

Both fields stay behind the same `Arc<Mutex<...>>` lock so the merge step is atomic.

Per refresh cycle, after merging the new state in:

```
for each YT channel that was refreshed this cycle:
    received = set of stream_keys returned
    
    for each existing stream in store under this channel where stream.unique_key() != channel.unique_key():
        if received.contains(stream.unique_key()):
            miss_counts.remove(stream.unique_key())
        else:
            n = miss_counts.entry(stream.unique_key()).or_insert(0); *n += 1
            if *n >= 2:
                store.remove(stream.unique_key())
                miss_counts.remove(stream.unique_key())
                // emit offline event for this stream
```

Threshold only applies to **secondary** streams (those with `:{video_id}` suffix). The primary (channel-key) stream uses the existing immediate-update behaviour — yt-dlp is authoritative for "is the channel currently live."

### `embed.rs` cleanup

Today `embed.rs::mount` extracts the YT video_id from the livestream's `thumbnail_url` via `yt_video_id()`. With the new `Livestream.video_id` field, replace this with a direct read; keep the URL parse as a fallback only for old-format livestreams that haven't refreshed yet (defensive).

### Frontend

No component changes. The list rendering already iterates per Livestream and uses `unique_key` as the React key. NASA's 3 streams just render as 3 rows with their per-video titles. The chat embed's `useChat(channelKey)` call already passes the livestream's unique_key — works for video-id keys without modification.

## Data flow

### Single-stream YT channel (the common case)

```
refresh tick
  → fetch_live(channel_id, cookies, http)
      → primary = yt-dlp /live
      → live_ids = scrape /streams  → ["video_xyz"]
      → primary.is_portrait? → no (typical)
      → return [primary]
  → flatten into store
  → unique_key: youtube:UC123:video_xyz
```

### NASA-style channel (3 simultaneous landscape streams)

```
refresh tick
  → fetch_live(channel_id, cookies, http)
      → primary = yt-dlp /live  → first stream the /live URL resolved to
      → live_ids = scrape /streams  → ["v1", "v2", "v3"]
      → primary is landscape, no swap
      → for v in [v2, v3] (excluding primary v1):
            scrape_player_response(v) → YouTubeStream
      → return [primary, v2_stream, v3_stream]
  → flatten into store, 3 unique_keys:
      youtube:UC456:v1
      youtube:UC456:v2
      youtube:UC456:v3
  → React renders 3 rows
```

### Channel that broadcasts landscape + auto-Shorts portrait variant

```
refresh tick
  → fetch_live(channel_id, cookies, http)
      → primary = yt-dlp /live  → portrait Shorts livestream (YT favours it)
      → live_ids = scrape /streams  → ["portrait_v", "landscape_v"]
      → primary.is_portrait? → yes (scrape primary's /watch to confirm)
      → find_landscape_alternative
            scrape_player_response(landscape_v) → not portrait → return it
      → primary_resolved = landscape_v
      → live_ids.len() == 2; for v in [portrait_v] (skipping primary_resolved):
            // portrait_v WOULD be added here as a separate row
            // This matches Qt's behaviour: the dedupe only swaps the
            // primary, it does NOT filter portraits from the multi-list.
            // In practice this is fine — most channels have ONLY the
            // primary (auto-Shorts pair) and no extra concurrent streams.
            scrape_player_response(portrait_v) → add as second row
      → return [landscape_v, portrait_v]
```

Worth flagging: if a channel has BOTH (a) an auto-Shorts portrait pair AND (b) an actual third stream, the user will see the portrait Shorts as a row. This matches Qt's behaviour exactly. If it becomes a real annoyance in practice we can add a "skip portraits in multi-stream branch when primary already has a landscape variant" filter; not in v1.

### Miss tolerance: secondary stream momentarily missing

```
cycle 1: /streams returns [v1, v2, v3]   → store has v1, v2, v3
cycle 2: /streams returns [v1, v2]       → miss_counts[v3] = 1; v3 stays in store
cycle 3: /streams returns [v1, v2, v3]   → miss_counts[v3] removed; reset
cycle 4: /streams returns [v1, v2]       → miss_counts[v3] = 1
cycle 5: /streams returns [v1, v2]       → miss_counts[v3] = 2 → REAP v3
```

## Error handling

| Failure | Handling |
|---|---|
| `yt-dlp` primary fails (non-zero exit, not the "not live" sentinel) | Bubble up — same as today |
| `/streams` scrape returns 0 video IDs (rate limit, parser drift, network 5xx) | Treat as scrape failure → fall back to primary-only. No miss-counter increment (we can't increment what we never saw). |
| `/watch?v=` scrape on a specific video fails | Skip that video, log `warn`, continue with the others |
| Network timeout on either scrape | Same as scrape failure |
| Parser hits unexpected JSON shape (YouTube DOM changed) | Log `warn` with first 500 chars of raw HTML, return empty result |
| `is_portrait_stream` can't find `streamingData.adaptiveFormats` | Returns `false` (assume landscape, no swap) |
| Both primary and secondaries land at the same `video_id` (deduplication needed) | The "skip if vid == primary_resolved.video_id" check in the orchestration handles it |

The "tolerate failures, prefer underreporting to overreporting" stance matches Qt — better to miss one of NASA's 3 streams for a cycle than to spam offline events.

## Testing

### Unit tests

- **`scrape_streams_page` parser** — capture two real fixtures (single-stream channel, NASA-style 3-stream channel) under `src-tauri/tests/fixtures/yt/`. Assert the parser returns the expected video IDs for each.
- **`scrape_player_response` parser** — fixture with portrait stream + fixture with landscape stream. Assert metadata extraction (title, viewers, video_id, thumbnail) and orientation classification.
- **`is_portrait_stream`** — synthetic JSON dicts: portrait (300×600), landscape (1920×1080), missing dimensions, empty `streamingData`. Verify each.
- **`channel_key_of`** — round-trips:
    - `"twitch:foo"` → `"twitch:foo"`
    - `"kick:bar"` → `"kick:bar"`
    - `"chaturbate:baz"` → `"chaturbate:baz"`
    - `"youtube:UC123"` → `"youtube:UC123"`
    - `"youtube:UC123:abc_xyz"` → `"youtube:UC123"`
    - `"youtube:@handle"` → `"youtube:@handle"`
    - `"youtube:@handle:vid"` → `"youtube:@handle"`
- **Miss-threshold** — feed synthetic refresh cycles to the merge logic, assert a stream survives 1 miss and dies on 2; assert a present-stream's counter resets.

### Integration test

None for the live HTTP scrapes — too flaky against real youtube.com. Manual smoke instead.

### Manual smoke (PR test plan)

- Add `youtube.com/@NASA` (or another known multi-stream channel). Refresh. Confirm 2-3 rows appear with distinct titles.
- Add a single-stream channel. Refresh. Confirm one row.
- Add a known-portrait-Shorts channel (find one). Refresh. Confirm we render the landscape variant, not the portrait.
- Click Play on each row of the multi-stream channel — each should launch its own video.
- Click "Open Chat" on each row of the multi-stream channel — each embed shows its own `live_chat?v=` page.
- Wait through one or two refresh cycles where one secondary stream goes offline. Confirm it persists in the list for one cycle (miss = 1) before disappearing on the second cycle (miss = 2).

## Out of scope (this phase)

- Switching primary detection from yt-dlp to HTML scraping (Qt does both; we keep yt-dlp)
- UI grouping / "expand to see all streams" interaction (Qt is flat, we match)
- Cookie injection into the `/streams` and `/watch` scrapes (member-only multi-streams aren't a real-world thing)
- Smarter portrait dedupe in the multi-stream branch (would filter portraits when primary already has a landscape variant; matches Qt's gap exactly, defer to follow-up if it bites)
