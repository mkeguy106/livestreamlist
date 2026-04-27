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
export default function Tooltip({
  text,
  placement = 'bottom',
  align = 'center',
  wrap = false,
  block = false,
  wrapperStyle,
  children,
}) {
  const [hover, setHover] = useState(false);

  // Horizontal anchoring. `center` (default) is fine for elements far
  // from the viewport edges. `right` anchors the popover's right edge
  // to the trigger's right edge — use it for triggers near the app's
  // right edge so the popover doesn't overflow the window. `left` is
  // the mirror for triggers near the left edge.
  const horizontal = (() => {
    if (align === 'left') return { left: 0 };
    if (align === 'right') return { right: 0 };
    return { left: '50%', transform: 'translateX(-50%)' };
  })();

  return (
    <span
      style={{
        position: 'relative',
        // `block` makes the wrapper stretch the full width of its
        // parent — needed when wrapping full-width buttons (e.g. the
        // channel rail row) where the default `inline-flex` would
        // shrink the click target to its content.
        display: block ? 'flex' : 'inline-flex',
        width: block ? '100%' : undefined,
        // `wrapperStyle` is the escape hatch for cases where the
        // wrapper itself needs vertical-align / margin tweaks (chat
        // emotes need the verticalAlign that previously lived on the
        // <img> they're wrapping).
        ...wrapperStyle,
      }}
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
            ...horizontal,
            padding: '3px 8px',
            background: 'var(--zinc-925)',
            color: 'var(--zinc-300)',
            border: '1px solid var(--zinc-800)',
            borderRadius: 3,
            fontFamily: 'var(--font-mono)',
            fontSize: 10,
            letterSpacing: '.02em',
            // wrap=true is for content that genuinely benefits from
            // multi-line display: stream titles, URLs, reply previews.
            // Capped width keeps the popover from spanning the
            // viewport when the text is paragraph-length.
            ...(wrap
              ? { whiteSpace: 'normal', maxWidth: 320, wordBreak: 'break-word' }
              : { whiteSpace: 'nowrap' }),
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
