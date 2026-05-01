/* Direction A — "Command"
 * Sidebar rail (all channels) + main pane showing the selected channel.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import ChatView from '../components/ChatView.jsx';
import ContextMenu from '../components/ContextMenu.jsx';
import SocialsBanner from '../components/SocialsBanner.jsx';
import TabStrip from '../components/TabStrip.jsx';
import TitleBanner from '../components/TitleBanner.jsx';
import Tooltip from '../components/Tooltip.jsx';
import { useCommandTabs } from '../hooks/useCommandTabs.js';
import { usePlayerState } from '../hooks/usePlayerState.js';
import { stopStream } from '../ipc.js';
import { formatUptime, formatViewers } from '../utils/format.js';

const FILTER_OPTS = [
  { k: 'all',       l: 'All channels' },
  { k: 'twitch',    l: 'Twitch',   icon: 't' },
  { k: 'kick',      l: 'Kick',     icon: 'k' },
  { k: 'youtube',   l: 'YouTube',  icon: 'y' },
  { k: 'favorites', l: 'Favorites' },
];
const SORT_OPTS = [
  { k: 'viewers',  l: 'Viewers'   },
  { k: 'name',     l: 'Name'      },
  { k: 'playing',  l: 'Playing'   },
  { k: 'lastseen', l: 'Last seen' },
  { k: 'timelive', l: 'Time live' },
];

const STORAGE_KEYS = {
  filter:      'livestreamlist.sidebar.filter',
  sort:        'livestreamlist.sidebar.sort',
  hideOffline: 'livestreamlist.sidebar.hideOffline',
};

function loadPref(key, fallback) {
  try {
    const v = localStorage.getItem(key);
    if (v == null) return fallback;
    if (v === 'true') return true;
    if (v === 'false') return false;
    return v;
  } catch {
    return fallback;
  }
}
function savePref(key, value) {
  try { localStorage.setItem(key, String(value)); } catch {}
}

export default function Command({ ctx }) {
  const {
    livestreams,
    loading,
    refresh,
    openAddDialog,
    launchStream,
    openInBrowser,
    removeChannel,
    setFavorite,
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  } = ctx;

  // Tab state — owned by Command. Focus and Columns continue to consume
  // ctx.selectedKey unchanged.
  const {
    tabKeys,
    detachedKeys,
    activeTabKey,
    mentions,
    closeTab,
    reorderTabs,
    setActiveTabKey,
    detachTab,
    rowClickHandler,
    notifyMention,
  } = useCommandTabs({ livestreams });

  const playing = usePlayerState();
  const [menu, setMenu] = useState(null); // { x, y, channel }

  const [filter, setFilter] = useState(() => loadPref(STORAGE_KEYS.filter, 'all'));
  const [sort, setSort] = useState(() => loadPref(STORAGE_KEYS.sort, 'viewers'));
  const [hideOffline, setHideOffline] = useState(() => loadPref(STORAGE_KEYS.hideOffline, false));
  const [openMenu, setOpenMenu] = useState(null); // 'filter' | 'sort' | null
  // Channel-list search. Ephemeral by design — restoring a stale query
  // on relaunch would surprise the user more than starting clean.
  const [query, setQuery] = useState('');
  const searchRef = useRef(null);

  useEffect(() => { savePref(STORAGE_KEYS.filter, filter); }, [filter]);
  useEffect(() => { savePref(STORAGE_KEYS.sort, sort); }, [sort]);
  useEffect(() => { savePref(STORAGE_KEYS.hideOffline, hideOffline); }, [hideOffline]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    let list = livestreams.filter((l) => {
      if (hideOffline && !l.is_live) return false;
      if (needle) {
        const hay = `${l.display_name} ${l.channel_id}`.toLowerCase();
        if (!hay.includes(needle)) return false;
      }
      switch (filter) {
        case 'twitch':    return l.platform === 'twitch';
        case 'kick':      return l.platform === 'kick';
        case 'youtube':   return l.platform === 'youtube';
        case 'favorites': return Boolean(l.favorite);
        default:          return true;
      }
    });

    const byName = (a, b) => a.display_name.localeCompare(b.display_name);
    const cmp = {
      viewers:  (a, b) => (b.viewers ?? -1) - (a.viewers ?? -1) || byName(a, b),
      name:     byName,
      playing:  (a, b) => Number(playing.has(b.unique_key)) - Number(playing.has(a.unique_key)) || byName(a, b),
      lastseen: (a, b) => (ts(b.last_checked) - ts(a.last_checked)) || byName(a, b),
      timelive: (a, b) => (ts(a.started_at) - ts(b.started_at)) || byName(a, b),
    }[sort] ?? byName;

    list = [...list];
    // Live rows always float above offline rows, regardless of sort key.
    list.sort((a, b) => (a.is_live === b.is_live ? cmp(a, b) : a.is_live ? -1 : 1));
    return list;
  }, [livestreams, filter, sort, hideOffline, playing, query]);

  const liveCount = filtered.filter((l) => l.is_live).length;
  const filterLabel = FILTER_OPTS.find((o) => o.k === filter)?.l ?? 'All';
  const sortLabel = SORT_OPTS.find((o) => o.k === sort)?.l ?? 'Viewers';

  return (
    <>
      <div className="cmd-row">
        {/* Sidebar */}
        <div className="cmd-sidebar">
          <div style={{ padding: '10px 12px 4px', display: 'flex', alignItems: 'center', gap: 6 }}>
            <div className="rx-chiclet">Channels</div>
            <div style={{ flex: 1 }} />
            <div className="rx-chiclet" style={{ color: 'var(--zinc-400)' }}>
              {liveCount}/{filtered.length}
            </div>
            <IconBtn
              title={loading ? 'Refreshing…' : 'Refresh now (F5)'}
              onClick={() => { if (!loading) refresh(); }}
            >
              <IconRefresh spinning={loading} />
            </IconBtn>
          </div>
          <div
            className="cmd-toolbar"
            style={{
              padding: '2px 10px 8px',
              display: 'flex',
              alignItems: 'center',
              gap: 2,
              borderBottom: 'var(--hair)',
            }}
          >
            <div style={{ position: 'relative' }}>
              <IconBtn
                title={`Filter: ${filterLabel}`}
                active={openMenu === 'filter' || filter !== 'all'}
                caret
                onClick={() => setOpenMenu(openMenu === 'filter' ? null : 'filter')}
              >
                <IconFilter />
              </IconBtn>
              {openMenu === 'filter' && (
                <Dropdown
                  items={FILTER_OPTS}
                  selected={filter}
                  onSelect={setFilter}
                  onClose={() => setOpenMenu(null)}
                />
              )}
            </div>
            <div style={{ position: 'relative' }}>
              <IconBtn
                title={`Sort: ${sortLabel}`}
                active={openMenu === 'sort'}
                caret
                onClick={() => setOpenMenu(openMenu === 'sort' ? null : 'sort')}
              >
                <IconSort />
              </IconBtn>
              {openMenu === 'sort' && (
                <Dropdown
                  items={SORT_OPTS}
                  selected={sort}
                  onSelect={setSort}
                  onClose={() => setOpenMenu(null)}
                  width={130}
                />
              )}
            </div>
            <IconBtn
              title={hideOffline ? 'Show offline channels' : 'Hide offline channels'}
              active={hideOffline}
              onClick={() => setHideOffline((v) => !v)}
            >
              <IconHide active={hideOffline} />
            </IconBtn>
            <div style={{ flex: 1 }} />
            <span
              className="rx-mono"
              style={{ fontSize: 9, color: 'var(--zinc-600)', whiteSpace: 'nowrap' }}
            >
              {filterLabel.toLowerCase()} · {sortLabel.toLowerCase()}
            </span>
          </div>
          <div className="cmd-search" style={{ padding: '6px 10px', borderBottom: 'var(--hair)' }}>
            <div style={{ position: 'relative' }}>
              <input
                ref={searchRef}
                type="text"
                className="rx-input"
                placeholder="Search channels…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape' && query) {
                    e.stopPropagation();
                    setQuery('');
                  }
                }}
                style={{
                  width: '100%',
                  boxSizing: 'border-box',
                  paddingRight: query ? 22 : undefined,
                }}
              />
              {query && (
                <button
                  type="button"
                  onClick={() => {
                    setQuery('');
                    searchRef.current?.focus();
                  }}
                  aria-label="Clear search"
                  style={{
                    position: 'absolute',
                    right: 4,
                    top: '50%',
                    transform: 'translateY(-50%)',
                    background: 'transparent',
                    border: 'none',
                    padding: 4,
                    color: 'var(--zinc-500)',
                    cursor: 'pointer',
                    lineHeight: 0,
                    display: 'inline-flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                  }}
                  onMouseEnter={(e) => {
                    e.currentTarget.style.color = 'var(--zinc-300)';
                  }}
                  onMouseLeave={(e) => {
                    e.currentTarget.style.color = 'var(--zinc-500)';
                  }}
                >
                  <IconClose />
                </button>
              )}
            </div>
          </div>
          <div style={{ flex: 1, overflowY: 'auto' }}>
            {filtered.length === 0 && query.trim() && (
              <div
                style={{
                  padding: '12px',
                  color: 'var(--zinc-500)',
                  fontSize: 'var(--t-12)',
                  textAlign: 'center',
                }}
              >
                No matches for “{query.trim()}”
              </div>
            )}
            {filtered.map((ch) => {
              const active = ch.unique_key === activeTabKey;
              const isPlaying = playing.has(ch.unique_key);
              return (
                <Tooltip
                  key={ch.unique_key}
                  block
                  text={ch.is_live ? 'Double-click to play' : null}
                >
                  <button
                    type="button"
                    className={`cmd-row-item${active ? ' active' : ''}`}
                    onClick={() => rowClickHandler(ch.unique_key)}
                    onDoubleClick={() => {
                      if (ch.is_live) launchStream(ch.unique_key);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      rowClickHandler(ch.unique_key);
                      setMenu({ x: e.clientX, y: e.clientY, channel: ch });
                    }}
                    style={{ opacity: ch.is_live ? 1 : 0.45 }}
                  >
                  <span className={`rx-status-dot ${ch.is_live ? 'live' : 'off'}`} />

                  {/* Collapsed-only chip — hidden by default, shown only when data-sidebar-collapsed="true". */}
                  <span className="cmd-row-chip-collapsed">
                    <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
                  </span>

                  {/* Center column — name row + meta line. Hidden as a unit in collapsed mode. */}
                  <div className="cmd-row-text" style={{ minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      {ch.favorite && (
                        <Tooltip text="Unfavorite">
                          <span
                            role="button"
                            aria-label="Unfavorite"
                            onClick={(e) => {
                              e.stopPropagation();
                              setFavorite(ch.unique_key, false);
                            }}
                            onDoubleClick={(e) => e.stopPropagation()}
                            style={{
                              display: 'inline-flex',
                              alignItems: 'center',
                              cursor: 'pointer',
                              color: 'var(--zinc-100)',
                              lineHeight: 0,
                            }}
                          >
                            <IconStar filled />
                          </span>
                        </Tooltip>
                      )}
                      <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>
                        {ch.display_name}
                      </span>
                      {isPlaying && (
                        <Tooltip text="Playing">
                          <span style={{ color: 'var(--ok)', fontSize: 9, lineHeight: 1 }}>▶</span>
                        </Tooltip>
                      )}
                      {/* Original inline chip — visible in expanded mode only (via cmd-row-text). */}
                      <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
                      {detachedKeys.has(ch.unique_key) && (
                        <Tooltip text="Open in detached window">
                          <span style={{ color: 'var(--zinc-500)', fontSize: 10, lineHeight: 1 }}>⤴</span>
                        </Tooltip>
                      )}
                    </div>
                    <div
                      className="rx-mono cmd-row-meta"
                      style={{
                        fontSize: 10,
                        color: 'var(--zinc-500)',
                        whiteSpace: 'nowrap',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                      }}
                    >
                      {ch.is_live ? (ch.game ?? 'live') : 'offline'}
                    </div>
                  </div>

                  {/* Viewers cluster — hidden in compact density and collapsed mode. */}
                  <div className="cmd-row-meta" style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
                    <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>
                      {ch.is_live ? formatViewers(ch.viewers) : '—'}
                    </span>
                  </div>
                  </button>
                </Tooltip>
              );
            })}
          </div>
          <button
            type="button"
            className="cmd-add"
            onClick={openAddDialog}
            style={{
              padding: '8px 12px',
              borderTop: 'var(--hair)',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              background: 'transparent',
              border: 'none',
              color: 'var(--zinc-300)',
              cursor: 'pointer',
              fontFamily: 'inherit',
              textAlign: 'left',
            }}
          >
            <div className="rx-kbd">N</div>
            <span className="rx-chiclet">Add channel</span>
          </button>
        </div>

        {/* Main */}
        <div className="cmd-main">
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            mentions={mentions}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            onReorder={reorderTabs}
            onDetach={detachTab}
          />
          <div style={{ flex: 1, position: 'relative', minWidth: 0 }}>
            {tabKeys.length === 0 && (
              <div
                style={{
                  position: 'absolute', inset: 0,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  color: 'var(--zinc-500)', fontSize: 'var(--t-12)',
                  textAlign: 'center', padding: '0 24px',
                }}
              >
                No chat selected — click a channel on the left to open it.
              </div>
            )}
            {tabKeys.map((k) => {
              const channel = livestreams.find((l) => l.unique_key === k);
              if (!channel) return null;
              return (
                <div
                  key={k}
                  style={{
                    position: 'absolute', inset: 0,
                    display: k === activeTabKey ? 'flex' : 'none',
                    flexDirection: 'column',
                  }}
                >
                  <SelectedPane
                    channel={channel}
                    isActiveTab={k === activeTabKey}
                    onMention={notifyMention}
                    onLaunch={() => launchStream(k)}
                    onOpenBrowser={() => openInBrowser(k)}
                    onFavorite={() => setFavorite(k, !channel.favorite)}
                    onUsernameOpen={onUsernameOpen}
                    onUsernameContext={onUsernameContext}
                    onUsernameHover={onUsernameHover}
                  />
                </div>
              );
            })}
          </div>
        </div>
      </div>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
        >
          <ContextMenu.Item
            disabled={!menu.channel.is_live || playing.has(menu.channel.unique_key)}
            onClick={() => {
              launchStream(menu.channel.unique_key);
              setMenu(null);
            }}
          >
            {menu.channel.is_live ? 'Play' : 'Play (offline)'}
          </ContextMenu.Item>
          <ContextMenu.Item
            disabled={!playing.has(menu.channel.unique_key)}
            onClick={() => {
              stopStream(menu.channel.unique_key).catch(() => {});
              setMenu(null);
            }}
          >
            Stop
          </ContextMenu.Item>
          <ContextMenu.Item
            onClick={() => {
              openInBrowser(menu.channel.unique_key);
              setMenu(null);
            }}
          >
            Open in browser
          </ContextMenu.Item>
          <ContextMenu.Separator />
          <ContextMenu.Item
            onClick={() => {
              setFavorite(menu.channel.unique_key, !menu.channel.favorite);
              setMenu(null);
            }}
          >
            {menu.channel.favorite ? 'Unpin from favorites' : 'Pin as favorite'}
          </ContextMenu.Item>
          <ContextMenu.Separator />
          <ContextMenu.Item
            danger
            onClick={() => {
              removeChannel(menu.channel.unique_key);
              setMenu(null);
            }}
          >
            Delete channel
          </ContextMenu.Item>
        </ContextMenu>
      )}
    </>
  );
}

