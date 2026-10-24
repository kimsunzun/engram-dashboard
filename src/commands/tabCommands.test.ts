
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))

import { invoke } from '@tauri-apps/api/core'

import './tabCommands' // side-effect register
import { run } from './registry'
import { useViewStore } from '../store/viewStore'
import type { WindowTabsPayload } from '../store/viewStore'
import type { LayoutNode } from '../api/layoutTypes'

const POPUP = 'slot-popup-0a1b2c3d-1'
const POPUP_ID = '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b'

function payload(label: string, window_id: string, number: number, ids: string[], active: string): WindowTabsPayload {
  return {
    label,
    window_id,
    number,
    title: '',
    tabs: ids.map(id => ({ id, name: id.toUpperCase(), named: true })),
    active,
    version: 1,
  }
}

/**
 * 셸 `list_tabs` 흉내 — 창 인자를 셸 해석기처럼 푼다(숫자만 = 번호 거절 · 영속 id · label). 거절은 셸처럼 문자열로 한다.
 */
function shellWith(...windows: WindowTabsPayload[]): void {
  vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd !== 'list_tabs') return undefined
    const arg = String((args as { window: string }).window).trim()
    if (/^\d+$/.test(arg)) throw '창 번호는 창 인자로 받지 않는다 — 번호는 다시 쓰인다'
    const hit = windows.find(w => w.label === arg || w.window_id === arg)
    if (!hit) throw `window 없음: ${arg}`
    return hit
  })
}

const createTabSpy = vi.fn(async () => 'new-view')
const switchTabSpy = vi.fn(async () => undefined)
const closeTabSpy = vi.fn(async () => undefined)
const NEW_WINDOW = { window: 'slot-popup-0a1b2c3d-1', window_id: '7d2f0b8e-1c3a-4f5e-9b6d-2a4c6e8f0a1b', number: 2 }
const createWindowSpy = vi.fn(async () => NEW_WINDOW)
const closeWindowSpy = vi.fn(async () => undefined)
const renameTabSpy = vi.fn(async () => undefined)

