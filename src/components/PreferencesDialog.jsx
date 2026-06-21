import { useCallback, useEffect, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { formatRelative } from '../utils/format.js';
import Tooltip from './Tooltip.jsx';
import SidebarPositionPicker from './SidebarPositionPicker.jsx';
import {
  importTwitchFollows,
  importYoutubeSubscriptions,
  importChaturbateFollows,
  listBlockedUsers,
  setUserMetadata,
  spellcheckListDicts,
  twitchWebClear,
  twitchWebLogin,
  youtubeDetectBrowsers,
} from '../ipc.js';

const TABS = [
  { id: 'general', label: 'General' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'chat', label: 'Chat' },
  { id: 'accounts', label: 'Accounts' },
];

const PLATFORMS = [
  {
    id: 'twitch', name: 'Twitch', letter: 'T', tag: 'TTV',
    accent: 'var(--twitch)', monoBg: 'rgba(167,139,250,.12)', monoBorder: 'rgba(167,139,250,.22)',
    importTitle: 'Import follows',
    importDesc: 'Adds every channel you follow on Twitch. Existing entries are skipped.',
  },
  {
    id: 'youtube', name: 'YouTube', letter: 'Y', tag: 'YT',
    accent: 'var(--youtube)', monoBg: 'rgba(248,113,113,.12)', monoBorder: 'rgba(248,113,113,.22)',
    importTitle: 'Import subscriptions',
    importDesc: 'Adds every channel you’re subscribed to on YouTube. Existing entries are skipped.',
  },
  {
    id: 'kick', name: 'Kick', letter: 'K', tag: 'KICK',
    accent: 'var(--kick)', monoBg: 'rgba(74,222,128,.12)', monoBorder: 'rgba(74,222,128,.22)',
    importTitle: 'Import follows', importDesc: '',
  },
  {
    id: 'chaturbate', name: 'Chaturbate', letter: 'C', tag: 'CB',
    accent: 'var(--cb)', monoBg: 'rgba(251,146,60,.12)', monoBorder: 'rgba(251,146,60,.22)',
    importTitle: 'Import follows',
    importDesc: 'Adds every model you follow on Chaturbate. Existing entries are skipped.',
  },
];

const IMPORT_RUNNERS = {
  twitch: importTwitchFollows,
  youtube: importYoutubeSubscriptions,
  chaturbate: importChaturbateFollows,
};

function platformConnected(id, auth) {
  switch (id) {
    case 'twitch': return !!auth.twitch;
    case 'youtube': return !!(auth.youtube?.has_paste || auth.youtube?.browser);
    case 'kick': return !!auth.kick;
    case 'chaturbate': return !!auth.chaturbate?.signed_in;
    default: return false;
  }
}

// Import is possible only when connected AND we actually have a working path.
// Kick has no follows API; YouTube needs keyring cookies (not browser-cookie).
function importCapable(id, auth) {
  switch (id) {
    case 'twitch': return !!auth.twitch;
    case 'youtube': return !!auth.youtube?.has_paste;
    case 'chaturbate': return !!auth.chaturbate?.signed_in;
    default: return false;
  }
}

export default function PreferencesDialog({ open, onClose }) {
  const [tab, setTab] = useState('general');
  const { settings, error, patch } = usePreferences();
  const auth = useAuth();
  const connectedCount = PLATFORMS.filter((p) => platformConnected(p.id, auth)).length;

  useEffect(() => {
    if (!open) return;
    const onKey = (e) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,.55)',
        zIndex: 100,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 40,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(720px, 100%)',
          height: 'min(540px, 100%)',
          background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)',
          borderRadius: 8,
          boxShadow: '0 24px 64px rgba(0,0,0,.7), 0 0 0 1px rgba(255,255,255,.04)',
          display: 'flex',
          overflow: 'hidden',
        }}
      >
        {/* Tab rail */}
        <div
          style={{
            width: 180,
            borderRight: 'var(--hair)',
            display: 'flex',
            flexDirection: 'column',
            padding: '10px 8px',
            gap: 2,
            background: 'var(--zinc-950)',
          }}
        >
          <div className="rx-chiclet" style={{ padding: '6px 8px 10px' }}>PREFERENCES</div>
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setTab(t.id)}
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                gap: 8,
                textAlign: 'left',
                padding: '6px 10px',
                border: 'none',
                borderRadius: 4,
                background: t.id === tab ? 'var(--zinc-900)' : 'transparent',
                color: t.id === tab ? 'var(--zinc-100)' : 'var(--zinc-400)',
                fontSize: 'var(--t-12)',
                fontFamily: 'inherit',
                cursor: 'pointer',
              }}
            >
              <span>{t.label}</span>
              {t.id === 'accounts' && (
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                  {connectedCount}/4
                </span>
              )}
            </button>
          ))}
          <div style={{ flex: 1 }} />
          <button
            type="button"
            onClick={onClose}
            className="rx-btn rx-btn-ghost"
            style={{ margin: '0 4px', justifyContent: 'center' }}
          >
            Close
          </button>
        </div>

        {/* Tab content */}
        <div
          style={{
            flex: 1,
            overflow: 'auto',
            padding: '20px 24px',
            display: 'flex',
            flexDirection: 'column',
            gap: 18,
          }}
        >
          {!settings && <div style={{ color: 'var(--zinc-500)' }}>Loading…</div>}
          {error && <div style={{ color: '#f87171', fontSize: 'var(--t-11)' }}>{error}</div>}
          {settings && tab === 'general' && <GeneralTab settings={settings} patch={patch} />}
          {settings && tab === 'appearance' && <AppearanceTab settings={settings} patch={patch} />}
          {settings && tab === 'chat' && <ChatTab settings={settings} patch={patch} />}
          {tab === 'accounts' && <AccountsTab />}
        </div>
      </div>
    </div>
  );
}

