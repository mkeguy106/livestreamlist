import { useEffect, useRef } from 'react';
import ReactDOM from 'react-dom';

/**
 * Fixed-position right-click menu. Anchored at `(x, y)` in viewport
 * coordinates, auto-flips if it would overflow the bottom/right edge.
 * Closes on outside click, scroll, Escape, or any item activation.
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

  // Flip to fit viewport.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let nx = x;
    let ny = y;
    if (rect.right > vw) nx = Math.max(8, vw - rect.width - 4);
    if (rect.bottom > vh) ny = Math.max(8, vh - rect.height - 4);
    el.style.left = `${nx}px`;
    el.style.top = `${ny}px`;
  }, [x, y]);

  return ReactDOM.createPortal(
    <div
      ref={ref}
      role="menu"
      style={{
        position: 'fixed',
        top: y,
        left: x,
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
