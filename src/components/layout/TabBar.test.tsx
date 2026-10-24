// TabBar 단위테스트 — 탭 렌더 · 누름 전환 · 닫기 · 추가 · 이름 인라인 편집(ADR-0057, §7-2) + 탭 끌기 출처(TRD
//   S21-slot-docking §4-2 · §4-4 · §4-6 — 훅 `useTabDragSource` · `dragSource` 를 실물로 · invoke · 사건은 가짜).
// 탭바(끌기 쪽 기대값의 손셈): 줄 x 100..800 · y 40..68 · `+` 40 → 쓸 수 있는 폭 660 · 탭 넷이면 모두 160 → 머리 x 0 · 160 ·
//   320 · 480. 탭 a(머리 x 100..260)를 x 180(잡은 오프셋 80)에서 누른다.

import { act, cleanup, createEvent, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// 창 사실 당기기(`window_frame`)는 탭바가 마운트마다 낸다 — 계약 모양으로 답해 경고를 내지 않게 한다.
const invokeMock = vi.hoisted(() =>
  vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> =>
    cmd === 'window_frame'
      ? {
          restored: null,
          maximized: false,
          insets: { t: 0, r: 0, b: 0, l: 0 },
          focused: true,
          caption: { buttons: true, leading_inset: 0, snap_overlay: false },
        }
      : undefined,
  ),
)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args) }))
const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    handlers.set(event, handler)
    return () => handlers.delete(event)
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))
vi.mock('@tauri-apps/api/app', () => ({ getIdentifier: async () => 'com.Engram.Test' }))

import TabBar from './TabBar'
import type { ViewMeta } from '../../api/layoutTypes'
import type { TabDrop } from '../../../src-tauri/bindings/TabDrop'
import { localDrag } from './dock/localDrag'
import { TAB_MIN_ACTIVE_W } from './tabStrip/layout'
import { __resetDragSourceForTest } from './tabStrip/useTabDragSource'

afterEach(cleanup)

const TABS: ViewMeta[] = [
  { id: 'v1', name: 'View 1', named: true },
  { id: 'v2', name: 'View 2', named: true },
]

type Props = Parameters<typeof TabBar>[0]

function renderBar(active: string | null = 'v1', tabs: ViewMeta[] = TABS, label = 'main') {
  const props: Props = {
    label,
    tabs,
    active,
    onSwitch: vi.fn(),
    onCreate: vi.fn(),
    onClose: vi.fn(),
    onRename: vi.fn(),
  }
  const view = render(<TabBar {...props} />)
  const rerender = (next: Partial<Props>) => {
    Object.assign(props, next)
    view.rerender(<TabBar {...props} />)
  }
  return { ...props, rerender }
}

const head = (id: string) => screen.getAllByTestId('tab').find(t => t.getAttribute('data-view-id') === id)!
const headIds = () => screen.getAllByTestId('tab').map(t => t.getAttribute('data-view-id'))
const nameOf = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-name"]')!

function openEdit(id: string) {
  fireEvent.doubleClick(nameOf(id))
  return screen.getByTestId('tab-rename-input') as HTMLInputElement
}

const openEditForV1 = () => openEdit('v1')

