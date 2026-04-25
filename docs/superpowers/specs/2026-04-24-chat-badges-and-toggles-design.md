# Chat user badges + visibility toggles

**Status:** Design
**Date:** 2026-04-24
**Scope:** Twitch + Kick chat. Restores user-badge rendering (parity with the Qt app) and adds three Preferences toggles for badges, mod badges, and timestamps.

## Goals

1. Resolve and render user badges in Twitch + Kick chat messages, including the local user's own badges on their own sent messages.
2. Add three independent visibility toggles in Preferences → Chat:
   - `show_badges` — cosmetic badges (subscriber, founder, premium, turbo, partner, bits, sub-gifter, hype-train, …). Default `true`.
   - `show_mod_badges` — mod-authority badges (broadcaster, moderator, vip, staff, admin, global_mod). Default `true`.
   - `show_timestamps` — message timestamps in chat rows. Default `true`.
3. As a prerequisite for own-badges on Twitch, add local echo for own Twitch messages (today they don't appear in chat at all — IRC doesn't echo, and there is no local-echo path).

## Non-goals

- Authenticated badge endpoints (Helix). The public anonymous endpoints (`badges.twitch.tv/v1/badges/...`) cover the same data and avoid an OAuth dependency.
- Per-chat overflow menu / right-click toggles. Preferences only for v1.
- Backoff or retry on badge-endpoint failures. Fail-quiet matches existing emote-cache behavior.
- YouTube / Chaturbate badges (those layouts use embedded webviews with their own native chat).
- Cache eviction. The cache is small enough that we keep it for the process lifetime.

## Architecture

### Backend (Rust)

A single `BadgeCache`, owned by `ChatManager` as `Arc<BadgeCache>`, mirrors the existing `EmoteCache` pattern.

```
src-tauri/src/chat/
├── badges.rs              # NEW: BadgeCache, Scope, JSON shapes, classify_mod, OwnBadges
├── models.rs              # ChatBadge gains `is_mod: bool`
├── twitch.rs              # parse_badges sets is_mod; resolve at emit; USERSTATE → OwnBadges; local echo on send
├── kick.rs                # ensure_channel on connect; resolve URLs from cache; is_mod from type
└── mod.rs                 # ChatManager::new constructs BadgeCache, threads Arc into task configs
```

**`BadgeCache` shape**

- Map keyed by `(Platform, Scope, badge_id) → BadgeUrl` where `Scope ∈ { Global, Channel(String) }`.
- Lookup tries `Channel(room)` first, then `Global`.
- Loaders are idempotent: `ensure_global(Platform)` and `ensure_channel(Platform, room_or_slug)` no-op if the scope is already populated.
- Per-channel `OwnBadges` map (separate from the URL cache) holds the local user's `badges=` list per Twitch channel, populated from `USERSTATE`.

**Twitch endpoints (anonymous, no auth):**

- Global: `GET https://badges.twitch.tv/v1/badges/global/display`
- Channel: `GET https://badges.twitch.tv/v1/badges/channels/{room_id}/display`
- JSON path: `badge_sets.{set_name}.versions.{version}.image_url_4x` → store as `(set_name/version) → url`.

**Kick:**

- System badges (broadcaster, moderator, vip, staff) seeded statically into the cache by `ensure_global(Kick)`. URLs are the same Kick CDN paths the Qt app already uses (e.g. `kickBadges/broadcaster.svg`).
- Subscriber badges fetched per channel via Kick's chatroom endpoint, mirroring `livestream.list.qt/src/livestream_list/chat/connections/kick.py:408-437`. Stored under `Channel(slug)` scope.
- Some Kick badges already arrive inline in the message payload (`badge.image.src`); when present, prefer that over the cache.

**Mod classification (`is_mod: bool` on `ChatBadge`)**

