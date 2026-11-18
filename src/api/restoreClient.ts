// Attaches the crash-copy "restore?" prompt state to the main window — holds the last pulled `restore_status` and passes it to subscribers (the restore modal · the state file notice). The only answer is the shell command `restore_answer` (TRD S21-storage §6-7).
// ★Calls invoke/listen directly★: the shell owns restore state, not the daemon (same exception as `settingsClient.ts`); the two are cut at `RestoreIpc` so tests plug in a fake (ADR-0012). The LLM path is the bus `restore.status` / `restore.answer`, so no new global handle — when that side answers, `restore:changed` arrives and this window follows.
// ★Main window only★: the shell emits `restore:changed` to main only; the main route (`AppLayout`) installs it.

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
 * The restore state this window holds — of the shell's `RestoreStatusView`, the two notice-only fields (`saves` · `state_file`) are loose: `undefined` = the payload lacked the field or held an unknown value (shell and frontend editions out of step).
 */
export type RestoreView = Omit<RestoreStatusView, 'saves' | 'state_file'> & {
  saves: boolean | undefined
  state_file: StateFileStatus | undefined
}

export const EVT_RESTORE_CHANGED = 'restore:changed'
export const CMD_RESTORE_STATUS = 'restore_status'
export const CMD_RESTORE_ANSWER = 'restore_answer'

/** The interval for pulling again after the boot pull failed through all the bounded retries. */
export const BOOT_REPULL_INTERVAL_MS = 2000

/** The contact point with Tauri. `listen` resolves to a function that releases the registration once it is done. */
export interface RestoreIpc {
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

export interface RestoreClient {
  /** The last received state — `null` if none was ever received. Returns the same object while it does not change. */
  status(): RestoreView | null
  /** Called whenever the state changes. Returns = unsubscribe (`useSyncExternalStore` shape). */
  subscribe(listener: () => void): () => void
  /**
   * Sends the answer and resolves **after** pulling the state again, success or failure — that state decides the closing. A rejection carries the shell's error string; do not tell kinds apart by string — the state after a failure is in `status()`.
   */
  answer(accept: boolean): Promise<AnswerReply>
  /** Subscribe to notices → first pull. Returns = disposer. */
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
 * `null` if `crash_copy` cannot be read — a net so main is not blocked by an unknown value. The two notice-only fields stay `undefined` when unreadable and the payload is kept — dropping it would stop even the asking modal; only that one notice is lost.
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
    // Leaving unknown as `undefined` is because there is no value to fill in — `true` would assert it saves and `false` would raise a false notice.
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
          console.warn(`[restore] retrying ${CMD_RESTORE_STATUS} #${attempt}:`, err)
        },
      },
    )
    // Do not paint the answer of a taken-down install (pre-remount · pre-unmount edition) — a retry cannot withdraw a pending request, so it may resolve after dispose.
    if (isCancelled?.()) return
    // ★The later-issued pull wins★: pulls happen on every notice, so answers can arrive out of issue order and the earlier-issued answer is staler; painting in arrival order would revive a closed modal with the old `awaiting`.
    if (seq < applied) return
    const view = viewOf(raw)
    // ★An unreadable payload is a failed pull, never silently discarded★: discarding would end the boot pull as if received, so no re-pull and the modal never shows though the shell is asking; throwing keeps the boot re-pull running until a valid payload.
    if (!view) throw new Error(`${CMD_RESTORE_STATUS}: the shape differs from the contract — ${JSON.stringify(raw)}`)
    // We pull again on every notice, so the same payload comes every time while editions are out of step — leave it once per field.
    const unknown = (['saves', 'state_file'] as const).filter(k => view[k] === undefined && !warned.has(k))
    if (unknown.length > 0) {
      for (const k of unknown) warned.add(k)
      console.warn(`[restore] ${CMD_RESTORE_STATUS}: field ${unknown.join(' · ')} is missing or has an unknown value — using it without that notice`, raw)
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
          console.error(`[restore] ${CMD_RESTORE_STATUS} after the answer finally failed:`, e)
        })
      }
    },

    install() {
      let disposed = false
      let unlisten: (() => void) | undefined
      let repullTimer: ReturnType<typeof setTimeout> | undefined
      // The issue number just before the boot pull was issued — if a pull issued after this one painted, it was received (state left by an earlier install does not count).
      let bootFloor = 0
      const isCancelled = () => disposed

      const refresh = (where: string): void => {
        pull(isCancelled).catch(e => {
          if (e instanceof RetryCancelledError) return
          console.error(`[restore] ${where}: ${CMD_RESTORE_STATUS} finally failed:`, e)
        })
      }

      // ★The boot pull is not released until received★: if never received the state stays `null`, so a shell that is `awaiting` still shows no modal and main opens as is. Notices come only on a state **change**, so waiting for one may mean forever; after the bounded retry, call once per attempt and log only the first time.
      const bootPull = (): void => {
        bootFloor = issued
        pull(isCancelled).catch(e => {
          if (e instanceof RetryCancelledError || disposed) return
          console.error(
            `[restore] boot ${CMD_RESTORE_STATUS} finally failed — pulling again every ${BOOT_REPULL_INTERVAL_MS}ms:`,
            e,
          )
          scheduleRepull()
        })
      }
      const scheduleRepull = (): void => {
        repullTimer = setTimeout(() => {
          repullTimer = undefined
          // If the pull called by a notice received first, stop here.
          if (disposed || applied > bootFloor) return
          pull(isCancelled, 1).catch(() => {
            if (!disposed) scheduleRepull()
          })
        }, BOOT_REPULL_INTERVAL_MS)
      }

      void (async () => {
        // ★Pull **after** the subscription registration finishes — not side by side★ (same clause as `theme/uiSettings.ts`): reversed, answers between the pull and the registration (`answered` · reverted) are missed and the modal stays on the old state.
        // ★Known residual★: if `listen()` never resolves without rejecting, the pull never goes out either (retry covers only rejections — same as `settingsClient.ts`); the state stays `null` and the boot re-pull below does not start.
        try {
          const off = await retryAsync(
            () =>
              ipc.listen(EVT_RESTORE_CHANGED, () => {
                // The payload (the single `crash_copy` field) has no revision number, so we pull again.
                if (disposed) return
                refresh(EVT_RESTORE_CHANGED)
              }),
            {
              isCancelled,
              onRetry: (err, attempt) => {
                console.warn(`[restore] retrying the ${EVT_RESTORE_CHANGED} subscription #${attempt}:`, err)
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
          // Issue the pull even if the subscription died — even if we cannot follow answers sent over the bus, we must know whether it is asking now.
          console.error(
            `[restore] subscribing to ${EVT_RESTORE_CHANGED} failed — this window will not receive later restore state changes:`,
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
  // ★`target` = own label★: the default `Any` of `listen()` also receives notices addressed to another window (same clause as `theme/uiSettings.ts`).
  listen: (event, handler) =>
    listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** One per window — main's `AppLayout` installs it. */
export const restoreClient: RestoreClient = createRestoreClient(tauriIpc)
