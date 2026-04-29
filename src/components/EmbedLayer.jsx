import { createContext, useCallback, useEffect, useMemo, useRef } from 'react';
import { embedMount, embedBounds, embedSetVisible, embedUnmount } from '../ipc.js';

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
    // key → { refs: Map<slotId, { ref, active }> }
    const registry = useRef(new Map());
    // mounted keys (in Rust) — used for cleanup
    const mountedKeys = useRef(new Set());

    const reflowKey = useCallback((key) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        const active = [...entry.refs.values()].find((s) => s.active);
        if (!active || !active.ref.current) {
            // No active slot for this key — hide if mounted
            if (mountedKeys.current.has(key)) {
                embedSetVisible(key, false).catch(() => {});
            }
            return;
        }
        const r = active.ref.current.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        const x = r.left * dpr;
        const y = r.top * dpr;
        const w = Math.max(1, r.width) * dpr;
        const h = Math.max(1, r.height) * dpr;

        if (!mountedKeys.current.has(key)) {
            embedMount(key, x, y, w, h).then((ok) => {
                if (ok) {
                    mountedKeys.current.add(key);
                    if (modalOpen) embedSetVisible(key, false).catch(() => {});
                }
            }).catch(() => {});
        } else {
            embedBounds(key, x, y, w, h).catch(() => {});
            embedSetVisible(key, !modalOpen).catch(() => {});
        }
    }, [modalOpen]);

    const register = useCallback((key, slotId, ref, active) => {
        let entry = registry.current.get(key);
        if (!entry) {
            entry = { refs: new Map() };
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
            registry.current.delete(key);
            if (mountedKeys.current.has(key)) {
                embedUnmount(key).catch(() => {});
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

    // Re-apply visibility when modalOpen toggles
    useEffect(() => {
        for (const key of mountedKeys.current) {
            embedSetVisible(key, !modalOpen).catch(() => {});
        }
    }, [modalOpen]);

    const ctx = useMemo(() => ({
        register, unregister, updateActive, reflowKey,
    }), [register, unregister, updateActive, reflowKey]);

    return (
        <EmbedLayerContext.Provider value={ctx}>
            {children}
        </EmbedLayerContext.Provider>
    );
}
