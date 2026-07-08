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
export default function SpellcheckOverlay({ inputRef, text, misspellings, recentCorrections }) {
  const overlayRef = useRef(null);
  const [style, setStyle] = useState(null);
  const [scroll, setScroll] = useState({ top: 0, left: 0 });

  // Copy font, padding, border, and wrapping metrics from the input. The
  // input is now a wrapping <textarea>, so the overlay must wrap identically —
  // `whiteSpace`, `overflowWrap`, and `wordBreak` are copied alongside the
  // box metrics so the transparent mirror text breaks lines in exactly the
  // same places, keeping every squiggle / green-pill aligned across wraps.
  // useLayoutEffect so the overlay paints synchronously aligned.
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    setStyle(readMetrics(input));
  }, [inputRef, text]);

  // Re-copy on input resize (system fonts settle late; the composer flexes to
  // fill the row, and columns resize).
  useEffect(() => {
    const input = inputRef.current;
    if (!input || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      setStyle((prev) => ({ ...(prev ?? {}), ...readMetrics(input) }));
    });
    ro.observe(input);
    return () => ro.disconnect();
  }, [inputRef]);

  // Mirror the input's scroll so squiggles under off-screen text shift with
  // it. A wrapping textarea scrolls VERTICALLY (never horizontally), so
  // translateY is what matters here; translateX is kept at 0 for safety.
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const onScroll = () => setScroll({ top: input.scrollTop, left: input.scrollLeft });
    input.addEventListener('scroll', onScroll);
    setScroll({ top: input.scrollTop, left: input.scrollLeft });
    return () => input.removeEventListener('scroll', onScroll);
  }, [inputRef, text]);

  if (!style) return null;

  const segments = buildSegments(text, misspellings, recentCorrections);

  return (
    <div
      ref={overlayRef}
      aria-hidden="true"
      style={{
        // Fill the wrapper exactly. The wrapper is sized by the in-flow
        // textarea (width: 100%, auto-grown height), so `inset: 0` +
        // box-sizing: border-box + a transparent border matching the
        // textarea's makes the overlay's *content box* line up pixel-for-pixel
        // with the textarea's — identical text origin AND identical wrap
        // width. Its getBoundingClientRect therefore equals the textarea's.
        position: 'absolute',
        inset: 0,
        boxSizing: 'border-box',
        pointerEvents: 'none',
        overflow: 'hidden',
        borderStyle: 'solid',
        borderColor: 'transparent',
        borderTopWidth: style.borderTopWidth,
        borderRightWidth: style.borderRightWidth,
        borderBottomWidth: style.borderBottomWidth,
        borderLeftWidth: style.borderLeftWidth,
        paddingTop: style.paddingTop,
        paddingRight: style.paddingRight,
        paddingBottom: style.paddingBottom,
        paddingLeft: style.paddingLeft,
        fontFamily: style.fontFamily,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        lineHeight: style.lineHeight,
        letterSpacing: style.letterSpacing,
        whiteSpace: style.whiteSpace,
        wordBreak: style.wordBreak,
        overflowWrap: style.overflowWrap,
        color: 'transparent',
        transform: `translate(-${scroll.left}px, -${scroll.top}px)`,
      }}
    >
      {segments.map((seg, i) => {
        if (seg.kind === 'plain') {
          return <span key={i}>{seg.text}</span>;
        }
        if (seg.kind === 'corrected') {
          // Use a key that incorporates `originalWord` + position, so
          // when a word fades and a new correction lands at a similar
          // position, React doesn't accidentally re-use the DOM node
          // (which would inherit the in-progress animation timer).
          return (
            <span
              key={`c:${seg.start}:${seg.originalWord}`}
              className="spellcheck-corrected"
              data-word={seg.word}
              data-original={seg.originalWord}
            >
              {seg.text}
            </span>
          );
        }
        return (
          <span key={i} className="spellcheck-misspelled" data-word={seg.word}>
            {seg.text}
          </span>
        );
      })}
    </div>
  );
}

/**
 * Slice `text` into alternating plain / misspelled / corrected segments.
 *
 * Precedence: corrected > misspelled (a recently-corrected word that
 * hunspell would still flag — perhaps the user typed a non-dict word
 * that got autocorrected to another non-dict word — should show the
 * green pill, not red squiggle, until the pill fades).
 *
 * Out-of-bounds or overlapping ranges of the SAME kind are tolerated;
 * cross-kind overlap resolves per the precedence above.
 */
function buildSegments(text, misspellings, recentCorrections) {
  const corrected = [];
  if (recentCorrections) {
    for (const c of recentCorrections.values()) {
      corrected.push({ ...c, kind: 'corrected' });
    }
  }
  const flagged = (misspellings ?? []).map((m) => ({ ...m, kind: 'misspelled' }));

  // Filter out misspelled ranges that overlap a corrected range.
  const survivors = flagged.filter((m) =>
    !corrected.some((c) => rangesOverlap(m, c)),
  );

  const all = [...corrected, ...survivors].sort((a, b) => a.start - b.start);

  if (all.length === 0) {
    return [{ kind: 'plain', text }];
  }

  const out = [];
  let cursor = 0;
  for (const r of all) {
    const start = Math.max(0, Math.min(r.start, text.length));
    const end = Math.max(start, Math.min(r.end, text.length));
    if (start > cursor) {
      out.push({ kind: 'plain', text: text.slice(cursor, start) });
    }
    if (end > start) {
      out.push({
        kind: r.kind,
        text: text.slice(start, end),
        word: r.word,
        // corrected ranges carry the original (pre-correction) word
        // for the green pill's data-original attribute.
        originalWord: r.originalWord,
        start,
      });
    }
    cursor = end;
  }
  if (cursor < text.length) {
    out.push({ kind: 'plain', text: text.slice(cursor) });
  }
  return out;
}

function rangesOverlap(a, b) {
  return a.start < b.end && b.start < a.end;
}

// Copy the box + wrapping metrics the overlay needs to mirror the textarea.
// Kept in one place so the initial useLayoutEffect and the ResizeObserver
// re-copy stay identical.
function readMetrics(input) {
  const cs = getComputedStyle(input);
  return {
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
    borderRightWidth: cs.borderRightWidth,
    borderBottomWidth: cs.borderBottomWidth,
    borderLeftWidth: cs.borderLeftWidth,
    // Wrapping metrics — the textarea wraps, so the overlay must too.
    whiteSpace: cs.whiteSpace,
    wordBreak: cs.wordBreak,
    overflowWrap: cs.overflowWrap,
  };
}
