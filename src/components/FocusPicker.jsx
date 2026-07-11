/* Centered live-channel chooser for the Focus layout's blank state.
 *
 * An inline card, NOT a modal: no backdrop, no Esc-close, no
 * useEmbedOcclusion (blank Focus mounts no native embeds). Card chrome
 * mirrors AddColumnPicker; list internals are the shared pure helpers in
 * src/utils/channelLists.js — live-only, viewers-desc, searchable.
 */
import { useMemo, useState } from 'react';
import { formatViewers, platformLetter } from '../utils/format.js';
import { liveOnlyRows } from '../utils/channelLists.js';

export default function FocusPicker({ livestreams, onPick }) {
  const [query, setQuery] = useState('');
  const rows = useMemo(() => liveOnlyRows(livestreams || [], query), [livestreams, query]);
  const anyLive = (livestreams || []).some((l) => l.is_live);

  return (
    <div
      style={{
        width: 480,
        maxHeight: '60vh',
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
          placeholder="Feature a live channel…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && rows.length > 0) onPick(rows[0].unique_key);
            else if (e.key === 'Escape') setQuery('');
          }}
        />
        <div className="rx-kbd">enter</div>
      </div>

      <div style={{ overflowY: 'auto', flex: 1, minHeight: 0 }}>
        {rows.length === 0 ? (
          <div style={{ padding: '18px 14px', color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>
            {anyLive ? 'No live channels match.' : 'No channels are live right now.'}
          </div>
        ) : (
          rows.map((l) => {
            const letter = platformLetter(l.platform);
            return (
              <button
                key={l.unique_key}
                type="button"
                data-focus-picker-row={l.unique_key}
                onClick={() => onPick(l.unique_key)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  width: '100%',
                  padding: '7px 14px',
                  background: 'transparent',
                  border: 'none',
                  cursor: 'pointer',
                  textAlign: 'left',
                  fontFamily: 'inherit',
                }}
                onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--zinc-900)'; }}
                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}
              >
                <span className="rx-live-dot pulse" />
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
                  {l.display_name || l.unique_key}
                </span>
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                  {formatViewers(l.viewers)}
                </span>
                <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}
