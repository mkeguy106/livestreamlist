import { useEffect, useState } from 'react';
import { videoBackend } from '../ipc.js';

// Resolved once per app run (the answer is a compile-time constant on the
// Rust side); cached module-wide so every VideoPanel shares one IPC call.
let backendPromise = null;

/** 'mpv' | 'mpegts' | null while resolving. */
export function useVideoBackend() {
  const [backend, setBackend] = useState(null);
  useEffect(() => {
    backendPromise ??= videoBackend();
    let on = true;
    backendPromise
      .then((b) => { if (on) setBackend(b === 'mpv' ? 'mpv' : 'mpegts'); })
      .catch(() => {
        // Don't poison the cache permanently on a transient rejection — a
        // later VideoPanel mount retries the probe. THIS mount still falls
        // back to mpegts.
        backendPromise = null;
        if (on) setBackend('mpegts');
      });
    return () => { on = false; };
  }, []);
  return backend;
}
