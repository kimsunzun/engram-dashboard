// App 경로 가름 — 안내 창(`#/dock-overlay` · TRD S21-slot-docking §2-5)은 탭 창의 전역 배선을 하나도 돌리지 않는다.

import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const wiring = vi.hoisted(() => ({
  installUiSettings: vi.fn(() => () => {}),
  installKeybindings: vi.fn(() => () => {}),
  installViewCommandBridge: vi.fn(() => () => {}),
  installNativeScrollActivity: vi.fn(() => () => {}),
  bootstrapDaemonIfNeeded: vi.fn(async () => {}),
  initEventBus: vi.fn(async () => {}),
  getAgents: vi.fn(async () => []),
  listProfiles: vi.fn(async () => []),
  listPresets: vi.fn(async () => []),
}))

vi.mock('./theme/uiSettings', () => ({ installUiSettings: wiring.installUiSettings }))
vi.mock('./commands/keybindings', () => ({ installKeybindings: wiring.installKeybindings }))
vi.mock('./commands/viewCommandBridge', () => ({ installViewCommandBridge: wiring.installViewCommandBridge }))
vi.mock('./components/ui/nativeScrollActivity', () => ({
  installNativeScrollActivity: wiring.installNativeScrollActivity,
}))
vi.mock('./api/clientFactory', () => ({
  bootstrapDaemonIfNeeded: wiring.bootstrapDaemonIfNeeded,
  agentClient: { getAgents: wiring.getAgents, listProfiles: wiring.listProfiles, listPresets: wiring.listPresets },
}))
vi.mock('./store/eventBus', () => ({ initEventBus: wiring.initEventBus }))
vi.mock('./commands/contributions', () => ({}))
vi.mock('./components/layout/AppLayout', () => ({ default: () => <div data-testid="app-layout" /> }))
vi.mock('./pages/PopoutPage', () => ({ default: () => <div data-testid="popout-page" /> }))
// 안내 창 페이지가 거는 `dock:overlay` 듣기 — jsdom 에는 Tauri 내부가 없다.
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'dock-overlay' }) }))

import App from './App'

const origHash = window.location.hash

beforeEach(() => {
  for (const fn of Object.values(wiring)) fn.mockClear()
})

afterEach(() => {
  cleanup()
  window.location.hash = origHash
})

describe('App — 안내 창 경로', () => {
  it('#/dock-overlay → 안내 창 페이지만 · 전역 배선(설정 · 키 · 버스 명령 보고 · 데몬 부팅 · 목록 조회)을 돌리지 않는다', async () => {
    window.location.hash = '#/dock-overlay'
    render(<App />)
    expect(screen.getByTestId('dock-overlay-page')).toBeTruthy()
    await new Promise(res => setTimeout(res, 20))
    for (const [name, fn] of Object.entries(wiring)) expect(fn, name).not.toHaveBeenCalled()
  })

  it('안내 창은 html · body · #root 바닥을 투명으로 걷고, 내리면 되돌린다', () => {
    window.location.hash = '#/dock-overlay'
    const root = document.createElement('div')
    root.id = 'root'
    document.body.appendChild(root)
    try {
      const view = render(<App />, { container: root })
      for (const el of [document.documentElement, document.body, root]) expect(el.style.background).toBe('transparent')
      view.unmount()
      for (const el of [document.documentElement, document.body, root]) expect(el.style.background).toBe('')
    } finally {
      root.remove()
    }
  })

  it('탭 창 경로(#/)는 전역 배선을 돌린다 — 위 시험의 관측이 살아 있다', async () => {
    window.location.hash = '#/'
    render(<App />)
    expect(screen.getByTestId('app-layout')).toBeTruthy()
    await waitFor(() => expect(wiring.getAgents).toHaveBeenCalled())
    for (const [name, fn] of Object.entries(wiring)) expect(fn, name).toHaveBeenCalled()
  })
})
