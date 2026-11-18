// Minimum window size report of TRD S26-window-chrome §7-7 — one at a time · only the last value · the same value is not sent · hand over again after a failure · deadline of an attempt
//   with no answer · a new sequence number (`seq`) on every send · `epoch` per page · the end the first-screen notice waits for (including the cap). invoke is fake.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args) }))

import {
  __resetMinSizeReportForTest,
  MIN_SIZE_ATTEMPT_TIMEOUT_MS,
  MIN_SIZE_WAIT_CAP_MS,
  minWindowSizeSettled,
  reportMinWindowSize,
} from './minSizeReport'

// The first backoff of the `retryAsync` default — after an attempt released at the deadline, the next turn goes out this much later.
const FIRST_BACKOFF_MS = 150

interface SentArgs {
  width: number
  height: number
  seq: number
  epoch: number
}

const args = (): SentArgs[] => invokeMock.mock.calls.map(c => c[1] as SentArgs)
const sent = () => args().map(({ width, height }) => ({ width, height }))
const seqs = () => args().map(a => a.seq)
const flush = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

/** Holds the next report — the returned function answers (rejects if `fail`). */
function holdNext(): (fail?: boolean) => Promise<void> {
  let answer: (fail: boolean) => void = () => {}
  invokeMock.mockImplementationOnce(
    () => new Promise((resolve, reject) => (answer = fail => (fail ? reject(new Error('refused (test)')) : resolve(undefined)))),
  )
  return async (fail = false) => {
    answer(fail)
    await flush()
  }
}

// Swaps in this page's `performance.timeOrigin` — `afterEach` restores the original property.
const ownTimeOrigin = Object.getOwnPropertyDescriptor(performance, 'timeOrigin')
function setTimeOrigin(ms: number): void {
  Object.defineProperty(performance, 'timeOrigin', { value: ms, configurable: true })
}

beforeEach(() => {
  __resetMinSizeReportForTest()
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
})

