// TRD S21-slot-docking §3-9 「웹」: 이 창 웹뷰의 대기 모드 감시. 진입점 = `watchSpareMode` — 창마다 하나 건다. 예비 창 풀의
//   창(숨은 예비 창)은 대기 모드(빈 틀)로, 나머지는 보통으로 그린다.
// 듣기는 `target` = 자기 label 로 건다 — 몸에 창 label 이 없고, 기본 타깃 `Any` 로 걸면 남의 창으로 보낸 사건까지 받는다
//   (`src/commands/viewCommandBridge.ts` 의 같은 조항). 듣기를 건 뒤에 `window_role` 을 묻는다 — 그 사이에 온 사건을 놓치지 않게.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { WindowRoleReply } from '../../../src-tauri/bindings/WindowRoleReply'

/** `spare` = 대기 모드(탭바 + 빈 캔버스 · 0탭 자가닫힘 끔 · 탭 끌어오기 없음 · 재기 보고는 그대로) · `normal` = 보통. */
export type WindowMode = 'spare' | 'normal'

/** 셸 `spare::wire::EVT_SPARE_ADOPTED` — 몸 없음(`null`). */
export const EVT_SPARE_ADOPTED = 'spare:adopted'
export const CMD_WINDOW_ROLE = 'window_role'

/**
 * 이 창의 모드를 받기 시작한다 — `onMode` 로 알린다: 첫 값 = `window_role` 답, 그 뒤 = `spare:adopted` 의 `normal`. 같은 값을
 * 거듭 알릴 수 있다. `window_role` 이 실패하면 `normal` 이다(대기 모드 이전과 같은 그리기). 정리 뒤에는 알리지 않는다.
 *
 * @returns 정리 함수 — 등록을 기다리지 않고 곧바로 돌려준다.
 */
// ADR-0295
// ADR-0299
export function watchSpareMode(onMode: (mode: WindowMode) => void): () => void {
  let disposed = false
  let unlisten: UnlistenFn | null = null
  const label = getCurrentWindow().label

  void (async () => {
    try {
      const off = await listen<null>(
        EVT_SPARE_ADOPTED,
        () => {
          if (!disposed) onMode('normal')
        },
        { target: label },
      )
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (err) {
      console.error(`[spareMode] ${EVT_SPARE_ADOPTED} 구독 실패 — 이 창은 대기 모드 바뀜을 못 받는다:`, err)
    }
    if (disposed) return
    // TODO(TRD S21-slot-docking U4d′): 모드를 한 방향으로 — 지금은 늦게 온 답이 그 사이 받은 `spare:adopted` 를 덮는다.
    let mode: WindowMode = 'normal'
    try {
      const reply = await invoke<WindowRoleReply>(CMD_WINDOW_ROLE)
      if (reply.role === 'spare') mode = 'spare'
    } catch (err) {
      console.warn(`[spareMode] ${CMD_WINDOW_ROLE} 실패 — 보통으로 그린다:`, err)
    }
    if (!disposed) onMode(mode)
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = null
  }
}
