// TRD S21-slot-docking §3-1 · §4-2 · §4-6: 놓기 명령 `tab_drop` 의 감싸개와 결과 알림 `tab_drop:done` 듣기 — 끌기 하나에 명령은
//   끝난 뒤 많아야 둘(출처 dragend 의 `drag_end` · 받은 창의 `drop`)이고, 결과는 답이 아니라 출처 창의 `done` 으로 온다.
// ★invoke/listen 을 직접 부른다★: 놓기는 셸이 주인인 웹뷰 안 배선이라 agentClient 가 나를 것이 아니고, 버스 명령도 아니다(같은
//   동작의 LLM 명령 = T-65 `tab.dock` · TRD §3-6). 둘은 `TabDropIpc` 로 끊어 시험이 가짜를 꽂는다(ADR-0012).
// ADR-0296

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { DropTo } from '../../../../src-tauri/bindings/DropTo'
import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropDone } from '../../../../src-tauri/bindings/TabDropDone'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

export type { DropOutcome, DropTo, TabDrop, TabDropDone, TabDropReply }

export const CMD_TAB_DROP = 'tab_drop'
export const EVT_TAB_DROP_DONE = 'tab_drop:done'

/** Tauri 와의 접점. `listen` 은 이 창으로 지목된 사건만 받고, 등록이 끝나면 푸는 함수로 풀린다. */
export interface TabDropIpc {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
}

export interface TabDropClient {
  /** 놓기 하나를 보낸다. 답은 로그용이다 — 결과는 `onDone` 으로 온다(`duplicate` 는 알림이 없다). IPC 오류면 거절한다. */
  send(drop: TabDrop): Promise<TabDropReply>
  /**
   * 이 창으로 온 `tab_drop:done` 을 듣는다(출처 창만 받는다 — 셸이 명령의 `from` 에만 낸다). 반환 = 떼기 — 등록이 끝나기 전에
   * 떼도 늦게 풀린 등록을 곧바로 푼다. 몸 모양이 계약과 다르면 경고하고 버린다.
   */
  onDone(handler: (done: TabDropDone) => void): () => void
}

const OUTCOMES: readonly string[] = ['applied', 'canceled', 'ignored', 'rejected'] satisfies DropOutcome[]

function doneOf(raw: unknown): TabDropDone | null {
  if (raw === null || typeof raw !== 'object') return null
  const v = raw as Record<string, unknown>
  if (typeof v.drag !== 'string' || typeof v.outcome !== 'string' || !OUTCOMES.includes(v.outcome)) return null
  return { drag: v.drag, outcome: v.outcome as DropOutcome }
}

export function createTabDropClient(ipc: TabDropIpc): TabDropClient {
  return {
    send: drop => ipc.invoke<TabDropReply>(CMD_TAB_DROP, { drop }),

    onDone(handler) {
      let disposed = false
      let off: (() => void) | undefined
      void retryAsync(
        () =>
          ipc.listen(EVT_TAB_DROP_DONE, raw => {
            if (disposed) return
            const done = doneOf(raw)
            if (done === null) {
              console.warn(`[tabDrop] ${EVT_TAB_DROP_DONE}: 모양이 계약과 다르다 — 버린다:`, raw)
              return
            }
            handler(done)
          }),
        {
          isCancelled: () => disposed,
          onRetry: (err, attempt) => console.warn(`[tabDrop] ${EVT_TAB_DROP_DONE} 구독 재시도 #${attempt}:`, err),
        },
      )
        .then(unlisten => {
          if (disposed) unlisten()
          else off = unlisten
        })
        .catch(err => {
          if (err instanceof RetryCancelledError) return
          // 이 창은 놓기 결과를 못 받는다 — 바꿔 그리기는 그물(다음 tabs-updated · pointerdown)이 푼다.
          console.error(`[tabDrop] ${EVT_TAB_DROP_DONE} 구독 최종 실패:`, err)
        })
      return () => {
        disposed = true
        off?.()
        off = undefined
      }
    },
  }
}

const tauriIpc: TabDropIpc = {
  // ★`target` = 자기 label★ — 셸이 `emit_to` 로 그 창에만 내도 기본 타깃 `Any` 는 남의 창으로 지목된 알림까지 받는다(TRD §2-2 ·
  //   `api/restoreClient.ts` 의 같은 조항). 몸에 창 label 이 없어 받은 쪽이 가를 수도 없다.
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** 웹뷰마다 하나. */
export const tabDropClient: TabDropClient = createTabDropClient(tauriIpc)
