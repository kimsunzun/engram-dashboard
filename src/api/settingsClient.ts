// The endpoint that attaches the shell settings (`shell\config\settings.json`) to the screen — holds the last applied value and `rev` per key,
// and passes changed values to subscribers (e.g. the chat style applier). The only writes are the shell commands `settings_set`/`settings_reset`.
//
// ★Apply rule = `rev` per key★ — the source of truth is the `SettingsSnapshot` doc (`src-tauri/src/settings/mod.rs`). Notices · the boot pull ·
// write answers all go through the same `apply`.
//
// ★Calls invoke/listen directly★: the owner of settings is the shell, not the daemon, so agentClient should not carry it —
// the same exception as `theme/uiSettings.ts`. The two are cut at `SettingsIpc` so tests plug in a fake (ADR-0012).
//
// ★`set`/`reset` before the shell `setup` opens writes are rejected with an error string★ — this module does not write during boot,
// and a rejection is returned as is through the caller's promise.
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

/** The contact point with Tauri. `listen` resolves to a function that releases the registration once it is done. */
export interface SettingsIpc {
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
}

export interface SettingChange {
  key: string
  /** The value the shell normalized — the default value arrives as the default string (an empty value meaning delete never comes). */
  value: string
}

export type SettingsListener = (changes: readonly SettingChange[]) => void

export interface SettingsClient {
  /** The value applied to this window — `undefined` for a key not yet received. */
  get(key: string): string | undefined
  /**
   * On subscribing, receives all values applied so far once (not called if there are none), and after that only keys whose value changed.
   * Returns = unsubscribe.
   */
  subscribe(listener: SettingsListener): () => void
  /** A rejection (unknown key · format · writes not open · disk failure) rejects with the shell's error string. */
  set(key: string, value: string): Promise<SetOutcome>
  /** `key` = an exact key or a prefix ending in `.`. A rejection is the same as `set`. */
  reset(key: string): Promise<ResetOutcome>
  /** Subscribe to notices → boot pull. Once per window. Returns = disposer. */
  install(): () => void
}

/** `null` if the payload shape differs from the contract — the shell exports it as a type so it should not break, and this is a safety net so the screen is not painted with a wrong value if it does. */
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

    // Paint the answer by the same rule — the shell only logs a notice emit failure, so even if a notice is lost the writing window stays correct.
    // ★Paint the `changed:false` answer too★: that answer's value is the shell's current value and its `rev` the current number, so if this window
    //   missed an earlier notice and holds an old value, this corrects it. It cannot overwrite a newer notice (`rev` per key).
    async set(key, value) {
      const outcome = await ipc.invoke<SetOutcome>(CMD_SET, { key, value })
      apply(outcome.rev, [{ key: outcome.key, value: outcome.value }])
      return outcome
    },

    // Only keys that actually changed (`changed`) carry a value in the answer — a key that was already default (a key only in `reset`) has no value
    //   and cannot be painted.
    async reset(key) {
      const outcome = await ipc.invoke<ResetOutcome>(CMD_RESET, { key })
      apply(outcome.rev, outcome.changed)
      return outcome
    },

    install() {
      let disposed = false
      let unlisten: (() => void) | undefined

      void (async () => {
        // ★Pull **after** the subscription registration finishes — do not fire them side by side★ (reason = the same clause in `theme/uiSettings.ts`).
        //   Reversed, a notice that arrives before registration never reaches this window, and a late answer may be older than it.
        // ★Known residual — if `listen()` never resolves without rejecting, the pull never goes out either★ (retry covers only rejections ·
        //   the same residual as `theme/uiSettings.ts`).
        try {
          const off = await retryAsync(
            () =>
              ipc.listen(EVT_SETTINGS_CHANGED, payload => {
                // If dispose runs while waiting on `listen()`, delivery may happen before `off()` below — a dead install does not paint.
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
          // Issue the pull even if the subscription died — even if later changes cannot be received, the current value must be right.
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
          // A key not received stays at the screen's fallback (e.g. `--chat-*` in `theme.css`) — notices carry only changed keys, so
          //   it stays that way until that key changes again (or until refresh).
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

// ★Do not pass `target`★ — the shell sends this notice to every webview in the same envelope (`emit`). Unlike `ui:settings-updated`, which the shell sends with
//   a different value addressed to each window — do not copy that side's window label subscription.
const tauriIpc: SettingsIpc = {
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload)),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** One per window — `main.tsx` installs it. */
export const settingsClient: SettingsClient = createSettingsClient(tauriIpc)
