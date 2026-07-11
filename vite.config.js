import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    // Pin the family: bare `localhost` lets Node bind ::1-only on some
    // resolver orders while WebKit resolves the devUrl to 127.0.0.1 —
    // connection refused, black window (hit live during slice-B smoke).
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
  },
  clearScreen: false,
});
