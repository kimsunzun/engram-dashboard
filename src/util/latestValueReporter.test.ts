// latestValueReporter unit test — one at a time · the last value overwrites · the same value is not sent · an observation withdraws a stale value · retry · detach ·
//   debounce. The flow of canvas size reporting is measured by `WindowLayout.test.tsx`.

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

import { LatestValueReporter, REPORT_DEBOUNCE_MS } from './latestValueReporter'

interface V {
  n: number
}
const v = (n: number): V => ({ n })

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

let send: Mock<(value: V) => Promise<unknown>>
let accepted: V[]
let rep: LatestValueReporter<V>

const sentNs = (): number[] => send.mock.calls.map(c => c[0].n)

/** The test finishes the next single send by hand. */
function holdSend(): { finish: () => void; fail: (e: unknown) => void } {
  const h = { finish: () => {}, fail: (_e: unknown) => {} }
  send.mockImplementationOnce(
    () =>
      new Promise<void>((res, rej) => {
        h.finish = res
        h.fail = rej
      }),
  )
  return h
}

beforeEach(() => {
  send = vi.fn(async (_value: V): Promise<unknown> => undefined)
  accepted = []
  rep = new LatestValueReporter<V>({
    send: value => send(value),
    same: (a, b) => a.n === b.n,
    log: 'test',
    describe: value => `report(${value.n})`,
    onAccepted: value => accepted.push(value),
  })
})
afterEach(() => {
  rep.release()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('LatestValueReporter', () => {
  it('submit sends right away, and on success lastSent · onAccepted — the same value is not sent again', async () => {
    const a = v(1)
    rep.submit(a)
    await ticks()
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toBe(a)
    expect(accepted).toEqual([a])

    rep.submit(v(1))
    await ticks()
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toBe(a)
    expect(accepted).toHaveLength(1)
  })

  it('one at a time — of the values that came while in flight, only the last one is sent after the response', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(2))
    rep.submit(v(3))
    await ticks()
    expect(sentNs()).toEqual([1])

    first.finish()
    await ticks()
    expect(sentNs()).toEqual([1, 3])
    expect(rep.lastSent).toEqual(v(3))
    expect(accepted).toEqual([v(1), v(3)])
  })

  it('when a value equal to the one in flight waits its turn, it is not sent again after the response', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(1))
    first.finish()
    await ticks()
    expect(sentNs()).toEqual([1])
  })

  it("a cancelled value's success also becomes lastSent — the response order is the shell's apply order", async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.observe(v(2))
    first.finish()
    await ticks()
    expect(rep.lastSent).toEqual(v(1))
    expect(sentNs()).toEqual([1])
  })

  it('on failure it retries by itself and only a succeeded value becomes lastSent — when exhausted, console.error once', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    send.mockRejectedValue(new Error('refused'))

    rep.submit(v(1))
    await vi.advanceTimersByTimeAsync(150 + 300 + 600 + 10)
    expect(sentNs()).toEqual([1, 1, 1, 1])
    expect(rep.lastSent).toBeNull()
    expect(err).toHaveBeenCalledTimes(1)
    expect(err).toHaveBeenCalledWith('[test] report(1) finally failed (retries exhausted):', expect.any(Error))

    // A failed value is not lastSent, so even the same value goes out again.
    send.mockReset()
    send.mockImplementation(async () => undefined)
    rep.submit(v(1))
    await ticks()
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toEqual(v(1))
  })

  it('when observe gives a different value, it stops the retries of the old value and drops the waiting value — a cancel is not a failure log', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    send.mockRejectedValueOnce(new Error('refused'))

    rep.submit(v(1))
    await ticks()
    rep.observe(v(2))
    await vi.advanceTimersByTimeAsync(3000)
    expect(sentNs()).toEqual([1])
    expect(err).not.toHaveBeenCalled()

    const held = holdSend()
    rep.submit(v(3))
    rep.submit(v(4))
    rep.observe(v(5))
    held.finish()
    await ticks()
    expect(sentNs()).toEqual([1, 3])
  })

  it('when observe gives the same value, it reaps nothing', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(2))
    rep.observe(v(2))
    first.finish()
    await ticks()
    expect(sentNs()).toEqual([1, 2])
  })

  it('release — stops the debounce · waiting value · retries, and the response in flight sets lastSent', async () => {
    vi.useFakeTimers()
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(2))
    rep.schedule(() => v(3))
    rep.release()
    first.finish()
    await vi.advanceTimersByTimeAsync(3000)
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toEqual(v(1))
  })

  it('schedule — calling again during the debounce calls only the last measure at the end of the timer, and sends nothing for null', async () => {
    vi.useFakeTimers()
    const early = vi.fn(() => v(1))
    rep.schedule(early)
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS - 1)
    rep.schedule(() => v(2))
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS - 1)
    expect(sentNs()).toEqual([])
    await vi.advanceTimersByTimeAsync(1)
    expect(early).not.toHaveBeenCalled()
    expect(sentNs()).toEqual([2])

    rep.schedule(() => null, 10)
    await vi.advanceTimersByTimeAsync(10)
    expect(sentNs()).toEqual([2])
  })
})
