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
        layer.register(channelKey, slotIdRef.current, ref, active);
        return () => {
            if (slotIdRef.current !== null) {
                layer.unregister(channelKey, slotIdRef.current);
            }
        };
        // The ref is stable; channelKey + isLive + active are the deps.
    }, [channelKey, isLive, active, layer]);

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
