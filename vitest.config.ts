// vitest 설정 — 프론트 로직 단위테스트(testing-strategy HIGH 갭 #1).
// 코로케이션 컨벤션: 테스트는 소스 옆 *.test.ts. 환경은 jsdom(clientFactory 가
// window/localStorage 를, daemonClient 가 globalThis.WebSocket/crypto 를 사용).
// vite.config.ts(앱 빌드 설정)와 분리 — 빌드 파이프라인(tsc && vite build)에 영향 0.
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    globals: false,
  },
})