function AccountsTab() {
  const { twitch, twitch_web, kick, youtube, chaturbate, login, logout, loginYoutubePaste, refresh } = useAuth();
  const { settings, patch } = usePreferences();
  const [ytLoginRunning, setYtLoginRunning] = useState(false);
  const [cbLoginRunning, setCbLoginRunning] = useState(false);
  const [cbError, setCbError] = useState(null);
  const [twWebRunning, setTwWebRunning] = useState(false);
  const [twWebError, setTwWebError] = useState(null);

  const runChaturbateLogin = async () => {
    setCbError(null);
    setCbLoginRunning(true);
    try {
      await login('chaturbate');
    } catch (e) {
      setCbError(String(e?.message ?? e));
    } finally {
      setCbLoginRunning(false);
    }
  };
  const runTwitchWebLogin = async () => {
    setTwWebError(null);
    setTwWebRunning(true);
    try {
      await twitchWebLogin();
      await refresh();
    } catch (e) {
      setTwWebError(String(e?.message ?? e));
    } finally {
      setTwWebRunning(false);
    }
  };

  const runTwitchWebClear = async () => {
    setTwWebError(null);
    try {
      await twitchWebClear();
      await refresh();
    } catch (e) {
      setTwWebError(String(e?.message ?? e));
    }
  };

  const [ytPasteOpen, setYtPasteOpen] = useState(false);
  const [ytError, setYtError] = useState(null);
  const [ytAdvanced, setYtAdvanced] = useState(false);
  const [browsers, setBrowsers] = useState(null); // null = loading; [] = nothing detected

  useEffect(() => {
    let cancelled = false;
    youtubeDetectBrowsers()
      .then((b) => {
        if (!cancelled) setBrowsers(Array.isArray(b) ? b : []);
      })
      .catch(() => {
        if (!cancelled) setBrowsers([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const runYoutubeLogin = async () => {
    setYtError(null);
    setYtLoginRunning(true);
    try {
      await login('youtube');
    } catch (e) {
      setYtError(String(e?.message ?? e));
    } finally {
      setYtLoginRunning(false);
    }
  };

  const setYtBrowser = (id) => {
    setYtError(null);
    patch((prev) => ({
      ...prev,
      general: { ...prev.general, youtube_cookies_browser: id ?? null },
    }));
    // settings patch debounces 200 ms before hitting the backend; refresh()
    // will read via auth_status once that lands.
    setTimeout(() => refresh(), 260);
  };

  const ytBrowser = settings?.general?.youtube_cookies_browser ?? null;
  const ytLabelFor = (id) =>
    browsers?.find((b) => b.id === id)?.label ?? id;

  const auth = useAuth();
  const [imports, setImports] = useState({}); // id -> { status, result, error }

  const runImport = useCallback(async (id) => {
    const runner = IMPORT_RUNNERS[id];
    if (!runner) return;
    setImports((s) => ({ ...s, [id]: { status: 'running' } }));
    try {
      const result = await runner();
      setImports((s) => ({ ...s, [id]: { status: 'done', result } }));
    } catch (e) {
      setImports((s) => ({ ...s, [id]: { status: 'error', error: String(e?.message ?? e) } }));
    }
  }, []);

  const anyImportRunning = Object.values(imports).some((x) => x?.status === 'running');

  const importAll = useCallback(() => {
    for (const p of PLATFORMS) {
      if (importCapable(p.id, auth) && imports[p.id]?.status !== 'running') {
        runImport(p.id);
      }
    }
  }, [auth, imports, runImport]);

  const anyCapable = PLATFORMS.some((p) => importCapable(p.id, auth));

  const detailFor = (id) => {
    switch (id) {
      case 'twitch':
        return auth.twitch ? `@${auth.twitch.login}` : 'Not logged in';
      case 'youtube':
        return ytBrowser
          ? `Using cookies from ${ytLabelFor(ytBrowser)}`
          : auth.youtube?.has_paste
          ? 'Signed in via Google'
          : 'Not signed in';
      case 'kick':
        return auth.kick ? `@${auth.kick.login}` : 'Not logged in';
      case 'chaturbate':
        return auth.chaturbate?.signed_in
          ? `Signed in · verified ${formatRelative(auth.chaturbate.last_verified_at)}`
          : 'Not signed in';
      default:
        return '';
    }
  };

  const authButtonFor = (id) => {
    const connected = platformConnected(id, auth);
    switch (id) {
      case 'twitch':
        return connected ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('twitch')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={() => login('twitch')}>
            Connect
          </button>
        );
      case 'youtube':
        return connected ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('youtube')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={runYoutubeLogin} disabled={ytLoginRunning}>
            {ytLoginRunning ? 'Waiting on Google…' : 'Connect'}
          </button>
        );
      case 'kick':
        return connected ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('kick')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={() => login('kick')}>
            Connect
          </button>
        );
      case 'chaturbate':
        return connected ? (
          <div style={{ display: 'flex', gap: 6 }}>
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={runChaturbateLogin}
              disabled={cbLoginRunning}
            >
              {cbLoginRunning ? 'Signing in…' : 'Sign in again'}
            </button>
            <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('chaturbate')}>
              Log out
            </button>
          </div>
        ) : (
          <button type="button" className="rx-btn" onClick={runChaturbateLogin} disabled={cbLoginRunning}>
            {cbLoginRunning ? 'Waiting on Chaturbate…' : 'Connect'}
          </button>
        );
      default:
        return null;
    }
  };

  const importZoneFor = (p) => {
    const connected = platformConnected(p.id, auth);
    if (p.id === 'kick') {
      return <ImportNote>Kick doesn't expose your follows to apps yet.</ImportNote>;
    }
    if (!connected) {
      return <ImportNote>Connect {p.name} to import the channels you follow.</ImportNote>;
    }
    if (p.id === 'youtube' && !auth.youtube?.has_paste) {
      return (
        <ImportNote>Sign in with Google or paste cookies to enable subscription import.</ImportNote>
      );
    }
    return (
      <ImportControl
        title={p.importTitle}
        desc={p.importDesc}
        accent={p.accent}
        state={imports[p.id]}
        onRun={() => runImport(p.id)}
      />
    );
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, margin: '-20px -24px', height: 'calc(100% + 40px)' }}>
      {/* Header */}
      <div
        style={{
          flexShrink: 0,
          padding: '4px 24px 14px',
          borderBottom: 'var(--hair)',
          display: 'flex',
          alignItems: 'flex-end',
          justifyContent: 'space-between',
          gap: 16,
        }}
      >
        <div>
          <div style={{ fontSize: 'var(--t-16)', fontWeight: 600, color: 'var(--zinc-100)', letterSpacing: '-.01em' }}>
            Accounts
          </div>
          <div style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-500)', marginTop: 3 }}>
            Connect a platform, then pull in everyone you already follow.
          </div>
        </div>
        <Tooltip text={anyCapable ? 'Import follows from every connected platform' : 'Connect a platform first'}>
          <button
            type="button"
            className="rx-btn"
            aria-label="Import all follows"
            onClick={importAll}
            disabled={!anyCapable || anyImportRunning}
            style={{ flexShrink: 0 }}
          >
            <span style={{ width: 6, height: 6, borderRadius: '50%', background: 'var(--ok)' }} />
            Import all follows
          </button>
        </Tooltip>
      </div>

      {/* Cards */}
      <div style={{ flex: 1, overflow: 'auto', padding: '0 24px 22px', display: 'flex', flexDirection: 'column', gap: 12 }}>
        {PLATFORMS.map((p) => (
          <PlatformCard
            key={p.id}
            cfg={p}
            connected={platformConnected(p.id, auth)}
            detail={detailFor(p.id)}
            authButton={authButtonFor(p.id)}
            importZone={importZoneFor(p)}
            error={p.id === 'chaturbate' ? cbError : null}
            disclosure={
              p.id === 'youtube' && !platformConnected('youtube', auth) ? (
                <YoutubeSignInExtras
                  browsers={browsers}
                  ytBrowser={ytBrowser}
                  setYtBrowser={setYtBrowser}
                  ytAdvanced={ytAdvanced}
                  setYtAdvanced={setYtAdvanced}
                  onPaste={() => setYtPasteOpen(true)}
                  ytError={ytError}
                />
              ) : null
            }
          />
        ))}

        {/* Twitch web session — secondary, de-emphasized */}
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            padding: '10px 13px',
            border: 'var(--hair)',
            borderRadius: 6,
            background: 'var(--zinc-925)',
          }}
        >
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-300)', fontWeight: 500 }}>
              Twitch web session
            </div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginTop: 2 }}>
              {twitch_web
                ? `Connected as @${twitch_web.login}`
                : 'Sign in once for sub-anniversary detection (separate from chat login)'}
            </div>
          </div>
          {twitch_web ? (
            <button type="button" className="rx-btn rx-btn-ghost" onClick={runTwitchWebClear}>
              Disconnect
            </button>
          ) : (
            <button type="button" className="rx-btn rx-btn-ghost" onClick={runTwitchWebLogin} disabled={twWebRunning}>
              {twWebRunning ? 'Waiting on Twitch…' : 'Connect'}
            </button>
          )}
        </div>
        {twWebError && (
          <div style={{ color: 'var(--warn, #f59e0b)', fontSize: 'var(--t-11)', paddingLeft: 4 }}>{twWebError}</div>
        )}
      </div>

      <YoutubePasteDialog
        open={ytPasteOpen}
        onClose={() => setYtPasteOpen(false)}
        onSubmit={async (text) => {
          await loginYoutubePaste(text);
          setYtPasteOpen(false);
        }}
      />
    </div>
  );
}

