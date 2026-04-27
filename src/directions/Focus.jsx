/* Direction C — "Focus"
 * One featured channel (selectedKey) with a tab strip of all channels above.
 */

import ChatView from '../components/ChatView.jsx';
import SocialsBanner from '../components/SocialsBanner.jsx';
import TitleBanner from '../components/TitleBanner.jsx';
import Tooltip from '../components/Tooltip.jsx';
import { formatUptime, formatViewers } from '../utils/format.js';

export default function Focus({ ctx }) {
  const {
    livestreams,
    selectedKey,
    setSelectedKey,
    openAddDialog,
    launchStream,
    openInBrowser,
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  } = ctx;

  // Tabs: live first (viewer-desc), then offline alpha
  const sorted = [...livestreams].sort((a, b) => {
    if (a.is_live !== b.is_live) return a.is_live ? -1 : 1;
    if (a.is_live) return (b.viewers ?? 0) - (a.viewers ?? 0);
    return a.display_name.localeCompare(b.display_name);
  });

  const featured = sorted.find((l) => l.unique_key === selectedKey) ?? sorted[0];

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
          flexShrink: 0,
        }}
      >
        {sorted.map((t) => {
          const active = t.unique_key === featured?.unique_key;
          const letter = t.platform.charAt(0);
          return (
            <button
              key={t.unique_key}
              type="button"
              onClick={() => setSelectedKey(t.unique_key)}
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
                color: t.is_live ? 'var(--zinc-100)' : 'var(--zinc-600)',
                cursor: 'pointer',
                fontFamily: 'inherit',
              }}
            >
              <span className={`rx-status-dot ${t.is_live ? 'live' : 'off'}`} />
              <span style={{ fontSize: 'var(--t-12)', fontWeight: active ? 600 : 500 }}>{t.display_name}</span>
              <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
              {t.is_live && (
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                  {formatViewers(t.viewers)}
                </span>
              )}
            </button>
          );
        })}
        <button
          type="button"
          onClick={openAddDialog}
          style={{
            padding: '0 12px',
            display: 'flex',
            alignItems: 'center',
            color: 'var(--zinc-500)',
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'inherit',
          }}
        >
          <span className="rx-chiclet">＋</span>
        </button>
      </div>

      {/* Split */}
      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        <div style={{ flex: '1 1 60%', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          {featured ? (
            <FeaturedStream
              channel={featured}
              onLaunch={() => launchStream(featured.unique_key)}
              onOpenBrowser={() => openInBrowser(featured.unique_key)}
            />
          ) : (
            <div
              style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--zinc-500)' }}
            >
              no channel selected
            </div>
          )}
        </div>

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
          <ChatView
            channelKey={featured?.unique_key}
            variant="irc"
            isLive={Boolean(featured?.is_live)}
            onUsernameOpen={onUsernameOpen}
            onUsernameContext={onUsernameContext}
            onUsernameHover={onUsernameHover}
            header={
              <>
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
                  <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                    {featured?.display_name ?? ''}
                  </span>
                </div>
                <TitleBanner channel={featured} />
                <SocialsBanner channelKey={featured?.unique_key} />
              </>
            }
          />
        </div>
      </div>
    </>
  );
}

function FeaturedStream({ channel, onLaunch, onOpenBrowser }) {
  const letter = channel.platform.charAt(0);
  return (
    <>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '10px 16px',
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        {channel.is_live ? <span className="rx-live-dot pulse" /> : <span className="rx-status-dot off" />}
        <span style={{ fontSize: 'var(--t-14)', color: 'var(--zinc-100)', fontWeight: 600 }}>
          {channel.display_name}
        </span>
        <span className={`rx-plat ${letter}`}>{channel.platform.toUpperCase()}</span>
        {channel.is_live && channel.title && (
          <>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span
              style={{
                fontSize: 'var(--t-12)',
                color: 'var(--zinc-300)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                maxWidth: 320,
              }}
            >
              <Tooltip wrap text={channel.title}>
                <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {channel.title}
                </span>
              </Tooltip>
            </span>
          </>
        )}
        <div style={{ flex: 1 }} />
        {channel.is_live && (
          <>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              {formatViewers(channel.viewers)}
            </span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>
              · {formatUptime(channel.started_at)}
            </span>
          </>
        )}
      </div>

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
        {channel.thumbnail_url && (
          <img
            src={channel.thumbnail_url}
            alt=""
            style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.5 }}
          />
        )}
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <Tooltip text={channel.is_live ? 'Launch via streamlink' : 'channel offline'}>
            <button
              type="button"
              onClick={channel.is_live ? onLaunch : undefined}
              disabled={!channel.is_live}
              style={{
                width: 64,
                height: 64,
                borderRadius: '50%',
                border: '1px solid var(--zinc-700)',
                background: channel.is_live ? 'rgba(9,9,11,.8)' : 'rgba(9,9,11,.4)',
                color: channel.is_live ? 'var(--zinc-100)' : 'var(--zinc-500)',
                cursor: channel.is_live ? 'pointer' : 'not-allowed',
                fontSize: 22,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              ▶
            </button>
          </Tooltip>
        </div>
      </div>

      <div style={{ display: 'flex', gap: 8, padding: '0 16px 12px' }}>
        <button
          type="button"
          className="rx-btn rx-btn-ghost"
          disabled={!channel.is_live}
          onClick={onLaunch}
        >
          Launch via streamlink
        </button>
        <div style={{ flex: 1 }} />
        <button type="button" className="rx-btn" onClick={onOpenBrowser}>
          Open on {channel.platform.charAt(0).toUpperCase() + channel.platform.slice(1)} ↗
        </button>
      </div>
    </>
  );
}
