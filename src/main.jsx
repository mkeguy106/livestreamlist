import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.jsx';
import LoginPopupRoot from './components/LoginPopupRoot.jsx';
import { AuthProvider } from './hooks/useAuth.jsx';
import { PreferencesProvider } from './hooks/usePreferences.jsx';
import './tokens.css';

// The Rust login_popup module spawns a sibling WebviewWindow with label
// "login-popup". That window renders only the account dropdown — bypass
// App entirely so we don't double-mount channel polling / chat tasks in
// the popup process. Detection uses the window's Tauri label (robust
// against URL/route quirks) with a query-string fallback so a plain
// browser dev session at `/?popup=login` can still preview the UI.
const isLoginPopup = (() => {
  const tauriLabel = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (tauriLabel === 'login-popup') return true;
  return new URLSearchParams(window.location.search).get('popup') === 'login';
})();

ReactDOM.createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <PreferencesProvider>
      <AuthProvider>
        {isLoginPopup ? <LoginPopupRoot /> : <App />}
      </AuthProvider>
    </PreferencesProvider>
  </React.StrictMode>,
);
