import { useContext, useEffect, useRef } from 'react';
import { EmbedLayerContext } from './EmbedLayer.jsx';

let nextSlotId = 1;
function generateSlotId() {
    return `slot-${nextSlotId++}`;
}

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * Reserves a rectangle for a chat embed. Reports its rect + active state
 * to the global EmbedLayer; the layer dispatches embed_* IPC.
 *
 * Outside Tauri (browser dev): renders a placeholder hint.
 */
export default function EmbedSlot({ channelKey, isLive, active, placeholderText }) {
    const ref = useRef(null);
    const slotIdRef = useRef(null);
    const layer = useContext(EmbedLayerContext);

    useEffect(() => {
        if (!layer || !inTauri) return;
        if (!isLive || !channelKey) return;
        if (slotIdRef.current === null) slotIdRef.current = generateSlotId();
        // Pass the CURRENT active value at register time — it's the initial
        // value the layer sees. After this, every change to `active` flows
        // through the separate updateActive effect below; we deliberately
        // do NOT re-register on active changes because that would unmount
        // the embed (the unregister path's `entry.refs.size === 0` branch
        // tears down the wry WebView entirely, which is exactly what
        // happened to YT/CB embeds on every chat-tab switch — the embed
        // would briefly become the only-and-zero slot and get destroyed,
        // then re-mount on re-register, producing a visible reload).
        layer.register(channelKey, slotIdRef.current, ref, active);
        return () => {
            if (slotIdRef.current !== null) {
                layer.unregister(channelKey, slotIdRef.current);
            }
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps -- `active` flows through updateActive
    }, [channelKey, isLive, layer]);

    useEffect(() => {
        if (!layer || !inTauri) return;
        if (slotIdRef.current === null) return;
        layer.updateActive(channelKey, slotIdRef.current, active);
    }, [active, channelKey, layer]);

    // Resize observer chain — observes the placeholder + every ancestor
    // up to body, so any layout shift triggers a reflow.
    useEffect(() => {
        if (!layer || !inTauri) return;
        const el = ref.current;
        if (!el) return;
        const ro = new ResizeObserver(() => layer.reflowKey(channelKey));
        ro.observe(el);
        for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
            ro.observe(p);
        }
        return () => ro.disconnect();
    }, [channelKey, layer]);

    return (
        <div
            ref={ref}
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
            ) : null}
        </div>
    );
}
