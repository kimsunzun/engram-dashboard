// 셸이 이 창 웹뷰에 보내는 끌기 세션 사건의 받는 쪽(TRD S21-slot-docking §3 · §4). 지금은 창 모드 하나다 — 대기 창
//   명부의 창(예비 창 · 끄는 중 비워 남긴 창)은 대기 모드(빈 틀)로, 나머지는 보통으로 그린다(§2-5).

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { SpareEvent } from '../../../../src-tauri/bindings/SpareEvent'
import type { WindowRoleReply } from '../../../../src-tauri/bindings/WindowRoleReply'

/** `parked` = 대기 모드(탭바 + 빈 캔버스 · 0탭 자가닫힘 끔 · 탭 끌어오기 없음 · 재기 보고는 그대로) · `normal` = 보통. */
export type WindowMode = 'parked' | 'normal'

export const EVT_SPARE = 'dock:spare'
export const CMD_WINDOW_ROLE = 'window_role'

/**
 * 이 창의 모드를 정해 `onMode` 로 알리고, 바뀔 때마다 다시 알린다 — 첫 값 = 마운트 때의 `window_role` 답, 그 뒤 =
 * `dock:spare`. 같은 값을 거듭 알릴 수 있다.
 *
 * `dock:spare` 를 먼저 건 뒤 `window_role` 을 묻는다 — 그 사이에 온 바뀜을 놓치지 않게. `window_role` 이 실패하면
 * `normal` 이다(대기 모드 이전과 같은 그리기). 정리 뒤에는 알리지 않는다.
 *
 * @returns 정리 함수 — 등록을 기다리지 않고 곧바로 돌려준다.
 */
// ADR-0295
export function watchWindowMode(onMode: (mode: WindowMode) => void): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined

  void (async () => {
    try {
      // ★`target` = 자기 label★ — 몸에 창 label 이 없다. 기본 타깃 `Any` 로 걸면 남의 창으로 보낸 사건까지 받는다
      //   (`src/commands/viewCommandBridge.ts` 의 같은 조항).
      const off = await listen<SpareEvent>(
        EVT_SPARE,
        e => {
          if (disposed) return
          onMode(e.payload.step === 'parked' ? 'parked' : 'normal')
        },
        { target: getCurrentWindow().label },
      )
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (err) {
      console.error(`[dockSession] ${EVT_SPARE} 구독 실패 — 이 창은 대기 모드 바뀜을 못 받는다:`, err)
    }
    if (disposed) return
    let mode: WindowMode = 'normal'
    try {
      const reply = await invoke<WindowRoleReply>(CMD_WINDOW_ROLE)
      if (reply.role === 'parked') mode = 'parked'
    } catch (err) {
      console.warn(`[dockSession] ${CMD_WINDOW_ROLE} 실패 — 보통으로 그린다:`, err)
    }
    if (!disposed) onMode(mode)
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = undefined
  }
}
