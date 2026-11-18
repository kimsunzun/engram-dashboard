// uiMetricsReport unit test — area frame default metrics report (ADR-0227, TRD §2d). Once per webview · shared while in progress ·
//   「sent」 only after success · bounded retry · if the final failure, the next call tries again.

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

/** Enough to get past all the retry backoffs (150·300·600ms). */
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
  it('reads the four computed border widths of the border element as decimals as is, and the minimum area carries the policy constant', () => {
    expect(measureUiMetrics(borderEl('1.25px', '2px', '3px', '4px'))).toEqual({
      frame_insets: { t: 1.25, r: 2, b: 3, l: 4 },
      min_pane_px: MIN_PANE_PX,
    })
  })

  it('null if the width is not read as a number', () => {
    const el = document.createElement('div')
    // If var() is mixed into a shorthand property, jsdom cannot compute the longhand (`medium`).
    el.style.border = '1px solid var(--border)'
    document.body.appendChild(el)
    expect(measureUiMetrics(el)).toBeNull()
  })
})

describe('reportUiMetrics', () => {
  it('measures and sends as report_ui_metrics — the argument shape is { metrics: UiMetrics }', async () => {
    await reportUiMetrics(borderEl('1px', '1px', '1px', '1px'))
    expect(reportCalls()).toEqual([
      ['report_ui_metrics', { metrics: { frame_insets: { t: 1, r: 1, b: 1, l: 1 }, min_pane_px: MIN_PANE_PX } }],
    ])
  })

  it('after success it does not send again per webview', async () => {
    await reportUiMetrics(borderEl())
    await reportUiMetrics(borderEl('2px', '2px', '2px', '2px'))
    expect(reportCalls()).toHaveLength(1)
  })

  it('a call arriving while sending waits for the same work (only one report goes out)', async () => {
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

  it('failure retries boundedly, and if it succeeds within the retries it does not send afterwards', async () => {
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

  it('if it fails even after using all retries it does not raise 「sent」 and releases the latch so the next call sends again', async () => {
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

  it('if the width is not read as a number it does not send, and the next call measures again', async () => {
    const bad = document.createElement('div')
    bad.style.border = '1px solid var(--border)'
    document.body.appendChild(bad)
    await reportUiMetrics(bad)
    expect(reportCalls()).toHaveLength(0)
    expect(errorSpy).toHaveBeenCalled()
    await reportUiMetrics(borderEl())
    expect(reportCalls()).toHaveLength(1)
  })

  it('the response of work that left before initialization does not touch the post-initialization state', async () => {
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
