// dockChromeReport 단위테스트 — 측정 → 보고 값(접힌 머리 뺌)과 보내기(한 번에 하나 · 마지막 값이 덮음 · 재시도 ·
//   디바운스). 언제 재는지(관측 · 떼어진 표시)는 부르는 쪽(`WindowLayout`)의 시험 몫이다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))

import type { DockChrome } from '../../../../src-tauri/bindings/DockChrome'
import {
  __resetDockChromeReportForTest,
  cancelDockChromeReport,
  dockChromeOf,
  DOCK_CHROME_DEBOUNCE_MS,
  scheduleDockChromeReport,
  submitDockChrome,
  type RectLike,
} from './dockChromeReport'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

const r = (left: number, top: number, right: number, bottom: number): RectLike => ({ left, top, right, bottom })

const chrome = (originY: number): DockChrome => ({
  canvas_origin: { x: 0, y: originY },
  tab_bar: { x0: 0, y0: 0, x1: 640, y1: 28 },
  tabs: [{ view_id: 'a', rect: { x0: 4, y0: 0, x1: 120.25, y1: 28 } }],
  scroll: null,
})

const sent = () => invokeMock.mock.calls.filter(c => c[0] === 'report_dock_chrome').map(c => c[1])

function holdInvoke(): { finish: () => void; fail: (e: unknown) => void } {
  const h = { finish: () => {}, fail: (_e: unknown) => {} }
  invokeMock.mockImplementationOnce(
    () =>
      new Promise<void>((res, rej) => {
        h.finish = res
        h.fail = rej
      }),
  )
  return h
}

beforeEach(() => {
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
  __resetDockChromeReportForTest()
})
afterEach(() => {
  __resetDockChromeReportForTest()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('dockChromeOf', () => {
  it('캔버스 왼쪽 위 = 원점 · 탭바 · 머리를 경계 꼴로 — 접힌 머리는 뺀다', () => {
    expect(
      dockChromeOf({
        canvas: r(0, 28.5, 800, 600),
        tabBar: r(0, 0, 800, 28),
        heads: [
          { viewId: 'a', rect: r(4, 0, 120.25, 28), collapsed: false },
          { viewId: 'dragged', rect: r(120.25, 0, 220, 28), collapsed: true },
          { viewId: 'c', rect: r(-50, 0, 2, 28), collapsed: false },
        ],
      }),
    ).toEqual({
      canvas_origin: { x: 0, y: 28.5 },
      tab_bar: { x0: 0, y0: 0, x1: 800, y1: 28 },
      tabs: [
        { view_id: 'a', rect: { x0: 4, y0: 0, x1: 120.25, y1: 28 } },
        { view_id: 'c', rect: { x0: -50, y0: 0, x1: 2, y1: 28 } },
      ],
      scroll: null,
    })
  })

  it('유한하지 않은 수가 있으면 null — 접힌 머리의 수는 보지 않는다', () => {
    const base = { canvas: r(0, 28, 800, 600), tabBar: r(0, 0, 800, 28) }
    expect(dockChromeOf({ ...base, heads: [{ viewId: 'a', rect: r(0, 0, Number.NaN, 28), collapsed: false }] })).toBe(
      null,
    )
    expect(dockChromeOf({ ...base, canvas: r(0, Infinity, 800, 600), heads: [] })).toBeNull()
    expect(
      dockChromeOf({ ...base, heads: [{ viewId: 'a', rect: r(0, 0, Number.NaN, 28), collapsed: true }] }),
    ).not.toBeNull()
  })
})

describe('submitDockChrome', () => {
  it('셸 인자 꼴 그대로 보낸다 — { chrome }', async () => {
    submitDockChrome(chrome(28))
    await ticks()
    expect(sent()).toEqual([{ chrome: chrome(28) }])
  })

  it('한 번에 하나 — 날아가는 동안 온 값은 마지막 것만 응답 뒤에 간다', async () => {
    const first = holdInvoke()
    submitDockChrome(chrome(1))
    submitDockChrome(chrome(2))
    submitDockChrome(chrome(3))
    await ticks()
    expect(sent()).toEqual([{ chrome: chrome(1) }])

    first.finish()
    await ticks()
    expect(sent()).toEqual([{ chrome: chrome(1) }, { chrome: chrome(3) }])
  })

  it('셸이 마지막으로 받은 값과 같으면 보내지 않는다', async () => {
    submitDockChrome(chrome(28))
    await ticks()
    submitDockChrome(chrome(28))
    await ticks()
    expect(sent()).toHaveLength(1)
  })

  it('날아가는 동안 같은 값이 다시 와도 한 번만 간다', async () => {
    const first = holdInvoke()
    submitDockChrome(chrome(28))
    submitDockChrome(chrome(28))
    first.finish()
    await ticks()
    expect(sent()).toHaveLength(1)
  })

  it('실패하면 스스로 재시도하고, 그 사이 새 값이 오면 옛 값은 다시 보내지 않는다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('window not in model yet'))
    submitDockChrome(chrome(1))
    await ticks()
    expect(sent()).toEqual([{ chrome: chrome(1) }])

    // backoff 대기 중(날아가는 invoke 없음)이라 새 값은 곧바로 간다.
    submitDockChrome(chrome(2))
    await ticks()
    expect(sent()).toEqual([{ chrome: chrome(1) }, { chrome: chrome(2) }])

    await vi.advanceTimersByTimeAsync(2000)
    expect(sent()).toEqual([{ chrome: chrome(1) }, { chrome: chrome(2) }])
  })

  it('실패한 값 하나뿐이면 backoff 뒤 같은 값을 다시 보낸다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('not yet'))
    submitDockChrome(chrome(1))
    await ticks()
    await vi.advanceTimersByTimeAsync(150)
    expect(sent()).toEqual([{ chrome: chrome(1) }, { chrome: chrome(1) }])
  })
})

