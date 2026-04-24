import { useEffect, useRef, useState } from 'react';
import { chatSend } from '../ipc.js';

const MAX_LEN = 500;

/**
 * Chat composer. Disabled until the user is authed on the channel's
 * platform — currently Twitch only; Kick lands in Phase 2b-2.
 */
export default function Composer({ channelKey, platform, auth, onLocalEcho }) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const inputRef = useRef(null);

  const authed = platform === 'twitch' ? Boolean(auth?.twitch) : false;
  const placeholder = !authed
    ? platform === 'twitch'
      ? 'Log in to Twitch to chat'
      : 'Sending for this platform arrives in Phase 2b-2'
    : 'Send a message…  —  Enter to send, Shift+Enter for newline';

  useEffect(() => {
    if (!authed) setError(null);
  }, [authed, channelKey]);

  const submit = async (e) => {
    e?.preventDefault?.();
    const body = text.trim();
    if (!body || !authed || busy || !channelKey) return;
    setBusy(true);
    setError(null);
    try {
      onLocalEcho?.(body, auth.twitch);
      await chatSend(channelKey, body);
      setText('');
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
      inputRef.current?.focus();
    }
  };

  const onKey = (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <form
      onSubmit={submit}
      style={{
        borderTop: 'var(--hair)',
        padding: '6px 10px',
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        background: 'var(--zinc-950)',
      }}
    >
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <div className="rx-mono rx-chiclet" style={{ color: 'var(--zinc-600)' }}>
          {authed ? `@${auth.twitch.login}` : platform}
        </div>
        <input
          ref={inputRef}
          type="text"
          className="rx-input"
          style={{ flex: 1 }}
          placeholder={placeholder}
          value={text}
          onChange={(e) => setText(e.target.value.slice(0, MAX_LEN))}
          onKeyDown={onKey}
          disabled={!authed || busy}
          maxLength={MAX_LEN}
        />
        <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)', minWidth: 54, textAlign: 'right' }}>
          {text.length} / {MAX_LEN}
        </span>
      </div>
      {error && (
        <div style={{ color: '#f87171', fontSize: 'var(--t-11)' }}>{error}</div>
      )}
    </form>
  );
}
