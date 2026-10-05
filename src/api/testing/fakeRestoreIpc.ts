// 테스트 전용 — `RestoreIpc` 가짜. 셸 쪽 상태(`server.view`)를 시험이 바꾸고, `restore_status` 는 **부른 순간의** 값을
// 돌려준다. 등록 · 당기기 · 답은 시험이 붙잡았다 놓을 수 있다.

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
  return {
    crash_copy,
    saved_at_ms: awaiting ? 1_759_640_000_000 : null,
    windows: awaiting ? 2 : null,
    tabs: awaiting ? 5 : null,
    durable: awaiting ? true : null,
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
  /** 켜면 `restore_status` 가 바로 답하지 않고 여기 쌓인다 — 시험이 순서를 골라 푼다. */
  const heldStatus: Array<(v: RestoreStatusView) => void> = []
  let holdStatus = false
  /** 켜면 `restore_status` 가 거절한다(IPC 미준비 흉내). */
  let failStatus = false
  /** 켜면 `listen` 이 거절한다. */
  let failListen = false
  const answerArgs: boolean[] = []
  /** `restore_answer` 의 동작 — 기본 = 받아서 `answered` 로 바꾸고 성공. */
  let onAnswer: (accept: boolean) => Promise<AnswerReply> = async accept => {
    server.view = view('answered', { state_file: server.view.state_file })
    return { restored_windows: accept ? 1 : 0, durable: true }
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
    /** 지금 등록된 핸들러 — 풀린 뒤에도 쥐고 불러 「죽은 설치에 늦게 온 배달」을 흉내 낸다. */
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
    /** 셸의 `restore:changed` — 짐은 `crash_copy` 한 칸이다. */
    push(payload: CrashCopyStatus = server.view.crash_copy): void {
      if (!handler) throw new Error(`${EVT_RESTORE_CHANGED} 리스너가 아직 없다`)
      handler(payload)
    },
  }
}

/** 마이크로태스크 사슬(등록 → 당기기 → 적용)을 흘려보낸다. */
export async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}
