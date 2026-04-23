/* Direction C — "Focus"
 * One featured stream, 60/40 chat. Hairline strip of other channels along the top (tabs w/ live dots).
 * Reader-mode-y. Maximum signal-to-noise on a single stream.
 */

const tabs = [
  { id: 'shroud',     plat: 't', live: true,  viewers: '47.2k', active: true },
  { id: 'xQc',        plat: 't', live: true,  viewers: '82.1k' },
  { id: 'HasanAbi',   plat: 't', live: true,  viewers: '29.4k' },
  { id: 'Ludwig',     plat: 'y', live: true,  viewers: '11.8k' },
  { id: 'Asmongold',  plat: 't', live: true,  viewers: '34.6k' },
  { id: 'Trainwrex',  plat: 'k', live: true,  viewers: '8.9k'  },
  { id: 'Mizkif',     plat: 't', live: true,  viewers: '6.1k'  },
  { id: 'pokimane',   plat: 't', live: false },
  { id: 'sodapoppin', plat: 't', live: false },
  { id: 'Fextralife', plat: 'y', live: false },
];

const chat = [
  { t: '21:04:11', u: 'vanishh',        c: '#a78bfa', m: 'welcome back btw' },
  { t: '21:04:12', u: 'mikael_ek',      c: '#f87171', m: 'the clutch was so clean' },
  { t: '21:04:12', u: 'kyra.',          c: '#60a5fa', m: 'skillgap' },
  { t: '21:04:13', u: 'tomjones',       c: '#4ade80', m: 'O7' },
  { t: '21:04:14', u: 'NightbotLUL',    c: '#fbbf24', m: '!discord → https://discord.gg/shroud' },
  { t: '21:04:14', u: 'marbled',        c: '#a78bfa', m: 'when the aim be aiming' },
  { t: '21:04:15', u: 'paulieboy',      c: '#fb923c', m: 'thats a W' },
  { t: '21:04:16', u: 'kirwan__',       c: '#22d3ee', m: 'shrdBl shrdBl shrdBl' },
  { t: '21:04:17', u: 'reyna.main',     c: '#f472b6', m: 'he cooked' },
  { t: '21:04:17', u: 'ilikepie',       c: '#84cc16', m: 'PogChamp' },
  { t: '21:04:18', u: 'dontban',        c: '#a78bfa', m: '@shroud how do you move so fast' },
  { t: '21:04:19', u: 'rainbowtoast',   c: '#f87171', m: 'literally insane' },
  { t: '21:04:19', u: 'moonbeam',       c: '#60a5fa', m: '🎯' },
  { t: '21:04:20', u: 'alley.cat',      c: '#4ade80', m: 'gamesense is something else' },
  { t: '21:04:21', u: 'BTTV_peepoClap', c: '#fbbf24', m: 'peepoClap peepoClap' },
  { t: '21:04:21', u: 'dustin_x',       c: '#fb923c', m: 'vod review when' },
  { t: '21:04:22', u: 'yz0',            c: '#22d3ee', m: 'cracked' },
  { t: '21:04:22', u: 'keltoi',         c: '#f472b6', m: 'did he pre-aim that' },
];

