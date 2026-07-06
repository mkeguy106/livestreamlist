import { useMemo } from 'react';
import { openUrl } from '../ipc.js';
import Tooltip from './Tooltip.jsx';

// Hoisted to module scope — a fresh TextEncoder/TextDecoder per render was
// pure allocation churn on every chat row (several times a second on busy
// channels). They are stateless and safe to share across all callers.
const ENCODER = new TextEncoder();
const DECODER = new TextDecoder();

// Shared empty array so `ranges ?? EMPTY` yields a STABLE reference when the
// payload omits emote/link ranges — otherwise `?? []` would mint a new array
// every render and defeat the useMemo dependency check below.
const EMPTY = [];

/**
 * Pure byte-range → segment segmentation. Extracted so it can be memoized and
 * DEV-asserted. Ranges are byte offsets (as emitted by the Rust IRC parser);
 * we slice the UTF-8 string with a TextEncoder to preserve character
 * boundaries when a range sits next to a multi-byte unicode codepoint.
 */
function computeSegments(text, emoteRanges, linkRanges) {
  // Merge both arrays into one sorted list. Emotes win on overlap (link scan
  // already overlap-skips emotes server-side; defensive client-side too).
  const all = [
    ...emoteRanges.map((r) => ({ kind: 'emote', range: r })),
    ...linkRanges.map((r) => ({ kind: 'link', range: r })),
  ].sort((a, b) => a.range.start - b.range.start);

  const bytes = ENCODER.encode(text);
  const segments = [];
  let cursor = 0;

  const pushText = (s, e) => {
    if (e > s) segments.push({ type: 'text', text: DECODER.decode(bytes.slice(s, e)) });
  };

  for (const item of all) {
    const { kind, range } = item;
    if (range.start < cursor) continue; // overlapping range; skip
    pushText(cursor, range.start);
    if (kind === 'emote') {
      segments.push({ type: 'emote', range });
    } else {
      const display = DECODER.decode(bytes.slice(range.start, range.end));
      segments.push({ type: 'link', range, display });
    }
    cursor = range.end;
  }
  pushText(cursor, bytes.length);
  return segments;
}

/**
 * Render chat text with emote and link byte-ranges substituted for <img> /
 * <a> elements.
 */
export default function EmoteText({ text, ranges, links, size = 20 }) {
  const emoteRanges = ranges ?? EMPTY;
  const linkRanges = links ?? EMPTY;
  // Memoize on the message's own (stable-per-message) props. The row wrapping
  // this is React.memo'd, so EmoteText only re-renders when its message
  // changes — but the memo still saves recompute when the row re-renders for
  // an unrelated reason (e.g. nickname map change) with the same text/ranges.
  const segments = useMemo(
    () => (text && (emoteRanges.length || linkRanges.length)
      ? computeSegments(text, emoteRanges, linkRanges)
      : null),
    [text, emoteRanges, linkRanges],
  );

  if (!text) return null;
  if (segments === null) {
    return <span>{text}</span>;
  }

  return (
    <span style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
      {segments.map((seg, i) => {
        if (seg.type === 'text') {
          return <span key={i}>{seg.text}</span>;
        }
        if (seg.type === 'emote') {
          return (
            <Tooltip
              key={i}
              placement="top"
              text={seg.range.name}
              wrapperStyle={{
                verticalAlign: -Math.round(size * 0.25),
                margin: '0 1px',
              }}
            >
              <img
                src={seg.range.url_1x}
                srcSet={
                  seg.range.url_2x
                    ? `${seg.range.url_1x} 1x, ${seg.range.url_2x} 2x${seg.range.url_4x ? `, ${seg.range.url_4x} 4x` : ''}`
                    : undefined
                }
                alt={seg.range.name}
                loading="lazy"
                style={{
                  height: size,
                  width: 'auto',
                }}
              />
            </Tooltip>
          );
        }
        // type === 'link'
        return (
          <a
            key={i}
            href={seg.range.url}
            className="rx-chat-link"
            onClick={(e) => {
              e.preventDefault();
              openUrl(seg.range.url);
            }}
          >
            {seg.display}
          </a>
        );
      })}
    </span>
  );
}

// ── Module-scope DEV asserts (run once on import in dev). ──────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // Plain text with no ranges → single text segment.
  {
    const segs = computeSegments('hello', [], []);
    console.assert(
      segs.length === 1 && segs[0].type === 'text' && segs[0].text === 'hello',
      'computeSegments plain text',
    );
  }
  // A single emote spanning the whole word.
  {
    const segs = computeSegments('Kappa', [{ start: 0, end: 5, name: 'Kappa' }], []);
    console.assert(
      segs.length === 1 && segs[0].type === 'emote',
      'computeSegments single emote',
    );
  }
  // Text + emote + text around a mid-string emote (byte offsets).
  {
    const segs = computeSegments('a Kappa b', [{ start: 2, end: 7, name: 'Kappa' }], []);
    console.assert(
      segs.length === 3 &&
        segs[0].type === 'text' && segs[0].text === 'a ' &&
        segs[1].type === 'emote' &&
        segs[2].type === 'text' && segs[2].text === ' b',
      'computeSegments text/emote/text split',
    );
  }
  // Multi-byte codepoint before a link range — byte slicing must preserve it.
  {
    // "é " is 3 bytes (0xC3 0xA9 0x20); the 8-byte link occupies bytes 3..11.
    const segs = computeSegments('é http://x', [], [{ start: 3, end: 11, url: 'http://x' }]);
    console.assert(
      segs.length === 2 &&
        segs[0].type === 'text' && segs[0].text === 'é ' &&
        segs[1].type === 'link' && segs[1].display === 'http://x',
      'computeSegments multi-byte boundary + link',
    );
  }
}
