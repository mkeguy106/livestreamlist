/* Live-only quick-switch strip for Focus — replaces the all-channels tab
 * strip (offline channels never appear in Focus). Renders nothing when no
 * channel is live: the blank-state picker already says so.
 */
import { formatViewers, platformLetter } from '../utils/format.js';
import { liveOnlyRows } from '../utils/channelLists.js';

export default function FocusLiveStrip({ livestreams, focusedKey, onPick }) {
  const rows = liveOnlyRows(livestreams || [], '');
  if (rows.length === 0) return null;
  return (
    <div
      style={{
        height: 38,
        display: 'flex',
        alignItems: 'stretch',
        borderBottom: 'var(--hair)',
        overflowX: 'auto',
        flexShrink: 0,
      }}
    >
      {rows.map((t) => {
        const active = t.unique_key === focusedKey;
        const letter = platformLetter(t.platform);
        return (
          <button
            key={t.unique_key}
            type="button"
            data-focus-strip-tab={t.unique_key}
            onClick={() => onPick(t.unique_key)}
            style={{
              flex: '0 0 auto',
              padding: '0 14px',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              borderRight: 'var(--hair)',
              borderTop: 'none',
              borderLeft: 'none',
              background: active ? 'var(--zinc-900)' : 'transparent',
              borderBottom: active ? '2px solid var(--zinc-100)' : '2px solid transparent',
              color: 'var(--zinc-100)',
              cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            <span className="rx-live-dot" />
            <span style={{ fontSize: 'var(--t-12)', fontWeight: active ? 600 : 500 }}>{t.display_name}</span>
            <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
            <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
              {formatViewers(t.viewers)}
            </span>
          </button>
        );
      })}
    </div>
  );
}
