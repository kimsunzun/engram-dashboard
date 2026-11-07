// ADR-0227: the sending side of the window canvas size report. Observation and debounce are done by the hook in `WindowLayout.tsx`; here it
//   decides what to send as `report_window_canvas` (sending rules = `util/latestValueReporter.ts`) and exposes the value the shell
//   last accepted (`subscribeCanvasReport` — the divider drag's allowed range reads it as the same input as the shell).
// Why state is module-scoped: hook instances change even within one webview (RootErrorBoundary re-render, HMR remount).
//   With per-instance state, a new instance would not know about the old instance's in-flight invoke and would send one more.
// Why it is kept apart from the component file: HMR re-runs only the edited module, so this state survives edits to `WindowLayout.tsx`. If a
//   component file exports a non-component (the test reset below), the Fast Refresh boundary breaks too.
//   Limit: editing this file itself creates fresh state (during development only).

import { invoke } from '@tauri-apps/api/core'

import { LatestValueReporter } from '../../util/latestValueReporter'

/** Window canvas size (integer CSS px) — the argument of shell `report_window_canvas`. */
export interface CanvasPx {
  w: number
  h: number
}

/** The reporter shared by one webview (= one window). `lastSent` = the last value the shell answered successfully. */
export type CanvasReportState = LatestValueReporter<CanvasPx>

const sameCanvas = (a: CanvasPx, b: CanvasPx): boolean => a.w === b.w && a.h === b.h
const isEmptyCanvas = (s: CanvasPx): boolean => s.w === 0 || s.h === 0

function createCanvasReport(): CanvasReportState {
  const reporter: CanvasReportState = new LatestValueReporter<CanvasPx>({
    send: size => invoke<void>('report_window_canvas', { w: size.w, h: size.h }),
    same: sameCanvas,
    log: 'windowCanvasReport',
    describe: size => `report_window_canvas(${size.w}x${size.h})`,
    // A late reply to a reporter discarded by a test reset does not notify the current reporter's subscribers.
    onAccepted: size => {
      if (reporter === canvasReport) notifyCanvas(size)
    },
  })
  return reporter
}

let canvasReport = createCanvasReport()

// Each subscription gets its own entry — even if the same function is hooked twice, releasing one does not remove the other.
const canvasListeners = new Set<{ cb: (size: CanvasPx | null) => void }>()

/** This webview's reporter, which the hook takes when it attaches observation. */
export function getCanvasReport(): CanvasReportState {
  return canvasReport
}

/**
 * Called with the new value whenever `getCanvasReport().lastSent` changes — when the shell answers successfully with a size different from now, and
 * when a test reset clears the value (null). Not called on failure or on a repeat success with the same value.
 * Returns the unsubscribe function (safe to call several times). If a subscriber throws, reporting carries on (only an error log is left).
 */
export function subscribeCanvasReport(cb: (size: CanvasPx | null) => void): () => void {
  const entry = { cb }
  canvasListeners.add(entry)
  return () => {
    canvasListeners.delete(entry)
  }
}

// The notification goes out inside the invoke reply handling — if a subscriber's throw leaked, it would count as a report failure and resend the same value.
function notifyCanvas(size: CanvasPx | null): void {
  for (const { cb } of [...canvasListeners]) {
    try {
      cb(size)
    } catch (err) {
      console.error('[windowCanvasReport] 캔버스 구독자 오류:', err)
    }
  }
}

/** Test only — resets module state (isolation between tests). Must not be called from production code. */
export function __resetCanvasReportForTest(): void {
  const had = canvasReport.lastSent !== null
  canvasReport.release()
  canvasReport = createCanvasReport()
  if (had) notifyCanvas(null)
}

/** Announces one rounded observation (before debounce). */
export function observeCanvasSize(st: CanvasReportState, size: CanvasPx): void {
  // A zero size is not a candidate to send, and does not make the old value stale.
  if (isEmptyCanvas(size)) return
  st.observe(size)
}

/** Puts a debounced measurement up for sending. */
export function submitCanvasSize(st: CanvasReportState, size: CanvasPx): void {
  // 0×0 RO callbacks do exist (TerminalSlot.tsx filters 0×0 too). The shell ignores them as well, but the screen does not send them.
  if (isEmptyCanvas(size)) return
  st.submit(size)
}

/** When observation is released — drops the pending value and stops retrying (an invoke in flight is left alone — `LatestValueReporter.release`). */
export function releaseCanvasReport(st: CanvasReportState): void {
  st.release()
}
