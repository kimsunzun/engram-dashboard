// TRD S21-slot-docking §3-1 · §4-2 · §4-6: the wrapper of the drop command `tab_drop` and listening to the result notice `tab_drop:done` — per drag at most two commands after
//   it ends (`drag_end` of the source dragend · `drop` of the receiving window), and the result comes not as an answer but as the source window's `done`.
// ★Call invoke/listen directly★: the drop is wiring inside a webview that the shell owns, so agentClient should not carry it, and it is not a bus command either (the LLM command
//   for the same action = T-65 `tab.dock` · TRD §3-6). The two are cut apart with `TabDropIpc` so the test plugs in a fake (ADR-0012).
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

/** Contact point with Tauri. `listen` receives only events addressed to this window, and when registration finishes it is released by the unlisten function. */
export interface TabDropIpc {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
}

export interface TabDropClient {
  /** Send one drop. The answer is for logging — the result arrives through `onDone` (`duplicate` has no notice). If the IPC errors, it rejects. */
  send(drop: TabDrop): Promise<TabDropReply>
  /**
   * Listen to `tab_drop:done` that came to this window (only the source window receives it — the shell emits only to the command's `from`). Return = detach — even if detached before registration
   * finishes, a late-resolved registration is released immediately. If the body shape differs from the contract, it warns and discards.
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
          // This window cannot receive the drop result — swap redraw is released by the net (next tabs-updated · pointerdown).
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
  // ★`target` = its own label★ — even if the shell emits only to that window with `emit_to`, the default target `Any` also receives notices addressed to another window (TRD §2-2 ·
  //   the same clause of `api/restoreClient.ts`). The body has no window label, so the receiver cannot tell them apart either.
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** One per webview. */
export const tabDropClient: TabDropClient = createTabDropClient(tauriIpc)
