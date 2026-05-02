/**
 * Pinned-above-composer banner shown when an anniversary is ready
 * to share. Props:
 *   info: SubAnniversaryInfo (months, channel_display_name, channel_login, …)
 *   onShare: () => void
 *   onDismiss: () => void
 */
export function SubAnniversaryBanner({ info, onShare, onDismiss }) {
  if (!info) return null;
  const months = info.months ?? 0;
  const monthWord = months === 1 ? 'month' : 'months';
  const display = info.channel_display_name || info.channel_login || 'this channel';
  return (
    <div
      className="rx-sub-anniv-banner"
      role="status"
      aria-label={`Sub anniversary ready to share for ${display}`}
    >
      <span className="rx-sub-anniv-banner__star" aria-hidden="true">⭐</span>
      <div className="rx-sub-anniv-banner__text">
        <strong>Your {months} {monthWord} anniversary at {display}</strong> is ready to share.
        <span className="rx-sub-anniv-banner__sub">Twitch will let you add a message.</span>
      </div>
      <button
        type="button"
        className="rx-btn rx-btn-primary rx-sub-anniv-banner__share"
        onClick={onShare}
      >
        Share now
      </button>
      <button
        type="button"
        className="rx-sub-anniv-banner__dismiss"
        onClick={onDismiss}
        aria-label="Dismiss anniversary banner"
      >
        ×
      </button>
    </div>
  );
}
