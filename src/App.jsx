import { useCallback, useEffect, useMemo, useState } from 'react';
import Command from './directions/Command.jsx';
import Columns from './directions/Columns.jsx';
import Focus from './directions/Focus.jsx';
import AddChannelDialog from './components/AddChannelDialog.jsx';
import WindowControls from './components/WindowControls.jsx';
import { useLivestreams } from './hooks/useLivestreams.js';
import { launchStream, openInBrowser, removeChannel, setFavorite } from './ipc.js';

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
  const [selectedKey, setSelectedKey] = useState(null);

  const { livestreams, loading, error, refresh } = useLivestreams();

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
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [selectLayout, refresh]);

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
  }), [livestreams, loading, error, refresh, selectedKey]);

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
      <div className="rx-titlebar" data-tauri-drag-region>
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
