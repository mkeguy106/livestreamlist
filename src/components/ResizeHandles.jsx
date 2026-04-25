import { useEffect, useState } from 'react';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

// 4 edges + 4 corners. Edges leave a 6 px gap at each end so corner handles
// always win the hit-test there.
const EDGE = 4;
const CORNER = 8;
const HANDLES = [
  { dir: 'North',     cursor: 'ns-resize',   style: { top: 0,    left: CORNER, right: CORNER, height: EDGE } },
  { dir: 'South',     cursor: 'ns-resize',   style: { bottom: 0, left: CORNER, right: CORNER, height: EDGE } },
  { dir: 'West',      cursor: 'ew-resize',   style: { top: CORNER, bottom: CORNER, left: 0,  width: EDGE } },
  { dir: 'East',      cursor: 'ew-resize',   style: { top: CORNER, bottom: CORNER, right: 0, width: EDGE } },
  { dir: 'NorthWest', cursor: 'nwse-resize', style: { top: 0,    left: 0,     width: CORNER, height: CORNER } },
  { dir: 'NorthEast', cursor: 'nesw-resize', style: { top: 0,    right: 0,    width: CORNER, height: CORNER } },
  { dir: 'SouthWest', cursor: 'nesw-resize', style: { bottom: 0, left: 0,     width: CORNER, height: CORNER } },
  { dir: 'SouthEast', cursor: 'nwse-resize', style: { bottom: 0, right: 0,    width: CORNER, height: CORNER } },
];

/**
 * Invisible edge/corner handles that drive native window resize.
 * Necessary because the window has `decorations: false` — without these,
 * the OS-provided resize cursor never shows even though resizing is allowed.
 *
 * Hidden while maximized/fullscreen so the cursor doesn't lie about
 * resize being possible there.
 */
export default function ResizeHandles() {
  const [active, setActive] = useState(inTauri);

  useEffect(() => {
    if (!inTauri) return;
    let unlisten = null;
    let cancelled = false;
    (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const win = getCurrentWindow();
      const update = async () => {
        try {
          const [max, full] = await Promise.all([win.isMaximized(), win.isFullscreen()]);
          if (!cancelled) setActive(!max && !full);
        } catch {}
      };
      await update();
      unlisten = await win.onResized(update);
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  if (!active) return null;

  const onMouseDown = (dir) => (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      getCurrentWindow().startResizeDragging(dir).catch(() => {});
    });
  };

  return (
    <>
      {HANDLES.map((h) => (
        <div
          key={h.dir}
          onMouseDown={onMouseDown(h.dir)}
          style={{
            position: 'absolute',
            zIndex: 9999,
            cursor: h.cursor,
            ...h.style,
          }}
        />
      ))}
    </>
  );
}
