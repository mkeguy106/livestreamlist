import { useState } from 'react';

/**
 * Themed tooltip wrapper — replacement for the native `title=""`
 * attribute, which WebKitGTK renders with system styling that breaks
 * the zinc design system. Wrap a hoverable element and a small
 * mono-font popover appears below (or above) it on mouseenter.
 *
 * Identical visual to the inline tooltip that previously lived inside
 * `Command.jsx`'s `IconBtn` — pulled out so every titlebar / sidebar /
 * dialog control can reach it without copy-pasting the popover JSX.
 *
 * The wrapper renders as `inline-flex` so it doesn't disrupt the
 * layout of buttons/icons sitting inside flex rows. `position:
 * relative` is what anchors the absolutely-positioned popover.
 *
 * Pass `text={null}` (or empty string) to disable the tooltip while
 * keeping the wrapper's hover handling — useful when the trigger's
 * tooltip is conditional (e.g. only show while disabled).
 */
export default function Tooltip({ text, placement = 'bottom', children }) {
  const [hover, setHover] = useState(false);
  return (
    <span
      style={{ position: 'relative', display: 'inline-flex' }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      {children}
      {hover && text && (
        <span
          role="tooltip"
          style={{
            position: 'absolute',
            ...(placement === 'top'
              ? { bottom: 'calc(100% + 6px)' }
              : { top: 'calc(100% + 6px)' }),
            left: '50%',
            transform: 'translateX(-50%)',
            padding: '3px 8px',
            background: 'var(--zinc-925)',
            color: 'var(--zinc-300)',
            border: '1px solid var(--zinc-800)',
            borderRadius: 3,
            fontFamily: 'var(--font-mono)',
            fontSize: 10,
            letterSpacing: '.02em',
            whiteSpace: 'nowrap',
            lineHeight: 1.4,
            pointerEvents: 'none',
            boxShadow: '0 4px 12px rgba(0,0,0,.4)',
            zIndex: 50,
          }}
        >
          {text}
        </span>
      )}
    </span>
  );
}
