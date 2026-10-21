/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The UI is served by `openstance serve` from the same origin as the API.
// In development, Vite proxies the API paths to the local server so the
// browser stays same-origin (no CORS) and the `Host` header the server sees is
// 127.0.0.1:7411 (`changeOrigin`), which passes its DNS-rebinding check.
const API_TARGET = process.env.OPENSTANCE_API ?? 'http://127.0.0.1:7411';
const API_PATHS = [
  '/health',
  '/people',
  '/search',
  '/connections',
  '/playbooks',
  '/quote',
  '/edits',
  '/review',
  '/views',
  '/import',
  '/sources',
  '/enrich-log',
  '/config',
  '/stats',
  '/shortlist',
  '/intros',
  '/mcp',
];

export default defineConfig({
  plugins: [react()],
  base: './',
  server: {
    port: 5173,
    strictPort: true,
    proxy: Object.fromEntries(API_PATHS.map((p) => [p, { target: API_TARGET, changeOrigin: true }])),
  },
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
    sourcemap: false,
    // No inline scripts: the server sends `Content-Security-Policy: default-src 'self'`.
    modulePreload: { polyfill: false },
    assetsInlineLimit: 0,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
    restoreMocks: true,
  },
});
