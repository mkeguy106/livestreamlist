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
