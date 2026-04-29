// src/hooks/useCommandTabs.js
//
// Owns the Command layout's tab + detach state. Persists each piece to
// localStorage. Cleans up tabs whose channel was deleted. Listens for
// chat-detach lifecycle events from Rust. The mention map and chat-detach
// IPC wiring land in PR 4 / PR 5 — this PR ships only the tab pieces.

import { useCallback, useEffect, useState } from 'react';
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

export function useCommandTabs({ livestreams }) {
  const [tabKeys, setTabKeys] = useState(loadInitialTabKeys);
  const [detachedKeys, setDetachedKeys] = useState(() => new Set(loadInitialDetachedKeys()));
  const [activeTabKey, setActiveTabKey] = useState(loadInitialActiveTabKey);

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

  // ── Public handlers ────────────────────────────────────────────────────
  const openOrFocusTab = useCallback((channelKey) => {
    setTabKeys((prev) => {
      const [next] = openOrFocusReducer(prev, activeTabKey, channelKey);
      return next;
    });
    setActiveTabKey(channelKey);
  }, [activeTabKey]);

  const closeTab = useCallback((channelKey) => {
    setTabKeys((prev) => {
      const [nextTabs, nextActive] = closeTabReducer(prev, activeTabKey, channelKey);
      // We need to update activeTabKey from inside this updater to keep the
      // promotion synchronous with the tab list change. setActiveTabKey is
      // fine to call here — React batches both updates together.
      if (nextActive !== activeTabKey) setActiveTabKey(nextActive);
      return nextTabs;
    });
  }, [activeTabKey]);

  const reorderTabs = useCallback((fromKey, toKey) => {
    setTabKeys((prev) => reorderTabsReducer(prev, fromKey, toKey));
  }, []);

  // Activating a tab is what users do when they click a tab in the strip
  // OR a row in the rail (when not detached). Both call setActiveTabKey
  // directly — the openOrFocusTab path covers the rail-row case where the
  // tab might not exist yet.
  const setActive = useCallback((channelKey) => {
    setActiveTabKey(channelKey);
  }, []);

  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    // PR 4 will add: detachTab, reattachTab, focusDetached
    // PR 5 will add: mentions, notifyMention, clearMention
  };
}
