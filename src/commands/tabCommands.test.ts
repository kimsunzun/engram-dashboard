// tabCommands 단위테스트 — 탭/창 command 어댑터가 store 액션으로 올바로 라우팅하는지(ADR-0055/0057).
//
// ★검증 불변식★:
//   1. tab.create/switch/close·window.create/close 가 대응 viewStore 액션을 (해소된 window, ...)로 부른다.
//   2. window 생략 → 이 웹뷰 창(readWindowLabelFromHash)으로 해소.
//   3. tab.next(Ctrl+Tab) → 이 창 탭을 오른쪽 순환(마지막이면 첫 탭 wrap). 1개 이하면 no-op.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// store 액션을 spy 로 갈아끼워 라우팅만 검증(실제 invoke 안 탐).
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))

import './tabCommands' // side-effect register
import { run } from './registry'
import { useViewStore } from '../store/viewStore'

const createTabSpy = vi.fn(async () => 'new-view')
const switchTabSpy = vi.fn(async () => undefined)
const closeTabSpy = vi.fn(async () => undefined)
const createWindowSpy = vi.fn(async () => 'slot-popup-1')
const closeWindowSpy = vi.fn(async () => undefined)

const origHash = window.location.hash

beforeEach(() => {
  createTabSpy.mockClear()
  switchTabSpy.mockClear()
  closeTabSpy.mockClear()
  createWindowSpy.mockClear()
  closeWindowSpy.mockClear()
  window.location.hash = '#/' // main 창 컨텍스트
  useViewStore.setState({
    windows: {},
    createTab: createTabSpy,
    switchTab: switchTabSpy,
    closeTab: closeTabSpy,
    createWindow: createWindowSpy,
    closeWindow: closeWindowSpy,
  })
})
afterEach(() => {
  window.location.hash = origHash
})

describe('tabCommands 라우팅 (window 해소)', () => {
  it('tab.create(window 생략) → createTab(이 창=main, undefined)', () => {
    run('tab.create')
    expect(createTabSpy).toHaveBeenCalledWith('main', undefined)
  })

  it('tab.create(window/name 지정) → createTab(그 창, name)', () => {
    run('tab.create', { window: 'slot-popup-2', name: 'My Tab' })
    expect(createTabSpy).toHaveBeenCalledWith('slot-popup-2', 'My Tab')
  })

  it('tab.switch(view 지정) → switchTab(이 창, view)', () => {
    run('tab.switch', { view: 'v7' })
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'v7')
  })

  it('tab.switch(view 누락) → throw', () => {
    expect(() => run('tab.switch')).toThrow()
  })

  it('tab.close(view 지정) → closeTab(이 창, view)', () => {
    run('tab.close', { view: 'v3' })
    expect(closeTabSpy).toHaveBeenCalledWith('main', 'v3')
  })

  // ── ★S4-F1★: view 생략 시 *지정된 창*의 active 를 쓴다(현재 웹뷰 active 아님) ──
  it('tab.close(window 지정·view 생략) → 그 창의 active 를 닫는다(현재 웹뷰 active 아님)', () => {
    // 이 웹뷰(main)의 active 는 mv, 타깃 창(slot-popup-1)의 active 는 pv. 타깃 pv 를 닫아야 한다.
    useViewStore.setState({
      windows: {
        main: { tabs: [{ id: 'mv', name: 'M' }], active: 'mv', version: 1 },
        'slot-popup-1': { tabs: [{ id: 'pv', name: 'P' }], active: 'pv', version: 1 },
      },
    })
    run('tab.close', { window: 'slot-popup-1' })
    // ★버그였다면 main 의 active(mv)로 닫아 (slot-popup-1, mv) 어긋남 → 백엔드 ViewNotFound★.
    expect(closeTabSpy).toHaveBeenCalledWith('slot-popup-1', 'pv')
  })

  it('tab.close(window·view 모두 생략) → 이 웹뷰 창(main)의 active 를 닫는다', () => {
    useViewStore.setState({
      windows: { main: { tabs: [{ id: 'mv', name: 'M' }], active: 'mv', version: 1 } },
    })
    run('tab.close')
    expect(closeTabSpy).toHaveBeenCalledWith('main', 'mv')
  })

  it('tab.close(view 미확정 — 그 창 상태 없음) → throw(닫을 탭 미확정)', () => {
    useViewStore.setState({ windows: {} })
    expect(() => run('tab.close', { window: 'slot-popup-9' })).toThrow()
  })

  it('window.create → createWindow()', () => {
    run('window.create')
    expect(createWindowSpy).toHaveBeenCalledTimes(1)
  })

  it('window.close(window 생략) → closeWindow(이 창=main)', () => {
    run('window.close')
    expect(closeWindowSpy).toHaveBeenCalledWith('main')
  })
})

describe('tab.next (Ctrl+Tab 순환)', () => {
  it('탭 여러 개 → 오른쪽 순환(active 다음 탭으로 switch)', () => {
    useViewStore.setState({
      windows: {
        main: { tabs: [{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }, { id: 'c', name: 'C' }], active: 'a', version: 1 },
      },
    })
    run('tab.next')
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'b')
  })

  it('마지막 탭이 active → 첫 탭으로 wrap', () => {
    useViewStore.setState({
      windows: { main: { tabs: [{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }], active: 'b', version: 1 } },
    })
    run('tab.next')
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'a')
  })

  it('탭 1개 → no-op(switch 안 부름)', () => {
    useViewStore.setState({
      windows: { main: { tabs: [{ id: 'a', name: 'A' }], active: 'a', version: 1 } },
    })
    run('tab.next')
    expect(switchTabSpy).not.toHaveBeenCalled()
  })

  it('창 상태 없음 → no-op', () => {
    useViewStore.setState({ windows: {} })
    run('tab.next')
    expect(switchTabSpy).not.toHaveBeenCalled()
  })
})
