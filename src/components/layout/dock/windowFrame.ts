// TRD S21-slot-docking §3-8: this window's window facts (restored inner size · maximized · frame inset · front window · chrome shape — the last two are TRD
//   S26-window-chrome §6-5) — pulled once at mount with `window_frame`,
//   after that it receives `window:frame`, which the shell pushes only when values change, and holds only the last value. It is the material the source webview uses at dragstart to compute the original size · new window
//   size (R7 · R22), and the material the title row uses to draw maximize · dimming · buttons (TRD S26-window-chrome §7 — `subscribe`).
//   It is communication outside the drag, so it is received without blocking even during a drag (ADR-0297) — the drag reads the value at that time only once at dragstart.
// ★Do not compute the frame inset in the web as `outer* − inner*`★ — whether that value matches the window frame on WebView2 is untested, so only the value the shell computed is used.
// The reason for calling invoke/listen directly · how to cut it apart is the same as `tabDrop.ts` (the shell owns it · not a bus command · ADR-0012).

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { CaptionChrome } from '../../../../src-tauri/bindings/CaptionChrome'
import type { Insets } from '../../../../src-tauri/bindings/Insets'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import type { WindowFrame } from '../../../../src-tauri/bindings/WindowFrame'
import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

export type { WindowFrame }

export const CMD_WINDOW_FRAME = 'window_frame'
export const EVT_WINDOW_FRAME = 'window:frame'

/** Contact point with Tauri. `listen` receives only events addressed to this window, and when registration finishes it is released by the unlisten function. */
export interface WindowFrameIpc {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
}

export interface WindowFrameSource {
  /** The last value received — `null` if not yet received (meanwhile the target treats it as no size · TRD §4-2). The same reference until the next value is received. */
  current(): WindowFrame | null
  /** Called every time `current()` changes (`useSyncExternalStore` style). Return = detach. Receiving is attached separately by `install`. */
  subscribe(listener: () => void): () => void
  /** Pull once after attaching event listening. Return = detach. */
  install(): () => void
}

function finite(x: unknown): x is number {
  return typeof x === 'number' && Number.isFinite(x)
}

function sizeOf(raw: unknown): SizeCss | null | undefined {
  if (raw === null) return null
  if (typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  return finite(v.w) && finite(v.h) ? { w: v.w, h: v.h } : undefined
}

function insetsOf(raw: unknown): Insets | undefined {
  if (raw === null || typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  return finite(v.t) && finite(v.r) && finite(v.b) && finite(v.l) ? { t: v.t, r: v.r, b: v.b, l: v.l } : undefined
}

function captionOf(raw: unknown): CaptionChrome | undefined {
  if (raw === null || typeof raw !== 'object') return undefined
  const v = raw as Record<string, unknown>
  return typeof v.buttons === 'boolean' && finite(v.leading_inset) && typeof v.snap_overlay === 'boolean'
    ? { buttons: v.buttons, leading_inset: v.leading_inset, snap_overlay: v.snap_overlay }
    : undefined
}

/** `null` if the shape differs from the contract. */
function frameOf(raw: unknown): WindowFrame | null {
  if (raw === null || typeof raw !== 'object') return null
  const v = raw as Record<string, unknown>
  const restored = sizeOf(v.restored)
  const insets = insetsOf(v.insets)
  const caption = captionOf(v.caption)
  if (
    restored === undefined ||
    insets === undefined ||
    caption === undefined ||
    typeof v.maximized !== 'boolean' ||
    typeof v.focused !== 'boolean'
  )
    return null
  return { restored, maximized: v.maximized, insets, focused: v.focused, caption }
}

export function createWindowFrame(ipc: WindowFrameIpc): WindowFrameSource {
  let frame: WindowFrame | null = null
  // Number of `window:frame` received — a pull answer is discarded if even one event came after it was issued.
  let pushes = 0
  const listeners = new Set<() => void>()
  const set = (next: WindowFrame): void => {
    frame = next
    listeners.forEach(fn => fn())
  }

  return {
    current: () => frame,

    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },

    install() {
      let disposed = false
      let off: (() => void) | undefined
      const isCancelled = () => disposed

      void (async () => {
        try {
          const unlisten = await retryAsync(
            () =>
              ipc.listen(EVT_WINDOW_FRAME, raw => {
                if (disposed) return
                const next = frameOf(raw)
                if (next === null) {
                  console.warn(`[windowFrame] ${EVT_WINDOW_FRAME}: 모양이 계약과 다르다 — 버린다:`, raw)
                  return
                }
                pushes += 1
                set(next)
              }),
            {
              isCancelled,
              onRetry: (err, attempt) => console.warn(`[windowFrame] ${EVT_WINDOW_FRAME} 구독 재시도 #${attempt}:`, err),
            },
          )
          if (disposed) {
            unlisten()
            return
          }
          off = unlisten
        } catch (err) {
          if (err instanceof RetryCancelledError) return
          // Issue the pull as is — even if events cannot be received, a value must exist at mount.
          console.error(`[windowFrame] ${EVT_WINDOW_FRAME} 구독 최종 실패 — 이 창은 창 사실 바뀜을 못 받는다:`, err)
        }
        if (disposed) return
        // ★Pull **after** attaching the subscription★ — if issued side by side, a change between the pull and the registration is missed and it stays at the old value
        //   (the same clause of `api/restoreClient.ts`).
        const seen = pushes
        try {
          const raw = await retryAsync(() => ipc.invoke<unknown>(CMD_WINDOW_FRAME), {
            isCancelled,
            onRetry: (err, attempt) => console.warn(`[windowFrame] ${CMD_WINDOW_FRAME} 재시도 #${attempt}:`, err),
          })
          // If an event came after the pull was issued, the event carries the last value (the shell pushes on every change) — discard the answer so that, arriving later than the event, it does not
          //   overwrite with the old value.
          if (disposed || pushes !== seen) return
          const next = frameOf(raw)
          if (next === null) {
            console.warn(`[windowFrame] ${CMD_WINDOW_FRAME}: 모양이 계약과 다르다 — 버린다:`, raw)
            return
          }
          set(next)
        } catch (err) {
          if (err instanceof RetryCancelledError) return
          console.error(`[windowFrame] ${CMD_WINDOW_FRAME} 최종 실패 — 다음 ${EVT_WINDOW_FRAME} 까지 창 사실이 없다:`, err)
        }
      })()

      return () => {
        disposed = true
        off?.()
        off = undefined
      }
    },
  }
}

const tauriIpc: WindowFrameIpc = {
  // ★`target` = its own label★ — the body has no window label, so listening with the default target `Any` would hold another window's value as its own (TRD §2-2).
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => invoke<T>(cmd, args),
}

/** One per webview. */
export const windowFrame: WindowFrameSource = createWindowFrame(tauriIpc)
