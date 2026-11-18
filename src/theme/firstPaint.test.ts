import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown) => {}))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import { __resetFirstPaintForTest, reportFirstPaint } from './firstPaint'

let frames: FrameRequestCallback[] = []

function flushFrame() {
  const cbs = frames
  frames = []
  cbs.forEach(cb => cb(0))
}

// Fake only the timers — the default fake also swaps `requestAnimationFrame`, bypassing the manual frame (`flushFrame`) above.
function useFakeTimeouts() {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state })
}

beforeEach(() => {
  __resetFirstPaintForTest()
  invoke.mockClear()
  frames = []
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    frames.push(cb)
    return frames.length
  })
})

afterEach(() => {
  vi.unstubAllGlobals()
  delete (document as { visibilityState?: unknown }).visibilityState
})

describe('reportFirstPaint', () => {
  it('visible page — sends once after two frames · calling again (StrictMode effect twice) sends no more', () => {
    setVisibility('visible')
    reportFirstPaint(null)
    reportFirstPaint(null)
    expect(invoke).not.toHaveBeenCalled()
    flushFrame()
    expect(invoke).not.toHaveBeenCalled()
    flushFrame()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith('webview_painted', { generation: null })
    flushFrame()
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('visible page but no frame comes — sends once after the cap · a late frame sends no more', () => {
    useFakeTimeouts()
    try {
      setVisibility('visible')
      reportFirstPaint(null)
      vi.advanceTimersByTime(99)
      expect(invoke).not.toHaveBeenCalled()
      vi.advanceTimersByTime(1)
      expect(invoke).toHaveBeenCalledTimes(1)
      flushFrame()
      flushFrame()
      expect(invoke).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('visible page — if already sent by two frames, the cap coming sends no more', () => {
    useFakeTimeouts()
    try {
      setVisibility('visible')
      reportFirstPaint(null)
      flushFrame()
      flushFrame()
      expect(invoke).toHaveBeenCalledTimes(1)
      vi.advanceTimersByTime(1000)
      expect(invoke).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('hidden page (a webview whose content the shell hid) — sends once right away without waiting for frames', () => {
    setVisibility('hidden')
    reportFirstPaint(null)
    reportFirstPaint(null)
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(frames).toHaveLength(0)
  })

  it('a failure does not throw and is left only as a warning', async () => {
    setVisibility('hidden')
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invoke.mockRejectedValueOnce(new Error('outside the shell'))
    expect(() => reportFirstPaint(null)).not.toThrow()
    await vi.waitFor(() => expect(warn).toHaveBeenCalled())
    warn.mockRestore()
  })

  // ── Adoption number (TRD S21-slot-docking §3-9 I26) — the shell door of the adopting window accepts only notices carrying that number ──

  it('a page that already sent sends once more when called with a new adoption number and that notice carries the number · not again with the same number or null', () => {
    setVisibility('hidden')
    reportFirstPaint(null)
    reportFirstPaint(4)
    reportFirstPaint(4)
    reportFirstPaint(null)
    expect(invoke.mock.calls).toEqual([
      ['webview_painted', { generation: null }],
      ['webview_painted', { generation: 4 }],
    ])
  })

  it('number 0 is a number too', () => {
    setVisibility('hidden')
    reportFirstPaint(null)
    reportFirstPaint(0)
    reportFirstPaint(0)
    expect(invoke.mock.calls).toEqual([
      ['webview_painted', { generation: null }],
      ['webview_painted', { generation: 0 }],
    ])
  })

  it('called with a number from the start (mount after adoption) → once with that number · not with null after that', () => {
    setVisibility('hidden')
    reportFirstPaint(7)
    reportFirstPaint(null)
    expect(invoke.mock.calls).toEqual([['webview_painted', { generation: 7 }]])
  })

  it('visible page — if a number comes while the earlier notice is waiting for frames, both are sent and the later notice carries the number', () => {
    setVisibility('visible')
    reportFirstPaint(null)
    reportFirstPaint(2)
    flushFrame()
    flushFrame()
    expect(invoke.mock.calls).toEqual([
      ['webview_painted', { generation: null }],
      ['webview_painted', { generation: 2 }],
    ])
  })
})
