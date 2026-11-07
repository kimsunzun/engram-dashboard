// ADR-0304 — snap overlay feature
// TRD S26-window-chrome §8-4: snap layout overlay — the maximize button rectangle is reported to the shell (invoke
//   `report_max_button_rect` — that invoke is registered only in that build) only in a build where the shell turned it on (window fact `caption.snap_overlay` — shell feature
//   `snap-overlay` · off by default · user decision Q1 2026-10-24). The shell puts an OS child window on that spot so the Windows 11 snap menu
//   comes up (platform `window::snap`), and since that window takes the cursor, hover changes are returned to this window as `window:snap-hover` (body = bool)
//   — the button draws hover from that value (`CaptionButtons`). In a build where it is off, nothing is called or listened to.
// Rectangle = physical px of this window's client — CSS px multiplied by `devicePixelRatio` and snapped outward (to cover the whole button). When it re-measures = when the window
//   size changes (the button touches the right end so its position moves) · when the button size changes (maximize changes the row height). ★When only the scale changes and the CSS size stays
//   the same (moving to a monitor with a different scale) it is not re-measured★ — it is measured on the day it is turned on (TRD §14 W8).
// Why invoke/listen are called directly = it is a measurement report, so not a bus command (ADR-0227 · same as `minSizeReport.ts`).

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useState, type RefObject } from 'react'

export const CMD_REPORT_MAX_BUTTON_RECT = 'report_max_button_rect'
export const EVT_SNAP_HOVER = 'window:snap-hover'

/** Client physical px rectangle — the same shape as shell `commands::snap_overlay::MaxButtonRect`. */
export interface PhysRect {
  x: number
  y: number
  width: number
  height: number
}

/** Contact point with Tauri. `listen` receives only events addressed to this window, and when registration finishes it is released by the unlisten function. */
export interface SnapOverlayIpc {
  invoke(cmd: string, args: Record<string, unknown>): Promise<unknown>
  listen(event: string, handler: (payload: unknown) => void): Promise<() => void>
}

const tauriIpc: SnapOverlayIpc = {
  invoke: (cmd, args) => invoke(cmd, args),
  // ★`target` = its own label★ — listening with the default target `Any` would hold another window's hover as its own (same as `dock/windowFrame.ts`).
  listen: (event, handler) => listen<unknown>(event, e => handler(e.payload), { target: getCurrentWindow().label }),
}

/** CSS px rectangle → physical px rectangle (snapped outward). If empty (width · height 0 — the button is not visible), `null` = withdraw. */
export function physicalRect(r: { left: number; top: number; right: number; bottom: number }, dpr: number): PhysRect | null {
  const x = Math.floor(r.left * dpr)
  const y = Math.floor(r.top * dpr)
  const width = Math.ceil(r.right * dpr) - x
  const height = Math.ceil(r.bottom * dpr) - y
  return width >= 1 && height >= 1 ? { x, y, width, height } : null
}

const keyOf = (r: PhysRect | null): string => (r === null ? '' : `${r.x},${r.y},${r.width},${r.height}`)

/**
 * While `enabled`, report the rectangle of `button` (the same value is not sent again) and return the overlay hover. When turned off or detached,
 * withdraw the overlay (`rect: null`). If `enabled` is false, nothing is called and it is always false.
 */
export function useSnapOverlay(enabled: boolean, button: RefObject<HTMLElement | null>, ipc: SnapOverlayIpc = tauriIpc): boolean {
  const [hover, setHover] = useState(false)
  useEffect(() => {
    const el = button.current
    if (!enabled || el === null) return
    let disposed = false
    let sent: string | null = null
    const report = (rect: PhysRect | null): void => {
      ipc.invoke(CMD_REPORT_MAX_BUTTON_RECT, { rect }).catch(err => console.warn(`[snapOverlay] ${CMD_REPORT_MAX_BUTTON_RECT} 실패:`, err))
    }
    const measure = (): void => {
      const rect = physicalRect(el.getBoundingClientRect(), window.devicePixelRatio || 1)
      const key = keyOf(rect)
      if (key === sent) return
      sent = key
      report(rect)
    }
    measure()
    window.addEventListener('resize', measure)
    // In environments without an observer such as jsdom, it listens only to window size changes.
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    ro?.observe(el)
    let off: (() => void) | undefined
    ipc.listen(EVT_SNAP_HOVER, payload => {
      if (!disposed) setHover(payload === true)
    }).then(
      unlisten => {
        if (disposed) unlisten()
        else off = unlisten
      },
      err => console.warn(`[snapOverlay] ${EVT_SNAP_HOVER} 구독 실패 — 단추가 맴돎을 못 그린다:`, err),
    )
    return () => {
      disposed = true
      window.removeEventListener('resize', measure)
      ro?.disconnect()
      off?.()
      setHover(false)
      // Withdraw — if an empty rectangle (withdraw) was already sent, it is not sent again.
      if (sent !== '') report(null)
    }
  }, [enabled, button, ipc])
  return enabled && hover
}