function PlatformCard({ cfg, connected, detail, authButton, importZone, disclosure, error }) {
  return (
    <div
      style={{
        flexShrink: 0,
        border: '1px solid var(--zinc-800)',
        borderRadius: 8,
        background: 'var(--zinc-900)',
        overflow: 'hidden',
      }}
    >
      {/* Identity row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '13px 14px' }}>
        <div
          style={{
            width: 36, height: 36, flexShrink: 0, borderRadius: 9,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontFamily: 'var(--font-mono)', fontWeight: 600, fontSize: 16,
            background: cfg.monoBg, color: cfg.accent, border: `1px solid ${cfg.monoBorder}`,
          }}
        >
          {cfg.letter}
        </div>
        <div style={{ minWidth: 0, flex: 1 }}>
          <div style={{ fontSize: 'var(--t-13)', fontWeight: 600, color: 'var(--zinc-100)', display: 'flex', alignItems: 'center', gap: 7 }}>
            {cfg.name}
            <span className="rx-mono" style={{ fontSize: 9, letterSpacing: '.07em', textTransform: 'uppercase', color: cfg.accent }}>
              {cfg.tag}
            </span>
          </div>
          <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginTop: 3, display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
            <span style={{ width: 6, height: 6, borderRadius: '50%', flexShrink: 0, background: connected ? 'var(--ok)' : 'var(--zinc-600)' }} />
            <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{detail}</span>
          </div>
        </div>
        <div style={{ flexShrink: 0 }}>{authButton}</div>
      </div>

      {/* Import zone */}
      <div style={{ borderTop: '1px solid rgba(255,255,255,.05)', background: 'var(--zinc-925)', padding: '12px 14px 13px' }}>
        {disclosure}
        {importZone}
        {error && (
          <div style={{ marginTop: 8, fontSize: 'var(--t-11)', color: 'var(--live)' }}>{error}</div>
        )}
      </div>
    </div>
  );
}

