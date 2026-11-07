// vitest config — frontend logic unit tests (testing-strategy HIGH gap #1).
// Colocation convention: tests are *.test.ts / *.test.tsx next to the source. The environment is jsdom (clientFactory uses
// window/localStorage, daemonClient uses globalThis.WebSocket/crypto).
// Separate from vite.config.ts (the app build config) — zero effect on the build pipeline (tsc && vite build).
// *.test.tsx added: render-branch tests of React components such as ViewLayoutRenderer (@testing-library/react).
import { fileURLToPath, URL } from 'node:url'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [react()],
  // ADR-0047: the same `@/*` alias applied to tests too (paired with vite.config.ts — it is a separate config so redeclared).
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    environment: 'jsdom',
    // The stub is turned off only for this file (index.css), because of the regression gate (index.css.test.ts) that reads the raw index.css (?raw) —
    // the default (css: false) replaces CSS imports with an empty string and that gate passes silently (measured).
    // The rest of the CSS (chat.css etc.) is stubbed as is, so existing test behavior is unchanged.
    css: { include: [/index\.css/] },
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
    globals: false,
  },
})
