// App 부팅 배선 — 탭 창은 전역 배선(설정 · 키 · 버스 명령 보고 · 스크롤 · 데몬 부팅 · 목록 조회)을 돌린다.

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

import App from './App'

const origHash = window.location.hash

beforeEach(() => {
  for (const fn of Object.values(wiring)) fn.mockClear()
})

afterEach(() => {
  cleanup()
  window.location.hash = origHash
})

describe('App — 부팅 배선', () => {
  it('탭 창 경로(#/)는 전역 배선을 돌린다', async () => {
    window.location.hash = '#/'
    render(<App />)
    expect(screen.getByTestId('app-layout')).toBeTruthy()
    await waitFor(() => expect(wiring.getAgents).toHaveBeenCalled())
    for (const [name, fn] of Object.entries(wiring)) expect(fn, name).toHaveBeenCalled()
  })
})