describe('TabBar', () => {
  it('tabs 를 렌더하고 active 탭을 data-active=true 로 강조한다', () => {
    renderBar('v2')
    const tabs = screen.getAllByTestId('tab')
    expect(tabs).toHaveLength(2)
    expect(head('v2').getAttribute('data-active')).toBe('true')
    expect(head('v1').getAttribute('data-active')).toBe('false')
  })

  it('비활성 탭 머리를 왼쪽 단추로 누르는 순간 onSwitch(그 view) — 클릭을 기다리지 않는다', () => {
    const { onSwitch } = renderBar('v1')
    fireEvent.pointerDown(nameOf('v2'), { button: 0, pointerId: 1 })
    expect(onSwitch).toHaveBeenCalledWith('v2')
    fireEvent.click(head('v2'))
    expect(onSwitch).toHaveBeenCalledTimes(1)
  })

  it('활성 탭 누름 · 오른쪽 단추 누름은 전환하지 않는다', () => {
    const { onSwitch } = renderBar('v1')
    fireEvent.pointerDown(head('v1'), { button: 0, pointerId: 1 })
    fireEvent.pointerDown(head('v2'), { button: 2, pointerId: 1 })
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('[+] 클릭 → onCreate', () => {
    const { onCreate } = renderBar()
    fireEvent.click(screen.getByTestId('tab-add'))
    expect(onCreate).toHaveBeenCalledTimes(1)
  })

  it('탭 × 클릭 → onClose(그 view id) · × 누름은 머리 누름이 아니다(전환 없음)', () => {
    const { onClose, onSwitch } = renderBar('v1')
    const close = head('v2').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.pointerDown(close, { button: 0, pointerId: 1 })
    fireEvent.click(close)
    expect(onClose).toHaveBeenCalledWith('v2')
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('닫기 · + 단추 = 22 px 원 안의 아이콘 · + 원 앞 틈은 margin 이 아니라 왼 패딩', () => {
    renderBar('v1')
    const close = head('v1').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    expect(close.style.width).toBe('22px')
    expect(close.style.height).toBe('22px')
    expect(close.style.borderRadius).toBe('50%')
    expect(close.querySelector('svg')).not.toBeNull()
    expect(close.textContent).toBe('')
    const add = screen.getByTestId('tab-add')
    expect(add.style.paddingLeft).toBe('4px')
    expect(add.style.marginLeft).toBe('')
    const circle = screen.getByTestId('tab-add-circle')
    expect(circle.style.width).toBe('22px')
    expect(circle.style.height).toBe('22px')
    expect(circle.style.borderRadius).toBe('50%')
    expect(circle.querySelector('svg')).not.toBeNull()
  })

  // jsdom 은 :hover 를 칠하지 않는다 — 클래스가 붙었는지 · 그것을 이기는 인라인 배경이 없는지까지만 본다.
  it('× · + 원의 맴돎 색은 테마 토큰 클래스 · 인라인 배경 없음 · 두 원은 머리 내용과 같은 세로 띠', () => {
    renderBar('v1')
    const close = head('v1').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    expect(close.className.split(' ')).toContain('hover:bg-[var(--tab-button-hover-bg)]')
    expect(close.style.background).toBe('')
    expect(close.style.backgroundColor).toBe('')
    const add = screen.getByTestId('tab-add')
    const circle = screen.getByTestId('tab-add-circle')
    expect(add.className.split(' ')).toContain('group/add')
    expect(circle.className.split(' ')).toContain('group-hover/add:bg-[var(--tab-button-hover-bg)]')
    expect([circle.style.background, circle.style.backgroundColor]).toEqual(['', ''])

    // × 원은 머리 내용 칸 가운데, + 원은 + 상자 가운데 — 두 칸의 위 · 아래가 같으면 원 가운데 높이도 같다.
    const content = close.parentElement!
    expect([content.style.top, content.style.bottom, content.style.alignItems]).toEqual(['4px', '0px', 'center'])
    expect([add.style.top, add.style.bottom, add.style.alignItems]).toEqual([content.style.top, content.style.bottom, 'center'])
  })

  // 이름은 머리 글자색을 물려받는다 — 머리와 × 의 인라인 색이 같으면 이름 · × 가 한 색이다.
  it('이름 · × 는 한 전경색 — 활성 · 맴돎 = --text · 쉴 때 = --tab-text-inactive · + 도 쉴 때 그 회색', () => {
    renderBar('v1')
    const close = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-close"]')!
    const colours = (id: string) => [nameOf(id).style.color, head(id).style.color, close(id).style.color]
    expect(colours('v1')).toEqual(['', 'var(--text)', 'var(--text)'])
    expect(colours('v2')).toEqual(['', 'var(--tab-text-inactive)', 'var(--tab-text-inactive)'])

    fireEvent.pointerEnter(head('v2'))
    expect(colours('v2')).toEqual(['', 'var(--text)', 'var(--text)'])
    fireEvent.pointerLeave(head('v2'))
    expect(colours('v2')).toEqual(['', 'var(--tab-text-inactive)', 'var(--tab-text-inactive)'])

    expect(screen.getByTestId('tab-add').style.color).toBe('var(--tab-text-inactive)')
  })

  it('+ 원(아이콘) 클릭 → onCreate · 닫기 아이콘 클릭 → onClose', () => {
    const { onCreate, onClose } = renderBar('v1')
    fireEvent.click(screen.getByTestId('tab-add-circle').querySelector('svg')!)
    expect(onCreate).toHaveBeenCalledTimes(1)
    fireEvent.click(head('v2').querySelector('[data-testid="tab-close"] svg')!)
    expect(onClose).toHaveBeenCalledWith('v2')
  })

  it('이름 더블클릭 → 인라인 편집 진입(input 등장, 현재 이름 시드)', () => {
    renderBar('v1')
    const input = openEditForV1()
    expect(input.value).toBe('View 1')
    expect(input.getAttribute('data-view-id')).toBe('v1')
  })

  it('Enter(새 이름) → onRename(id, trim된 새이름) — 정확히 1회(Enter+blur 이중호출 방지, FIX 3)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '  Renamed  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).toHaveBeenCalledWith('v1', 'Renamed')
    // 언마운트 뒤 blur 는 React 19 가 commit 중 배달을 꺼 버리고 그 뒤엔 떨어진 노드라 루트 리스너에 닿지 않는다.
    //   그래서 이 떨어진 input 의 fireEvent.blur 는 commitEdit 의 `editingId` 확인까지 가지 않는다 — 그 확인을 재는 시험이 아니다.
    fireEvent.blur(input)
    expect(onRename).toHaveBeenCalledTimes(1)
  })

  it('편집 진입 시 select() 가 정확히 1회만 실행(매 키입력 재선택 방지, FIX 1)', () => {
    renderBar('v1')
    const selectSpy = vi.spyOn(HTMLInputElement.prototype, 'select')
    const input = openEditForV1()
    expect(selectSpy).toHaveBeenCalledTimes(1)
    // 키입력(setDraft→re-render)이 select 를 재실행하면 방금 친 글자가 통째로 덮어써진다.
    fireEvent.change(input, { target: { value: 'N' } })
    fireEvent.change(input, { target: { value: 'Ne' } })
    fireEvent.change(input, { target: { value: 'New' } })
    expect(selectSpy).toHaveBeenCalledTimes(1)
    expect(input.value).toBe('New')
    selectSpy.mockRestore()
  })

  it('Esc → 편집 취소(onRename 안 부름, revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: 'Whatever' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(onRename).not.toHaveBeenCalled()
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('빈/공백 이름 확정(Enter) → onRename 안 부름(revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  it('이름 미변경 확정(Enter) → onRename 안 부름(no-op)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  // TRD S26-window-chrome §3-1 — 자동 이름을 그 글자 그대로 확정해도 이름을 정한 것이다.
  it('자동 이름 탭의 같은 글자 확정(Enter) → onRename(id, 그 글자) 1회', () => {
    const { onRename } = renderBar('v1', [
      { id: 'v1', name: '에이전트 트리', named: false },
      { id: 'v2', name: 'View 2', named: true },
    ])
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: ' 에이전트 트리 ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    fireEvent.blur(input)
    expect(onRename).toHaveBeenCalledTimes(1)
    expect(onRename).toHaveBeenCalledWith('v1', '에이전트 트리')
  })

  it('자동 이름 탭도 빈/공백 확정(Enter) → onRename 안 부름', () => {
    const { onRename } = renderBar('v1', [{ id: 'v1', name: '빈 칸', named: false }])
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  // 같은 글자는 Enter 로만 보낸다 — 메인 결정(U3a 리뷰 · 2026-10-25).
  it('자동 이름 탭 — 글자를 안 바꾸고 포커스를 잃으면 onRename 안 부르고 편집만 끝낸다', () => {
    const { onRename } = renderBar('v1', [
      { id: 'v1', name: '에이전트 트리', named: false },
      { id: 'v2', name: 'View 2', named: true },
    ])
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: ' 에이전트 트리 ' } })
    fireEvent.blur(input)
    expect(onRename).not.toHaveBeenCalled()
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('자동 이름 탭 — 바꾼 글자는 포커스 잃음으로도 보낸다 · 1회', () => {
    const { onRename } = renderBar('v1', [
      { id: 'v1', name: '에이전트 트리', named: false },
      { id: 'v2', name: 'View 2', named: true },
    ])
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: ' 내 탭 ' } })
    fireEvent.blur(input)
    expect(onRename).toHaveBeenCalledTimes(1)
    expect(onRename).toHaveBeenCalledWith('v1', '내 탭')
  })

  it('편집 중 이름이 바뀌어도(자동 이름이 따라감 · LLM 이름 바꾸기) 손대지 않은 칸의 포커스 잃음 · Enter 는 옛 이름을 써 넣지 않는다', () => {
    const bar = renderBar('v1', [
      { id: 'v1', name: '에이전트 트리', named: false },
      { id: 'v2', name: 'View 2', named: true },
    ])
    let input = openEditForV1()
    bar.rerender({ tabs: [{ id: 'v1', name: 'claude', named: false }, bar.tabs[1]] })
    fireEvent.blur(input)
    expect(bar.onRename).not.toHaveBeenCalled()

    input = openEditForV1()
    bar.rerender({ tabs: [{ id: 'v1', name: 'LLM 이름', named: true }, bar.tabs[1]] })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(bar.onRename).not.toHaveBeenCalled()
  })

  it('앞뒤 공백이 붙은 자동 이름 — 손대지 않고 포커스를 잃으면 onRename 안 부른다', () => {
    const { onRename } = renderBar('v1', [{ id: 'v1', name: ' proj ', named: false }])
    const input = openEditForV1()
    expect(input.value).toBe(' proj ')
    fireEvent.blur(input)
    expect(onRename).not.toHaveBeenCalled()
  })

  it('앞뒤 공백이 붙은 정한 이름 — 편집 중 LLM 이 바꿔도 손대지 않은 칸의 포커스 잃음 · Enter 는 옛 이름을 써 넣지 않는다', () => {
    const bar = renderBar('v1', [{ id: 'v1', name: ' old ', named: true }])
    let input = openEditForV1()
    bar.rerender({ tabs: [{ id: 'v1', name: 'new', named: true }] })
    fireEvent.blur(input)

    bar.rerender({ tabs: [{ id: 'v1', name: ' old ', named: true }] })
    input = openEditForV1()
    bar.rerender({ tabs: [{ id: 'v1', name: 'new', named: true }] })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(bar.onRename).not.toHaveBeenCalled()
  })

  it('편집 중 input 키 입력이 바깥(전역 키바인딩)으로 새지 않는다(stopPropagation)', () => {
    renderBar('v1')
    const input = openEditForV1()
    const outside = vi.fn()
    document.addEventListener('keydown', outside)
    fireEvent.keyDown(input, { key: 'a' })
    document.removeEventListener('keydown', outside)
    expect(outside).not.toHaveBeenCalled()
  })

  it('비활성 탭 이름 더블클릭 제스처 → 누름에서 그 탭으로 전환 · 편집 진입', () => {
    const { onSwitch } = renderBar('v1')
    const name = nameOf('v2')
    for (const detail of [1, 2]) {
      fireEvent.pointerDown(name, { button: 0, pointerId: 1 })
      fireEvent.pointerUp(name, { button: 0, pointerId: 1 })
      fireEvent.click(name, { detail })
    }
    fireEvent.doubleClick(name, { detail: 2 })
    expect(screen.getByTestId('tab-rename-input').getAttribute('data-view-id')).toBe('v2')
    expect(onSwitch).toHaveBeenCalled()
    expect(vi.mocked(onSwitch).mock.calls.every(c => c[0] === 'v2')).toBe(true)
  })

  it('× 를 연속 클릭으로 닫다가 밀려 온 다음 탭의 × (detail 2)는 닫는다 — 전에 머리 누름으로 활성이 된 탭이라도 · 이름 편집은 열지 않는다', () => {
    const { onClose } = renderBar('v1')
    fireEvent.pointerDown(nameOf('v2'), { button: 0, pointerId: 1 })
    const close = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.click(close('v1'), { detail: 1 })
    fireEvent.click(close('v2'), { detail: 2 })
    fireEvent.doubleClick(close('v2'), { detail: 2 })
    expect(vi.mocked(onClose).mock.calls).toEqual([['v1'], ['v2']])
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('이름 칸 밖 머리 더블클릭도 편집을 열고 · 편집 중 입력 칸 안의 더블클릭은 친 글자를 지우지 않는다', () => {
    renderBar('v1')
    fireEvent.doubleClick(head('v1'))
    const input = screen.getByTestId('tab-rename-input') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'Typed' } })
    fireEvent.doubleClick(input)
    expect((screen.getByTestId('tab-rename-input') as HTMLInputElement).value).toBe('Typed')
  })

  it('머리 = div · OS 끌기 출처(draggable)', () => {
    renderBar('v1')
    for (const el of screen.getAllByTestId('tab')) {
      expect(el.tagName).toBe('DIV')
      expect(el.getAttribute('draggable')).toBe('true')
    }
  })

  it('탭바 줄 = 위치 기준 · 스크롤 없음 · 왼쪽 패딩 없음 · 머리와 + 만 담는다', () => {
    renderBar('v1')
    const row = screen.getByTestId('tab-strip-row')
    expect(row.style.position).toBe('relative')
    expect(row.style.overflow).toBe('')
    expect(row.style.overflowX).toBe('')
    expect(row.style.paddingLeft).toBe('')
    expect(row.style.borderLeft).toBe('')
    const kinds = Array.from(row.children, c => (c as HTMLElement).dataset.testid)
    expect(kinds).toEqual(['tab', 'tab', 'tab-add'])
    expect(screen.getByTestId('tab-add').style.marginLeft).toBe('')
  })

  it('구분선은 그린 이웃 사이마다 있고 · 활성 탭과 맴도는 탭의 양옆에서는 떼지 않고 사라진다 · 활성 면은 활성 탭에만', () => {
    const tabs = ['a', 'b', 'c', 'd', 'e', 'f'].map(id => ({ id, name: id, named: true }))
    renderBar('b', tabs)
    // 구분선은 머리 왼쪽 가에 둔다 — 머리 x 의 구분선 = (x 앞 이웃 | x) 사이.
    const sep = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-separator"]')
    const separated = () => tabs.map(t => t.id).filter(id => sep(id)?.style.opacity === '1')
    const has = (id: string, testid: string) => head(id).querySelector(`[data-testid="${testid}"]`) !== null
    expect(tabs.map(t => sep(t.id) !== null)).toEqual([false, true, true, true, true, true])
    expect(separated()).toEqual(['d', 'e', 'f'])
    expect(sep('c')!.style.opacity).toBe('0')
    expect(tabs.map(t => has(t.id, 'tab-surface'))).toEqual([false, true, false, false, false, false])

    fireEvent.pointerEnter(head('e'))
    expect(separated()).toEqual(['d'])
    expect(tabs.map(t => sep(t.id) !== null)).toEqual([false, true, true, true, true, true])
    expect(has('e', 'tab-hover')).toBe(true)

    // 활성 탭 위 맴돎은 맴돎 면을 따로 그리지 않는다.
    fireEvent.pointerLeave(head('e'))
    fireEvent.pointerEnter(head('b'))
    expect(separated()).toEqual(['d', 'e', 'f'])
    expect(has('b', 'tab-hover')).toBe(false)
  })

  // PRD R27 · ADR-0295 결정 1 — 위 모서리 10 · 아래 날개 12 · 좌우 여백 2 · ADR-0298 결정 3 — 구분선 1×13.
  it('활성 탭 모양 = 위 모서리 반지름 10 · 아래 바깥 날개 반지름 12 · 구분선 두께 1 · 높이 13 · 좌우 여백 2', () => {
    const tabs = ['a', 'b', 'c'].map(id => ({ id, name: id, named: true }))
    renderBar('b', tabs)
    const part = (id: string, testid: string) => head(id).querySelector<HTMLElement>(`[data-testid="${testid}"]`)!

    expect(part('b', 'tab-surface').style.borderRadius).toBe('10px 10px 0 0')
    const left = part('b', 'tab-flare-left').style
    const right = part('b', 'tab-flare-right').style
    expect([left.width, left.height, left.left, left.right]).toEqual(['12px', '12px', '-12px', ''])
    expect([right.width, right.height, right.right, right.left]).toEqual(['12px', '12px', '-12px', ''])
    for (const flare of [left, right]) {
      expect(flare.background).toContain('transparent 11.5px')
      expect(flare.background).toMatch(/var\(--tab-active-bg\) 12px/)
    }

    // 여백 · 두께 · 여백(2 + 1 + 2) 칸이 머리 경계 가운데에 걸린다.
    const slot = part('c', 'tab-separator')
    expect([slot.style.width, slot.style.left, slot.style.paddingLeft, slot.style.paddingRight]).toEqual([
      '5px',
      '-2.5px',
      '2px',
      '2px',
    ])
    expect(slot.style.boxSizing).toBe('border-box')
    expect(slot.style.alignItems).toBe('center')
    const line = (slot.firstElementChild as HTMLElement).style
    expect([line.width, line.height, line.borderRadius]).toEqual(['1px', '13px', '0.5px'])
  })

  // Chrome `horizontal_tab_style_views.cc` `GetSeparatorOpacity` · `GetHoverInterpolatedSeparatorOpacity`.
  it('구분선은 곧바로 사라지고 · 맴돎이 끝난 머리 양옆만 200 ms EASE_IN 으로 나타나며 · 그 뒤와 활성이 옮은 되살아남은 곧바로다', () => {
    vi.useFakeTimers()
    try {
      const tabs = ['a', 'b', 'c', 'd'].map(id => ({ id, name: id, named: true }))
      const bar = renderBar('a', tabs)
      const sep = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-separator"]')!
      const FADE_IN = 'opacity 200ms cubic-bezier(0.3333, 0, 0.6667, 0.3333)'
      const d = sep('d')
      expect([d.style.opacity, d.style.transition]).toEqual(['1', ''])

      fireEvent.pointerEnter(head('d'))
      expect(sep('d')).toBe(d)
      expect([d.style.opacity, d.style.transition]).toEqual(['0', ''])

      fireEvent.pointerLeave(head('d'))
      expect([d.style.opacity, d.style.transition]).toEqual(['1', FADE_IN])
      act(() => {
        vi.advanceTimersByTime(200)
      })
      expect([d.style.opacity, d.style.transition]).toEqual(['1', ''])

      // 활성이 b → c 로 옮으면 (a|b) 는 맴돎 없이 곧바로 되살아나고 (c|d) 는 곧바로 사라진다.
      bar.rerender({ active: 'b' })
      bar.rerender({ active: 'c' })
      expect([sep('b').style.opacity, sep('b').style.transition]).toEqual(['1', ''])
      expect([d.style.opacity, d.style.transition]).toEqual(['0', ''])
    } finally {
      vi.useRealTimers()
    }
  })

  // 사용자 보고 2026-10-21 — 마지막 비활성 머리의 × 와 + 가 붙어 보였다.
  it('마지막 머리 오른쪽 가에 끝 구분선 — 마지막 머리에만 · 경계 가운데 · 비활성 · 맴돎 아님일 때만 보인다', () => {
    const tabs = ['a', 'b', 'c'].map(id => ({ id, name: id, named: true }))
    const bar = renderBar('a', tabs)
    const trail = () => screen.getByTestId('tab-separator-trailing')
    expect(screen.getAllByTestId('tab-separator-trailing')).toHaveLength(1)
    expect(head('c').contains(trail())).toBe(true)
    expect([trail().style.right, trail().style.left, trail().style.width]).toEqual(['-2.5px', '', '5px'])
    expect(trail().style.opacity).toBe('1')

    fireEvent.pointerEnter(head('c'))
    expect(trail().style.opacity).toBe('0')
    fireEvent.pointerLeave(head('c'))
    expect(trail().style.opacity).toBe('1')

    bar.rerender({ active: 'c' })
    expect(trail().style.opacity).toBe('0')
  })
})

// ── 탭 끌기(OS 끌기 출처 — TRD S21-slot-docking §4-2 · §4-4 · §4-6 · §8-2 「훅 · 컴포넌트」) ──
// jsdom 에는 DragEvent · DataTransfer 가 없어 끌기 자료 칸을 붙인 사건을 낸다. 출처 창 탭바 판정(`localDrag.setSourceStrip`)은
//   뿌리 받기의 몫이라 시험이 곧바로 적는다. 엔진이 끌기를 접는 판정 · 실제 사건 간격은 이 시험이 못 덮는다.

let rowWidth: number

const EASE = 'transform 200ms cubic-bezier(0.3333, 0.6667, 0.6667, 1)'
// 끌기 밖 — 속성은 그대로 · 시간만 0(진행 중인 전환을 끊지 않는다).
const SNAP = 'transform 0ms cubic-bezier(0.3333, 0.6667, 0.6667, 1)'
/** 셸이 미는 창 사실(`window_frame` 당기기의 답). */
const FRAME = {
  restored: { w: 1000, h: 700 },
  maximized: false,
  insets: { t: 31, r: 8, b: 8, l: 8 },
  focused: true,
  caption: { buttons: true, leading_inset: 0, snap_overlay: false },
}

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}
const invokes = (cmd: string) => invokeMock.mock.calls.filter(c => c[0] === cmd).map(c => c[1])
const drops = () => invokes('tab_drop').map(a => (a as { drop: TabDrop }).drop)

