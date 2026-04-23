import { useCallback, useEffect, useState } from 'react';
import Command from './directions/Command.jsx';
import Columns from './directions/Columns.jsx';
import Focus from './directions/Focus.jsx';

const LAYOUTS = [
  { id: 'command', label: 'Command', letter: 'A', Component: Command, rightLabel: '7 live · 18 channels', showKbd: true },
  { id: 'columns', label: 'Columns', letter: 'B', Component: Columns, rightLabel: '5 columns · 7 live', showKbd: true },
  { id: 'focus',   label: 'Focus',   letter: 'C', Component: Focus,   rightLabel: 'focus: shroud',        showKbd: false },
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
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [selectLayout]);

  const current = LAYOUTS.find((l) => l.id === layoutId) ?? LAYOUTS[0];
  const Layout = current.Component;

  return (
    <div className="rx-root">
      <div className="rx-titlebar">
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
        <div className="rx-tb-label rx-mono">{current.rightLabel}</div>
        {current.showKbd && <div className="rx-kbd">⌘K</div>}
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
        <Layout />
      </main>
    </div>
  );
}
