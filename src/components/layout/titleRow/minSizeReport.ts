// TRD S26-window-chrome §7-7: minimum window size report (`report_window_min_width`) — sends the shell the value this window's title row counted (`metrics.ts` `minWindowSize`), only after the window number is received (`TitleRow`). In one synchronous command the shell applies it as the OS minimum size (normal-size windows only) and, on the first report after the window opened, widens the window once and then answers (`src-tauri/src/chrome` — `report_min_size`). So the first-screen notice waits for this answer (`minWindowSizeSettled`) to finish the widening before the first screen shows.
// State is module scope: one report per webview (window); a reload re-executes the module and resets it (same as `uiMetricsReport.ts`).
// ★One at a time · only the last value★ — a value taken while sending is sent after that response; if a newer value is taken meanwhile the old one is not sent.
// ★An attempt whose response never arrives is released at the deadline (`MIN_SIZE_ATTEMPT_TIMEOUT_MS`)★, or all later reports block behind it. A released attempt may reach the shell later than the next attempt (see `util/latestValueReporter.ts`), so every send (retries included) carries a new sequence number `seq` and this page's `epoch`; the shell discards an older `epoch`, or within the same `epoch` a sequence not newer than the accepted one (`chrome::book` `record_min`). Regardless of arrival order the shell keeps the largest sequence of the newest page = the last sent value.
// The believed shell value = only the value whose largest sequence got a success response; it becomes unknown the moment a new sequence is sent, and an older sequence's answer (even after the deadline) changes nothing.

import { invoke } from '@tauri-apps/api/core'

import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

export interface MinWindowSize {
  width: number
  height: number
}

// Cap on how long the first-screen notice waits for the report — shorter than the shell's show deadline (1500 ms). A normal answer takes a few ms; it waits through one retry after a first failure (150 ms — `retryAsync` default). Beyond it the first screen is announced and the report finishes in the background (the window may widen after it shows — the look TRD T1 accepted).
export const MIN_SIZE_WAIT_CAP_MS = 300

// Cap on how long one attempt waits for an answer — beyond it the attempt counts as failed and is retried (a newer value, if taken, is sent instead). Normally a few ms, so this only reclaims the report line from a stuck response; too short and a slow response sends the same value twice.
export const MIN_SIZE_ATTEMPT_TIMEOUT_MS = 1000

interface MinSizeReportState {
  /**
   * This page's mark = `Math.floor(performance.timeOrigin)` (wall-clock ms at page load) — grows with every reload so the shell can order pages (a late report from an old page cannot overwrite a new page's value). A larger `epoch` makes the shell count sequence numbers anew (`seq` restarts from 1 on reload). Two pages with the same-ms `timeOrigin` mix as one page — not handled.
   */
  epoch: number
  /**
   * The last sequence number sent — rises by one on every send (including retries); first = 1. u32 overflow is not handled — one page does not send 4.2 billion times.
   */
  seq: number
  /** The last value taken — the same value is not taken again. On final failure it is cleared so the same value can be taken again. */
  latest: MinWindowSize | null
  /** Generation of the taken value — rises every time a value is taken. A report whose generation has passed is not attempted further. */
  gen: number
  /** What is believed to be the shell's current value = the value that sequence `seq` got a success response for. Unknown (`null`) if that sequence's answer has not arrived yet. */
  delivered: MinWindowSize | null
  /** Resolves when the reports in hand all finish (success · final failure · pushed out by a newer value). Does not reject. */
  chain: Promise<void>
}

const fresh = (): MinSizeReportState => ({
  epoch: Math.floor(performance.timeOrigin),
  seq: 0,
  latest: null,
  gen: 0,
  delivered: null,
  chain: Promise.resolve(),
})

// Initialization replaces the whole state object, so the response of a report taken before it is written only to the discarded object.
let report: MinSizeReportState = fresh()

const same = (a: MinWindowSize, b: MinWindowSize): boolean => a.width === b.width && a.height === b.height

/** Hands over the title row's current minimum window size — does nothing if it equals the last value taken. */
export function reportMinWindowSize(size: MinWindowSize): void {
  const st = report
  if (st.latest !== null && same(st.latest, size)) return
  st.latest = size
  const gen = ++st.gen
  st.chain = st.chain.then(() => send(st, size, gen))
}

async function send(st: MinSizeReportState, size: MinWindowSize, gen: number): Promise<void> {
  if (st.gen !== gen) return
  try {
    await retryAsync(() => attempt(st, size), {
      isCancelled: () => st.gen !== gen,
      onRetry: (err, n) => {
        if (st.gen === gen) console.warn(`[minSizeReport] report_window_min_width(${size.width}x${size.height}) retry ${n}:`, err)
      },
    })
  } catch (err) {
    if (err instanceof RetryCancelledError) return
    if (st.gen === gen) st.latest = null
    console.error(`[minSizeReport] report_window_min_width(${size.width}x${size.height}) final failure (retries exhausted):`, err)
  }
}

/**
 * Send once — not sent if the shell's current value is already `size`. Carries a new sequence number and rejects if there is no answer within `MIN_SIZE_ATTEMPT_TIMEOUT_MS`. A success answer is recorded as "reached" only when that sequence is still the last (also for an answer after the deadline).
 */
function attempt(st: MinSizeReportState, size: MinWindowSize): Promise<void> {
  if (st.delivered !== null && same(st.delivered, size)) return Promise.resolve()
  const seq = ++st.seq
  st.delivered = null
  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no response (${MIN_SIZE_ATTEMPT_TIMEOUT_MS} ms)`)), MIN_SIZE_ATTEMPT_TIMEOUT_MS)
    const fail = (err: unknown): void => {
      clearTimeout(timer)
      reject(err)
    }
    try {
      invoke<void>('report_window_min_width', { width: size.width, height: size.height, seq, epoch: st.epoch }).then(() => {
        if (st.seq === seq) st.delivered = size
        clearTimeout(timer)
        resolve()
      }, fail)
    } catch (err) {
      fail(err)
    }
  })
}

/**
 * Resolves when the reports handed over so far all finish — at the latest after `MIN_SIZE_WAIT_CAP_MS` (never rejects); immediately if none. The first-screen notice (`theme/firstPaint.ts`) is sent after this.
 */
export function minWindowSizeSettled(): Promise<void> {
  const chain = report.chain
  return new Promise(resolve => {
    const timer = setTimeout(resolve, MIN_SIZE_WAIT_CAP_MS)
    void chain.then(() => {
      clearTimeout(timer)
      resolve()
    })
  })
}

/** Test only — reset module state (same as reloading the page: `epoch` re-read from `performance.timeOrigin`, sequence 0, retries of the previous state stop). Never call from production code. */
export function __resetMinSizeReportForTest(): void {
  report.gen += 1
  report = fresh()
}