function rect(left: number, top: number, width: number, height: number): DOMRect {
  return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top } as DOMRect
}

// 그린 그대로를 잰다 — 머리는 `translateX` · 폭에서, 줄 · `+` 는 위 손셈의 상수.
function fakeRect(this: HTMLElement): DOMRect {
  const id = this.dataset.testid
  if (id === 'tab-strip-row') return rect(100, 40, rowWidth, 28)
  if (id === 'tab-add') return rect(0, 40, 40, 28)
  if (id === 'tab') return rect(100 + xOf(this), 40, parseFloat(this.style.width), 28)
  return rect(0, 0, 0, 0)
}

function xOf(el: HTMLElement): number {
  return Number(/translateX\((-?[\d.]+)px\)/.exec(el.style.transform)![1])
}

function pointer(type: string, x: number, y = 50, buttons = type === 'pointermove' ? 1 : 0): void {
  act(() => {
    window.dispatchEvent(new PointerEvent(type, { clientX: x, clientY: y, pointerId: 1, buttons, bubbles: true }))
  })
}

const emit = (event: string, payload: unknown) => act(() => handlers.get(event)!({ payload }))

interface FakeDt {
  data: Map<string, string>
  effectAllowed: string
  dropEffect: string
  setData: ReturnType<typeof vi.fn>
  clearData: ReturnType<typeof vi.fn>
  setDragImage: ReturnType<typeof vi.fn>
}

