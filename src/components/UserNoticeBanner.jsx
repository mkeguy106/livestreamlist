import EmoteText from './EmoteText.jsx';

/**
 * Pinned-above-composer banner for chat events (subs, gifts, raids,
 * announcements). One slot — driven by useEventBanner's current event.
 *
 * Props:
 *   event: BannerEvent (must be non-null when rendered)
 *   onDismiss: () => void
 */
const GLYPHS = {
  sub: '★',
  resub: '★',
  subgift: '★',
  submysterygift: '★',
  raid: '⤴',
  announcement: '✦',
  bitsbadgetier: '✦',
};

export default function UserNoticeBanner({ event, onDismiss }) {
  if (!event) return null;
  const glyph = GLYPHS[event.kind] ?? '✦';
  const heading = event.text || `${event.kind} event`;
  const userText = event.userText && event.userText.trim().length > 0
    ? event.userText
    : null;

  return (
    <div
      className="rx-event-banner"
      data-kind={event.kind}
      role="status"
      aria-label={`Chat event: ${heading}`}
    >
      <span className="rx-event-banner__glyph" aria-hidden="true">{glyph}</span>
      <div className="rx-event-banner__text">
        <strong>{heading}</strong>
        {userText && (
          <span className="rx-event-banner__user">
            <EmoteText
              text={userText}
              ranges={event.emoteRanges}
              links={event.linkRanges}
              size={20}
            />
          </span>
        )}
      </div>
      <button
        type="button"
        className="rx-event-banner__dismiss"
        onClick={onDismiss}
        aria-label="Dismiss event banner"
      >
        ×
      </button>
    </div>
  );
}
