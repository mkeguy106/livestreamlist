import { useEffect, useRef, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';

/**
 * Titlebar dropdown showing Twitch / Kick / YouTube auth state and a
 * login/logout action per platform.
 *
 * Dropdown stays OPEN during login so the OAuth callback errors surface
 * inline instead of vanishing behind the closed menu.
 */
export default function LoginButton() {
  const { loading, twitch, kick, youtube, login, logout, error } = useAuth();
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
  const summaryParts = [];
  if (twitch) summaryParts.push(`@${twitch.login}`);
  if (kick) summaryParts.push(`k:@${kick.login}`);
  if (ytSignedIn) summaryParts.push('y');
  const summary = summaryParts.length ? summaryParts.join(' · ') : 'Log in';

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
      <button
        type="button"
        className={twitch || kick || ytSignedIn ? 'rx-btn rx-btn-ghost' : 'rx-btn'}
        onClick={() => setOpen((v) => !v)}
        title={error ?? 'Accounts'}
        style={{ padding: '2px 8px', fontSize: 10 }}
      >
        {summary}
      </button>
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
            signedIn={Boolean(twitch)}
            busy={busyPlatform === 'twitch'}
            onLogin={() => doLogin('twitch')}
            onLogout={() => doLogout('twitch')}
          />
          <AccountRow
            label="Kick"
            color="var(--kick)"
            statusText={kick ? `@${kick.login}` : 'Not logged in'}
            signedIn={Boolean(kick)}
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
      <span style={{ color, fontSize: 'var(--t-11)', fontWeight: 600, minWidth: 54 }}>{label}</span>
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