export default function Focus() {
  return (
    <>
      {/* Tab strip */}
      <div
        style={{
          height: 38,
          display: 'flex',
          alignItems: 'stretch',
          borderBottom: 'var(--hair)',
          overflowX: 'auto',
          WebkitAppRegion: 'no-drag',
          flexShrink: 0,
        }}
      >
        {tabs.map((t) => (
          <div
            key={t.id}
            style={{
              flex: '0 0 auto',
              padding: '0 14px',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              borderRight: 'var(--hair)',
              background: t.active ? 'var(--zinc-900)' : 'transparent',
              borderBottom: t.active ? '2px solid var(--zinc-100)' : '2px solid transparent',
              color: t.live ? 'var(--zinc-100)' : 'var(--zinc-600)',
              cursor: 'pointer',
            }}
          >
            <span className={`rx-status-dot ${t.live ? 'live' : 'off'}`} />
            <span style={{ fontSize: 'var(--t-12)', fontWeight: t.active ? 600 : 500 }}>{t.id}</span>
            <span className={`rx-plat ${t.plat}`}>{t.plat.toUpperCase()}</span>
            {t.live && (
              <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                {t.viewers}
              </span>
            )}
          </div>
        ))}
        <div style={{ padding: '0 12px', display: 'flex', alignItems: 'center', color: 'var(--zinc-500)' }}>
          <span className="rx-chiclet">＋</span>
        </div>
      </div>

      {/* Split */}
      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        {/* Stream */}
        <div style={{ flex: '1 1 60%', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          {/* meta */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              padding: '10px 16px',
              borderBottom: 'var(--hair)',
            }}
          >
            <span className="rx-live-dot pulse" />
            <span style={{ fontSize: 'var(--t-14)', color: 'var(--zinc-100)', fontWeight: 600 }}>shroud</span>
            <span className="rx-plat t">TWITCH</span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-300)' }}>ranked grind to radiant</span>
            <div style={{ flex: 1 }} />
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>47,204</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>· 2h 14m</span>
            <button className="rx-btn rx-btn-ghost">⌘⇧F</button>
          </div>

          {/* video */}
          <div
            style={{
              flex: 1,
              margin: 16,
              background: 'var(--zinc-925)',
              border: '1px solid var(--zinc-800)',
              borderRadius: 4,
              position: 'relative',
              overflow: 'hidden',
              minHeight: 0,
            }}
          >
            <svg width="100%" height="100%" style={{ position: 'absolute', inset: 0, opacity: 0.4 }}>
              <defs>
                <pattern id="gf" width="18" height="18" patternUnits="userSpaceOnUse">
                  <path d="M 18 0 L 0 0 0 18" fill="none" stroke="rgba(255,255,255,.03)" strokeWidth=".5" />
                </pattern>
              </defs>
              <rect width="100%" height="100%" fill="url(#gf)" />
            </svg>
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
                  width: 56,
                  height: 56,
                  borderRadius: '50%',
                  border: '1px solid var(--zinc-700)',
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  color: 'var(--zinc-300)',
                }}
              >
                ▶
              </div>
            </div>
            <div
              style={{
                position: 'absolute',
                left: 16,
                right: 16,
                bottom: 16,
                display: 'flex',
                alignItems: 'center',
                gap: 10,
              }}
            >
              <span className="rx-live-dot pulse" />
              <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-100)', fontWeight: 600 }}>LIVE</span>
              <div
                style={{
                  flex: 1,
                  height: 2,
                  background: 'var(--zinc-800)',
                  borderRadius: 1,
                  position: 'relative',
                }}
              >
                <div
                  style={{
                    position: 'absolute',
                    left: 0,
                    top: 0,
                    bottom: 0,
                    width: '96%',
                    background: 'var(--zinc-300)',
                  }}
                />
              </div>
              <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>1080p60 · 6.2 Mbps</span>
            </div>
          </div>

          {/* hairline action strip */}
          <div style={{ display: 'flex', gap: 8, padding: '0 16px 12px' }}>
            <button className="rx-btn rx-btn-ghost">Theatre</button>
            <button className="rx-btn rx-btn-ghost">Quality · Source</button>
            <button className="rx-btn rx-btn-ghost">Pop out</button>
            <div style={{ flex: 1 }} />
            <button className="rx-btn rx-btn-ghost">Follow</button>
            <button className="rx-btn">Open on Twitch ↗</button>
          </div>
        </div>

        {/* Chat */}
        <div
          style={{
            flex: '1 1 40%',
            display: 'flex',
            flexDirection: 'column',
            minWidth: 340,
            borderLeft: 'var(--hair)',
            minHeight: 0,
          }}
        >
          <div
            style={{
              padding: '10px 14px',
              borderBottom: 'var(--hair)',
              display: 'flex',
              alignItems: 'center',
              gap: 10,
            }}
          >
            <span className="rx-chiclet">CHAT</span>
            <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>· 142 msg/min</span>
            <div style={{ flex: 1 }} />
            <button className="rx-btn rx-btn-ghost" style={{ padding: '2px 6px', fontSize: 10 }}>Slow</button>
            <button className="rx-btn rx-btn-ghost" style={{ padding: '2px 6px', fontSize: 10 }}>Emotes</button>
            <button className="rx-btn rx-btn-ghost" style={{ padding: '2px 6px', fontSize: 10 }}>Pause</button>
          </div>
          <div style={{ flex: 1, overflowY: 'auto', padding: '6px 0' }}>
            {chat.map((m, i) => (
              <div
                key={i}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '52px 110px 1fr',
                  columnGap: 10,
                  padding: '2px 14px',
                  fontSize: 'var(--t-12)',
                  lineHeight: 1.45,
                }}
              >
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)' }}>{m.t}</span>
                <span
                  style={{
                    color: m.c,
                    fontWeight: 500,
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {m.u}
                </span>
                <span style={{ color: 'var(--zinc-200)' }}>{m.m}</span>
              </div>
            ))}
          </div>
          <div
            style={{
              borderTop: 'var(--hair)',
              padding: '8px 14px',
              display: 'flex',
              gap: 8,
              alignItems: 'center',
            }}
          >
            <div className="rx-mono rx-chiclet" style={{ color: 'var(--zinc-600)' }}>shroud ›</div>
            <input className="rx-input" style={{ flex: 1 }} placeholder="Send a message…" />
            <div className="rx-kbd">↵</div>
          </div>
        </div>
      </div>
    </>
  );
}