afterEach(() => {
  if (ownTimeOrigin) Object.defineProperty(performance, 'timeOrigin', ownTimeOrigin)
  else delete (performance as { timeOrigin?: number }).timeOrigin
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('reportMinWindowSize', () => {
  it('sends as report_window_min_width { width, height, seq, epoch } — first sequence 1 · epoch = integer ms of the page timeOrigin · the same value is not sent again', async () => {
    setTimeOrigin(1_761_000_000_123.75)
    __resetMinSizeReportForTest()
    reportMinWindowSize({ width: 286, height: 64 })
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    expect(invokeMock.mock.calls).toEqual([
      ['report_window_min_width', { width: 286, height: 64, seq: 1, epoch: 1_761_000_000_123 }],
    ])
  })

  it('epoch is one per page — on reading again (reset) the sequence restarts from 1 at that page\'s epoch', async () => {
    setTimeOrigin(1_761_000_000_000.5)
    __resetMinSizeReportForTest()
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()

    setTimeOrigin(1_761_000_005_000.25)
    __resetMinSizeReportForTest()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    expect(args().map(a => [a.epoch, a.seq])).toEqual([
      [1_761_000_000_000, 1],
      [1_761_000_000_000, 2],
      [1_761_000_005_000, 1],
    ])
  })

  it('one at a time — of the values taken while in flight only the last is sent after the response', async () => {
    const answer = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    reportMinWindowSize({ width: 288, height: 64 })
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    expect(sent()).toEqual([{ width: 286, height: 64 }])

    await answer()
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2])
  })

  it('a value equal to the reached value — if it was pushed out before sending another value taken in between — is not sent', async () => {
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    reportMinWindowSize({ width: 295, height: 64 })
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    expect(sent()).toEqual([{ width: 286, height: 64 }])
  })

  it('even returning to the earlier reached value, if another value was sent in between it is sent again — the answer of that value (reached · refused) is not weighed', async () => {
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    let answer = holdNext()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    reportMinWindowSize({ width: 286, height: 64 })
    await answer()
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2, 3])

    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockClear()
    answer = holdNext()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    reportMinWindowSize({ width: 286, height: 64 })
    await answer(true)
    await minWindowSizeSettled()
    expect(sent()).toEqual([
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([4, 5])
  })

  it('an attempt that gets no answer is put on a deadline — a newer value goes out within a bounded time with a new sequence · the late answer of an old sequence does not change 「reached」', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const late = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    reportMinWindowSize({ width: 295, height: 64 })
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS - 1)
    expect(sent()).toEqual([{ width: 286, height: 64 }])
    await vi.advanceTimersByTimeAsync(1 + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2])

    await late()
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS * 10)
    expect(sent()).toHaveLength(2)

    // If the late 286 was recorded as 「reached」, this report is skipped.
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2, 3])
  })

  it('even if late answers mix in, the largest sequence carries the last value · only that sequence\'s answer becomes 「reached」', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const lateFirst = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    const lateSecond = holdNext()
    reportMinWindowSize({ width: 288, height: 64 })
    reportMinWindowSize({ width: 295, height: 64 })
    // 286 is released at the deadline and 295 goes out (288 was pushed out) — that 295 is also released at the deadline and goes out again.
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 295, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2, 3])

    await lateSecond()
    await lateFirst()
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS * 10)
    expect(sent()).toHaveLength(3)

    // 「reached」 = exactly 295 of sequence 3 — if it went back to 286 of the late sequence 1, this report is skipped.
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    expect(sent()).toHaveLength(4)
    expect(args()[3]).toMatchObject({ width: 286, height: 64, seq: 4 })
  })

  it('if the answer of an attempt put on the deadline comes before the retry — that sequence is still the last so it counts as 「reached」 and is not sent again', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const late = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS)
    await late()
    await vi.advanceTimersByTimeAsync(FIRST_BACKOFF_MS * 10)
    expect(sent()).toEqual([{ width: 286, height: 64 }])
  })

  it('if there is no newer value the same value put on the deadline is sent again with a new sequence', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2])
  })

  it('after an attempt that passed the deadline the shell\'s value is treated as unknown — even returning to the earlier reached value it is sent again', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    holdNext()
    reportMinWindowSize({ width: 295, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
  })

  it('when retries are exhausted it leaves an error · if the same value is taken again it is sent again — a new sequence on each retry', async () => {
    vi.useFakeTimers()
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockImplementation(async () => {
      throw new Error('refused (test)')
    })
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.runAllTimersAsync()
    expect(seqs()).toEqual([1, 2, 3, 4])
    expect(error).toHaveBeenCalledTimes(1)

    invokeMock.mockImplementation(async () => undefined)
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.runAllTimersAsync()
    expect(seqs()).toEqual([1, 2, 3, 4, 5])
  })
})

describe('minWindowSizeSettled — the end the first-screen notice waits for', () => {
  it('if no report is taken then at once · if there is one it releases after its response', async () => {
    let settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await flush()
    expect(settled).toBe(true)

    const answer = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await flush()
    expect(settled).toBe(false)
    await answer()
    expect(settled).toBe(true)
  })

  it('even with no response it releases after the cap — it does not block the first-screen notice', async () => {
    vi.useFakeTimers()
    invokeMock.mockImplementation(() => new Promise(() => {}))
    reportMinWindowSize({ width: 286, height: 64 })
    let settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await vi.advanceTimersByTimeAsync(MIN_SIZE_WAIT_CAP_MS - 1)
    expect(settled).toBe(false)
    await vi.advanceTimersByTimeAsync(1)
    expect(settled).toBe(true)
  })

  it('if the report is refused it does not wait out all retries — it releases within the cap', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockImplementation(async () => {
      throw new Error('refused (test)')
    })
    reportMinWindowSize({ width: 286, height: 64 })
    let settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await vi.advanceTimersByTimeAsync(MIN_SIZE_WAIT_CAP_MS)
    expect(settled).toBe(true)
  })
})
