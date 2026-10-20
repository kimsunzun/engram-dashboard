// TabBar 단위테스트 — 탭 렌더 · 누름 전환 · 닫기 · 추가 · 이름 인라인 편집(ADR-0057, §7-2) + 탭 끌기 그리기(TRD
//   S21-slot-docking §2-2 · §5-1 — 훅 `useTabDrag` 를 실물로 · invoke · 사건은 가짜).
// 탭바(끌기 쪽 기대값의 손셈): 줄 x 100..800 · y 40..68 · `+` 40 → 쓸 수 있는 폭 660 · 탭 넷이면 모두 160 → 머리 x 0 · 160 ·
//   320 · 480. 탭 a(머리 x 100..260)를 x 180(잡은 오프셋 80)에서 누른다.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args) }))
const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    handlers.set(event, handler)
    return () => handlers.delete(event)
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))

import TabBar from './TabBar'
import type { ViewMeta } from '../../api/layoutTypes'
import type { BeginReply } from '../../../src-tauri/bindings/BeginReply'
import type { LeaveReply } from '../../../src-tauri/bindings/LeaveReply'
import { watchDockSession } from './dock/dockSession'
import { TAB_MIN_ACTIVE_W } from './tabStrip/layout'

afterEach(cleanup)

const TABS: ViewMeta[] = [
  { id: 'v1', name: 'View 1' },
  { id: 'v2', name: 'View 2' },
]

type Props = Parameters<typeof TabBar>[0]

