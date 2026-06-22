/** Format a viewer count as 1.2K / 47.2k / 127.5K. */
export function formatViewers(n) {
  if (n == null) return '—';
  if (n < 1000) return String(n);
  const k = n / 1000;
  return `${k.toFixed(1)}k`;
}

/** Format an RFC3339 start timestamp as uptime "2h 14m" or "47m". */
export function formatUptime(iso) {
  if (!iso) return '';
  const start = new Date(iso);
  const ms = Date.now() - start.getTime();
  if (ms < 0) return '';
  const totalMin = Math.floor(ms / 60_000);
  const hours = Math.floor(totalMin / 60);
  const mins = totalMin % 60;
  return hours > 0 ? `${hours}h ${String(mins).padStart(2, '0')}m` : `${mins}m`;
}

/** First letter of platform name for the `rx-plat` chip. */
export function platformLetter(platform) {
  return (platform ?? '').charAt(0).toLowerCase(); // t/y/k/c
}

/**
 * Build the public channel URL for a chat user, given the platform they're
 * chatting on and their login/handle. Used by the user card's "Open channel"
 * action so it links to *that user's* channel, not the channel you're watching.
 * Returns null for unknown platforms or a missing login.
 */
export function userChannelUrl(platform, login) {
  if (!login) return null;
  const handle = encodeURIComponent(login);
  switch (platform) {
    case 'twitch': return `https://www.twitch.tv/${handle}`;
    case 'kick': return `https://kick.com/${handle}`;
    case 'chaturbate': return `https://chaturbate.com/${handle}/`;
    case 'youtube': return `https://www.youtube.com/@${handle}`;
    default: return null;
  }
}

/**
 * Turn an RFC3339 timestamp (or anything Date.parse() handles) into a
 * coarse relative string: "just now", "5m ago", "3h ago", "2d ago".
 * Returns the original string on parse failure.
 */
export function formatRelative(ts) {
  if (!ts) return '';
  const ms = Date.parse(ts);
  if (Number.isNaN(ms)) return ts;
  const diff = Date.now() - ms;
  if (diff < 0 || diff < 30_000) return 'just now';
  const m = Math.floor(diff / 60_000);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ago`;
  const d = Math.floor(h / 24);
  return `${d}d ago`;
}

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

// ── Module-scope DEV asserts (run once on import in dev). ──────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // userChannelUrl links to the clicked user's channel, per platform.
  console.assert(
    userChannelUrl('twitch', 'shroud') === 'https://www.twitch.tv/shroud',
    'userChannelUrl twitch',
  );
  console.assert(
    userChannelUrl('kick', 'xqc') === 'https://kick.com/xqc',
    'userChannelUrl kick',
  );
  console.assert(
    userChannelUrl('chaturbate', 'emma') === 'https://chaturbate.com/emma/',
    'userChannelUrl chaturbate',
  );
  console.assert(
    userChannelUrl('youtube', 'Ludwig') === 'https://www.youtube.com/@Ludwig',
    'userChannelUrl youtube',
  );
  console.assert(userChannelUrl('twitch', '') === null, 'userChannelUrl empty login');
  console.assert(userChannelUrl('mystery', 'x') === null, 'userChannelUrl unknown platform');
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
}
