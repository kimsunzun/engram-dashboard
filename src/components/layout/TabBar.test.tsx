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
})

// ── 탭 끌기 그리기 ──

interface Pending<T> {
  resolve: (v: T) => void
}
let begins: Pending<BeginReply>[]
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
    rowWidth = 700
    captured.clear()
    handlers.clear()
    invokeMock.mockReset()
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'tab_drag_begin') return new Promise(resolve => begins.push({ resolve }))
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