function ImportNote({ children }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
      <span style={{ display: 'inline-flex', width: 14, height: 14, alignItems: 'center', justifyContent: 'center' }}>⌗</span>
      {children}
    </div>
  );
}

function ImportControl({ title, desc, accent, state, onRun }) {
  const status = state?.status ?? 'idle';
  const running = status === 'running';
  const label = running ? 'Importing' : status === 'done' ? 'Import again' : 'Import now';
  const btnClass = status === 'done' ? 'rx-btn rx-btn-ghost' : 'rx-btn';
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 14 }}>
        <div style={{ minWidth: 0 }}>
          <div style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-300)', fontWeight: 500 }}>{title}</div>
          <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginTop: 2, lineHeight: 1.45 }}>{desc}</div>
        </div>
        <button
          type="button"
          className={btnClass}
          onClick={onRun}
          disabled={running}
          style={{ flexShrink: 0, minWidth: 96, justifyContent: 'center' }}
        >
          {running && (
            <span
              style={{
                width: 11, height: 11, borderRadius: '50%',
                border: '1.5px solid currentColor', borderTopColor: 'transparent',
                display: 'inline-block', animation: 'rx-spin .7s linear infinite',
              }}
            />
          )}
          {label}
        </button>
      </div>

      {running && (
        <div style={{ marginTop: 11 }}>
          <div style={{ position: 'relative', height: 4, borderRadius: 3, background: 'var(--zinc-800)', overflow: 'hidden' }}>
            <div
              style={{
                position: 'absolute', top: 0, height: '100%', width: '40%',
                borderRadius: 3, background: accent,
                animation: 'acc-indeterminate 1.1s ease-in-out infinite',
              }}
            />
          </div>
          <div className="rx-mono" style={{ fontSize: 10.5, color: 'var(--zinc-400)', marginTop: 6 }}>
            Importing your follows…
          </div>
        </div>
      )}

      {status === 'done' && state?.result && (
        <div style={{ marginTop: 10, display: 'flex', alignItems: 'center', gap: 8, animation: 'acc-pop .2s ease' }}>
          <span style={{ display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: 15, height: 15, borderRadius: '50%', background: 'rgba(34,197,94,.16)', color: 'var(--ok)', fontSize: 10 }}>✓</span>
          <span className="rx-mono" style={{ fontSize: 10.5, color: 'var(--zinc-300)' }}>
            Added {state.result.added} · skipped {state.result.skipped} · {state.result.total_seen} seen
          </span>
        </div>
      )}

      {status === 'error' && (
        <div style={{ marginTop: 8, fontSize: 'var(--t-11)', color: '#f87171' }}>{state.error}</div>
      )}
    </div>
  );
}

