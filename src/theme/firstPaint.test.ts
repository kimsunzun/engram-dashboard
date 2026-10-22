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

// 타이머만 가짜로 — 기본 가짜는 `requestAnimationFrame` 도 갈아 위의 손 프레임(`flushFrame`)을 우회한다.
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
  it('보이는 페이지 — 프레임 둘 뒤에 한 번 보낸다 · 다시 불러도(StrictMode effect 두 번) 더 보내지 않는다', () => {
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

  it('보이는 페이지인데 프레임이 오지 않으면 상한 뒤에 한 번 보낸다 · 늦게 온 프레임은 더 보내지 않는다', () => {
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

  it('보이는 페이지 — 프레임 둘로 이미 보냈으면 상한이 와도 더 보내지 않는다', () => {
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

  it('숨은 페이지(셸이 내용을 숨긴 웹뷰) — 프레임을 기다리지 않고 곧바로 한 번 보낸다', () => {
    setVisibility('hidden')
    reportFirstPaint(null)
    reportFirstPaint(null)
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(frames).toHaveLength(0)
  })

  it('실패는 던지지 않고 경고로만 남긴다', async () => {
    setVisibility('hidden')
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invoke.mockRejectedValueOnce(new Error('셸 밖'))
    expect(() => reportFirstPaint(null)).not.toThrow()
    await vi.waitFor(() => expect(warn).toHaveBeenCalled())
    warn.mockRestore()
  })

  // ── 채택 번호(TRD S21-slot-docking §3-9 I26) — 채택한 창의 셸 문은 그 번호를 실은 알림만 받는다 ──

  it('이미 보낸 페이지도 새 채택 번호로 부르면 한 번 더 보내고 그 알림이 번호를 싣는다 · 같은 번호 · null 로는 더 보내지 않는다', () => {
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

  it('번호 0 도 번호다', () => {
    setVisibility('hidden')
    reportFirstPaint(null)
    reportFirstPaint(0)
    reportFirstPaint(0)
    expect(invoke.mock.calls).toEqual([
      ['webview_painted', { generation: null }],
      ['webview_painted', { generation: 0 }],
    ])
  })

  it('처음부터 번호로 부르면(채택 뒤 마운트) 그 번호로 한 번 · 그 뒤 null 로는 보내지 않는다', () => {
    setVisibility('hidden')
    reportFirstPaint(7)
    reportFirstPaint(null)
    expect(invoke.mock.calls).toEqual([['webview_painted', { generation: 7 }]])
  })

  it('보이는 페이지 — 앞 알림이 프레임을 기다리는 중 번호가 오면 둘 다 보내고 뒤 알림이 번호를 싣는다', () => {
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
