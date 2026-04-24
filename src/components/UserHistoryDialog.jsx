import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { getUserMessages } from '../ipc';

export default function UserHistoryDialog({ open, channelKey, user, onClose }) {
  const [messages, setMessages] = useState([]);
  const [filter, setFilter] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || !user?.id || !channelKey) return;
    setLoading(true);
    getUserMessages(channelKey, user.id, 500)
      .then(ms => setMessages(ms || []))
      .catch(() => setMessages([]))
      .finally(() => setLoading(false));
  }, [open, channelKey, user?.id]);

  useEffect(() => {
    if (!open) return;
    const onKey = e => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const filtered = filter
    ? messages.filter(m => m.text.toLowerCase().includes(filter.toLowerCase()))
    : messages;

  return createPortal(
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.55)',
        zIndex: 250, display: 'grid', placeItems: 'center',
      }}
      onClick={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        style={{
          width: 560, maxHeight: '70vh', display: 'flex', flexDirection: 'column',
          background: 'var(--zinc-925)', border: '1px solid var(--zinc-800)',
          borderRadius: 8, boxShadow: '0 24px 64px rgba(0,0,0,.7)',
          overflow: 'hidden',
        }}
      >
        <div style={{ padding: '12px 14px', borderBottom: 'var(--hair)', display: 'flex', gap: 8, alignItems: 'center' }}>
          <strong style={{ color: 'var(--zinc-100)' }}>
            {user?.display_name || user?.login || 'User'}
          </strong>
          <span style={{ color: 'var(--zinc-500)', fontSize: 11 }}>
            {filtered.length} message{filtered.length === 1 ? '' : 's'}
          </span>
          <input
            className="rx-input"
            placeholder="Filter…"
            value={filter}
            onChange={e => setFilter(e.target.value)}
            style={{ marginLeft: 'auto', width: 200 }}
          />
        </div>
        <div style={{ overflow: 'auto', padding: '8px 14px', flex: 1 }}>
          {loading ? (
            <div style={{ color: 'var(--zinc-500)' }}>Loading…</div>
          ) : filtered.length === 0 ? (
            <div style={{ color: 'var(--zinc-500)' }}>No messages.</div>
          ) : (
            filtered.map(m => (
              <div
                key={m.id}
                onClick={() => navigator.clipboard?.writeText(m.text)}
                title="Click to copy"
                style={{
                  padding: '6px 0', borderBottom: 'var(--hair)',
                  color: 'var(--zinc-300)', cursor: 'copy', fontSize: 12,
                }}
              >
                <span style={{ color: 'var(--zinc-500)', marginRight: 8, fontSize: 11 }}>
                  {new Date(m.timestamp).toLocaleTimeString()}
                </span>
                {m.text}
              </div>
            ))
          )}
        </div>
      </div>
    </div>,
    document.body
  );
}
