import { useCallback, useEffect, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { formatRelative } from '../utils/format.js';
import Tooltip from './Tooltip.jsx';
import SidebarPositionPicker from './SidebarPositionPicker.jsx';
import {
  importTwitchFollows,
  listBlockedUsers,
  setUserMetadata,
  youtubeDetectBrowsers,
} from '../ipc.js';

const TABS = [
  { id: 'general', label: 'General' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'chat', label: 'Chat' },
  { id: 'accounts', label: 'Accounts' },
];

export default function PreferencesDialog({ open, onClose }) {
  const [tab, setTab] = useState('general');
  const { settings, error, patch } = usePreferences();

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
              {t.label}
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
  const { twitch, kick, youtube, chaturbate, login, logout, loginYoutubePaste, refresh } = useAuth();
  const { settings, patch } = usePreferences();
  const [importState, setImportState] = useState(null); // {running, result, error}
  const [ytLoginRunning, setYtLoginRunning] = useState(false);
  const [cbLoginRunning, setCbLoginRunning] = useState(false);
  const [cbError, setCbError] = useState(null);

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

  const runImport = async () => {
    setImportState({ running: true });
    try {
      const r = await importTwitchFollows();
      setImportState({ running: false, result: r });
    } catch (e) {
      setImportState({ running: false, error: String(e?.message ?? e) });
    }
  };

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

  return (
    <>
      <Row label="Twitch" hint={twitch ? `Logged in as @${twitch.login}` : 'Not logged in'}>
        {twitch ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('twitch')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={() => login('twitch')}>
            Log in to Twitch
          </button>
        )}
      </Row>

      <Row label="Kick" hint={kick ? `Logged in as @${kick.login}` : 'Not logged in'}>
        {kick ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('kick')}>
            Log out
          </button>
        ) : (
          <button type="button" className="rx-btn" onClick={() => login('kick')}>
            Log in to Kick
          </button>
        )}
      </Row>

      <Row
        label="YouTube"
        hint={
          ytBrowser
            ? `Using cookies from ${ytLabelFor(ytBrowser)}`
            : youtube?.has_paste
            ? 'Signed in via Google'
            : 'Sign in for subs / age-restricted / member content'
        }
      >
        {ytBrowser || youtube?.has_paste ? (
          <button type="button" className="rx-btn rx-btn-ghost" onClick={() => logout('youtube')}>
            Log out
          </button>
        ) : (
          <button
            type="button"
            className="rx-btn"
            onClick={runYoutubeLogin}
            disabled={ytLoginRunning}
          >
            {ytLoginRunning ? 'Waiting on Google…' : 'Log in to YouTube'}
          </button>
        )}
        <div style={{ marginTop: 6 }}>
          <button
            type="button"
            onClick={() => setYtAdvanced((v) => !v)}
            style={{
              all: 'unset',
              cursor: 'pointer',
              fontSize: 'var(--t-11)',
              color: 'var(--zinc-500)',
            }}
          >
            {ytAdvanced ? '▾ Other ways to sign in' : '▸ Other ways to sign in'}
          </button>
        </div>
        {ytAdvanced && (
          <div
            style={{
              marginTop: 6,
              padding: '8px 10px',
              border: '1px solid var(--zinc-800)',
              borderRadius: 4,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
            }}
          >
            <div>
              <div
                style={{
                  fontSize: 'var(--t-11)',
                  color: 'var(--zinc-300)',
                  marginBottom: 4,
                  fontWeight: 500,
                }}
              >
                Use cookies from a browser
              </div>
              <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginBottom: 6 }}>
                Reuses an existing browser session — no extra sign-in needed.
              </div>
              {browsers === null ? (
                <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>Detecting…</div>
              ) : browsers.length === 0 ? (
                <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)' }}>
                  No supported browsers found on this system.
                </div>
              ) : (
                <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                  {browsers.map((b) => {
                    const active = ytBrowser === b.id;
                    return (
                      <Tooltip
                        key={b.id}
                        text={active ? `Stop using ${b.label} cookies` : `Use ${b.label} cookies`}
                      >
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
              <div
                style={{
                  fontSize: 'var(--t-11)',
                  color: 'var(--zinc-300)',
                  marginBottom: 4,
                  fontWeight: 500,
                }}
              >
                Paste cookies
              </div>
              <div style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-500)', marginBottom: 6 }}>
                For sandboxed builds (Flatpak) where the app can't reach a browser cookie store.
              </div>
              <button
                type="button"
                className="rx-btn rx-btn-ghost"
                onClick={() => setYtPasteOpen(true)}
              >
                Paste cookies…
              </button>
            </div>
          </div>
        )}
        {ytError && (
          <div style={{ marginTop: 6, fontSize: 'var(--t-11)', color: '#f87171' }}>{ytError}</div>
        )}
      </Row>

      <Row
        label="Chaturbate"
        hint={
          chaturbate?.signed_in
            ? `Signed in · verified ${formatRelative(chaturbate.last_verified_at)}`
            : 'Sign in to chat as yourself'
        }
      >
        {chaturbate?.signed_in ? (
          <div style={{ display: 'flex', gap: 6 }}>
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={runChaturbateLogin}
              disabled={cbLoginRunning}
            >
              {cbLoginRunning ? 'Signing in…' : 'Sign in again'}
            </button>
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={() => logout('chaturbate')}
            >
              Log out
            </button>
          </div>
        ) : (
          <button
            type="button"
            className="rx-btn"
            onClick={runChaturbateLogin}
            disabled={cbLoginRunning}
          >
            {cbLoginRunning ? 'Waiting on Chaturbate…' : 'Sign in to Chaturbate'}
          </button>
        )}
        {cbError && (
          <div style={{ marginTop: 6, fontSize: 'var(--t-11)', color: 'var(--live)' }}>
            {cbError}
          </div>
        )}
      </Row>

      <Row
        label="Import Twitch follows"
        hint="Adds every channel you follow on Twitch to this app. Existing entries are skipped."
      >
        <button
          type="button"
          className="rx-btn"
          onClick={runImport}
          disabled={!twitch || importState?.running}
        >
          {importState?.running ? 'Importing…' : 'Import now'}
        </button>
        {importState?.result && (
          <div style={{ marginTop: 6, fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
            Added {importState.result.added} · skipped {importState.result.skipped} · seen {importState.result.total_seen}
          </div>
        )}
        {importState?.error && (
          <div style={{ marginTop: 6, fontSize: 'var(--t-11)', color: '#f87171' }}>
            {importState.error}
          </div>
        )}
      </Row>

      <YoutubePasteDialog
        open={ytPasteOpen}
        onClose={() => setYtPasteOpen(false)}
        onSubmit={async (text) => {
          await loginYoutubePaste(text);
          setYtPasteOpen(false);
        }}
      />
    </>
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

function Toggle({ checked, onChange }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      aria-pressed={checked}
      style={{
        width: 36, height: 20, borderRadius: 10, border: 'none',
        background: checked ? 'var(--zinc-100)' : 'var(--zinc-800)',
        position: 'relative', cursor: 'pointer',
        transition: 'background 120ms',
      }}
    >
      <span style={{
        position: 'absolute', top: 2, left: checked ? 18 : 2,
        width: 16, height: 16, borderRadius: '50%',
        background: checked ? 'var(--zinc-950)' : 'var(--zinc-400)',
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

function ChatTab({ settings, patch }) {
  const c = settings.chat || {};
  return (
    <>
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
