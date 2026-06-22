import { Fragment, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import Tooltip from './Tooltip.jsx';
import { readableColor } from '../utils/color.js';
import { formatDate } from '../utils/format.js';

/**
 * Anchored portal popover for a single chat user.
 *
 * Redesign (toolbar-header direction): actions (History / Channel / Block /
 * More) live in a header toolbar beside the identity; a platform-color edge
 * accents the card; global profile stats and your-relationship-to-the-user
 * data sit in separate zones; nickname / note are inline-editable.
 *
 * Props:
 *   open, anchor, user, metadata, profile, profileLoading, profileError,
 *   sessionMessageCount,
 *   onClose, onOpenHistory, onOpenChannel, onCardHover,
 *   onToggleBlocked, onEditNickname, onEditNote, onMore(point)
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
  onToggleBlocked,
  onEditNickname,
  onEditNote,
  onMore,
  sessionMessageCount,
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
  // var(--zinc-100) when no color set; otherwise apply the same lightness floor
  // we use in chat rows so dark Twitch colors don't vanish in the popover.
  const nameColor = user.color ? readableColor(user.color) : 'var(--zinc-100)';
  const blocked = !!metadata?.blocked;
  const isSignedOut = !!profileError;
  const accentColor = blocked ? 'var(--zinc-700)' : 'var(--twitch)';

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
        width: 300,
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 'var(--r-2)',
        boxShadow: '0 12px 32px rgba(0,0,0,.6)',
        overflow: 'hidden',
        font: 'var(--t-12) var(--font-sans)',
        color: 'var(--zinc-200)',
      }}
    >
      {blocked ? <BlockedBanner /> : null}

      <Header
        display={display}
        login={user.login}
        nameColor={nameColor}
        avatar={profile?.profile_image_url}
        broadcasterType={profile?.broadcaster_type}
        blocked={blocked}
        isSignedOut={isSignedOut}
        accentColor={accentColor}
        onOpenHistory={onOpenHistory}
        onOpenChannel={onOpenChannel}
        onToggleBlocked={onToggleBlocked}
        onMore={onMore}
      />

      {isSignedOut ? (
        <SignedOutNotice message={profileError} />
      ) : profileLoading ? (
        <LoadingStats />
      ) : profile ? (
        <>
          {profile.description ? (
            <div
              className="uc-clamp"
              style={{
                padding: '0 12px 11px',
                font: '11px var(--font-sans)',
                color: 'var(--zinc-400)',
                lineHeight: 1.45,
              }}
            >
              {profile.description}
            </div>
          ) : null}
          <StatsRow profile={profile} />
        </>
      ) : null}

      <RelationshipZone
        sessionMessageCount={sessionMessageCount}
        followingSince={profile?.following_since}
        nickname={metadata?.nickname}
        note={metadata?.note}
        canEdit={!!onEditNickname}
        onEditNickname={onEditNickname}
        onEditNote={onEditNote}
        onMore={onMore}
      />
    </div>
  );

  return createPortal(card, document.body);
}

function Header({
  display,
  login,
  nameColor,
  avatar,
  broadcasterType,
  blocked,
  isSignedOut,
  accentColor,
  onOpenHistory,
  onOpenChannel,
  onToggleBlocked,
  onMore,
}) {
  const isPartnerOrAffiliate = broadcasterType === 'partner' || broadcasterType === 'affiliate';
  return (
    <div
      style={{
        display: 'flex',
        gap: 11,
        alignItems: 'flex-start',
        padding: '13px 12px 11px',
        borderLeft: `2px solid ${accentColor}`,
      }}
    >
      <div
        style={{
          width: 42,
          height: 42,
          borderRadius: '50%',
          overflow: 'hidden',
          background: 'var(--zinc-800)',
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--zinc-500)',
          font: '600 16px var(--font-sans)',
          filter: blocked ? 'grayscale(1)' : undefined,
          opacity: blocked ? 0.6 : 1,
        }}
      >
        {avatar ? (
          <img src={avatar} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} />
        ) : (
          display.charAt(0).toUpperCase()
        )}
      </div>

      <div style={{ flex: 1, minWidth: 0, opacity: blocked ? 0.85 : 1 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span
            style={{
              color: blocked ? 'var(--zinc-300)' : nameColor,
              fontWeight: 600,
              fontSize: 13,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}
          >
            {display}
          </span>
          <span className="rx-plat t">t</span>
          {isPartnerOrAffiliate ? (
            <span
              style={{
                font: '600 8px var(--font-mono)',
                letterSpacing: '.04em',
                textTransform: 'uppercase',
                color: 'var(--twitch)',
                border: '1px solid rgba(167,139,250,.28)',
                borderRadius: 3,
                padding: '0 4px',
                lineHeight: '14px',
              }}
            >
              {broadcasterType === 'partner' ? 'Partner' : 'Affiliate'}
            </span>
          ) : null}
        </div>

        <div style={{ color: 'var(--zinc-500)', font: '11px var(--font-mono)', marginTop: 2 }}>
          @{login}
        </div>

        <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginTop: 8 }}>
          <button
            type="button"
            className="rx-btn rx-btn-ghost"
            onClick={onOpenHistory}
            style={{ padding: '3px 8px', gap: 5, fontSize: 11 }}
          >
            <span style={{ fontSize: 11 }}>◷</span> History
          </button>
          {blocked ? (
            <button
              type="button"
              className="rx-btn"
              onClick={onToggleBlocked}
              style={{ padding: '3px 9px', fontSize: 11 }}
            >
              Unblock
            </button>
          ) : (
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={onOpenChannel}
              style={{ padding: '3px 8px', gap: 5, fontSize: 11 }}
            >
              <span style={{ fontSize: 11 }}>↗</span> Channel
            </button>
          )}

          {!blocked && !isSignedOut ? (
            <>
              <div style={{ flex: 1 }} />
              <Tooltip text="Block user" align="right">
                <button
                  type="button"
                  aria-label="Block user"
                  className="uc-iconbtn"
                  onClick={onToggleBlocked}
                  style={{ color: 'var(--live)' }}
                >
                  <span style={{ fontSize: 13, lineHeight: 1 }}>⊘</span>
                </button>
              </Tooltip>
              <Tooltip text="More actions" align="right">
                <button
                  type="button"
                  aria-label="More actions"
                  className="uc-iconbtn"
                  onClick={e => onMore?.(pointFromEvent(e))}
                >
                  <span style={{ fontSize: 14, lineHeight: 1 }}>⋯</span>
                </button>
              </Tooltip>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function StatsRow({ profile }) {
  const cells = [];
  if (profile.follower_count != null) {
    cells.push(
      <Stat key="fc" label="Followers" value={profile.follower_count.toLocaleString('de-DE')} mono />,
    );
  }
  if (profile.created_at) {
    cells.push(
      <Tooltip key="ca" text={formatDate(profile.created_at)}>
        <Stat label="Account" value={formatAge(profile.created_at)} mono />
      </Tooltip>,
    );
  }
  if (profile.pronouns) {
    cells.push(<Stat key="pn" label="Pronouns" value={profile.pronouns} />);
  }
  if (cells.length === 0) return null;

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '9px 12px',
        borderTop: 'var(--hair)',
        borderBottom: 'var(--hair)',
        fontVariantNumeric: 'tabular-nums',
      }}
    >
      {cells.map((cell, i) => (
        <Fragment key={i}>
          {i > 0 ? (
            <div style={{ width: 1, alignSelf: 'stretch', background: 'rgba(255,255,255,.06)' }} />
          ) : null}
          {cell}
        </Fragment>
      ))}
    </div>
  );
}

function Stat({ label, value, mono }) {
  return (
    <div>
      <div
        style={{
          font: '9px var(--font-mono)',
          letterSpacing: '.06em',
          textTransform: 'uppercase',
          color: 'var(--zinc-600)',
        }}
      >
        {label}
      </div>
      <div
        style={
          mono
            ? { font: '13px var(--font-mono)', color: 'var(--zinc-100)', marginTop: 2 }
            : { font: '12px var(--font-sans)', color: 'var(--zinc-200)', marginTop: 3 }
        }
      >
        {value}
      </div>
    </div>
  );
}

function RelationshipZone({
  sessionMessageCount,
  followingSince,
  nickname,
  note,
  canEdit,
  onEditNickname,
  onEditNote,
  onMore,
}) {
  const summary = [];
  if (sessionMessageCount != null) summary.push(`${sessionMessageCount} msgs`);
  if (followingSince) summary.push(`follows ${formatAge(followingSince)}`);

  const hasMeta = !!(nickname || note);

  return (
    <div style={{ background: 'rgba(255,255,255,.02)' }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 8,
          padding: '8px 12px 5px',
        }}
      >
        <span
          style={{
            font: '9px var(--font-mono)',
            letterSpacing: '.1em',
            textTransform: 'uppercase',
            color: 'var(--zinc-500)',
          }}
        >
          In this chat
        </span>
        {summary.length ? (
          <span
            style={{
              font: '11px var(--font-mono)',
              color: 'var(--zinc-400)',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {summary.join(' · ')}
          </span>
        ) : null}
      </div>

      {!canEdit ? null : hasMeta ? (
        <div
          className="uc-editrow"
          style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 12px 9px' }}
        >
          {nickname ? (
            <button
              type="button"
              onClick={onEditNickname}
              style={{
                all: 'unset',
                cursor: 'pointer',
                color: 'var(--zinc-200)',
                fontSize: 12,
                display: 'inline-flex',
                alignItems: 'center',
                gap: 5,
              }}
            >
              <span style={{ color: 'var(--zinc-600)' }}>★</span>
              {nickname}
            </button>
          ) : null}
          {nickname && note ? <span style={{ color: 'var(--zinc-700)' }}>·</span> : null}
          {note ? (
            <button
              type="button"
              onClick={onEditNote}
              style={{
                all: 'unset',
                cursor: 'pointer',
                flex: 1,
                minWidth: 0,
                color: 'var(--zinc-300)',
                font: '11px var(--font-sans)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              <span style={{ color: 'var(--zinc-600)' }}>✎</span> {note}
            </button>
          ) : (
            <div style={{ flex: 1 }} />
          )}
          <button
            type="button"
            className="rx-btn rx-btn-ghost uc-row-edit"
            onClick={e => onMore?.(pointFromEvent(e))}
            style={{ fontSize: 11, padding: '2px 7px' }}
          >
            Edit
          </button>
        </div>
      ) : (
        <div style={{ padding: '2px 12px 9px' }}>
          <button
            type="button"
            className="rx-btn rx-btn-ghost"
            onClick={e => onMore?.(pointFromEvent(e))}
            style={{ fontSize: 11, padding: '2px 7px', gap: 5 }}
          >
            <span style={{ color: 'var(--zinc-600)' }}>✎</span> Add nickname or note
          </button>
        </div>
      )}
    </div>
  );
}

function BlockedBanner() {
  return (
    <div
      style={{
        background: 'rgba(239,68,68,.08)',
        borderBottom: '1px solid rgba(239,68,68,.28)',
        padding: '5px 12px',
        display: 'flex',
        alignItems: 'center',
        gap: 6,
      }}
    >
      <span
        style={{
          color: 'var(--live)',
          font: '600 9px var(--font-mono)',
          letterSpacing: '.1em',
          textTransform: 'uppercase',
        }}
      >
        ⊘ Blocked
      </span>
      <span style={{ font: '10px var(--font-mono)', color: 'var(--zinc-500)' }}>messages hidden</span>
    </div>
  );
}

function SignedOutNotice({ message }) {
  const text =
    message && (message.includes('sign in') || message.includes('not signed in'))
      ? 'Sign in to Twitch in Settings to load profile data.'
      : 'Couldn’t load profile.';
  return (
    <div
      role="alert"
      style={{
        margin: '0 12px 11px',
        background: 'rgba(234,179,8,.07)',
        border: '1px solid rgba(234,179,8,.3)',
        borderRadius: 'var(--r-2)',
        padding: '8px 10px',
        display: 'flex',
        gap: 8,
        alignItems: 'flex-start',
      }}
    >
      <span style={{ color: 'var(--warn)', fontSize: 12, lineHeight: 1.3 }}>⚠</span>
      <span style={{ color: 'var(--zinc-300)', font: '11px var(--font-sans)', lineHeight: 1.45 }}>
        {text}
      </span>
    </div>
  );
}

function LoadingStats() {
  const bar = (w, extra) => (
    <div
      style={{
        height: 24,
        flex: 1,
        borderRadius: 2,
        background: 'var(--zinc-800)',
        animation: 'usercard-pulse 1.4s ease-in-out infinite',
        ...extra,
      }}
    />
  );
  return (
    <div style={{ display: 'flex', gap: 14, padding: '10px 12px', borderTop: 'var(--hair)' }}>
      {bar()}
      {bar()}
    </div>
  );
}

function pointFromEvent(e) {
  const r = e.currentTarget.getBoundingClientRect();
  return { x: r.left, y: r.bottom + 4 };
}

function formatAge(isoStr) {
  const then = new Date(isoStr);
  const ms = Date.now() - then.getTime();
  const days = Math.floor(ms / (1000 * 60 * 60 * 24));
  const years = Math.floor(days / 365);
  const months = Math.floor((days % 365) / 30);
  if (years > 0) return `${years}y ${months}mo`;
  if (months > 0) return `${months}mo`;
  return `${days}d`;
}
