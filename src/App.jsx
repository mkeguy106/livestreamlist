import { useCallback, useEffect, useMemo, useState } from 'react';
import Command from './directions/Command.jsx';
import Columns from './directions/Columns.jsx';
import Focus from './directions/Focus.jsx';
import AddChannelDialog from './components/AddChannelDialog.jsx';
import LoginButton from './components/LoginButton.jsx';
import UserCard from './components/UserCard.jsx';
import WindowControls from './components/WindowControls.jsx';
import PreferencesDialog from './components/PreferencesDialog.jsx';
import { useDragHandler } from './hooks/useDragRegion.js';
import { useLivestreams } from './hooks/useLivestreams.js';
import { usePreferences } from './hooks/usePreferences.js';
import { useUserCard } from './hooks/useUserCard.js';
import { launchStream, listenEvent, openInBrowser, removeChannel, setFavorite } from './ipc.js';

const LAYOUTS = [
  { id: 'command', label: 'Command', letter: 'A', Component: Command },
  { id: 'columns', label: 'Columns', letter: 'B', Component: Columns },
  { id: 'focus',   label: 'Focus',   letter: 'C', Component: Focus   },
];
const STORAGE_KEY = 'livestreamlist.layout';

function loadInitialLayout() {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved && LAYOUTS.some((l) => l.id === saved)) return saved;
  } catch {}
  return LAYOUTS[0].id;
}

