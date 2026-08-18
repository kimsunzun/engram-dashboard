import { fileURLToPath, URL } from "node:url";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],
  // ADR-0047: `@/*` → src/* 경로 별칭(shadcn 관례). tsconfig paths 와 짝.
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
    // 감시 제외 = **프론트 소스가 없는 경로**들. 개발 서버 전용 설정이라 릴리스 빌드엔 로드조차 안 된다.
    // ★`target/**`·`crates/**` 를 빼는 이유(실측 2026-08-17)★: dev 앱을 띄워 둔 채 러스트를 빌드·테스트하면
    //   `target/` 아래에 산출물이 쏟아지는데, 그게 감시에 걸려 **창 전체가 주기적으로 리로드**된다 —
    //   화면을 보며 실측하는 동안 계속 갱신돼 관찰이 불가능해진다.
    // `.engram-data/**` = 디버그 빌드의 데이터 폴더(저장소 루트 안이다 — ADR-0136). 데몬이 로그·명부를 계속 쓴다.
    watch: {
      ignored: [
        "**/src-tauri/**",
        "**/target/**",
        "**/crates/**",
        "**/.engram-data/**",
      ],
    },
  },
}));
