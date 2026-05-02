import { useEffect, useLayoutEffect, useRef, useState } from 'react';

/**
 * Spellcheck overlay — renders red squiggles on misspelled words by
 * sitting on top of an `<input type="text">` with transparent text.
 *
 * Why an overlay instead of contenteditable: the existing Composer's
 * autocomplete (emote/mention popup), keyboard handling, and caret
 * tracking all depend on `<input>` semantics. Replacing the input with
 * contenteditable would require reimplementing all of that. The overlay
 * pattern is the standard "highlight while typing" approach used by
 * Slack, Linear, etc.
 *
 * Why `text-decoration` survives `color: transparent`: text decorations
 * (underline, line-through) are styled independently of `color` per the
 * CSS spec. So the overlay's spans render as transparent text WITH
 * visible red wavy underlines beneath the baseline.
 *
 * Props:
 *   inputRef       React ref to the underlying <input> — used for size + scroll sync
 *   text           current input value
 *   misspellings   Array<{ start, end, word }> from useSpellcheck
 */
export default function SpellcheckOverlay({ inputRef, text, misspellings }) {
  const overlayRef = useRef(null);
  const [style, setStyle] = useState(null);
  const [scrollLeft, setScrollLeft] = useState(0);

  // Copy font, padding, line-height etc. from the input. useLayoutEffect
  // so the overlay paints synchronously aligned (no flash of misalignment).
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const cs = getComputedStyle(input);
    setStyle({
      fontFamily: cs.fontFamily,
      fontSize: cs.fontSize,
      fontWeight: cs.fontWeight,
      lineHeight: cs.lineHeight,
      letterSpacing: cs.letterSpacing,
      paddingTop: cs.paddingTop,
      paddingRight: cs.paddingRight,
      paddingBottom: cs.paddingBottom,
      paddingLeft: cs.paddingLeft,
      borderTopWidth: cs.borderTopWidth,
      borderLeftWidth: cs.borderLeftWidth,
    });
  }, [inputRef, text]);

  // Re-copy on input resize (system fonts settle late, input flexes).
  useEffect(() => {
    const input = inputRef.current;
    if (!input || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      const cs = getComputedStyle(input);
      setStyle((prev) => ({
        ...(prev ?? {}),
        fontFamily: cs.fontFamily,
        fontSize: cs.fontSize,
        fontWeight: cs.fontWeight,
        lineHeight: cs.lineHeight,
        letterSpacing: cs.letterSpacing,
        paddingTop: cs.paddingTop,
        paddingRight: cs.paddingRight,
        paddingBottom: cs.paddingBottom,
        paddingLeft: cs.paddingLeft,
        borderTopWidth: cs.borderTopWidth,
        borderLeftWidth: cs.borderLeftWidth,
      }));
    });
    ro.observe(input);
    return () => ro.disconnect();
  }, [inputRef]);

  // Mirror the input's scrollLeft so underlines under off-screen text
  // shift with the text.
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const onScroll = () => setScrollLeft(input.scrollLeft);
    input.addEventListener('scroll', onScroll);
    setScrollLeft(input.scrollLeft);
    return () => input.removeEventListener('scroll', onScroll);
  }, [inputRef, text]);

  if (!style) return null;

  const segments = buildSegments(text, misspellings);

  return (
    <div
      ref={overlayRef}
      aria-hidden="true"
      style={{
        position: 'absolute',
        top: style.borderTopWidth,
        left: style.borderLeftWidth,
        right: 0,
        bottom: 0,
        pointerEvents: 'none',
        overflow: 'hidden',
        paddingTop: style.paddingTop,
        paddingRight: style.paddingRight,
        paddingBottom: style.paddingBottom,
        paddingLeft: style.paddingLeft,
        fontFamily: style.fontFamily,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        lineHeight: style.lineHeight,
        letterSpacing: style.letterSpacing,
        color: 'transparent',
        whiteSpace: 'pre',
        transform: `translateX(-${scrollLeft}px)`,
      }}
    >
      {segments.map((seg, i) =>
        seg.kind === 'plain' ? (
          <span key={i}>{seg.text}</span>
        ) : (
          <span key={i} className="spellcheck-misspelled" data-word={seg.word}>
            {seg.text}
          </span>
        ),
      )}
    </div>
  );
}

/**
 * Slice `text` into alternating plain / misspelled segments based on
 * `ranges`. Out-of-bounds or overlapping ranges are tolerated — last
 * one wins per byte.
 */
function buildSegments(text, ranges) {
  if (!ranges || ranges.length === 0) {
    return [{ kind: 'plain', text }];
  }
  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const out = [];
  let cursor = 0;
  for (const r of sorted) {
    const start = Math.max(0, Math.min(r.start, text.length));
    const end = Math.max(start, Math.min(r.end, text.length));
    if (start > cursor) {
      out.push({ kind: 'plain', text: text.slice(cursor, start) });
    }
    if (end > start) {
      out.push({ kind: 'misspelled', text: text.slice(start, end), word: r.word });
    }
    cursor = end;
  }
  if (cursor < text.length) {
    out.push({ kind: 'plain', text: text.slice(cursor) });
  }
  return out;
}
