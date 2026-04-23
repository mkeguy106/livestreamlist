/* Direction A — "Command"
 * Command palette is the entire app. Single unified near-black surface.
 * Sidebar rail with tiny status dots. IRC-dense chat. Cmd+K always floating.
 */

const channels = [
  { id: 'shroud',     plat: 't', live: true,  viewers: '47.2k', game: 'VALORANT',         unread: 0 },
  { id: 'xQc',        plat: 't', live: true,  viewers: '82.1k', game: 'Just Chatting',    unread: 14 },
  { id: 'HasanAbi',   plat: 't', live: true,  viewers: '29.4k', game: 'Just Chatting',    unread: 3 },
  { id: 'pokimane',   plat: 't', live: false, viewers: '—',     game: 'offline',          unread: 0 },
  { id: 'Ludwig',     plat: 'y', live: true,  viewers: '11.8k', game: 'Chess',            unread: 0 },
  { id: 'Asmongold',  plat: 't', live: true,  viewers: '34.6k', game: 'World of Warcraft', unread: 1 },
  { id: 'Fextralife', plat: 'y', live: false, viewers: '—',     game: 'offline',          unread: 0 },
  { id: 'Trainwrex',  plat: 'k', live: true,  viewers: '8.9k',  game: 'Slots',            unread: 22 },
  { id: 'sodapoppin', plat: 't', live: false, viewers: '—',     game: 'offline',          unread: 0 },
  { id: 'Mizkif',     plat: 't', live: true,  viewers: '6.1k',  game: 'OSRS',             unread: 0 },
];

const chat = [
  { t: '21:04:11', u: 'vanishh',        c: '#a78bfa', m: 'welcome back btw' },
  { t: '21:04:12', u: 'mikael_ek',      c: '#f87171', m: 'the clutch was so clean' },
  { t: '21:04:12', u: 'kyra.',          c: '#60a5fa', m: 'skillgap' },
  { t: '21:04:13', u: 'tomjones',       c: '#4ade80', m: 'O7' },
  { t: '21:04:14', u: 'NightbotLUL',    c: '#fbbf24', m: '!discord → https://discord.gg/shroud' },
  { t: '21:04:14', u: 'marbled',        c: '#a78bfa', m: 'when the aim be aiming' },
  { t: '21:04:15', u: 'paulieboy',      c: '#fb923c', m: 'thats a W' },
  { t: '21:04:15', u: 'Sub: spiffy',    c: '#71717a', m: 'subscribed for 12 months', sys: true },
  { t: '21:04:16', u: 'kirwan__',       c: '#22d3ee', m: 'shrdBl shrdBl shrdBl' },
  { t: '21:04:17', u: 'reyna.main',     c: '#f472b6', m: 'he cooked' },
  { t: '21:04:17', u: 'ilikepie',       c: '#84cc16', m: 'PogChamp' },
  { t: '21:04:18', u: 'dontban',        c: '#a78bfa', m: '@shroud how do you move so fast' },
  { t: '21:04:19', u: 'rainbowtoast',   c: '#f87171', m: 'literally insane' },
  { t: '21:04:19', u: 'moonbeam',       c: '#60a5fa', m: '🎯' },
  { t: '21:04:20', u: 'alley.cat',      c: '#4ade80', m: 'how does he have that gamesense' },
  { t: '21:04:21', u: 'BTTV_peepoClap', c: '#fbbf24', m: 'peepoClap peepoClap' },
  { t: '21:04:21', u: 'dustin_x',       c: '#fb923c', m: 'vod review time' },
  { t: '21:04:22', u: 'yz0',            c: '#22d3ee', m: 'man is cracked' },
  { t: '21:04:22', u: 'keltoi',         c: '#f472b6', m: 'did he pre-aim that' },
  { t: '21:04:23', u: 'ShinyMTB',       c: '#a78bfa', m: 'yes. he always does' },
];