describe('scheduleDockChromeReport', () => {
  it('디바운스 끝에 마지막 측정 함수 하나만 부른다', async () => {
    vi.useFakeTimers()
    const m1 = vi.fn(() => chrome(1))
    const m2 = vi.fn(() => chrome(2))
    scheduleDockChromeReport(m1)
    await vi.advanceTimersByTimeAsync(DOCK_CHROME_DEBOUNCE_MS - 1)
    scheduleDockChromeReport(m2)
    await vi.advanceTimersByTimeAsync(DOCK_CHROME_DEBOUNCE_MS - 1)
    expect(sent()).toEqual([])

    await vi.advanceTimersByTimeAsync(1)
    expect(m1).not.toHaveBeenCalled()
    expect(m2).toHaveBeenCalledTimes(1)
    expect(sent()).toEqual([{ chrome: chrome(2) }])
  })

  it('측정이 null 이거나 던지면 보내지 않는다', async () => {
    vi.useFakeTimers()
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    scheduleDockChromeReport(() => null)
    await vi.advanceTimersByTimeAsync(DOCK_CHROME_DEBOUNCE_MS)
    scheduleDockChromeReport(() => {
      throw new Error('detached node')
    })
    await vi.advanceTimersByTimeAsync(DOCK_CHROME_DEBOUNCE_MS)
    expect(sent()).toEqual([])
    expect(err).toHaveBeenCalledTimes(1)
  })
})

describe('cancelDockChromeReport', () => {
  it('디바운스 · 대기 값을 버린다', async () => {
    vi.useFakeTimers()
    const first = holdInvoke()
    submitDockChrome(chrome(1))
    submitDockChrome(chrome(2))
    const m = vi.fn(() => chrome(3))
    scheduleDockChromeReport(m)

    cancelDockChromeReport()
    first.finish()
    await vi.advanceTimersByTimeAsync(DOCK_CHROME_DEBOUNCE_MS * 2)
    expect(m).not.toHaveBeenCalled()
    expect(sent()).toEqual([{ chrome: chrome(1) }])
  })

  it('날아가던 invoke 의 성공은 남는다 — 다시 붙은 쪽이 같은 값을 다시 보내지 않는다', async () => {
    const first = holdInvoke()
    submitDockChrome(chrome(1))
    cancelDockChromeReport()
    first.finish()
    await ticks()

    submitDockChrome(chrome(1))
    await ticks()
    expect(sent()).toHaveLength(1)
  })

  it('날아가던 invoke 가 끝나기 전에 다시 붙어 보낸 값은 그 응답 뒤에 간다(한 번에 하나)', async () => {
    const first = holdInvoke()
    submitDockChrome(chrome(1))
    cancelDockChromeReport()
    submitDockChrome(chrome(2))
    await ticks()
    expect(sent()).toHaveLength(1)

    first.fail(new Error('gone'))
    await ticks()
    expect(sent()).toEqual([{ chrome: chrome(1) }, { chrome: chrome(2) }])
  })
})
