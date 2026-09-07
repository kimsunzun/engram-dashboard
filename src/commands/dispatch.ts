// ADR-0055 ★두 실행 경로의 분리★:
//   - fireAndForget(id, args)  ← 사람 UI 클릭·키바인딩(FIX-4): 결과 무관, 실패는 warn 으로만. 삼킨다.
//   - run(id, args)            ← cdp/__engramCmd/await 호출부: 반환·throw 를 그대로 노출(await·에러 관찰).
//   click/keybinding 소비자는 반드시 이 helper 를 재사용한다(안전망 재구현 금지 — 이 파일이 복사 템플릿).

import { runAsHuman } from './registry'
import type { CommandArgs } from './registry'

/**
 * click/keybinding 등 결과를 await 하지 않는 소비자 전용.
 * - 동기 throw: try/catch 로 잡아 warn(리스너가 죽지 않게).
 * - 비동기 reject: Promise.resolve(result).catch 로 삼킨다 — instanceof Promise 로 좁히지 않아
 *   thenable/cross-realm Promise 도 커버한다(FIX-3). run() 자체는 손대지 않아 await 호출부는 무손실.
 */
export function fireAndForget(id: string, args?: CommandArgs): void {
  try {
    // ★`registry.run` 이 아니라 `runAsHuman` 이다★: 이 경로의 호출자는 사람 클릭·키바인딩이라
    //   `humanOnly` 게이트를 지나지 않는다(그 칸의 doc 이 정본). 여기를 `run` 으로 되돌리면 사람이
    //   트리 메뉴에서 codex 를 만드는 길이 막힌다 — 게이트가 겨냥한 것은 LLM 경로뿐이다.
    const result = runAsHuman(id, args)
    Promise.resolve(result).catch((err) => console.warn(`[commands] '${id}' 실패(async):`, err))
  } catch (err) {
    console.warn(`[commands] '${id}' 실패:`, err)
  }
}
