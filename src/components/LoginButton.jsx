import { useAuth } from '../hooks/useAuth.js';

/** Small login/logout toggle for the titlebar. */
export default function LoginButton() {
  const { loading, twitch, loginTwitch, logoutTwitch, error } = useAuth();

  if (loading) {
    return (
      <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>auth…</div>
    );
  }

  if (twitch) {
    return (
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={logoutTwitch}
        title={`Click to log out · ${twitch.login}`}
        style={{ padding: '2px 8px', fontSize: 10 }}
      >
        @{twitch.login}
      </button>
    );
  }

  return (
    <button
      type="button"
      className="rx-btn"
      onClick={loginTwitch}
      title={error ?? 'Log in to Twitch'}
      style={{ padding: '2px 8px', fontSize: 10 }}
    >
      Log in to Twitch
    </button>
  );
}
