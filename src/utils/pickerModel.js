export function buildPickerModel(emotes, { query = '', filter = 'all' } = {}) {
  const q = query.trim().toLowerCase();
  const match = (e) =>
    (!q || e.name.toLowerCase().includes(q)) &&
    (filter === 'all' || (filter === 'animated' ? e.animated : !e.animated));
  const sections = [
    { title: 'Channel', pred: (e) => e.origin === 'channel' },
    { title: 'Twitch', pred: (e) => e.origin === 'user' && e.provider === 'twitch' },
    { title: '7TV', pred: (e) => e.origin === 'global' && e.provider === '7tv' },
    { title: 'BTTV', pred: (e) => e.origin === 'global' && e.provider === 'bttv' },
    { title: 'FFZ', pred: (e) => e.origin === 'global' && e.provider === 'ffz' },
    { title: 'Kick', pred: (e) => e.provider === 'kick' },
  ];
  const used = new Set();
  return sections
    .map(({ title, pred }) => ({
      title,
      emotes: emotes.filter((e) => {
        if (used.has(e.name) || !pred(e) || !match(e)) return false;
        used.add(e.name);
        return true;
      }),
    }))
    .filter((s) => s.emotes.length > 0);
}

if (import.meta.env.DEV) {
  const mk = (name, provider, origin, animated = false, locked = false) =>
    ({ name, url_1x: 'u', animated, provider, origin, locked });
  const data = [
    mk('chanA', 'twitch', 'channel', false, true),
    mk('mine', 'twitch', 'user'),
    mk('Glob7', '7tv', 'global', true),
    mk('globB', 'bttv', 'global'),
  ];
  const all = buildPickerModel(data, {});
  console.assert(all.length === 4 && all[0].title === 'Channel', 'sections ordered, channel first');
  console.assert(buildPickerModel(data, { filter: 'animated' }).length === 1, 'animated filter');
  console.assert(
    buildPickerModel(data, { query: 'glob' }).flatMap((s) => s.emotes).length === 2,
    'case-insensitive substring search'
  );
  console.assert(
    buildPickerModel(data, {}).flatMap((s) => s.emotes).some((e) => e.locked),
    'locked emotes included'
  );
}
