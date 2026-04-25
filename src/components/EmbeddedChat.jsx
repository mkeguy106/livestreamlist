import { useEffect, useRef, useState } from 'react';
import { embedMount, embedPosition, embedUnmount } from '../ipc.js';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * Reserves an empty rectangle in the chat-pane region. A separate borderless
 * top-level WebviewWindow is mounted by Rust at this rectangle's screen
 * coordinates. Channel switches reuse the same window via Rust-side
 * `navigate()` so the WM never closes/recreates the window — no animations
 * on every channel click.
 *
 * Why a top-level window: YouTube blocks `<iframe>` via X-Frame-Options, and
 * Tauri's `add_child` puts child webviews in `gtk::Box` (auto-positioned,
 * `set_position`/`set_size` ignored). Top-level borderless transient_for
 * windows ARE positionable and don't have iframe restrictions.
 */
export default function EmbeddedChat({ channelKey, isLive, placeholderText }) {
    const placeholderRef = useRef(null);
    const mountedKeyRef = useRef(null);
    const winApiRef = useRef(null);
    const cleanupListenersRef = useRef(null);
    const outerPosRef = useRef({ x: 0, y: 0 });
    const [error, setError] = useState(null);
    const [waiting, setWaiting] = useState(false);

    // Effect A: handles channel/live changes. On switch, calls embedMount
    // which the Rust side resolves into either a fresh window or a reuse-
    // via-navigate. NO embedUnmount here — closing on every channelKey
    // change would re-trigger KWin's close animation.
    useEffect(() => {
        if (!inTauri) return;
        if (!channelKey || !isLive) {
            // Channel offline / no selection: tear down so the React
            // placeholder ('channel offline…') becomes visible.
            if (mountedKeyRef.current) {
                embedUnmount(mountedKeyRef.current).catch(() => {});
                mountedKeyRef.current = null;
            }
            return;
        }

        const el = placeholderRef.current;
        if (!el) return;
        let cancelled = false;

        // Synchronous rect compute using the cached outer position (kept up-
        // to-date by the onMoved listener). Avoids an IPC round-trip on every
        // mouse-move during a drag, which was the source of the floaty lag.
        const computeRectSync = () => {
            const r = el.getBoundingClientRect();
            const dpr = window.devicePixelRatio || 1;
            const outer = outerPosRef.current;
            return {
                x: outer.x + r.left * dpr,
                y: outer.y + r.top * dpr,
                width: r.width * dpr,
                height: r.height * dpr,
            };
        };

        const reposition = () => {
            if (!mountedKeyRef.current) return;
            const rect = computeRectSync();
            if (rect.width < 1 || rect.height < 1) return;
            embedPosition(mountedKeyRef.current, rect.x, rect.y, rect.width, rect.height)
                .catch(() => {});
        };

        const onMovedEvent = (e) => {
            // Update cache and reposition in one synchronous step — no IPC.
            const p = e?.payload;
            if (p && typeof p.x === 'number') outerPosRef.current = { x: p.x, y: p.y };
            reposition();
        };
        const onResizedEvent = () => {
            // Resize doesn't change outer position; reposition uses current cache.
            reposition();
        };
        const onLayoutChange = () => reposition();

        setWaiting(true);
        const init = async () => {
            try {
                if (!winApiRef.current) {
                    winApiRef.current = await import('@tauri-apps/api/window');
                }
                if (cancelled) return;
                // (Re)attach listeners. If a previous instance set them up,
                // tear those down first so we don't double-fire.
                if (cleanupListenersRef.current) {
                    cleanupListenersRef.current();
                    cleanupListenersRef.current = null;
                }
                const w = winApiRef.current.getCurrentWindow();
                // Seed the outer-position cache once so the first reposition
                // after mount uses the real position (otherwise it's 0,0).
                try {
                    const p = await w.outerPosition();
                    outerPosRef.current = { x: p.x, y: p.y };
                } catch {}
                const unlistenMoved = await w.onMoved(onMovedEvent);
                const unlistenResized = await w.onResized(onResizedEvent);
                window.addEventListener('resize', onLayoutChange);
                window.addEventListener('scroll', onLayoutChange, true);
                cleanupListenersRef.current = () => {
                    unlistenMoved();
                    unlistenResized();
                    window.removeEventListener('resize', onLayoutChange);
                    window.removeEventListener('scroll', onLayoutChange, true);
                };

                await new Promise((r) => requestAnimationFrame(r));
                if (cancelled) return;

                const rect = computeRectSync();
                if (rect.width < 1 || rect.height < 1) {
                    setError('chat pane has no size');
                    return;
                }
                const ok = await embedMount(channelKey, rect.x, rect.y, rect.width, rect.height);
                if (cancelled) return;
                if (ok) {
                    mountedKeyRef.current = channelKey;
                    setError(null);
                    [200, 600, 1200].forEach((ms) => setTimeout(reposition, ms));
                } else {
                    setError(placeholderText ?? 'Channel offline.');
                }
            } catch (e) {
                if (!cancelled) setError(String(e?.message ?? e));
            } finally {
                if (!cancelled) setWaiting(false);
            }
        };
        init();

        const ro = new ResizeObserver(onLayoutChange);
        ro.observe(el);
        for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
            ro.observe(p);
        }

        return () => {
            cancelled = true;
            ro.disconnect();
            // Note: NOT calling embedUnmount here. Effect B handles that
            // on true component unmount only — channel switches just call
            // embedMount again, which Rust resolves to navigate().
        };
    }, [channelKey, isLive, placeholderText]);

    // Effect B: true unmount cleanup. Only runs when the component is
    // removed from the tree (e.g. user switches to a Twitch channel and
    // ChatView no longer renders EmbeddedChat).
    useEffect(() => {
        return () => {
            if (cleanupListenersRef.current) {
                cleanupListenersRef.current();
                cleanupListenersRef.current = null;
            }
            if (mountedKeyRef.current) {
                embedUnmount(mountedKeyRef.current).catch(() => {});
                mountedKeyRef.current = null;
            }
        };
    }, []);

    return (
        <div
            ref={placeholderRef}
            style={{
                width: '100%',
                height: '100%',
                position: 'relative',
                overflow: 'hidden',
            }}
        >
            {!isLive ? (
                <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                    {placeholderText ?? 'Channel offline.'}
                </div>
            ) : !inTauri ? (
                <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                    Embedded chat is only available in the desktop app.
                </div>
            ) : waiting ? (
                <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                    Loading chat…
                </div>
            ) : error ? (
                <div style={{ padding: 16, color: '#f87171', fontSize: 'var(--t-11)' }}>{error}</div>
            ) : null}
        </div>
    );
}