- Twitch: `broadcaster | moderator | vip | staff | admin | global_mod` → `true`. Everything else → `false`.
- Kick: `broadcaster | moderator | vip | staff` → `true`. Everything else (`subscriber`, `og`, `founder`, `sub_gifter`) → `false`.
- Stored on the badge so the frontend filter logic doesn't need to duplicate the set.

**Twitch own-message local echo (new)**

Today `chat_send` queues an IRC PRIVMSG and returns. IRC doesn't echo own PRIVMSGs, so own messages never appear in chat. To support own-badges on Twitch we add local echo:

1. After the per-channel outbound task confirms a successful WebSocket write, it constructs a synthetic `ChatMessage`:
   - `text` = the just-sent body
   - `user.login` / `user.display_name` = cached identity from the `auth` module
   - `user.badges` = current `OwnBadges` for this channel (from the most recent `USERSTATE`)
   - `timestamp` = `chrono::Utc::now()`
   - `id` = locally generated (e.g. `format!("self-{}", uuid)`) — kept distinct so future de-dup logic can identify it
2. URLs resolve through the same `resolve_badges` step as incoming messages.
3. Sent through `persist_and_emit` so logging, block-list filtering, and event emission share a single path.

Kick already echoes own messages via Pusher with `sender.identity.badges` populated, so no special path is needed there.

### Frontend (React)

**New component**

`src/components/UserBadges.jsx` — small, ~25 lines:

```jsx
function UserBadges({ badges, showCosmetic, showMod, size = 14 }) {
  const filtered = (badges ?? []).filter(b =>
    (b.is_mod ? showMod : showCosmetic) && b.url
  );
  if (filtered.length === 0) return null;
  return (
    <span style={{ display: 'inline-flex', gap: 2, marginRight: 4, verticalAlign: 'middle' }}>
      {filtered.map(b => (
        <img key={b.id} src={b.url} alt="" title={b.title}
             width={size} height={size} style={{ display: 'block' }} />
      ))}
    </span>
  );
}
```

**Modified components**

- `src/components/ChatView.jsx` — IrcRow + CompactRow:
  - Read `showBadges`, `showModBadges`, `showTimestamps` from the existing settings hook (same one that exposes `timestamp_24h`).
  - Wrap timestamp render in `showTimestamps && (...)`.
  - Insert `<UserBadges badges={m.user.badges} showCosmetic={showBadges} showMod={showModBadges} />` immediately before the username.
- `src/components/PreferencesDialog.jsx` — three new `<Toggle>` rows in the Chat section, matching the existing pattern (`c.show_badges !== false` to honor "default true" before the user has saved settings).

### Settings

`src-tauri/src/settings.rs::ChatSettings` gains:

```rust
pub show_badges: bool,           // default true
pub show_mod_badges: bool,       // default true
pub show_timestamps: bool,       // default true
```

All three use `#[serde(default = "default_true")]` so existing `settings.json` files load without migration.

## Data flow

**App start**

1. Settings load — three new fields default to `true` if missing.
2. `ChatManager::new` constructs an empty `BadgeCache`.

**First chat connect to a Twitch channel**

3. Per-channel task fires `BadgeCache::ensure_global(Twitch)` (idempotent).
4. WebSocket connects, IRC handshake, JOIN.
5. First `ROOMSTATE` carries `room-id`; task fires `BadgeCache::ensure_channel(Twitch, room_id)`.
6. `USERSTATE` arrives — new handler writes `badges=` into the per-channel `OwnBadges` map.

**Incoming PRIVMSG**

7. `build_privmsg` → `parse_badges` returns `Vec<ChatBadge>` with `is_mod` set, `url` empty.
8. `resolve_badges(cache, room_id, &mut badges)` stamps URLs from the cache (channel scope first, then global). Misses leave `url` empty.
9. `persist_and_emit` (unchanged path) — block-list, log append, event emit.

**Outgoing Twitch message (new local echo)**

10. Composer → `chat_send` → `send_raw` queues IRC PRIVMSG.
11. After the WebSocket write succeeds, the task builds a synthetic `ChatMessage` from cached identity + `OwnBadges[channel]`.
12. URLs resolved by `resolve_badges`.
13. `persist_and_emit` — same path as incoming.

