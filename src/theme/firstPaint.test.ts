import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.hoisted(() => vi.fn(async (_cmd: string) => {}))
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
    reportFirstPaint()
    reportFirstPaint()
    expect(invoke).not.toHaveBeenCalled()
    flushFrame()
    expect(invoke).not.toHaveBeenCalled()
    flushFrame()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith('webview_painted')
    flushFrame()
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('보이는 페이지인데 프레임이 오지 않으면 상한 뒤에 한 번 보낸다 · 늦게 온 프레임은 더 보내지 않는다', () => {
    useFakeTimeouts()
    try {
      setVisibility('visible')
      reportFirstPaint()
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
      reportFirstPaint()
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
    reportFirstPaint()
    reportFirstPaint()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(frames).toHaveLength(0)
  })

  it('실패는 던지지 않고 경고로만 남긴다', async () => {
    setVisibility('hidden')
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invoke.mockRejectedValueOnce(new Error('셸 밖'))
    expect(() => reportFirstPaint()).not.toThrow()
    await vi.waitFor(() => expect(warn).toHaveBeenCalled())
    warn.mockRestore()
  })
})
