import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import Tooltip from './Tooltip.jsx';

/**
 * Anchored portal popover for a single chat user. Caller mounts one of these
 * per ChatView (or per app); state comes from useUserCard.
 *
 * Props:
 *   open, anchor, user, channelKey, metadata, profile, profileLoading,
 *   profileError, onClose, onOpenHistory, onOpenChannel
 */
export default function UserCard({
  open,
  anchor,
  user,
  metadata,
  profile,
  profileLoading,
  profileError,
  onClose,
  onOpenHistory,
  onOpenChannel,
  onCardHover,
}) {
  const cardRef = useRef(null);
  const [pos, setPos] = useState(null);

  // Position the card with viewport flip-to-fit.
  useLayoutEffect(() => {
    if (!open || !anchor || !cardRef.current) return;
    const cw = cardRef.current.offsetWidth;
    const ch = cardRef.current.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    // anchor is a DOMRect — fields are x/y/width/height (not h/w).
    let x = anchor.x;
    let y = anchor.y + anchor.height + 8;
    if (y + ch > vh) y = anchor.y - ch - 8; // flip up
    if (x + cw > vw) x = vw - cw - 8;       // clamp right
    if (x < 8) x = 8;
    if (y < 8) y = 8;
    setPos({ x, y });
  }, [open, anchor]);

  // Close on outside click or chat scroll.
  useEffect(() => {
    if (!open) return;
    const onDown = e => {
      if (!cardRef.current) return;
      if (cardRef.current.contains(e.target)) return;
      // Treat the original anchor as "inside" so a click on the username
      // toggles rather than chains close→reopen.
      if (e.target.closest?.('[data-user-card-anchor]')) return;
      onClose();
    };
    const onWheel = e => {
      // Close on user-initiated scroll only — never on programmatic scroll
      // (the chat list auto-scrolls as new messages arrive, which would
      // dismiss the card the moment it opens).
      if (cardRef.current?.contains(e.target)) return;
      onClose();
    };
    document.addEventListener('mousedown', onDown, true);
    document.addEventListener('wheel', onWheel, { capture: true, passive: true });
    return () => {
      document.removeEventListener('mousedown', onDown, true);
      document.removeEventListener('wheel', onWheel, { capture: true });
    };
  }, [open, onClose]);

  if (!open || !user) return null;

  const display = user.display_name || user.login;
  const nameColor = user.color || 'var(--zinc-100)';

  const card = (
    <div
      ref={cardRef}
      role="dialog"
      aria-label={`User card for ${display}`}
      onMouseEnter={() => onCardHover?.(true)}
      onMouseLeave={() => onCardHover?.(false)}
      style={{
        position: 'fixed',
        left: pos?.x ?? -9999,
        top: pos?.y ?? -9999,
        zIndex: 200,
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 'var(--r-2)',
        boxShadow: '0 12px 32px rgba(0,0,0,.6)',
        padding: '12px 14px',
        minWidth: 280,
        maxWidth: 320,
        font: 'var(--t-12) var(--font-sans)',
        color: 'var(--zinc-200)',
      }}
    >
      <Header
        display={display}
        login={user.login}
        nameColor={nameColor}
        avatar={profile?.profile_image_url}
        platformLetter="t"
        badges={user.badges /* may be undefined; fall back below */}
      />

      <Divider />

      {profileError ? (
        <ErrorBanner
          message={
            profileError.includes('sign in') || profileError.includes('not signed in')
              ? 'Sign in to Twitch in Settings to load profile data.'
              : 'Couldn’t load profile.'
          }
        />
      ) : (
        <Stats
          loading={profileLoading}
          profile={profile}
          sessionMessageCount={undefined /* wired by parent via prop in Task 18 */}
        />
      )}

      {profile?.description ? (
        <>
          <Divider />
          <div style={{ font: 'var(--t-11) var(--font-sans)', color: 'var(--zinc-400)', lineHeight: 1.4 }}>
            {profile.description}
          </div>
        </>
      ) : null}

      {(metadata?.nickname || metadata?.note) ? (
        <>
          <Divider />
          {metadata.nickname ? (
            <div style={{ font: 'var(--t-11) var(--font-sans)', color: 'var(--zinc-300)' }}>
              ★ Nickname: {metadata.nickname}
            </div>
          ) : null}
          {metadata.note ? (
            <div style={{ font: 'var(--t-11) var(--font-sans)', color: 'var(--zinc-300)' }}>
              ✎ Note: {metadata.note}
            </div>
          ) : null}
        </>
      ) : null}

      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <button className="rx-btn rx-btn-ghost" onClick={onOpenHistory} style={{ flex: 1 }}>
          Chat History
        </button>
        <button className="rx-btn rx-btn-ghost" onClick={onOpenChannel} style={{ flex: 1 }}>
          Open Channel
        </button>
      </div>
    </div>
  );

  return createPortal(card, document.body);
}

