// windowCanvasReport unit test — the `lastSent` subscription (ADR-0227). The divider drag's allowed range (`useSplitDrag`) consumes it.
// What is sent and when (one at a time, coalescing, retry) is measured by `WindowLayout.test.tsx`. Here only notifications are checked.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))

import {
  __resetCanvasReportForTest,
  getCanvasReport,
  submitCanvasSize,
  subscribeCanvasReport,
  type CanvasPx,
} from './windowCanvasReport'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

async function report(w: number, h: number): Promise<void> {
  submitCanvasSize(getCanvasReport(), { w, h })
  await ticks()
}

const reportCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'report_window_canvas')

let unsubs: Array<() => void>
const listen = (cb: (size: CanvasPx | null) => void): (() => void) => {
  const off = subscribeCanvasReport(cb)
  unsubs.push(off)
  return off
}

beforeEach(() => {
  unsubs = []
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
  __resetCanvasReportForTest()
})
afterEach(() => {
  unsubs.forEach(off => off())
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('subscribeCanvasReport', () => {
  it('on every change of lastSent by a success response it notifies with the new value', async () => {
    const seen: Array<CanvasPx | null> = []
    listen(s => seen.push(s))

    await report(100, 50)
    expect(seen).toEqual([{ w: 100, h: 50 }])
    expect(getCanvasReport().lastSent).toBe(seen[0])

    await report(200, 100)
    expect(seen).toEqual([
      { w: 100, h: 50 },
      { w: 200, h: 100 },
    ])

    // The same size is neither sent nor notified.
    await report(200, 100)
    expect(reportCalls()).toHaveLength(2)
    expect(seen).toHaveLength(2)
  })

  it('on failure it does not notify, and when the retry succeeds it notifies once then', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('not yet'))
    const cb = vi.fn()
    listen(cb)

    await report(100, 50)
    expect(reportCalls()).toHaveLength(1)
    expect(cb).not.toHaveBeenCalled()
    expect(getCanvasReport().lastSent).toBeNull()

    await vi.advanceTimersByTimeAsync(150)
    await ticks()
    expect(reportCalls()).toHaveLength(2)
    expect(cb).toHaveBeenCalledTimes(1)
    expect(cb).toHaveBeenCalledWith({ w: 100, h: 50 })
  })

  it('on release it notifies no more — even hooking the same function twice, release is each its own', async () => {
    const cb = vi.fn()
    const off1 = listen(cb)
    const off2 = listen(cb)

    await report(100, 50)
    expect(cb).toHaveBeenCalledTimes(2)

    off1()
    await report(200, 100)
    expect(cb).toHaveBeenCalledTimes(3)

    off2()
    off2()
    await report(300, 150)
    expect(cb).toHaveBeenCalledTimes(3)
  })

  it('a test reset notifies null only when there was a value', async () => {
    const cb = vi.fn()
    listen(cb)

    __resetCanvasReportForTest()
    expect(cb).not.toHaveBeenCalled()

    await report(100, 50)
    cb.mockClear()
    __resetCanvasReportForTest()
    expect(cb).toHaveBeenCalledTimes(1)
    expect(cb).toHaveBeenCalledWith(null)
    expect(getCanvasReport().lastSent).toBeNull()
  })

  it('a late success of a state discarded by reset is not notified to the current subscribers', async () => {
    let finish: () => void = () => {}
    invokeMock.mockImplementationOnce(
      () =>
        new Promise<void>(res => {
          finish = res
        }),
    )
    const cb = vi.fn()
    listen(cb)

    await report(100, 50)
    __resetCanvasReportForTest()
    finish()
    await ticks()

    expect(cb).not.toHaveBeenCalled()
    expect(getCanvasReport().lastSent).toBeNull()
  })

  it('a subscriber throwing is not counted as a report failure — it does not resend and other subscribers also receive', async () => {
    vi.useFakeTimers()
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    listen(() => {
      throw new Error('subscriber bug')
    })
    const cb = vi.fn()
    listen(cb)

    await report(100, 50)
    await vi.advanceTimersByTimeAsync(5000)
    await ticks()

    expect(reportCalls()).toHaveLength(1)
    expect(cb).toHaveBeenCalledTimes(1)
    expect(getCanvasReport().lastSent).toEqual({ w: 100, h: 50 })
    expect(err).toHaveBeenCalledWith('[windowCanvasReport] canvas subscriber error:', expect.any(Error))
  })
})
