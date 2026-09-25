// uiMetricsReport 단위테스트 — 칸 틀 기본 지표 보고(ADR-0227, TRD §2d). 웹뷰당 한 번 · 진행 중 공유 ·
// 성공 뒤에만 「보냈음」 · 유계 재시도 · 최종 실패면 다음 호출이 다시 시도.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))

import { MIN_PANE_PX } from './splitPreview'
import { __resetUiMetricsReportForTest, measureUiMetrics, reportUiMetrics } from './uiMetricsReport'

const reportCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'report_ui_metrics')

function borderEl(t = '1px', r = '1px', b = '1px', l = '1px'): HTMLElement {
  const el = document.createElement('div')
  el.style.borderStyle = 'solid'
  el.style.borderTopWidth = t
  el.style.borderRightWidth = r
  el.style.borderBottomWidth = b
  el.style.borderLeftWidth = l
  document.body.appendChild(el)
  return el
}

/** 재시도 backoff(150·300·600ms)를 전부 넘길 만큼. */
const RETRY_SPAN_MS = 5000

let errorSpy: ReturnType<typeof vi.spyOn>
let warnSpy: ReturnType<typeof vi.spyOn>

beforeEach(() => {
  __resetUiMetricsReportForTest()
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
  errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {})
  warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
})

afterEach(() => {
  vi.useRealTimers()
  errorSpy.mockRestore()
  warnSpy.mockRestore()
  document.body.innerHTML = ''
})

describe('measureUiMetrics', () => {
  it('테두리 요소의 계산된 테두리 폭 넷을 소수 그대로 읽고, 최소 칸은 정책 상수를 싣는다', () => {
    expect(measureUiMetrics(borderEl('1.25px', '2px', '3px', '4px'))).toEqual({
      frame_insets: { t: 1.25, r: 2, b: 3, l: 4 },
      min_pane_px: MIN_PANE_PX,
    })
  })

  it('폭이 수로 읽히지 않으면 null', () => {
    const el = document.createElement('div')
    // 단축 속성에 var() 가 섞이면 jsdom 은 longhand 를 계산하지 못한다(`medium`).
    el.style.border = '1px solid var(--border)'
    document.body.appendChild(el)
    expect(measureUiMetrics(el)).toBeNull()
  })
})

describe('reportUiMetrics', () => {
  it('재서 report_ui_metrics 로 보낸다 — 인자 모양은 { metrics: UiMetrics }', async () => {
    await reportUiMetrics(borderEl('1px', '1px', '1px', '1px'))
    expect(reportCalls()).toEqual([
      ['report_ui_metrics', { metrics: { frame_insets: { t: 1, r: 1, b: 1, l: 1 }, min_pane_px: MIN_PANE_PX } }],
    ])
  })

  it('성공한 뒤로는 웹뷰당 다시 보내지 않는다', async () => {
    await reportUiMetrics(borderEl())
    await reportUiMetrics(borderEl('2px', '2px', '2px', '2px'))
    expect(reportCalls()).toHaveLength(1)
  })

  it('보내는 중에 온 호출은 같은 작업을 기다린다(보고는 하나만 나간다)', async () => {
    let resolve!: () => void
    invokeMock.mockImplementationOnce(() => new Promise<undefined>(res => (resolve = () => res(undefined))))
    const first = reportUiMetrics(borderEl())
    const second = reportUiMetrics(borderEl())
    const third = reportUiMetrics(borderEl())
    expect(second).toBe(first)
    expect(third).toBe(first)
    await Promise.resolve()
    expect(reportCalls()).toHaveLength(1)
    resolve()
    await Promise.all([first, second, third])
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(1)
  })

  it('실패는 유계 재시도하고, 재시도 안에서 성공하면 그 뒤로 보내지 않는다', async () => {
    vi.useFakeTimers()
    invokeMock.mockImplementationOnce(async () => {
      throw new Error('not ready')
    })
    const job = reportUiMetrics(borderEl())
    await vi.advanceTimersByTimeAsync(RETRY_SPAN_MS)
    await job
    expect(reportCalls()).toHaveLength(2)
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(2)
  })

  it('재시도를 다 써도 실패하면 「보냈음」을 세우지 않고 걸쇠를 풀어 다음 호출이 다시 보낸다', async () => {
    vi.useFakeTimers()
    invokeMock.mockImplementation(async () => {
      throw new Error('down')
    })
    const job = reportUiMetrics(borderEl())
    await vi.advanceTimersByTimeAsync(RETRY_SPAN_MS)
    await expect(job).resolves.toBeUndefined()
    const exhausted = reportCalls().length
    expect(exhausted).toBeGreaterThan(1)
    expect(errorSpy).toHaveBeenCalled()

    invokeMock.mockImplementation(async () => undefined)
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(exhausted + 1)
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(exhausted + 1)
  })

  it('폭을 수로 읽지 못하면 보내지 않고, 다음 호출이 다시 잰다', async () => {
    const bad = document.createElement('div')
    bad.style.border = '1px solid var(--border)'
    document.body.appendChild(bad)
    await reportUiMetrics(bad)
    expect(reportCalls()).toHaveLength(0)
    expect(errorSpy).toHaveBeenCalled()
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(1)
  })

  it('초기화 전에 떠난 작업의 응답은 초기화 뒤 상태를 건드리지 않는다', async () => {
    let resolve!: () => void
    invokeMock.mockImplementationOnce(() => new Promise<undefined>(res => (resolve = () => res(undefined))))
    const stale = reportUiMetrics(borderEl())
    await Promise.resolve()
    __resetUiMetricsReportForTest()
    resolve()
    await stale
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(2)
  })
})