function fakeDt(dropEffect = 'none'): FakeDt {
  const data = new Map<string, string>()
  return {
    data,
    effectAllowed: 'uninitialized',
    dropEffect,
    setData: vi.fn((k: string, v: string) => void data.set(k, v)),
    clearData: vi.fn(() => data.clear()),
    setDragImage: vi.fn(),
  }
}

/** 끌기 사건 하나 — `clientX` · `clientY` 를 싣는다(testing-library 는 DragEvent 가 없으면 그 칸을 버린다). 반환 = 취소 안 됨. */
function drag(type: 'dragStart' | 'dragEnd', el: HTMLElement, dt: FakeDt, x = 191, y = 50): boolean {
  const ev = createEvent[type](el, { dataTransfer: dt })
  Object.defineProperty(ev, 'clientX', { value: x })
  Object.defineProperty(ev, 'clientY', { value: y })
  let notCancelled = true
  act(() => {
    notCancelled = fireEvent(el, ev)
  })
  return notCancelled
}

const typeOf = (dt: FakeDt) => [...dt.data.keys()][0]
const valueOf = (dt: FakeDt) => JSON.parse(dt.data.get(typeOf(dt))!) as { drag: string; view: string; initial: string | null }

/** dragstart 와 다른 task — 엔진이 접은 끌기가 아니다. */
const nextTask = () =>
  act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0))
    await ticks()
  })

const strip = (index: number, left: number, x: number) => act(() => localDrag.setSourceStrip({ index, left, x }))
const leaveZone = () => act(() => localDrag.setSourceStrip(null))

// 탭바의 전환 — jsdom 에는 `getAnimations` 가 없어 손으로 단다(없으면 전환 없음으로 읽힌다).
interface FakeAnimation {
  playState: string
  pending: boolean
  finished: Promise<unknown>
}
let running: FakeAnimation[] = []
const animatable = HTMLElement.prototype as unknown as { getAnimations?: () => unknown[] }

/** 맞추기는 뗌 다음 사건 뒤의 타이머에서 시작한다 — 그 타이머와 뒤따른 기다림까지 보낸다. */
const flush = () =>
  act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0))
    await ticks()
  })

/** 자리 전환 하나를 진행 중으로 둔다 — 돌려준 함수가 그 전환을 끝낸다. */
function holdTransition(): () => Promise<void> {
  let finish = (): void => {}
  const finished = new Promise<void>(resolve => (finish = resolve))
  const anim = { playState: 'running', pending: false, finished }
  running.push(anim)
  return async () => {
    running = running.filter(a => a !== anim)
    finish()
    await flush()
  }
}

const ABCD: ViewMeta[] = ['a', 'b', 'c', 'd'].map(id => ({ id, name: id.toUpperCase(), named: true }))
const BACD = [ABCD[1], ABCD[0], ABCD[2], ABCD[3]]

/**
 * 탭바 a b c d · 누르기 전 활성 b. a 머리(x 100..260)를 x 180 에서 눌러(잡은 오프셋 80 · 전환 echo 까지) x 191 에서 dragstart.
 * 반환 = 그 끌기의 dataTransfer.
 */
async function startA(bar: ReturnType<typeof renderBar>): Promise<FakeDt> {
  fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
  expect(bar.onSwitch).toHaveBeenCalledWith('a')
  bar.rerender({ active: 'a' })
  const dt = fakeDt()
  drag('dragStart', head('a'), dt)
  await nextTask()
  return dt
}

