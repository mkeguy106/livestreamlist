import { useState } from 'react';
import { useAuth } from '../hooks/useAuth.js';

/**
 * Titlebar dropdown showing Twitch + Kick auth state and a login/logout
 * action per platform. Tiny footprint when everything's logged in.
 */
export default function LoginButton() {
  const { loading, twitch, kick, login, logout, error } = useAuth();
  const [open, setOpen] = useState(false);

  if (loading) {
    return (
      <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>auth…</div>
    );
  }

  const summary =
    twitch && kick
      ? `@${twitch.login} · k:@${kick.login}`
      : twitch
      ? `@${twitch.login}`
      : kick
      ? `k:@${kick.login}`
      : 'Log in';

  return (
    <div style={{ position: 'relative' }}>
      <button
        type="button"
        className={twitch || kick ? 'rx-btn rx-btn-ghost' : 'rx-btn'}
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
            minWidth: 220,
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
            identity={twitch}
            onLogin={() => { setOpen(false); login('twitch'); }}
            onLogout={() => { setOpen(false); logout('twitch'); }}
          />
          <AccountRow
            label="Kick"
            color="var(--kick)"
            identity={kick}
            onLogin={() => { setOpen(false); login('kick'); }}
            onLogout={() => { setOpen(false); logout('kick'); }}
          />
          {error && (
            <div
              style={{
                padding: '6px 10px',
                fontSize: 'var(--t-11)',
                color: '#f87171',
                borderTop: 'var(--hair)',
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

function AccountRow({ label, color, identity, onLogin, onLogout }) {
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
          color: identity ? 'var(--zinc-200)' : 'var(--zinc-500)',
          flex: 1,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {identity ? `@${identity.login}` : 'Not logged in'}
      </span>
      {identity ? (
        <button
          type="button"
          className="rx-btn rx-btn-ghost"
          onClick={onLogout}
          style={{ padding: '1px 6px', fontSize: 10 }}
        >
          Log out
        </button>
      ) : (
        <button
          type="button"
          className="rx-btn"
          onClick={onLogin}
          style={{ padding: '1px 6px', fontSize: 10 }}
        >
          Log in
        </button>
      )}
    </div>
  );
}
