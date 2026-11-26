// App boot wiring — a tab window runs the global wiring (settings · keys · bus command report · scroll · daemon boot · list query).

import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
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
}))

vi.mock('./theme/uiSettings', () => ({ installUiSettings: wiring.installUiSettings }))
vi.mock('./commands/keybindings', () => ({ installKeybindings: wiring.installKeybindings }))
vi.mock('./commands/viewCommandBridge', () => ({ installViewCommandBridge: wiring.installViewCommandBridge }))
vi.mock('./components/ui/nativeScrollActivity', () => ({
  installNativeScrollActivity: wiring.installNativeScrollActivity,
}))
vi.mock('./api/clientFactory', () => ({
  bootstrapDaemonIfNeeded: wiring.bootstrapDaemonIfNeeded,
  agentClient: { getAgents: wiring.getAgents, listProfiles: wiring.listProfiles },
}))
vi.mock('./store/eventBus', () => ({ initEventBus: wiring.initEventBus }))
vi.mock('./commands/contributions', () => ({}))
// The layout stand-in renders a table string and has no language subscription of its own — only `App` re-renders it.
vi.mock('./components/layout/AppLayout', async () => {
  const { t } = await import('./i18n')
  return { default: () => <div data-testid="app-layout">{t('tab.create')}</div> }
})
vi.mock('./pages/PopoutPage', () => ({ default: () => <div data-testid="popout-page" /> }))

import App from './App'
import { setLanguage } from './i18n'

const origHash = window.location.hash

beforeEach(() => {
  for (const fn of Object.values(wiring)) fn.mockClear()
})

afterEach(() => {
  cleanup()
  window.location.hash = origHash
})

describe('App — boot wiring', () => {
  it('a tab window path (#/) runs the global wiring', async () => {
    window.location.hash = '#/'
    render(<App />)
    expect(screen.getByTestId('app-layout')).toBeTruthy()
    await waitFor(() => expect(wiring.getAgents).toHaveBeenCalled())
    for (const [name, fn] of Object.entries(wiring)) expect(fn, name).toHaveBeenCalled()
  })
})

// ADR-0373: a language switch re-renders the tree from `App`.
describe('App — language switch', () => {
  afterEach(() => {
    act(() => {
      setLanguage('ko')
    })
  })

  it('text below App follows setLanguage without a remount', async () => {
    window.location.hash = '#/'
    render(<App />)
    const layout = screen.getByTestId('app-layout')
    expect(layout.textContent).toBe('새 탭')

    act(() => {
      setLanguage('en')
    })

    expect(screen.getByTestId('app-layout')).toBe(layout)
    expect(layout.textContent).toBe('New tab')
    await waitFor(() => expect(wiring.getAgents).toHaveBeenCalled())
  })
})
