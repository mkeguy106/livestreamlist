import { useRef, useState } from 'react';
import ContextMenu from './ContextMenu.jsx';
import Tooltip from './Tooltip.jsx';

/**
 * Streamlink quality strings + human labels. Shared by the Preferences
 * "Default stream quality" dropdown and the per-launch override menu here,
 * so both stay in lockstep. Streamlink falls back to the nearest available
 * quality if the exact one isn't published by the stream.
 */
export const QUALITY_OPTIONS = [
  { value: 'best', label: 'Best' },
  { value: '1080p60', label: '1080p60' },
  { value: '1080p', label: '1080p' },
  { value: '720p60', label: '720p60' },
  { value: '720p', label: '720p' },
  { value: '480p', label: '480p' },
  { value: '360p', label: '360p' },
  { value: '160p', label: '160p' },
  { value: 'audio_only', label: 'Audio only' },
  { value: 'worst', label: 'Worst' },
];

/**
 * Split "Play" button. The left half launches the stream at the user's saved
 * default quality — it calls `onLaunch()` with no argument, so the caller
 * supplies the default. The right chevron half opens a themed, viewport-
 * clamping `ContextMenu` of every streamlink quality; picking one launches at
 * THAT quality as a one-shot override via `onLaunch(quality)` — it does NOT
 * change the saved default.
 *
 * `onLaunch(quality?)` — quality omitted for the default launch.
 * `disabled` — greys both halves (e.g. channel offline).
 */
export default function PlaySplitButton({ onLaunch, disabled }) {
  const [menu, setMenu] = useState(null); // { x, y } | null
  const chevronRef = useRef(null);

  const openMenu = () => {
    const r = chevronRef.current?.getBoundingClientRect();
    if (!r) return;
    // Anchor at the chevron's bottom-right; ContextMenu flips/clamps to fit.
    setMenu({ x: r.right, y: r.bottom + 2 });
  };

  const dim = disabled ? { opacity: 0.4, cursor: 'not-allowed' } : undefined;

  return (
    <div style={{ display: 'inline-flex', alignItems: 'stretch', flexShrink: 0 }}>
      <Tooltip text="Play stream">
        <button
          type="button"
          className="rx-btn rx-btn-primary"
          aria-label="Play stream"
          disabled={disabled}
          onClick={() => !disabled && onLaunch()}
          style={{ borderTopRightRadius: 0, borderBottomRightRadius: 0, ...dim }}
        >
          Play ↗
        </button>
      </Tooltip>
      <Tooltip text="Play at quality…" align="right">
        <button
          type="button"
          ref={chevronRef}
          className="rx-btn rx-btn-primary"
          aria-label="Play at quality…"
          aria-haspopup="menu"
          disabled={disabled}
          onClick={() => !disabled && openMenu()}
          style={{
            borderTopLeftRadius: 0,
            borderBottomLeftRadius: 0,
            // Collapse the seam between the two halves onto one hairline
            // divider, kept dark so it reads against the light primary fill.
            marginLeft: -1,
            borderLeft: '1px solid var(--zinc-400)',
            padding: '3px 6px',
            ...dim,
          }}
        >
          ▾
        </button>
      </Tooltip>
      {menu && (
        <ContextMenu x={menu.x} y={menu.y} onClose={() => setMenu(null)}>
          {QUALITY_OPTIONS.map((q) => (
            <ContextMenu.Item
              key={q.value}
              onClick={() => {
                onLaunch(q.value);
                setMenu(null);
              }}
            >
              {q.label}
            </ContextMenu.Item>
          ))}
        </ContextMenu>
      )}
    </div>
  );
}
