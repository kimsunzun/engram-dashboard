
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

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
const renameTabSpy = vi.fn(async () => undefined)

const origHash = window.location.hash

beforeEach(() => {
  createTabSpy.mockClear()
  switchTabSpy.mockClear()
  closeTabSpy.mockClear()
  createWindowSpy.mockClear()
  closeWindowSpy.mockClear()
  renameTabSpy.mockClear()
  window.location.hash = '#/' // main 창 컨텍스트
  useViewStore.setState({
    windows: {},
    createTab: createTabSpy,
    switchTab: switchTabSpy,
    closeTab: closeTabSpy,
    createWindow: createWindowSpy,
    closeWindow: closeWindowSpy,
    renameTab: renameTabSpy,
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

  // ── ★S4-F1★ ──
  it('tab.close(window 지정·view 생략) → 그 창의 active 를 닫는다(현재 웹뷰 active 아님)', () => {
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

  // ── ★tab.rename(ADR-0057)★ ──
  it('tab.rename(view/name 지정) → renameTab(view, trim된 name)', () => {
    run('tab.rename', { view: 'v7', name: '  My Tab  ' })
    expect(renameTabSpy).toHaveBeenCalledWith('v7', 'My Tab')
  })

  it('tab.rename(view 누락) → throw(renameTab 안 부름)', () => {
    expect(() => run('tab.rename', { name: 'x' })).toThrow()
    expect(renameTabSpy).not.toHaveBeenCalled()
  })

  it('tab.rename(name 누락) → throw', () => {
    expect(() => run('tab.rename', { view: 'v1' })).toThrow()
    expect(renameTabSpy).not.toHaveBeenCalled()
  })

  it('tab.rename(공백 name) → throw(invoke 전 loud fail)', () => {
    expect(() => run('tab.rename', { view: 'v1', name: '   ' })).toThrow()
    expect(renameTabSpy).not.toHaveBeenCalled()
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

// ── ★layout.setSlotContent variant 형태 검증(FIX LOW)★ ─────────────────────────────────────────
// 이 스위트가 막는 것: {type:'agent'}(agent_id 누락) 같은 malformed 값이 레지스트리를 통과해 Rust
// 역직렬화에서야 늦게 터지는 것 — invoke 전에 loud fail.
describe('layout.setSlotContent variant 형태 검증', () => {
  const setSlotContentSpy = vi.fn(async () => undefined)
  beforeEach(() => {
    setSlotContentSpy.mockClear()
    useViewStore.setState({ setSlotContent: setSlotContentSpy })
  })

  it('agent variant 에 agent_id 없으면 throw(레지스트리 경계에서 loud fail, invoke 전)', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent' } }),
    ).toThrow(/agent_id/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('agent variant 의 agent_id 가 비문자열(숫자)이면 throw', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent', agent_id: 42 } }),
    ).toThrow(/agent_id/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('agent variant 의 agent_id 가 빈 문자열이면 throw', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent', agent_id: '' } }),
    ).toThrow(/agent_id/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('알 수 없는 type 은 여전히 throw(화이트리스트)', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'bogus' } }),
    ).toThrow(/SlotContent variant/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('정상 agent variant(agent_id 문자열) → setSlotContent 로 라우팅', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agent', agent_id: 'a-1' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'agent', agent_id: 'a-1' })
  })

  it('unit variant(empty/agent_list/preset_palette)는 추가 필드 없이 통과', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'empty' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's2', content: { type: 'agent_list' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's3', content: { type: 'preset_palette' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'empty' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's2', { type: 'agent_list' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's3', { type: 'preset_palette' })
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
