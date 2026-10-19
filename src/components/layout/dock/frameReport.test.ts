// frameReport 단위테스트 — 측정 → 보고 값(`tab_bar` = 탭 끌기 영역)과 보내기 배선.
//   보내기 규칙(한 번에 하나 · 재시도)은 `util/latestValueReporter.test.ts` 가 잰다. 언제 재는지는 부르는 쪽의 시험 몫이다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))

import { REPORT_DEBOUNCE_MS } from '../../../util/latestValueReporter'
import type { StripBounds } from '../tabStrip/reorderGeometry'
import {
  __resetFrameReportForTest,
  cancelFrameReport,
  dragAreaRect,
  frameChromeOf,
  scheduleFrameReport,
  submitFrame,
  type FrameMeasure,
} from './frameReport'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

// 탭 셋(폭 100 · 120 · 80) — 셋째는 숨김.
const bounds: StripBounds = {
  tabs: [
    { x0: 0, x1: 100 },
    { x0: 100, x1: 220 },
    { x0: 220, x1: 300 },
  ],
  visible: [true, true, false],
  lastVisible: 1,
}

const frame = (over: { canvasTop?: number; available?: number | null } = {}): FrameMeasure => ({
  canvas: { left: 0, top: over.canvasTop ?? 32 },
  area: { left: 8, top: 2, bottom: 30, available: over.available === undefined ? 600 : over.available, bounds },
})

const sent = () => invokeMock.mock.calls.filter(c => c[0] === 'report_dock_chrome').map(c => c[1])

beforeEach(() => {
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
  __resetFrameReportForTest()
})
afterEach(() => {
  __resetFrameReportForTest()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('dragAreaRect', () => {
  it('x = 첫 탭 왼쪽 ~ 그 + 잰 available · y = 탭바 줄', () => {
    expect(dragAreaRect(frame().area)).toEqual({ x0: 8, y0: 2, x1: 608, y1: 30 })
  })

  it('available 을 재지 못하면 보이는 탭 폭 합(숨긴 탭 뺌)', () => {
    expect(dragAreaRect(frame({ available: null }).area)).toEqual({ x0: 8, y0: 2, x1: 228, y1: 30 })
  })
})

describe('frameChromeOf', () => {
  it('캔버스 왼쪽 위 = 원점 · tab_bar = 끌기 영역 · 그 둘만 싣는다', () => {
    expect(frameChromeOf(frame({ canvasTop: 32.5 }))).toEqual({
      canvas_origin: { x: 0, y: 32.5 },
      tab_bar: { x0: 8, y0: 2, x1: 608, y1: 30 },
    })
  })
})

describe('보내기', () => {
  it('submitFrame = report_dock_chrome{chrome} — 셸이 받은 값과 같으면 다시 보내지 않는다', async () => {
    submitFrame(frame())
    await ticks()
    expect(sent()).toEqual([{ chrome: frameChromeOf(frame()) }])

    submitFrame(frame())
    await ticks()
    expect(sent()).toHaveLength(1)

    submitFrame(frame({ available: 700 }))
    await ticks()
    expect(sent()).toHaveLength(2)
    expect(sent()[1]).toEqual({ chrome: frameChromeOf(frame({ available: 700 })) })
  })

  it('scheduleFrameReport — 디바운스 끝에 마지막 measure 로 재어 보내고, null 이면 보내지 않는다', async () => {
    vi.useFakeTimers()
    scheduleFrameReport(() => frame({ canvasTop: 10 }))
    scheduleFrameReport(() => frame({ canvasTop: 20 }))
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS - 1)
    expect(sent()).toEqual([])
    await vi.advanceTimersByTimeAsync(1)
    expect(sent()).toEqual([{ chrome: frameChromeOf(frame({ canvasTop: 20 })) }])

    scheduleFrameReport(() => null)
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS)
    expect(sent()).toHaveLength(1)
  })

  it('cancelFrameReport — 디바운스 중이면 보내지 않는다', async () => {
    vi.useFakeTimers()
    scheduleFrameReport(() => frame())
    cancelFrameReport()
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS * 3)
    expect(sent()).toEqual([])
  })
})
