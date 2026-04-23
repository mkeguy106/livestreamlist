import { useEffect, useState } from 'react';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export default function WindowControls() {
  const [win, setWin] = useState(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!inTauri) return;
    let unlisten = null;
    (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const w = getCurrentWindow();
      setWin(w);
      setMaximized(await w.isMaximized());
      unlisten = await w.onResized(async () => setMaximized(await w.isMaximized()));
    })();
    return () => { if (unlisten) unlisten(); };
  }, []);

  if (!inTauri || !win) return null;

  const btn = (label, onClick, ariaLabel) => (
    <button
      type="button"
      aria-label={ariaLabel}
      onClick={onClick}
      style={{
        width: 28, height: 22,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'transparent', border: 'none', color: 'var(--zinc-400)',
        cursor: 'pointer', padding: 0, borderRadius: 4,
      }}
      onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--zinc-900)'; e.currentTarget.style.color = 'var(--zinc-100)'; }}
      onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'var(--zinc-400)'; }}
    >
      {label}
    </button>
  );

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 2, WebkitAppRegion: 'no-drag' }}>
      {btn(
        <svg width="10" height="1"><rect width="10" height="1" fill="currentColor" /></svg>,
        () => win.minimize(),
        'Minimize',
      )}
      {btn(
        maximized ? (
          <svg width="10" height="10" fill="none" stroke="currentColor" strokeWidth="1.2">
            <rect x="0.5" y="2.5" width="7" height="7" />
            <path d="M2.5 2.5V0.5h7v7h-2" />
          </svg>
        ) : (
          <svg width="10" height="10" fill="none" stroke="currentColor" strokeWidth="1.2">
            <rect x="0.5" y="0.5" width="9" height="9" />
          </svg>
        ),
        () => win.toggleMaximize(),
        maximized ? 'Restore' : 'Maximize',
      )}
      {btn(
        <svg width="10" height="10" fill="none" stroke="currentColor" strokeWidth="1.2">
          <path d="M1 1l8 8M9 1l-8 8" />
        </svg>,
        () => win.close(),
        'Close',
      )}
    </div>
  );
}
