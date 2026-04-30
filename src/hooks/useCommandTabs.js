// src/hooks/useCommandTabs.js
//
// Owns the Command layout's tab + detach state. Persists each piece to
// localStorage. Cleans up tabs whose channel was deleted. Listens for
// chat-detach lifecycle events from Rust. The mention map and chat-detach
// IPC wiring land in PR 4 / PR 5 — this PR ships only the tab pieces.

import { useCallback, useEffect, useRef, useState } from 'react';
import {
  closeTab as closeTabReducer,
  loadInitialActiveTabKey,
  loadInitialDetachedKeys,
  loadInitialTabKeys,
  openOrFocus as openOrFocusReducer,
  reorderTabs as reorderTabsReducer,
  saveActiveTabKey,
  saveDetachedKeys,
  saveTabKeys,
} from '../utils/commandTabs.js';
import { chatDetach, chatFocusDetached, listenEvent } from '../ipc.js';

export function useCommandTabs({ livestreams }) {
  const [tabKeys, setTabKeys] = useState(loadInitialTabKeys);
  const [detachedKeys, setDetachedKeys] = useState(() => new Set(loadInitialDetachedKeys()));
  const [activeTabKey, setActiveTabKey] = useState(loadInitialActiveTabKey);

  // mentions: Map<channelKey, { blinkUntil: number, hasUnseenMention: boolean }>
  // blinkUntil = 0 means no active blink; > now means blinking.
  // hasUnseenMention is sticky until the tab is focused.
  const [mentions, setMentions] = useState(() => new Map());

  const notifyMention = useCallback((channelKey) => {
    setMentions((prev) => {
      const next = new Map(prev);
      next.set(channelKey, {
        blinkUntil: Date.now() + 10_000,
        hasUnseenMention: true,
      });
      return next;
    });
  }, []);

  const clearMention = useCallback((channelKey) => {
    setMentions((prev) => {
      if (!prev.has(channelKey)) return prev;
      const next = new Map(prev);
      next.delete(channelKey);
      return next;
    });
  }, []);

  // 1s ticker prunes elapsed blinkUntil values. Doesn't touch
  // hasUnseenMention — only tab focus clears that.
  useEffect(() => {
    const id = setInterval(() => {
      setMentions((prev) => {
        let mutated = false;
        const next = new Map(prev);
        const now = Date.now();
        for (const [k, v] of next) {
          if (v.blinkUntil !== 0 && v.blinkUntil < now) {
            next.set(k, { ...v, blinkUntil: 0 });
            mutated = true;
          }
        }
        return mutated ? next : prev;
      });
    }, 1000);
    return () => clearInterval(id);
  }, []);

  // ── Persistence ────────────────────────────────────────────────────────
  useEffect(() => { saveTabKeys(tabKeys); }, [tabKeys]);
  useEffect(() => { saveDetachedKeys([...detachedKeys]); }, [detachedKeys]);
  useEffect(() => { saveActiveTabKey(activeTabKey); }, [activeTabKey]);

  // ── Cleanup on channel removal ─────────────────────────────────────────
  // If a channel is removed from the channel list (deleted via context menu,
  // or filtered out by some future mechanism), drop it from tabKeys and
  // detachedKeys so we don't render ghost tabs / dangling windows.
  useEffect(() => {
    if (livestreams.length === 0) return; // don't prune while empty/loading
    setTabKeys((prev) => {
      const valid = prev.filter((k) => livestreams.some((l) => l.unique_key === k));
      return valid.length === prev.length ? prev : valid;
    });
    setDetachedKeys((prev) => {
      const next = new Set();
      let mutated = false;
      for (const k of prev) {
        if (livestreams.some((l) => l.unique_key === k)) next.add(k);
        else mutated = true;
      }
      return mutated ? next : prev;
    });
    setActiveTabKey((prev) => {
      if (!prev) return prev;
      return livestreams.some((l) => l.unique_key === prev) ? prev : null;
    });
  }, [livestreams]);

  // Listen for the detach window's :closed event (fires for both close-button
  // and reattach-driven closes — the listener is idempotent).
  useEffect(() => {
    let cancelled = false;
    let unlisten = null;
    listenEvent('chat-detach:closed', (key) => {
      if (cancelled) return;
      setDetachedKeys((prev) => {
        if (!prev.has(key)) return prev;
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // Listen for the redock event (chat_reattach emits this BEFORE closing the
  // window). Move the channel back to tabs and focus it.
  useEffect(() => {
    let cancelled = false;
    let unlisten = null;
    listenEvent('chat-detach:redock', (key) => {
      if (cancelled) return;
      setDetachedKeys((prev) => {
        if (!prev.has(key)) return prev;
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
      setTabKeys((prev) => (prev.includes(key) ? prev : [...prev, key]));
      setActiveTabKey(key);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // On mount, fire chat_detach for any persisted detached entries. This runs
  // before the tab strip even renders, so the windows have a head start.
  // Filter to channels that still exist (the cleanup-on-channel-removal effect
  // will also catch this, but starting up valid avoids transient empty windows).
  // Note: we depend on `livestreams` so this re-runs once the first non-empty
  // snapshot arrives. Use a ref-guarded one-shot pattern.
  const restoredDetachedRef = useRef(false);
  useEffect(() => {
    if (restoredDetachedRef.current) return;
    if (livestreams.length === 0) return;
    restoredDetachedRef.current = true;
    for (const key of detachedKeys) {
      if (!livestreams.some((l) => l.unique_key === key)) {
        // Channel deleted between sessions — drop from set silently.
        setDetachedKeys((prev) => {
          if (!prev.has(key)) return prev;
          const next = new Set(prev);
          next.delete(key);
          return next;
        });
        continue;
      }
      chatDetach(key).catch((e) => console.error('chat_detach (restore)', e));
    }
  }, [livestreams, detachedKeys]);

  // ── Public handlers ────────────────────────────────────────────────────
  // We compute next state from the current closure values and dispatch each
  // setter independently. React 18 batches the calls within the same
  // synchronous handler, so promotion stays atomic from React's POV without
  // calling other setters from inside an updater function (which Strict
  // Mode would invoke twice, doubling those side-effects).
  const openOrFocusTab = useCallback((channelKey) => {
    const [nextTabs] = openOrFocusReducer(tabKeys, activeTabKey, channelKey);
    if (nextTabs !== tabKeys) setTabKeys(nextTabs);
    if (channelKey !== activeTabKey) setActiveTabKey(channelKey);
    setMentions((prev) => {
      if (!prev.has(channelKey)) return prev;
      const next = new Map(prev);
      next.delete(channelKey);
      return next;
    });
  }, [tabKeys, activeTabKey]);

  const closeTab = useCallback((channelKey) => {
    const [nextTabs, nextActive] = closeTabReducer(tabKeys, activeTabKey, channelKey);
    if (nextTabs !== tabKeys) setTabKeys(nextTabs);
    if (nextActive !== activeTabKey) setActiveTabKey(nextActive);
  }, [tabKeys, activeTabKey]);

  const reorderTabs = useCallback((fromKey, toKey, position) => {
    setTabKeys((prev) => reorderTabsReducer(prev, fromKey, toKey, position));
  }, []);

  const detachTab = useCallback(async (channelKey) => {
    try {
      await chatDetach(channelKey);
    } catch (e) {
      console.error('chat_detach', e);
      return;
    }
    // Move from tabKeys → detachedKeys. Promote the active tab if needed.
    // Compute next state from closure values and dispatch each setter
    // independently — same pattern as closeTab/openOrFocusTab. Calling
    // setActiveTabKey from inside a setTabKeys updater would double-fire
    // under React StrictMode. (See commit 19324dc for context.)
    const [nextTabs, nextActive] = closeTabReducer(tabKeys, activeTabKey, channelKey);
    if (nextTabs !== tabKeys) setTabKeys(nextTabs);
    if (nextActive !== activeTabKey) setActiveTabKey(nextActive);
    setDetachedKeys((prev) => {
      if (prev.has(channelKey)) return prev;
      const next = new Set(prev);
      next.add(channelKey);
      return next;
    });
  }, [tabKeys, activeTabKey]);

  // Smart row click for the rail: if the channel is currently detached, raise
  // its window. Otherwise open as a tab.
  const rowClickHandler = useCallback((channelKey) => {
    if (detachedKeys.has(channelKey)) {
      chatFocusDetached(channelKey).catch((e) => console.error('chat_focus_detached', e));
    } else {
      openOrFocusTab(channelKey);
    }
  }, [detachedKeys, openOrFocusTab]);

  // Activating a tab is what users do when they click a tab in the strip
  // OR a row in the rail (when not detached). Both call setActiveTabKey
  // directly — the openOrFocusTab path covers the rail-row case where the
  // tab might not exist yet.
  const setActive = useCallback((channelKey) => {
    setActiveTabKey(channelKey);
    if (channelKey) {
      setMentions((prev) => {
        if (!prev.has(channelKey)) return prev;
        const next = new Map(prev);
        next.delete(channelKey);
        return next;
      });
    }
  }, []);

  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    mentions,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    detachTab,
    rowClickHandler,
    notifyMention,
    clearMention,
  };
}
