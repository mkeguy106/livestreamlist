import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Command from './directions/Command.jsx';
import Columns from './directions/Columns.jsx';
import Focus from './directions/Focus.jsx';
import AddChannelDialog from './components/AddChannelDialog.jsx';
import LoginButton from './components/LoginButton.jsx';
import NicknameDialog from './components/NicknameDialog.jsx';
import NoteDialog from './components/NoteDialog.jsx';
import UserCard from './components/UserCard.jsx';
import UserCardContextMenu from './components/UserCardContextMenu.jsx';
import UserHistoryDialog from './components/UserHistoryDialog.jsx';
import WindowControls from './components/WindowControls.jsx';
import PreferencesDialog from './components/PreferencesDialog.jsx';
import ResizeHandles from './components/ResizeHandles.jsx';
import Tooltip from './components/Tooltip.jsx';
import { useDragHandler } from './hooks/useDragRegion.js';
import { useLivestreams } from './hooks/useLivestreams.js';
import { usePreferences } from './hooks/usePreferences.jsx';
import { useUserCard } from './hooks/useUserCard.js';
import { embedSetVisible, getUserMetadata, launchStream, listenEvent, openInBrowser, removeChannel, setFavorite, setUserMetadata } from './ipc.js';

const LAYOUTS = [
  { id: 'command', label: 'Command', letter: 'A', Component: Command },
  { id: 'columns', label: 'Columns', letter: 'B', Component: Columns },
  { id: 'focus',   label: 'Focus',   letter: 'C', Component: Focus   },
];
const STORAGE_KEY = 'livestreamlist.layout';
const SELECTED_STORAGE_KEY = 'livestreamlist.lastChannel';

function loadInitialSelectedKey() {
  try {
    return localStorage.getItem(SELECTED_STORAGE_KEY) || null;
  } catch {
    return null;
  }
}

function loadInitialLayout() {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved && LAYOUTS.some((l) => l.id === saved)) return saved;
  } catch {}
  return LAYOUTS[0].id;
}

