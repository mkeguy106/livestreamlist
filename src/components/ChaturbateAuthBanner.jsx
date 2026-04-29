import { useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';

/**
 * Renders inside ChatView's embed branch above <EmbedSlot>. Shows a
 * thin banner when the user is on a Chaturbate channel and not signed in,
 * with a one-click recovery path. Returns null when signed in.
 */
export default function ChaturbateAuthBanner() {
  const { chaturbate, login } = useAuth();
  const [running, setRunning] = useState(false);
  const [error, setError] = useState(null);

  if (chaturbate?.signed_in) return null;

  const onSignIn = async () => {
    setError(null);
    setRunning(true);
    try {
      await login('chaturbate');
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div
      role="status"
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 12,
        padding: '6px 10px',
        background: 'var(--zinc-900)',
        borderBottom: 'var(--hair)',
        fontSize: 'var(--t-11)',
        color: 'var(--zinc-300)',
      }}
    >
      <span>
        {error
          ? `Sign-in failed: ${error}`
          : 'Signed out of Chaturbate — chat is read-only.'}
      </span>
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={onSignIn}
        disabled={running}
      >
        {running ? 'Signing in…' : 'Sign in'}
      </button>
    </div>
  );
}
