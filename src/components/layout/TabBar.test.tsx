// TabBar unit test — tab render · press to switch · close · add · inline name edit (ADR-0057, §7-2) + the tab drag source (TRD S21-slot-docking §4-2 · §4-4 · §4-6; `useTabDragSource` · `dragSource` real, invoke · events fake).
// Drag expectations by hand: row x 100..800 · y 40..68 · `+` 40 → usable width 660; four tabs at 160 → head x 0 · 160 · 320 · 480. Press tab a (head x 100..260) at x 180 (grab offset 80).

import { act, cleanup, createEvent, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// The tab bar emits the window fact pull (`window_frame`) on every mount — answer in the contract shape so it raises no warning.
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
  it('renders tabs and highlights the active tab with data-active=true', () => {
    renderBar('v2')
    const tabs = screen.getAllByTestId('tab')
    expect(tabs).toHaveLength(2)
    expect(head('v2').getAttribute('data-active')).toBe('true')
    expect(head('v1').getAttribute('data-active')).toBe('false')
  })

  it('the moment an inactive tab head is pressed with the left button, onSwitch(that view) — it does not wait for the click', () => {
    const { onSwitch } = renderBar('v1')
    fireEvent.pointerDown(nameOf('v2'), { button: 0, pointerId: 1 })
    expect(onSwitch).toHaveBeenCalledWith('v2')
    fireEvent.click(head('v2'))
    expect(onSwitch).toHaveBeenCalledTimes(1)
  })

  it('pressing the active tab · pressing with the right button does not switch', () => {
    const { onSwitch } = renderBar('v1')
    fireEvent.pointerDown(head('v1'), { button: 0, pointerId: 1 })
    fireEvent.pointerDown(head('v2'), { button: 2, pointerId: 1 })
    expect(onSwitch).not.toHaveBeenCalled()
  })

  // TRD S26-window-chrome §7-1 · §7-2 — the tab bar inside the title bar.
  it('tab row = window-drag spot (a valueless attribute — itself only) · absent on the tab bar root · head · × · +', () => {
    renderBar('v1')
    expect(screen.getByTestId('tab-strip-row').getAttribute('data-tauri-drag-region')).toBe('')
    const others = [
      screen.getByTestId('tab-bar'),
      ...screen.getAllByTestId('tab'),
      ...screen.getAllByTestId('tab-close'),
      screen.getByTestId('tab-add'),
    ]
    for (const el of others) expect(el.hasAttribute('data-tauri-drag-region'), el.dataset.testid).toBe(false)
  })

  it('the tab bar root is the side that shrinks in the title row — flex 1 1 0 · min-width 0 · height 28', () => {
    renderBar('v1')
    const s = screen.getByTestId('tab-bar').style
    expect([s.flexGrow, s.flexShrink, s.flexBasis, s.minWidth, s.height]).toEqual(['1', '1', '0px', '0px', '28px'])
  })

  it('[+] click → onCreate', () => {
    const { onCreate } = renderBar()
    fireEvent.click(screen.getByTestId('tab-add'))
    expect(onCreate).toHaveBeenCalledTimes(1)
  })

  it('tab × click → onClose(that view id) · pressing × is not a head press (no switch)', () => {
    const { onClose, onSwitch } = renderBar('v1')
    const close = head('v2').querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.pointerDown(close, { button: 0, pointerId: 1 })
    fireEvent.click(close)
    expect(onClose).toHaveBeenCalledWith('v2')
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('close · + button = an icon inside a 22 px circle · the gap before the + circle is left padding, not margin', () => {
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

  // jsdom does not paint :hover — we check only that the class is attached and that there is no inline background that beats it.
  it('the hover color of the × · + circles is a theme token class · no inline background · the two circles are the same vertical band as the head content', () => {
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

    const content = close.parentElement!
    expect([content.style.top, content.style.bottom, content.style.alignItems]).toEqual(['4px', '0px', 'center'])
    expect([add.style.top, add.style.bottom, add.style.alignItems]).toEqual([content.style.top, content.style.bottom, 'center'])
  })

  // The name inherits the head's text color — if the inline colors of the head and × are equal, the name · × are one color.
  it('name · × share one foreground color — active · hover = --tab-text · at rest = --tab-text-inactive · + is also that gray at rest', () => {
    renderBar('v1')
    const close = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-close"]')!
    const colours = (id: string) => [nameOf(id).style.color, head(id).style.color, close(id).style.color]
    expect(colours('v1')).toEqual(['', 'var(--tab-text)', 'var(--tab-text)'])
    expect(colours('v2')).toEqual(['', 'var(--tab-text-inactive)', 'var(--tab-text-inactive)'])

    fireEvent.pointerEnter(head('v2'))
    expect(colours('v2')).toEqual(['', 'var(--tab-text)', 'var(--tab-text)'])
    fireEvent.pointerLeave(head('v2'))
    expect(colours('v2')).toEqual(['', 'var(--tab-text-inactive)', 'var(--tab-text-inactive)'])

    expect(screen.getByTestId('tab-add').style.color).toBe('var(--tab-text-inactive)')
  })

  it('+ circle (icon) click → onCreate · close icon click → onClose', () => {
    const { onCreate, onClose } = renderBar('v1')
    fireEvent.click(screen.getByTestId('tab-add-circle').querySelector('svg')!)
    expect(onCreate).toHaveBeenCalledTimes(1)
    fireEvent.click(head('v2').querySelector('[data-testid="tab-close"] svg')!)
    expect(onClose).toHaveBeenCalledWith('v2')
  })

  it('name double-click → enters inline edit (input appears, seeded with the current name)', () => {
    renderBar('v1')
    const input = openEditForV1()
    expect(input.value).toBe('View 1')
    expect(input.getAttribute('data-view-id')).toBe('v1')
  })

  it('Enter (new name) → onRename(id, trimmed new name) — exactly once (prevents the Enter+blur double call, FIX 3)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '  Renamed  ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).toHaveBeenCalledWith('v1', 'Renamed')
    // A blur after unmount is not delivered by React 19 during commit and the input is detached, so this blur never reaches the root listener or commitEdit's `editingId` check — it does not measure that check.
    fireEvent.blur(input)
    expect(onRename).toHaveBeenCalledTimes(1)
  })

  it('select() runs exactly once on entering edit (prevents re-selecting on every keystroke, FIX 1)', () => {
    renderBar('v1')
    const selectSpy = vi.spyOn(HTMLInputElement.prototype, 'select')
    const input = openEditForV1()
    expect(selectSpy).toHaveBeenCalledTimes(1)
    // If a keystroke (setDraft→re-render) re-runs select, the character just typed is overwritten wholesale.
    fireEvent.change(input, { target: { value: 'N' } })
    fireEvent.change(input, { target: { value: 'Ne' } })
    fireEvent.change(input, { target: { value: 'New' } })
    expect(selectSpy).toHaveBeenCalledTimes(1)
    expect(input.value).toBe('New')
    selectSpy.mockRestore()
  })

  it('Esc → cancels edit (onRename not called, revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: 'Whatever' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(onRename).not.toHaveBeenCalled()
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('committing an empty/blank name (Enter) → onRename not called (revert)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  it('committing an unchanged name (Enter) → onRename not called (no-op)', () => {
    const { onRename } = renderBar('v1')
    const input = openEditForV1()
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  // TRD S26-window-chrome §3-1 — confirming an auto name with exactly those characters still counts as having set the name.
  it('committing the same text of an auto-named tab (Enter) → onRename(id, that text) once', () => {
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

  it('an auto-named tab also committing empty/blank (Enter) → onRename not called', () => {
    const { onRename } = renderBar('v1', [{ id: 'v1', name: '빈 칸', named: false }])
    const input = openEditForV1()
    fireEvent.change(input, { target: { value: '   ' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onRename).not.toHaveBeenCalled()
  })

  // Unchanged text is sent only with Enter.
  it('auto-named tab — losing focus without changing the text does not call onRename and just ends the edit', () => {
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

  it('auto-named tab — changed text is also sent on focus loss · once', () => {
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

  it('even if the name changes during edit (auto name follows · LLM rename), focus loss · Enter on the untouched field do not write in the old name', () => {
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

  // Tauri `drag.js` blocks the default action of a press on the draggable area (focus move), so the input does not blur; imitated by a document bubble-phase listener attached first that also cuts off later listeners (jsdom moves no focus on press).
  it('pressing the window-drag spot during edit (empty part of the tab row) → ends the edit — the same rule as focus loss (only changed text is sent)', () => {
    const dragJs = (e: MouseEvent) => {
      if (!(e.target instanceof Element) || !e.target.hasAttribute('data-tauri-drag-region')) return
      e.preventDefault()
      e.stopImmediatePropagation()
    }
    document.addEventListener('mousedown', dragJs)
    try {
      const { onRename } = renderBar('v1')
      let input = openEditForV1()
      expect(document.activeElement).toBe(input)
      fireEvent.change(input, { target: { value: ' Renamed ' } })
      fireEvent.mouseDown(screen.getByTestId('tab-strip-row'))
      expect(screen.queryByTestId('tab-rename-input')).toBeNull()
      expect(onRename).toHaveBeenCalledTimes(1)
      expect(onRename).toHaveBeenCalledWith('v1', 'Renamed')

      input = openEditForV1()
      fireEvent.mouseDown(screen.getByTestId('tab-strip-row'))
      expect(screen.queryByTestId('tab-rename-input')).toBeNull()
      expect(onRename).toHaveBeenCalledTimes(1)
    } finally {
      document.removeEventListener('mousedown', dragJs)
    }
  })

  it('a press that is not a drag spot does not take focus away — the same rule where pressing × of the tab being edited keeps the input field focus', () => {
    renderBar('v1')
    const input = openEditForV1()
    fireEvent.mouseDown(head('v1').querySelector('[data-testid="tab-close"]')!)
    expect(document.activeElement).toBe(input)
    expect(screen.getByTestId('tab-rename-input')).toBe(input)
  })

  it('auto name with surrounding whitespace — losing focus untouched does not call onRename', () => {
    const { onRename } = renderBar('v1', [{ id: 'v1', name: ' proj ', named: false }])
    const input = openEditForV1()
    expect(input.value).toBe(' proj ')
    fireEvent.blur(input)
    expect(onRename).not.toHaveBeenCalled()
  })

  it('set name with surrounding whitespace — even if the LLM changes it during edit, focus loss · Enter on the untouched field do not write in the old name', () => {
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

  it('input keystrokes during edit do not leak outside (to global keybindings) (stopPropagation)', () => {
    renderBar('v1')
    const input = openEditForV1()
    const outside = vi.fn()
    document.addEventListener('keydown', outside)
    fireEvent.keyDown(input, { key: 'a' })
    document.removeEventListener('keydown', outside)
    expect(outside).not.toHaveBeenCalled()
  })

  it('inactive tab name double-click gesture → switches to that tab on press · enters edit', () => {
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

  it('while closing with consecutive × clicks, the × of the next tab pushed in (detail 2) closes — even for a tab made active earlier by a head press · it does not open name edit', () => {
    const { onClose } = renderBar('v1')
    fireEvent.pointerDown(nameOf('v2'), { button: 0, pointerId: 1 })
    const close = (id: string) => head(id).querySelector<HTMLElement>('[data-testid="tab-close"]')!
    fireEvent.click(close('v1'), { detail: 1 })
    fireEvent.click(close('v2'), { detail: 2 })
    fireEvent.doubleClick(close('v2'), { detail: 2 })
    expect(vi.mocked(onClose).mock.calls).toEqual([['v1'], ['v2']])
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('a head double-click outside the name field also opens edit · a double-click inside the input field during edit does not erase typed text', () => {
    renderBar('v1')
    fireEvent.doubleClick(head('v1'))
    const input = screen.getByTestId('tab-rename-input') as HTMLInputElement
    fireEvent.change(input, { target: { value: 'Typed' } })
    fireEvent.doubleClick(input)
    expect((screen.getByTestId('tab-rename-input') as HTMLInputElement).value).toBe('Typed')
  })

  it('head = div · OS drag source (draggable)', () => {
    renderBar('v1')
    for (const el of screen.getAllByTestId('tab')) {
      expect(el.tagName).toBe('DIV')
      expect(el.getAttribute('draggable')).toBe('true')
    }
  })

  it('tab bar row = position reference · no scroll · no left padding · holds only heads and +', () => {
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

  it('a divider exists between each pair of drawn neighbours · beside the active and hovered tab it disappears without detaching · the active surface is only on the active tab', () => {
    const tabs = ['a', 'b', 'c', 'd', 'e', 'f'].map(id => ({ id, name: id, named: true }))
    renderBar('b', tabs)
    // The divider sits on the head's left edge — the divider at head x = between (the neighbor before x | x).
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

    // Hovering over the active tab does not draw a separate hover surface.
    fireEvent.pointerLeave(head('e'))
    fireEvent.pointerEnter(head('b'))
    expect(separated()).toEqual(['d', 'e', 'f'])
    expect(has('b', 'tab-hover')).toBe(false)
  })

  // PRD R27 · ADR-0295 decision 1 — top corner 10 · bottom wing 12 · left-right margin 2 · ADR-0298 decision 3 — divider 1×13.
  it('active tab shape = top corner radius 10 · bottom outer wing radius 12 · divider thickness 1 · height 13 · side margin 2', () => {
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

    // The margin · thickness · margin (2 + 1 + 2) area hangs at the middle of the head boundary.
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
  it('the divider disappears at once · only the two sides of a head whose hover ended appear with 200 ms EASE_IN · after that and where the active moved, revival is at once', () => {
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

      // When the active moves b → c, (a|b) revives at once without hover and (c|d) disappears at once.
      bar.rerender({ active: 'b' })
      bar.rerender({ active: 'c' })
      expect([sep('b').style.opacity, sep('b').style.transition]).toEqual(['1', ''])
      expect([d.style.opacity, d.style.transition]).toEqual(['0', ''])
    } finally {
      vi.useRealTimers()
    }
  })

  // The × and + of the last inactive head must not look stuck together.
  it('end divider on the right edge of the last head — only on the last head · centered on the boundary · shown only when inactive · not hovered', () => {
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

// ── Tab drag (OS drag source — TRD S21-slot-docking §4-2 · §4-4 · §4-6 · §8-2) ──
// jsdom has no DragEvent · DataTransfer, so events carry a drag data field. The source tab bar decision (`localDrag.setSourceStrip`) belongs to root receiving, so the test writes it directly; the engine's drag folding · real event spacing are not covered.

let rowWidth: number

const EASE = 'transform 200ms cubic-bezier(0.3333, 0.6667, 0.6667, 1)'
// Outside a drag — the property stays as is · only the time is 0 (a transition in progress is not cut).
const SNAP = 'transform 0ms cubic-bezier(0.3333, 0.6667, 0.6667, 1)'
/** The window fact the shell pushes (the answer to the `window_frame` pull). */
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

// Measure exactly as drawn — heads from `translateX` · width, the row · `+` from the constants of the hand calculation above.
function fakeRect(this: HTMLElement): DOMRect {
  const id = this.dataset.testid
  if (id === 'tab-strip-row') return rect(100, 40, rowWidth, 28)
  if (id === 'tab-add') return rect(0, 40, 40, 28)
  if (id === 'tab') return rect(100 + xOf(this), 40, parseFloat(this.style.width), 28)
  // The title bar containing the tab bar — top margin 6 · a draggable blank area on the right · number marker · buttons stand, so it is wider than the tab bar row.
  if (id === 'title-row') return rect(100, 34, 1100, 34)
  // The tab canvas — in the viewport (jsdom 1024 × 768) the area minus the maximized title bar 28.
  if (id === 'tab-canvas') return rect(0, 28, 1024, 740)
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

/** One drag event — carries `clientX` · `clientY` (testing-library drops those fields if there is no DragEvent). Return = not cancelled. */
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

/** A task different from dragstart — not a drag the engine folded. */
const nextTask = () =>
  act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0))
    await ticks()
  })

const strip = (index: number, left: number, x: number) => act(() => localDrag.setSourceStrip({ index, left, x }))
const leaveZone = () => act(() => localDrag.setSourceStrip(null))

// The tab bar's transition — jsdom has no `getAnimations`, so attach it by hand (without it, it reads as no transition).
interface FakeAnimation {
  playState: string
  pending: boolean
  finished: Promise<unknown>
}
let running: FakeAnimation[] = []
const animatable = HTMLElement.prototype as unknown as { getAnimations?: () => unknown[] }

/** Settling starts at the timer after the event following the release — advance through that timer and the wait that follows. */
const flush = () =>
  act(async () => {
    await new Promise(resolve => setTimeout(resolve, 0))
    await ticks()
  })

/** Leave one position transition in progress — the returned function ends that transition. */
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
 * Tab bar a b c d · active b before pressing. Press the a head (x 100..260) at x 180 (grab offset 80 · up to the transition echo) and dragstart at x 191.
 * Return = that drag's dataTransfer.
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

describe('TabBar — tab drag (OS drag source)', () => {
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

  /** After mount, until the identifier · window fact pull arrive. */
  async function mount(active: string | null = 'b', tabs: ViewMeta[] = ABCD, label = 'main') {
    const bar = renderBar(active, tabs, label)
    await act(async () => {
      await ticks()
    })
    return bar
  }

  // ── Drawing (outside a drag) ──

  it('head width · position = tab bar layout (usable width = row − +) · + is after the last head', async () => {
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

  it('narrow tab bar — draws only visible heads and an inactive head narrower than the active minimum width hides its close button', async () => {
    rowWidth = 140 // Usable width 100 → widths 52 · 32 · 32 · 32 · visible = the first two
    await mount('a')
    expect(headIds()).toEqual(['a', 'b'])
    expect(head('a').style.width).toBe(`${TAB_MIN_ACTIVE_W}px`)
    expect(head('a').querySelector('[data-testid="tab-close"]')).not.toBeNull()
    expect(head('b').style.width).toBe('32px')
    expect(head('b').querySelector('[data-testid="tab-close"]')).toBeNull()
  })

  // When the window is narrower than the title bar's minimum width — if row width 0 is counted as 「could not measure」, all heads stand at 160 and cover the right of the title bar.
  it('row width 0 (height present) = the measured value — draws no head at all · the tab bar root clips horizontally', async () => {
    rowWidth = 0
    await mount('a')
    expect(screen.queryAllByTestId('tab')).toEqual([])
    expect(xOf(screen.getByTestId('tab-add'))).toBe(0)
    expect(screen.getByTestId('tab-bar').style.overflowX).toBe('clip')
  })

  // TRD S26-window-chrome §7-9 (T-66): only where tabs are (first tab ~ `+`) is the tab bar zone; a drop on the blank to its right · draggable blank · number marker · window buttons creates a new window.
  it('tab bar zone background = first tab left ~ + right end × tab bar row (the title row if inside the title row — including the top margin) · 15 above and below it', async () => {
    const bar = await mount('a')
    // Four heads 640 · + 640..680 → viewport 100..780 (the tab bar row goes to 800).
    expect(xOf(screen.getByTestId('tab-add'))).toBe(640)
    expect(localDrag.readStripGeometry()!.zoneViewport).toEqual({ x0: 100, y0: 25, x1: 780, y1: 83 })

    const title = document.createElement('div')
    title.dataset.testid = 'title-row'
    bar.rerender({ titleRow: { current: title } })
    expect(localDrag.readStripGeometry()).toMatchObject({ left: 100, available: 660 })
    // To the right of the + in the title bar (100..1200) is not the zone — only the vertical follows the title bar.
    expect(localDrag.readStripGeometry()!.zoneViewport).toEqual({ x0: 100, y0: 19, x1: 780, y1: 83 })
  })

  it('tab bar zone — if there are hidden heads up to the visible end where + stands · with no tabs only +', async () => {
    rowWidth = 140 // Usable width 100 → visible heads a(52) · b(32) · + 84..124
    const bar = await mount('a')
    expect(localDrag.readStripGeometry()!.zoneViewport).toEqual({ x0: 100, y0: 25, x1: 224, y1: 83 })

    bar.rerender({ tabs: [], active: null })
    expect(localDrag.readStripGeometry()!.zoneViewport).toEqual({ x0: 100, y0: 25, x1: 140, y1: 83 })
  })

  // ── draggable condition ──

  it('head = drag source (draggable) — not a head under name edit · not main\'s last tab (Q35) · a popout with only one tab turns it off (Q34)', async () => {
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

  it('dragstart = one of our formats (type string · value) · effectAllowed move · label image · does not reach window · head position · DOM as is', async () => {
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
    // Type = lowercase · app · drag · original size (restore 1000×700 − chrome 0) · no shape field (no cache).
    const value = valueOf(dt)
    expect(typeOf(dt)).toBe(`application/x-engram-tab;app=com.engram.test;drag=${value.drag};w=1000;h=700`)
    expect(value).toEqual({
      v: 1,
      drag: value.drag,
      view: 'a',
      from: 'main',
      initial: 'b',
      size: { w: 1000, h: 700 },
      // Tab bar left 100 + grab ratio 0.5 × the width of one new-window tab 160 · the pressed y.
      grab: { x: 180, y: 50 },
    })
    // VS Code style label — cursor (−10, −10) · removed at the next task.
    const [label, ox, oy] = dt.setDragImage.mock.calls[0] as [HTMLElement, number, number]
    expect([label.textContent, ox, oy]).toEqual(['A', -10, -10])
    expect(label.isConnected).toBe(true)
    // ★If the dragged head leaves from under the pressed point inside dragstart, the engine folds the drag★ — same node · same place · no insert or remove of the head.
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    expect(nodes.map(n => n.style.transform)).toEqual(before)
    expect(added).toEqual([])
    expect(localDrag.getOutgoing()).toEqual({ drag: value.drag, window: 'main', view: 'a', grabOffset: 80 })
    expect(invokeMock.mock.calls.filter(c => c[0] !== 'window_frame')).toEqual([])

    await nextTask()
    expect(label.isConnected).toBe(false)
  })

  // TRD S26-window-chrome §7-6 — a maximized title bar has no top margin, and a new window is in the normal state so that margin (6) stands.
  it('a tab released from a maximized window — the new window\'s chrome · the grabbed y counts as the normal-state title row height', async () => {
    invokeMock.mockImplementation(async (cmd: string) => (cmd === 'window_frame' ? { ...FRAME, maximized: true } : undefined))
    const bar = await mount('b')
    const title = document.createElement('div')
    title.dataset.testid = 'title-row'
    const canvas = document.createElement('div')
    canvas.dataset.testid = 'tab-canvas'
    bar.rerender({ titleRow: { current: title }, canvas: () => canvas })
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    bar.rerender({ active: 'a' })
    const dt = fakeDt()
    drag('dragStart', head('a'), dt)
    // Chrome = 768 − 740 = 28 (maximized row) → new window 34 · original size = restore 1000 × (700 − 34).
    const value = valueOf(dt)
    expect(typeOf(dt)).toBe(`application/x-engram-tab;app=com.engram.test;drag=${value.drag};w=1000;h=666`)
    expect(value).toMatchObject({ size: { w: 1000, h: 700 }, grab: { x: 180, y: 56 } })
  })

  it('a drag pressed on the close button (the engine fires it on the head) · a text drag started from the name input → cancelled · no data · reaches window', async () => {
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

    // The next press drags again.
    fireEvent.keyDown(screen.getByTestId('tab-rename-input'), { key: 'Escape' })
    fireEvent.pointerDown(nameOf('c'), { button: 0, clientX: 450, clientY: 50 })
    const ok = fakeDt()
    expect(drag('dragStart', head('c'), ok)).toBe(true)
    expect(ok.setData).toHaveBeenCalledTimes(1)
  })

  it('a drag outside heads (tree row · file etc.) is not touched — neither blocked, cut nor carried', async () => {
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

  // ── Source tab bar while dragging ──

  it('inside the zone — the dragged head follows the cursor directly (no switch · on top) and neighbours step aside in the confirmed order with 200 ms EASE_OUT · DOM · key as is', async () => {
    const bar = await mount('b')
    const row = screen.getByTestId('tab-strip-row')
    const nodes = screen.getAllByTestId('tab')
    await startA(bar)

    // The first decision that enters the zone = confirmed without a threshold (Chrome `justAttached`).
    strip(1, 120, 300)
    expect(xOf(head('a'))).toBe(120)
    expect(head('a').style.transition).toBe('none')
    expect(head('a').style.zIndex).toBe('1')
    expect([xOf(head('b')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 320, 480])
    expect(head('b').style.transition).toBe(EASE)
    expect(screen.getAllByTestId('tab')).toEqual(nodes)
    expect(screen.getByTestId('tab-strip-row')).toBe(row)
    expect(localDrag.readStripGeometry()).toMatchObject({ left: 100, tabs: ['b', 'a', 'c', 'd'], available: 660 })
    expect(localDrag.readStripGeometry()!.zoneViewport).toEqual({ x0: 100, y0: 25, x1: 780, y1: 83 })

    // A move back within the threshold (tab width 160 → 16 px) does not change the position — only the dragged head follows.
    strip(0, 110, 290)
    expect(xOf(head('a'))).toBe(110)
    expect(xOf(head('b'))).toBe(0)
    // Past the threshold, to the position of that decision.
    strip(0, 60, 240)
    expect([xOf(head('a')), xOf(head('b'))]).toEqual([60, 160])
    expect(invokes('tab_drop')).toEqual([])
  })

  it('outside the zone = paints the remaining tab (the active before pressing) and the head returns to the model position in 200 ms · when it comes back the dragged tab', async () => {
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

  it('the dragged head is drawn even if the layout hides it — detaching the source node means dragend never comes', async () => {
    rowWidth = 140 // Usable width 100 — with active b only a · b are visible
    const bar = await mount('b', ABCD)
    fireEvent.pointerDown(head('b'), { button: 0, clientX: 140, clientY: 50 })
    bar.rerender({ tabs: [ABCD[1], ABCD[0], ABCD[2], ABCD[3]], active: 'b' })
    const b = head('b')
    drag('dragStart', b, fakeDt(), 145)
    await nextTask()
    // While dragging, the LLM inserted one tab in front and b was pushed past the visible end.
    bar.rerender({ tabs: [{ id: 'z', name: 'Z', named: true }, ABCD[0], ABCD[2], ABCD[1], ABCD[3]], active: 'b' })
    leaveZone()
    expect(head('b')).toBe(b)
  })

  // ── dragend ──

  it.each([
    ['none', 'none'],
    ['move', 'move'],
    ['copy', 'none'],
  ])('dragend(dropEffect=%s) → tab_drop{NewWindow{by: DragEnd{effect: %s}}} once', async (dropEffect, effect) => {
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
    // Two dragends of the same drag = one.
    drag('dragEnd', head('a'), fakeDt(dropEffect))
    expect(drops()).toHaveLength(1)
  })

  it('after the drop — the dragged head slides to the confirmed position, and in the very frame the model order arrives model position = the same position · same node', async () => {
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

  it('same-webview in-place drop (the root drop records it) → sends nothing · released at once to the model position · no revert', async () => {
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

  it('dragend in the same task as dragstart (a drag the engine collapsed) → sends nothing · released · no revert', async () => {
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

  it('tab_drop invoke error → releases the swapped drawing at once', async () => {
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

  // ── Result notice · restoring the original selection (§4-6) ──

  it.each(['canceled', 'ignored'] as const)(
    'done{%s} (own dragend none) → switch_tab_if(initial, expect = dragged tab) once · initial is painted while reverting',
    async outcome => {
      const bar = await mount('b')
      const dt = await startA(bar)
      drag('dragEnd', head('a'), fakeDt('none'))
      emit('tab_drop:done', { drag: valueOf(dt).drag, outcome })
      emit('tab_drop:done', { drag: valueOf(dt).drag, outcome })
      expect(invokes('switch_tab_if')).toEqual([{ window: 'main', view: 'b', expect: 'a' }])
      expect(head('b').getAttribute('data-active')).toBe('true')
      // The restore arrives — release it with that notice.
      bar.rerender({ active: 'b' })
      expect(head('b').getAttribute('data-active')).toBe('true')
      expect(localDrag.getOutgoing()).toBeNull()
    },
  )

  it('branches that do not revert — the active changed in between · initial = dragged tab · Rejected · own dragend is move · someone else\'s drag', async () => {
    const cases: Array<{ name: string; run: (bar: ReturnType<typeof renderBar>) => Promise<void> }> = [
      {
        name: 'active changed',
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
        name: 'a drag of someone else',
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

  it('a revert error is caught, only a log is left and it falls back to the model active', async () => {
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

  it('if the dragged tab is pressed before the revert lands, switch_tab is sent even if the model active is that tab — if the shell applies in order the final active = the pressed tab', async () => {
    // The shell — applies the commands it received in order (a restore changes only when `expect` matches). The apply is pushed by hand.
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
    // Even if a canvas press releases the redraw first, the restore is still in the shell.
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

    // A press on the active tab after the restore arrives is an ordinary press.
    onSwitch.mockClear()
    fireEvent.pointerDown(head('a'), { button: 0, pointerId: 1, clientX: 180, clientY: 50 })
    expect(onSwitch).not.toHaveBeenCalled()
  })

  it('if that tab leaves that window during the drag (LLM close) it is released — remaining heads as the model says', async () => {
    const bar = await mount('b')
    await startA(bar)
    leaveZone()
    bar.rerender({ tabs: [ABCD[1], ABCD[2], ABCD[3]], active: 'b' })
    expect(headIds()).toEqual(['b', 'c', 'd'])
    expect([xOf(head('b')), xOf(head('c')), xOf(head('d'))]).toEqual([0, 160, 320])
    expect(localDrag.getOutgoing()).toBeNull()
  })

  // ── The vertical line of another window's drag (the root dragover writes it) ──

  it('vertical line = 2 px · centered on that x (1 px at each head end) · not drawn if absent', async () => {
    await mount('a')
    expect(screen.queryByTestId('tab-drop-marker')).toBeNull()
    act(() => localDrag.setTargetMarker(160))
    const line = screen.getByTestId('tab-drop-marker')
    expect([xOf(line), line.style.width]).toEqual([159, '2px'])
    act(() => localDrag.setTargetMarker(null))
    expect(screen.queryByTestId('tab-drop-marker')).toBeNull()
  })

  // ── Head DOM order = tab key · screen reader order ──

  /** Until a is moved behind b (model echo b · a · c · d) and that drag is released — after the drop a position transition is in progress. */
  async function reorderAThenRelease(bar: ReturnType<typeof renderBar>): Promise<() => Promise<void>> {
    const dt = await startA(bar)
    strip(1, 120, 300)
    const settle = holdTransition()
    drag('dragEnd', head('a'), fakeDt('move'))
    bar.rerender({ tabs: BACD })
    emit('tab_drop:done', { drag: valueOf(dt).drag, outcome: 'applied' })
    return settle
  }

  it('once the drag is released and the position transition ends, head DOM order is aligned to the visible order — same node · same position · row as is', async () => {
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

  it('after dropping but before the drag is released (waiting for done) it does not align even when the transition ends', async () => {
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

  it('a press arriving before aligning blocks aligning even if the transition ends meanwhile, and on release it aligns after the event following that release', async () => {
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

  it('an animation left as fill after finishing is not waited for', async () => {
    const bar = await mount('b')
    const [a, b, c, d] = screen.getAllByTestId('tab')
    await reorderAThenRelease(bar)
    running = [{ playState: 'finished', pending: false, finished: Promise.resolve() }]
    let polls = 0
    // If the wait spins in place, after the fifth give an empty list so the test fails instead of hanging.
    animatable.getAnimations = () => (++polls > 5 ? [] : running)

    await flush()
    expect(screen.getAllByTestId('tab')).toEqual([b, a, c, d])
    expect(polls).toBe(1)
  })

  it('an order changed by another path at rest becomes the DOM order at once, and during name edit it is deferred until the edit ends', async () => {
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

  // ── Name edit · narrow tab bar ──
  it('narrow tab bar — even if the second click of an inactive head double-click lands on a just-appeared close button it does not close and opens edit · a close pressed separately after that closes', () => {
    rowWidth = 140 // Usable width 100 → a 52 · b 32 (no close button) · visible = the first two · b is at screen x 152..184
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

  it('narrow tab bar — even an active head whose name field width is 0 opens edit by a head double-click, and the head under edit is pulled to the standard width · on top · to the row\'s left end and covers beyond the usable width (the + spot)', () => {
    rowWidth = 140 // Active b → a 32 (x 0) · b 52 (x 32) · + x 84 · usable width 100 — while editing b is 0..160
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

  it('a head under edit widens in place if there is room on the right', () => {
    rowWidth = 500 // Usable width 460 → heads 115 · x 0 · 115 · 230 · 345
    renderBar('a', ABCD)
    fireEvent.doubleClick(nameOf('c'))
    expect([xOf(head('c')), head('c').style.width]).toEqual([230, '160px'])
  })

  it('a press on a neighbour spot covered by the widened edit head commits the edit and only switches to that neighbour · presses on the pre-widening spot · on the input field are as is', () => {
    rowWidth = 500 // Heads 115 · x 0 · 115 · 230 · 345 → while editing, c 230..390 covers the left of d (345..460)
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

  it('auto-named tab — pressing a covered neighbour spot without changing the text does not call onRename and only switches to that neighbour', () => {
    rowWidth = 500 // The same place as the test above — while editing, c 230..390 covers the left of d (345..460)
    const bar = renderBar('c', ABCD.map(tab => ({ ...tab, named: false })))
    fireEvent.doubleClick(nameOf('c'))
    fireEvent.pointerDown(head('c'), { button: 0, pointerId: 1, clientX: 450, clientY: 50 })
    expect(bar.onRename).not.toHaveBeenCalled()
    expect(bar.onSwitch).toHaveBeenCalledWith('d')
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('narrow tab bar — a press on a neighbour spot covered by the edit head pulled left also switches to that neighbour, and a press on the + spot with no head underneath is left as now', () => {
    rowWidth = 140 // Active b → a 32 (x 0) · b 52 (x 32) · + x 84 — while editing b is 0..160
    const bar = renderBar('b', ABCD)
    fireEvent.doubleClick(head('b'))
    fireEvent.pointerDown(head('b'), { button: 0, pointerId: 1, clientX: 190, clientY: 50 })
    expect(screen.getByTestId('tab-rename-input').getAttribute('data-view-id')).toBe('b')
    expect(bar.onCreate).not.toHaveBeenCalled()

    fireEvent.pointerDown(head('b'), { button: 0, pointerId: 1, clientX: 110, clientY: 50 })
    expect(bar.onSwitch).toHaveBeenCalledWith('a')
    expect(screen.queryByTestId('tab-rename-input')).toBeNull()
  })

  it('the × of a tab under name edit does not steal input field focus on press and closes on the first click · when that tab leaves the model the edit also ends and head DOM order is aligned again', () => {
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
