# The "Focus + Chaturbate black window" — root cause and fix

**Date**: 2026-07-11 · **Fixed in**: `feat/focus-redesign` (ChatView dispatcher split)
**Symptom**: entire window goes black; app process healthy; refresh loop keeps
running; zero log output; unrecoverable without relaunch. Three real incidents
(2026-07-10): cold boot into Focus with a high-viewer live CB channel as
auto-feature; selecting CB in Command then switching to Focus; vite reload over
a page with a CB embed.

## TL;DR

It was never Chaturbate, never the embeds, never WebKit. **`ChatView` had a
platform early-return that changed its React hook count between renders.**
Focus renders one unkeyed `<ChatView channelKey={featured}>`, so switching the
featured channel between an embed platform (YT/CB — 2 hooks) and an IRC
platform (Twitch/Kick — dozens of hooks) re-rendered the *same component
instance* with a different hook count. React 18 throws
`Rendered more hooks than during the previous render` and **unmounts the
entire root**. An empty DOM over the dark body = "black window". The error
only ever went to the WebKit console, which is invisible in `tauri:dev` —
hence "zero log output". Fix: `ChatView` is now a zero-hook dispatcher over
two real components (`EmbedChatView` / `IrcChatView`); a platform change
swaps the component *type* (clean remount), and each component's hook count
is stable.

## Why it looked like a WebKit/embed bug

- Black window + process healthy + silent — pattern-matched the repo's real
  WebKitGTK history (dmabuf black window, profile-dir crashes).
- It correlated perfectly with CB channels because CB channels top this
  user's live-viewer list, so Focus's auto-feature (base code) put CB in the
  featured slot, and *leaving* that slot (auto-flip to the default selection,
  or any manual/programmatic switch to a Twitch channel) was the killer
  transition.
- Command was immune **structurally**: its chat tabs keep one ChatView mounted
  per channel (switching tabs flips `active`, never unmounts or re-purposes
  an instance across platforms), so the hook count never changed.
- The long-known "mock-mode ChatView hooks-order error on Focus→YouTube tab"
  (progress notes, 2026-07-08 round, "check in real tauri:dev") was this same
  bug — visible in the browser console in mock mode, invisible in the app.

## Diagnosis trail (what was ruled out, with evidence)

Oracle: X-window-id capture (`import -window` on the single Xwayland client,
never active-window capture) + grayscale mean. Calibration: painted Command
≈ 0.042–0.14, painted Focus with content 0.12–0.27, black ≈ 0.036 (constant
to 7 digits — a frozen frame never varies). Programmatic driver: temporary
dev-only effect polling `/lsl-diag.json` and executing
`setSelectedKey`/`setLayoutId` step sequences — no input injection.

| Experiment | Result |
|---|---|
| E2a/E2b: Command CB select → Focus flip, dwell 500–3000 ms (destroy embed mid-load) | painted, 7/7 |
| E3: cold boot into Focus ×11 (lastChannel legacy key is dead — default selection landed on NASA/YT) | painted, but CB never actually featured |
| E4/seq41: select CB then Twitch 50 ms later in Focus | **black** — and JS provably dead (driver stopped) |
| runA/runD: fresh CB embed, loaded 8–9 s, single destroy | **black** at the destroy transition |
| runE: NASA (YT) → Twitch, CB never involved | **black** — not CB-specific |
| V2/V1: defer embed destroy 3 s / park at about:blank first | **still black** — teardown timing irrelevant |
| **V0: leak the child entirely (unmount = no GTK/WebKit op at all)** | **still black** — Rust/WebKit teardown is NOT the trigger |
| runC3: CB → *offline* Twitch (no video mount) | **black** — mpv not required |
| Autopsy of wedged state | app main thread: healthy `gtk_main_iteration_do` poll; React web process alive **with JSC threads**, 0 CPU; no coredumps; no `web-process-terminated` events; minimize/restore + resize don't recover |
| Module-scope `window.onerror` + DOM heartbeat (survives root unmount) | **smoking gun**: `Rendered more hooks than during the previous render` + heartbeat `dom=1670 → dom=15` |

The heartbeat continuing while React's driver interval died proved the page
was alive and the React root was gone — everything that read as "WebKit
stopped painting" was an empty DOM.

## The fix

`src/components/ChatView.jsx`: the default export is now a dispatcher that
calls **no hooks** and returns `<EmbedChatView>` (YT/CB) or `<IrcChatView>`
(everything else). The old in-component early return between the two hook
sets is gone. No call-site changes; no behavioral change for either path.

Verification (all previously-fatal shapes, zero `window.onerror`, DOM stable,
driver alive, painted with frame-to-frame variation):
- CB featured → live Twitch (mpv mounts): previously 100% fatal → clean
- Churn: CB→Twitch @50 ms, CB→YT @400 ms, →offline Twitch: clean
- Cold boot into Focus ×3: clean

## Leads noted for later (not this branch's scope)

- `EmbedLayer.jsx`'s webview mount path lacks the mpv path's
  "slot unregistered while mount in flight" guard: a mount resolving after
  its slot unregistered leaves a zombie webview in `mountedKeys` with no
  registry entry (observed at every boot-into-Focus on base code). Harmless
  to painting, but it leaks a webview + network process until some later
  slot adopts the key. Worth the same `registry.has(key)` check the mpv
  branch has.
- `livestreamlist.lastChannel` is a dead (legacy, migrated) localStorage key
  — boot restores selection via `command.tabs`/default-selection only.
- Dev-loop kills (SIGKILL, watcher restarts) orphan streamlink children —
  known watch item, reconfirmed here; `pkill -f "player-external-http"`
  cleans.
