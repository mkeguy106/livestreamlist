import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import ReactDOM from 'react-dom';

/**
 * Fixed-position right-click menu. Anchored at `(x, y)` in viewport
 * coordinates, auto-flips if it would overflow the right/bottom edge.
 * Closes on outside click, scroll, Escape, or any item activation.
 *
 * Position is state-driven and computed in a layout effect so the
 * corrected coords are applied before paint — avoids the visible
 * flicker the previous useEffect+imperative-style approach had, and
 * keeps React's reconciler from clobbering the fix on re-renders.
 *
 * Usage:
 *   <ContextMenu x={e.clientX} y={e.clientY} onClose={…}>
 *     <ContextMenu.Item onClick={…}>Play</ContextMenu.Item>
 *     <ContextMenu.Separator />
 *     <ContextMenu.Item onClick={…} danger>Delete</ContextMenu.Item>
 *   </ContextMenu>
 */
export default function ContextMenu({ x, y, onClose, children }) {
  const ref = useRef(null);
  // Track the applied position separately from the requested anchor so
  // the layout effect can flip it without a tug-of-war with JSX inline.
  const [pos, setPos] = useState({ x, y });

  useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'Escape') onClose();
    };
    const onDown = (e) => {
      if (ref.current && !ref.current.contains(e.target)) onClose();
    };
    const onScroll = () => onClose();
    document.addEventListener('keydown', onKey);
    document.addEventListener('mousedown', onDown);
    document.addEventListener('wheel', onScroll, { passive: true });
    return () => {
      document.removeEventListener('keydown', onKey);
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('wheel', onScroll);
    };
  }, [onClose]);

  // Flip to fit viewport. useLayoutEffect runs synchronously after DOM
  // mutation but BEFORE the browser paints — the corrected coords land
  // in place without the menu briefly rendering off-screen first.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let nx = x;
    let ny = y;
    if (x + rect.width  > vw) nx = Math.max(8, vw - rect.width  - 8);
    if (y + rect.height > vh) ny = Math.max(8, vh - rect.height - 8);
    if (nx !== pos.x || ny !== pos.y) setPos({ x: nx, y: ny });
  }, [x, y, pos.x, pos.y]);

  return ReactDOM.createPortal(
    <div
      ref={ref}
      role="menu"
      style={{
        position: 'fixed',
        top: pos.y,
        left: pos.x,
        zIndex: 200,
        minWidth: 180,
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 6,
        boxShadow: '0 12px 32px rgba(0,0,0,.6)',
        padding: 4,
        fontSize: 'var(--t-12)',
        display: 'flex',
        flexDirection: 'column',
        gap: 1,
      }}
    >
      {children}
    </div>,
    document.body,
  );
}

function Item({ onClick, disabled, danger, children }) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onClick?.();
      }}
      style={{
        textAlign: 'left',
        padding: '6px 10px',
        background: 'transparent',
        border: 'none',
        color: disabled
          ? 'var(--zinc-600)'
          : danger
            ? '#f87171'
            : 'var(--zinc-200)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        fontFamily: 'inherit',
        fontSize: 'var(--t-12)',
        borderRadius: 3,
      }}
      onMouseEnter={(e) => {
        if (!disabled) e.currentTarget.style.background = 'var(--zinc-900)';
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = 'transparent';
      }}
    >
      {children}
    </button>
  );
}

function Separator() {
  return <div style={{ borderTop: 'var(--hair)', margin: '4px 0' }} />;
}

ContextMenu.Item = Item;
ContextMenu.Separator = Separator;
