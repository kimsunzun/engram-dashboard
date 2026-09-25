// windowCanvasReport 단위테스트 — `lastSent` 구독(ADR-0227). 구분선 드래그의 허용 범위(`useSplitDrag`)가 소비한다.
// 무엇을 언제 보내나(한 번에 하나·합치기·재시도)는 `WindowLayout.test.tsx` 가 잰다. 여기서는 알림만 본다.

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
  it('성공 응답으로 lastSent 가 바뀔 때마다 새 값으로 알린다', async () => {
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

    // 같은 크기는 보내지도 알리지도 않는다.
    await report(200, 100)
    expect(reportCalls()).toHaveLength(2)
    expect(seen).toHaveLength(2)
  })

  it('실패에는 알리지 않고, 재시도가 성공하면 그때 한 번 알린다', async () => {
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

  it('해제하면 더 알리지 않는다 — 같은 함수를 두 번 걸어도 해제는 각자다', async () => {
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

  it('테스트 초기화는 값이 있었을 때만 null 로 알린다', async () => {
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

  it('초기화로 버려진 상태의 늦은 성공은 지금 구독자에게 알리지 않는다', async () => {
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

  it('구독자가 던져도 보고 실패로 치지 않는다 — 다시 보내지 않고 다른 구독자도 받는다', async () => {
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
    expect(err).toHaveBeenCalledWith('[windowCanvasReport] 캔버스 구독자 오류:', expect.any(Error))
  })
})
