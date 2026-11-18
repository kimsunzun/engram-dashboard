// TRD S26-window-chrome §8-4 — the web's share of the snap layout overlay: physical px rectangle computation · report only when on · the same value is not sent again ·
//   hover events · withdrawing and releasing listening on detach. Whether the window fact field opens and closes this path is covered by `TitleRow.test.tsx`.

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

// Waits for the registration of hover event listening (the promise of `listen`) to resolve.
const settle = () => act(async () => {})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

describe('physicalRect — CSS px × scale → physical px (fitted outward)', () => {
  it('100 % = as is · 150 % rounds the fractional end outward · null if empty (withdraw)', () => {
    expect(physicalRect({ left: 900, top: 0, right: 946, bottom: 34 }, 1)).toEqual({ x: 900, y: 0, width: 46, height: 34 })
    // 900.5 × 1.5 = 1350.75 → 1350 · 946.2 × 1.5 = 1419.3 → 1420 · 34 × 1.5 = 51
    expect(physicalRect({ left: 900.5, top: 0, right: 946.2, bottom: 34 }, 1.5)).toEqual({ x: 1350, y: 0, width: 70, height: 51 })
    expect(physicalRect({ left: 10, top: 0, right: 10, bottom: 34 }, 1)).toBeNull()
    expect(physicalRect({ left: 0, top: 0, right: 0, bottom: 0 }, 2)).toBeNull()
  })
})

describe('useSnapOverlay', () => {
  it('if off it neither reports nor listens and is always false', async () => {
    const f = fakeIpc()
    const b = button({ x: 900, y: 0, width: 46, height: 34 })
    const { result, unmount } = renderHook(() => useSnapOverlay(false, b.ref, f.ipc))
    await settle()
    unmount()
    expect([f.invoke.mock.calls.length, f.listen.mock.calls.length, result.current]).toEqual([0, 0, false])
  })

  it('if on it reports the rectangle once · when the window size changes it re-measures but does not send the same value · multiplies by the scale', async () => {
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

  it('hover event (this window\'s share) = returns true / false as is · false if not a bool', async () => {
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

  it('if turned off or detached it withdraws the overlay (rect null) and releases listening · hover goes false', async () => {
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

  it('if listening registration ends after detach it releases on the spot', async () => {
    const f = fakeIpc()
    const b = button({ x: 900, y: 0, width: 46, height: 34 })
    const { unmount } = renderHook(() => useSnapOverlay(true, b.ref, f.ipc))
    unmount()
    await settle()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
  })

  it('a button that is not visible (size 0) sends only a withdraw from the start, and does not send again even when detached', async () => {
    const f = fakeIpc()
    const b = button({ x: 0, y: 0, width: 0, height: 0 })
    const { unmount } = renderHook(() => useSnapOverlay(true, b.ref, f.ipc))
    await settle()
    unmount()
    expect(f.rects()).toEqual([null])
  })
})