const origHash = window.location.hash

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation(async () => undefined)
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

  it('tab.create(name 앞뒤 공백) → createTab(그 창, trim된 name)', () => {
    run('tab.create', { window: 'slot-popup-2', name: '  My Tab  ' })
    expect(createTabSpy).toHaveBeenCalledWith('slot-popup-2', 'My Tab')
  })

  it('tab.create(공백 name) → throw(createTab 안 부름)', () => {
    expect(() => run('tab.create', { name: '   ' })).toThrow()
    expect(createTabSpy).not.toHaveBeenCalled()
  })

  it('tab.create(name 생략 · window 지정) → createTab(그 창, undefined) — 자동 이름', () => {
    run('tab.create', { window: 'slot-popup-2' })
    expect(createTabSpy).toHaveBeenCalledWith('slot-popup-2', undefined)
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

  // ── ★S4-F1★ — view 생략 = 셸이 푼 그 창의 활성 ──
  it('tab.close(window label·view 생략) → 그 창의 active 를 닫는다(현재 웹뷰 active 아님)', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'), payload(POPUP, POPUP_ID, 2, ['pv'], 'pv'))
    await run('tab.close', { window: POPUP })
    // ★버그였다면 main 의 active(mv)로 닫아 (popup, mv) 어긋남 → 백엔드 ViewNotFound★.
    expect(closeTabSpy).toHaveBeenCalledWith(POPUP, 'pv')
  })

  it('tab.close(window_id·view 생략) → 셸이 푼 label 과 그 창의 active 로 닫는다', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'), payload(POPUP, POPUP_ID, 2, ['pv'], 'pv'))
    await run('tab.close', { window: POPUP_ID })
    expect(invoke).toHaveBeenCalledWith('list_tabs', { window: POPUP_ID })
    expect(closeTabSpy).toHaveBeenCalledWith(POPUP, 'pv')
  })

  it('tab.close(window·view 모두 생략) → 이 웹뷰 창(main)의 active 를 닫는다', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'))
    await run('tab.close')
    expect(closeTabSpy).toHaveBeenCalledWith('main', 'mv')
  })

  it('tab.close(창 번호·view 생략) → 셸 거절을 던진다(닫지 않는다)', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'), payload(POPUP, POPUP_ID, 2, ['pv'], 'pv'))
    await expect(run('tab.close', { window: '2' })).rejects.toThrow(/^\[tab\.close\] 창 번호/)
    expect(closeTabSpy).not.toHaveBeenCalled()
  })

  it('tab.close(모르는 창·view 생략) → 셸 거절을 던진다', async () => {
    shellWith(payload('main', 'main', 1, ['mv'], 'mv'))
    await expect(run('tab.close', { window: 'slot-popup-9' })).rejects.toThrow(/window 없음: slot-popup-9/)
    expect(closeTabSpy).not.toHaveBeenCalled()
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

  it('window.create → createWindow() · 답 = {window, window_id, number}', async () => {
    await expect(run('window.create')).resolves.toEqual(NEW_WINDOW)
    expect(createWindowSpy).toHaveBeenCalledTimes(1)
  })

  it('window.close(window 생략) → closeWindow(이 창=main)', () => {
    run('window.close')
    expect(closeWindowSpy).toHaveBeenCalledWith('main')
  })

  // ── 빈 window 인자는 이 창으로 접지 않는다 ──
  it('window: undefined → 이 창(main)으로 떨어진다', () => {
    run('window.close', { window: undefined })
    expect(closeWindowSpy).toHaveBeenCalledWith('main')
  })

  it('window: null → undefined 처럼 이 창(main)으로 떨어진다', () => {
    run('window.close', { window: null })
    expect(closeWindowSpy).toHaveBeenCalledWith('main')
  })

  it('window 가 BigInt · 순환 객체여도 명령 접두를 단 오류로 던진다', () => {
    const circular: Record<string, unknown> = {}
    circular.self = circular
    for (const w of [1n, circular]) {
      expect(() => run('tab.create', { window: w })).toThrow(/^\[tab\.create\] window/)
    }
    expect(createTabSpy).not.toHaveBeenCalled()
  })

  it('window 빈 문자열 · 공백뿐 → throw(이 창으로 접지 않는다)', () => {
    for (const w of ['', '   ']) {
      expect(() => run('window.close', { window: w })).toThrow(/^\[window\.close\] window 는 비어 있지 않은 문자열/)
      expect(() => run('tab.create', { window: w })).toThrow(/^\[tab\.create\] window/)
    }
    expect(closeWindowSpy).not.toHaveBeenCalled()
    expect(createTabSpy).not.toHaveBeenCalled()
  })

  it('window 가 문자열이 아니면(숫자) throw', async () => {
    expect(() => run('tab.switch', { window: 2, view: 'v1' })).toThrow(/^\[tab\.switch\] window/)
    await expect(run('tab.close', { window: 2 })).rejects.toThrow(/^\[tab\.close\] window/)
    await expect(run('tab.next', { window: 2 })).rejects.toThrow(/^\[tab\.next\] window/)
    expect(switchTabSpy).not.toHaveBeenCalled()
    expect(closeTabSpy).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
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

  // ★대소문자 무관 수용 + 선언 철자 정규화(사용자 결정 2026-09-23)★: store·백엔드로 흘러가는 태그는
  //   호출자가 쓴 철자가 아니라 SlotContent 유니온의 선언 철자여야 한다(wire 바이트 동일).
  it('type 대소문자 무관 수용 → 선언 철자로 정규화해 라우팅', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'EMPTY' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's2', content: { type: 'Agent_List' } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's3', content: { type: 'PRESET_PALETTE' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'empty' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's2', { type: 'agent_list' })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's3', { type: 'preset_palette' })
  })

  it('agent variant 는 태그만 접고 agent_id 는 그대로 나른다', () => {
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'Agent', agent_id: 'A-1' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'agent', agent_id: 'A-1' })
  })

  // ── 사용량 슬롯(TRD S21 usage-limit-slot §1-8) — show_* 는 있으면 boolean · 빠진 칸은 그 슬롯의 지금 값(없으면 셸 serde 기본값) ──
  it('usage variant — show_* boolean 은 그대로 · 모르는 슬롯의 빠진 칸은 채우지 않고 넘긴다 · 태그는 접는다', () => {
    run('layout.setSlotContent', {
      viewId: 'v1',
      slotId: 's1',
      content: { type: 'Usage', show_claude: false, show_codex: true },
    })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's2', content: { type: 'usage', show_codex: false } })
    run('layout.setSlotContent', { viewId: 'v1', slotId: 's3', content: { type: 'usage' } })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's1', { type: 'usage', show_claude: false, show_codex: true })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's2', { type: 'usage', show_codex: false })
    expect(setSlotContentSpy).toHaveBeenCalledWith('v1', 's3', { type: 'usage' })
  })

  // 버스의 `layout.setSlotContent`(셸 락 안 병합)와 같은 결과여야 한다 — 두 LLM 길이 갈리지 않게(TRD §3 #20).
  it('usage variant — 이미 사용량인 슬롯의 빠진 show 칸은 지금 값으로 채운다 · 준 칸은 그대로', () => {
    const layout: LayoutNode = {
      type: 'split',
      id: 'sp',
      dir: 'left_right',
      ratio: 0.5,
      a: { type: 'slot', id: 'u1', content: { type: 'usage', show_claude: false, show_codex: false } },
      b: { type: 'slot', id: 'e1', content: { type: 'empty' } },
    }
    useViewStore.setState({ layouts: { v1: { layout } as never } })
    try {
      run('layout.setSlotContent', { viewId: 'v1', slotId: 'u1', content: { type: 'usage', show_codex: true } })
      run('layout.setSlotContent', { viewId: 'v1', slotId: 'u1', content: { type: 'usage' } })
      // 지금 사용량이 아닌 슬롯은 병합하지 않는다 — 셸이 빠진 칸을 true 로 읽는다.
      run('layout.setSlotContent', { viewId: 'v1', slotId: 'e1', content: { type: 'usage', show_claude: false } })
      expect(setSlotContentSpy.mock.calls).toEqual([
        ['v1', 'u1', { type: 'usage', show_claude: false, show_codex: true }],
        ['v1', 'u1', { type: 'usage', show_claude: false, show_codex: false }],
        ['v1', 'e1', { type: 'usage', show_claude: false }],
      ])
    } finally {
      useViewStore.setState({ layouts: {} })
    }
  })

  it('usage variant — show_* 가 boolean 이 아니면 invoke 전에 throw', () => {
    for (const bad of ['false', 0, null, {}]) {
      expect(() =>
        run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'usage', show_claude: bad } }),
      ).toThrow(/show_claude 는 boolean/)
      expect(() =>
        run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'usage', show_codex: bad } }),
      ).toThrow(/show_codex 는 boolean/)
    }
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })

  it('대소문자만 접는다 — 철자가 다르면 여전히 throw', () => {
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'agentlist' } }),
    ).toThrow(/SlotContent variant/)
    expect(() =>
      run('layout.setSlotContent', { viewId: 'v1', slotId: 's1', content: { type: 'Emptyy' } }),
    ).toThrow(/SlotContent variant/)
    expect(setSlotContentSpy).not.toHaveBeenCalled()
  })
})

