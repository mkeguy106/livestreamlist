import { useEffect, useRef, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';

/**
 * Titlebar account widget. Two display modes:
 *
 *   - **Empty state** — no platform signed in: a single "Log in" button.
 *   - **Compact state** — at least one signed in: a row of T/Y/K/C
 *     chiclets, each with a small green dot when signed in or red when
 *     not. The whole row is one click target that opens the dropdown.
 *
 * The dropdown lists all four platforms (Twitch, Kick, YouTube,
 * Chaturbate) with per-row login/logout actions. Stays open during a
 * login so OAuth-callback errors surface inline.
 */
export default function LoginButton() {
  const { loading, twitch, kick, youtube, chaturbate, login, logout, error } = useAuth();
  const [open, setOpen] = useState(false);
  const [busyPlatform, setBusyPlatform] = useState(null);
  const containerRef = useRef(null);

  // Close the dropdown on outside click.
  useEffect(() => {
    if (!open) return;
    const onDown = (e) => {
      if (containerRef.current && !containerRef.current.contains(e.target)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  if (loading) {
    return (
      <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>auth…</div>
    );
  }

  const ytSignedIn = Boolean(youtube?.browser || youtube?.has_paste);
  const cbSignedIn = Boolean(chaturbate?.signed_in);
  const twitchSignedIn = Boolean(twitch);
  const kickSignedIn = Boolean(kick);
  const anySignedIn = twitchSignedIn || kickSignedIn || ytSignedIn || cbSignedIn;

  const platforms = [
    { id: 'twitch',     letter: 'T', color: 'var(--twitch)',   signedIn: twitchSignedIn },
    { id: 'youtube',    letter: 'Y', color: 'var(--youtube)',  signedIn: ytSignedIn },
    { id: 'kick',       letter: 'K', color: 'var(--kick)',     signedIn: kickSignedIn },
    { id: 'chaturbate', letter: 'C', color: 'var(--cb)',       signedIn: cbSignedIn },
  ];

  const doLogin = async (platform) => {
    setBusyPlatform(platform);
    try {
      await login(platform);
    } finally {
      setBusyPlatform(null);
    }
  };

  const doLogout = async (platform) => {
    setBusyPlatform(platform);
    try {
      await logout(platform);
    } finally {
      setBusyPlatform(null);
    }
  };

  const ytStatusText = youtube?.browser
    ? `Cookies from ${youtube.browser}`
    : youtube?.has_paste
    ? 'Signed in'
    : 'Not signed in';

  return (
    <div ref={containerRef} style={{ position: 'relative' }}>
      {anySignedIn ? (
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          title={error ?? 'Accounts'}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 4,
            background: 'transparent',
            border: '1px solid var(--zinc-800)',
            borderRadius: 6,
            padding: '2px 4px',
            cursor: 'pointer',
          }}
        >
          {platforms.map((p) => (
            <PlatformChiclet
              key={p.id}
              letter={p.letter}
              color={p.color}
              signedIn={p.signedIn}
            />
          ))}
        </button>
      ) : (
        <button
          type="button"
          className="rx-btn"
          onClick={() => setOpen((v) => !v)}
          title={error ?? 'Accounts'}
          style={{ padding: '2px 8px', fontSize: 10 }}
        >
          Log in
        </button>
      )}
      {open && (
        <div
          onClick={(e) => e.stopPropagation()}
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: 4,
            minWidth: 280,
            background: 'var(--zinc-925)',
            border: '1px solid var(--zinc-800)',
            borderRadius: 6,
            boxShadow: '0 12px 32px rgba(0,0,0,.6)',
            padding: 4,
            zIndex: 40,
            display: 'flex',
            flexDirection: 'column',
            gap: 2,
          }}
        >
          <AccountRow
            label="Twitch"
            color="var(--twitch)"
            statusText={twitch ? `@${twitch.login}` : 'Not logged in'}
            signedIn={twitchSignedIn}
            busy={busyPlatform === 'twitch'}
            onLogin={() => doLogin('twitch')}
            onLogout={() => doLogout('twitch')}
          />
          <AccountRow
            label="Kick"
            color="var(--kick)"
            statusText={kick ? `@${kick.login}` : 'Not logged in'}
            signedIn={kickSignedIn}
            busy={busyPlatform === 'kick'}
            onLogin={() => doLogin('kick')}
            onLogout={() => doLogout('kick')}
          />
          <AccountRow
            label="YouTube"
            color="var(--youtube)"
            statusText={ytStatusText}
            signedIn={ytSignedIn}
            busy={busyPlatform === 'youtube'}
            onLogin={() => doLogin('youtube')}
            onLogout={() => doLogout('youtube')}
          />
          <AccountRow
            label="Chaturbate"
            color="var(--cb)"
            statusText={cbSignedIn ? 'Signed in' : 'Not logged in'}
            signedIn={cbSignedIn}
            busy={busyPlatform === 'chaturbate'}
            onLogin={() => doLogin('chaturbate')}
            onLogout={() => doLogout('chaturbate')}
          />
          {busyPlatform && (
            <div
              style={{
                padding: '6px 10px',
                fontSize: 'var(--t-11)',
                color: 'var(--zinc-400)',
                borderTop: 'var(--hair)',
              }}
            >
              {busyPlatform === 'youtube'
                ? 'Sign in to Google in the window that just opened — it will close when cookies are captured.'
                : busyPlatform === 'chaturbate'
                ? 'Sign in to Chaturbate in the window that just opened — it will close once the session cookie is captured.'
                : 'Waiting for the browser — approve the login in the page that just opened, then come back here.'}
            </div>
          )}
          {error && (
            <div
              style={{
                padding: '6px 10px',
                fontSize: 'var(--t-11)',
                color: '#f87171',
                borderTop: 'var(--hair)',
                wordBreak: 'break-word',
              }}
            >
              {error}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * One letter chiclet in the compact account row. Letter is in the
 * platform accent color; the small dot at top-right is green when the
 * user is signed into that platform, red when not.
 */
function PlatformChiclet({ letter, color, signedIn }) {
  return (
    <div
      style={{
        position: 'relative',
        width: 18,
        height: 18,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontFamily: 'var(--font-mono)',
        fontSize: 11,
        fontWeight: 700,
        color,
        lineHeight: 1,
      }}
    >
      {letter}
      <span
        style={{
          position: 'absolute',
          top: -1,
          right: -1,
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: signedIn ? 'var(--ok, #22c55e)' : 'var(--live, #ef4444)',
          boxShadow: '0 0 0 1.5px var(--zinc-950)',
        }}
      />
    </div>
  );
}

function AccountRow({ label, color, statusText, signedIn, busy, onLogin, onLogout }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: '6px 10px',
      }}
    >
      <span style={{ color, fontSize: 'var(--t-11)', fontWeight: 600, minWidth: 70 }}>{label}</span>
      <span
        style={{
          fontSize: 'var(--t-11)',
          color: signedIn ? 'var(--zinc-200)' : 'var(--zinc-500)',
          flex: 1,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {statusText}
      </span>
      {signedIn ? (
        <button
          type="button"
          className="rx-btn rx-btn-ghost"
          onClick={onLogout}
          disabled={busy}
          style={{ padding: '1px 6px', fontSize: 10 }}
        >
          {busy ? '…' : 'Log out'}
        </button>
      ) : (
        <button
          type="button"
          className="rx-btn"
          onClick={onLogin}
          disabled={busy}
          style={{ padding: '1px 6px', fontSize: 10 }}
        >
          {busy ? 'Logging in…' : 'Log in'}
        </button>
      )}
    </div>
  );
}
