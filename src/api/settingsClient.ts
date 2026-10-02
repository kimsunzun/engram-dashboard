// 셸 설정(`shell\config\settings.json`)을 화면에 붙이는 창구 — 키마다 마지막으로 적용한 값과 `rev` 를 쥐고,
// 바뀐 값을 구독자(예: 챗 스타일 적용자)에게 흘린다. 쓰기는 셸 명령 `settings_set`/`settings_reset` 뿐이다.
//
// ★적용 규칙 = 키마다 `rev`★ — 정본은 `SettingsSnapshot` doc(`src-tauri/src/settings/mod.rs`). 알림 · 부팅 당기기 ·
// 쓰기 답이 모두 같은 `apply` 를 탄다.
//
// ★invoke/listen 을 직접 부른다★: 설정의 주인은 데몬이 아니라 셸이라 agentClient 가 나를 것이 아니다 —
// `theme/uiSettings.ts` 와 같은 예외. 그 둘은 `SettingsIpc` 로 끊어 시험이 가짜를 꽂는다(ADR-0012).
//
// ★셸 `setup` 이 쓰기를 열기 전의 `set`/`reset` 은 오류 문자열로 거절된다★ — 이 모듈은 부팅 중에 쓰지 않고,
// 거절은 부른 쪽 promise 로 그대로 돌려준다.
// ADR-0265

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import type { ResetOutcome } from '../../src-tauri/bindings/ResetOutcome'
import type { SetOutcome } from '../../src-tauri/bindings/SetOutcome'
import { retryAsync, RetryCancelledError } from '../util/retryInvoke'

export type { ResetOutcome, SetOutcome }

const EVT_SETTINGS_CHANGED = 'settings:changed'
const CMD_GET = 'settings_get'
const CMD_SET = 'settings_set'
const CMD_RESET = 'settings_reset'

