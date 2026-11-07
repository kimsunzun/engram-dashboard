// ADR-0227: area frame default metrics report (TRD §2d) — once per webview, when a leaf mounts, the border element of that leaf is measured and sent with
//   `report_ui_metrics`. The shell computes area px from this value and the window canvas and does not ask the screen.
// Why the state is module scope: 「보냈음」 is one per webview, and a webview reload re-executes the module and resets it together.
//   At boot N leaves mount at once, so everyone shares the one report in progress.
// The reason for keeping it apart from the component file is the same as `windowCanvasReport.ts` (HMR re-executes only the modified module · Fast Refresh boundary).

import { invoke } from '@tauri-apps/api/core'
import type { CSSProperties } from 'react'

import type { UiMetrics } from '../../api/layoutTypes'
import { retryAsync } from '../../util/retryInvoke'
import { MIN_PANE_PX } from './splitPreview'

interface UiMetricsReportState {
  /** The shell responded with success. It is not set on failure. */
  reported: boolean
  /** The job being sent now (including retry waits). Cleared when it ends regardless of success or failure. */
  inFlight: Promise<void> | null
}

/**
 * Area frame border — worn together by a leaf (`LayoutLeaf`) and the empty canvas of waiting mode (`WindowLayout`). The metrics are the measured value of this border, so
 * if the two diverge, the shell's area content is off.
 * ★Do not merge into a shorthand property (`border`)★ — if the width is mixed with `var()` in one declaration, jsdom cannot give the computed width, so the metrics measurement
 * cannot be measured in a test.
 */
export const SLOT_FRAME_BORDER = {
  borderWidth: 1,
  borderStyle: 'solid',
  borderColor: 'var(--border)',
} as const satisfies CSSProperties

// Initialization replaces the whole state object, so the response of a job that left before it is written only to the discarded object.
let uiMetricsReport: UiMetricsReportState = { reported: false, inFlight: null }

/** The four computed border widths of the area border element (CSS px) + policy constant. null if any one width does not read as a finite number. */
export function measureUiMetrics(borderEl: Element): UiMetrics | null {
  const cs = getComputedStyle(borderEl)
  const t = parseFloat(cs.borderTopWidth)
  const r = parseFloat(cs.borderRightWidth)
  const b = parseFloat(cs.borderBottomWidth)
  const l = parseFloat(cs.borderLeftWidth)
  if (![t, r, b, l].every(Number.isFinite)) return null
  return { frame_insets: { t, r, b, l }, min_pane_px: MIN_PANE_PX }
}

/**
 * If this webview has not yet successfully sent the metrics, measure `borderEl` and send. If it is sending, wait together for that job.
 * If it fails after bounded retry (`retryAsync`), leave an error and keep it cleared so the next call (the next leaf mount) tries again.
 * The returned promise does not reject.
 */
export function reportUiMetrics(borderEl: Element): Promise<void> {
  const st = uiMetricsReport
  if (st.reported) return Promise.resolve()
  if (st.inFlight !== null) return st.inFlight
  const metrics = measureUiMetrics(borderEl)
  if (metrics === null) {
    // The shell rejects a non-finite inset — sending it only exhausts retries.
    console.error('[uiMetricsReport] 칸 테두리 폭을 수로 읽지 못해 보고하지 않는다:', borderEl)
    return Promise.resolve()
  }
  const job = retryAsync(() => invoke<void>('report_ui_metrics', { metrics }), {
    onRetry: (err, attempt) => console.warn(`[uiMetricsReport] report_ui_metrics 재시도 ${attempt}:`, err),
  }).then(
    () => {
      st.reported = true
      st.inFlight = null
    },
    err => {
      st.inFlight = null
      console.error('[uiMetricsReport] report_ui_metrics 최종 실패(재시도 소진):', err)
    },
  )
  st.inFlight = job
  return job
}

/** Test only — reset module state (isolation between tests). Do not call from production code. */
export function __resetUiMetricsReportForTest(): void {
  uiMetricsReport = { reported: false, inFlight: null }
}
