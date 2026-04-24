import { useEffect, useMemo } from 'react';
import EmoteText from './EmoteText.jsx';

/**
 * Show the full @-mention thread between two users, filtered out of the
 * channel's in-memory chat buffer. Messages are included when:
 *   - authored by one of the pair AND @-mentions the other, OR
 *   - directly replying to the other user (Twitch reply threading)
 *
 * Scope is limited to whatever's currently in the live buffer (~250 msgs).
 * Deeper history would require querying the JSONL log store — Phase 4.
 */
export default function ConversationDialog({ open, messages, pair, onClose }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  const filtered = useMemo(() => {
    if (!open || !pair) return [];
    const [a, b] = [pair.a?.toLowerCase(), pair.b?.toLowerCase()];
    if (!a || !b || a === b) return [];
    const mentionsA = new RegExp(`@${escapeRegex(a)}\\b`, 'i');
    const mentionsB = new RegExp(`@${escapeRegex(b)}\\b`, 'i');
    return messages.filter((m) => {
      if (m.system) return false;
      const login = (m.user?.login || '').toLowerCase();
      const parent = (m.reply_to?.parent_login || '').toLowerCase();
      if (login === a && (mentionsB.test(m.text) || parent === b)) return true;
      if (login === b && (mentionsA.test(m.text) || parent === a)) return true;
      return false;
    });
  }, [messages, pair, open]);

  if (!open || !pair) return null;

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,.55)',
        zIndex: 100,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 40,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(640px, 100%)',
          maxHeight: '80vh',
          background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)',
          borderRadius: 8,
          boxShadow: '0 24px 64px rgba(0,0,0,.7), 0 0 0 1px rgba(255,255,255,.04)',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            padding: '12px 16px',
            borderBottom: 'var(--hair)',
            display: 'flex',
            alignItems: 'baseline',
            gap: 10,
          }}
        >
          <span className="rx-chiclet">CONVERSATION</span>
          <span style={{ color: 'var(--zinc-200)', fontSize: 'var(--t-13)' }}>
            @{pair.a} <span style={{ color: 'var(--zinc-700)' }}>↔</span> @{pair.b}
          </span>
          <div style={{ flex: 1 }} />
          <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
            {filtered.length} {filtered.length === 1 ? 'message' : 'messages'}
          </span>
          <button
            type="button"
            className="rx-btn rx-btn-ghost"
            onClick={onClose}
            style={{ padding: '2px 8px' }}
          >
            esc
          </button>
        </div>
        <div
          style={{
            flex: 1,
            overflowY: 'auto',
            padding: '8px 0',
          }}
        >
          {filtered.length === 0 ? (
            <div
              style={{
                padding: 24,
                color: 'var(--zinc-500)',
                fontSize: 'var(--t-12)',
                textAlign: 'center',
              }}
            >
              No matching messages in the visible chat buffer.
              <div style={{ color: 'var(--zinc-600)', fontSize: 'var(--t-11)', marginTop: 6 }}>
                Deeper scan-back arrives with the preferences dialog.
              </div>
            </div>
          ) : (
            filtered.map((m) => (
              <div
                key={m.id}
                style={{
                  padding: '3px 16px',
                  display: 'grid',
                  gridTemplateColumns: '60px minmax(0,1fr)',
                  columnGap: 10,
                  fontSize: 'var(--t-12)',
                  lineHeight: 1.45,
                }}
              >
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)' }}>
                  {formatShortTime(m.timestamp)}
                </span>
                <span style={{ minWidth: 0 }}>
                  <span style={{ color: m.user.color || '#a1a1aa', fontWeight: 500 }}>
                    {m.user.display_name || m.user.login}
                  </span>
                  <span style={{ color: 'var(--zinc-600)' }}>:</span>{' '}
                  <span style={{ color: 'var(--zinc-200)' }}>
                    <EmoteText text={m.text} ranges={m.emote_ranges} size={20} />
                  </span>
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function escapeRegex(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function formatShortTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  const h = String(d.getHours()).padStart(2, '0');
  const m = String(d.getMinutes()).padStart(2, '0');
  return `${h}:${m}`;
}
