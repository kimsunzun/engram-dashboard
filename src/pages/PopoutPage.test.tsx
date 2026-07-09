// PopoutPage 단위테스트 — 탭 소유 모델(ADR-0057)로 재작성.
//
// ★이 스위트가 검증하는 것★: PopoutPage 는 이제 "고정 단일 View" 팝업이 아니라 **탭 가진 창**의 얇은
// 껍데기다. URL `?window=<label>` 에서 자기 창 label 을 뽑아 WindowLayout(label) 을 마운트한다(§7-1).
//   1. ?window=<label> → WindowLayout 이 그 label 로 마운트된다.
//   2. ★view:closed 은퇴(G2)★: PopoutPage 는 view:closed 를 구독하지 않는다(자가종료는 WindowLayout 의
//      0탭 신호로만 — 옛 view:closed→close() 리스너 제거를 이 스위트가 회귀 안전망으로 못박는다).

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── listen mock: 이벤트명별 등록을 기록(view:closed 구독이 없음을 단언) ──
const listeners = new Map<string, (e: { payload: unknown }) => void>()
const unlistenMock = vi.fn()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler)
    return unlistenMock
  }),
}))

// ── invoke mock: list_tabs/get_view 응답(WindowLayout mount 시 pull) ──
const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
  Channel: class {
    onmessage: unknown = null
  },
}))

// ── getCurrentWindow mock — WindowLayout 0탭 자가닫힘 경로가 참조(이 테스트에선 안 불림) ──
const closeMock = vi.fn(async () => undefined)
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: closeMock, label: () => 'slot-popup-1' }),
}))

// ── WindowLayout stub — PopoutPage 가 넘기는 label prop 만 관측(내부 로직은 WindowLayout.test 가 커버) ──
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
  // 팝업 컨텍스트 hash — readWindowLabelFromHash 가 POPUP_LABEL 을 파싱하게.
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
    // 옛 버그: view:closed 리스너로 창 자가종료 → 이중 발화/재진입. 이제 구독 자체가 없어야 한다.
    expect(listeners.has('view:closed')).toBe(false)
    expect(closeMock).not.toHaveBeenCalled()
  })
})