export default function App() {
  const [layoutId, setLayoutId] = useState(loadInitialLayout);
  const [addOpen, setAddOpen] = useState(false);
  const [prefsOpen, setPrefsOpen] = useState(false);
  const [selectedKey, setSelectedKey] = useState(null);

  const { settings } = usePreferences();
  const intervalSeconds = settings?.general?.refresh_interval_seconds;
  const { livestreams, loading, error, refresh } = useLivestreams({ intervalSeconds });
  const onTitlebarMouseDown = useDragHandler();
  const card = useUserCard();

  // Hover and right-click placeholders — wired in Tasks 16 and 21.
  const onUsernameOpen = useCallback(
    (user, rect, channelKey) => card.openFor(user, channelKey, rect),
    [card],
  );
  const onUsernameContext = useCallback(() => {}, []);
  const onUsernameHover = useCallback(() => {}, []);

  // Apply appearance overrides to CSS variables on the root.
  useEffect(() => {
    const root = document.documentElement;
    const accent = settings?.appearance?.accent_override;
    const live = settings?.appearance?.live_color_override;
    if (accent && /^#[0-9a-f]{6}$/i.test(accent)) {
      root.style.setProperty('--zinc-100', accent);
    } else {
      root.style.removeProperty('--zinc-100');
    }
    if (live && /^#[0-9a-f]{6}$/i.test(live)) {
      root.style.setProperty('--live', live);
    } else {
      root.style.removeProperty('--live');
    }
  }, [settings?.appearance?.accent_override, settings?.appearance?.live_color_override]);

  // Honor default layout on first launch (i.e. when localStorage hasn't been
  // written yet by a user-driven switch).
  useEffect(() => {
    const saved = (() => { try { return localStorage.getItem(STORAGE_KEY); } catch { return null; } })();
    if (saved) return;
    const def = settings?.appearance?.default_layout;
    if (def && LAYOUTS.some((l) => l.id === def)) setLayoutId(def);
  }, [settings?.appearance?.default_layout]);

  // Default selection: first live channel, else first in list.
  useEffect(() => {
    if (selectedKey && livestreams.some((l) => l.unique_key === selectedKey)) return;
    const firstLive = livestreams.find((l) => l.is_live);
    const first = firstLive ?? livestreams[0];
    setSelectedKey(first?.unique_key ?? null);
  }, [livestreams, selectedKey]);

  useEffect(() => {
    try { localStorage.setItem(STORAGE_KEY, layoutId); } catch {}
  }, [layoutId]);

  const selectLayout = useCallback((id) => setLayoutId(id), []);

  useEffect(() => {
    const onKey = (e) => {
      const target = e.target;
      const inField = target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
      if (inField) return;
      if (e.key === '1') selectLayout('command');
      else if (e.key === '2') selectLayout('columns');
      else if (e.key === '3') selectLayout('focus');
      else if (e.key === 'n' || (e.key.toLowerCase() === 'a' && (e.metaKey || e.ctrlKey) && e.shiftKey)) {
        e.preventDefault();
        setAddOpen(true);
      } else if (e.key.toLowerCase() === 'r' && !(e.metaKey || e.ctrlKey)) {
        refresh();
      } else if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        setPrefsOpen(true);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [selectLayout, refresh]);

  // Tray "Refresh now" menu item fires this event.
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    (async () => {
      unlisten = await listenEvent('tray:refresh-requested', () => {
        if (!cancelled) refresh();
      });
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [refresh]);

  const ctx = useMemo(() => ({
    livestreams,
    loading,
    error,
    refresh,
    selectedKey,
    setSelectedKey,
    openAddDialog: () => setAddOpen(true),
    launchStream: (key, quality = 'best') =>
      launchStream(key, quality).catch((e) => console.error('launch_stream', e)),
    openInBrowser: (key) =>
      openInBrowser(key).catch((e) => console.error('open_in_browser', e)),
    removeChannel: (key) =>
      removeChannel(key).then(refresh).catch((e) => console.error('remove_channel', e)),
    setFavorite: (key, fav) =>
      setFavorite(key, fav).then(refresh).catch((e) => console.error('set_favorite', e)),
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  }), [livestreams, loading, error, refresh, selectedKey, onUsernameOpen, onUsernameContext, onUsernameHover]);

  const current = LAYOUTS.find((l) => l.id === layoutId) ?? LAYOUTS[0];
  const Layout = current.Component;

  const liveCount = livestreams.filter((l) => l.is_live).length;
  const totalCount = livestreams.length;
  const selected = livestreams.find((l) => l.unique_key === selectedKey);

  const rightLabel = layoutId === 'focus' && selected
    ? `focus: ${selected.display_name}`
    : layoutId === 'columns'
    ? `${liveCount} live · ${totalCount} channels`
    : `${liveCount} live · ${totalCount} channels`;

  return (
    <div className="rx-root">
      <div className="rx-titlebar" data-tauri-drag-region onMouseDown={onTitlebarMouseDown}>
        <div className="rx-tb-dots" role="tablist" aria-label="Layout">
          {LAYOUTS.map((l) => (
            <button
              key={l.id}
              type="button"
              role="tab"
              aria-selected={l.id === layoutId}
              aria-label={`${l.label} layout (${l.letter})`}
              title={`${l.letter} · ${l.label}`}
              className={`rx-tb-dot ${l.id === layoutId ? 'active' : ''}`}
              onClick={() => selectLayout(l.id)}
            />
          ))}
        </div>
        <div style={{ width: 12 }} />
        <div className="rx-tb-label rx-mono">livestream.list</div>
        <div style={{ color: 'var(--zinc-700)' }}>·</div>
        <div className="rx-tb-label rx-mono" style={{ color: 'var(--zinc-400)' }}>
          {current.letter} · {current.label}
        </div>
        <div style={{ flex: 1 }} />
        <div className="rx-tb-label rx-mono">{rightLabel}</div>
        {error && <div className="rx-tb-label rx-mono" style={{ color: '#f87171' }}>· refresh failed</div>}
        <div style={{ width: 8 }} />
        <button
          type="button"
          className="rx-btn rx-btn-ghost"
          onClick={() => setPrefsOpen(true)}
          title="Preferences (⌘,)"
          style={{ padding: '2px 6px', fontSize: 10 }}
        >
          ⚙
        </button>
        <LoginButton />
        <div style={{ width: 4 }} />
        <WindowControls />
      </div>

      <main
        style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          minHeight: 0,
          position: 'relative',
        }}
      >
        {totalCount === 0 ? <EmptyState onAdd={() => setAddOpen(true)} /> : <Layout ctx={ctx} />}
      </main>

      <AddChannelDialog open={addOpen} onClose={() => setAddOpen(false)} onAdded={refresh} />
      <PreferencesDialog open={prefsOpen} onClose={() => setPrefsOpen(false)} />
      <UserCard
        open={card.open}
        anchor={card.anchor}
        user={card.user}
        metadata={card.metadata}
        profile={card.profile}
        profileLoading={card.profileLoading}
        profileError={card.profileError}
        onClose={card.close}
        onOpenHistory={() => { card.close(); }}
        onOpenChannel={() => {
          if (card.channelKey) openInBrowser(card.channelKey).catch((e) => console.error('open_in_browser', e));
          card.close();
        }}
      />
    </div>
  );
}

function EmptyState({ onAdd }) {
  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 14,
        color: 'var(--zinc-400)',
      }}
    >
      <div style={{ fontSize: 14, color: 'var(--zinc-100)', fontWeight: 500 }}>No channels yet</div>
      <div style={{ fontSize: 12, color: 'var(--zinc-500)', maxWidth: 420, textAlign: 'center' }}>
        Paste a Twitch, YouTube, Kick, or Chaturbate URL to start monitoring.
      </div>
      <button className="rx-btn rx-btn-primary" onClick={onAdd}>
        Add channel
      </button>
      <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>press N to add</div>
    </div>
  );
}
