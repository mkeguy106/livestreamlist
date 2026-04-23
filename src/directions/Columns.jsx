/* Direction B — "Columns"
 * TweetDeck for livestreams. Horizontal scroll of per-stream columns.
 * Each column = header + tiny video + IRC chat. Mono chrome; platform color only on live dot.
 */

const cols = [
  { id: 'shroud',    plat: 't', live: true, viewers: '47.2k', game: 'VALORANT',      title: 'ranked grind to radiant' },
  { id: 'xQc',       plat: 't', live: true, viewers: '82.1k', game: 'Just Chatting', title: 'reacting to everything' },
  { id: 'HasanAbi',  plat: 't', live: true, viewers: '29.4k', game: 'Just Chatting', title: 'late night pol takes' },
  { id: 'Ludwig',    plat: 'y', live: true, viewers: '11.8k', game: 'Chess',         title: 'pogo tournament round 3' },
  { id: 'Trainwrex', plat: 'k', live: true, viewers: '8.9k',  game: 'Slots',         title: 'max bets only' },
];

const sampleChat = (seed) => [
  { u: 'vanishh',    c: '#a78bfa', m: 'first' },
  { u: 'mikael',     c: '#f87171', m: 'LUL' },
  { u: 'kyra.',      c: '#60a5fa', m: 'cooked' },
  { u: 'tomj',       c: '#4ade80', m: 'O7' },
  { u: 'marbled',    c: '#a78bfa', m: '?' },
  { u: 'paulie',     c: '#fb923c', m: 'W' },
  { u: 'kirwan__',   c: '#22d3ee', m: 'shrdBl shrdBl' },
  { u: 'reyna.main', c: '#f472b6', m: 'he cooked' },
  { u: 'ilikepie',   c: '#84cc16', m: 'PogChamp' },
  { u: 'dontban',    c: '#a78bfa', m: '@' + seed + ' cracked' },
  { u: 'moonbeam',   c: '#60a5fa', m: '🎯' },
  { u: 'alley.cat',  c: '#4ade80', m: 'insane gamesense' },
  { u: 'BTTVfan',    c: '#fbbf24', m: 'peepoClap' },
  { u: 'dustin_x',   c: '#fb923c', m: 'vod review time' },
  { u: 'yz0',        c: '#22d3ee', m: 'cracked' },
  { u: 'keltoi',     c: '#f472b6', m: 'clean' },
];

export default function Columns() {
  return (
    <>
      {/* Toolbar */}
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
        <button className="rx-btn">＋ Add column</button>
        <button className="rx-btn rx-btn-ghost">Layout</button>
        <button className="rx-btn rx-btn-ghost">Group: Live</button>
        <div style={{ flex: 1 }} />
        <input className="rx-input" placeholder="Filter channels…" style={{ width: 220 }} />
      </div>

      {/* Columns strip */}
      <div style={{ flex: 1, display: 'flex', minHeight: 0, overflowX: 'auto' }}>
        {cols.map((col, ci) => {
          const chat = sampleChat(col.id);
          return (
            <div
              key={col.id}
              style={{
                flex: '0 0 260px',
                display: 'flex',
                flexDirection: 'column',
                minWidth: 0,
                borderRight: 'var(--hair)',
                background: ci === 0 ? 'rgba(244,244,245,.015)' : 'transparent',
              }}
            >
              {/* Column header */}
              <div style={{ padding: '10px 12px 8px', borderBottom: 'var(--hair)' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span className="rx-live-dot pulse" />
                  <span style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-100)', fontWeight: 600 }}>{col.id}</span>
                  <span className={`rx-plat ${col.plat}`}>{col.plat.toUpperCase()}</span>
                  <div style={{ flex: 1 }} />
                  <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>{col.viewers}</span>
                </div>
                <div
                  style={{
                    marginTop: 4,
                    fontSize: 'var(--t-11)',
                    color: 'var(--zinc-400)',
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    minWidth: 0,
                  }}
                  title={col.title}
                >
                  {col.title}
                </div>
                <div className="rx-mono" style={{ marginTop: 2, fontSize: 10, color: 'var(--zinc-500)' }}>
                  {col.game}
                </div>
              </div>

              {/* Tiny video */}
              <div
                style={{
                  height: 120,
                  margin: '8px 10px',
                  background: 'linear-gradient(135deg, var(--zinc-900), var(--zinc-850))',
                  border: '1px solid var(--zinc-800)',
                  borderRadius: 4,
                  position: 'relative',
                  overflow: 'hidden',
                  flexShrink: 0,
                }}
              >
                <svg width="100%" height="100%" style={{ position: 'absolute', inset: 0, opacity: 0.35 }}>
                  <defs>
                    <pattern id={`g${ci}`} width="12" height="12" patternUnits="userSpaceOnUse">
                      <path d="M 12 0 L 0 0 0 12" fill="none" stroke="rgba(255,255,255,.04)" strokeWidth=".5" />
                    </pattern>
                  </defs>
                  <rect width="100%" height="100%" fill={`url(#g${ci})`} />
                </svg>
                <div style={{ position: 'absolute', top: 6, left: 6, display: 'flex', gap: 4, alignItems: 'center' }}>
                  <span className="rx-live-dot pulse" />
                  <span
                    className="rx-mono"
                    style={{ fontSize: 9, color: 'var(--zinc-100)', fontWeight: 600, letterSpacing: '.08em' }}
                  >
                    LIVE
                  </span>
                </div>
                <div style={{ position: 'absolute', bottom: 6, right: 6 }} className="rx-mono">
                  <span style={{ fontSize: 9, color: 'var(--zinc-400)' }}>1080p60</span>
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
                      width: 28,
                      height: 28,
                      borderRadius: '50%',
                      border: '1px solid var(--zinc-600)',
                      display: 'flex',
                      alignItems: 'center',
                      justifyContent: 'center',
                      color: 'var(--zinc-300)',
                      fontSize: 10,
                    }}
                  >
                    ▶
                  </div>
                </div>
              </div>

              {/* Chat */}
              <div style={{ flex: 1, overflowY: 'auto', padding: '2px 10px 8px' }}>
                {chat.map((m, i) => (
                  <div
                    key={i}
                    style={{
                      display: 'flex',
                      gap: 6,
                      fontSize: 'var(--t-11)',
                      lineHeight: 1.45,
                      padding: '1px 0',
                    }}
                  >
                    <span style={{ color: m.c, fontWeight: 500, flex: '0 0 auto' }}>{m.u}</span>
                    <span
                      style={{
                        color: 'var(--zinc-300)',
                        minWidth: 0,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {m.m}
                    </span>
                  </div>
                ))}
              </div>

              {/* Composer */}
              <div style={{ borderTop: 'var(--hair)', padding: '6px 10px' }}>
                <input
                  className="rx-input"
                  placeholder={`Send to ${col.id}…`}
                  style={{ width: '100%', fontSize: 'var(--t-11)' }}
                />
              </div>
            </div>
          );
        })}

        {/* Add column ghost */}
        <div
          style={{
            flex: '0 0 200px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--zinc-600)',
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
            <span className="rx-chiclet">Add column</span>
          </div>
        </div>
      </div>

      {/* Statusbar */}
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
        <span className="rx-chiclet">5 columns</span>
        <span className="rx-chiclet">7 live</span>
        <span className="rx-chiclet">179.4k total viewers</span>
        <div style={{ flex: 1 }} />
        <span className="rx-chiclet">sync: idle</span>
      </div>
    </>
  );
}
