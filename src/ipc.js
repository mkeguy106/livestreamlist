// Thin wrappers over Tauri's invoke() with a browser-dev fallback so the UI
// can be iterated without spinning up the full app shell.

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

let _invoke = null;
async function invoke(name, args) {
  if (!inTauri) return mockInvoke(name, args);
  if (!_invoke) {
    const mod = await import('@tauri-apps/api/core');
    _invoke = mod.invoke;
  }
  return _invoke(name, args);
}

export const listLivestreams = () => invoke('list_livestreams');
export const listChannels = () => invoke('list_channels');
export const addChannelFromInput = (input) => invoke('add_channel_from_input', { input });
export const removeChannel = (uniqueKey) => invoke('remove_channel', { uniqueKey });
export const setFavorite = (uniqueKey, favorite) => invoke('set_favorite', { uniqueKey, favorite });
export const refreshAll = () => invoke('refresh_all');
export const launchStream = (uniqueKey, quality) => invoke('launch_stream', { uniqueKey, quality });
export const openInBrowser = (uniqueKey) => invoke('open_in_browser', { uniqueKey });
export const chatConnect = (uniqueKey) => invoke('chat_connect', { uniqueKey });
export const chatDisconnect = (uniqueKey) => invoke('chat_disconnect', { uniqueKey });
export const chatSend = (uniqueKey, text) => invoke('chat_send', { uniqueKey, text });
export const chatOpenPopout = (uniqueKey) => invoke('chat_open_popout', { uniqueKey });
export const listEmotes = (uniqueKey) => invoke('list_emotes', { uniqueKey });
export const replayChatHistory = (uniqueKey, limit = 100) =>
  invoke('replay_chat_history', { uniqueKey, limit });
export const authStatus = () => invoke('auth_status');
export const twitchLogin = () => invoke('twitch_login');
export const twitchLogout = () => invoke('twitch_logout');
export const kickLogin = () => invoke('kick_login');
export const kickLogout = () => invoke('kick_logout');
export const openUrl = (url) => invoke('open_url', { url });
export const listSocials = (uniqueKey) => invoke('list_socials', { uniqueKey });

/**
 * Subscribe to a Tauri-side event. Returns an unlisten function.
 * In browser-dev mode this routes through our in-memory mock event bus so the
 * chat UI still feels alive without the Tauri runtime.
 */
export async function listenEvent(name, handler) {
  if (!inTauri) return mockListen(name, handler);
  const mod = await import('@tauri-apps/api/event');
  return mod.listen(name, (e) => handler(e.payload));
}

// ── Browser-dev mock data ─────────────────────────────────────────────────
const MOCK_CHANNELS = [
  { platform: 'twitch', channel_id: 'shroud',    display_name: 'shroud',    favorite: true,  dont_notify: false, auto_play: false },
  { platform: 'twitch', channel_id: 'xqc',       display_name: 'xQc',       favorite: false, dont_notify: false, auto_play: false },
  { platform: 'twitch', channel_id: 'hasanabi',  display_name: 'HasanAbi',  favorite: false, dont_notify: false, auto_play: false },
  { platform: 'twitch', channel_id: 'pokimane',  display_name: 'pokimane',  favorite: false, dont_notify: false, auto_play: false },
  { platform: 'youtube',channel_id: 'LudwigAhgren', display_name: 'Ludwig', favorite: false, dont_notify: false, auto_play: false },
  { platform: 'twitch', channel_id: 'asmongold', display_name: 'Asmongold', favorite: false, dont_notify: false, auto_play: false },
  { platform: 'kick',   channel_id: 'trainwreckstv', display_name: 'Trainwrex', favorite: false, dont_notify: false, auto_play: false },
  { platform: 'twitch', channel_id: 'mizkif',    display_name: 'Mizkif',    favorite: false, dont_notify: false, auto_play: false },
];

const MOCK_LIVE = {
  'twitch:shroud':    { title: 'ranked grind to radiant', game: 'VALORANT',       viewers: 47204 },
  'twitch:xqc':       { title: 'reacting to everything',  game: 'Just Chatting',  viewers: 82112 },
  'twitch:hasanabi':  { title: 'late night pol takes',    game: 'Just Chatting',  viewers: 29421 },
  'youtube:LudwigAhgren': { title: 'pogo tournament round 3', game: 'Chess',      viewers: 11803 },
  'twitch:asmongold': { title: 'WoW classic',             game: 'World of Warcraft', viewers: 34612 },
  'kick:trainwreckstv': { title: 'max bets only',         game: 'Slots',          viewers: 8912  },
  'twitch:mizkif':    { title: 'osrs grind',              game: 'OSRS',           viewers: 6143  },
};

