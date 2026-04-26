import { useEffect, useRef, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';
import { loginPopupClose, loginPopupResize } from '../ipc.js';

// Width is fixed by the chiclet anchor in LoginButton; only the height
// is dynamic. Feeding `rect.width` back to set_size triggered a shrink
// loop with `height: auto` on html/body — the wrapper's measured width
// drifted under shrink-to-fit each layout pass and we resized to the
// drifted value, narrowing the popup until rows clipped from the right.
const POPUP_W_CSS = 280;

/**
 * Standalone root rendered when the bundle is loaded with `?popup=login`
 * by the Rust `login_popup` module. Renders the account dropdown content
 * sized to fill its borderless top-level WebviewWindow. Auth state is
 * shared with the main window via the `auth:changed` event broadcast
 * from each auth IPC (see `useAuth`).
 *
 * Intentionally NOT mounted from App — App's livestream poll, chat
 * tasks, etc. would otherwise duplicate inside the popup.
 */
export default function LoginPopupRoot() {
  const { loading, twitch, kick, youtube, chaturbate, login, logout, error } = useAuth();
  const [busyPlatform, setBusyPlatform] = useState(null);
  const wrapperRef = useRef(null);

  // Style the popup body itself: dark bg, hairline border, rounded corners
  // so the borderless OS window looks like a popover. Body height becomes
  // auto so it shrinks to content (paired with the OS window resize-to-
  // content driven by ResizeObserver below).
  useEffect(() => {
    const prev = {
      background: document.body.style.background,
      border: document.body.style.border,
      borderRadius: document.body.style.borderRadius,
      overflow: document.body.style.overflow,
      height: document.body.style.height,
      htmlHeight: document.documentElement.style.height,
    };
    document.body.style.background = 'var(--zinc-925)';
    document.body.style.border = '1px solid var(--zinc-800)';
    document.body.style.borderRadius = '6px';
    document.body.style.overflow = 'hidden';
    // tokens.css sets html/body height: 100% so they match the popup
    // window. We want the inverse — content drives the window — so let
    // both shrink to fit.
    document.body.style.height = 'auto';
    document.documentElement.style.height = 'auto';
    return () => {
      Object.assign(document.body.style, {
        background: prev.background,
        border: prev.border,
        borderRadius: prev.borderRadius,
        overflow: prev.overflow,
        height: prev.height,
      });
      document.documentElement.style.height = prev.htmlHeight;
    };
  }, []);

  // Self-dismiss when not focused. Rust's WindowEvent::Focused(false)
  // hook handles the common case, but a rapid second chiclet click can
  // move focus back to main BEFORE the popup ever became focused — no
  // Focused(true) means no later Focused(false), and the popup gets
  // stranded. Polling document.hasFocus() catches that case directly.
  // 300 ms grace covers the WM bring-up window where focus may legitly
  // not have settled yet.
  useEffect(() => {
    const start = performance.now();
    const id = setInterval(() => {
      if (performance.now() - start < 300) return;
      if (!document.hasFocus()) {
        loginPopupClose().catch(() => {});
      }
    }, 100);
    return () => clearInterval(id);
  }, []);

  // Resize the OS window to fit content (initial mount, auth resolves,
  // busy/error banners appear, future platforms added). Width stays
  // constant — only the height varies meaningfully.
  useEffect(() => {
    const el = wrapperRef.current;
    if (!el) return;
    let pending = null;
    const send = () => {
      const rect = el.getBoundingClientRect();
      if (rect.height < 1) return;
      const dpr = window.devicePixelRatio || 1;
      loginPopupResize(POPUP_W_CSS * dpr, rect.height * dpr).catch(() => {});
    };
    const observer = new ResizeObserver(() => {
      // rAF-coalesce so layout-thrashing during initial paint doesn't
      // burn an IPC per intermediate row layout.
      if (pending !== null) return;
      pending = requestAnimationFrame(() => {
        pending = null;
        send();
      });
    });
    observer.observe(el);
    send();
    return () => {
      observer.disconnect();
      if (pending !== null) cancelAnimationFrame(pending);
    };
  }, []);

  const ytSignedIn = Boolean(youtube?.browser || youtube?.has_paste);
  const cbSignedIn = Boolean(chaturbate?.signed_in);
  const twitchSignedIn = Boolean(twitch);
  const kickSignedIn = Boolean(kick);

  const doLogin = async (platform) => {
    setBusyPlatform(platform);
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
    <div
      ref={wrapperRef}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 2,
        padding: 4,
        boxSizing: 'border-box',
      }}
    >
      {loading && (
        <div style={{ padding: '6px 10px', color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
          auth…
        </div>
      )}
      {!loading && (
      <>
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
      </>
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
