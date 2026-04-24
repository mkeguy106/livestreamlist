import { useCallback } from 'react';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * Return a `mousedown` handler that starts a native window drag.
 *
 * Background: Tauri v2 ships a `data-tauri-drag-region` attribute that's
 * supposed to make an element draggable, but on WebKitGTK the injected
 * listener is unreliable. A manual handler that calls `startDragging()`
 * directly works everywhere the Tauri API is reachable.
 *
 * Skips the drag if the user clicked an interactive descendant — buttons,
 * inputs, links, etc. — so child controls keep working.
 */
export function useDragHandler() {
  return useCallback((e) => {
    if (!inTauri) return;
    if (e.button !== 0) return;
    const t = e.target;
    if (!(t instanceof HTMLElement)) return;
    if (t.closest('button, input, textarea, select, a, [role="button"], [data-no-drag]')) {
      return;
    }
    // Two clicks in quick succession → toggle maximize, matching native titlebars.
    if (e.detail === 2) {
      import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
        getCurrentWindow().toggleMaximize().catch(() => {});
      });
      return;
    }
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      getCurrentWindow().startDragging().catch(() => {});
    });
  }, []);
}
