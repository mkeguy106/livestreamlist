# Richer Twitch User Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface three more facts on the chat user card — session message count, explicit account-creation date, and Partner/Affiliate status — to approach Qt parity.

**Architecture:** Frontend-only. A new pure helper counts a user's messages in the current `ChatView` buffer; the count is threaded through the existing username open/hover handlers into the card. Account date and Partner/Affiliate come from profile fields we already fetch (`created_at`, `broadcaster_type`).

**Tech Stack:** React 18, plain CSS variables. No Rust, no new IPC.

## Global Constraints

- All data is already client-side — **no backend / IPC / API changes**.
- Session message count is a **snapshot at card-open** (does not live-update while open); per-channel; bounded by `useChat`'s 250-message buffer.
- Pure helpers live in `src/utils/format.js` and are covered by module-scope DEV asserts (the project's convention — see the existing `userChannelUrl` asserts at the bottom of that file). `format.js` has **no imports** and is safe to import in Node for testing.
- Account row shows **date · age** combined: e.g. `21 Jun 2015 · 9y 2mo`. Dates format with `Intl.DateTimeFormat('en-GB', { day:'numeric', month:'short', year:'numeric', timeZone:'UTC' })` (day-first, UTC so the calendar day is stable).
- Partner/Affiliate renders as a **small header chip** (twitch-accent pill), only when `broadcaster_type` is `"partner"` or `"affiliate"`.
- Never use native `title=""` for hover text — use the themed `<Tooltip>` (not needed by this plan, but the rule stands).
- No AI/Claude references in commit messages.

---

### Task 1: Pure helpers — `formatDate` + `countSessionMessages`

Add two pure functions to `src/utils/format.js` plus DEV asserts, verified directly in Node.

**Files:**
- Modify: `src/utils/format.js`

**Interfaces:**
- Produces:
  - `export function formatDate(iso: string): string` — `"21 Jun 2015"`; `""` for empty/invalid.
  - `export function countSessionMessages(messages: array, user: object): number`

- [ ] **Step 1: Write the failing test (Node check)**

Run this exact command — it imports the not-yet-exported functions and must FAIL:

```bash
node --input-type=module -e "
import { formatDate, countSessionMessages } from './src/utils/format.js';
import assert from 'node:assert';
assert.equal(formatDate('2015-06-21T08:00:00Z'), '21 Jun 2015');
assert.equal(formatDate(''), '');
assert.equal(formatDate('not-a-date'), '');
const M = (id, login) => ({ user: { id, login } });
assert.equal(countSessionMessages([M('1','a'), M('1','a'), M('2','b')], { id:'1', login:'a' }), 2);
assert.equal(countSessionMessages([M(null,'Abc'), M(null,'abc')], { login:'abc' }), 2);
assert.equal(countSessionMessages([{}, { text:'x' }, M('1','a')], { id:'1', login:'a' }), 1);
assert.equal(countSessionMessages([M('2','b')], { id:'1', login:'a' }), 0);
assert.equal(countSessionMessages(null, { id:'1' }), 0);
console.log('OK');
"
```

Expected: FAIL — `SyntaxError: The requested module './src/utils/format.js' does not provide an export named 'countSessionMessages'` (and `formatDate`).

- [ ] **Step 2: Add the two functions**

In `src/utils/format.js`, insert these two functions immediately above the `// ── Module-scope DEV asserts` comment line (currently line 63):

```js
/**
 * Format an RFC3339 / ISO timestamp as a short, unambiguous date: "21 Jun
 * 2015". Day-first and UTC-pinned so the rendered calendar day matches the
 * timestamp's instant regardless of the viewer's timezone. Returns "" on
 * empty/unparseable input.
 */
export function formatDate(iso) {
  if (!iso) return '';
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return '';
  return new Intl.DateTimeFormat('en-GB', {
    day: 'numeric',
    month: 'short',
    year: 'numeric',
    timeZone: 'UTC',
  }).format(new Date(ms));
}

/**
 * Count how many of `messages` were sent by `user`. Matches on `user.id` when
 * both the message author and `user` have an id; otherwise falls back to a
 * case-insensitive `login` match. Rows without an author (system rows) are
 * skipped. Used for the user card's "session messages" stat — a snapshot of
 * the current channel buffer.
 */
export function countSessionMessages(messages, user) {
  if (!Array.isArray(messages) || !user) return 0;
  const id = user.id;
  const login = user.login ? user.login.toLowerCase() : null;
  let n = 0;
  for (const m of messages) {
    const u = m?.user;
    if (!u) continue;
    if (id && u.id) {
      if (u.id === id) n += 1;
    } else if (login && u.login) {
      if (u.login.toLowerCase() === login) n += 1;
    }
  }
  return n;
}
```

- [ ] **Step 3: Add DEV asserts**

In `src/utils/format.js`, inside the existing `if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {` block (currently ending at line 84), add these lines just before its closing `}`:

```js
  // formatDate — UTC-pinned short date.
  console.assert(formatDate('2015-06-21T08:00:00Z') === '21 Jun 2015', 'formatDate basic');
  console.assert(formatDate('') === '', 'formatDate empty');
  console.assert(formatDate('not-a-date') === '', 'formatDate invalid');
  // countSessionMessages — id match, login fallback, skip system rows.
  {
    const M = (id, login) => ({ user: { id, login } });
    console.assert(
      countSessionMessages([M('1', 'a'), M('1', 'a'), M('2', 'b')], { id: '1', login: 'a' }) === 2,
      'countSessionMessages by id',
    );
    console.assert(
      countSessionMessages([M(null, 'Abc'), M(null, 'abc')], { login: 'abc' }) === 2,
      'countSessionMessages by login (case-insensitive)',
    );
    console.assert(
      countSessionMessages([{}, { text: 'x' }, M('1', 'a')], { id: '1', login: 'a' }) === 1,
      'countSessionMessages skips system rows',
    );
    console.assert(
      countSessionMessages(null, { id: '1' }) === 0,
      'countSessionMessages null messages',
    );
  }
```

- [ ] **Step 4: Run the Node check to verify it passes**

Run the same command from Step 1.
Expected: prints `OK` and exits 0.

- [ ] **Step 5: Verify the build still compiles**

Run: `npm run build`
Expected: succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/utils/format.js
git commit -m "feat(user-card): add formatDate + countSessionMessages helpers"
```

---

### Task 2: Wire the count through + render the three additions

Thread the session count from `ChatView` → `App` → `useUserCard` → `UserCard`, and render the count, the combined Account date·age row, and the Partner/Affiliate chip.

**Files:**
- Modify: `src/components/ChatView.jsx`
- Modify: `src/App.jsx`
- Modify: `src/hooks/useUserCard.js`
- Modify: `src/components/UserCard.jsx`

**Interfaces:**
- Consumes: `countSessionMessages`, `formatDate` (Task 1).
- Produces: card renders `sessionMessageCount`; `useUserCard.openFor(user, channelKey, anchor, extras)` stores `extras.sessionMessageCount`.

- [ ] **Step 1: ChatView — import the helper**

In `src/components/ChatView.jsx`, add `countSessionMessages` to the import from `../utils/format.js`. If there's no existing `format.js` import, add one near the other component imports (after line 4's `useChat` import):

```js
import { countSessionMessages } from '../utils/format.js';
```

(If a `from '../utils/format.js'` import already exists, add `countSessionMessages` to its named list instead of a second import.)

- [ ] **Step 2: ChatView — compute the count in the open + hover wrappers**

In `src/components/ChatView.jsx`, replace the `handleOpen` and `handleHover` callbacks (currently lines 277-288):

```js
  const handleOpen = useCallback(
    (user, rect) => onUsernameOpen?.(user, rect, channelKey),
    [onUsernameOpen, channelKey],
  );
  const handleContext = useCallback(
    (user, point) => onUsernameContext?.(user, point, channelKey),
    [onUsernameContext, channelKey],
  );
  const handleHover = useCallback(
    (user, rect) => onUsernameHover?.(user, rect, channelKey),
    [onUsernameHover, channelKey],
  );
```

with:

```js
  const handleOpen = useCallback(
    (user, rect) =>
      onUsernameOpen?.(user, rect, channelKey, countSessionMessages(messages, user)),
    [onUsernameOpen, channelKey, messages],
  );
  const handleContext = useCallback(
    (user, point) => onUsernameContext?.(user, point, channelKey),
    [onUsernameContext, channelKey],
  );
  const handleHover = useCallback(
    (user, rect) =>
      onUsernameHover?.(user, rect, channelKey, user ? countSessionMessages(messages, user) : 0),
    [onUsernameHover, channelKey, messages],
  );
```

(`messages` is already in scope from `useChat` at line 86; `handleContext` is unchanged.)

- [ ] **Step 3: App — forward the count in `onUsernameOpen`**

In `src/App.jsx`, replace the `onUsernameOpen` callback (currently lines 86-95):

```js
  const onUsernameOpen = useCallback(
    (user, rect, channelKey) => {
      lockedByClick.current = true;
      // Cancel any pending hover-open / hover-close timers — the click wins.
      if (hoverTimer.current) clearTimeout(hoverTimer.current);
      if (closeTimer.current) clearTimeout(closeTimer.current);
      cardOpenFor(user, channelKey, rect);
    },
    [cardOpenFor],
  );
```

with:

```js
  const onUsernameOpen = useCallback(
    (user, rect, channelKey, sessionMessageCount) => {
      lockedByClick.current = true;
      // Cancel any pending hover-open / hover-close timers — the click wins.
      if (hoverTimer.current) clearTimeout(hoverTimer.current);
      if (closeTimer.current) clearTimeout(closeTimer.current);
      cardOpenFor(user, channelKey, rect, { sessionMessageCount });
    },
    [cardOpenFor],
  );
```

- [ ] **Step 4: App — forward the count in `onUsernameHover`**

In `src/App.jsx`, replace the `onUsernameHover` callback (currently lines 120-147). Change the signature to accept `sessionMessageCount` and pass it into the deferred `cardOpenFor`:

```js
  const onUsernameHover = useCallback(
    (user, rect, channelKey, sessionMessageCount) => {
      if (!hoverEnabled) return;
      // While a click-opened card is showing, ignore all hover signals so the
      // card doesn't yoink to a different user when chat scrolls or the cursor
      // drifts onto another name.
      if (lockedByClick.current) return;
      if (user) {
        // entering an anchor
        overAnchor.current = true;
        if (hoverTimer.current) clearTimeout(hoverTimer.current);
        if (closeTimer.current) clearTimeout(closeTimer.current);
        hoverTimer.current = setTimeout(() => {
          if (overAnchor.current) cardOpenFor(user, channelKey, rect, { sessionMessageCount });
        }, hoverDelay);
      } else {
        // leaving the anchor
        overAnchor.current = false;
        if (hoverTimer.current) clearTimeout(hoverTimer.current);
        // Small delay so the cursor can move into the card before we close it.
        if (closeTimer.current) clearTimeout(closeTimer.current);
        closeTimer.current = setTimeout(() => {
          if (!overAnchor.current && !overCard.current) cardClose();
        }, 100);
      }
    },
    [hoverEnabled, hoverDelay, cardOpenFor, cardClose],
  );
```

- [ ] **Step 5: App — pass the count to `<UserCard>`**

In `src/App.jsx`, the `<UserCard ... />` element (around line 366) — add the prop alongside the existing ones (e.g. right after `profileError={card.profileError}`):

```jsx
        sessionMessageCount={card.sessionMessageCount}
```

- [ ] **Step 6: useUserCard — accept + store the count**

In `src/hooks/useUserCard.js`:

(a) Add `sessionMessageCount: null,` to the initial `useState` object (currently lines 7-16), after `profileError: null,`.

(b) Replace the `openFor` signature + its `setState` (currently lines 20-31):

```js
  const openFor = useCallback(async (user, channelKey, anchor) => {
    const myInstance = ++instanceRef.current;
    setState({
      open: true,
      anchor,
      user,
      channelKey,
      metadata: null,
      profile: null,
      profileLoading: !!user.id,
      profileError: null,
    });
```

with:

```js
  const openFor = useCallback(async (user, channelKey, anchor, extras) => {
    const myInstance = ++instanceRef.current;
    setState({
      open: true,
      anchor,
      user,
      channelKey,
      metadata: null,
      profile: null,
      profileLoading: !!user.id,
      profileError: null,
      sessionMessageCount: extras?.sessionMessageCount ?? null,
    });
```

(The later `setState(s => ({ ...s, metadata }))` / `{ ...s, profile }` handlers spread `...s`, so `sessionMessageCount` is preserved as the profile/metadata resolve.)

- [ ] **Step 7: UserCard — accept the prop and feed `<Stats>`**

In `src/components/UserCard.jsx`:

(a) Add `sessionMessageCount,` to the props destructure (after `onCardHover,` on line 25).

(b) Replace the dead placeholder (line 127):

```jsx
          sessionMessageCount={undefined /* wired by parent via prop in Task 18 */}
```

with:

```jsx
          sessionMessageCount={sessionMessageCount}
```

- [ ] **Step 8: UserCard — combined Account date · age row**

In `src/components/UserCard.jsx`, add `formatDate` to the imports (the file imports `readableColor` from `../utils/color.js`; add a new import line after it):

```js
import { formatDate } from '../utils/format.js';
```

Then in the `Stats` function, replace the account-age row (line 217):

```jsx
    if (profile.created_at) rows.push(<Row key="ca" label="Account age" value={formatAge(profile.created_at)} />);
```

with:

```jsx
    if (profile.created_at)
      rows.push(
        <Row
          key="ca"
          label="Account"
          value={`${formatDate(profile.created_at)} · ${formatAge(profile.created_at)}`}
        />,
      );
```

(`formatAge` remains the local helper at the bottom of the file.)

- [ ] **Step 9: UserCard — Partner/Affiliate chip in the header**

In `src/components/UserCard.jsx`, pass the broadcaster type into `<Header>` (in the card body, line 104-111) by adding this prop to the `<Header ... />`:

```jsx
        broadcasterType={profile?.broadcaster_type}
```

Then update the `Header` function signature (line 170) to accept it:

```jsx
function Header({ display, login, nameColor, avatar, platformLetter, badges = [], broadcasterType }) {
```

And render the chip inside the header's right column — insert it immediately after the `@{login}` block (after line 190, before the badges block):

```jsx
        {broadcasterType === 'partner' || broadcasterType === 'affiliate' ? (
          <span
            style={{
              display: 'inline-block',
              marginTop: 4,
              font: '600 9px var(--font-mono)',
              letterSpacing: '.04em',
              textTransform: 'uppercase',
              color: 'var(--twitch)',
              background: 'rgba(167,139,250,.12)',
              border: '1px solid rgba(167,139,250,.22)',
              borderRadius: 4,
              padding: '1px 6px',
            }}
          >
            {broadcasterType === 'partner' ? 'Partner' : 'Affiliate'}
          </span>
        ) : null}
```

- [ ] **Step 10: Verify the build**

Run: `npm run build`
Expected: succeeds, no errors.

- [ ] **Step 11: Commit**

```bash
git add src/components/ChatView.jsx src/App.jsx src/hooks/useUserCard.js src/components/UserCard.jsx
git commit -m "feat(user-card): session msg count, account date, partner/affiliate"
```

---

### Task 3: Final verification + roadmap

**Files:**
- Modify: `docs/ROADMAP.md` (add a checked entry)

- [ ] **Step 1: Full gate**

Run: `npm run build`
Expected: succeeds.
Run: `cargo test --manifest-path src-tauri/Cargo.toml` (no Rust changed; confirm still green)
Expected: PASS (250).
Run the Task 1 Node check once more:

```bash
node --input-type=module -e "
import { formatDate, countSessionMessages } from './src/utils/format.js';
import assert from 'node:assert';
assert.equal(formatDate('2015-06-21T08:00:00Z'), '21 Jun 2015');
const M = (id, login) => ({ user: { id, login } });
assert.equal(countSessionMessages([M('1','a'), M('1','a')], { id:'1', login:'a' }), 2);
console.log('OK');
"
```
Expected: `OK`.

- [ ] **Step 2: Roadmap**

In `docs/ROADMAP.md`, add a checked bullet under the **Phase 3 follow-ups — UX consistency** section (the same area as the "User card "Open channel"" entry), with the PR number filled in once known:

```markdown
- [x] **Richer user card** (PR #N) — surfaced three more facts on the chat user card toward Qt parity: session message count (was a dead placeholder; now counts the user's messages in the current channel buffer, snapshot at open), explicit account-creation date alongside the existing age ("21 Jun 2015 · 9y 2mo"), and a Partner/Affiliate header chip from `broadcaster_type`. Frontend-only — all data was already client-side. Pure helpers `formatDate` + `countSessionMessages` (DEV-asserted). Spec/plan: `docs/superpowers/{specs,plans}/2026-06-21-richer-user-card{-design,}.md`.
```

```bash
git add docs/ROADMAP.md
git commit -m "docs(roadmap): note richer user card"
```

(If shipping via the standard "ship it" flow, the roadmap mark normally lands in the follow-up docs PR with the real PR number — coordinate with that flow rather than committing a placeholder `#N`.)

---

## Notes for the implementer

- `formatAge` already exists locally in `UserCard.jsx` — do not move or duplicate it; only `formatDate` is imported from `format.js`.
- The session count is intentionally a snapshot taken at open and bounded by the 250-message chat buffer; do not add live-updating or unbounded history.
- Adding `messages` to the `handleOpen`/`handleHover` dependency arrays is intentional (the count must reflect the buffer at click time); the callbacks already feed chat rows that re-render on new messages.
