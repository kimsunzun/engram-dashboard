import { fileURLToPath, URL } from "node:url";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],
  // ADR-0047: `@/*` → src/* path alias (shadcn convention). Paired with tsconfig paths.
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    // Watch exclusions = paths **that have no frontend source**. It is a dev-server-only setting so it is not even loaded in a release build.
    // ★Why `target/**` · `crates/**` are excluded (measured 2026-08-17)★: if you build/test Rust with the dev app up,
    //   artifacts pour out under `target/`, which trips the watcher and **the whole window reloads periodically** —
    //   it keeps refreshing while you measure by looking at the screen, so observation becomes impossible.
    // `.engram-dev/**` = the data folder of the debug build (inside the repository root — ADR-0136). The daemon keeps writing logs · the registry.
    // `.engram-data/**` = the development data folder of the old layout (ADR-0264 decision 5) — during the transition the old daemon may still write logs.
    watch: {
      ignored: [
        "**/src-tauri/**",
        "**/target/**",
        "**/crates/**",
        "**/.engram-dev/**",
        "**/.engram-data/**",
      ],
    },
  },
}));
