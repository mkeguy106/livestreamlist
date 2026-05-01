/* Variant A picker for Command sidebar position.
 * Two cards each drawing a simplified outline of the app window.
 * Receives `value` ("left" | "right") + `onChange(next)`. */

export default function SidebarPositionPicker({ value, onChange }) {
  return (
    <div style={{ display: 'flex', gap: 8 }}>
      <Card selected={value === 'left'}  side="left"  onClick={() => onChange('left')} />
      <Card selected={value === 'right'} side="right" onClick={() => onChange('right')} />
    </div>
  );
}

function Card({ selected, side, onClick }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      aria-label={`Sidebar ${side}`}
      style={{
        background: selected ? 'var(--zinc-900)' : 'var(--zinc-925)',
        border: `1px solid ${selected ? 'var(--zinc-700)' : 'var(--zinc-800)'}`,
        borderRadius: 4,
        padding: '6px 8px',
        display: 'flex',
        alignItems: 'center',
        gap: 6,
        cursor: 'pointer',
        fontFamily: 'inherit',
        transition: 'border-color 80ms, background 80ms',
      }}
    >
      <Bullet selected={selected} />
      <Glyph side={side} />
      <span style={{ fontSize: 'var(--t-12)', color: selected ? 'var(--zinc-100)' : 'var(--zinc-400)' }}>
        {side === 'left' ? 'Left' : 'Right'}
      </span>
    </button>
  );
}

function Bullet({ selected }) {
  return (
    <span
      style={{
        width: 10,
        height: 10,
        borderRadius: '50%',
        border: `1px solid ${selected ? 'var(--zinc-300)' : 'var(--zinc-700)'}`,
        flexShrink: 0,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <span
        style={{
          width: 5,
          height: 5,
          borderRadius: '50%',
          background: selected ? 'var(--zinc-100)' : 'transparent',
        }}
      />
    </span>
  );
}

function Glyph({ side }) {
  // 60 × 40 outline of the app window. `side` flips which side has the rail.
  // (Compact size — earlier 84×56 overflowed the prefs row at the dialog's
  // current 720px modal width.)
  const railX        = side === 'left' ? 1  : 41;
  const railDivX     = side === 'left' ? 19 : 41;
  const dotX         = side === 'left' ? 3.5 : 43.5;
  const rowsXStart   = side === 'left' ? 4  : 44;
  const mainStart    = side === 'left' ? 23 : 6;
  const mainEnd      = side === 'left' ? 55 : 37;

  return (
    <svg
      width="60"
      height="40"
      viewBox="0 0 60 40"
      fill="none"
      stroke="#52525b"
      strokeWidth="1"
      style={{ flexShrink: 0 }}
    >
      {/* Outer window */}
      <rect x="1" y="1" width="58" height="38" rx="2" />
      {/* Titlebar bottom */}
      <line x1="1" y1="7" x2="59" y2="7" />
      {/* Titlebar dots */}
      <circle cx="4"  cy="4" r="0.8" fill="#52525b" stroke="none" />
      <circle cx="7"  cy="4" r="0.8" fill="#52525b" stroke="none" />
      <circle cx="10" cy="4" r="0.8" fill="#52525b" stroke="none" />
      {/* Sidebar rail (shaded fill + divider) */}
      <rect x={railX} y="7" width="18" height="32" fill="rgba(244,244,245,.04)" stroke="none" />
      <line x1={railDivX} y1="7" x2={railDivX} y2="39" />
      {/* Channel rows */}
      <line x1={rowsXStart} y1="13" x2={rowsXStart + 12} y2="13" stroke="#71717a" />
      <line x1={rowsXStart} y1="19" x2={rowsXStart + 10} y2="19" stroke="#52525b" />
      <line x1={rowsXStart} y1="25" x2={rowsXStart + 12} y2="25" stroke="#52525b" />
      {/* Live dot on first row */}
      <circle cx={dotX} cy="13" r="0.8" fill="#ef4444" stroke="none" />
      {/* Main pane lines (chat-line placeholders) */}
      <line x1={mainStart} y1="13" x2={mainEnd}     y2="13" stroke="#3f3f46" />
      <line x1={mainStart} y1="19" x2={mainEnd - 6} y2="19" stroke="#27272a" />
      <line x1={mainStart} y1="25" x2={mainEnd - 3} y2="25" stroke="#27272a" />
    </svg>
  );
}
