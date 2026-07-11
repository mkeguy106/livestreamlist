/* Direction C — "Focus"
 * One explicitly-picked featured channel (ctx.focusKey). Opens blank with a
 * centered live-channel picker; a live-only strip across the top is the
 * quick switcher (offline channels never appear here). The featured channel
 * going offline falls back to the picker — App clears focusKey; the
 * live-gated lookup below also covers the render in between.
 */

import ChatView from '../components/ChatView.jsx';
import FocusLiveStrip from '../components/FocusLiveStrip.jsx';
import FocusPicker from '../components/FocusPicker.jsx';
import PlaySplitButton from '../components/PlaySplitButton.jsx';
import SocialsBanner from '../components/SocialsBanner.jsx';
import TitleBanner from '../components/TitleBanner.jsx';
import Tooltip from '../components/Tooltip.jsx';
import VideoPanel from '../components/VideoPanel.jsx';
import { formatUptime, formatViewers } from '../utils/format.js';

export default function Focus({ ctx }) {
  const {
    livestreams,
    focusKey,
    setFocusKey,
    launchStream,
    openInBrowser,
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  } = ctx;

  const featured = focusKey
    ? livestreams.find((l) => l.unique_key === focusKey && l.is_live) ?? null
    : null;

  return (
    <>
      <FocusLiveStrip
        livestreams={livestreams}
        focusedKey={featured?.unique_key ?? null}
        onPick={setFocusKey}
      />

      {featured ? (
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          <div style={{ flex: '1 1 60%', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
            <FeaturedStream
              channel={featured}
              onLaunch={(quality) => launchStream(featured.unique_key, quality)}
              onOpenBrowser={() => openInBrowser(featured.unique_key)}
            />
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
              channelKey={featured.unique_key}
              variant="irc"
              isLive
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
                      {featured.display_name}
                    </span>
                  </div>
                  <TitleBanner channel={featured} />
                  <SocialsBanner channelKey={featured.unique_key} />
                </>
              }
            />
          </div>
        </div>
      ) : (
        <div
          style={{
            flex: 1,
            display: 'flex',
            alignItems: 'flex-start',
            justifyContent: 'center',
            paddingTop: 90,
            minHeight: 0,
            overflow: 'hidden',
          }}
        >
          <FocusPicker livestreams={livestreams} onPick={setFocusKey} />
        </div>
      )}
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
        {channel.is_live && channel.platform === 'twitch' ? (
          /* key forces a clean remount per channel — mount-seeded state (muted/volume) must not bleed across tab switches */
          <VideoPanel
            key={channel.unique_key}
            channelKey={channel.unique_key}
            thumbnailUrl={channel.thumbnail_url}
            variant="focus"
          />
        ) : (
          <>
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
                  onClick={channel.is_live ? () => onLaunch() : undefined}
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
          </>
        )}
      </div>

      <div style={{ display: 'flex', gap: 8, padding: '0 16px 12px' }}>
        <PlaySplitButton onLaunch={onLaunch} disabled={!channel.is_live} />
        <div style={{ flex: 1 }} />
        <button type="button" className="rx-btn" onClick={onOpenBrowser}>
          Open on {channel.platform.charAt(0).toUpperCase() + channel.platform.slice(1)} ↗
        </button>
      </div>
    </>
  );
}
