import { useEffect, useState } from 'react';
import { listSocials, openUrl } from '../ipc.js';

/**
 * Channel socials strip. Silently empty while fetching or when a channel
 * has no socials configured. Re-fetches when channelKey changes.
 */
export default function SocialsBanner({ channelKey }) {
  const [links, setLinks] = useState([]);

  useEffect(() => {
    if (!channelKey) {
      setLinks([]);
      return;
    }
    let cancelled = false;
    listSocials(channelKey)
      .then((data) => {
        if (cancelled) return;
        setLinks(Array.isArray(data) ? data : []);
      })
      .catch(() => {
        if (!cancelled) setLinks([]);
      });
    return () => {
      cancelled = true;
    };
  }, [channelKey]);

  if (!links.length) return null;

  return (
    <div
      style={{
        padding: '6px 16px',
        borderBottom: 'var(--hair)',
        display: 'flex',
        flexWrap: 'wrap',
        gap: 6,
        alignItems: 'center',
      }}
    >
      <span className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>SOCIALS</span>
      {links.map((l) => (
        <button
          key={l.id || `${l.name}:${l.url}`}
          type="button"
          className="rx-btn rx-btn-ghost"
          onClick={() => openUrl(l.url).catch(() => {})}
          style={{ padding: '1px 7px', fontSize: 10 }}
          title={l.url}
        >
          {l.title || l.name}
        </button>
      ))}
    </div>
  );
}