function SelectedPane({ channel, isActiveTab, onMention, onLaunch, onOpenBrowser, onUsernameOpen, onUsernameContext, onUsernameHover }) {
  return (
    <>
      <div
        style={{
          height: 40,
          display: 'flex',
          alignItems: 'center',
          gap: 14,
          padding: '0 16px',
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        {channel.is_live ? <span className="rx-live-dot pulse" /> : <span className="rx-status-dot off" />}
        <span style={{ fontSize: 'var(--t-13)', color: 'var(--zinc-100)', fontWeight: 600 }}>
          {channel.display_name}
        </span>
        <span className={`rx-plat ${channel.platform.charAt(0)}`}>{channel.platform.toUpperCase()}</span>
        {channel.is_live && (
          <>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              {channel.game ?? ''}
            </span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              {formatViewers(channel.viewers)} viewers
            </span>
            <span style={{ color: 'var(--zinc-700)' }}>·</span>
            <span className="rx-mono" style={{ fontSize: 'var(--t-11)', color: 'var(--zinc-400)' }}>
              up {formatUptime(channel.started_at)}
            </span>
          </>
        )}
        <div style={{ flex: 1 }} />
        <button className="rx-btn rx-btn-ghost" onClick={onOpenBrowser}>Open in browser</button>
        <button
          className="rx-btn rx-btn-primary"
          disabled={!channel.is_live}
          onClick={onLaunch}
          style={channel.is_live ? undefined : { opacity: 0.4, cursor: 'not-allowed' }}
        >
          {channel.is_live ? 'Play ↗' : 'Offline'}
        </button>
      </div>

      <ChatView
        channelKey={channel.unique_key}
        variant="irc"
        isLive={Boolean(channel.is_live)}
        isActiveTab={isActiveTab !== false}
        onMention={onMention}
        header={
          <>
            <TitleBanner channel={channel} />
            <SocialsBanner channelKey={channel.unique_key} />
          </>
        }
        onUsernameOpen={onUsernameOpen}
        onUsernameContext={onUsernameContext}
        onUsernameHover={onUsernameHover}
      />
    </>
  );
}

/* ── timestamp helper for sort keys ────────────────────────────── */
function ts(iso) {
  if (!iso) return 0;
  const d = new Date(iso).getTime();
  return Number.isFinite(d) ? d : 0;
}

/* ── hairline SVG icons, 12px — match design language ──────────── */
function IconFilter() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
    >
      <path d="M1 2.5 L11 2.5 M3 6 L9 6 M5 9.5 L7 9.5" />
    </svg>
  );
}
function IconSort() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
      strokeLinejoin="miter"
    >
      <path d="M3 2 L3 10 M1.5 8.5 L3 10 L4.5 8.5" />
      <path d="M9 10 L9 2 M7.5 3.5 L9 2 L10.5 3.5" />
    </svg>
  );
}
function IconHide({ active }) {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
    >
      <path d="M1 6 C3 3, 9 3, 11 6 C9 9, 3 9, 1 6 Z" />
      <circle cx="6" cy="6" r="1.5" fill={active ? 'currentColor' : 'none'} />
      {active && <path d="M1.5 1.5 L10.5 10.5" />}
    </svg>
  );
}
function IconCaret() {
  return (
    <svg width="7" height="7" viewBox="0 0 7 7" fill="none" stroke="currentColor" strokeWidth="1">
      <path d="M1.5 2.5 L3.5 4.5 L5.5 2.5" />
    </svg>
  );
}
function IconStar({ filled }) {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 12 12"
      fill={filled ? 'currentColor' : 'none'}
      stroke="currentColor"
      strokeWidth="1"
      strokeLinejoin="miter"
    >
      <path d="M6 1.2 L7.4 4.4 L11 4.8 L8.4 7.3 L9.1 11 L6 9.2 L2.9 11 L3.6 7.3 L1 4.8 L4.6 4.4 Z" />
    </svg>
  );
}
function IconClose() {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
    >
      <path d="M2 2 L8 8 M8 2 L2 8" />
    </svg>
  );
}
function IconRefresh({ spinning }) {
  // Two-arrow loop — two arcs forming most of a circle with a small
  // gap at each side, each ending in a chunky chevron pointing into
  // the gap. Classic browser-style refresh affordance.
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
      strokeLinejoin="miter"
      style={
        spinning
          ? { animation: 'rx-spin 800ms linear infinite', transformOrigin: '50% 50%' }
          : undefined
      }
    >
      <path d="M 2.5 8 A 4 4 0 0 1 8 2.5" />
      <path d="M 8 1.5 L 8 2.5 L 7 2.5" />
      <path d="M 9.5 4 A 4 4 0 0 1 4 9.5" />
      <path d="M 4 10.5 L 4 9.5 L 5 9.5" />
    </svg>
  );
}

