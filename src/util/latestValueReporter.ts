// Sender-side helper for measurement reporting — one at a time · the same value is not sent · debounce (ADR-0227 · TRD S21-slot-docking §5-3). The window canvas size (`windowCanvasReport.ts`) uses it. The value left in the shell = the last successful report.
// ★One invoke per reporter at a time★: if the shell handled two overlapping requests in reverse arrival order its value would stay stale until the next report, and WebView2's overlapping-request order is undocumented. One at a time makes the response order the shell's apply order, so even a cancelled job's success can be trusted as the shell's current value. Cost: if a response never comes, that reporter stops there.
// A value arriving while one is in flight goes into a single pending field (a new value overwrites it); only the last value is sent after the response.
// A failure retries itself with a bounded retry (`retryAsync`) — measurements come only on change, so waiting for the next one may mean never; if a different value arrives meanwhile, the old value's retry stops.

import { retryAsync, RetryCancelledError } from './retryInvoke'

/** Debounce (ms) of measurement reports — reports with frequent observations (canvas size · window frame) use it together. */
export const REPORT_DEBOUNCE_MS = 100

export interface LatestValueReporterOptions<T> {
  /** Send to the shell — a refusal throws. */
  send: (value: T) => Promise<unknown>
  same: (a: T, b: T) => boolean
  /** Log head (`[log]`). */
  log: string
  /** Report name to put in the log — e.g. `report_window_canvas(800x600)`. */
  describe: (value: T) => string
  /**
   * Called after the shell answered success with a value different from `lastSent` and `lastSent` changed — not on a failure or a repeat success of the same value. ★Do not throw★: it runs inside response handling, and a throw counts as a report failure and resends the same value.
   */
  onAccepted?: (value: T) => void
}

export class LatestValueReporter<T extends object> {
  private readonly opts: LatestValueReporterOptions<T>
  /** A value waiting for its turn to be sent. Only one is kept and a new value overwrites it. */
  private pending: T | null = null
  private sent: T | null = null
  /** The job sending now (including retry waits). null when cancelled. */
  private job: { value: T; gen: number } | null = null
  /** Is invoke waiting for a response — counted separately from the retry's backoff wait. */
  private inAir = false
  /** Job generation — when it changes, the earlier job's retry tries no more. */
  private gen = 0
  private timer: ReturnType<typeof setTimeout> | null = null

  constructor(opts: LatestValueReporterOptions<T>) {
    this.opts = opts
  }

  /** The last value the shell answered success for — a repeat success of the same value does not replace the reference. */
  get lastSent(): T | null {
    return this.sent
  }

  /**
   * One observation before debounce — if the value being sent · pending differs from it, withdraw without waiting for the debounce; waiting would let an old retry or the turn after a response send a stale value once more.
   */
  observe(value: T): void {
    if (this.job && !this.opts.same(this.job.value, value)) this.cancelJob()
    if (this.pending !== null && !this.opts.same(this.pending, value)) this.pending = null
  }

  /** Put it in the turn to be sent immediately (no debounce). If equal to `lastSent`, do not send. */
  submit(value: T): void {
    this.observe(value)
    this.pending = value
    this.pump()
  }

  /**
   * After `delayMs` call `measure` and `submit` that value; a call in between resets the timer and only the last `measure` is used. Measuring at timer end reads the DOM at that time; a `null` measure sends nothing.
   */
  schedule(measure: () => T | null, delayMs: number = REPORT_DEBOUNCE_MS): void {
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = setTimeout(() => {
      this.timer = null
      const value = measure()
      if (value !== null) this.submit(value)
    }, delayMs)
  }

  /**
   * When detaching the observation — discard the debounce · pending value and stop retrying; keep the in-flight invoke and `lastSent` (that response sets the shell's current value, and the reattached side waits for it so one at a time holds).
   */
  release(): void {
    if (this.timer !== null) clearTimeout(this.timer)
    this.timer = null
    this.pending = null
    this.cancelJob()
  }

  private cancelJob(): void {
    this.gen += 1
    this.job = null
  }

  /** Send out the pending value. If there is an invoke in flight, it is called again after that response (the `finally` of `start`). */
  private pump(): void {
    if (this.inAir || this.pending === null) return
    const value = this.pending
    this.pending = null
    if (this.job) {
      if (this.opts.same(this.job.value, value)) return
      this.cancelJob()
    }
    if (this.sent !== null && this.opts.same(this.sent, value)) return
    this.start(value)
  }

  private start(value: T): void {
    const gen = ++this.gen
    this.job = { value, gen }
    const attempt = async (): Promise<void> => {
      this.inAir = true
      try {
        await this.opts.send(value)
        // If the value is the same, do not replace the object — so the reference of the value subscribers read does not change without a notification.
        if (this.sent === null || !this.opts.same(this.sent, value)) {
          this.sent = value
          this.opts.onAccepted?.(value)
        }
      } finally {
        this.inAir = false
        this.pump()
      }
    }
    void retryAsync(attempt, {
      isCancelled: () => this.gen !== gen,
      onRetry: (err, n) => {
        if (this.gen === gen) console.warn(`[${this.opts.log}] ${this.opts.describe(value)} retry ${n}:`, err)
      },
    })
      .then(() => {
        if (this.gen === gen) this.job = null
      })
      .catch(err => {
        if (err instanceof RetryCancelledError || this.gen !== gen) return
        this.job = null
        console.error(`[${this.opts.log}] ${this.opts.describe(value)} finally failed (retries exhausted):`, err)
      })
  }
}
