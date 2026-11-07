// TRD S26-window-chrome §7-7: minimum window size report (`report_window_min_width`) — sends to the shell the value this window's title row counted (`metrics.ts`
//   `minWindowSize`) (only after the window number is received — `TitleRow`). Within the same synchronous command the shell applies that value as the OS minimum size
//   (normal-size windows only) and, if it is the first report after the window was opened, widens the window once and then answers (`src-tauri/src/chrome` — `report_min_size`). So the
//   first-screen notice waits for the answer of this report (`minWindowSizeSettled` — so the widening is finished before the first screen is shown).
// Why the state is module scope: there is one report per webview (window), and a reload re-executes the module and resets it together (`uiMetricsReport.ts`
//   is the same).
// ★One at a time · only the last value★ — a value taken while sending is sent after that response, and if a newer value is taken meanwhile the old value is not sent.
// ★An attempt whose response never arrives is released at the deadline (`MIN_SIZE_ATTEMPT_TIMEOUT_MS`)★ — otherwise all later reports are blocked behind it.
//   A released attempt may reach the shell later than the next attempt (head of `util/latestValueReporter.ts`) — so every send (including retries) carries a new
//   sequence number `seq` and this page's `epoch`, and the shell discards one with an older `epoch` or, within the same `epoch`, a sequence number not newer than the one accepted
//   (`chrome::book` `record_min`). Regardless of arrival order, the shell is left with the largest sequence number of the newest page = the last sent
//   value.
// What is believed to be the shell's current value = only the value whose largest sequence number got a success response — it becomes unknown the moment a new sequence number is sent, and the answer of an older sequence
//   (even if it came after the deadline) changes nothing.

import { invoke } from '@tauri-apps/api/core'

import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

export interface MinWindowSize {
  width: number
  height: number
}

// Cap on how long the first-screen notice waits for the report — comfortably shorter than the shell's show deadline (1500 ms). A normal answer takes a few ms, and it waits through one retry
//   after the first failure (after 150 ms — `retryAsync` default). If exceeded, the first screen is announced and the report finishes in the background — that window may be widened after it is shown
//   (the same look as what TRD T1 accepted).
export const MIN_SIZE_WAIT_CAP_MS = 300

// Cap on how long one attempt waits for an answer — if exceeded, that attempt counts as failed and is retried (if a newer value was taken, that value is sent instead of retrying). A normal
//   answer takes a few ms, so this value is used only to reclaim the report line from a stuck response. If short, even one slow response causes the same value to be sent twice.
export const MIN_SIZE_ATTEMPT_TIMEOUT_MS = 1000

interface MinSizeReportState {
  /**
   * This page's mark = `Math.floor(performance.timeOrigin)` (wall-clock ms when the page was loaded) — it grows with every reload so the shell can tell
   * the order of pages (a late report from an old page cannot overwrite a new page's value). When a larger `epoch` arrives the shell counts sequence numbers anew (on reload
   * `seq` starts again from 1). If two pages have `timeOrigin` in the same ms they mix as one page — not handled.
   */
  epoch: number
  /**
   * The last sequence number sent — rises by one on every send (including retries). First sequence number = 1. Exceeding u32 is not handled — one
   * page does not send 4.2 billion times.
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
        if (st.gen === gen) console.warn(`[minSizeReport] report_window_min_width(${size.width}x${size.height}) 재시도 ${n}:`, err)
      },
    })
  } catch (err) {
    if (err instanceof RetryCancelledError) return
    if (st.gen === gen) st.latest = null
    console.error(`[minSizeReport] report_window_min_width(${size.width}x${size.height}) 최종 실패(재시도 소진):`, err)
  }
}

/**
 * Send once — not sent if the shell's current value is already `size`. Carries a new sequence number and rejects if there is no answer within `MIN_SIZE_ATTEMPT_TIMEOUT_MS`.
 * A success answer is recorded as 「닿음」 only when that sequence is still the last sequence (also for an answer that came after the deadline).
 */
function attempt(st: MinSizeReportState, size: MinWindowSize): Promise<void> {
  if (st.delivered !== null && same(st.delivered, size)) return Promise.resolve()
  const seq = ++st.seq
  st.delivered = null
  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`응답 없음(${MIN_SIZE_ATTEMPT_TIMEOUT_MS} ms)`)), MIN_SIZE_ATTEMPT_TIMEOUT_MS)
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
 * Resolves when the reports handed over so far all finish — at the latest after `MIN_SIZE_WAIT_CAP_MS` (does not reject). If nothing is in hand it resolves immediately.
 * The first-screen notice (`theme/firstPaint.ts`) is sent after this.
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

/** Test only — reset module state (the same as reloading the page — `epoch` is re-read from `performance.timeOrigin` · sequence number 0 · retries of the previous state also stop). Do not call from production code. */
export function __resetMinSizeReportForTest(): void {
  report.gen += 1
  report = fresh()
}
