// src/components/UserBadges.jsx
//
// Renders user badges before the username in chat rows. Filters by the
// `is_mod` flag stamped server-side so cosmetic vs mod-authority badges
// can be toggled independently in Preferences.

import Tooltip from './Tooltip.jsx';

export default function UserBadges({ badges, showCosmetic, showMod, size = 14 }) {
  const filtered = (badges ?? []).filter(
    (b) => (b.is_mod ? showMod : showCosmetic) && b.url,
  );
  if (filtered.length === 0) return null;
  return (
    <span
      style={{
        display: 'inline-flex',
        gap: 2,
        marginRight: 4,
        verticalAlign: 'middle',
      }}
    >
      {filtered.map((b) => (
        <Tooltip key={`${b.id}-${b.url}`} placement="top" text={b.title || b.id}>
          <img
            src={b.url}
            alt=""
            width={size}
            height={size}
            style={{ display: 'block', flexShrink: 0 }}
          />
        </Tooltip>
      ))}
    </span>
  );
}
