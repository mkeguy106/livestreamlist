import { useEffect, useMemo, useRef, useState } from 'react';
import { buildPickerModel } from '../utils/pickerModel.js';
import Tooltip from './Tooltip.jsx';

// The grid's `repeat(auto-fill, 40px)` CSS lays cells out responsively, but
// the keyboard model needs a fixed column count to do flat-index arithmetic
// (±1 for left/right, ±COLUMNS for up/down). 420px panel width, minus the
// body's padding, fits 9 columns of 40px cells + 4px gaps — matches the
// panel width the spec calls for. If the panel width ever changes, this
// constant needs to move with it (there's no live measurement — see the
// "Simple index model" note in the brief).
const COLUMNS = 9;

/**
 * Self-contained emote picker panel. Parent owns:
 *  - a `position: relative` wrapper to anchor this panel against (this
 *    component positions itself absolutely within that wrapper)
 *  - open/closed state (mount/unmount this component)
 *  - the emote list (fetched once, shared with autocomplete — see
 *    Composer.jsx wiring)
 *
 * `onInsert(name, { keepOpen })` fires on click/Enter; `onClose()` fires on
 * Esc or outside click. `onRetry` is optional — pass it to show a Retry
 * button in the "couldn't load" empty state; omit it to hide the button.
 */
