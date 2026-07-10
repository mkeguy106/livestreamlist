import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { embedMount, embedBounds, embedSetVisible, embedUnmount,
         mpvMount, mpvBounds, mpvSetVisible, mpvUnmount } from '../ipc.js';

export const EmbedLayerContext = createContext(null);

/**
 * Global registry that arbitrates which <EmbedSlot> is canonical per
 * EmbedKey and dispatches embed_* IPC accordingly.
 *
 * One slot per key may be active at a time. Multiple slots with the
 * same key are allowed (e.g., future chat-tabs may register the same
 * channel in multiple tab contexts) only if at most one is `active`.
 *
 * The active slot's getBoundingClientRect() is the canonical rect we
 * report to Rust via embed_bounds. When no slot is active for a key,
 * the embed is hidden via embed_set_visible(key, false). When the last
 * slot for a key unregisters, embed_unmount(key) tears the embed down.
 *
 * `modalOpen` is App-level state — when true, every active embed is
 * hidden via embed_set_visible(key, false). This replaces the old
 * embedSetVisibleAll(false) singleton from before Phase 7.
 */
export default function EmbedLayer({ children, modalOpen }) {
    // In-React popups (dropdowns, pickers, confirms) that render over embed
    // regions are otherwise hidden behind the NATIVE child webviews (embeds
    // live above the React surface in the GTK overlay). Any such popup
    // ref-counts itself here via useEmbedOcclusion(open); embeds hide while
    // the count is non-zero, same as App-level modals.
    const [overlayCount, setOverlayCount] = useState(0);
    const hidden = modalOpen || overlayCount > 0;
    // key → { refs: Map<slotId, { ref, active }> }
    const registry = useRef(new Map());
    // mounted keys (in Rust) — used for cleanup
    const mountedKeys = useRef(new Set());
    // Per-key occlusion (mpv hover-controls): a key in this set has its
    // native surface hidden while its DOM controls are interacted with —
    // composes with the global modal/overlay `hidden`.
    const occludedKeys = useRef(new Set());
    // mpv mount lifecycle: failed keys don't remount on every ResizeObserver
    // reflow (remountKey clears); mounting keys don't double-mount while the
    // async mpv_mount (streamlink startup — seconds) is in flight.
    const failedKeys = useRef(new Set());
    const mountingKeys = useRef(new Set());
    // Latest `hidden` for async .then handlers — reflowKey's closure goes stale
    // across the multi-second mpv mount.
    const hiddenRef = useRef(hidden);
    useEffect(() => { hiddenRef.current = hidden; }, [hidden]);
    // Keys whose remount was requested while their mount was still in flight.
    const pendingRemounts = useRef(new Set());

    const reflowKey = useCallback((key) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        const backend = entry.backend ?? 'webview';
        const setVis = backend === 'mpv' ? mpvSetVisible : embedSetVisible;
        const active = [...entry.refs.values()].find((s) => s.active);
        if (!active || !active.ref.current) {
            if (mountedKeys.current.has(key)) {
                setVis(key, false).catch(() => {});
            }
            return;
        }
        const r = active.ref.current.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        const x = r.left * dpr;
        const y = r.top * dpr;
        const w = Math.max(1, r.width) * dpr;
        const h = Math.max(1, r.height) * dpr;
        const shown = !hidden && !occludedKeys.current.has(key);

        if (!mountedKeys.current.has(key)) {
            if (backend === 'mpv') {
                if (failedKeys.current.has(key) || mountingKeys.current.has(key)) return;
                const args = entry.getMountArgs?.() ?? {};
                mountingKeys.current.add(key);
                mpvMount(key, x, y, w, h, args.quality ?? null, !!args.muted, args.volume ?? 0.5)
                    .then((ok) => {
                        if (!ok) return;
                        // The slot unregistered while the mount was in flight — tear the
                        // fresh native surface down instead of leaking mpv+streamlink.
                        if (!registry.current.has(key)) {
                            mpvUnmount(key).catch(() => {});
                            return;
                        }
                        mountedKeys.current.add(key);
                        // Recompute visibility NOW — hidden/occluded may have changed
                        // during the mount (stale-closure fix).
                        const shownNow = !hiddenRef.current && !occludedKeys.current.has(key);
                        if (!shownNow) mpvSetVisible(key, false).catch(() => {});
                    })
                    .catch(() => { if (registry.current.has(key)) failedKeys.current.add(key); })
                    .finally(() => {
                        mountingKeys.current.delete(key);
                        // A remount (quality switch) arrived mid-mount: run it now that the
                        // original mount has settled.
                        if (pendingRemounts.current.delete(key)) {
                            if (mountedKeys.current.has(key)) {
                                mountedKeys.current.delete(key);
                                mpvUnmount(key).then(() => reflowKey(key)).catch(() => reflowKey(key));
                            } else {
                                reflowKey(key);
                            }
                        }
                    });
            } else {
                embedMount(key, x, y, w, h).then((ok) => {
                    if (ok) {
                        mountedKeys.current.add(key);
                        const shownNow = !hiddenRef.current && !occludedKeys.current.has(key);
                        if (!shownNow) embedSetVisible(key, false).catch(() => {});
                    }
                }).catch(() => {});
            }
        } else {
            (backend === 'mpv' ? mpvBounds : embedBounds)(key, x, y, w, h).catch(() => {});
            setVis(key, shown).catch(() => {});
        }
    }, [hidden]);

    const register = useCallback((key, slotId, ref, active, opts = {}) => {
        let entry = registry.current.get(key);
        if (!entry) {
            entry = {
                refs: new Map(),
                backend: opts.backend ?? 'webview',
                getMountArgs: opts.getMountArgs,
            };
            registry.current.set(key, entry);
        }
        entry.refs.set(slotId, { ref, active });
        // Defer to next frame so the placeholder's bounding rect is real.
        requestAnimationFrame(() => reflowKey(key));
    }, [reflowKey]);

    const unregister = useCallback((key, slotId) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        entry.refs.delete(slotId);
        if (entry.refs.size === 0) {
            const backend = entry.backend ?? 'webview';
            registry.current.delete(key);
            occludedKeys.current.delete(key);
            failedKeys.current.delete(key);
            pendingRemounts.current.delete(key);
            if (mountedKeys.current.has(key)) {
                (backend === 'mpv' ? mpvUnmount : embedUnmount)(key).catch(() => {});
                mountedKeys.current.delete(key);
            }
        } else {
            reflowKey(key);
        }
    }, [reflowKey]);

    const updateActive = useCallback((key, slotId, active) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        const slot = entry.refs.get(slotId);
        if (!slot || slot.active === active) return;
        slot.active = active;
        reflowKey(key);
    }, [reflowKey]);

    // Reflow all keys on viewport changes
    useEffect(() => {
        const onResize = () => {
            for (const key of registry.current.keys()) reflowKey(key);
        };
        window.addEventListener('resize', onResize);
        return () => window.removeEventListener('resize', onResize);
    }, [reflowKey]);

    // Re-apply visibility when modal/overlay state toggles
    useEffect(() => {
        for (const key of mountedKeys.current) {
            const backend = registry.current.get(key)?.backend ?? 'webview';
            const shown = !hidden && !occludedKeys.current.has(key);
            (backend === 'mpv' ? mpvSetVisible : embedSetVisible)(key, shown).catch(() => {});
        }
    }, [hidden]);

    const pushOverlay = useCallback(() => {
        setOverlayCount((c) => c + 1);
        return () => setOverlayCount((c) => Math.max(0, c - 1));
    }, []);

    // Hide/show ONE key's native surface (mpv hover-controls occlusion).
    const occludeKey = useCallback((key, occluded) => {
        if (occluded) occludedKeys.current.add(key);
        else occludedKeys.current.delete(key);
        reflowKey(key); // re-applies bounds + composed visibility
    }, [reflowKey]);

    // Kill + respawn with fresh getMountArgs (mpv quality switch, and the
    // panel's Retry — after a Rust-side death the client-side mountedKeys is
    // stale-true, so a plain reflow would take the "already mounted" branch
    // and no-op; the unmount-first path here is a safe no-op Rust-side and
    // yields a genuine fresh mount). Also clears the failed flag — a failed
    // mount otherwise stays failed so ResizeObserver ticks don't re-spawn a
    // doomed mount.
    const remountKey = useCallback((key) => {
        const entry = registry.current.get(key);
        if (!entry || (entry.backend ?? 'webview') !== 'mpv') return;
        failedKeys.current.delete(key);
        if (mountingKeys.current.has(key)) {
            pendingRemounts.current.add(key); // picked up in the mount's .finally
            return;
        }
        if (mountedKeys.current.has(key)) {
            mountedKeys.current.delete(key);
            mpvUnmount(key).then(() => reflowKey(key)).catch(() => reflowKey(key));
        } else {
            reflowKey(key);
        }
    }, [reflowKey]);

    const ctx = useMemo(() => ({
        register, unregister, updateActive, reflowKey, pushOverlay,
        occludeKey, remountKey,
    }), [register, unregister, updateActive, reflowKey, pushOverlay,
        occludeKey, remountKey]);

    return (
        <EmbedLayerContext.Provider value={ctx}>
            {children}
        </EmbedLayerContext.Provider>
    );
}


/**
 * Hide native embeds while an in-React popup is open. Ref-counted, so
 * nested/concurrent popups compose; the pop runs on close or unmount.
 * No-ops when no EmbedLayer is mounted (e.g. isolated component tests).
 */
export function useEmbedOcclusion(open) {
    const layer = useContext(EmbedLayerContext);
    useEffect(() => {
        if (!open || !layer?.pushOverlay) return undefined;
        return layer.pushOverlay();
    }, [open, layer]);
}
