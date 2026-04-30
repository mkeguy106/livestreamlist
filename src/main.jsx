import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.jsx';
import DetachedChatRoot from './components/DetachedChatRoot.jsx';
import LoginPopupRoot from './components/LoginPopupRoot.jsx';
import { AuthProvider } from './hooks/useAuth.jsx';
import { PreferencesProvider } from './hooks/usePreferences.jsx';
import './tokens.css';

// Window-routing decision.
//
// 1. Login-popup: detected via Tauri label (set by login_popup.rs) with a
//    query-string fallback for browser-dev preview.
// 2. Chat-detach: detected via URL hash #chat-detach=<encoded unique_key>.
//    Used by chat_detach IPC; carries the channel key in the hash so the
//    React bundle can mount DetachedChatRoot for that specific channel.
// 3. Default: main App.
const isLoginPopup = (() => {
  const tauriLabel = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (tauriLabel === 'login-popup') return true;
  return new URLSearchParams(window.location.search).get('popup') === 'login';
})();

const detachedChannelKey = (() => {
  const hash = window.location.hash || '';
  const prefix = '#chat-detach=';
  if (hash.startsWith(prefix)) {
    try {
      return decodeURIComponent(hash.slice(prefix.length));
    } catch {
      return null;
    }
  }
  return null;
})();

const rootEl = document.getElementById('root');
const root = ReactDOM.createRoot(rootEl);

let content;
if (isLoginPopup) {
  content = <LoginPopupRoot />;
} else if (detachedChannelKey) {
  content = <DetachedChatRoot channelKey={detachedChannelKey} />;
} else {
  content = <App />;
}

root.render(
  <React.StrictMode>
    <PreferencesProvider>
      <AuthProvider>
        {content}
      </AuthProvider>
    </PreferencesProvider>
  </React.StrictMode>,
);
