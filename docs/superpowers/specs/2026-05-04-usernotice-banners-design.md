# Chat Event Banners (USERNOTICE) — Design

**Date:** 2026-05-04
**Status:** approved (brainstorming complete)
**Goal:** promote in-stream chat-event rows (subs, gift bombs, raids, announcements) to a dismissible banner above the chat composer for the duration the user is most likely to notice them. Twitch + Kick parity.

## Background

Twitch broadcasts `USERNOTICE` messages on IRC for in-channel events: regular subscriptions (`msg-id=sub`), resubs (`resub`), gift subs (`subgift`), mass-gift "bombs" (`submysterygift`), incoming raids (`raid`), bits-badge tier-ups (`bitsbadgetier`), and mod announcements (`announcement`). Each message carries a Twitch-formatted human-readable string in the `system-msg` tag.

The current implementation (`src-tauri/src/chat/twitch.rs::build_usernotice`) parses these into a `ChatMessage` with `system: Some(SystemEvent { kind, text })`. The frontend renders them as in-stream `SystemRow` (`src/components/ChatView.jsx:1013`) — a colored left-border + glyph + heading line, with the user's optional attached message rendered below for resubs.

This works for a user actively reading chat, but **a user who just glanced away misses the event**. Roadmap line 97 (Phase 3) calls for "promoted DismissibleBanner at top of chat" — a temporary highlight pinned above the composer that surfaces the most recent event(s) without the user having to be watching the chat scroll.

Kick uses Pusher WebSockets and is not currently parsing any subscription/gift/host events at all (`kick.rs:238` says "Moderation + banner events land in Phase 3 follow-ups"). The Qt predecessor also doesn't handle them, so this design ships with a **research spike** to identify Kick's Pusher event names by capture during implementation.

### Roadmap entry

`docs/ROADMAP.md` line 97 (Phase 3): "`USERNOTICE` handling: sub/resub/raid/subgift/mystery-gift banners — promoted DismissibleBanner at top of chat".

## Goals