function YoutubeSignInExtras({ browsers, ytBrowser, setYtBrowser, ytAdvanced, setYtAdvanced, onPaste, ytError }) {
  return (
    <div style={{ marginBottom: 10 }}>
      <button
        type="button"
        onClick={() => setYtAdvanced((v) => !v)}
        style={{ all: 'unset', cursor: 'pointer', fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}
      >
        {ytAdvanced ? '▾ Other ways to sign in' : '▸ Other ways to sign in'}
      </button>
      {ytAdvanced && (
        <div style={{ marginTop: 8, padding: '8px 10px', border: '1px solid var(--zinc-800)', borderRadius: 4, display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-300)', marginBottom: 4, fontWeight: 500 }}>
              Use cookies from a browser
            </div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginBottom: 6 }}>
              Reuses an existing browser session — no extra sign-in needed.
            </div>
            {browsers === null ? (
              <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>Detecting…</div>
            ) : browsers.length === 0 ? (
              <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>No supported browsers found on this system.</div>
            ) : (
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                {browsers.map((b) => {
                  const active = ytBrowser === b.id;
                  return (
                    <Tooltip key={b.id} text={active ? `Stop using ${b.label} cookies` : `Use ${b.label} cookies`}>
                      <button
                        type="button"
                        className={active ? 'rx-btn' : 'rx-btn rx-btn-ghost'}
                        onClick={() => setYtBrowser(active ? null : b.id)}
                      >
                        {active ? `✓ ${b.label}` : b.label}
                      </button>
                    </Tooltip>
                  );
                })}
              </div>
            )}
          </div>
          <div style={{ borderTop: 'var(--hair)', paddingTop: 8 }}>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-300)', marginBottom: 4, fontWeight: 500 }}>Paste cookies</div>
            <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginBottom: 6 }}>
              For sandboxed builds (Flatpak) where the app can't reach a browser cookie store.
            </div>
            <button type="button" className="rx-btn rx-btn-ghost" onClick={onPaste}>Paste cookies…</button>
          </div>
          {ytError && <div style={{ fontSize: 'var(--t-11)', color: '#f87171' }}>{ytError}</div>}
        </div>
      )}
    </div>
  );
}

function YoutubePasteDialog({ open, onClose, onSubmit }) {
  const [text, setText] = useState('');
  const [error, setError] = useState(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) {
      setText('');
      setError(null);
      setBusy(false);
    }
  }, [open]);

  if (!open) return null;

  const submit = async () => {
    setError(null);
    setBusy(true);
    try {
      await onSubmit(text);
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,.65)',
        zIndex: 110,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 40,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 'min(560px, 100%)',
          background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)',
          borderRadius: 8,
          padding: 18,
          display: 'flex',
          flexDirection: 'column',
          gap: 10,
        }}
      >
        <div style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-100)', fontWeight: 600 }}>
          Paste YouTube cookies
        </div>
        <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)', lineHeight: 1.5 }}>
          Export the five Google session cookies — <code>SID</code>, <code>HSID</code>,{' '}
          <code>SSID</code>, <code>APISID</code>, <code>SAPISID</code> — from a logged-in browser
          and paste them here. Cookie-header (<code>SID=…; HSID=…</code>), one-per-line, or
          Netscape <code>cookies.txt</code> format all work.
        </div>
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          spellCheck={false}
          autoFocus
          placeholder="SID=…; HSID=…; SSID=…; APISID=…; SAPISID=…"
          style={{
            width: '100%',
            minHeight: 140,
            background: 'var(--zinc-950)',
            border: '1px solid var(--zinc-800)',
            borderRadius: 4,
            padding: 8,
            color: 'var(--zinc-100)',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--t-11)',
            resize: 'vertical',
          }}
        />
        {error && (
          <div style={{ fontSize: 'var(--t-11)', color: '#f87171' }}>{error}</div>
        )}
        <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end' }}>
          <button type="button" className="rx-btn rx-btn-ghost" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            type="button"
            className="rx-btn"
            onClick={submit}
            disabled={busy || !text.trim()}
          >
            {busy ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  );
}

function Row({ label, hint, children }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
        <div style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-200)', fontWeight: 500, minWidth: 160 }}>{label}</div>
        <div style={{ flex: 1 }}>{children}</div>
      </div>
      {hint && <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginLeft: 168 }}>{hint}</div>}
    </div>
  );
}

