import { useState } from 'react';
import { twitchWebLogin } from '../ipc.js';

/**
 * Lazy "connect Twitch web session" prompt. Mounted by ChatView when
 * useSubAnniversary detects a missing/expired cookie. One-shot per
 * app session — clicking Connect or Not now dismisses; we don't
 * persist dismissal across app launches.
 *
 * Props:
 *   onDismiss: () => void
 *   onConnected?: () => void   — called after successful login (optional)
 */
export function TwitchWebConnectPrompt({ onDismiss, onConnected }) {
  const [running, setRunning] = useState(false);
  const [error, setError] = useState(null);

  const handleConnect = async () => {
    setError(null);
    setRunning(true);
    try {
      await twitchWebLogin();
      onConnected?.();
      onDismiss?.();
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="rx-twitch-web-prompt" role="status">
      <div className="rx-twitch-web-prompt__text">
        We can detect your Twitch sub anniversaries. Sign in once to enable.
        {error && (
          <div style={{
            color: 'var(--warn, #f59e0b)',
            fontSize: 'var(--t-11)',
            marginTop: 4,
          }}>
            {error}
          </div>
        )}
      </div>
      <button
        type="button"
        className="rx-btn"
        onClick={handleConnect}
        disabled={running}
      >
        {running ? 'Waiting on Twitch…' : 'Connect'}
      </button>
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={onDismiss}
      >
        Not now
      </button>
    </div>
  );
}