export default function EmotePicker({ emotes, onInsert, onClose, onRetry }) {
  const panelRef = useRef(null);
  const searchRef = useRef(null);
  const gridRef = useRef(null);

  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all'); // 'all' | 'animated' | 'static'
  const [selected, setSelected] = useState(-1); // flat index into the visible list; -1 = none

  const sections = useMemo(
    () => buildPickerModel(emotes ?? [], { query, filter }),
    [emotes, query, filter],
  );

  // Flattened visible list — this is the ONLY geometry the keyboard model
  // needs. Arrow keys move `selected` through this array by ±1 (left/right)
  // or ±COLUMNS (up/down); there is no 2D row/column bookkeeping. Moving
  // past either end just clamps (see onGridKeyDown) rather than wrapping
  // across section boundaries — a flat list has no natural "row" concept
  // at a section seam, so clamping is the honest behavior here.
  const flat = useMemo(() => sections.flatMap((s) => s.emotes), [sections]);

  // Selection can go stale when the query/filter changes the flat list out
  // from under it (e.g. the selected emote got filtered out). Clamp back
  // into range whenever the list changes.
  useEffect(() => {
    setSelected((prev) => {
      if (flat.length === 0) return -1;
      if (prev < 0) return prev;
      return Math.min(prev, flat.length - 1);
    });
  }, [flat.length]);

  // --- Outside click -------------------------------------------------
  useEffect(() => {
    const onDown = (e) => {
      if (panelRef.current && !panelRef.current.contains(e.target)) onClose?.();
    };
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [onClose]);

  // --- Viewport culling for animated emotes ---------------------------
  //
  // The spec's original idea was "swap to the static CDN variant when
  // off-screen" — but 7TV/BTTV don't uniformly expose a static-URL sibling
  // in the payload we get, so deriving one per-provider would mean baking
  // CDN-specific URL knowledge into this component. Refinement actually
  // shipped: a `data-src` pattern. Every animated `<img>` renders with NO
  // `src` by default; the real URL lives on `data-src`. A single
  // IntersectionObserver (root = the scrollable body, rootMargin 100px so
  // cells just outside the fold are pre-loaded before they're scrolled
  // into view) flips `src` on when a cell intersects and clears it again
  // when it leaves. Clearing `src` fully unloads the image — the browser
  // stops decoding/animating a GIF/WebP the moment it's off-screen —
  // which is a stronger cull than a static-variant swap AND needs zero
  // per-CDN URL logic. Static emotes never get culled (no benefit; they
  // don't animate).
  const bodyRef = useRef(null);
  useEffect(() => {
    const root = bodyRef.current;
    if (!root || typeof IntersectionObserver === 'undefined') return undefined;

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const img = entry.target;
          if (entry.isIntersecting) {
            const real = img.getAttribute('data-src');
            if (real && img.getAttribute('src') !== real) img.setAttribute('src', real);
          } else {
            img.removeAttribute('src');
          }
        }
      },
      { root, rootMargin: '100px' },
    );

    const imgs = root.querySelectorAll('img[data-animated="true"]');
    imgs.forEach((img) => observer.observe(img));

    // Disconnect on unmount AND whenever the model (flat list) changes —
    // stale observed nodes from a previous section/filter would otherwise
    // leak (the observer holds a reference to every observed element,
    // so re-observing without disconnecting first accumulates DOM nodes
    // that no longer exist in the tree once React re-renders the grid).
    return () => observer.disconnect();
  }, [flat]);

  const insert = (emote, keepOpen) => {
    if (!emote || emote.locked) return;
    onInsert?.(emote.name, { keepOpen: !!keepOpen });
    if (!keepOpen) onClose?.();
  };

  const onGridKeyDown = (e) => {
    if (flat.length === 0) return;
    const clamp = (i) => Math.max(0, Math.min(flat.length - 1, i));
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      setSelected((s) => clamp(s < 0 ? 0 : s + 1));
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      setSelected((s) => clamp(s < 0 ? 0 : s - 1));
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelected((s) => clamp(s < 0 ? 0 : s + COLUMNS));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelected((s) => clamp(s < 0 ? 0 : s - COLUMNS));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const emote = flat[selected];
      insert(emote, e.shiftKey);
    }
  };

  const onPanelKeyDown = (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose?.();
      return;
    }
    // ArrowDown from the search input hands focus (and the keyboard model)
    // to the grid. Once focus is inside the grid, onGridKeyDown handles
    // subsequent arrows.
    if (
      e.key === 'ArrowDown' &&
      document.activeElement === searchRef.current
    ) {
      e.preventDefault();
      setSelected((s) => (s < 0 && flat.length > 0 ? 0 : s));
      gridRef.current?.focus();
      return;
    }
    if (document.activeElement !== searchRef.current) onGridKeyDown(e);
  };

  const noEmotesAtAll = (emotes?.length ?? 0) === 0;
  const noMatches = !noEmotesAtAll && flat.length === 0;

  return (
    <div
      ref={panelRef}
      onKeyDown={onPanelKeyDown}
      style={{
        position: 'absolute',
        bottom: 'calc(100% + 6px)',
        right: 0,
        width: 420,
        height: 360,
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 'var(--r-3)',
        boxShadow: '0 12px 40px rgba(0,0,0,.55)',
        boxSizing: 'border-box',
        display: 'flex',
        flexDirection: 'column',
        zIndex: 30,
      }}
    >
      {/* Header — search + filter segmented control, pinned above the scroll body */}
      <div
        style={{
          flexShrink: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 6,
          padding: 8,
          borderBottom: 'var(--hair)',
          boxSizing: 'border-box',
        }}
      >
        <input
          ref={searchRef}
          className="rx-input"
          autoFocus
          placeholder="Search emotes…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          style={{ width: '100%' }}
        />
        <div style={{ display: 'flex', gap: 4 }}>
          {[
            { key: 'all', label: 'All' },
            { key: 'animated', label: 'Animated' },
            { key: 'static', label: 'Static' },
          ].map((f) => (
            <button
              key={f.key}
              type="button"
              className="rx-btn"
              aria-pressed={filter === f.key}
              onClick={() => setFilter(f.key)}
              style={{
                background: filter === f.key ? 'var(--zinc-800)' : undefined,
                color: filter === f.key ? 'var(--zinc-100)' : undefined,
              }}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>

      {/* Body — scrollable sections, or an empty state */}
      <div
        ref={bodyRef}
        tabIndex={-1}
        style={{ flex: 1, minHeight: 0, overflowY: 'auto', padding: noEmotesAtAll || noMatches ? 0 : 8 }}
      >
        {noEmotesAtAll ? (
          <EmptyState
            text="Couldn't load emotes"
            onRetry={onRetry}
          />
        ) : noMatches ? (
          <EmptyState text="No emotes match" />
        ) : (
          <div ref={gridRef} tabIndex={-1}>
            {sections.map((section) => (
              <Section
                key={section.title}
                section={section}
                flat={flat}
                selected={selected}
                onSelect={setSelected}
                onInsert={insert}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function EmptyState({ text, onRetry }) {
  return (
    <div
      style={{
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 10,
        color: 'var(--zinc-500)',
        fontSize: 'var(--t-12)',
      }}
    >
      <span>{text}</span>
      {onRetry && (
        <button type="button" className="rx-btn" onClick={onRetry}>
          Retry
        </button>
      )}
    </div>
  );
}

function Section({ section, flat, selected, onSelect, onInsert }) {
  // Index of this section's first emote within the flattened list — lets
  // each cell compute its own flat index without the section needing to
  // know about its neighbors.
  const startIndex = flat.indexOf(section.emotes[0]);

  return (
    <div style={{ marginBottom: 8 }}>
      <div
        className="rx-mono"
        style={{
          position: 'sticky',
          top: 0,
          background: 'var(--zinc-925)',
          fontSize: 'var(--t-10)',
          color: 'var(--zinc-500)',
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
          padding: '4px 0',
          zIndex: 1,
        }}
      >
        {section.title}
      </div>
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, 40px)',
          gap: 4,
        }}
      >
        {section.emotes.map((e, i) => {
          const flatIndex = startIndex + i;
          // Each section renders its own CSS grid (wraps independently of
          // its neighbors), so the rightmost-column test is section-local:
          // i % COLUMNS === COLUMNS - 1. Tooltips on that last column
          // anchor `align="right"` so the popover's right edge lines up
          // with the cell instead of centering — a centered popover on
          // the rightmost cell would overflow the panel's own right edge
          // (the panel itself sits flush against the composer's right
          // edge, so there's no room to spill further right).
          const isRightEdge = i % COLUMNS === COLUMNS - 1;
          return (
            <Cell
              key={e.name}
              emote={e}
              isSelected={flatIndex === selected}
              align={isRightEdge ? 'right' : 'center'}
              onHover={() => onSelect(flatIndex)}
              onInsert={(shiftKey) => onInsert(e, shiftKey)}
            />
          );
        })}
      </div>
    </div>
  );
}

function Cell({ emote, isSelected, align, onHover, onInsert }) {
  const locked = !!emote.locked;
  return (
    <Tooltip text={locked ? 'Subscribe to use' : emote.name} align={align}>
      <button
        type="button"
        aria-label={locked ? 'Subscribe to use' : emote.name}
        disabled={locked}
        onMouseEnter={onHover}
        onClick={(e) => {
          if (locked) return;
          onInsert(e.shiftKey);
        }}
        style={{
          width: 40,
          height: 40,
          boxSizing: 'border-box',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: 'transparent',
          border: 'none',
          borderRadius: 'var(--r-2)',
          outline: isSelected ? '1px solid var(--zinc-400)' : 'none',
          outlineOffset: -1,
          opacity: locked ? 0.4 : 1,
          cursor: locked ? 'not-allowed' : 'pointer',
          padding: 0,
        }}
      >
        <img
          loading="lazy"
          width={28}
          height={28}
          alt={emote.name}
          data-animated={emote.animated ? 'true' : undefined}
          // Static emotes render immediately via `src`. Animated emotes
          // start with NO `src` — the culling IntersectionObserver (see
          // the panel-level effect above) sets it the moment the cell
          // intersects the scroll body, and clears it again once it
          // scrolls out. The real URL always lives on `data-src` so the
          // observer can restore it without re-deriving anything.
          {...(emote.animated
            ? { 'data-src': emote.url_1x }
            : { src: emote.url_1x })}
        />
      </button>
    </Tooltip>
  );
}
