import { useCallback, useEffect, useRef, useState } from 'react';
import { getSettings, updateSettings } from '../ipc.js';

/**
 * Central preferences state. Loads on mount, syncs writes to Rust with a
 * short debounce so rapid slider moves don't fsync the world.
 */
export function usePreferences() {
  const [settings, setSettings] = useState(null);
  const [error, setError] = useState(null);
  const timer = useRef(null);

  useEffect(() => {
    getSettings()
      .then(setSettings)
      .catch((e) => setError(String(e?.message ?? e)));
  }, []);

  const patch = useCallback((updater) => {
    setSettings((prev) => {
      if (!prev) return prev;
      const next = typeof updater === 'function' ? updater(prev) : { ...prev, ...updater };
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        updateSettings(next).catch((e) => setError(String(e?.message ?? e)));
      }, 200);
      return next;
    });
  }, []);

  return { settings, error, patch };
}
