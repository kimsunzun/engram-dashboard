// ADR-0055: fire-and-forget dispatch — 사람 클릭·전역 keydown 처럼 "결과를 기다리지 않는" 소비자용
//   안전 실행 경로다. run(id, args) 을 try/catch 로 감싸고, 반환이 Promise/thenable 이면 rejection 을
//   삼켜(unhandled rejection 방지) warn 만 남긴다.
//
// ★두 실행 경로의 분리★:
//   - fireAndForget(id, args)  ← 사람 UI 클릭·키바인딩(FIX-4): 결과 무관, 실패는 warn 으로만. 삼킨다.
//   - run(id, args)            ← cdp/__engramCmd/await 호출부: 반환·throw 를 그대로 노출(await·에러 관찰).
//   click/keybinding 소비자는 반드시 이 helper 를 재사용한다(안전망 재구현 금지 — 이 파일이 복사 템플릿).

import { run } from './registry'
import type { CommandArgs } from './registry'

/**
 * 안전한 fire-and-forget 실행. click/keybinding 등 결과를 await 하지 않는 소비자 전용.
 * - 동기 throw: try/catch 로 잡아 warn(리스너가 죽지 않게).
 * - 비동기 reject: Promise.resolve(result).catch 로 삼킨다 — instanceof Promise 로 좁히지 않아
 *   thenable/cross-realm Promise 도 커버한다(FIX-3). run() 자체는 손대지 않아 await 호출부는 무손실.
 */
export function fireAndForget(id: string, args?: CommandArgs): void {
  try {
    const result = run(id, args)
    // Promise.resolve 로 감싸 어떤 thenable 도 정규화 → reject 를 삼킨다(unhandled rejection 방지).
    Promise.resolve(result).catch((err) => console.warn(`[commands] '${id}' 실패(async):`, err))
  } catch (err) {
    console.warn(`[commands] '${id}' 실패:`, err)
  }
}