function Toggle({ checked, onChange, disabled }) {
  return (
    <button
      type="button"
      onClick={() => !disabled && onChange(!checked)}
      aria-pressed={checked}
      disabled={disabled}
      style={{
        width: 36, height: 20, borderRadius: 10, border: 'none',
        background: checked && !disabled ? 'var(--zinc-100)' : 'var(--zinc-800)',
        position: 'relative', cursor: disabled ? 'not-allowed' : 'pointer',
        transition: 'background 120ms',
        opacity: disabled ? 0.4 : 1,
      }}
    >
      <span style={{
        position: 'absolute', top: 2, left: checked && !disabled ? 18 : 2,
        width: 16, height: 16, borderRadius: '50%',
        background: checked && !disabled ? 'var(--zinc-950)' : 'var(--zinc-400)',
        transition: 'left 120ms',
      }} />
    </button>
  );
}

function GeneralTab({ settings, patch }) {
  const g = settings.general;
  return (
    <>
      <Row
        label="Refresh interval"
        hint="How often the stream list polls all platforms. Lower = fresher but more network."
      >
        <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
          <input
            type="number"
            min={15}
            max={600}
            value={g.refresh_interval_seconds}
            onChange={(e) => {
              const v = Math.max(15, Math.min(600, Number(e.target.value) || 60));
              patch((prev) => ({ ...prev, general: { ...prev.general, refresh_interval_seconds: v } }));
            }}
            className="rx-input"
            style={{ width: 80 }}
          />
          <span style={{ color: 'var(--zinc-500)', fontSize: 'var(--t-11)' }}>seconds</span>
        </div>
      </Row>

      <Row
        label="Notify when a channel goes live"
        hint="Desktop notification when an offline channel transitions to live."
      >
        <Toggle
          checked={g.notify_on_live}
          onChange={(v) => patch((prev) => ({ ...prev, general: { ...prev.general, notify_on_live: v } }))}
        />
      </Row>

      <Row
        label="Close to tray"
        hint="Clicking the window close button hides the app to the tray instead of quitting. (Wired in Phase 4b-2.)"
      >
        <Toggle
          checked={g.close_to_tray}
          onChange={(v) => patch((prev) => ({ ...prev, general: { ...prev.general, close_to_tray: v } }))}
        />
      </Row>
    </>
  );
}

function AppearanceTab({ settings, patch }) {
  const a = settings.appearance;
  return (
    <>
      <GroupLabel>General</GroupLabel>
      <Row label="Default layout" hint="Which of the three dots is selected when the app starts.">
        <LayoutSegment
          value={a.default_layout}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, default_layout: v } }))
          }
        />
      </Row>

      <Divider />
      <GroupLabel>Command layout</GroupLabel>

      <Row
        label="Sidebar position"
        hint="Where the channel list sits in the Command layout."
      >
        <SidebarPositionPicker
          value={a.command_sidebar_position === 'right' ? 'right' : 'left'}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, command_sidebar_position: v } }))
          }
        />
      </Row>

      <Row
        label="Sidebar density"
        hint="Compact halves the row height by hiding the secondary line. Width &amp; collapse: drag the rail edge in-app, or click the rail chevron."
      >
        <DensitySegment
          value={a.command_sidebar_density === 'compact' ? 'compact' : 'comfortable'}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, command_sidebar_density: v } }))
          }
        />
      </Row>

      <Divider />
      <GroupLabel>Colors</GroupLabel>

      <Row
        label="Primary accent"
        hint="Overrides --zinc-100 (active dots, primary button). Clear to use default white."
      >
        <ColorField
          value={a.accent_override}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, accent_override: v } }))
          }
          placeholder="#f4f4f5"
        />
      </Row>

      <Row
        label="Live indicator"
        hint="Overrides the red live-dot color. Clear to use default #ef4444."
      >
        <ColorField
          value={a.live_color_override}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, live_color_override: v } }))
          }
          placeholder="#ef4444"
        />
      </Row>
    </>
  );
}

function SpellcheckSection({ settings, patch }) {
  const c = settings.chat || {};
  const spellcheckEnabled = c.spellcheck_enabled !== false; // default on
  const autocorrectEnabled = c.autocorrect_enabled !== false; // default on
  const currentLang = c.spellcheck_language ?? 'en_US';

  const [dicts, setDicts] = useState(null); // null = loading

  useEffect(() => {
    let cancelled = false;
    spellcheckListDicts()
      .then((list) => {
        if (cancelled) return;
        setDicts(Array.isArray(list) && list.length > 0
          ? list
          : [{ code: 'en_US', name: 'English (US)' }]);
      })
      .catch(() => {
        if (!cancelled) setDicts([{ code: 'en_US', name: 'English (US)' }]);
      });
    return () => { cancelled = true; };
  }, []);

  return (
    <>
      <Row label="Enable spellcheck" hint="Red wavy underlines on misspelled words in the chat composer.">
        <Toggle
          checked={spellcheckEnabled}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, spellcheck_enabled: v } }))}
        />
      </Row>

      <Row
        label="Auto-correct misspelled words"
        hint={spellcheckEnabled
          ? 'Apostrophe expansions and high-confidence single suggestions are auto-applied.'
          : 'Requires spellcheck to be enabled.'}
      >
        <Toggle
          checked={autocorrectEnabled && spellcheckEnabled}
          disabled={!spellcheckEnabled}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, autocorrect_enabled: v } }))}
        />
      </Row>

      <Row label="Language" hint="Hunspell dictionary used for spellcheck.">
        <select
          className="rx-input"
          value={currentLang}
          disabled={!spellcheckEnabled || dicts === null}
          onChange={(e) =>
            patch((prev) => ({ ...prev, chat: { ...c, spellcheck_language: e.target.value } }))
          }
          style={{ width: 240 }}
        >
          {dicts === null ? (
            <option>Loading…</option>
          ) : (
            dicts.map((d) => (
              <option key={d.code} value={d.code}>
                {d.name} ({d.code})
              </option>
            ))
          )}
        </select>
      </Row>
    </>
  );
}