// 탭 차례 · 활성은 셸 답(list_tabs) 기준 — 웹 스토어는 보지 않는다.
describe('tab.next (Ctrl+Tab 순환)', () => {
  it('탭 여러 개 → 오른쪽 순환(active 다음 탭으로 switch)', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b', 'c'], 'a'))
    await run('tab.next')
    expect(invoke).toHaveBeenCalledWith('list_tabs', { window: 'main' })
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'b')
  })

  it('마지막 탭이 active → 첫 탭으로 wrap', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'b'))
    await run('tab.next')
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'a')
  })

  it('탭 1개 → no-op(switch 안 부름)', async () => {
    shellWith(payload('main', 'main', 1, ['a'], 'a'))
    await run('tab.next')
    expect(switchTabSpy).not.toHaveBeenCalled()
  })

  it('웹 스토어에 그 창이 없어도 셸 답으로 옮긴다', async () => {
    useViewStore.setState({ windows: {} })
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'))
    await run('tab.next')
    expect(switchTabSpy).toHaveBeenCalledWith('main', 'b')
  })

  it('window label → 그 창에서 옮긴다', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'), payload(POPUP, POPUP_ID, 2, ['p', 'q'], 'q'))
    await run('tab.next', { window: POPUP })
    expect(switchTabSpy).toHaveBeenCalledWith(POPUP, 'p')
  })

  it('window_id → 셸이 푼 label 로 옮긴다', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'), payload(POPUP, POPUP_ID, 2, ['p', 'q'], 'p'))
    await run('tab.next', { window: POPUP_ID })
    expect(invoke).toHaveBeenCalledWith('list_tabs', { window: POPUP_ID })
    expect(switchTabSpy).toHaveBeenCalledWith(POPUP, 'q')
  })

  it('창 번호 → 셸 거절을 던진다(옮기지 않는다)', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'), payload(POPUP, POPUP_ID, 2, ['p', 'q'], 'p'))
    await expect(run('tab.next', { window: '2' })).rejects.toThrow(/^\[tab\.next\] 창 번호/)
    expect(switchTabSpy).not.toHaveBeenCalled()
  })

  it('모르는 창 → 셸 거절을 던진다', async () => {
    shellWith(payload('main', 'main', 1, ['a', 'b'], 'a'))
    await expect(run('tab.next', { window: 'slot-popup-9' })).rejects.toThrow(/window 없음: slot-popup-9/)
    expect(switchTabSpy).not.toHaveBeenCalled()
  })
})
