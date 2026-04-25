/**
 * Render chat text with emote byte-ranges substituted for <img> elements.
 * Ranges are byte offsets (as emitted by the Rust IRC parser). We slice the
 * UTF-8 string with a TextEncoder to preserve character boundaries when an
 * emote name happens to sit next to a multi-byte unicode codepoint.
 */
export default function EmoteText({ text, ranges, size = 20 }) {
  if (!text) return null;
  if (!ranges || ranges.length === 0) {
    return <span>{text}</span>;
  }

  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const bytes = new TextEncoder().encode(text);
  const decoder = new TextDecoder();
  const segments = [];
  let cursor = 0;

  const pushText = (s, e) => {
    if (e > s) segments.push({ type: 'text', text: decoder.decode(bytes.slice(s, e)) });
  };

  for (const r of sorted) {
    if (r.start < cursor) continue; // overlapping range; skip
    pushText(cursor, r.start);
    segments.push({ type: 'emote', range: r });
    cursor = r.end;
  }
  pushText(cursor, bytes.length);

  return (
    <span style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
      {segments.map((seg, i) =>
        seg.type === 'text' ? (
          <span key={i}>{seg.text}</span>
        ) : (
          <img
            key={i}
            src={seg.range.url_1x}
            srcSet={
              seg.range.url_2x
                ? `${seg.range.url_1x} 1x, ${seg.range.url_2x} 2x${seg.range.url_4x ? `, ${seg.range.url_4x} 4x` : ''}`
                : undefined
            }
            alt={seg.range.name}
            title={seg.range.name}
            loading="lazy"
            style={{
              height: size,
              width: 'auto',
              verticalAlign: -Math.round(size * 0.25),
              margin: '0 1px',
            }}
          />
        ),
      )}
    </span>
  );
}
