import { useEffect, useState } from 'react';
import { useAuth } from '../hooks/useAuth.js';
import { usePreferences } from '../hooks/usePreferences.js';
import { importTwitchFollows } from '../ipc.js';

const TABS = [
  { id: 'general', label: 'General' },
  { id: 'appearance', label: 'Appearance' },
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
          {tab === 'accounts' && <AccountsTab />}
        </div>
      </div>
    </div>
  );
}

function AccountsTab() {
  const { twitch, kick, login, logout } = useAuth();
  const [importState, setImportState] = useState(null); // {running, result, error}

  const runImport = async () => {
    setImportState({ running: true });
    try {
      const r = await importTwitchFollows();
      setImportState({ running: false, result: r });
    } catch (e) {
      setImportState({ running: false, error: String(e?.message ?? e) });
    }
  };

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
    </>
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
      <Row label="Default layout" hint="Which of the three dots is selected when the app starts.">
        <select
          value={a.default_layout}
          onChange={(e) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, default_layout: e.target.value } }))
          }
          className="rx-input"
          style={{ width: 200 }}
        >
          <option value="command">A · Command</option>
          <option value="columns">B · Columns</option>
          <option value="focus">C · Focus</option>
        </select>
      </Row>

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