**Kick connect + messages**

14. On connect, `ensure_global(Kick)` (seeds system badges) + `ensure_channel(Kick, slug)` (fetches sub badges) fire in parallel with the Pusher subscription.
15. PRIVMSG-equivalent extracts `sender.identity.badges`, runs `resolve_badges` with the inline-image fallback. Own messages echo through Pusher and follow the same path.

**Frontend render**

16. IrcRow / CompactRow read settings from the hook.
17. Timestamp gated on `showTimestamps`.
18. `<UserBadges>` filters by `is_mod` and `url`, renders `<img>` 14×14 per badge.

**Toggle change in Preferences**

19. `<Toggle>` `onChange` → `saveSettings({ chat: { show_badges: v, ... } })` → settings hook re-emits → consumers re-render. No re-fetch.

## Error handling

| Failure | Behavior |
|---|---|
| Badge endpoint 4xx/5xx/network fail | Log `warn`, scope stays empty, badges render text-only. Same posture as `EmoteCache` for 7TV/BTTV/FFZ outages. |
| Malformed JSON from badge endpoint | `serde` error logged; that scope stays empty; other scopes unaffected. |
| Channel has no custom badge sets | Empty `badge_sets` deserializes fine; only globals contribute. |
| `USERSTATE` arrives without `badges=` | `OwnBadges` stays empty; own messages render with no badges. |
| Twitch send succeeds but local echo construction fails | Log `warn`, skip the echo. Send was successful — strictly better than today's "send vanishes." |
| `settings.json` missing new fields | `serde(default = "default_true")` returns `true`. No migration. |
| `settings.json` write fails | Existing UI surface for save errors is unchanged. |
| Unknown badge `set/version` (new Twitch badge type) | Cache miss → `url` empty → `<UserBadges>` skips. Forward-compatible. |
| Race: PRIVMSG arrives before badge fetch completes | `url` empty for affected badges → skipped by render. Self-corrects within ~1 s once the fetch resolves; subsequent messages render fully. |

## Testing

**Rust unit tests (extend `cargo test`):**

- `badges::test_parse_twitch_global_response` — JSON fixture → assert `lookup(Twitch, Global, "broadcaster/1")` returns expected URL.
- `badges::test_parse_twitch_channel_response` — same with channel scope.
- `badges::test_classify_mod_twitch` — broadcaster/moderator/vip/staff/admin/global_mod → `is_mod=true`; subscriber/turbo/partner/founder/bits → `is_mod=false`.
- `badges::test_classify_mod_kick` — broadcaster/moderator/vip/staff → `is_mod=true`; subscriber/og/founder/sub_gifter → `is_mod=false`.
- `badges::test_kick_static_seed` — after `ensure_global(Kick)`, system badge IDs are present in the cache.
- `chat::twitch::test_userstate_extracts_own_badges` — feed a `USERSTATE` IRC line, assert `OwnBadges` is populated.
- `chat::twitch::test_resolve_badges_channel_overrides_global` — same id present in both scopes → channel URL wins.

**Manual / dev-loop checklist (run under `npm run tauri:dev` before merging):**

- Connect to a Partner channel with custom subscriber badges → channel-specific badges appear.
- Connect to a small channel with no custom badges → global mod/broadcaster badges still appear.
- Toggle `show_badges` off → cosmetic badges vanish, mod badges remain (if `show_mod_badges` on). Toggle each independently.
- Toggle `show_timestamps` off → timestamps vanish in IrcRow + CompactRow.
- Send a Twitch message while logged in → appears locally with own badges.
- Send a Kick message → appears via Pusher echo with own badges.
- Disconnect/reconnect chat → badges still render from cache, no refetch.

**Out of scope for testing:** load under thousands of concurrent channels; cache eviction; rate-limit handling on the badge endpoints (undocumented; if it bites us in production we add backoff then).
