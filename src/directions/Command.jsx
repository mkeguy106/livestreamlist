/* Direction A — "Command"
 * Sidebar rail (all channels) + main pane showing the selected channel.
 */

import { useState } from 'react';
import ChatView from '../components/ChatView.jsx';
import ContextMenu from '../components/ContextMenu.jsx';
import SocialsBanner from '../components/SocialsBanner.jsx';
import TitleBanner from '../components/TitleBanner.jsx';
import { usePlayerState } from '../hooks/usePlayerState.js';
import { stopStream } from '../ipc.js';
import { formatUptime, formatViewers } from '../utils/format.js';

export default function Command({ ctx }) {
  const {
    livestreams,
    selectedKey,
    setSelectedKey,
    openAddDialog,
    launchStream,
    openInBrowser,
    removeChannel,
    setFavorite,
  } = ctx;

  const playing = usePlayerState();
  const [menu, setMenu] = useState(null); // { x, y, channel }

  // Sort: live first (by viewers desc), then offline alpha
  const sorted = [...livestreams].sort((a, b) => {
    if (a.is_live !== b.is_live) return a.is_live ? -1 : 1;
    if (a.is_live) return (b.viewers ?? 0) - (a.viewers ?? 0);
    return a.display_name.localeCompare(b.display_name);
  });

  const liveCount = sorted.filter((l) => l.is_live).length;
  const selected = sorted.find((l) => l.unique_key === selectedKey) ?? sorted[0];

  return (
    <>
      <div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
        {/* Sidebar */}
        <div
          style={{
            width: 240,
            borderRight: 'var(--hair)',
            display: 'flex',
            flexDirection: 'column',
            background: 'var(--zinc-950)',
            minHeight: 0,
            flexShrink: 0,
          }}
        >
          <div style={{ padding: '10px 12px 6px', display: 'flex', alignItems: 'center', gap: 8 }}>
            <div className="rx-chiclet">Channels</div>
            <div style={{ flex: 1 }} />
            <div className="rx-chiclet" style={{ color: 'var(--zinc-400)' }}>
              {liveCount}/{sorted.length}
            </div>
          </div>
          <div style={{ flex: 1, overflowY: 'auto' }}>
            {sorted.map((ch) => {
              const active = ch.unique_key === selected?.unique_key;
              const isPlaying = playing.has(ch.unique_key);
              return (
                <button
                  key={ch.unique_key}
                  type="button"
                  onClick={() => setSelectedKey(ch.unique_key)}
                  onDoubleClick={() => {
                    if (ch.is_live) launchStream(ch.unique_key);
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setSelectedKey(ch.unique_key);
                    setMenu({ x: e.clientX, y: e.clientY, channel: ch });
                  }}
                  title={ch.is_live ? 'Double-click to play' : undefined}
                  style={{
                    width: '100%',
                    textAlign: 'left',
                    background: active ? 'var(--zinc-900)' : 'transparent',
                    borderLeft: active ? '2px solid var(--zinc-200)' : '2px solid transparent',
                    borderTop: 'none',
                    borderRight: 'none',
                    borderBottom: 'none',
                    padding: '6px 12px',
                    display: 'grid',
                    gridTemplateColumns: '10px 1fr auto',
                    columnGap: 10,
                    alignItems: 'center',
                    color: 'inherit',
                    cursor: 'pointer',
                    opacity: ch.is_live ? 1 : 0.45,
                    fontFamily: 'inherit',
                  }}
                >
                  <span className={`rx-status-dot ${ch.is_live ? 'live' : 'off'}`} />
                  <div style={{ minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>
                        {ch.display_name}
                      </span>
                      {isPlaying && (
                        <span
                          title="Playing"
                          style={{
                            color: 'var(--ok)',
                            fontSize: 9,
                            lineHeight: 1,
                          }}
                        >
                          ▶
                        </span>
                      )}
                      <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
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
                      {ch.is_live ? (ch.game ?? 'live') : 'offline'}
                    </div>
                  </div>
                  <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
                    <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>
                      {ch.is_live ? formatViewers(ch.viewers) : '—'}
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
          <button
            type="button"
            onClick={openAddDialog}
            style={{
              padding: '8px 12px',
              borderTop: 'var(--hair)',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              background: 'transparent',
              border: 'none',
              color: 'var(--zinc-300)',
              cursor: 'pointer',
              fontFamily: 'inherit',
              textAlign: 'left',
            }}
          >
            <div className="rx-kbd">N</div>
            <span className="rx-chiclet">Add channel</span>
          </button>
        </div>

        {/* Main */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          {selected ? (
            <SelectedPane
              channel={selected}
              onLaunch={() => launchStream(selected.unique_key)}
              onOpenBrowser={() => openInBrowser(selected.unique_key)}
              onFavorite={() => setFavorite(selected.unique_key, true)}
            />
          ) : (
            <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--zinc-500)' }}>
              no channels
            </div>
          )}
        </div>
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
        >
          <ContextMenu.Item
            disabled={!menu.channel.is_live || playing.has(menu.channel.unique_key)}
            onClick={() => {
              launchStream(menu.channel.unique_key);
              setMenu(null);
            }}
          >
            {menu.channel.is_live ? 'Play' : 'Play (offline)'}
          </ContextMenu.Item>
          <ContextMenu.Item
            disabled={!playing.has(menu.channel.unique_key)}
            onClick={() => {
              stopStream(menu.channel.unique_key).catch(() => {});
              setMenu(null);
            }}
          >
            Stop
          </ContextMenu.Item>
          <ContextMenu.Item
            onClick={() => {
              openInBrowser(menu.channel.unique_key);
              setMenu(null);
            }}
          >
            Open in browser
          </ContextMenu.Item>
          <ContextMenu.Separator />
          <ContextMenu.Item
            onClick={() => {
              setFavorite(menu.channel.unique_key, true);
              setMenu(null);
            }}
          >
            Pin as favorite
          </ContextMenu.Item>
          <ContextMenu.Separator />
          <ContextMenu.Item
            danger
            onClick={() => {
              removeChannel(menu.channel.unique_key);
              setMenu(null);
            }}
          >
            Delete channel
          </ContextMenu.Item>
        </ContextMenu>
      )}
    </>
  );
}

function SelectedPane({ channel, onLaunch, onOpenBrowser }) {
  return (
    <>
      <div
        style={{
          height: 40,
          display: 'flex',
          alignItems: 'center',
          gap: 14,
          padding: '0 16px',
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        {channel.is_live ? <span className="rx-live-dot pulse" /> : <span className="rx-status-dot off" />}
        <span style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-100)', fontWeight: 600 }}>
          {channel.display_name}
        </span>
        <span className={`rx-plat ${channel.platform.charAt(0)}`}>{channel.platform.toUpperCase()}</span>
        {channel.is_live && (
          <>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              {channel.game ?? ''}
            </span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              {formatViewers(channel.viewers)} viewers
            </span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              up {formatUptime(channel.started_at)}
            </span>
          </>
        )}
        <div style={{ flex: 1 }} />
        <button className="rx-btn rx-btn-ghost" onClick={onOpenBrowser}>Open in browser</button>
        <button
          className="rx-btn rx-btn-primary"
          disabled={!channel.is_live}
          onClick={onLaunch}
          style={channel.is_live ? undefined : { opacity: 0.4, cursor: 'not-allowed' }}
        >
          {channel.is_live ? 'Play ↗' : 'Offline'}
        </button>
      </div>

      <ChatView
        channelKey={channel.unique_key}
        variant="irc"
        header={
          <>
            <TitleBanner channel={channel} />
            <SocialsBanner channelKey={channel.unique_key} />
          </>
        }
      />
    </>
  );
}
