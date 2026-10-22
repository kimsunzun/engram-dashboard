// TRD S21-slot-docking §3-9 「웹」: 이 창 웹뷰의 대기 모드 감시. 진입점 = `watchSpareMode` — 창마다 하나 건다. 예열 창 풀의
//   창(숨은 예열 창)은 대기 모드(빈 틀)로, 나머지는 보통으로 그린다.
// 듣기는 `target` = 자기 label 로 건다 — 몸에 창 label 이 없고, 기본 타깃 `Any` 로 걸면 남의 창으로 보낸 사건까지 받는다
//   (`src/commands/viewCommandBridge.ts` 의 같은 조항). 듣기를 건 뒤에 `window_role` 을 묻는다 — 그 사이에 온 사건을 놓치지 않게.
// 모드는 한 방향(대기 → 보통)이다 — `window_role` 답과 `spare:adopted` 는 다른 길로 와 차례가 없다. 늦게 온 답 `spare` 가
//   먼저 본 채택을 덮으면 채택된 창이 빈 틀로 남는다.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { SpareAdopted } from '../../../src-tauri/bindings/SpareAdopted'
import type { WindowRoleReply } from '../../../src-tauri/bindings/WindowRoleReply'

/** `spare` = 대기 모드(탭바 + 빈 캔버스 · 0탭 자가닫힘 끔 · 탭 끌어오기 없음 · 재기 보고는 그대로) · `normal` = 보통. */
export type WindowMode = 'spare' | 'normal'

/**
 * 모드 알림. `generation` = 이 페이지가 받은 채택 번호(`spare:adopted` 몸 · `window_role` 답 — I26) — 셸 첫 화면 문은 채택한
 * 창의 알림 중 이 번호를 실은 것만 받는다. `null` = 받지 않았다. `0` 도 번호다.
 */
export interface ModeChange {
  mode: WindowMode
  generation: number | null
}

/** 셸 `spare::wire::EVT_SPARE_ADOPTED` — 몸 = `SpareAdopted`. */
export const EVT_SPARE_ADOPTED = 'spare:adopted'
/** 셸 `commands::spare::window_role` — 인자 `{ adoptable }` = `spare:adopted` 듣기를 걸었다. */
export const CMD_WINDOW_ROLE = 'window_role'

/**
 * 이 창의 모드를 받기 시작한다 — `onMode` 로 알린다: `window_role` 답 하나, 그리고 `spare:adopted` 마다 `normal` + 그 번호.
 * 채택을 본 뒤에 온 `window_role` 답은 알리지 않는다(한 방향). `window_role` 이 실패하면 `normal` 이다(대기 모드 이전과 같은
 * 그리기). `spare:adopted` 듣기를 못 걸면 대기 모드로 들지 않는다 — `adoptable: false` 로 물어 셸이 풀의 창을 닫게 하고,
 * 답이 무엇이든 `normal` 이다(I28 — 듣기 없는 대기 창은 채택돼도 빈 채 남는다). 같은 값을 거듭 알릴 수 있다. 정리
 * 뒤에는 알리지도 묻지도 않는다.
 *
 * @returns 정리 함수 — 등록을 기다리지 않고 곧바로 돌려준다.
 */
// ADR-0295
// ADR-0299
export function watchSpareMode(onMode: (change: ModeChange) => void): () => void {
  let disposed = false
  let adopted = false
  let unlisten: UnlistenFn | null = null
  const label = getCurrentWindow().label

  void (async () => {
    let listening = false
    try {
      const off = await listen<SpareAdopted>(
        EVT_SPARE_ADOPTED,
        e => {
          if (disposed) return
          adopted = true
          onMode({ mode: 'normal', generation: generationOf(e.payload) })
        },
        { target: label },
      )
      if (disposed) {
        off()
        return
      }
      unlisten = off
      listening = true
    } catch (err) {
      console.error(`[spareMode] ${EVT_SPARE_ADOPTED} 구독 실패 — 대기 모드로 들지 않는다(adoptable: false):`, err)
    }
    // 정리된 감시는 묻지 않는다 — `adoptable: false` 로 물으면 StrictMode 의 두 번째 감시가 듣기를 걸었어도 셸이 그 창을 닫는다.
    if (disposed) return
    let change: ModeChange = { mode: 'normal', generation: null }
    try {
      const reply = await invoke<WindowRoleReply>(CMD_WINDOW_ROLE, { adoptable: listening })
      if (reply.role === 'spare') {
        if (listening) change = { mode: 'spare', generation: null }
      } else if (reply.role === 'model') {
        change = { mode: 'normal', generation: generationOf(reply) }
      }
    } catch (err) {
      console.warn(`[spareMode] ${CMD_WINDOW_ROLE} 실패 — 보통으로 그린다:`, err)
    }
    if (!disposed && !adopted) onMode(change)
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = null
  }
}

// 몸은 다른 프로세스가 준 값이라 모양을 믿지 않는다 — 깨진 몸에 처리기가 던지면 채택된 창이 빈 틀로 남는다. 번호만 잃으면
//   셸 시한에 보인다.
function generationOf(body: unknown): number | null {
  const generation = (body as { generation?: unknown } | null)?.generation
  return typeof generation === 'number' ? generation : null
}
