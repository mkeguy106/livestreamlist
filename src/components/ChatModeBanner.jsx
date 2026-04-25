import { useRoomState } from '../hooks/useRoomState.js';

/**
 * Single-row banner pinned above the chat composer summarising every active
 * restrictive chat mode. Dismissible per-session-per-channel; reappears when
 * the underlying state changes.
 */
export default function ChatModeBanner({ channelKey, variant = 'irc' }) {
  const { state, visible, dismiss } = useRoomState(channelKey);
  if (!visible || !state) return null;

  const compact = variant === 'compact';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: compact ? '3px 8px' : '4px 14px',
        background: 'rgba(255,255,255,.025)',
        borderTop: 'var(--hair)',
        borderLeft: '2px solid var(--warn)',
        color: 'var(--zinc-300)',
        fontSize: compact ? 10 : 'var(--t-11)',
        lineHeight: 1.4,
      }}
    >
      <span style={{ color: 'var(--warn)', flex: '0 0 auto' }}>ⓘ</span>
      <span
        style={{
          flex: '1 1 auto',
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {formatModes(state)}
      </span>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss chat-mode banner"
        style={{
          all: 'unset',
          cursor: 'pointer',
          color: 'var(--zinc-500)',
          padding: '0 4px',
          fontSize: compact ? 12 : 14,
          lineHeight: 1,
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.color = 'var(--zinc-300)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.color = 'var(--zinc-500)';
        }}
      >
        ×
      </button>
    </div>
  );
}

function formatModes(state) {
  const parts = [];
  if (state.slow_seconds > 0) parts.push(`Slow mode (${state.slow_seconds}s)`);
  if (state.subs_only) parts.push('Subs-only');
  if (state.followers_only_minutes >= 0) {
    const m = state.followers_only_minutes;
    parts.push(m === 0 ? 'Followers-only' : `Followers-only (${formatFollowersDuration(m)})`);
  }
  if (state.emote_only) parts.push('Emote-only');
  if (state.r9k) parts.push('Unique chat');
  return parts.join(' · ');
}

function formatFollowersDuration(minutes) {
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.round(minutes / 60)}h`;
  if (minutes < 10080) return `${Math.round(minutes / 1440)}d`;
  if (minutes < 43200) return `${Math.round(minutes / 10080)}w`;
  return `${Math.round(minutes / 43200)}mo`;
}