/** Tauri 와의 접점. `listen` 은 등록이 끝나면 푸는 함수로 풀린다. */
export interface SettingsIpc {
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

export interface SettingChange {
  key: string
  /** 셸이 정규화한 값 — 기본값이면 기본값 문자열이다(지운다는 뜻의 빈 값은 오지 않는다). */
  value: string
}

export type SettingsListener = (changes: readonly SettingChange[]) => void

export interface SettingsClient {
  /** 이 창에 적용된 값 — 아직 못 받은 키는 `undefined`. */
  get(key: string): string | undefined
  /**
   * 구독하자마자 지금까지 적용된 값 전부를 한 번 받고(없으면 안 부른다), 그 뒤로는 값이 달라진 키만 받는다.
   * 반환 = 구독 해제.
   */
  subscribe(listener: SettingsListener): () => void
  /** 거절(모르는 키 · 형식 · 쓰기 미개방 · 디스크 실패)은 셸의 오류 문자열로 reject 한다. */
  set(key: string, value: string): Promise<SetOutcome>
  /** `key` = 정확한 키 또는 `.` 으로 끝나는 접두. 거절은 `set` 과 같다. */
  reset(key: string): Promise<ResetOutcome>
  /** 알림 구독 → 부팅 당기기. 창마다 한 번. 반환 = disposer. */
  install(): () => void
}

/** 짐 모양이 계약과 다르면 `null` — 셸이 타입으로 내보내므로 깨질 일은 없고, 깨졌을 때 화면을 엉뚱한 값으로 칠하지 않게 하는 그물이다. */
function snapshotOf(raw: unknown): { rev: number; items: SettingChange[] } | null {
  if (raw === null || typeof raw !== 'object') return null
  const { rev, items } = raw as { rev?: unknown; items?: unknown }
  if (typeof rev !== 'number' || !Array.isArray(items)) return null
  const clean: SettingChange[] = []
  for (const item of items as unknown[]) {
    const { key, value } = (item ?? {}) as { key?: unknown; value?: unknown }
    if (typeof key === 'string' && typeof value === 'string') clean.push({ key, value })
  }
  return { rev, items: clean }
}

export function createSettingsClient(ipc: SettingsIpc): SettingsClient {
  const applied = new Map<string, { value: string; rev: number }>()
  const listeners = new Set<SettingsListener>()

  function notify(listener: SettingsListener, changes: readonly SettingChange[]): void {
    try {
      listener(changes)
    } catch (e) {
      console.error('[settings] 구독자가 던졌다:', e)
    }
  }

  function apply(rev: number, items: readonly SettingChange[]): void {
    const changes: SettingChange[] = []
    for (const { key, value } of items) {
      const prev = applied.get(key)
      if (prev !== undefined && rev <= prev.rev) continue
      applied.set(key, { value, rev })
      if (prev?.value !== value) changes.push({ key, value })
    }
    if (changes.length === 0) return
    for (const listener of [...listeners]) notify(listener, changes)
  }

  function applyRaw(raw: unknown, where: string): void {
    const snap = snapshotOf(raw)
    if (!snap) {
      console.warn(`[settings] ${where}: 모양이 계약과 다르다 — 버린다`, raw)
      return
    }
    apply(snap.rev, snap.items)
  }

  return {
    get: key => applied.get(key)?.value,

    subscribe(listener) {
      listeners.add(listener)
      const current = [...applied].map(([key, { value }]) => ({ key, value }))
      if (current.length > 0) notify(listener, current)
      return () => {
        listeners.delete(listener)
      }
    },

    // 답도 같은 규칙으로 칠한다 — 셸은 알림 emit 실패를 로그로만 남기므로, 알림이 유실돼도 쓴 창은 맞는다.
    // ★`changed:false` 답도 칠한다★: 그 답의 값은 셸의 현재 값이고 `rev` 는 현재 번호라, 이 창이 앞선 알림을
    //   놓쳐 옛 값을 쥐고 있으면 이것이 바로잡는다. 더 새 알림을 덮을 수는 없다(키마다 `rev`).
    async set(key, value) {
      const outcome = await ipc.invoke<SetOutcome>(CMD_SET, { key, value })
      apply(outcome.rev, [{ key: outcome.key, value: outcome.value }])
      return outcome
    },

    // 답이 값을 싣는 것은 실제로 바뀐 키(`changed`)뿐이다 — 이미 기본값이던 키(`reset` 에만 있는 키)는 값이
    //   없어 칠할 수 없다.
    async reset(key) {
      const outcome = await ipc.invoke<ResetOutcome>(CMD_RESET, { key })
      apply(outcome.rev, outcome.changed)
      return outcome
    },

    install() {
      let disposed = false
      let unlisten: (() => void) | undefined

      void (async () => {
        // ★구독 등록이 끝난 **뒤에** 당긴다 — 나란히 띄우지 말 것★(사유 = `theme/uiSettings.ts` 의 같은 조항).
        //   뒤집으면 등록 전에 온 알림은 이 창에 닿지도 않고, 늦게 온 답이 그보다 옛 값일 수 있다.
        // ★알려진 잔여 — `listen()` 이 거절 없이 영영 안 풀리면 당기기도 영영 안 나간다★(재시도는 거절만 덮는다 ·
        //   `theme/uiSettings.ts` 와 같은 잔여).
        try {
          const off = await retryAsync(
            () =>
              ipc.listen(EVT_SETTINGS_CHANGED, payload => {
                // dispose 가 `listen()` 대기 중에 돌면 아래 `off()` 전에 배달될 수 있다 — 죽은 설치는 칠하지 않는다.
                if (disposed) return
                applyRaw(payload, EVT_SETTINGS_CHANGED)
              }),
            {
              isCancelled: () => disposed,
              onRetry: (err, attempt) => {
                console.warn(`[settings] ${EVT_SETTINGS_CHANGED} 구독 재시도 #${attempt}:`, err)
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
          // 구독이 죽어도 당기기는 낸다 — 이후 변경은 못 받아도 지금 값은 맞아야 한다.
          console.error(
            `[settings] ${EVT_SETTINGS_CHANGED} 구독 실패 — 이 창은 이후 설정 변경을 못 받는다:`,
            e,
          )
        }

        try {
          const answer = await retryAsync(() => ipc.invoke<unknown>(CMD_GET), {
            isCancelled: () => disposed,
            onRetry: (err, attempt) => {
              console.warn(`[settings] ${CMD_GET} 재시도 #${attempt}:`, err)
            },
          })
          if (disposed) return
          applyRaw(answer, CMD_GET)
        } catch (e) {
          if (e instanceof RetryCancelledError) return
          // 받지 못한 키는 화면의 fallback(예: `theme.css` 의 `--chat-*`)으로 남는다 — 알림은 바뀐 키만 실으므로
          //   그 키가 다시 바뀌기 전까지(또는 새로고침까지) 그대로다.
          console.error(`[settings] ${CMD_GET} 최종 실패:`, e)
        }
      })()

      return () => {
        disposed = true
        unlisten?.()
        unlisten = undefined
      }
    },
  }
}

// ★`target` 을 주지 않는다★ — 셸은 이 알림을 모든 웹뷰에 같은 봉투로 보낸다(`emit`). 창마다 다른 값을 지목해
//   보내는 `ui:settings-updated` 와 달라 그쪽의 창 label 구독을 베껴 오지 말 것.
const tauriIpc: SettingsIpc = {
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload)),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** 창마다 하나 — `main.tsx` 가 설치한다. */
export const settingsClient: SettingsClient = createSettingsClient(tauriIpc)
