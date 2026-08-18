// vitest 설정 — 프론트 로직 단위테스트(testing-strategy HIGH 갭 #1).
// 코로케이션 컨벤션: 테스트는 소스 옆 *.test.ts / *.test.tsx. 환경은 jsdom(clientFactory 가
// window/localStorage 를, daemonClient 가 globalThis.WebSocket/crypto 를 사용).
// vite.config.ts(앱 빌드 설정)와 분리 — 빌드 파이프라인(tsc && vite build)에 영향 0.
// *.test.tsx 추가: ViewLayoutRenderer 등 React 컴포넌트 렌더 분기 테스트(@testing-library/react).
import { fileURLToPath, URL } from 'node:url'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [react()],
  // ADR-0047: `@/*` 별칭을 테스트에도 동일 적용(vite.config.ts 와 짝 — 분리 config 라 재선언).
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    environment: 'jsdom',
    // index.css 원문(?raw)을 읽는 회귀 게이트(index.css.test.ts) 때문에 이 파일만 stub 을 끈다 —
    // 기본값(css: false)은 CSS import 를 빈 문자열로 갈아 그 게이트가 조용히 통과해 버린다(실측).
    // 나머지 CSS(chat.css 등)는 그대로 stub 이라 기존 테스트 동작은 불변.
    css: { include: [/index\.css/] },
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
    globals: false,
  },
})
