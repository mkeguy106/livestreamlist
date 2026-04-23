/* Direction B — "Columns"
 * TweetDeck for livestreams. One column per live channel.
 */

import { formatViewers } from '../utils/format.js';

export default function Columns({ ctx }) {
  const { livestreams, openAddDialog, launchStream, openInBrowser } = ctx;

  const live = livestreams
    .filter((l) => l.is_live)
    .sort((a, b) => (b.viewers ?? 0) - (a.viewers ?? 0));
  const totalViewers = live.reduce((sum, l) => sum + (l.viewers ?? 0), 0);

  return (
    <>
      <div
        style={{
          height: 36,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '0 12px',
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        <button type="button" className="rx-btn" onClick={openAddDialog}>＋ Add channel</button>
        <button type="button" className="rx-btn rx-btn-ghost" onClick={ctx.refresh}>↻ Refresh</button>
        <div style={{ flex: 1 }} />
        <span className="rx-chiclet">{live.length} live · {livestreams.length} total</span>
      </div>

      <div style={{ flex: 1, display: 'flex', minHeight: 0, overflowX: 'auto' }}>
        {live.map((ch, ci) => (
          <Column
            key={ch.unique_key}
            channel={ch}
            accentColumn={ci === 0}
            onLaunch={() => launchStream(ch.unique_key)}
            onOpenBrowser={() => openInBrowser(ch.unique_key)}
          />
        ))}

        {live.length === 0 && (
          <div
            style={{
              flex: 1,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--zinc-500)',
              fontSize: 'var(--t-12)',
            }}
          >
            No live channels right now.
          </div>
        )}

        <button
          type="button"
          onClick={openAddDialog}
          style={{
            flex: '0 0 200px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--zinc-600)',
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 6 }}>
            <div
              style={{
                width: 32,
                height: 32,
                border: '1px dashed var(--zinc-800)',
                borderRadius: 4,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontSize: 16,
                color: 'var(--zinc-500)',
              }}
            >
              ＋
            </div>
            <span className="rx-chiclet">Add channel</span>
          </div>
        </button>
      </div>

      <div
        style={{
          height: 24,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          borderTop: 'var(--hair)',
          gap: 12,
          flexShrink: 0,
        }}
      >
        <span className="rx-chiclet">{live.length} live</span>
        <span className="rx-chiclet">{formatViewers(totalViewers)} total viewers</span>
        <div style={{ flex: 1 }} />
        <span className="rx-chiclet">sync: {ctx.loading ? 'refreshing' : 'idle'}</span>
      </div>
    </>
  );
}

function Column({ channel, accentColumn, onLaunch, onOpenBrowser }) {
  const letter = channel.platform.charAt(0);

  return (
    <div
      style={{
        flex: '0 0 280px',
        display: 'flex',
        flexDirection: 'column',
        minWidth: 0,
        borderRight: 'var(--hair)',
        background: accentColumn ? 'rgba(244,244,245,.015)' : 'transparent',
      }}
    >
      <div style={{ padding: '10px 12px 8px', borderBottom: 'var(--hair)' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span className="rx-live-dot pulse" />
          <span style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-100)', fontWeight: 600 }}>
            {channel.display_name}
          </span>
          <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
          <div style={{ flex: 1 }} />
          <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>
            {formatViewers(channel.viewers)}
          </span>
        </div>
        <div
          style={{
            marginTop: 4,
            fontSize: 'var(--t-11)',
            color: 'var(--zinc-400)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
          title={channel.title ?? ''}
        >
          {channel.title ?? '—'}
        </div>
        <div className="rx-mono" style={{ marginTop: 2, fontSize: 10, color: 'var(--zinc-500)' }}>
          {channel.game ?? ''}
        </div>
      </div>

      <button
        type="button"
        onClick={onLaunch}
        style={{
          height: 140,
          margin: '8px 10px',
          background: 'linear-gradient(135deg, var(--zinc-900), var(--zinc-850))',
          border: '1px solid var(--zinc-800)',
          borderRadius: 4,
          position: 'relative',
          overflow: 'hidden',
          flexShrink: 0,
          cursor: 'pointer',
          padding: 0,
        }}
        title={`Launch ${channel.display_name} via streamlink`}
      >
        {channel.thumbnail_url && (
          <img
            src={channel.thumbnail_url}
            alt=""
            style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.7 }}
          />
        )}
        <div style={{ position: 'absolute', top: 6, left: 6, display: 'flex', gap: 4, alignItems: 'center' }}>
          <span className="rx-live-dot pulse" />
          <span
            className="rx-mono"
            style={{ fontSize: 9, color: 'var(--zinc-100)', fontWeight: 600, letterSpacing: '.08em' }}
          >
            LIVE
          </span>
        </div>
        <div
          style={{
            position: 'absolute',
            inset: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <div
            style={{
              width: 32,
              height: 32,
              borderRadius: '50%',
              border: '1px solid var(--zinc-500)',
              background: 'rgba(9,9,11,.6)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--zinc-100)',
              fontSize: 12,
            }}
          >
            ▶
          </div>
        </div>
      </button>

      {/* Chat placeholder — Phase 2 */}
      <div
        style={{
          flex: 1,
          overflowY: 'auto',
          padding: '10px',
          color: 'var(--zinc-600)',
          fontSize: 'var(--t-11)',
          lineHeight: 1.5,
        }}
      >
        Chat arrives in Phase 2.
      </div>

      <div style={{ borderTop: 'var(--hair)', padding: '6px 10px', display: 'flex', gap: 6 }}>
        <button type="button" className="rx-btn rx-btn-ghost" style={{ flex: 1, fontSize: 10 }} onClick={onOpenBrowser}>
          Open in browser
        </button>
      </div>
    </div>
  );
}
