// src/DetachedChatRoot.jsx
//
// Mounted by main.jsx when the URL fragment is #chat-detach=<key>. Renders
// a single ChatView with a thin titlebar above it. Re-dock button calls
// chat_reattach IPC; close button (X in the titlebar) is the system close
// = dismiss path. Closing emits chat-detach:closed which the main window
// uses to drop the channel from detachedKeys.

import { useEffect } from 'react';
import ChatView from './components/ChatView.jsx';
import SocialsBanner from './components/SocialsBanner.jsx';
import TitleBanner from './components/TitleBanner.jsx';
import WindowControls from './components/WindowControls.jsx';
import { useDragHandler } from './hooks/useDragRegion.js';
import { useLivestreams } from './hooks/useLivestreams.js';
import { useUserCard } from './hooks/useUserCard.js';
import { chatReattach } from './ipc.js';

export default function DetachedChatRoot({ channelKey }) {
  const { livestreams } = useLivestreams();
  const onTitlebarMouseDown = useDragHandler();
  const card = useUserCard();
  const channel = livestreams.find((l) => l.unique_key === channelKey);

  // Re-set window title when channel display name resolves.
  useEffect(() => {
    if (channel?.display_name) {
      document.title = `Chat — ${channel.display_name}`;
    }
  }, [channel?.display_name]);

  const onRedock = () => {
    chatReattach(channelKey).catch((e) => console.error('chat_reattach', e));
  };

  return (
    <div
      style={{
        height: '100vh',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--zinc-950)',
      }}
    >
      <div
        onMouseDown={onTitlebarMouseDown}
        style={{
          height: 32,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          gap: 10,
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        <span
          className={`rx-status-dot ${channel?.is_live ? 'live' : 'off'}`}
          style={{ pointerEvents: 'none' }}
        />
        <span
          style={{
            fontSize: 'var(--t-12)',
            color: 'var(--zinc-200)',
            fontWeight: 500,
            pointerEvents: 'none',
          }}
        >
          {channel?.display_name ?? channelKey}
        </span>
        <span
          className={`rx-plat ${(channel?.platform ?? channelKey.split(':')[0]).charAt(0)}`}
          style={{ pointerEvents: 'none' }}
        >
          {(channel?.platform ?? channelKey.split(':')[0]).charAt(0).toUpperCase()}
        </span>
        <div style={{ flex: 1 }} />
        <button
          type="button"
          className="rx-btn rx-btn-ghost"
          onClick={onRedock}
          title="Re-dock to main window"
          style={{ padding: '2px 8px', fontSize: 11 }}
        >
          ⤴ Re-dock
        </button>
        <WindowControls />
      </div>
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        {channel ? (
          <ChatView
            channelKey={channelKey}
            variant="irc"
            isLive={Boolean(channel.is_live)}
            isActiveTab={true}
            header={
              <>
                <TitleBanner channel={channel} />
                <SocialsBanner channelKey={channelKey} />
              </>
            }
            onUsernameOpen={card.openFor}
            onUsernameContext={() => {}}
            onUsernameHover={() => {}}
          />
        ) : (
          <div
            style={{
              flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
              color: 'var(--zinc-500)', fontSize: 'var(--t-12)',
            }}
          >
            Channel not found — close this window and reopen from the main app.
          </div>
        )}
      </div>
    </div>
  );
}
