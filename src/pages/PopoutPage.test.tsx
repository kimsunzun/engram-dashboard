import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler)
    return unlistenMock
  }),
}))

// ── invoke mock ──
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
  Channel: class {
    onmessage: unknown = null
  },
}))

// ── getCurrentWindow mock ──
const closeMock = vi.fn(async () => undefined)
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: closeMock, label: () => 'slot-popup-1' }),
}))

// ── WindowLayout stub — 내부 로직은 WindowLayout.test 가 커버 ──
vi.mock('../components/layout/WindowLayout', () => ({
  default: ({ label }: { label: string }) => <div data-testid="window-layout" data-label={label} />,
}))

import PopoutPage from './PopoutPage'

const POPUP_LABEL = 'slot-popup-1'

const origHash = window.location.hash

beforeEach(() => {
  listeners.clear()
  unlistenMock.mockClear()
  closeMock.mockClear()
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
  window.location.hash = `#/popup?window=${POPUP_LABEL}`
})

afterEach(() => {
  cleanup()
  window.location.hash = origHash
})

describe('PopoutPage (탭 소유 모델, ADR-0057)', () => {
  it('?window=<label> → WindowLayout 이 그 label 로 마운트된다', () => {
    render(<PopoutPage />)
    const wl = screen.getByTestId('window-layout')
    expect(wl.getAttribute('data-label')).toBe(POPUP_LABEL)
  })

  it('★view:closed 은퇴(G2)★: PopoutPage 는 view:closed 를 구독하지 않는다(자가종료 리스너 제거)', () => {
    render(<PopoutPage />)
    // 옛 버그: view:closed 리스너로 창 자가종료 → 이중 발화/재진입.
    expect(listeners.has('view:closed')).toBe(false)
    expect(closeMock).not.toHaveBeenCalled()
  })
})
