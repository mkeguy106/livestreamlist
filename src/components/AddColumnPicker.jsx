/* Add-column picker for a manual Columns group (Task 5).
 *
 * Follows AddChannelDialog's modal idioms verbatim: full-viewport backdrop
 * that closes on click, `stopPropagation` on the panel itself so clicks
 * inside don't bubble to the backdrop, and a document-level Esc listener
 * armed only while open.
 *
 * The channel roster comes from `ctx.livestreams` (passed in as the
 * `livestreams` prop) rather than a separate `list_channels` IPC call —
 * `Livestream::offline_for` (src-tauri/src/channels.rs) synthesizes an
 * offline placeholder row for every configured channel that isn't
 * currently live, so `list_livestreams` / `refresh_all`'s snapshot already
 * has one entry per channel, live or not. `list_channels` would return the
 * same set of keys with less data (no viewers, no live status) — no need
 * for a second source of truth here.
 *
 * Props: open, onClose, livestreams (full snapshot, live + offline),
 * existingKeys (keys already in the target group — rendered checked +
 * disabled), onConfirm(selectedKeys).
 */
import { useEffect, useMemo, useState } from 'react';
import { platformLetter } from '../utils/format.js';

export default function AddColumnPicker({ open, onClose, livestreams, existingKeys, onConfirm }) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState(() => new Set());

  useEffect(() => {
    if (!open) return;
    setQuery('');
    setSelected(new Set());
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  const existing = useMemo(() => new Set(existingKeys || []), [existingKeys]);

  // Live first (viewers desc), then offline alpha by display name — same
  // ordering rule as the Command sidebar's channel list.
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    const all = livestreams || [];
    const filtered = q
      ? all.filter((l) => (l.display_name || l.unique_key).toLowerCase().includes(q))
      : all;
    const live = filtered
      .filter((l) => l.is_live)
      .sort((a, b) => (b.viewers ?? 0) - (a.viewers ?? 0));
    const offline = filtered
      .filter((l) => !l.is_live)
      .sort((a, b) => (a.display_name || a.unique_key).localeCompare(b.display_name || b.unique_key));
    return [...live, ...offline];
  }, [livestreams, query]);

  const liveKeysAvailable = useMemo(
    () => (livestreams || []).filter((l) => l.is_live && !existing.has(l.unique_key)).map((l) => l.unique_key),
    [livestreams, existing],
  );

  if (!open) return null;

  const toggle = (key) => {
    if (existing.has(key)) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const selectAllLive = () => {
    setSelected((prev) => {
      const next = new Set(prev);
      for (const k of liveKeysAvailable) next.add(k);
      return next;
    });
  };

  const confirm = () => {
    if (selected.size === 0) return;
    onConfirm(Array.from(selected));
    onClose();
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,.5)',
        zIndex: 100,
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: 90,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 480,
          maxHeight: '70vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)',
          borderRadius: 8,
          boxShadow: '0 24px 64px rgba(0,0,0,.7), 0 0 0 1px rgba(255,255,255,.04)',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            padding: '12px 14px',
            borderBottom: 'var(--hair)',
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            flexShrink: 0,
          }}
        >
          <span style={{ color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>›</span>
          <input
            autoFocus
            className="rx-input"
            style={{ border: 'none', background: 'transparent', flex: 1, fontSize: 'var(--t-13)', padding: 0 }}
            placeholder="Search channels…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <div className="rx-kbd">esc</div>
        </div>

        <div
          style={{
            padding: '8px 14px',
            borderBottom: 'var(--hair)',
            display: 'flex',
            alignItems: 'center',
            flexShrink: 0,
          }}
        >
          <button
            type="button"
            className="rx-btn rx-btn-ghost"
            disabled={liveKeysAvailable.length === 0}
            onClick={selectAllLive}
          >
            Select all live
          </button>
        </div>

        <div style={{ overflowY: 'auto', flex: 1, minHeight: 0 }}>
          {rows.length === 0 ? (
            <div style={{ padding: '18px 14px', color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>
              No channels match.
            </div>
          ) : (
            rows.map((l) => {
              const key = l.unique_key;
              const already = existing.has(key);
              const checked = already || selected.has(key);
              const letter = platformLetter(l.platform);
              return (
                <label
                  key={key}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    padding: '6px 14px',
                    cursor: already ? 'default' : 'pointer',
                    opacity: already ? 0.5 : 1,
                  }}
                  onMouseEnter={(e) => {
                    if (!already) e.currentTarget.style.background = 'var(--zinc-900)';
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.background = 'transparent';
                  }}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={already}
                    onChange={() => toggle(key)}
                  />
                  {l.is_live ? <span className="rx-live-dot pulse" /> : <span className="rx-status-dot off" />}
                  <span
                    style={{
                      flex: 1,
                      minWidth: 0,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                      fontSize: 'var(--t-12)',
                      color: 'var(--zinc-100)',
                    }}
                  >
                    {l.display_name || key}
                  </span>
                  <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
                </label>
              );
            })
          )}
        </div>

        <div
          style={{
            padding: '10px 14px',
            borderTop: 'var(--hair)',
            display: 'flex',
            gap: 12,
            alignItems: 'center',
            flexShrink: 0,
          }}
        >
          <div style={{ flex: 1 }} />
          <button type="button" className="rx-btn rx-btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="rx-btn rx-btn-primary" disabled={selected.size === 0} onClick={confirm}>
            Add {selected.size} column{selected.size === 1 ? '' : 's'}
          </button>
        </div>
      </div>
    </div>
  );
}