let mockChannels = [...MOCK_CHANNELS];
let mockAuth = { twitch: null, kick: null };

function mockSnapshot() {
  const nowIso = new Date().toISOString();
  const startedIso = new Date(Date.now() - 2 * 3600_000 - 14 * 60_000).toISOString();
  return mockChannels.map((c) => {
    const key = `${c.platform}:${c.channel_id}`;
    const live = MOCK_LIVE[key];
    return {
      unique_key: key,
      platform: c.platform,
      channel_id: c.channel_id,
      display_name: c.display_name,
      is_live: Boolean(live),
      title: live?.title ?? null,
      game: live?.game ?? null,
      game_slug: null,
      viewers: live?.viewers ?? null,
      started_at: live ? startedIso : null,
      thumbnail_url: null,
      profile_image_url: null,
      last_checked: nowIso,
      error: null,
    };
  });
}

const mockSubscribers = new Map(); // name -> Set<handler>
const mockChatTimers = new Map();  // channelKey -> intervalId

function mockListen(name, handler) {
  const set = mockSubscribers.get(name) ?? new Set();
  set.add(handler);
  mockSubscribers.set(name, set);
  return () => {
    const s = mockSubscribers.get(name);
    if (s) s.delete(handler);
  };
}

function mockEmit(name, payload) {
  const s = mockSubscribers.get(name);
  if (s) for (const h of s) h(payload);
}

const MOCK_CHAT_TEMPLATES = [
  { u: 'vanishh',    c: '#a78bfa', m: 'Kappa welcome back btw PogChamp' },
  { u: 'mikael_ek',  c: '#f87171', m: 'the clutch was so clean' },
  { u: 'kyra.',      c: '#60a5fa', m: 'skillgap EZ' },
  { u: 'tomjones',   c: '#4ade80', m: 'O7' },
  { u: 'marbled',    c: '#a78bfa', m: 'when the aim be aiming Kreygasm' },
  { u: 'paulieboy',  c: '#fb923c', m: 'thats a W LUL' },
  { u: 'reyna.main', c: '#f472b6', m: 'he cooked' },
  { u: 'ilikepie',   c: '#84cc16', m: 'PogChamp PogChamp PogChamp' },
  { u: 'dontban',    c: '#a78bfa', m: '@shroud how do you move so fast' },
  { u: 'moonbeam',   c: '#60a5fa', m: '🎯 insane' },
];

function startMockChat(uniqueKey) {
  if (mockChatTimers.has(uniqueKey)) return;
  const id = setInterval(() => {
    const tpl = MOCK_CHAT_TEMPLATES[Math.floor(Math.random() * MOCK_CHAT_TEMPLATES.length)];
    mockEmit(`chat:message:${uniqueKey}`, {
      id: `mock-${Date.now()}-${Math.random().toString(16).slice(2, 6)}`,
      channel_key: uniqueKey,
      platform: uniqueKey.split(':')[0],
      timestamp: new Date().toISOString(),
      user: { login: tpl.u, display_name: tpl.u, color: tpl.c },
      text: tpl.m,
      emote_ranges: [],
      badges: [],
      is_action: false,
    });
  }, 900 + Math.random() * 1400);
  mockChatTimers.set(uniqueKey, id);
  mockEmit(`chat:status:${uniqueKey}`, { channel_key: uniqueKey, status: 'connected' });
}

function stopMockChat(uniqueKey) {
  const id = mockChatTimers.get(uniqueKey);
  if (id) clearInterval(id);
  mockChatTimers.delete(uniqueKey);
  mockEmit(`chat:status:${uniqueKey}`, { channel_key: uniqueKey, status: 'closed' });
}

