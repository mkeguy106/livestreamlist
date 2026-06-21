# Fix: Chaturbate refresh rate-limiting (online channels flicker offline)

**Date:** 2026-06-21
**Branch:** `fix/chaturbate-refresh-rate-limit`

## Root cause (confirmed)

`refresh.rs::fetch_chaturbate_all` fires one `/api/chatvideocontext/{user}/`
request **per channel, all at once** (`join_all`, no concurrency cap). With 106
imported CB follows that's 106 simultaneous requests every 60 s → Chaturbate's
Cloudflare returns **HTTP 429** ("Just a moment…"). Live logs showed 29× 429
across all 106 channels. On a 429 the channel is dropped from `cb_map`, and the
merge falls back to `offline_for` → online channels flicker offline.

Qt avoids this with a two-tier strategy: a **bulk** `/api/ts/roomlist/room-list/
?follow=true` call (authenticated, 1–2 requests) finds all online followed rooms;
per-channel checks run only on the small online subset. ~2–22 requests/cycle vs
our 106.

## Fix (Qt parity)

Rewrite the CB refresh path into a two-tier fetch returning both determined
states and an "uncertain" set:

```rust
struct CbResult {
    live: HashMap<String, ChaturbateLive>, // online followed rooms (or per-channel results)
    errored: HashSet<String>,              // couldn't determine (429/network) — preserve last state
}
```

1. **Primary — bulk (when a captured `sessionid` exists):**
   `chaturbate::fetch_followed_online(client, sessionid)` calls
   `/api/ts/roomlist/room-list/?follow=true&limit=90&offset=N` with
   `Cookie: sessionid=…`, paginates, filters rows to `is_following == true`, and
   builds `username → ChaturbateLive` (status from `current_show`, viewers from
   `num_users`, title from `room_subject`, thumb from `img`). Channels absent
   from the map are **definitively offline**. ~2–3 requests/cycle.
   - **Anonymous guard:** if the first page is a large list (`total_count > 500`)
     with zero `is_following` rows, the session is invalid → return `Err` so we
     fall through (don't treat the public roomlist as "your follows").
2. **Fallback — per-channel, capped (no sessionid, or bulk failed):**
   `/api/chatvideocontext/{user}/` but with `buffer_unordered(CB_CONCURRENCY=4)`
   instead of unbounded, so we never burst. `Ok(Some)` → `live`; `Ok(None)` →
   offline (absent); `Err` (429/network) → `errored`.
3. **Defense in depth — preserve last-known state:** in the merge, a CB channel
   in `errored` keeps its previous snapshot state instead of flipping offline, so
   a transient rate-limit/network blip never flickers the row. If the entire bulk
   call fails *and* fallback also errors a channel, the row holds its last state.

The captured `sessionid` is the same one `auth::chaturbate::stored_session_cookie()`
provides for the follows-import (so the bulk path "just works" after sign-in).

## Out of scope

- 429 exponential-backoff retry inside the HTTP client (the bulk path removes the
  volume that caused 429; capped fallback + preserve-state cover the rest).
- Private-show detection fidelity in the bulk path depends on `current_show` in
  the roomlist rows; if the bulk feed omits non-public rooms, those show offline
  (Qt has the same limitation). The per-channel fallback remains fully
  authoritative for `room_status`.

## Testing

- Rust unit tests (pure) for bulk row parsing → `ChaturbateLive` and for the
  anonymous-guard predicate (large list + no follows → invalid).
- Manual: with 106 CB follows + signed in, confirm logs show ~2 bulk requests
  per cycle, zero 429, and online channels stay online across cycles.
