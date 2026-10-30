// 크래시 사본 「복원할까요?」의 상태를 main 창에 붙이는 창구 — 마지막으로 당긴 `restore_status` 하나를 쥐고 구독자
// (복원 모달 · 상태 파일 알림)에게 흘린다. 답은 셸 명령 `restore_answer` 하나다(TRD S21-storage §6-7).
//
// ★invoke/listen 을 직접 부른다★: 복원 상태의 주인은 데몬이 아니라 셸이라 agentClient 가 나를 것이 아니다 —
// `settingsClient.ts` 와 같은 예외. 둘은 `RestoreIpc` 로 끊어 시험이 가짜를 꽂는다(ADR-0012). LLM 경로는 버스
// `restore.status` / `restore.answer` 가 이미 맡아 새 전역 핸들을 두지 않는다 — 그쪽이 답해도 `restore:changed` 가
// 와서 이 창이 따라간다.
//
// ★main 창 전용★: 셸은 `restore:changed` 를 main 에만 낸다. 설치는 main 라우트(`AppLayout`)가 한다.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { AnswerReply } from '../../src-tauri/bindings/AnswerReply'
import type { CrashCopyStatus } from '../../src-tauri/bindings/CrashCopyStatus'
import type { RestoreStatusView } from '../../src-tauri/bindings/RestoreStatusView'
import type { StateFileStatus } from '../../src-tauri/bindings/StateFileStatus'
import { retryAsync, RetryCancelledError } from '../util/retryInvoke'

export type { AnswerReply, CrashCopyStatus, RestoreStatusView, StateFileStatus }

/**
 * 이 창이 쥐는 복원 상태 — 셸의 `RestoreStatusView` 에서 알림만 쓰는 두 칸(`saves` · `state_file`)이 느슨하다.
 * `undefined` = 짐에 그 칸이 없었거나 모르는 값이었다(셸과 프론트의 판이 어긋남) — 그 칸이 말하는 것을 모른다.
 */
export type RestoreView = Omit<RestoreStatusView, 'saves' | 'state_file'> & {
  saves: boolean | undefined
  state_file: StateFileStatus | undefined
}

export const EVT_RESTORE_CHANGED = 'restore:changed'
export const CMD_RESTORE_STATUS = 'restore_status'
export const CMD_RESTORE_ANSWER = 'restore_answer'

/** 부팅 당기기가 유계 재시도까지 다 실패한 뒤 다시 당기는 간격. */
export const BOOT_REPULL_INTERVAL_MS = 2000