describe('TabBar — 탭 끌기(OS 끌기 출처)', () => {
  beforeEach(async () => {
    rowWidth = 700
    handlers.clear()
    invokeMock.mockReset()
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'window_frame') return FRAME
      if (cmd === 'tab_drop') return { type: 'ignored', why: 'lost-drop' }
      if (cmd === 'switch_tab_if') return true
      return undefined
    })
    __resetDragSourceForTest()
    act(() => {
      localDrag.setOutgoing(null)
      localDrag.setSourceStrip(null)
      localDrag.setTargetMarker(null)
    })
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(fakeRect)
    window.matchMedia = vi.fn(() => ({ matches: false }) as MediaQueryList)
    Object.defineProperty(window.screen, 'availWidth', { value: 1920, configurable: true })
    Object.defineProperty(window.screen, 'availHeight', { value: 1080, configurable: true })
    running = []
    animatable.getAnimations = () => running
  })
  afterEach(() => {
    cleanup()
    delete animatable.getAnimations
    vi.restoreAllMocks()
  })

  /** 마운트 뒤 identifier · 창 사실 당기기가 닿기까지. */
  async function mount(active: string | null = 'b', tabs: ViewMeta[] = ABCD, label = 'main') {
    const bar = renderBar(active, tabs, label)
    await act(async () => {
      await ticks()
    })
    return bar
  }

  // ── 그리기(끌기 밖) ──

  it('머리 폭 · 자리 = 탭바 레이아웃(쓸 수 있는 폭 = 줄 − +) · + 는 마지막 머리 뒤', async () => {
    await mount('a')
    expect(['a', 'b', 'c', 'd'].map(id => [xOf(head(id)), head(id).style.width])).toEqual([
      [0, '160px'],
      [160, '160px'],
      [320, '160px'],
      [480, '160px'],
    ])
    expect(xOf(screen.getByTestId('tab-add'))).toBe(640)
    expect(head('a').style.transition).toBe(SNAP)
  })

  it('좁은 탭바 — 보이는 머리만 그리고 활성 최소 폭보다 좁은 비활성 머리는 닫기 버튼을 감춘다', async () => {
    rowWidth = 140 // 쓸 수 있는 폭 100 → 폭 52 · 32 · 32 · 32 · 보이는 것 = 앞 둘
    await mount('a')
    expect(headIds()).toEqual(['a', 'b'])
    expect(head('a').style.width).toBe(`${TAB_MIN_ACTIVE_W}px`)
    expect(head('a').querySelector('[data-testid="tab-close"]')).not.toBeNull()
    expect(head('b').style.width).toBe('32px')
    expect(head('b').querySelector('[data-testid="tab-close"]')).toBeNull()
  })

  // ── draggable 조건 ──

  it('머리 = 끌기 출처(draggable) — 이름 편집 중인 머리 · main 의 마지막 탭(Q35)은 아니다 · 탭 하나뿐인 팝아웃은 끈다(Q34)', async () => {
    const bar = await mount('a')
    expect(['a', 'b', 'c', 'd'].map(id => head(id).getAttribute('draggable'))).toEqual(['true', 'true', 'true', 'true'])
    fireEvent.doubleClick(nameOf('b'))
    expect(head('b').getAttribute('draggable')).toBe('false')
    expect(head('a').getAttribute('draggable')).toBe('true')
    fireEvent.keyDown(screen.getByTestId('tab-rename-input'), { key: 'Escape' })

    bar.rerender({ tabs: [ABCD[0]], active: 'a' })
    expect(head('a').getAttribute('draggable')).toBe('false')
    cleanup()
    await mount('p', [{ id: 'p', name: 'P', named: true }], 'slot-popup-2')
    expect(head('p').getAttribute('draggable')).toBe('true')
  })

  // ── dragstart ──

  it('dragstart = 우리 형식 하나(타입 문자열 · 값) · effectAllowed move · 라벨 그림 · window 에 안 닿음 · 머리 자리 · DOM 그대로', async () => {
    const bar = await mount('b')
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    bar.rerender({ active: 'a' })
    const row = screen.getByTestId('tab-strip-row')
    const nodes = screen.getAllByTestId('tab')
    const before = nodes.map(n => n.style.transform)
    const mo = new MutationObserver(() => undefined)
    mo.observe(row, { subtree: true, childList: true })
    const seen = vi.fn()
    window.addEventListener('dragstart', seen)
    const dt = fakeDt()

    const notCancelled = drag('dragStart', head('a'), dt)
    const added = mo.takeRecords()
    mo.disconnect()
    window.removeEventListener('dragstart', seen)

    expect(notCancelled).toBe(true)
    expect(seen).not.toHaveBeenCalled()
    expect(dt.clearData).toHaveBeenCalled()
    expect(dt.setData).toHaveBeenCalledTimes(1)
    expect(dt.effectAllowed).toBe('move')
    // 타입 = 소문자 · app · drag · 원래 크기(복원 1000×700 − 크롬 0) · 모양 칸 없음(캐시 없음).
    const value = valueOf(dt)
    expect(typeOf(dt)).toBe(`application/x-engram-tab;app=com.engram.test;drag=${value.drag};w=1000;h=700`)
    expect(value).toEqual({
      v: 1,
      drag: value.drag,
      view: 'a',
      from: 'main',
      initial: 'b',
      size: { w: 1000, h: 700 },
      // 탭바 왼쪽 100 + 잡은 비율 0.5 × 새 창 탭 하나 폭 160 · 누른 y.
      grab: { x: 180, y: 50 },
    })
    // VS Code 꼴 라벨 — 커서 (−10, −10) · 다음 task 에 뗀다.
    const [label, ox, oy] = dt.setDragImage.mock.calls[0] as [HTMLElement, number, number]
    expect([label.textContent, ox, oy]).toEqual(['A', -10, -10])
    expect(label.isConnected).toBe(true)
    // ★dragstart 안에서 끄는 머리가 누른 점 밑을 떠나면 엔진이 끌기를 접는다★ — 같은 노드 · 같은 자리 · 머리 넣고 빼기 없음.
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    expect(nodes.map(n => n.style.transform)).toEqual(before)
    expect(added).toEqual([])
    expect(localDrag.getOutgoing()).toEqual({ drag: value.drag, window: 'main', view: 'a', grabOffset: 80 })
    expect(invokeMock.mock.calls.filter(c => c[0] !== 'window_frame')).toEqual([])

    await nextTask()
    expect(label.isConnected).toBe(false)
  })

  it('닫기 버튼에서 누른 끌기(엔진은 머리에 낸다) · 이름 입력에서 시작한 글자 끌기 → 취소 · 자료 없음 · window 까지 닿음', async () => {
    await mount('a')
    const seen = vi.fn()
    window.addEventListener('dragstart', seen)
    try {
      fireEvent.pointerDown(screen.getAllByTestId('tab-close')[1], { button: 0 })
      const blocked = fakeDt()
      expect(drag('dragStart', head('b'), blocked)).toBe(false)
      expect(blocked.setData).not.toHaveBeenCalled()

      const input = openEdit('a')
      const text = fakeDt()
      expect(drag('dragStart', input, text)).toBe(false)
      expect(text.setData).not.toHaveBeenCalled()
      expect(seen).toHaveBeenCalledTimes(2)
    } finally {
      window.removeEventListener('dragstart', seen)
    }
    expect(localDrag.getOutgoing()).toBeNull()

    // 다음 누름은 다시 끈다.
    fireEvent.keyDown(screen.getByTestId('tab-rename-input'), { key: 'Escape' })
    fireEvent.pointerDown(nameOf('c'), { button: 0, clientX: 450, clientY: 50 })
    const ok = fakeDt()
    expect(drag('dragStart', head('c'), ok)).toBe(true)
    expect(ok.setData).toHaveBeenCalledTimes(1)
  })

  it('머리 밖의 끌기(트리 행 · 파일 따위)는 건드리지 않는다 — 막지도 끊지도 싣지도 않는다', async () => {
    await mount('a')
    const seen = vi.fn()
    window.addEventListener('dragstart', seen)
    const dt = fakeDt()
    const notCancelled = drag('dragStart', screen.getByTestId('tab-bar'), dt)
    window.removeEventListener('dragstart', seen)
    expect(notCancelled).toBe(true)
    expect(seen).toHaveBeenCalledTimes(1)
    expect(dt.setData).not.toHaveBeenCalled()
    expect(localDrag.getOutgoing()).toBeNull()
  })

  // ── 끄는 동안 출처 탭바 ──

  it('구역 안 — 끄는 머리는 커서를 바로 따르고(전환 없음 · 맨 위) 이웃은 확정한 순서로 200 ms EASE_OUT 비켜난다 · DOM · key 그대로', async () => {
    const bar = await mount('b')
    const row = screen.getByTestId('tab-strip-row')
    const nodes = screen.getAllByTestId('tab')
    await startA(bar)

    // 구역에 든 첫 판정 = 문턱 없이 확정(Chrome `justAttached`).
    strip(1, 120, 300)
    expect(xOf(head('a'))).toBe(120)
    expect(head('a').style.transition).toBe('none')
    expect(head('a').style.zIndex).toBe('1')
    expect([xOf(head('b')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 320, 480])
    expect(head('b').style.transition).toBe(EASE)
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    expect(screen.getByTestId('tab-strip-row')).toBe(row)
    expect(localDrag.readStripGeometry()).toMatchObject({ left: 100, tabs: ['b', 'a', 'c', 'd'], available: 660 })
    expect(localDrag.readStripGeometry()!.zoneViewport).toEqual({ x0: 100, y0: 25, x1: 760, y1: 83 })

    // 문턱(탭 폭 160 → 16 px) 안의 되돌림은 자리를 바꾸지 않는다 — 끄는 머리만 따른다.
    strip(0, 110, 290)
    expect(xOf(head('a'))).toBe(110)
    expect(xOf(head('b'))).toBe(0)
    // 문턱을 넘으면 그 판정의 자리로.
    strip(0, 60, 240)
    expect([xOf(head('a')), xOf(head('b'))]).toEqual([60, 160])
    expect(invokes('tab_drop')).toEqual([])
  })

  it('구역 밖 = 남은 탭(누르기 전 활성)을 칠하고 머리는 모델 자리로 200 ms 돌아감 · 돌아오면 끄는 탭', async () => {
    const bar = await mount('b')
    await startA(bar)
    strip(1, 120, 300)
    leaveZone()
    expect(head('b').getAttribute('data-active')).toBe('true')
    expect(head('a').getAttribute('data-active')).toBe('false')
    expect([0, 160, 320, 480]).toEqual(['a', 'b', 'c', 'd'].map(id => xOf(head(id))))
    expect(head('a').style.transition).toBe(EASE)

    strip(0, 30, 210)
    expect(head('a').getAttribute('data-active')).toBe('true')
    expect(xOf(head('a'))).toBe(30)
    expect(invokes('tab_drop')).toEqual([])
  })

  it('끄는 머리는 레이아웃이 숨겨도 그린다 — 출처 노드를 떼면 dragend 가 오지 않는다', async () => {
    rowWidth = 140 // 쓸 수 있는 폭 100 — 활성 b 면 a · b 만 보인다
    const bar = await mount('b', ABCD)
    fireEvent.pointerDown(head('b'), { button: 0, clientX: 140, clientY: 50 })
    bar.rerender({ tabs: [ABCD[1], ABCD[0], ABCD[2], ABCD[3]], active: 'b' })
    const b = head('b')
    drag('dragStart', b, fakeDt(), 145)
    await nextTask()
    // 끄는 동안 LLM 이 탭을 하나 앞에 끼워 b 가 보이는 끝 너머로 밀렸다.
    bar.rerender({ tabs: [{ id: 'z', name: 'Z', named: true }, ABCD[0], ABCD[2], ABCD[1], ABCD[3]], active: 'b' })
    leaveZone()
    expect(head('b')).toBe(b)
  })

  // ── dragend ──

  it.each([
    ['none', 'none'],
    ['move', 'move'],
    ['copy', 'none'],
  ])('dragend(dropEffect=%s) → tab_drop{NewWindow{by: DragEnd{effect: %s}}} 하나', async (dropEffect, effect) => {
    const bar = await mount('b')
    const dt = await startA(bar)
    drag('dragEnd', head('a'), fakeDt(dropEffect))
    expect(drops()).toEqual([
      {
        drag: valueOf(dt).drag,
        view: 'a',
        from: 'main',
        initial: 'b',
        to: { type: 'new_window', by: { type: 'drag_end', effect }, size: { w: 1000, h: 700 }, grab: { x: 180, y: 50 } },
      },
    ])
    // 같은 끌기의 dragend 두 번 = 한 번.
    drag('dragEnd', head('a'), fakeDt(dropEffect))
    expect(drops()).toHaveLength(1)
  })

  it('놓은 뒤 — 끄는 머리는 확정한 자리로 미끄러지고, 모델 순서가 오는 그 그림에 모델 자리 = 같은 자리 · 같은 노드', async () => {
    const bar = await mount('b')
    const nodes = screen.getAllByTestId('tab')
    const dt = await startA(bar)
    strip(1, 120, 300)
    drag('dragEnd', head('a'), fakeDt('move'))
    expect(xOf(head('a'))).toBe(160)
    expect(head('a').style.transition).toBe(EASE)

    bar.rerender({ tabs: BACD })
    expect([xOf(head('b')), xOf(head('a')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 160, 320, 480])
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'applied' })
    expect(localDrag.getOutgoing()).toBeNull()
    expect(head('a').style.zIndex).toBe('1')
  })

  it('같은 웹뷰 제자리 놓기(뿌리 drop 이 적음) → 아무것도 안 보냄 · 곧바로 풀려 모델 자리 · 되돌리지 않음', async () => {
    const bar = await mount('b')
    const dt = await startA(bar)
    strip(1, 120, 300)
    localDrag.markLocalDrop(valueOf(dt).drag)
    drag('dragEnd', head('a'), fakeDt('move'))
    expect(invokes('tab_drop')).toEqual([])
    expect(['a', 'b', 'c', 'd'].map(id => xOf(head(id)))).toEqual([0, 160, 320, 480])
    expect(head('a').getAttribute('data-active')).toBe('true')
    expect(localDrag.getOutgoing()).toBeNull()
    expect(localDrag.takeLocalDrop(valueOf(dt).drag)).toBe(false)
    await act(async () => {
      await ticks()
    })
    expect(invokes('switch_tab_if')).toEqual([])
  })

  it('dragstart 와 같은 task 의 dragend(엔진이 접은 끌기) → 아무것도 안 보냄 · 풀림 · 되돌리지 않음', async () => {
    const bar = await mount('b')
    fireEvent.pointerDown(head('a'), { button: 0, clientX: 180, clientY: 50 })
    bar.rerender({ active: 'a' })
    const dt = fakeDt()
    drag('dragStart', head('a'), dt)
    drag('dragEnd', head('a'), fakeDt('none'))
    await nextTask()
    expect(invokes('tab_drop')).toEqual([])
    expect(localDrag.getOutgoing()).toBeNull()
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'canceled' })
    expect(invokes('switch_tab_if')).toEqual([])
  })

  it('tab_drop invoke 오류 → 바꿔 그리기를 곧바로 푼다', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'window_frame') return FRAME
      if (cmd === 'tab_drop') throw new Error('ipc down')
      return undefined
    })
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const bar = await mount('b')
    await startA(bar)
    leaveZone()
    drag('dragEnd', head('a'), fakeDt('none'))
    expect(head('b').getAttribute('data-active')).toBe('true')
    await act(async () => {
      await ticks()
    })
    expect(head('a').getAttribute('data-active')).toBe('true')
    expect(localDrag.getOutgoing()).toBeNull()
  })

  // ── 결과 알림 · 원래 선택 되돌리기(§4-6) ──

  it.each(['canceled', 'ignored'] as const)(
    'done{%s}(자기 dragend none) → switch_tab_if(initial, expect = 끄는 탭) 한 번 · 되돌리는 동안 initial 을 칠한다',
    async outcome => {
      const bar = await mount('b')
      const dt = await startA(bar)
      drag('dragEnd', head('a'), fakeDt('none'))
      emit('tab_drop:done', { drag: valueOf(dt).drag, outcome })
      emit('tab_drop:done', { drag: valueOf(dt).drag, outcome })
      expect(invokes('switch_tab_if')).toEqual([{ window: 'main', view: 'b', expect: 'a' }])
      expect(head('b').getAttribute('data-active')).toBe('true')
      // 되돌리기가 닿음 — 그 알림으로 푼다.
      bar.rerender({ active: 'b' })
      expect(head('b').getAttribute('data-active')).toBe('true')
      expect(localDrag.getOutgoing()).toBeNull()
    },
  )

  it('되돌리지 않는 갈래 — 그사이 활성이 바뀜 · initial = 끄는 탭 · Rejected · 자기 dragend 가 move · 남의 끌기', async () => {
    const cases: Array<{ name: string; run: (bar: ReturnType<typeof renderBar>) => Promise<void> }> = [
      {
        name: '활성 바뀜',
        run: async bar => {
          const dt = await startA(bar)
          drag('dragEnd', head('a'), fakeDt('none'))
          bar.rerender({ active: 'c' })
          emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'canceled' })
        },
      },
      {
        name: 'initial = view',
        run: async bar => {
          bar.rerender({ active: 'a' })
          fireEvent.pointerDown(head('a'), { button: 0, clientX: 180, clientY: 50 })
          const dt = fakeDt()
          drag('dragStart', head('a'), dt)
          await nextTask()
          drag('dragEnd', head('a'), fakeDt('none'))
          emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'ignored' })
        },
      },
      {
        name: 'Rejected',
        run: async bar => {
          const dt = await startA(bar)
          drag('dragEnd', head('a'), fakeDt('none'))
          emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'rejected' })
        },
      },
      {
        name: 'move + Ignored',
        run: async bar => {
          const dt = await startA(bar)
          drag('dragEnd', head('a'), fakeDt('move'))
          emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'ignored' })
          emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'canceled' })
        },
      },
      {
        name: '남의 끌기',
        run: async bar => {
          await startA(bar)
          drag('dragEnd', head('a'), fakeDt('none'))
          emit('tab_drop:done', { drag: 'someone-else', outcome: 'canceled' })
        },
      },
    ]
    for (const c of cases) {
      cleanup()
      __resetDragSourceForTest()
      invokeMock.mockClear()
      const bar = await mount('b')
      await c.run(bar)
      await act(async () => {
        await ticks()
      })
      expect(invokes('switch_tab_if'), c.name).toEqual([])
    }
  })

  it('되돌리기 오류는 잡아 로그만 남기고 모델 활성으로 물러난다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'window_frame') return FRAME
      if (cmd === 'tab_drop') return { type: 'canceled' }
      if (cmd === 'switch_tab_if') throw new Error('TabNotFound')
      return undefined
    })
    const bar = await mount('b')
    const dt = await startA(bar)
    drag('dragEnd', head('a'), fakeDt('none'))
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'canceled' })
    expect(head('b').getAttribute('data-active')).toBe('true')
    await act(async () => {
      await ticks()
    })
    expect(warn.mock.calls.some(c => String(c[0]).includes('switch_tab_if'))).toBe(true)
    expect(head('a').getAttribute('data-active')).toBe('true')
  })

  it('되돌리기가 닿기 전에 끄는 탭을 누르면 모델 활성이 그 탭이어도 switch_tab 을 보낸다 — 셸이 차례대로 적용하면 끝 활성 = 누른 탭', async () => {
    // 셸 — 받은 명령을 차례대로 적용한다(되돌리기는 `expect` 일 때만 바꾼다). 적용은 손으로 민다.
    let shellActive = 'a'
    const queue: Array<() => void> = []
    invokeMock.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'window_frame') return FRAME
      if (cmd === 'tab_drop') return { type: 'canceled' }
      if (cmd === 'switch_tab_if') {
        const { view, expect: want } = args as { view: string; expect: string }
        return new Promise<boolean>(resolve =>
          queue.push(() => {
            const changed = shellActive === want
            if (changed) shellActive = view
            resolve(changed)
          }),
        )
      }
      return undefined
    })
    const bar = await mount('b')
    const dt = await startA(bar)
    drag('dragEnd', head('a'), fakeDt('none'))
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'canceled' })
    expect(invokes('switch_tab_if')).toEqual([{ window: 'main', view: 'b', expect: 'a' }])
    expect(head('b').getAttribute('data-active')).toBe('true')

    const onSwitch = vi.mocked(bar.onSwitch)
    onSwitch.mockClear()
    onSwitch.mockImplementation(view => void queue.push(() => (shellActive = view)))
    // 캔버스 누름이 바꿔 그리기를 먼저 풀어도 되돌리기는 아직 셸에 있다.
    fireEvent.pointerDown(document.body, { button: 0 })
    expect(head('a').getAttribute('data-active')).toBe('true')
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    expect(onSwitch.mock.calls).toEqual([['a']])

    for (const apply of queue.splice(0)) {
      await act(async () => {
        apply()
        await ticks()
      })
      bar.rerender({ active: shellActive })
    }
    expect(shellActive).toBe('a')
    expect(head('a').getAttribute('data-active')).toBe('true')

    // 되돌리기가 닿은 뒤의 활성 탭 누름은 보통 누름이다.
    onSwitch.mockClear()
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('끄는 중 그 탭이 그 창에서 빠지면(LLM 닫기) 풀린다 — 남은 머리를 모델대로', async () => {
    const bar = await mount('b')
    await startA(bar)
    leaveZone()
    bar.rerender({ tabs: [ABCD[1], ABCD[2], ABCD[3]], active: 'b' })
    expect(headIds()).toEqual(['b', 'c', 'd'])
    expect([xOf(head('b')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 160, 320])
    expect(localDrag.getOutgoing()).toBeNull()
  })

  // ── 다른 창 끌기의 세로선(뿌리 dragover 가 적음) ──

  it('세로선 = 2 px · 그 x 를 가운데로(양쪽 머리 끝에 1 px 씩) · 없으면 그리지 않는다', async () => {
    await mount('a')
    expect(screen.queryByTestId('tab-drop-marker')).toBeNull()
    act(() => localDrag.setTargetMarker(160))
    const line = screen.getByTestId('tab-drop-marker')
    expect([xOf(line), line.style.width]).toEqual([159, '2px'])
    act(() => localDrag.setTargetMarker(null))
    expect(screen.queryByTestId('tab-drop-marker')).toBeNull()
  })

  // ── 머리 DOM 순서 = 탭 키 · 화면 낭독 순서 ──

  /** a 를 b 뒤로 옮겨 놓고(모델 메아리 b · a · c · d) 그 끌기가 풀리기까지 — 놓은 뒤 자리 전환이 진행 중이다. */
  async function reorderAThenRelease(bar: ReturnType<typeof renderBar>): Promise<() => Promise<void>> {
    const dt = await startA(bar)
    strip(1, 120, 300)
    const settle = holdTransition()
    drag('dragEnd', head('a'), fakeDt('move'))
    bar.rerender({ tabs: BACD })
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'applied' })
    return settle
  }

  it('끌기가 풀리고 자리 전환까지 끝나면 머리 DOM 순서를 보이는 순서로 맞춘다 — 같은 노드 · 같은 자리 · 줄 그대로', async () => {
    const bar = await mount('b')
    const row = screen.getByTestId('tab-strip-row')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    const settle = await reorderAThenRelease(bar)
    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])

    await settle()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
    expect([xOf(b), xOf(a), xOf(c), xOf(d)]).toEqual([0, 160, 320, 480])
    expect(screen.getByTestId('tab-strip-row')).toBe(row)
  })

  it('놓았지만 끌기가 풀리기 전(done 을 기다림)은 전환이 끝나도 맞추지 않는다', async () => {
    const bar = await mount('b')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    const dt = await startA(bar)
    strip(1, 120, 300)
    drag('dragEnd', head('a'), fakeDt('move'))
    bar.rerender({ tabs: BACD })
    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'applied' })
    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
  })

  it('맞추기 전에 온 누름은 그동안 전환이 끝나도 맞추기를 막고, 떼면 그 뗌 다음 사건 뒤에 맞춘다', async () => {
    const bar = await mount('b')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    const settle = await reorderAThenRelease(bar)

    fireEvent.pointerDown(c, { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    await settle()
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])

    pointer('pointerup', 450)
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])
    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
  })

  it('끝나고 채움으로 남은 애니메이션은 기다리지 않는다', async () => {
    const bar = await mount('b')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    await reorderAThenRelease(bar)
    running = [{ playState: 'finished', pending: false, finished: Promise.resolve() }]
    let polls = 0
    // 기다림이 제자리를 돌면 다섯 번째 뒤로는 빈 목록을 줘 시험이 멈추지 않고 실패하게 한다.
    animatable.getAnimations = () => (++polls > 5 ? [] : running)

    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
    expect(polls).toBe(1)
  })

  it('쉴 때 다른 경로가 바꾼 순서는 곧바로 DOM 순서가 되고, 이름 편집 중에는 편집이 끝날 때까지 미룬다', async () => {
    const bar = await mount('a')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    bar.rerender({ tabs: BACD })
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])

    fireEvent.doubleClick(nameOf('c'))
    bar.rerender({ tabs: [ABCD[3], ABCD[1], ABCD[0], ABCD[2]] })
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])

    fireEvent.keyDown(screen.getByTestId('tab-rename-input'), { key: 'Escape' })
    expect(screen.getAllByTestId('tab')).toEqual([d, b, a, c])
  })

  // ── 이름 편집 · 좁은 탭바 ──
  it('좁은 탭바 — 비활성 머리 더블클릭의 둘째 클릭이 막 생긴 닫기 버튼에 떨어져도 닫지 않고 편집을 연다 · 그 뒤 따로 누른 닫기는 닫는다', () => {
    rowWidth = 140 // 쓸 수 있는 폭 100 → a 52 · b 32(닫기 버튼 없음) · 보이는 것 = 앞 둘 · b 는 화면 x 152..184
    const bar = renderBar('a', ABCD)
    expect(head('b').querySelector('[data-testid="tab-close"]')).toBeNull()
    fireEvent.pointerDown(nameOf('b'), { button: 0, pointerId: 1, clientX: 160, clientY: 50 })
    expect(bar.onSwitch).toHaveBeenCalledWith('b')
    bar.rerender({ active: 'b' })
    pointer('pointerup', 160)
    fireEvent.click(head('b'), { detail: 1 })

    const close = head('b').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.pointerDown(close, { button: 0, pointerId: 1, clientX: 160, clientY: 50 })
    fireEvent.click(close, { detail: 2 })
    fireEvent.doubleClick(close, { detail: 2 })
    expect(bar.onClose).not.toHaveBeenCalled()
    const input = screen.getByTestId('tab-rename-input')
    expect(input.getAttribute('data-view-id')).toBe('b')

    fireEvent.keyDown(input, { key: 'Escape' })
    fireEvent.click(head('b').querySelector<HTMLElement>('[data-testid="tab-close"]')!, { detail: 1 })
    expect(bar.onClose).toHaveBeenCalledWith('b')
  })

  it('좁은 탭바 — 이름 칸 폭이 0 인 활성 머리도 머리 더블클릭으로 편집을 열고, 편집 중 머리는 표준 폭 · 맨 위 · 줄 왼쪽 끝까지 당겨져 쓸 수 있는 폭 너머(+ 자리)까지 덮는다', () => {
    rowWidth = 140 // 활성 b → a 32(x 0) · b 52(x 32) · + x 84 · 쓸 수 있는 폭 100 — 편집 중 b 는 0..160
    renderBar('b', ABCD)
    expect(xOf(head('b'))).toBe(32)
    fireEvent.doubleClick(head('b'))
    expect(screen.getByTestId('tab-rename-input').getAttribute('data-view-id')).toBe('b')
    expect(head('b').style.width).toBe('160px')
    expect(head('b').style.zIndex).toBe('2')
    expect(xOf(head('b'))).toBe(0)

    fireEvent.keyDown(screen.getByTestId('tab-rename-input'), { key: 'Escape' })
    expect(head('b').style.width).toBe(`${TAB_MIN_ACTIVE_W}px`)
    expect(xOf(head('b'))).toBe(32)
  })

  it('편집 중 머리는 오른쪽에 자리가 있으면 제자리에서 넓어진다', () => {
    rowWidth = 500 // 쓸 수 있는 폭 460 → 머리 115 · x 0 · 115 · 230 · 345
    renderBar('a', ABCD)
    fireEvent.doubleClick(nameOf('c'))
    expect([xOf(head('c')), head('c').style.width]).toEqual([230, '160px'])
  })

  it('넓힌 편집 머리가 덮은 이웃 자리의 누름은 편집을 확정하고 그 이웃으로 전환만 한다 · 넓히기 전 자리 · 입력 칸의 누름은 그대로다', () => {
    rowWidth = 500 // 머리 115 · x 0 · 115 · 230 · 345 → 편집 중 c 230..390 이 d(345..460)의 왼쪽을 덮는다
    const bar = renderBar('c', ABCD)
    fireEvent.doubleClick(nameOf('c'))
    const input = screen.getByTestId('tab-rename-input')
    fireEvent.change(input, { target: { value: 'Renamed' } })

    fireEvent.pointerDown(input, { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    fireEvent.pointerDown(head('c'), { button: 0, pointerId: 1, clientX: 400, clientY: 50 })
    expect(bar.onSwitch).not.toHaveBeenCalled()
    expect(screen.getByTestId('tab-rename-input')).toBe(input)

    fireEvent.pointerDown(head('c'), { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    expect(bar.onRename).toHaveBeenCalledWith('c', 'Renamed')
    expect(bar.onSwitch).toHaveBeenCalledWith('d')
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('자동 이름 탭 — 글자를 안 바꾸고 덮인 이웃 자리를 누르면 onRename 안 부르고 그 이웃으로 전환만 한다', () => {
    rowWidth = 500 // 위 시험과 같은 자리 — 편집 중 c 230..390 이 d(345..460)의 왼쪽을 덮는다
    const bar = renderBar('c', ABCD.map(tab => ({ ...tab, named: false })))
    fireEvent.doubleClick(nameOf('c'))
    fireEvent.pointerDown(head('c'), { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    expect(bar.onRename).not.toHaveBeenCalled()
    expect(bar.onSwitch).toHaveBeenCalledWith('d')
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('좁은 탭바 — 왼쪽으로 당긴 편집 머리가 덮은 이웃 자리의 누름도 그 이웃으로 전환하고, 밑에 머리가 없는 + 자리의 누름은 지금처럼 둔다', () => {
    rowWidth = 140 // 활성 b → a 32(x 0) · b 52(x 32) · + x 84 — 편집 중 b 는 0..160
    const bar = renderBar('b', ABCD)
    fireEvent.doubleClick(head('b'))
    fireEvent.pointerDown(head('b'), { button: 0, pointerId: 1, clientX: 190, clientY: 50 })
    expect(screen.getByTestId('tab-rename-input').getAttribute('data-view-id')).toBe('b')
    expect(bar.onCreate).not.toHaveBeenCalled()

    fireEvent.pointerDown(head('b'), { button: 0, pointerId: 1, clientX: 110, clientY: 50 })
    expect(bar.onSwitch).toHaveBeenCalledWith('a')
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('이름 편집 중인 탭의 × 는 누름에 입력 칸 포커스를 뺏지 않고 첫 클릭에 닫는다 · 그 탭이 모델에서 빠지면 편집도 끝나 머리 DOM 순서를 다시 맞춘다', () => {
    const bar = renderBar('a', ABCD)
    const [a, b, , d] = screen.getAllByTestId('tab')
    fireEvent.doubleClick(nameOf('c'))
    const input = screen.getByTestId('tab-rename-input')
    const close = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-close"]')!

    expect(fireEvent.mouseDown(close('b'))).toBe(true)
    expect(fireEvent.mouseDown(close('c'))).toBe(false)
    expect(document.activeElement).toBe(input)
    fireEvent.click(close('c'), { detail: 1 })
    expect(bar.onClose).toHaveBeenCalledWith('c')
    expect(bar.onRename).not.toHaveBeenCalled()

    bar.rerender({ tabs: [ABCD[0], ABCD[1], ABCD[3]] })
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
    bar.rerender({ tabs: [ABCD[3], ABCD[0], ABCD[1]] })
    expect(screen.getAllByTestId('tab')).toEqual([d, a, b])
  })
})
