import { useEffect, useRef, useState } from 'react';
import { addChannelFromInput, clipboardChannelUrl } from '../ipc.js';

export default function AddChannelDialog({ open, onClose, onAdded }) {
  const [value, setValue] = useState('');
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef(null);

  useEffect(() => {
    if (!open) return;
    setValue('');
    setError(null);
    setBusy(false);

    // Auto-prefill with a clipboard URL when it points at a supported
    // platform (Twitch / YouTube / Kick / Chaturbate). Mirrors the Qt
    // app. Fires only on dialog open; no clipboard polling.
    let cancelled = false;
    clipboardChannelUrl()
      .then((url) => {
        if (cancelled || !url) return;
        setValue(url);
        // Select-all so a single keystroke replaces it for users who
        // didn't intend to paste.
        requestAnimationFrame(() => inputRef.current?.select());
      })
      .catch(() => {
        // Clipboard read can fail if the system denies access; the
        // user can still type or paste manually.
      });

    const id = requestAnimationFrame(() => inputRef.current?.focus());
    return () => {
      cancelled = true;
      cancelAnimationFrame(id);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const submit = async (e) => {
    e?.preventDefault?.();
    if (!value.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const ch = await addChannelFromInput(value.trim());
      onAdded?.(ch);
      onClose();
    } catch (e) {
      setError(String(e?.message ?? e));
      setBusy(false);
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,.5)',
        zIndex: 100,
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: 120,
      }}
    >
      <form
        onSubmit={submit}
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 560,
          background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)',
          borderRadius: 8,
          boxShadow: '0 24px 64px rgba(0,0,0,.7), 0 0 0 1px rgba(255,255,255,.04)',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            padding: '12px 14px',
            borderBottom: 'var(--hair)',
            display: 'flex',
            alignItems: 'center',
            gap: 10,
          }}
        >
          <span style={{ color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>›</span>
          <input
            ref={inputRef}
            className="rx-input"
            style={{
              border: 'none',
              background: 'transparent',
              flex: 1,
              fontSize: 'var(--t-13)',
              padding: 0,
            }}
            placeholder="Paste a channel URL or handle — twitch.tv/shroud, @ludwig, k:xqc…"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            disabled={busy}
          />
          <div className="rx-kbd">esc</div>
        </div>
        {error && (
          <div style={{ padding: '8px 14px', color: '#f87171', fontSize: 'var(--t-11)' }}>
            {error}
          </div>
        )}
        <div
          style={{
            padding: '10px 14px',
            borderTop: 'var(--hair)',
            display: 'flex',
            gap: 12,
            alignItems: 'center',
          }}
        >
          <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>
            Twitch · YouTube · Kick · Chaturbate
          </div>
          <div style={{ flex: 1 }} />
          <button type="button" className="rx-btn rx-btn-ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button type="submit" className="rx-btn rx-btn-primary" disabled={busy || !value.trim()}>
            {busy ? 'Adding…' : 'Add'}
          </button>
        </div>
      </form>
    </div>
  );
}