/* ── icon button with optional caret + active state + themed tooltip ── */
function IconBtn({ children, caret, active, onClick, title }) {
  return (
    <Tooltip text={title}>
      <button
        type="button"
        onClick={onClick}
        aria-label={title}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 3,
          padding: '3px 5px',
          background: active ? 'var(--zinc-900)' : 'transparent',
          border: '1px solid',
          borderColor: active ? 'var(--zinc-800)' : 'transparent',
          borderRadius: 3,
          color: active ? 'var(--zinc-200)' : 'var(--zinc-500)',
          cursor: 'pointer',
          lineHeight: 0,
          fontFamily: 'inherit',
        }}
        onMouseEnter={(e) => {
          if (!active) e.currentTarget.style.color = 'var(--zinc-300)';
        }}
        onMouseLeave={(e) => {
          if (!active) e.currentTarget.style.color = 'var(--zinc-500)';
        }}
      >
        {children}
        {caret && <IconCaret />}
      </button>
    </Tooltip>
  );
}

/* ── tiny dropdown, anchored under trigger ─────────────────────── */
function Dropdown({ items, selected, onSelect, onClose, width = 150 }) {
  return (
    <>
      <div
        onClick={onClose}
        style={{ position: 'fixed', inset: 0, zIndex: 10 }}
      />
      <div
        style={{
          position: 'absolute',
          top: '100%',
          left: 0,
          marginTop: 4,
          width,
          zIndex: 11,
          background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)',
          borderRadius: 4,
          boxShadow: '0 8px 24px rgba(0,0,0,.6), 0 0 0 1px rgba(255,255,255,.03)',
          padding: '3px 0',
        }}
      >
        {items.map((it) => (
          <div
            key={it.k}
            onClick={() => {
              onSelect(it.k);
              onClose();
            }}
            style={{
              padding: '5px 10px',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              fontSize: 'var(--t-12)',
              color: it.k === selected ? 'var(--zinc-100)' : 'var(--zinc-400)',
              background: it.k === selected ? 'var(--zinc-900)' : 'transparent',
              cursor: 'pointer',
              whiteSpace: 'nowrap',
            }}
            onMouseEnter={(e) => {
              if (it.k !== selected) e.currentTarget.style.background = 'var(--zinc-900)';
            }}
            onMouseLeave={(e) => {
              if (it.k !== selected) e.currentTarget.style.background = 'transparent';
            }}
          >
            <span style={{ width: 8, display: 'inline-flex', color: 'var(--zinc-500)' }}>
              {it.k === selected ? '›' : ''}
            </span>
            {it.icon && <span className={`rx-plat ${it.icon}`}>{it.icon.toUpperCase()}</span>}
            <span style={{ flex: 1 }}>{it.l}</span>
          </div>
        ))}
      </div>
    </>
  );
}