- Promote selected in-stream `SystemRow` events to a dismissible banner above the composer.
- User-configurable scope per event type (7 toggles on Twitch; Kick's set determined by spike).
- Default scope: `subgift` / `submysterygift` / `raid` only (the celebratory / rare events).
- Single-banner queue with 8 s auto-dismiss; finish-then-advance on new arrivals.
- Twitch + Kick parity for whatever Kick events the spike confirms.
- Per-channel, per-mount lifecycle: switching channels in Command/Focus drops the queue; Columns layout has independent queues per visible column.
- In-stream `SystemRow` is **unchanged** — it remains the durable record. Banner is purely additive.

## Non-goals

- Aggregate-coalescing logic (collapse 100 individual `subgift` events into one banner). Twitch's own `submysterygift` aggregate event is sufficient; per-gift banners drain naturally via the 8 s timer.
- Banner queue persistence across app restarts.
- Sound effects on banner display.
- Per-channel banner mute (master toggle + per-kind toggles is enough for v1).
- Banner reactions / clickability beyond the dismiss `×`. Raid-author profiles, gift-recipient lists, etc. remain accessible via the in-stream `SystemRow`.
- Mobile / touch-only behaviors.

## Module layout

```
src-tauri/src/
├── chat/
│   ├── twitch.rs          # build_usernotice already attaches SystemEvent — no changes (or minimal: verify the 7 kinds from the design's coverage list are populated correctly)
│   └── kick.rs            # NEW: subscribe to additional Pusher channel(s) — to be confirmed via spike. Parse sub/gift/host events into ChatMessage.system on the existing pipe.
└── settings.rs            # NEW: chat.event_banners: EventBannerSettings { enabled, kinds: EventBannerKinds }

src/
├── components/
│   ├── UserNoticeBanner.jsx   # NEW
│   ├── ChatView.jsx           # mounts <UserNoticeBanner> above composer, between SubAnniversaryBanner and Composer
│   └── PreferencesDialog.jsx  # NEW Chat-tab section "Event banners"
├── hooks/
│   └── useEventBanner.js      # NEW
└── tokens.css                 # NEW .rx-event-banner styles + per-kind data-attribute selectors
```

Net new: 1 Rust source area edit (`kick.rs`), 1 settings struct, 1 component, 1 hook, 1 CSS section, 1 Preferences section.

## Data flow

### Twitch (existing pipe)

```
Twitch IRC USERNOTICE
  → chat/twitch.rs::build_usernotice
  → ChatMessage { system: Some(SystemEvent { kind, text }), text: optional user-attached msg, ... }
  → app.emit("chat:message:{key}", msg)              [existing]
  → React listenEvent("chat:message:{key}", ...)
       ├─ useChat — appends to bufferRef → SystemRow renders in-stream    [existing]
       └─ useEventBanner — filters m.system && enabled && kinds[m.system.kind]:
              push BannerEvent onto queueRef
              if !current && queue: shift() → current; setTimeout(8000, advance)
              if current && new event arrives: append to queueRef; do NOT touch timer (finish-then-advance)
              advance(): clear timer; current = queue.shift() ?? null; if current: setTimeout(8000, advance)
              dismiss(): clear timer; advance() immediately
  → ChatView renders <UserNoticeBanner event={current} onDismiss={dismiss} />
```

### Kick (new, gated by spike)

```
Connect to Kick chat:
  ws.send(pusher:subscribe → chatrooms.{id}.v2)      [existing]
  ws.send(pusher:subscribe → channel.{id})           [NEW — exact channel name confirmed by spike]

handle_pusher_line:
  match event {
    "App\Events\ChatMessageEvent"          → existing chat path
    "App\Events\ChatroomUpdatedEvent"      → existing room-state path
    "App\Events\GiftedSubscriptionsEvent"  → NEW: build_kick_event(SUBGIFT_OR_MYSTERY, payload) → ChatMessage with .system populated
    "App\Events\SubscriptionEvent"         → NEW: build_kick_event(SUB_OR_RESUB, payload) → ChatMessage with .system populated
    "App\Events\StreamHostEvent"           → NEW: build_kick_event(RAID, payload) → ChatMessage with .system populated
    _ => silent skip
  }
  Each new event flows through the existing emit("chat:message:{key}", msg) — React side is identical to Twitch.
```

**Mapping note**: Kick events synthesize their own `system-msg`-equivalent text since Kick payloads don't include a pre-formatted human-readable string. Format mirrors Twitch's: `"{user} subscribed at Tier 1!"`, `"{user} gifted {n} subs to the community!"`, `"{user} hosted with {n} viewers"`.

### Settings flow

```
PreferencesDialog (Chat tab) → patch({ chat: { event_banners: { enabled, kinds: {...} } } })
  → usePreferences syncs to Rust via existing settings IPC
  → settings.rs persists to settings.json
  → useEventBanner reads from usePreferences().settings.chat.event_banners on every event
  → toggle changes are reactive: queue stops accepting filtered-out kinds at filter-time-of-arrival; a banner already on screen finishes its 8 s regardless of subsequent toggle changes
```

### Kick fallback path

- Kick connects with the additional subscribe; if events never arrive, queue stays empty — no UI signal, no crash, no log spam.
- If a payload arrives but doesn't parse cleanly, log at `warn!` level and skip — same robustness pattern as `parse_chatroom_modes` in the existing chat-mode-banners shipping path.
- If the spike finds zero events, the PR ships Twitch-only (with full settings + UI) and adds a follow-up roadmap entry "Kick event banners — confirm Pusher event names." The spec degrades gracefully; we do **not** hold up Twitch's ship on Kick research.

## Settings shape

### Rust (`src-tauri/src/settings.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSettings {
    // existing fields...
    #[serde(default)]
    pub event_banners: EventBannerSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBannerSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kinds: EventBannerKinds,
}