function EventBannerSection({ settings, patch }) {
  const c = settings.chat || {};
  const eb = c.event_banners || {};
  const kinds = eb.kinds || {};
  const enabled = eb.enabled !== false; // default on

  // Default kinds shape if eb.kinds is missing (settings.json predates this field).
  const k = (name, fallback) => (kinds[name] ?? fallback) === true;
  const sub = k('sub', false);
  const resub = k('resub', false);
  const subgift = k('subgift', true);
  const submysterygift = k('submysterygift', true);
  const raid = k('raid', true);
  const bitsbadgetier = k('bitsbadgetier', false);
  const announcement = k('announcement', false);

  const setKind = (name, value) => {
    patch((prev) => ({
      ...prev,
      chat: {
        ...c,
        event_banners: {
          enabled: enabled,
          kinds: {
            sub, resub, subgift, submysterygift,
            raid, bitsbadgetier, announcement,
            [name]: value,
          },
        },
      },
    }));
  };

  return (
    <>
      <Row
        label="Show chat event banners"
        hint={enabled
          ? 'Highlight subscriber events, gift bombs, raids, and announcements above the chat composer.'
          : 'Banners disabled. In-stream rows still appear in chat.'}
      >
        <Toggle
          checked={enabled}
          onChange={(v) => patch((prev) => ({
            ...prev,
            chat: {
              ...c,
              event_banners: {
                enabled: v,
                kinds: { sub, resub, subgift, submysterygift,
                         raid, bitsbadgetier, announcement },
              },
            },
          }))}
        />
      </Row>

      <Row
        label="Show banner for"
        hint={enabled ? null : 'Enable banners above to choose which events surface.'}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <EventKindCheckbox label="Subscriber alerts (new subs)" checked={sub} disabled={!enabled} onChange={(v) => setKind('sub', v)} />
          <EventKindCheckbox label="Resubscriber alerts" checked={resub} disabled={!enabled} onChange={(v) => setKind('resub', v)} />
          <EventKindCheckbox label="Gift subs" checked={subgift} disabled={!enabled} onChange={(v) => setKind('subgift', v)} />
          <EventKindCheckbox label="Mystery gift bombs" checked={submysterygift} disabled={!enabled} onChange={(v) => setKind('submysterygift', v)} />
          <EventKindCheckbox label="Raids and hosts" checked={raid} disabled={!enabled} onChange={(v) => setKind('raid', v)} />
          <EventKindCheckbox label="Bits badge tier-ups" checked={bitsbadgetier} disabled={!enabled} onChange={(v) => setKind('bitsbadgetier', v)} />
          <EventKindCheckbox label="Mod announcements" checked={announcement} disabled={!enabled} onChange={(v) => setKind('announcement', v)} />
        </div>
      </Row>
    </>
  );
}

function EventKindCheckbox({ label, checked, disabled, onChange }) {
  return (
    <label
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        fontSize: 'var(--t-12)',
        color: 'var(--zinc-200)',
      }}
    >
      <input
        type="checkbox"
        checked={checked && !disabled}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      {label}
    </label>
  );
}

function ChatTab({ settings, patch }) {
  const c = settings.chat || {};
  return (
    <>
      <SpellcheckSection settings={settings} patch={patch} />

      <EventBannerSection settings={settings} patch={patch} />

      <Row label="24-hour timestamps">
        <Toggle
          checked={c.timestamp_24h !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, timestamp_24h: v } }))}
        />
      </Row>

      <Row label="Show user badges" hint="Subscriber, premium, partner, founder, bits, …">
        <Toggle
          checked={c.show_badges !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, show_badges: v } }))}
        />
      </Row>

      <Row label="Show mod badges" hint="Broadcaster, moderator, VIP, staff, admin.">
        <Toggle
          checked={c.show_mod_badges !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, show_mod_badges: v } }))}
        />
      </Row>

      <Row label="Show timestamps">
        <Toggle
          checked={c.show_timestamps !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, show_timestamps: v } }))}
        />
      </Row>

      <Row
        label="Show sub anniversary banner"
        hint="When you have a Twitch sub anniversary ready to share, show a banner above chat with a one-click Share button."
      >
        <Toggle
          checked={c.show_sub_anniversary_banner !== false}
          onChange={(v) =>
            patch((prev) => ({ ...prev, chat: { ...c, show_sub_anniversary_banner: v } }))
          }
        />
      </Row>

      <Row
        label="Open user card on hover"
        hint="When off, only clicking the username opens the card."
      >
        <Toggle
          checked={c.user_card_hover !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, user_card_hover: v } }))}
        />
      </Row>

      <Row label="Hover delay (ms)">
        <input
          className="rx-input"
          type="number"
          min="0"
          max="2000"
          step="50"
          value={c.user_card_hover_delay_ms ?? 400}
          onChange={(e) =>
            patch((prev) => ({
              ...prev,
              chat: {
                ...c,
                user_card_hover_delay_ms: Math.max(0, Number(e.target.value) || 0),
              },
            }))
          }
          style={{ width: 90 }}
        />
      </Row>

      <BlockedUsersList />
    </>
  );
}

