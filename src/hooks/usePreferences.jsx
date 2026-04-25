import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { getSettings, updateSettings } from '../ipc.js';

/**
 * Shared preferences state, lifted into a React Context so every component —
 * preferences dialog, chat view's badge/timestamp gates, future consumers —
 * sees the same snapshot. Without this, each `usePreferences()` call kept its
 * own state and a setting flipped in the dialog stayed invisible to the chat
 * view until a manual reload.
 */
const PreferencesContext = createContext(null);

export function PreferencesProvider({ children }) {
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

  return (
    <PreferencesContext.Provider value={{ settings, error, patch }}>
      {children}
    </PreferencesContext.Provider>
  );
}

export function usePreferences() {
  const ctx = useContext(PreferencesContext);
  if (!ctx) {
    throw new Error('usePreferences must be used inside <PreferencesProvider>');
  }
  return ctx;
}
