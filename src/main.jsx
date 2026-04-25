import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.jsx';
import { AuthProvider } from './hooks/useAuth.jsx';
import { PreferencesProvider } from './hooks/usePreferences.jsx';
import './tokens.css';

ReactDOM.createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <PreferencesProvider>
      <AuthProvider>
        <App />
      </AuthProvider>
    </PreferencesProvider>
  </React.StrictMode>,
);
