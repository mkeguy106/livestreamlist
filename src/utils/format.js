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
