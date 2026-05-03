import { openUrl } from '../ipc.js';
import Tooltip from './Tooltip.jsx';

/**
 * Render chat text with emote and link byte-ranges substituted for <img> /
 * <a> elements. Ranges are byte offsets (as emitted by the Rust IRC parser).
 * We slice the UTF-8 string with a TextEncoder to preserve character
 * boundaries when a range happens to sit next to a multi-byte unicode
 * codepoint.
 */
export default function EmoteText({ text, ranges, links, size = 20 }) {
  if (!text) return null;
  const emoteRanges = ranges ?? [];
  const linkRanges = links ?? [];
  if (emoteRanges.length === 0 && linkRanges.length === 0) {
    return <span>{text}</span>;
  }

  // Merge both arrays into one sorted list. Emotes win on overlap (link scan
  // already overlap-skips emotes server-side; defensive client-side too).
  const all = [
    ...emoteRanges.map((r) => ({ kind: 'emote', range: r })),
    ...linkRanges.map((r) => ({ kind: 'link', range: r })),
  ].sort((a, b) => a.range.start - b.range.start);

  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  const segments = [];
  let cursor = 0;

  const pushText = (s, e) => {
    if (e > s) segments.push({ type: 'text', text: decoder.decode(bytes.slice(s, e)) });
  };

  for (const item of all) {
    const { kind, range } = item;
    if (range.start < cursor) continue; // overlapping range; skip
    pushText(cursor, range.start);
    if (kind === 'emote') {
      segments.push({ type: 'emote', range });
    } else {
      const display = decoder.decode(bytes.slice(range.start, range.end));
      segments.push({ type: 'link', range, display });
    }
    cursor = range.end;
  }
  pushText(cursor, bytes.length);

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