function BlockedUsersList() {
  const [rows, setRows] = useState([]);
  const refresh = useCallback(() => {
    listBlockedUsers().then(setRows).catch(() => setRows([]));
  }, []);
  useEffect(() => { refresh(); }, [refresh]);
  return (
    <div style={{ marginTop: 4 }}>
      <div style={{ color: 'var(--zinc-300)', marginBottom: 6 }}>Blocked users</div>
      {rows.length === 0 ? (
        <div style={{ color: 'var(--zinc-500)', fontSize: 12 }}>No blocked users.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {rows.map(r => {
            const userKey = `${r.platform}:${r.user_id}`;
            const label = r.last_known_display_name || r.last_known_login || userKey;
            return (
              <div
                key={userKey}
                style={{
                  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                  padding: '6px 8px', background: 'var(--zinc-900)',
                  border: 'var(--hair)', borderRadius: 'var(--r-1)',
                }}
              >
                <span style={{ color: 'var(--zinc-200)' }}>{label}</span>
                <button
                  className="rx-btn rx-btn-ghost"
                  onClick={async () => {
                    try {
                      await setUserMetadata(userKey, { blocked: false });
                    } catch (e) { console.error('set_user_metadata', e); }
                    refresh();
                  }}
                >
                  Unblock
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

function ColorField({ value, onChange, placeholder }) {
  const normalized = /^#[0-9a-f]{6}$/i.test(value) ? value : null;
  return (
    <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
      <input
        type="color"
        value={normalized ?? '#000000'}
        onChange={(e) => onChange(e.target.value)}
        style={{
          width: 30, height: 24, padding: 0, background: 'transparent',
          border: '1px solid var(--zinc-800)', borderRadius: 3, cursor: 'pointer',
        }}
      />
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="rx-input"
        style={{ width: 120 }}
      />
      {value && (
        <button
          type="button"
          onClick={() => onChange('')}
          className="rx-btn rx-btn-ghost"
          style={{ padding: '1px 8px', fontSize: 10 }}
        >
          Clear
        </button>
      )}
    </div>
  );
}

function GroupLabel({ children }) {
  return (
    <div
      style={{
        fontSize: 9,
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: 'var(--zinc-500)',
        fontWeight: 500,
        padding: '2px 0',
        marginTop: 0,
      }}
    >
      {children}
    </div>
  );
}

function Divider() {
  return <hr style={{ border: 'none', borderTop: 'var(--hair)', margin: 0 }} />;
}

function LayoutSegment({ value, onChange }) {
  const opt = (k, label) => (
    <button
      type="button"
      key={k}
      onClick={() => onChange(k)}
      style={{
        background: value === k ? 'var(--zinc-900)' : 'transparent',
        border: `1px solid ${value === k ? 'var(--zinc-800)' : 'transparent'}`,
        borderRadius: 3,
        padding: '5px 10px',
        color: value === k ? 'var(--zinc-200)' : 'var(--zinc-500)',
        cursor: 'pointer',
        fontFamily: 'inherit',
        fontSize: 'var(--t-12)',
      }}
    >
      {label}
    </button>
  );
  return (
    <div style={{ display: 'inline-flex', gap: 2 }}>
      {opt('command', 'A · Command')}
      {opt('columns', 'B · Columns')}
      {opt('focus',   'C · Focus')}
    </div>
  );
}

function DensitySegment({ value, onChange }) {
  const opt = (k, label) => (
    <button
      type="button"
      key={k}
      onClick={() => onChange(k)}
      style={{
        background: value === k ? 'var(--zinc-900)' : 'transparent',
        border: `1px solid ${value === k ? 'var(--zinc-800)' : 'transparent'}`,
        borderRadius: 3,
        padding: '5px 10px',
        color: value === k ? 'var(--zinc-200)' : 'var(--zinc-500)',
        cursor: 'pointer',
        fontFamily: 'inherit',
        fontSize: 'var(--t-12)',
      }}
    >
      {label}
    </button>
  );
  return (
    <div style={{ display: 'inline-flex', gap: 2 }}>
      {opt('comfortable', 'Comfortable')}
      {opt('compact', 'Compact')}
    </div>
  );
}