/** Tauri 와의 접점. `listen` 은 등록이 끝나면 푸는 함수로 풀린다. */
export interface RestoreIpc {
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

export interface RestoreClient {
  /** 마지막으로 받은 상태 — 아직 한 번도 못 받았으면 `null`. 바뀌지 않는 동안 같은 객체를 돌려준다. */
  status(): RestoreView | null
  /** 상태가 바뀔 때마다 부른다. 반환 = 구독 해제(`useSyncExternalStore` 모양). */
  subscribe(listener: () => void): () => void
  /**
   * 답을 내고, 성공이든 실패든 상태를 다시 당긴 **뒤에** 풀린다 — 닫힘은 그 상태가 가른다.
   * 거절은 셸의 오류 문자열로 reject 한다. 종류를 문자열로 가르지 말 것 — 실패 뒤 상태는 `status()` 가 싣는다.
   */
  answer(accept: boolean): Promise<AnswerReply>
  /** 알림 구독 → 첫 당기기. 반환 = disposer. */
  install(): () => void
}

const CRASH_COPY: readonly string[] = ['none', 'awaiting', 'answered'] satisfies CrashCopyStatus[]
const STATE_FILE: readonly string[] = [
  'ok',
  'unreadable',
  'newer',
  'corrupt_copied_aside',
  'corrupt_not_copied',
] satisfies StateFileStatus[]

/**
 * `crash_copy` 를 못 읽으면 `null` — 모르는 값으로 main 을 막지 않게 하는 그물이다. 알림만 쓰는 두 칸은 못 읽어도
 * `undefined` 로 두고 짐을 살린다 — 버리면 묻는 모달까지 안 뜬다. 잃는 것은 그 알림 하나다.
 */
function viewOf(raw: unknown): RestoreView | null {
  if (raw === null || typeof raw !== 'object') return null
  const v = raw as Record<string, unknown>
  if (typeof v.crash_copy !== 'string' || !CRASH_COPY.includes(v.crash_copy)) return null
  const num = (x: unknown): number | null => (typeof x === 'number' ? x : null)
  return {
    crash_copy: v.crash_copy as CrashCopyStatus,
    saved_at_ms: num(v.saved_at_ms),
    windows: num(v.windows),
    tabs: num(v.tabs),
    durable: typeof v.durable === 'boolean' ? v.durable : null,
    // 모른다를 `undefined` 로 두는 것은 대신 칠 값이 없어서다 — `true` 는 저장한다고 단정하고 `false` 는 거짓 알림을 띄운다.
    saves: typeof v.saves === 'boolean' ? v.saves : undefined,
    state_file:
      typeof v.state_file === 'string' && STATE_FILE.includes(v.state_file)
        ? (v.state_file as StateFileStatus)
        : undefined,
  }
}

export function createRestoreClient(ipc: RestoreIpc): RestoreClient {
  let current: RestoreView | null = null
  const warned = new Set<'saves' | 'state_file'>()
  let issued = 0
  let applied = 0
  const listeners = new Set<() => void>()

  async function pull(isCancelled?: () => boolean, attempts?: number): Promise<void> {
    const { seq, raw } = await retryAsync(
      async () => {
        const seq = ++issued
        return { seq, raw: await ipc.invoke<unknown>(CMD_RESTORE_STATUS) }
      },
      {
        attempts,
        isCancelled,
        onRetry: (err, attempt) => {
          console.warn(`[restore] ${CMD_RESTORE_STATUS} 재시도 #${attempt}:`, err)
        },
      },
    )
    // 내린 설치(재마운트 · 언마운트 전 판)의 답은 칠하지 않는다 — 재시도는 떠 있는 요청을 거두지 못해 dispose
    //   뒤에도 풀릴 수 있다.
    if (isCancelled?.()) return
    // ★나중에 낸 당기기가 이긴다★ — 알림마다 당기므로 답이 낸 순서와 다르게 올 수 있고, 먼저 낸 쪽의 답은 더
    //   옛 상태다. 도착 순서로 칠하면 이미 닫힌 모달이 옛 `awaiting` 으로 되살아난다.
    if (seq < applied) return
    const view = viewOf(raw)
    // ★못 읽은 짐은 실패한 당기기로 친다 — 말없이 버리지 말 것★: 버리면 부팅 당기기가 받은 것처럼 끝나 다시 당기지
    //   않고, 셸이 묻고 있어도 모달이 영영 안 뜬다. 던지면 부팅 다시 당기기가 맞는 짐을 받을 때까지 돈다.
    if (!view) throw new Error(`${CMD_RESTORE_STATUS}: 모양이 계약과 다르다 — ${JSON.stringify(raw)}`)
    // 알림마다 다시 당기므로 판이 어긋난 동안 매번 같은 짐이 온다 — 칸마다 한 번만 남긴다.
    const unknown = (['saves', 'state_file'] as const).filter(k => view[k] === undefined && !warned.has(k))
    if (unknown.length > 0) {
      for (const k of unknown) warned.add(k)
      console.warn(`[restore] ${CMD_RESTORE_STATUS}: ${unknown.join(' · ')} 칸이 없거나 모르는 값이다 — 그 알림 없이 쓴다`, raw)
    }
    applied = seq
    current = view
    for (const listener of [...listeners]) listener()
  }

  return {
    status: () => current,

    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },

    async answer(accept) {
      try {
        return await ipc.invoke<AnswerReply>(CMD_RESTORE_ANSWER, { accept })
      } finally {
        await pull().catch(e => {
          console.error(`[restore] 답 뒤 ${CMD_RESTORE_STATUS} 최종 실패:`, e)
        })
      }
    },

    install() {
      let disposed = false
      let unlisten: (() => void) | undefined
      let repullTimer: ReturnType<typeof setTimeout> | undefined
      // 부팅 당기기를 내기 직전의 발급 번호 — 이보다 뒤에 낸 당기기가 칠했으면 받은 것이다(앞 설치가 남긴 상태는 안 센다).
      let bootFloor = 0
      const isCancelled = () => disposed

      const refresh = (where: string): void => {
        pull(isCancelled).catch(e => {
          if (e instanceof RetryCancelledError) return
          console.error(`[restore] ${where}: ${CMD_RESTORE_STATUS} 최종 실패:`, e)
        })
      }

      // ★부팅 당기기는 받을 때까지 놓지 않는다★ — 못 받으면 상태가 `null` 이라 셸이 `awaiting` 이어도 모달이 안 뜨고
      //   main 이 그대로 열린다. 알림은 상태가 **바뀔** 때만 와서 그것을 기다리면 영영 안 올 수 있다.
      //   유계 재시도 뒤로는 시도마다 한 번만 부르고 로그는 처음 한 번만 남긴다.
      const bootPull = (): void => {
        bootFloor = issued
        pull(isCancelled).catch(e => {
          if (e instanceof RetryCancelledError || disposed) return
          console.error(
            `[restore] 부팅 ${CMD_RESTORE_STATUS} 최종 실패 — ${BOOT_REPULL_INTERVAL_MS}ms 마다 다시 당긴다:`,
            e,
          )
          scheduleRepull()
        })
      }
      const scheduleRepull = (): void => {
        repullTimer = setTimeout(() => {
          repullTimer = undefined
          // 알림이 부른 당기기가 먼저 받았으면 여기서 멈춘다.
          if (disposed || applied > bootFloor) return
          pull(isCancelled, 1).catch(() => {
            if (!disposed) scheduleRepull()
          })
        }, BOOT_REPULL_INTERVAL_MS)
      }

      void (async () => {
        // ★구독 등록이 끝난 **뒤에** 당긴다 — 나란히 띄우지 말 것★(사유 = `theme/uiSettings.ts` 의 같은 조항).
        //   뒤집으면 당기기와 등록 사이의 답(`answered` · 되돌림)을 놓쳐 모달이 옛 상태에 머문다.
        // ★알려진 잔여 — `listen()` 이 거절 없이 영영 안 풀리면 당기기도 영영 안 나간다★(재시도는 거절만 덮는다 ·
        //   `settingsClient.ts` 와 같은 잔여). 그러면 상태가 `null` 로 남아 모달이 안 뜨고, 아래 부팅 다시 당기기도
        //   시작되지 않는다.
        try {
          const off = await retryAsync(
            () =>
              ipc.listen(EVT_RESTORE_CHANGED, () => {
                // 짐(`crash_copy` 한 칸)에는 수가 없어 다시 당긴다.
                if (disposed) return
                refresh(EVT_RESTORE_CHANGED)
              }),
            {
              isCancelled,
              onRetry: (err, attempt) => {
                console.warn(`[restore] ${EVT_RESTORE_CHANGED} 구독 재시도 #${attempt}:`, err)
              },
            },
          )
          if (disposed) {
            off()
            return
          }
          unlisten = off
        } catch (e) {
          if (e instanceof RetryCancelledError) return
          // 구독이 죽어도 당기기는 낸다 — 버스로 낸 답은 못 따라가도 지금 묻고 있는지는 알아야 한다.
          console.error(
            `[restore] ${EVT_RESTORE_CHANGED} 구독 실패 — 이 창은 이후 복원 상태 변경을 못 받는다:`,
            e,
          )
        }
        if (disposed) return
        bootPull()
      })()

      return () => {
        disposed = true
        clearTimeout(repullTimer)
        repullTimer = undefined
        unlisten?.()
        unlisten = undefined
      }
    },
  }
}

const tauriIpc: RestoreIpc = {
  // ★`target` = 자기 label★ — `listen()` 의 기본 타깃 `Any` 는 남의 창으로 지목된 알림까지 받는다
  //   (`theme/uiSettings.ts` 의 같은 조항).
  listen: (event, handler) =>
    listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** 창마다 하나 — main 의 `AppLayout` 이 설치한다. */
export const restoreClient: RestoreClient = createRestoreClient(tauriIpc)