async function mockInvoke(name, args) {
  switch (name) {
    case 'list_livestreams':
    case 'refresh_all':
      return mockSnapshot();
    case 'chat_connect':
      startMockChat(args.uniqueKey);
      return null;
    case 'chat_disconnect':
      stopMockChat(args.uniqueKey);
      return null;
    case 'chat_open_popout':
      window.open('https://example.com', '_blank', 'noopener');
      return null;
    case 'replay_chat_history':
      return [];
    case 'list_socials':
      // Stub sample so the UI still shows something in browser-dev mode.
      return [
        { id: 'twitter', name: 'twitter', title: 'Twitter',  url: 'https://twitter.com/' },
        { id: 'discord', name: 'discord', title: 'Discord',  url: 'https://discord.com/' },
      ];
    case 'list_emotes':
      return [
        { name: 'Kappa',    url_1x: 'https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0', url_2x: null, url_4x: null, animated: false },
        { name: 'PogChamp', url_1x: 'https://static-cdn.jtvnw.net/emoticons/v2/305954156/default/dark/1.0', url_2x: null, url_4x: null, animated: false },
        { name: 'LUL',      url_1x: 'https://static-cdn.jtvnw.net/emoticons/v2/425618/default/dark/1.0', url_2x: null, url_4x: null, animated: false },
        { name: 'Kreygasm', url_1x: 'https://static-cdn.jtvnw.net/emoticons/v2/41/default/dark/1.0', url_2x: null, url_4x: null, animated: false },
        { name: 'peepoClap',url_1x: '', url_2x: null, url_4x: null, animated: false },
      ];
    case 'auth_status':
      return mockAuth;
    case 'twitch_login':
      mockAuth = { ...mockAuth, twitch: { login: 'mock_user', user_id: '0', scopes: ['chat:edit'] } };
      return mockAuth.twitch;
    case 'twitch_logout':
      mockAuth = { ...mockAuth, twitch: null };
      return null;
    case 'kick_login':
      mockAuth = { ...mockAuth, kick: { login: 'mock_kick', user_id: '0' } };
      return mockAuth.kick;
    case 'kick_logout':
      mockAuth = { ...mockAuth, kick: null };
      return null;
    case 'chat_send':
      mockEmit(`chat:message:${args.uniqueKey}`, {
        id: `self-${Date.now()}`,
        channel_key: args.uniqueKey,
        platform: args.uniqueKey.split(':')[0],
        timestamp: new Date().toISOString(),
        user: { login: 'you', display_name: 'you', color: '#f4f4f5' },
        text: args.text,
        emote_ranges: [],
        badges: [],
        is_action: false,
      });
      return null;
    case 'list_channels':
      return mockChannels;
    case 'add_channel_from_input': {
      const parsed = parseMockInput(args.input);
      if (!parsed) throw new Error(`couldn't recognise '${args.input}' as a channel URL`);
      const key = `${parsed.platform}:${parsed.channel_id}`;
      if (mockChannels.some((c) => `${c.platform}:${c.channel_id}` === key)) {
        throw new Error(`${key} is already in the list`);
      }
      const ch = { ...parsed, favorite: false, dont_notify: false, auto_play: false };
      mockChannels = [...mockChannels, ch];
      return ch;
    }
    case 'remove_channel':
      mockChannels = mockChannels.filter((c) => `${c.platform}:${c.channel_id}` !== args.uniqueKey);
      return true;
    case 'set_favorite':
      mockChannels = mockChannels.map((c) =>
        `${c.platform}:${c.channel_id}` === args.uniqueKey ? { ...c, favorite: args.favorite } : c,
      );
      return true;
    case 'launch_stream':
      console.warn('[mock] launch_stream', args);
      return 0;
    case 'open_in_browser': {
      const c = mockChannels.find((c) => `${c.platform}:${c.channel_id}` === args.uniqueKey);
      if (c) {
        const url = `https://twitch.tv/${c.channel_id}`;
        window.open(url, '_blank', 'noopener');
      }
      return null;
    }
    case 'open_url':
      window.open(args.url, '_blank', 'noopener');
      return null;
    default:
      throw new Error(`[mock] unknown invoke ${name}`);
  }
}

function parseMockInput(input) {
  const t = (input ?? '').trim();
  if (!t) return null;
  // A tiny subset of the Rust parser, enough to exercise the dialog in browser dev.
  try {
    const u = new URL(t.startsWith('http') ? t : `https://${t}`);
    const host = u.hostname.toLowerCase();
    const seg = u.pathname.split('/').filter(Boolean);
    if (host.includes('twitch.tv') && seg[0]) return { platform: 'twitch', channel_id: seg[0], display_name: seg[0] };
    if (host.includes('youtube.com') && seg[0]) {
      const id = seg[0] === 'channel' && seg[1] ? seg[1] : seg[0].replace(/^@/, '');
      return { platform: 'youtube', channel_id: id, display_name: id };
    }
    if (host.includes('kick.com') && seg[0]) return { platform: 'kick', channel_id: seg[0], display_name: seg[0] };
    if (host.includes('chaturbate.com') && seg[0]) return { platform: 'chaturbate', channel_id: seg[0], display_name: seg[0] };
  } catch {}
  if (/^[a-z0-9_-]+$/i.test(t.replace(/^@/, ''))) {
    const id = t.replace(/^@/, '');
    return { platform: 'twitch', channel_id: id, display_name: id };
  }
  return null;
}