impl Default for EventBannerSettings {
    fn default() -> Self {
        Self { enabled: true, kinds: EventBannerKinds::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBannerKinds {
    #[serde(default)]                      pub sub: bool,
    #[serde(default)]                      pub resub: bool,
    #[serde(default = "default_true")]     pub subgift: bool,
    #[serde(default = "default_true")]     pub submysterygift: bool,
    #[serde(default = "default_true")]     pub raid: bool,
    #[serde(default)]                      pub bitsbadgetier: bool,
    #[serde(default)]                      pub announcement: bool,
}

impl Default for EventBannerKinds {
    fn default() -> Self {
        Self {
            sub: false, resub: false,
            subgift: true, submysterygift: true, raid: true,
            bitsbadgetier: false, announcement: false,
        }
    }
}

fn default_true() -> bool { true }
```

Per-field `serde(default)` is critical for forward-compat — adding an 8th kind in the future doesn't require migrating existing `settings.json` files. Existing users get the field's `Default` value automatically.

### Preferences UI

New `EventBannerSection` placed between the Spellcheck section and the existing chat-display toggles in the Chat tab. UI shape mirrors `SpellcheckSection`:

```
┌─────────────────────────────────────────────────────────────┐
│ Event banners                                                │
├─────────────────────────────────────────────────────────────┤
│ Show chat event banners                       [ON / off]    │  ← master toggle
│ Highlight subscriber events, gift bombs, raids, and          │
│ announcements above the chat composer.                       │
├─────────────────────────────────────────────────────────────┤
│   Show banner for:                                           │  ← chained-disable when master off
│   ☐  Subscriber alerts (new subs)                            │     greyed labels + disabled checkboxes
│   ☐  Resubscriber alerts                                     │
│   ☑  Gift subs                                               │
│   ☑  Mystery gift bombs                                      │
│   ☑  Raids and hosts                                         │
│   ☐  Bits badge tier-ups                                     │
│   ☐  Mod announcements                                       │
└─────────────────────────────────────────────────────────────┘
```

- Master toggle `event_banners.enabled` ⇄ `settings.chat.event_banners.enabled`
- Each checkbox ⇄ `settings.chat.event_banners.kinds.{sub|resub|subgift|...}`
- Hint text under master toggle when on: "Highlight subscriber events, gift bombs, raids, and announcements above the chat composer."
- Hint text under master toggle when off: "Banners disabled. In-stream rows still appear in chat."
- Patch helper: `patch({ chat: { event_banners: { kinds: { subgift: true } } } })` — usePreferences supports nested patches.

`useEventBanner` reads `settings?.chat?.event_banners` defensively (default to `enabled=true` with C kinds if shape is missing — catches the case where settings.json predates this PR).

## Hook + component contracts

### `src/hooks/useEventBanner.js`

```js
/**
 * Owns the per-channel UserNotice banner queue and 8 s auto-dismiss timer.
 * Subscribes to chat:message:{channelKey}, filters m.system events against
 * settings.chat.event_banners, queues them FIFO, advances on timer or
 * manual dismiss.
 *
 * Returns { current: BannerEvent | null, dismiss: () => void }.
 *
 * BannerEvent = {
 *   id: string,           // ChatMessage.id, used as React key
 *   kind: string,         // 'sub' | 'resub' | 'subgift' | ... — drives palette + glyph
 *   text: string,         // m.system.text — primary banner copy
 *   userText: string,     // m.text — optional user-attached resub message
 *   emoteRanges: [...],   // m.emote_ranges — for rendering user-attached msg
 *   linkRanges: [...],    // m.link_ranges
 *   timestamp: string,    // m.timestamp ISO
 *   channelKey: string,
 * }
 */
export function useEventBanner(channelKey) { ... }
```

Internal state:
- `current: BannerEvent | null` (`useState`)
- `queueRef: BannerEvent[]` (`useRef` so adding to queue doesn't re-render)
- `timerRef: number | null` (`setTimeout` handle)

Lifecycle:
- Mount: subscribe to `chat:message:{channelKey}` via `listenEvent`. On each message: if `m.system && enabled && kinds[m.system.kind]`, push to `queueRef`. If `current` is null, advance immediately. Cleanup unlistens, clears timer, drops queue.
- Channel switch: hook re-runs because `channelKey` changed; cleanup drains queue and timer; fresh subscribe starts. Confirms the "drop queue on channel switch" decision.
- **Master toggle flips off** while a banner is up: clear `queueRef`, clear the current banner immediately, clear the timer. Implemented as a `useEffect` watching `enabled`. User flipping the master switch off should mean "stop now, all of it."
- **Per-kind toggle flips off** while a banner is up: queue stops accepting new events of that kind on filter-at-arrival. The current banner (if of the just-disabled kind) finishes its 8 s; previously-queued events of that kind still play out. Less surgical than clearing the queue, but per-kind toggles communicate "stop preferring this type going forward," not "purge whatever's in flight."

Pure helper (exported for test):

```js
export function shouldQueue(message, settings) {
  if (!message?.system?.kind) return false;
  if (!settings?.enabled) return false;
  return settings.kinds?.[message.system.kind] === true;
}
```

### `src/components/UserNoticeBanner.jsx`

```jsx
/**
 * Pinned-above-composer banner for chat events (subs, gifts, raids,
 * announcements). One slot — driven by useEventBanner's current event.
 *
 * Props:
 *   event: BannerEvent (must be non-null when rendered)
 *   onDismiss: () => void  // user clicked × — advances queue immediately
 */
export function UserNoticeBanner({ event, onDismiss }) {
  // <div class="rx-event-banner" data-kind={event.kind} role="status">
  //   <span class="rx-event-banner__glyph" aria-hidden>{glyphFor(kind)}</span>
  //   <div class="rx-event-banner__text">
  //     <strong>{event.text}</strong>
  //     {event.userText && <span class="rx-event-banner__user">
  //       <EmoteText text={event.userText} ranges={event.emoteRanges} links={event.linkRanges} size={20} />
  //     </span>}
  //   </div>
  //   <button onClick={onDismiss} aria-label="Dismiss event banner">×</button>
  // </div>
}
```

Glyph map (parallels `SystemRow`):

```js
const GLYPHS = {
  sub: '★', resub: '★', subgift: '★', submysterygift: '★',
  raid: '⤴', announcement: '✦', bitsbadgetier: '✦',
};
```

### `src/tokens.css` additions

```css
.rx-event-banner {
  display: flex; align-items: center; gap: 10px;
  padding: 6px 14px;
  border-top: var(--hair);
  border-left: 2px solid var(--zinc-700);
  background: rgba(255,255,255,.03);
  font-size: var(--t-12);
  line-height: 1.4;
}
.rx-event-banner[data-kind="raid"]                                    { border-left-color: #fb923c; }
.rx-event-banner[data-kind="raid"] .rx-event-banner__glyph            { color: #fb923c; }
.rx-event-banner[data-kind="sub"],
.rx-event-banner[data-kind="resub"],
.rx-event-banner[data-kind="subgift"],
.rx-event-banner[data-kind="submysterygift"]                          { border-left-color: #a78bfa; }
.rx-event-banner[data-kind="sub"] .rx-event-banner__glyph,
.rx-event-banner[data-kind="resub"] .rx-event-banner__glyph,
.rx-event-banner[data-kind="subgift"] .rx-event-banner__glyph,
.rx-event-banner[data-kind="submysterygift"] .rx-event-banner__glyph  { color: #a78bfa; }
.rx-event-banner[data-kind="announcement"]                            { border-left-color: #4ade80; }
.rx-event-banner[data-kind="announcement"] .rx-event-banner__glyph    { color: #4ade80; }
.rx-event-banner[data-kind="bitsbadgetier"]                           { border-left-color: #fbbf24; }
.rx-event-banner[data-kind="bitsbadgetier"] .rx-event-banner__glyph   { color: #fbbf24; }
```

Palette is identical to `SystemRow` (`ChatView.jsx:1018`) so the in-stream row and banner look unified.

### `ChatView.jsx` mounting

```jsx
const { current: bannerEvent, dismiss: dismissBanner } = useEventBanner(channelKey);

{platform === 'chaturbate' && <ChaturbateAuthBanner />}
<ChatModeBanner channelKey={channelKey} variant={variant} />
<SubAnniversaryBanner ... />
<TwitchWebConnectPrompt ... />
{bannerEvent && <UserNoticeBanner event={bannerEvent} onDismiss={dismissBanner} />}
<Composer ... />
```

Banner ordering rationale: **`UserNoticeBanner` closest to the composer** because it's the most ephemeral and the most attention-grabbing; the other banners are persistent state (chat modes, anniversary share, auth prompts) that should hold their position above.

## Testing

### Rust unit tests

- `chat::twitch::build_usernotice` — fixture-based tests asserting `system: Some(SystemEvent { kind, text })` is attached for each of the 7 msg-ids. Pure-fn, no network.
- `chat::kick::build_event` (new) — fixture Pusher payloads for `GiftedSubscriptionsEvent`, `SubscriptionEvent`, `StreamHostEvent` (or whatever the spike confirms). Assert `ChatMessage.system` correctly populated and `ChatMessage.text` extracted from payload (gift-bomb count, raider login, etc.).
- `chat::kick::handle_pusher_line` — fixture for one of each new event type going through the full match arm. Asserts the right downstream call shape (build_event invoked with right payload).
- `settings::EventBannerSettings::default()` — assert C defaults (`subgift / submysterygift / raid` true; rest false; `enabled` true).
- `settings::EventBannerSettings` deserialize-from-empty — JSON `{}` deserializes to defaults via the `serde(default)` field attributes (forward-compat regression guard).

### Frontend dev-asserts

`useEventBanner.js` exports a pure helper `shouldQueue(message, settings)` and tests it via DEV asserts at module load (matching the `commandTabs.js` / `autocorrect.js` pattern):

```js
if (process.env.NODE_ENV !== 'production') {
  console.assert(shouldQueue({ system: { kind: 'subgift' } }, { enabled: true,  kinds: { subgift: true  } }) === true);
  console.assert(shouldQueue({ system: { kind: 'subgift' } }, { enabled: false, kinds: { subgift: true  } }) === false);
  console.assert(shouldQueue({ system: { kind: 'subgift' } }, { enabled: true,  kinds: { subgift: false } }) === false);
  console.assert(shouldQueue({ system: null }, { enabled: true, kinds: { subgift: true } }) === false);
  console.assert(shouldQueue({ system: { kind: 'something_new' } }, { enabled: true, kinds: {} }) === false);
}
```

The queue + timer machinery is exercised manually during dev (see "Manual UI verification"). React Testing Library was deemed overkill for this size in prior PRs (#118, #126).

### Manual UI verification (golden path)

1. Set Preferences → Chat → Event banners → Enabled, all 7 kinds checked.
2. Connect to a Twitch channel currently receiving subs (e.g. xqc, summit1g — high-volume).
3. Verify: each USERNOTICE row in chat **also** flashes a banner above the composer for ~8 s.
4. Toggle a kind off mid-stream — events of that kind stop appearing as banners (in-stream row continues).
5. Toggle master off — current banner finishes, no new banners appear.
6. Switch channels in Command — banner disappears with channel; new channel starts fresh.
7. Stress-test: connect during a 50-sub gift bomb — verify `submysterygift` shows first, ~3 individual `subgift`s queue behind, no UI hang.

### Kick spike — implementation Task 1

- Pick a Kick channel known to have active subs/gifts/hosts (community streamer; researcher-of-record handles confirming a candidate during implementation).
- Connect via `kick.rs` modified to subscribe to candidate Pusher channel(s) — try `channel.{id}` first, fall back to `channel_{id}` and any other variants observed.
- Log every received `event` field at `info!` level — include the full data payload.
- Run for a 30–60 minute window during a peak time. Document observed event names + payload shapes inline as code comments in `build_event` / via fixture files in tests.
- If subscription/host events don't fire on any subscribed channel → either (a) they're auth-only, in which case Kick parity stays empty until we add auth context, or (b) they're on a channel name we haven't tried.

**Decision boundary**: Kick parity is best-effort, not blocking. If the spike finds zero events, the PR ships Twitch-only and adds a follow-up roadmap entry "Kick event banners — confirm Pusher event names."

## Risks / open questions

- **Kick Pusher events may be auth-only.** The spike could discover that anonymous Pusher subscribers don't receive sub/gift/host events. Mitigation: ship Twitch-only; document the finding; queue a follow-up that wires Kick event banners through the existing Kick OAuth context.
- **Kick payload schemas may change.** Kick has historically iterated their internal Pusher schema without versioning. The graceful-skip behavior means a future schema change manifests as "the banner just stops firing on Kick" — debuggable via `warn!` logs but invisible to the user. Acceptable for v1.
- **Burst with master-off mid-burst.** Resolved in the lifecycle spec — flipping master off clears the queue, clears the current banner, clears the timer in a single `useEffect`.
- **`announcement` color collision with `ok` status.** Both green. Considered alternative palettes; settled on green for both because they're semantically related ("good news") and never co-occur in a banner queue.
