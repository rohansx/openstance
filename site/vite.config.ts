/// <reference types="vitest/config" />
import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';

// The production page loads nothing from another origin. The policy is added only to the
// build because the dev server injects an inline script for React Fast Refresh.
const CSP =
  "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; " +
  "font-src 'self'; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'";

function contentSecurityPolicy(): Plugin {
  return {
    name: 'openstance-csp',
    apply: 'build',
    transformIndexHtml(html) {
      return html.replace(
        '<meta charset="utf-8" />',
        `<meta charset="utf-8" />\n    <meta http-equiv="Content-Security-Policy" content="${CSP}" />`,
      );
    },
  };
}

export default defineConfig({
  base: './',
  plugins: [react(), contentSecurityPolicy()],
  build: {
    // Keep the favicon and other small assets as files so the CSP needs no data: for scripts.
    assetsInlineLimit: 0,
    sourcemap: false,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
  },
});
