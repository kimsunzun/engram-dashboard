// DockOverlayPage 단위테스트 — 안내 창이 셸의 `dock:overlay` 를 자기 창 대상으로 듣고 그 몸(`OverlayView | null`)을 그린다.

import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const handlers = new Map<string, (e: { payload: unknown }) => void>()
const listenOptions = new Map<string, unknown>()
const unlisten = vi.fn()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void, options?: unknown) => {
    handlers.set(event, handler)
    listenOptions.set(event, options)
    return unlisten
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'dock-overlay' }) }))

import DockOverlayPage from './DockOverlayPage'

beforeEach(() => {
  handlers.clear()
  listenOptions.clear()
  unlisten.mockClear()
})
afterEach(cleanup)

const push = (payload: unknown) => act(() => handlers.get('dock:overlay')!({ payload }))

describe('DockOverlayPage', () => {
  it('dock:overlay 를 자기 창 대상으로 듣고 민 그림을 그린다 · null 이면 흐려진 뒤 치운다', async () => {
    render(<DockOverlayPage />)
    await waitFor(() => expect(handlers.has('dock:overlay')).toBe(true))
    expect(listenOptions.get('dock:overlay')).toEqual({ target: 'dock-overlay' })
    expect(screen.queryByTestId('dock-overlay')).toBeNull()

    push({ outer: [], bands: [{ x0: 10, y0: 20, x1: 30, y1: 40 }], preview: { x0: 10, y0: 20, x1: 50, y1: 40 } })
    expect(screen.getByTestId('dock-preview').style.left).toBe('10px')
    expect(screen.getByTestId('dock-preview').style.width).toBe('40px')
    expect(screen.getByTestId('dock-overlay-page').style.position).toBe('fixed')

    push(null)
    await waitFor(() => expect(screen.queryByTestId('dock-overlay')).toBeNull())
  })

  it('내리면 듣기를 뗀다', async () => {
    const view = render(<DockOverlayPage />)
    await waitFor(() => expect(handlers.has('dock:overlay')).toBe(true))
    await act(async () => {})
    view.unmount()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
