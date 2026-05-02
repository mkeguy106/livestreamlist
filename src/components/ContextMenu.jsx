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

  // Flip to fit viewport. Re-runs on mount AND whenever the menu's size
  // changes (via ResizeObserver) — important because content can arrive
  // async (e.g. spellcheck suggestions fetched after first paint) and
  // the menu's height grows; a one-shot useLayoutEffect would miss that.
  //
  // Y axis: prefer below the click point. If it would overflow the
  // bottom (typical for the chat composer at the bottom of the window),
  // FLIP the menu so its bottom edge sits just above the click point —
  // standard desktop right-click behavior (vs. simply clamping to the
  // bottom edge, which obscures the very element the user right-clicked).
  // X axis: simple clamp inside the viewport.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const reposition = () => {
      const rect = el.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      let nx = x;
      let ny = y;
      if (x + rect.width > vw) nx = Math.max(8, vw - rect.width - 8);
      if (y + rect.height + 8 > vh) {
        // Flip-up. 6 px buffer keeps the menu's bottom edge slightly
        // above the click point (visual breathing room around the word).
        ny = Math.max(8, y - rect.height - 6);
      }
      // Functional setState + identity guard — safe even though pos isn't
      // in the dep array (resize callback can re-fire and we don't want a
      // setState→re-render→setState loop).
      setPos((prev) => (nx !== prev.x || ny !== prev.y ? { x: nx, y: ny } : prev));
    };
    reposition();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(reposition);
    ro.observe(el);
    return () => ro.disconnect();
  }, [x, y]);

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
        // Hard cap so the menu can NEVER be taller than the viewport —
        // worst case it gets a scroll bar instead of cutting items off.
        // 16 px = 8 px buffer top + 8 px bottom.
        maxHeight: 'calc(100vh - 16px)',
        overflowY: 'auto',
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
