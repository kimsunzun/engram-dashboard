// TRD S26-window-chrome §8-4 — 스냅 레이아웃 오버레이의 웹 몫: 물리 px 사각형 셈 · 켰을 때만 보고 · 같은 값 다시 안 보냄 ·
//   맴돎 사건 · 떼면 걷기와 듣기 풀기. 창 사실 칸이 이 길을 여닫는지는 `TitleRow.test.tsx` 가 본다.

import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { CMD_REPORT_MAX_BUTTON_RECT, EVT_SNAP_HOVER, physicalRect, useSnapOverlay, type SnapOverlayIpc } from './snapOverlay'

function fakeIpc() {
  const invoke = vi.fn(async (_cmd: string, _args: Record<string, unknown>): Promise<unknown> => undefined)
  const unlisten = vi.fn()
  let handler: ((payload: unknown) => void) | null = null
  const listen = vi.fn(async (event: string, h: (payload: unknown) => void) => {
    expect(event).toBe(EVT_SNAP_HOVER)
    handler = h
    return unlisten
  })
  const ipc: SnapOverlayIpc = { invoke, listen }
  return {
    ipc,
    invoke,
    listen,
    unlisten,
    rects: () => invoke.mock.calls.filter(c => c[0] === CMD_REPORT_MAX_BUTTON_RECT).map(c => c[1].rect),
    hover: (payload: unknown) => act(() => handler?.(payload)),
  }
}

function button(rect: { x: number; y: number; width: number; height: number }) {
  const el = document.createElement('button')
  let current = rect
  el.getBoundingClientRect = () => DOMRect.fromRect(current)
  return { el, ref: { current: el }, move: (next: typeof rect) => (current = next) }
}

// 맴돎 사건 등록(`listen` 의 약속)이 풀리기를 기다린다.
const settle = () => act(async () => {})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe('physicalRect — CSS px × 배율 → 물리 px(바깥으로 맞춤)', () => {
  it('100 % = 그대로 · 150 % 는 소수 끝을 바깥으로 · 비면 null(걷기)', () => {
    expect(physicalRect({ left: 900, top: 0, right: 946, bottom: 34 }, 1)).toEqual({ x: 900, y: 0, width: 46, height: 34 })
    // 900.5 × 1.5 = 1350.75 → 1350 · 946.2 × 1.5 = 1419.3 → 1420 · 34 × 1.5 = 51
    expect(physicalRect({ left: 900.5, top: 0, right: 946.2, bottom: 34 }, 1.5)).toEqual({ x: 1350, y: 0, width: 70, height: 51 })
    expect(physicalRect({ left: 10, top: 0, right: 10, bottom: 34 }, 1)).toBeNull()
    expect(physicalRect({ left: 0, top: 0, right: 0, bottom: 0 }, 2)).toBeNull()
  })
})

describe('useSnapOverlay', () => {
  it('꺼져 있으면 보고도 듣기도 하지 않고 늘 거짓', async () => {
    const f = fakeIpc()
    const b = button({ x: 900, y: 0, width: 46, height: 34 })
    const { result, unmount } = renderHook(() => useSnapOverlay(false, b.ref, f.ipc))
    await settle()
    unmount()
    expect([f.invoke.mock.calls.length, f.listen.mock.calls.length, result.current]).toEqual([0, 0, false])
  })

  it('켜면 사각형을 한 번 보고 · 창 크기가 바뀌면 다시 재되 같은 값은 보내지 않는다 · 배율을 곱한다', async () => {
    vi.stubGlobal('devicePixelRatio', 1.25)
    const f = fakeIpc()
    const b = button({ x: 800, y: 0, width: 46, height: 34 })
    renderHook(() => useSnapOverlay(true, b.ref, f.ipc))
    await settle()
    expect(f.rects()).toEqual([{ x: 1000, y: 0, width: 58, height: 43 }])
    act(() => void window.dispatchEvent(new Event('resize')))
    expect(f.rects()).toHaveLength(1)
    b.move({ x: 960, y: 0, width: 46, height: 34 })
    act(() => void window.dispatchEvent(new Event('resize')))
    expect(f.rects().slice(-1)[0]).toEqual({ x: 1200, y: 0, width: 58, height: 43 })
  })

  it('맴돎 사건(이 창 몫) = 참 / 거짓을 그대로 돌려준다 · bool 이 아니면 거짓', async () => {
    const f = fakeIpc()
    const b = button({ x: 900, y: 0, width: 46, height: 34 })
    const { result } = renderHook(() => useSnapOverlay(true, b.ref, f.ipc))
    await settle()
    expect(result.current).toBe(false)
    f.hover(true)
    expect(result.current).toBe(true)
    f.hover(false)
    expect(result.current).toBe(false)
    f.hover('yes')
    expect(result.current).toBe(false)
  })

  it('꺼지거나 떼면 오버레이를 걷고(rect null) 듣기를 푼다 · 맴돎은 거짓으로', async () => {
    const f = fakeIpc()
    const b = button({ x: 900, y: 0, width: 46, height: 34 })
    const { result, rerender, unmount } = renderHook(({ on }) => useSnapOverlay(on, b.ref, f.ipc), { initialProps: { on: true } })
    await settle()
    f.hover(true)
    rerender({ on: false })
    expect(f.rects().slice(-1)[0]).toBeNull()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
    expect(result.current).toBe(false)
    const before = f.invoke.mock.calls.length
    unmount()
    expect(f.invoke.mock.calls.length).toBe(before)
  })

  it('듣기 등록이 떼기 뒤에 끝나면 그 자리에서 푼다', async () => {
    const f = fakeIpc()
    const b = button({ x: 900, y: 0, width: 46, height: 34 })
    const { unmount } = renderHook(() => useSnapOverlay(true, b.ref, f.ipc))
    unmount()
    await settle()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
  })

  it('보이지 않는 단추(크기 0)는 처음부터 걷기만 보내고, 떼어도 다시 보내지 않는다', async () => {
    const f = fakeIpc()
    const b = button({ x: 0, y: 0, width: 0, height: 0 })
    const { unmount } = renderHook(() => useSnapOverlay(true, b.ref, f.ipc))
    await settle()
    unmount()
    expect(f.rects()).toEqual([null])
  })
})