function Header({ display, login, nameColor, avatar, platformLetter, badges = [] }) {
  return (
    <div style={{ display: 'flex', gap: 10, alignItems: 'flex-start' }}>
      <div
        style={{
          width: 44, height: 44, borderRadius: '50%', overflow: 'hidden',
          background: 'var(--zinc-800)', flexShrink: 0,
        }}
      >
        {avatar ? <img src={avatar} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} /> : null}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8 }}>
          <span style={{ color: nameColor, fontWeight: 600, fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {display}
          </span>
          <span className={`rx-plat ${platformLetter}`}>{platformLetter}</span>
        </div>
        {display.toLowerCase() !== login.toLowerCase() ? (
          <div style={{ color: 'var(--zinc-400)', fontSize: 11 }}>@{login}</div>
        ) : null}
        {badges?.length ? (
          <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
            {badges.map(b => (
              <Tooltip key={b.id} text={b.title}>
                <img src={b.url} alt="" width={18} height={18} />
              </Tooltip>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function Divider() {
  return <div style={{ borderTop: 'var(--hair)', margin: '10px 0' }} />;
}

function Stats({ loading, profile, sessionMessageCount }) {
  const rows = [];
  if (loading) {
    rows.push(<Skeleton key="s1" />, <Skeleton key="s2" />, <Skeleton key="s3" />);
  } else if (profile) {
    if (profile.pronouns) rows.push(<Row key="pn" label="Pronouns" value={profile.pronouns} />);
    if (profile.follower_count != null)
      rows.push(<Row key="fc" label="Followers" value={profile.follower_count.toLocaleString('de-DE')} />);
    if (profile.created_at) rows.push(<Row key="ca" label="Account age" value={formatAge(profile.created_at)} />);
    if (profile.following_since)
      rows.push(<Row key="fs" label="Following since" value={formatAge(profile.following_since)} />);
  }
  if (sessionMessageCount != null)
    rows.push(<Row key="sm" label="Session msgs" value={String(sessionMessageCount)} />);
  return <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>{rows}</div>;
}

function Row({ label, value }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12 }}>
      <span style={{ color: 'var(--zinc-400)' }}>{label}</span>
      <span style={{ color: 'var(--zinc-200)' }}>{value}</span>
    </div>
  );
}

function Skeleton() {
  return (
    <div
      style={{
        height: 8, borderRadius: 2, background: 'var(--zinc-800)',
        animation: 'usercard-pulse 1.4s ease-in-out infinite',
      }}
    />
  );
}

function ErrorBanner({ message }) {
  return (
    <div
      role="alert"
      style={{
        background: 'rgba(239,68,68,.08)', border: '1px solid rgba(239,68,68,.4)',
        borderRadius: 'var(--r-2)', padding: '6px 8px', color: 'var(--zinc-300)',
        fontSize: 11,
      }}
    >
      {message}
    </div>
  );
}

function formatAge(isoStr) {
  const then = new Date(isoStr);
  const ms = Date.now() - then.getTime();
  const days = Math.floor(ms / (1000 * 60 * 60 * 24));
  const years = Math.floor(days / 365);
  const months = Math.floor((days % 365) / 30);
  if (years > 0) return `${years} y ${months} mo`;
  if (months > 0) return `${months} mo`;
  return `${days} d`;
}
