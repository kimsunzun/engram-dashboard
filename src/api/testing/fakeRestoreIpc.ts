// Test only — a fake of `RestoreIpc`. The test changes the shell-side state (`server.view`), and `restore_status` returns the value **at the moment it is called**.
// Registration · pull · answer can be held and released by the test.

import type {
  AnswerReply,
  CrashCopyStatus,
  RestoreIpc,
  RestoreStatusView,
  StateFileStatus,
} from '../restoreClient'
import { CMD_RESTORE_ANSWER, CMD_RESTORE_STATUS, EVT_RESTORE_CHANGED } from '../restoreClient'

export function deferred<T>(): {
  promise: Promise<T>
  resolve: (v: T) => void
  reject: (e: unknown) => void
} {
  let resolve: (v: T) => void = () => {}
  let reject: (e: unknown) => void = () => {}
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

export function view(
  crash_copy: CrashCopyStatus,
  over: Partial<RestoreStatusView> = {},
): RestoreStatusView {
  const awaiting = crash_copy === 'awaiting'
  const saves = over.saves ?? true
  return {
    crash_copy,
    saved_at_ms: awaiting ? 1_759_640_000_000 : null,
    windows: awaiting ? 2 : null,
    tabs: awaiting ? 5 : null,
    // Shell contract — while asking it equals `saves` (`RestoreStatusView.ts`). If the test gives `durable` separately, that wins.
    durable: awaiting ? saves : null,
    saves,
    state_file: 'ok' as StateFileStatus,
    ...over,
  }
}

export function fakeRestoreIpc(initial: RestoreStatusView) {
  const calls: string[] = []
  const server = { view: initial }
  let handler: ((payload: unknown) => void) | undefined
  let unlistenCalls = 0
  let listenGate: Promise<void> = Promise.resolve()
  /** When on, `restore_status` does not answer right away but piles up here — the test picks an order and releases them. */
  const heldStatus: Array<(v: RestoreStatusView) => void> = []
  let holdStatus = false
  /** When on, `restore_status` rejects (imitates IPC not ready). */
  let failStatus = false
  /** When on, `listen` rejects. */
  let failListen = false
  const answerArgs: boolean[] = []
  /** Behavior of `restore_answer` — default = accept it, change to `answered`, and succeed. `saves` · `state_file` stay as they are even when answered. */
  let onAnswer: (accept: boolean) => Promise<AnswerReply> = async accept => {
    server.view = view('answered', { saves: server.view.saves, state_file: server.view.state_file })
    return { restored_windows: accept ? 1 : 0, durable: server.view.saves }
  }

  const ipc: RestoreIpc = {
    async listen(event, h) {
      calls.push(`listen:${event}`)
      if (failListen) throw new Error('listen 실패(가짜)')
      await listenGate
      handler = h
      return () => {
        unlistenCalls++
        if (handler === h) handler = undefined
      }
    },
    async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
      calls.push(`invoke:${cmd}`)
      if (cmd === CMD_RESTORE_STATUS) {
        if (failStatus) throw new Error('restore_status 실패(가짜)')
        if (holdStatus) {
          const d = deferred<RestoreStatusView>()
          heldStatus.push(d.resolve)
          return (await d.promise) as T
        }
        return { ...server.view } as T
      }
      if (cmd === CMD_RESTORE_ANSWER) {
        const accept = args?.accept === true
        answerArgs.push(accept)
        return (await onAnswer(accept)) as T
      }
      throw new Error(`예상 밖 명령 ${cmd}`)
    },
  }

  return {
    ipc,
    calls,
    server,
    answerArgs,
    heldStatus,
    get unlistenCalls() {
      return unlistenCalls
    },
    hasListener: () => handler !== undefined,
    /** The currently registered handler — held and called even after release to imitate 「죽은 설치에 늦게 온 배달」 (a late delivery to a dead install). */
    currentHandler: () => handler,
    holdListen(): () => void {
      const d = deferred<void>()
      listenGate = d.promise
      return () => d.resolve()
    },
    setHoldStatus(on: boolean): void {
      holdStatus = on
    },
    setFailStatus(on: boolean): void {
      failStatus = on
    },
    setFailListen(on: boolean): void {
      failListen = on
    },
    statusCalls: () => calls.filter(c => c === `invoke:${CMD_RESTORE_STATUS}`).length,
    setOnAnswer(fn: (accept: boolean) => Promise<AnswerReply>): void {
      onAnswer = fn
    },
    /** The shell's `restore:changed` — the payload is the single `crash_copy` field. */
    push(payload: CrashCopyStatus = server.view.crash_copy): void {
      if (!handler) throw new Error(`${EVT_RESTORE_CHANGED} 리스너가 아직 없다`)
      handler(payload)
    },
  }
}

/** Flush the microtask chain (registration → pull → apply). */
export async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}