function renderBar(active: string | null = 'v1', tabs: ViewMeta[] = TABS) {
  const props: Props = {
    label: 'main',
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

function openEditForV1() {
  fireEvent.doubleClick(nameOf('v1'))
  return screen.getByTestId('tab-rename-input') as HTMLInputElement
}

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
    // ★멱등★: Enter 가 setEditingId(null) 로 input 언마운트 → blur 가 commitEdit 을 한 번 더 부르지만
    //   editingId 는 이미 null 이라 no-op. onRename 은 정확히 1회여야 한다(중복 rename_tab invoke 방지).
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

  it('머리는 HTML5 끌기 출처가 아니다 — draggable 없음 · div', () => {
    renderBar('v1')
    for (const el of screen.getAllByTestId('tab')) {
      expect(el.tagName).toBe('DIV')
      expect(el.getAttribute('draggable')).toBeNull()
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
    const tabs = ['a', 'b', 'c', 'd', 'e', 'f'].map(id => ({ id, name: id }))
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

  // PRD R27 · ADR-0295 결정 1 — 위 모서리 10 · 아래 날개 12 · 구분선 2×16 · 좌우 여백 2.
  it('활성 탭 모양 = 위 모서리 반지름 10 · 아래 바깥 날개 반지름 12 · 구분선 두께 2 · 높이 16 · 좌우 여백 2', () => {
    const tabs = ['a', 'b', 'c'].map(id => ({ id, name: id }))
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

    // 여백 · 두께 · 여백(2 + 2 + 2) 칸이 머리 경계 가운데에 걸린다.
    const slot = part('c', 'tab-separator')
    expect([slot.style.width, slot.style.left, slot.style.paddingLeft, slot.style.paddingRight]).toEqual([
      '6px',
      '-3px',
      '2px',
      '2px',
    ])
    expect(slot.style.boxSizing).toBe('border-box')
    expect(slot.style.alignItems).toBe('center')
    const line = (slot.firstElementChild as HTMLElement).style
    expect([line.width, line.height, line.borderRadius]).toEqual(['2px', '16px', '1px'])
  })

  // Chrome `horizontal_tab_style_views.cc` `GetSeparatorOpacity` · `GetHoverInterpolatedSeparatorOpacity`.
  it('구분선은 곧바로 사라지고 · 맴돎이 끝난 머리 양옆만 200 ms EASE_IN 으로 나타나며 · 그 뒤와 활성이 옮은 되살아남은 곧바로다', () => {
    vi.useFakeTimers()
    try {
      const tabs = ['a', 'b', 'c', 'd'].map(id => ({ id, name: id }))
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
})

// ── 탭 끌기 그리기 ──

interface Pending<T> {
  resolve: (v: T) => void
}
let begins: Pending<BeginReply>[]
let leaves: Pending<LeaveReply>[]
let stopWatch: () => void
let rowWidth: number
const captured = new Set<number>()

const ZONE = { x0: 100, y0: 25, x1: 760, y1: 83 }
const EASE = 'transform 200ms cubic-bezier(0.3333, 0.6667, 0.6667, 1)'
// 끌기 밖 — 속성은 그대로 · 시간만 0(진행 중인 전환을 끊지 않는다).
const SNAP = 'transform 0ms cubic-bezier(0.3333, 0.6667, 0.6667, 1)'

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}
const calls = (cmd: string) => invokeMock.mock.calls.filter(c => c[0] === cmd).map(c => (c[1] as { args: unknown }).args)

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

const ABCD: ViewMeta[] = ['a', 'b', 'c', 'd'].map(id => ({ id, name: id.toUpperCase() }))

/** a 를 누르고 문턱을 넘긴 뒤 begin 답 `strip{7}` 까지. */
async function dragA(bar: ReturnType<typeof renderBar>): Promise<void> {
  fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
  bar.rerender({ active: 'a' })
  pointer('pointermove', 191)
  await act(async () => {
    begins[0].resolve({ type: 'strip', session: 7, zone: ZONE })
    await ticks()
  })
}

describe('TabBar — 탭 끌기 그리기', () => {
  beforeEach(async () => {
    begins = []
    leaves = []
    rowWidth = 700
    captured.clear()
    handlers.clear()
    invokeMock.mockReset()
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'tab_drag_begin') return new Promise(resolve => begins.push({ resolve }))
      if (cmd === 'tab_drag_leave') return new Promise(resolve => leaves.push({ resolve }))
      if (cmd === 'window_role') return { role: 'model', drag: null }
      return undefined
    })
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(fakeRect)
    HTMLElement.prototype.setPointerCapture = (id: number) => void captured.add(id)
    HTMLElement.prototype.releasePointerCapture = (id: number) => void captured.delete(id)
    HTMLElement.prototype.hasPointerCapture = (id: number) => captured.has(id)
    window.matchMedia = vi.fn(() => ({ matches: false }) as MediaQueryList)
    running = []
    animatable.getAnimations = () => running
    stopWatch = watchDockSession(() => {})
    await ticks()
  })
  afterEach(() => {
    cleanup()
    stopWatch()
    delete animatable.getAnimations
    vi.restoreAllMocks()
  })

  it('머리 폭 · 자리 = 탭바 레이아웃(쓸 수 있는 폭 = 줄 − +) · + 는 마지막 머리 뒤', () => {
    renderBar('a', ABCD)
    expect(['a', 'b', 'c', 'd'].map(id => [xOf(head(id)), head(id).style.width])).toEqual([
      [0, '160px'],
      [160, '160px'],
      [320, '160px'],
      [480, '160px'],
    ])
    expect(xOf(screen.getByTestId('tab-add'))).toBe(640)
  })

  it('좁은 탭바 — 보이는 머리만 그리고 활성 최소 폭보다 좁은 비활성 머리는 닫기 버튼을 감춘다', () => {
    rowWidth = 140 // 쓸 수 있는 폭 100 → 폭 42 · 32 · 32 · 32 · 보이는 것 = 앞 둘
    renderBar('a', ABCD)
    expect(headIds()).toEqual(['a', 'b'])
    expect(head('a').style.width).toBe(`${TAB_MIN_ACTIVE_W}px`)
    expect(head('a').querySelector('[data-testid="tab-close"]')).not.toBeNull()
    expect(head('b').style.width).toBe('32px')
    expect(head('b').querySelector('[data-testid="tab-close"]')).toBeNull()
  })

  it('누름 → 전환은 훅이 누르기 전 활성을 적은 뒤 — begin 의 initial_active 는 누르기 전 활성', async () => {
    const bar = renderBar('b', ABCD)
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    expect(bar.onSwitch).toHaveBeenCalledWith('a')
    bar.rerender({ active: 'a' })
    pointer('pointermove', 191)
    expect(calls('tab_drag_begin')).toEqual([
      expect.objectContaining({ view: 'a', initial_active: 'b', area: { x0: 100, y0: 40, x1: 760, y1: 68 } }),
    ])
  })

  it('끄는 동안 — 끄는 탭은 커서를 바로 따르고(전환 없음 · 맨 위) 이웃은 200 ms EASE_OUT 로 비켜난다 · 줄 · 머리 DOM 은 그대로', async () => {
    const bar = renderBar('a', ABCD)
    const row = screen.getByTestId('tab-strip-row')
    const nodes = screen.getAllByTestId('tab')
    await dragA(bar)
    pointer('pointermove', 300)

    expect(calls('tab_drag_reorder')).toEqual([{ session: 7, seq: 1, index: 1 }])
    expect(xOf(head('a'))).toBe(120)
    expect(head('a').style.transition).toBe('none')
    expect(head('a').style.zIndex).toBe('1')
    expect([xOf(head('b')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 320, 480])
    expect(head('b').style.transition).toBe(EASE)

    // 모델 메아리 — 보이는 순서가 바뀌어도 DOM 순서 · 노드 · 줄은 그대로다.
    bar.rerender({ tabs: [ABCD[1], ABCD[0], ABCD[2], ABCD[3]] })
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    expect(screen.getByTestId('tab-strip-row')).toBe(row)
    expect(captured.has(1)).toBe(true)
  })

  it('놓으면 끄는 탭이 자리로 미끄러지고, 끌기가 끝나도(drag = null) 같은 자리 · 같은 노드 · 전환 속성 그대로', async () => {
    const bar = renderBar('a', ABCD)
    const nodes = screen.getAllByTestId('tab')
    await dragA(bar)
    pointer('pointermove', 300)
    bar.rerender({ tabs: [ABCD[1], ABCD[0], ABCD[2], ABCD[3]] })
    pointer('pointerup', 300)

    expect(calls('tab_drag_end')).toEqual([{ session: 7, how: 'drop', src: 'pointer', at: 1 }])
    expect(xOf(head('a'))).toBe(160)
    expect(head('a').style.transition).toBe(EASE)

    holdTransition()
    emit('dock:drag', { session: 7, phase: 'ended' })
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    expect([xOf(head('b')), xOf(head('a')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 160, 320, 480])
    expect(head('a').style.transition).toBe(SNAP)
    expect(head('b').style.transition).toBe(SNAP)
    expect(head('a').style.zIndex).toBe('1')
  })

  // ── 머리 DOM 순서 = 탭 키 · 화면 낭독 순서 ──

  /** a 를 b 뒤로 옮겨 놓고(모델 메아리 b · a · c · d) 끌기가 끝나기까지 — 끄는 동안 · 놓은 뒤 자리 전환이 진행 중이다. */
  async function reorderAThenEnd(bar: ReturnType<typeof renderBar>): Promise<() => Promise<void>> {
    await dragA(bar)
    pointer('pointermove', 300)
    bar.rerender({ tabs: [ABCD[1], ABCD[0], ABCD[2], ABCD[3]] })
    const settle = holdTransition()
    pointer('pointerup', 300)
    emit('dock:drag', { session: 7, phase: 'ended' })
    return settle
  }

  it('끌기가 끝나고 자리 전환까지 끝나면 머리 DOM 순서를 보이는 순서로 맞춘다 — 같은 노드 · 같은 자리 · 줄 그대로', async () => {
    const bar = renderBar('a', ABCD)
    const row = screen.getByTestId('tab-strip-row')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    const settle = await reorderAThenEnd(bar)
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])

    await settle()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
    expect([xOf(b), xOf(a), xOf(c), xOf(d)]).toEqual([0, 160, 320, 480])
    expect(screen.getByTestId('tab-strip-row')).toBe(row)
  })

  it('맞추기 전에 온 누름은 그동안 전환이 끝나도 맞추기를 막고, 떼면 그 뗌 다음 사건 뒤에 맞춘다', async () => {
    const bar = renderBar('a', ABCD)
    const [a, b, c, d] = screen.getAllByTestId('tab')
    const settle = await reorderAThenEnd(bar)

    fireEvent.pointerDown(c, { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    await settle()
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])

    pointer('pointerup', 450)
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])
    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
  })

  it('begin 답을 기다리는 동안의 뗌은 맞추지 않는다(캡처 그대로) — 그 끌기가 끝나고 전환이 끝난 뒤에 맞춘다', async () => {
    const bar = renderBar('a', ABCD)
    const [a, b, c, d] = screen.getAllByTestId('tab')
    await reorderAThenEnd(bar)

    fireEvent.pointerDown(c, { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    bar.rerender({ active: 'c' })
    pointer('pointermove', 470)
    pointer('pointerup', 470)
    running = []
    await flush()
    expect(captured.has(1)).toBe(true)
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])

    await act(async () => {
      begins[1].resolve({ type: 'strip', session: 8, zone: ZONE })
      await ticks()
    })
    const settle = holdTransition()
    emit('dock:drag', { session: 8, phase: 'ended' })
    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([a, b, c, d])

    await settle()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
  })

  it('끝나고 채움으로 남은 애니메이션은 기다리지 않는다', async () => {
    const bar = renderBar('a', ABCD)
    const [a, b, c, d] = screen.getAllByTestId('tab')
    await reorderAThenEnd(bar)
    running = [{ playState: 'finished', pending: false, finished: Promise.resolve() }]
    let polls = 0
    // 기다림이 제자리를 돌면 다섯 번째 뒤로는 빈 목록을 줘 시험이 멈추지 않고 실패하게 한다.
    animatable.getAnimations = () => (++polls > 5 ? [] : running)

    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
    expect(polls).toBe(1)
  })

  it('쉴 때 다른 경로가 바꾼 순서는 곧바로 DOM 순서가 되고, 이름 편집 중에는 편집이 끝날 때까지 미룬다', () => {
    const bar = renderBar('a', ABCD)
    const [a, b, c, d] = screen.getAllByTestId('tab')
    bar.rerender({ tabs: [ABCD[1], ABCD[0], ABCD[2], ABCD[3]] })
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])

    fireEvent.doubleClick(nameOf('c'))
    bar.rerender({ tabs: [ABCD[3], ABCD[1], ABCD[0], ABCD[2]] })
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])

    fireEvent.keyDown(screen.getByTestId('tab-rename-input'), { key: 'Escape' })
    expect(screen.getAllByTestId('tab')).toEqual([d, b, a, c])
  })

  it('막 붙은 창 — 모델 메아리 전의 끄는 탭은 이름 없는 머리로 자리 · 폭만 그리고, 메아리가 오면 같은 노드가 된다', () => {
    const bar = renderBar('a', ABCD)
    emit('dock:feed', {
      session: 9,
      view: 'n',
      step: { type: 'attach', zone: ZONE, grab: { x_ratio: 0.25, y: 10, single_x: 40 }, x: 420, y: 50 },
    })
    const n = head('n')
    expect(n.style.width).toBe('132px') // 탭 다섯 · 660 → 132
    expect(n.getAttribute('data-active')).toBe('true')
    expect(nameOf('n').textContent).toBe('')

    bar.rerender({ tabs: [ABCD[0], ABCD[1], { id: 'n', name: 'New' }, ABCD[2], ABCD[3]] })
    expect(head('n')).toBe(n)
    expect(nameOf('n').textContent).toBe('New')
  })

  it('떠나기를 청한 뒤 셸이 탭을 빼면(모델 메아리) 답 전에도 그 머리를 지우고 남은 머리를 모델대로 그린다 · not_ready 면 다시 그린다', async () => {
    const bar = renderBar('a', ABCD)
    const row = screen.getByTestId('tab-strip-row')
    await dragA(bar)
    pointer('pointermove', 300, 120)
    expect(calls('tab_drag_leave')).toEqual([{ session: 7 }])

    bar.rerender({ tabs: [ABCD[1], ABCD[2], ABCD[3]], active: 'b' })
    expect(headIds()).toEqual(['b', 'c', 'd'])
    expect(head('b').getAttribute('data-active')).toBe('true')
    expect([xOf(head('b')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 160, 320])
    expect(screen.getByTestId('tab-strip-row')).toBe(row)

    await act(async () => {
      leaves[0].resolve('not_ready')
      await ticks()
    })
    expect(head('a').getAttribute('data-active')).toBe('true')
    expect(captured.has(1)).toBe(true)
  })

  it('좁은 탭바 — 비활성 머리 더블클릭의 둘째 클릭이 막 생긴 닫기 버튼에 떨어져도 닫지 않고 편집을 연다 · 그 뒤 따로 누른 닫기는 닫는다', () => {
    rowWidth = 140 // 쓸 수 있는 폭 100 → a 42 · b 32(닫기 버튼 없음) · 보이는 것 = 앞 둘
    const bar = renderBar('a', ABCD)
    expect(head('b').querySelector('[data-testid="tab-close"]')).toBeNull()
    fireEvent.pointerDown(nameOf('b'), { button: 0, pointerId: 1, clientX: 150, clientY: 50 })
    expect(bar.onSwitch).toHaveBeenCalledWith('b')
    bar.rerender({ active: 'b' })
    pointer('pointerup', 150)
    fireEvent.click(head('b'), { detail: 1 })

    const close = head('b').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.pointerDown(close, { button: 0, pointerId: 1, clientX: 150, clientY: 50 })
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
    rowWidth = 140 // 활성 b → a 32(x 0) · b 42(x 32) · + x 74 · 쓸 수 있는 폭 100 — 편집 중 b 는 0..160
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
    pointer('pointermove', 480)
    expect(calls('tab_drag_begin')).toEqual([])
  })

  it('좁은 탭바 — 왼쪽으로 당긴 편집 머리가 덮은 이웃 자리의 누름도 그 이웃으로 전환하고, 밑에 머리가 없는 + 자리의 누름은 지금처럼 둔다', () => {
    rowWidth = 140 // 활성 b → a 32(x 0) · b 42(x 32) · + x 74 — 편집 중 b 는 0..160
    const bar = renderBar('b', ABCD)
    fireEvent.doubleClick(head('b'))
    fireEvent.pointerDown(head('b'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
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

  it('이름 편집 중인 머리 · 닫기 버튼에서 시작한 누름은 끌기가 아니다', () => {
    renderBar('a', ABCD)
    fireEvent.doubleClick(nameOf('a'))
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    pointer('pointermove', 220)
    pointer('pointerup', 220)
    const close = head('b').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.pointerDown(close, { button: 0, pointerId: 1, clientX: 400, clientY: 50 })
    pointer('pointermove', 440)
    expect(calls('tab_drag_begin')).toEqual([])
  })
})
