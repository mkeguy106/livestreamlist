import { useEffect, useRef, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';
import { listenEvent, loginPopupClose, loginPopupOpen } from '../ipc.js';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
const POPUP_W_CSS = 280;
// Close-to-resting initial height; LoginPopupRoot's ResizeObserver
// snaps to exact content height immediately after first paint.
const POPUP_H_CSS = 130;

/**
 * Titlebar account widget. Two display modes:
 *
 *   - **Empty state** — no platform signed in: a single "Log in" button.
 *   - **Compact state** — at least one signed in: a row of T/Y/K/C
 *     chiclets, each with a small green dot when signed in or red when
 *     not. The whole row is one click target that opens the dropdown.
 *
 * Inside Tauri the dropdown is a separate borderless transient_for
 * WebviewWindow (`login_popup` Rust module) so it stacks above the
 * YouTube/Chaturbate embed window. In a plain-browser dev session
 * (`!inTauri`) the dropdown renders inline as a plain HTML overlay so the
 * UI is still iterable without the desktop shell.
 */
export default function LoginButton() {
  const { loading, twitch, kick, youtube, chaturbate, login, logout, error } = useAuth();
  const [open, setOpen] = useState(false);
  const [busyPlatform, setBusyPlatform] = useState(null);
  const containerRef = useRef(null);
  const buttonRef = useRef(null);
  // Mirror of `open` for sync access — rapid back-to-back chiclet
  // clicks would otherwise both read the stale `open` from the same
  // render closure and miss the toggle.
  const openRef = useRef(false);

  // Reset the local "open" flag when the Tauri popup closes (focus loss,
  // explicit close IPC, etc.) so the next chiclet click re-opens it.
  useEffect(() => {
    if (!inTauri) return;
    let unlisten = null;
    let cancelled = false;
    listenEvent('login-popup:closed', () => {
      openRef.current = false;
      setOpen(false);
    })
      .then((u) => {
        if (cancelled) u?.();
        else unlisten = u;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // Inline dropdown (browser dev only) — close on outside click.
  useEffect(() => {
    if (inTauri || !open) return;
    const onDown = (e) => {
      if (containerRef.current && !containerRef.current.contains(e.target)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  const openPopup = async () => {
    // Use the ref so rapid clicks toggle correctly without waiting for
    // a re-render to flow `open` back to the closure.
    if (openRef.current) {
      openRef.current = false;
      setOpen(false);
      if (inTauri) loginPopupClose().catch(() => {});
      return;
    }
    if (!inTauri) {
      openRef.current = true;
      setOpen(true);
      return;
    }
    const btn = buttonRef.current;
    if (!btn) return;
    try {
      const r = btn.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const winApi = await import('@tauri-apps/api/window');
      const outer = await winApi.getCurrentWindow().outerPosition();
      // Right-align with the chiclet button; sit just below it. No
      // multi-monitor screen-bounds clamp — `window.screen.availWidth`
      // returns the PRIMARY monitor on most platforms, so clamping yanks
      // the popup off whichever monitor the main window is actually on.
      const w = POPUP_W_CSS * dpr;
      const h = POPUP_H_CSS * dpr;
      const x = outer.x + (r.right - POPUP_W_CSS) * dpr;
      const y = outer.y + (r.bottom + 4) * dpr;
      openRef.current = true;
      setOpen(true);
      await loginPopupOpen(x, y, w, h);
    } catch (e) {
      console.error('login_popup_open', e);
      openRef.current = false;
      setOpen(false);
    }
  };

  // Close the popup on unmount so it doesn't outlive the main window.
  useEffect(() => {
    return () => {
      if (inTauri) loginPopupClose().catch(() => {});
    };
  }, []);

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
    // Defensive watchdog: if the IPC promise never settles (Rust-side
    // hang on a destroyed webview, lost event, etc.), unstick the UI
    // after 6 minutes — comfortably past Rust's own 5-minute login
    // timeout so a real long-but-valid sign-in isn't yanked.
    const watchdog = setTimeout(() => {
      setBusyPlatform((current) => (current === platform ? null : current));
    }, 6 * 60 * 1000);
    try {
      await login(platform);
    } finally {
      clearTimeout(watchdog);
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
          ref={buttonRef}
          type="button"
          onClick={openPopup}
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
          ref={buttonRef}
          type="button"
          className="rx-btn"
          onClick={openPopup}
          title={error ?? 'Accounts'}
          style={{ padding: '2px 8px', fontSize: 10 }}
        >
          Log in
        </button>
      )}
      {!inTauri && open && (
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
 * platform accent color (toned down via opacity); the small dot at
 * top-right is a muted green when signed in, muted red when not.
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
        opacity: 0.45,
        lineHeight: 1,
      }}
    >
      {letter}
      <span
        style={{
          position: 'absolute',
          top: -1,
          right: -1,
          width: 5,
          height: 5,
          borderRadius: '50%',
          // Tailwind green-900 / red-900 — chiclet should never
          // visually compete with live-stream dots; the titlebar
          // affordance stays in the background until clicked.
          background: signedIn ? '#14532d' : '#7f1d1d',
          boxShadow: '0 0 0 1.5px var(--zinc-925)',
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