export default function Command() {
  return (
    <>
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {/* Sidebar */}
        <div
          style={{
            width: 220,
            borderRight: 'var(--hair)',
            display: 'flex',
            flexDirection: 'column',
            background: 'var(--zinc-950)',
            minHeight: 0,
          }}
        >
          <div style={{ padding: '10px 12px 6px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <div className="rx-chiclet">Channels</div>
            <div style={{ flex: 1 }} />
            <div className="rx-chiclet" style={{ color: 'var(--zinc-400)' }}>7/10</div>
          </div>
          <div style={{ flex: 1, overflowY: 'auto' }}>
            {channels.map((ch, i) => {
              const active = i === 0;
              return (
                <div
                  key={ch.id}
                  style={{
                    padding: '6px 12px',
                    display: 'grid',
                    gridTemplateColumns: '10px 1fr auto',
                    columnGap: 10,
                    alignItems: 'center',
                    background: active ? 'var(--zinc-900)' : 'transparent',
                    borderLeft: active ? '2px solid var(--zinc-200)' : '2px solid transparent',
                    cursor: 'pointer',
                    opacity: ch.live ? 1 : 0.45,
                  }}
                >
                  <span className={`rx-status-dot ${ch.live ? 'live' : 'off'}`} />
                  <div style={{ minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>{ch.id}</span>
                      <span className={`rx-plat ${ch.plat}`}>{ch.plat.toUpperCase()}</span>
                    </div>
                    <div
                      className="rx-mono"
                      style={{
                        fontSize: 10,
                        color: 'var(--zinc-500)',
                        whiteSpace: 'nowrap',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                      }}
                    >
                      {ch.live ? ch.game : 'offline'}
                    </div>
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
                    <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>{ch.viewers}</span>
                    {ch.unread > 0 && (
                      <span
                        className="rx-mono"
                        style={{
                          fontSize: 9,
                          padding: '0 4px',
                          borderRadius: 3,
                          background: 'var(--zinc-100)',
                          color: 'var(--zinc-950)',
                          fontWeight: 600,
                        }}
                      >
                        {ch.unread}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          <div style={{ padding: '8px 12px', borderTop: 'var(--hair)', display: 'flex', alignItems: 'center', gap: 8 }}>
            <div className="rx-kbd">⌘⇧A</div>
            <span className="rx-chiclet">Add channel</span>
          </div>
        </div>

        {/* Main */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          {/* Sub-header */}
          <div
            style={{
              height: 40,
              display: 'flex',
              alignItems: 'center',
              gap: 14,
              padding: '0 16px',
              borderBottom: 'var(--hair)',
            }}
          >
            <span className="rx-live-dot pulse" />
            <span style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-100)', fontWeight: 600 }}>shroud</span>
            <span className="rx-plat t">TWITCH</span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>VALORANT</span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>47,204 viewers</span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>up 2h 14m</span>
            <div style={{ flex: 1 }} />
            <button className="rx-btn rx-btn-ghost">Pop out</button>
            <button className="rx-btn">Open stream ↗</button>
          </div>

          {/* Chat list + composer */}
          <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div style={{ flex: 1, overflowY: 'auto', padding: '6px 0' }}>
              {chat.map((m, i) => (
                <div
                  key={i}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '58px 110px 1fr',
                    columnGap: 10,
                    padding: '2px 16px',
                    fontSize: 'var(--t-12)',
                    lineHeight: 1.45,
                    background: i === 6 ? 'rgba(244,244,245,.025)' : 'transparent',
                  }}
                >
                  <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)' }}>{m.t}</span>
                  <span
                    style={{
                      color: m.c,
                      fontWeight: 500,
                      fontSize: 'var(--t-12)',
                      whiteSpace: 'nowrap',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                    }}
                  >
                    {m.sys ? <span style={{ color: 'var(--zinc-500)' }}>★ {m.u}</span> : m.u}
                  </span>
                  <span style={{ color: m.sys ? 'var(--zinc-500)' : 'var(--zinc-200)' }}>{m.m}</span>
                </div>
              ))}
            </div>
            {/* Composer */}
            <div
              style={{
                borderTop: 'var(--hair)',
                padding: '8px 16px',
                display: 'flex',
                gap: 8,
                alignItems: 'center',
              }}
            >
              <div className="rx-mono rx-chiclet" style={{ color: 'var(--zinc-600)' }}>shroud ›</div>
              <input
                className="rx-input"
                style={{ flex: 1 }}
                placeholder="Send a message…  —  : for emotes,  @ for mentions,  / for commands"
              />
              <div className="rx-kbd">↵</div>
            </div>
          </div>
        </div>
      </div>

      {/* Command palette overlay */}
      <div style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}>
        <div
          style={{
            position: 'absolute',
            top: 76,
            left: '50%',
            transform: 'translateX(-50%)',
            width: 560,
            background: 'var(--zinc-925)',
            border: '1px solid var(--zinc-800)',
            borderRadius: 8,
            boxShadow: '0 24px 64px rgba(0,0,0,.7), 0 0 0 1px rgba(255,255,255,.04)',
            pointerEvents: 'auto',
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
            <span style={{ color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>›</span>
            <input
              className="rx-input"
              style={{ border: 'none', background: 'transparent', flex: 1, fontSize: 'var(--t-13)', padding: 0 }}
              defaultValue="add "
            />
            <div className="rx-kbd">esc</div>
          </div>
          {[
            { k: 'Add channel…', hint: 'twitch.tv / youtube.com / kick.com', kbd: '⌘⇧A' },
            { k: 'Add from clipboard', hint: 'twitch.tv/shroud', kbd: '⌘V' },
            { k: 'Add folder…', hint: 'Group channels', kbd: '' },
          ].map((it, i) => (
            <div
              key={i}
              style={{
                padding: '8px 14px',
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                background: i === 0 ? 'var(--zinc-900)' : 'transparent',
                borderLeft: i === 0 ? '2px solid var(--zinc-200)' : '2px solid transparent',
              }}
            >
              <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', whiteSpace: 'nowrap' }}>{it.k}</span>
              <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)', whiteSpace: 'nowrap' }}>
                {it.hint}
              </span>
              <span style={{ flex: 1 }} />
              {it.kbd && <div className="rx-kbd">{it.kbd}</div>}
            </div>
          ))}
          <div
            style={{
              padding: '6px 14px',
              borderTop: 'var(--hair)',
              display: 'flex',
              gap: 12,
              alignItems: 'center',
            }}
          >
            <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>↑↓ navigate</div>
            <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>↵ select</div>
            <div style={{ flex: 1 }} />
            <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>⌘K to close</div>
          </div>
        </div>
      </div>
    </>
  );
}
