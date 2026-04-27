import { openUrl } from '../ipc.js';

/**
 * Stream title + clickable category banner. Rendered above the chat in
 * Command and Focus layouts while the channel is live.
 */
export default function TitleBanner({ channel, compact = false }) {
  if (!channel || !channel.is_live || !channel.title) return null;

  const categoryUrl = buildCategoryUrl(channel);
  const handleCategoryClick = (e) => {
    if (!categoryUrl) return;
    e.preventDefault();
    openUrl(categoryUrl).catch((err) => console.error('open_url', err));
  };

  return (
    <div
      style={{
        padding: compact ? '6px 10px' : '8px 16px',
        borderBottom: 'var(--hair)',
        display: 'flex',
        flexDirection: 'column',
        gap: 3,
      }}
    >
      <div
        style={{
          color: 'var(--zinc-200)',
          fontSize: compact ? 'var(--t-11)' : 'var(--t-12)',
          lineHeight: 1.35,
        }}
      >
        {channel.title}
      </div>
      {channel.game && (
        <div
          style={{
            display: 'flex',
            gap: 6,
            alignItems: 'baseline',
            fontSize: 10,
          }}
        >
          <span className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>IN</span>
          {categoryUrl ? (
            <a
              href={categoryUrl}
              onClick={handleCategoryClick}
              style={{
                color: 'var(--zinc-300)',
                textDecoration: 'none',
                borderBottom: '1px solid var(--zinc-700)',
              }}
              onMouseEnter={(e) => {
                e.currentTarget.style.color = 'var(--zinc-100)';
                e.currentTarget.style.borderColor = 'var(--zinc-400)';
              }}
              onMouseLeave={(e) => {
                e.currentTarget.style.color = 'var(--zinc-300)';
                e.currentTarget.style.borderColor = 'var(--zinc-700)';
              }}
            >
              {channel.game}
            </a>
          ) : (
            <span style={{ color: 'var(--zinc-400)' }}>{channel.game}</span>
          )}
        </div>
      )}
    </div>
  );
}

function buildCategoryUrl(channel) {
  const slug = channel.game_slug;
  const name = channel.game;
  if (!slug && !name) return null;
  switch (channel.platform) {
    case 'twitch': {
      const s = slug ?? encodeURIComponent(name);
      return `https://www.twitch.tv/directory/category/${s}`;
    }
    case 'kick': {
      const s = slug ?? encodeURIComponent(name);
      return `https://kick.com/category/${s}`;
    }
    default:
      return null;
  }
}
