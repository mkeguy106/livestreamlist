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
  // We compute next state from the current closure values and dispatch each
  // setter independently. React 18 batches the calls within the same
  // synchronous handler, so promotion stays atomic from React's POV without
  // calling other setters from inside an updater function (which Strict
  // Mode would invoke twice, doubling those side-effects).
  const openOrFocusTab = useCallback((channelKey) => {
    const [nextTabs] = openOrFocusReducer(tabKeys, activeTabKey, channelKey);
    if (nextTabs !== tabKeys) setTabKeys(nextTabs);
    if (channelKey !== activeTabKey) setActiveTabKey(channelKey);
  }, [tabKeys, activeTabKey]);

  const closeTab = useCallback((channelKey) => {
    const [nextTabs, nextActive] = closeTabReducer(tabKeys, activeTabKey, channelKey);
    if (nextTabs !== tabKeys) setTabKeys(nextTabs);
    if (nextActive !== activeTabKey) setActiveTabKey(nextActive);
  }, [tabKeys, activeTabKey]);

  const reorderTabs = useCallback((fromKey, toKey, position) => {
    setTabKeys((prev) => reorderTabsReducer(prev, fromKey, toKey, position));
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