export default function App() {
  const [layoutId, setLayoutId] = useState(loadInitialLayout);
  const [addOpen, setAddOpen] = useState(false);
  const [prefsOpen, setPrefsOpen] = useState(false);
  const [selectedKey, setSelectedKey] = useState(loadInitialSelectedKey);

  const { settings } = usePreferences();
  const hoverEnabled = settings?.chat?.user_card_hover !== false; // default true
  const hoverDelay = settings?.chat?.user_card_hover_delay_ms ?? 400;
  const intervalSeconds = settings?.general?.refresh_interval_seconds;
  const { livestreams, loading, error, refresh } = useLivestreams({ intervalSeconds });
  const onTitlebarMouseDown = useDragHandler();
  const card = useUserCard();
  // Destructure stable callbacks once so dependent useCallbacks don't
  // rebuild on every card-state change (which would cascade re-renders
  // through ctx → all three layouts → ChatView during active chat).
  const { openFor: cardOpenFor, close: cardClose } = card;

  const hoverTimer = useRef(null);
  const closeTimer = useRef(null);
  const overCard = useRef(false);
  const overAnchor = useRef(false);
  // True while the open card was opened by an explicit click. Hover-driven
  // open/close is suppressed until the card is dismissed (Esc, outside click,
  // or another explicit click on a different username).
  const lockedByClick = useRef(false);

  const onUsernameOpen = useCallback(
    (user, rect, channelKey) => {
      lockedByClick.current = true;
      // Cancel any pending hover-open / hover-close timers — the click wins.
      if (hoverTimer.current) clearTimeout(hoverTimer.current);
      if (closeTimer.current) clearTimeout(closeTimer.current);
      cardOpenFor(user, channelKey, rect);
    },
    [cardOpenFor],
  );

  // Reset the click-lock whenever the card actually closes (Esc, outside-click,
  // or any other path that flips card.open back to false).
  useEffect(() => {
    if (!card.open) lockedByClick.current = false;
  }, [card.open]);

  const [userCtx, setUserCtx] = useState({ open: false, point: null, user: null, channelKey: null, metadata: null });
  const [nickDlg, setNickDlg] = useState({ open: false });
  const [noteDlg, setNoteDlg] = useState({ open: false });
  const [historyDlg, setHistoryDlg] = useState({ open: false });

  const onUsernameContext = useCallback(async (user, point, channelKey) => {
    let metadata = null;
    if (user.id) {
      try {
        metadata = await getUserMetadata(`twitch:${user.id}`);
      } catch (e) {
        console.error('get_user_metadata', e);
      }
    }
    setUserCtx({ open: true, point, user, channelKey, metadata });
  }, []);

  const onUsernameHover = useCallback(
    (user, rect, channelKey) => {
      if (!hoverEnabled) return;
      // While a click-opened card is showing, ignore all hover signals so the
      // card doesn't yoink to a different user when chat scrolls or the cursor
      // drifts onto another name.
      if (lockedByClick.current) return;
      if (user) {
        // entering an anchor
        overAnchor.current = true;
        if (hoverTimer.current) clearTimeout(hoverTimer.current);
        if (closeTimer.current) clearTimeout(closeTimer.current);
        hoverTimer.current = setTimeout(() => {
          if (overAnchor.current) cardOpenFor(user, channelKey, rect);
        }, hoverDelay);
      } else {
        // leaving the anchor
        overAnchor.current = false;
        if (hoverTimer.current) clearTimeout(hoverTimer.current);
        // Small delay so the cursor can move into the card before we close it.
        if (closeTimer.current) clearTimeout(closeTimer.current);
        closeTimer.current = setTimeout(() => {
          if (!overAnchor.current && !overCard.current) cardClose();
        }, 100);
      }
    },
    [hoverEnabled, hoverDelay, cardOpenFor, cardClose],
  );

  const onCardHover = useCallback((over) => {
    overCard.current = over;
    // Click-locked cards never auto-close on cursor leave.
    if (lockedByClick.current) {
      if (closeTimer.current) {
        clearTimeout(closeTimer.current);
        closeTimer.current = null;
      }
      return;
    }
    if (!over) {
      if (closeTimer.current) clearTimeout(closeTimer.current);
      closeTimer.current = setTimeout(() => {
        if (!overAnchor.current && !overCard.current) cardClose();
      }, 100);
    } else if (closeTimer.current) {
      // Cursor entered the card before the close timer fired — keep it open.
      clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, [cardClose]);

  // Apply appearance overrides to CSS variables on the root.
  useEffect(() => {
    const root = document.documentElement;
    const accent = settings?.appearance?.accent_override;
    const live = settings?.appearance?.live_color_override;
    if (accent && /^#[0-9a-f]{6}$/i.test(accent)) {
      root.style.setProperty('--zinc-100', accent);
    } else {
      root.style.removeProperty('--zinc-100');
    }
    if (live && /^#[0-9a-f]{6}$/i.test(live)) {
      root.style.setProperty('--live', live);
    } else {
      root.style.removeProperty('--live');
    }
  }, [settings?.appearance?.accent_override, settings?.appearance?.live_color_override]);

  // Honor default layout on first launch (i.e. when localStorage hasn't been
  // written yet by a user-driven switch).
  useEffect(() => {
    const saved = (() => { try { return localStorage.getItem(STORAGE_KEY); } catch { return null; } })();
    if (saved) return;
    const def = settings?.appearance?.default_layout;
    if (def && LAYOUTS.some((l) => l.id === def)) setLayoutId(def);
  }, [settings?.appearance?.default_layout]);

  // The cached `list_livestreams` snapshot returns all channels with
  // is_live=false on a fresh launch (live state is transient — not
  // persisted across runs). Both effects below gate on `loading` so we
  // wait for the first `refresh_all` to actually populate live state
  // before making any selection decisions; otherwise we'd validate the
  // restored channel against stale data and always fall back.
  const restoredKeyAtMount = useRef(selectedKey);
  const restoredValidated = useRef(false);

  // One-shot: a channel restored from localStorage at mount is only
  // honored if the first refresh confirms it's live. Only validates
  // the value that was in localStorage at mount — a user click during
  // the loading window is left alone.
  useEffect(() => {
    if (loading) return;
    if (livestreams.length === 0) return;
    if (restoredValidated.current) return;
    restoredValidated.current = true;
    if (selectedKey == null || selectedKey !== restoredKeyAtMount.current) return;
    const ch = livestreams.find((l) => l.unique_key === selectedKey);
    if (!ch || !ch.is_live) {
      setSelectedKey(null);
    }
  }, [livestreams, selectedKey, loading]);

  // Default selection: first live channel, else first in list. Skips
  // while loading so the cached-with-offline snapshot doesn't pick a
  // wrong default that we'd then have to correct.
  useEffect(() => {
    if (loading) return;
    if (livestreams.length === 0) return;
    if (selectedKey && livestreams.some((l) => l.unique_key === selectedKey)) return;
    const firstLive = livestreams.find((l) => l.is_live);
    const first = firstLive ?? livestreams[0];
    setSelectedKey(first?.unique_key ?? null);
  }, [livestreams, selectedKey, loading]);

  // Persist selection across runs.
  useEffect(() => {
    try {
      if (selectedKey) localStorage.setItem(SELECTED_STORAGE_KEY, selectedKey);
      else localStorage.removeItem(SELECTED_STORAGE_KEY);
    } catch {}
  }, [selectedKey]);

  useEffect(() => {
    try { localStorage.setItem(STORAGE_KEY, layoutId); } catch {}
  }, [layoutId]);

  const selectLayout = useCallback((id) => setLayoutId(id), []);

  useEffect(() => {
    const onKey = (e) => {
      const target = e.target;
      const inField = target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable);
      if (inField) return;
      if (e.key === '1') selectLayout('command');
      else if (e.key === '2') selectLayout('columns');
      else if (e.key === '3') selectLayout('focus');
      else if (e.key === 'n' || (e.key.toLowerCase() === 'a' && (e.metaKey || e.ctrlKey) && e.shiftKey)) {
        e.preventDefault();
        setAddOpen(true);
      } else if (e.key.toLowerCase() === 'r' && !(e.metaKey || e.ctrlKey)) {
        refresh();
      } else if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault();
        setPrefsOpen(true);
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [selectLayout, refresh]);

  // Tray "Refresh now" menu item fires this event.
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    (async () => {
      unlisten = await listenEvent('tray:refresh-requested', () => {
        if (!cancelled) refresh();
      });
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [refresh]);

  // Native child webviews (YouTube/Chaturbate inline chat) sit above the HTML
  // layer on Linux/Wayland — anything that overlaps them gets occluded. Hide
  // every embed while a full-window modal is up; restore on close.
  const anyDialogOpen = addOpen || prefsOpen || nickDlg.open || noteDlg.open
    || historyDlg.open || userCtx.open || card.open;
  useEffect(() => {
    embedSetVisible(!anyDialogOpen).catch(() => {});
  }, [anyDialogOpen]);

  const ctx = useMemo(() => ({
    livestreams,
    loading,
    error,
    refresh,
    selectedKey,
    setSelectedKey,
    openAddDialog: () => setAddOpen(true),
    launchStream: (key, quality = 'best') =>
      launchStream(key, quality).catch((e) => console.error('launch_stream', e)),
    openInBrowser: (key) =>
      openInBrowser(key).catch((e) => console.error('open_in_browser', e)),
    removeChannel: (key) =>
      removeChannel(key).then(refresh).catch((e) => console.error('remove_channel', e)),
    setFavorite: (key, fav) =>
      setFavorite(key, fav).then(refresh).catch((e) => console.error('set_favorite', e)),
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  }), [livestreams, loading, error, refresh, selectedKey, onUsernameOpen, onUsernameContext, onUsernameHover]);

  const current = LAYOUTS.find((l) => l.id === layoutId) ?? LAYOUTS[0];
  const Layout = current.Component;

  const liveCount = livestreams.filter((l) => l.is_live).length;
  const totalCount = livestreams.length;
  const selected = livestreams.find((l) => l.unique_key === selectedKey);

  const rightLabel = layoutId === 'focus' && selected
    ? `focus: ${selected.display_name}`
    : layoutId === 'columns'
    ? `${liveCount} live · ${totalCount} channels`
    : `${liveCount} live · ${totalCount} channels`;

  return (
    <div className="rx-root">
      <ResizeHandles />
      <div className="rx-titlebar" data-tauri-drag-region onMouseDown={onTitlebarMouseDown}>
        <div className="rx-tb-dots" role="tablist" aria-label="Layout">
          {LAYOUTS.map((l) => (
            <Tooltip key={l.id} text={`${l.letter} · ${l.label}`}>
              <button
                type="button"
                role="tab"
                aria-selected={l.id === layoutId}
                aria-label={`${l.label} layout (${l.letter})`}
                className={`rx-tb-dot ${l.id === layoutId ? 'active' : ''}`}
                onClick={() => selectLayout(l.id)}
              />
            </Tooltip>
          ))}
        </div>
        <div style={{ width: 12 }} />
        <div className="rx-tb-label rx-mono">livestream.list</div>
        <div style={{ color: 'var(--zinc-700)' }}>·</div>
        <div className="rx-tb-label rx-mono" style={{ color: 'var(--zinc-400)' }}>
          {current.letter} · {current.label}
        </div>
        <div style={{ flex: 1 }} />
        <div className="rx-tb-label rx-mono">{rightLabel}</div>
        {error && <div className="rx-tb-label rx-mono" style={{ color: '#f87171' }}>· refresh failed</div>}
        <div style={{ width: 8 }} />
        <LoginButton />
        <Tooltip text="Preferences (⌘,)">
          <button
            type="button"
            className="rx-btn rx-btn-ghost"
            onClick={() => setPrefsOpen(true)}
            aria-label="Preferences"
            style={{ padding: '1px 6px', fontSize: 14 }}
          >
            ⚙
          </button>
        </Tooltip>
        <div style={{ width: 4 }} />
        <WindowControls />
      </div>

      <main
        style={{
          flex: 1,
          display: 'flex',
          flexDirection: 'column',
          minHeight: 0,
          position: 'relative',
        }}
      >
        {totalCount === 0 ? <EmptyState onAdd={() => setAddOpen(true)} /> : <Layout ctx={ctx} />}
      </main>

      <AddChannelDialog open={addOpen} onClose={() => setAddOpen(false)} onAdded={refresh} />
      <UserHistoryDialog
        open={historyDlg.open}
        channelKey={historyDlg.channelKey}
        user={historyDlg.user}
        onClose={() => setHistoryDlg({ open: false })}
      />
      <PreferencesDialog open={prefsOpen} onClose={() => setPrefsOpen(false)} />
      <UserCard
        open={card.open}
        anchor={card.anchor}
        user={card.user}
        metadata={card.metadata}
        profile={card.profile}
        profileLoading={card.profileLoading}
        profileError={card.profileError}
        onClose={card.close}
        onOpenHistory={() => {
          setHistoryDlg({ open: true, channelKey: card.channelKey, user: card.user });
          card.close();
        }}
        onOpenChannel={() => {
          if (card.channelKey) openInBrowser(card.channelKey).catch((e) => console.error('open_in_browser', e));
          card.close();
        }}
        onCardHover={onCardHover}
      />
      <UserCardContextMenu
        open={userCtx.open}
        point={userCtx.point}
        user={userCtx.user || {}}
        metadata={userCtx.metadata}
        onClose={() => setUserCtx(c => ({ ...c, open: false }))}
        onEditNickname={() => {
          setNickDlg({ open: true, user: userCtx.user, currentValue: userCtx.metadata?.nickname || '' });
        }}
        onEditNote={() => {
          setNoteDlg({ open: true, user: userCtx.user, currentValue: userCtx.metadata?.note || '' });
        }}
        onToggleBlocked={async () => {
          if (!userCtx.user?.id) return;
          const userKey = `twitch:${userCtx.user.id}`;
          try {
            await setUserMetadata(userKey, {
              blocked: !userCtx.metadata?.blocked,
              login_hint: userCtx.user.login,
              display_name_hint: userCtx.user.display_name,
            });
          } catch (e) {
            console.error('set_user_metadata', e);
          }
          if (card.open && card.user?.id === userCtx.user?.id) card.refreshMetadata();
        }}
      />
      <NicknameDialog
        open={nickDlg.open}
        user={nickDlg.user}
        currentValue={nickDlg.currentValue}
        onClose={() => setNickDlg({ open: false })}
        onSave={async (v) => {
          if (!nickDlg.user?.id) return;
          try {
            await setUserMetadata(`twitch:${nickDlg.user.id}`, {
              nickname: v,
              login_hint: nickDlg.user.login,
              display_name_hint: nickDlg.user.display_name,
            });
          } catch (e) { console.error('set_user_metadata', e); }
          setNickDlg({ open: false });
          if (card.user?.id === nickDlg.user.id) card.refreshMetadata();
        }}
        onClear={async () => {
          if (!nickDlg.user?.id) return;
          try {
            await setUserMetadata(`twitch:${nickDlg.user.id}`, {
              nickname: null,
              login_hint: nickDlg.user.login,
              display_name_hint: nickDlg.user.display_name,
            });
          } catch (e) { console.error('set_user_metadata', e); }
          setNickDlg({ open: false });
          if (card.user?.id === nickDlg.user.id) card.refreshMetadata();
        }}
      />
      <NoteDialog
        open={noteDlg.open}
        user={noteDlg.user}
        currentValue={noteDlg.currentValue}
        onClose={() => setNoteDlg({ open: false })}
        onSave={async (v) => {
          if (!noteDlg.user?.id) return;
          try {
            await setUserMetadata(`twitch:${noteDlg.user.id}`, {
              note: v,
              login_hint: noteDlg.user.login,
              display_name_hint: noteDlg.user.display_name,
            });
          } catch (e) { console.error('set_user_metadata', e); }
          setNoteDlg({ open: false });
          if (card.user?.id === noteDlg.user.id) card.refreshMetadata();
        }}
        onClear={async () => {
          if (!noteDlg.user?.id) return;
          try {
            await setUserMetadata(`twitch:${noteDlg.user.id}`, {
              note: null,
              login_hint: noteDlg.user.login,
              display_name_hint: noteDlg.user.display_name,
            });
          } catch (e) { console.error('set_user_metadata', e); }
          setNoteDlg({ open: false });
          if (card.user?.id === noteDlg.user.id) card.refreshMetadata();
        }}
      />
    </div>
  );
}

function EmptyState({ onAdd }) {
  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 14,
        color: 'var(--zinc-400)',
      }}
    >
      <div style={{ fontSize: 14, color: 'var(--zinc-100)', fontWeight: 500 }}>No channels yet</div>
      <div style={{ fontSize: 12, color: 'var(--zinc-500)', maxWidth: 420, textAlign: 'center' }}>
        Paste a Twitch, YouTube, Kick, or Chaturbate URL to start monitoring.
      </div>
      <button className="rx-btn rx-btn-primary" onClick={onAdd}>
        Add channel
      </button>
      <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>press N to add</div>
    </div>
  );
}
