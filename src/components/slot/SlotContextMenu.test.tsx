// ★Reject-leak safety is not retested here★: the menu calls only fireAndForget, so "swallows sync throw · async reject · thenable with no leak" is owned by dispatch.test.ts. The only concern here is "does the menu route through that shared helper".

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ResolvedSlotMenuItem } from '../../commands/slotMenu'
import { setLanguage } from '../../i18n'

const dispatchMock = vi.hoisted(() => ({ fireAndForget: vi.fn() }))
vi.mock('../../commands/dispatch', () => ({
  fireAndForget: (...args: unknown[]) => dispatchMock.fireAndForget(...args),
}))

import SlotContextMenu, { ANCHOR_GAP, clampMenuPosition, flyoutPosition } from './SlotContextMenu'
import { OVERLAY_SELECTOR } from './interruptKey'

function item(id: string, over: Partial<ResolvedSlotMenuItem> = {}): ResolvedSlotMenuItem {
  return { id, title: id, run: vi.fn(), group: 'slot-ops', separatorBefore: false, ...over }
}

beforeEach(() => {
  dispatchMock.fireAndForget.mockClear()
})
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('SlotContextMenu — overlay marker (ADR-0237)', () => {
  it('the root carries the marker — while open the chat area\'s Esc does not interrupt the turn', () => {
    const { container } = render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('slot.close')]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: 'a1' }}
        onClose={vi.fn()}
      />,
    )
    expect(document.querySelector(OVERLAY_SELECTOR)).toBe(container.firstElementChild)
  })
})

describe('SlotContextMenu — close on outside press', () => {
  it('it also closes on pressing the window-drag spot — even if the document bubble-phase listener hooked later by Tauri `drag.js` cuts it', () => {
    const drag = document.createElement('div')
    drag.setAttribute('data-tauri-drag-region', '')
    document.body.appendChild(drag)
    // Imitates `drag.js` — a document bubble-phase listener registered before the menu cuts the press on a draggable spot.
    const dragJs = (e: MouseEvent) => {
      if (e.target !== drag) return
      e.preventDefault()
      e.stopImmediatePropagation()
    }
    document.addEventListener('mousedown', dragJs)
    try {
      const onClose = vi.fn()
      render(
        <SlotContextMenu
          x={0}
          y={0}
          items={[item('slot.close')]}
          ctx={{ viewId: 'v1', slotId: 's1', agentId: 'a1' }}
          onClose={onClose}
        />,
      )
      fireEvent.mouseDown(drag)
      expect(onClose).toHaveBeenCalledTimes(1)
    } finally {
      document.removeEventListener('mousedown', dragJs)
      drag.remove()
    }
  })
})

describe('SlotContextMenu — shared dispatch path (FIX-3)', () => {
  it('item click → routed with fireAndForget(item.id, ctx) (not a direct run call)', () => {
    const onClose = vi.fn()
    const it0 = item('slot.close')
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[it0]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: 'a1' }}
        onClose={onClose}
      />,
    )
    fireEvent.click(screen.getByText('slot.close'))
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('slot.close', {
      viewId: 'v1',
      slotId: 's1',
      agentId: 'a1',
    })
    expect(it0.run).not.toHaveBeenCalled()
    expect(onClose).toHaveBeenCalled()
  })

  it('container (children) item: the flyout opens on hover and a child click → fireAndForget(child.id, ctx)', () => {
    const onClose = vi.fn()
    const container = item('container:slot.newContent', {
      title: '새 콘텐츠',
      children: [item('slot.fill.agentList', { title: '트리' })],
    })
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[container]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={onClose}
      />,
    )
    expect(screen.getByText('새 콘텐츠')).toBeTruthy()
    expect(screen.queryByText('트리')).toBeNull()
    fireEvent.mouseEnter(screen.getByText('새 콘텐츠'))
    const child = screen.getByText('트리')
    expect(child).toBeTruthy()
    fireEvent.click(child)
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('slot.fill.agentList', {
      viewId: 'v1',
      slotId: 's1',
      agentId: null,
    })
    expect(onClose).toHaveBeenCalled()
  })

  it('an unassigned agentId (null) is also passed through as ctx', () => {
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('slot.split')]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByText('slot.split'))
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('slot.split', {
      viewId: 'v1',
      slotId: 's1',
      agentId: null,
    })
  })
})

// ── on/off items · disabled · role (ADR-0252) ──
describe('SlotContextMenu — checked · enabled · role', () => {
  const slotCtx = {
    viewId: 'v1',
    slotId: 's1',
    agentId: null,
    content: { type: 'agent_list' as const },
  }
  const row = (id: string) => document.querySelector(`[data-slot-menu-item="${id}"]`) as HTMLElement

  it('with checked it is menuitemcheckbox + aria-checked, without it menuitem · no aria-checked — checked receives ctx', () => {
    const seen: unknown[] = []
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[
          item('on', {
            checked: ctx => {
              seen.push(ctx)
              return true
            },
          }),
          item('off', { checked: () => false }),
          item('plain'),
        ]}
        ctx={slotCtx}
        onClose={vi.fn()}
      />,
    )
    expect(row('on').getAttribute('role')).toBe('menuitemcheckbox')
    expect(row('on').getAttribute('aria-checked')).toBe('true')
    expect(row('off').getAttribute('aria-checked')).toBe('false')
    expect(row('plain').getAttribute('role')).toBe('menuitem')
    expect(row('plain').hasAttribute('aria-checked')).toBe(false)
    expect(seen[0]).toBe(slotCtx)
  })

  it('role — root · submenu = menu · divider = separator · container line = a menuitem that opens a submenu', () => {
    const container = item('container:묶음', { title: '묶음', children: [item('kid')] })
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('a'), { ...container, separatorBefore: true }]}
        ctx={slotCtx}
        onClose={vi.fn()}
      />,
    )
    const menus = () => document.querySelectorAll('[role="menu"]')
    expect(menus()).toHaveLength(1)
    expect(document.querySelectorAll('[role="separator"]')).toHaveLength(1)
    const opener = screen.getByText('묶음').parentElement as HTMLElement
    expect(opener.getAttribute('role')).toBe('menuitem')
    expect(opener.getAttribute('aria-haspopup')).toBe('menu')
    expect(opener.getAttribute('aria-expanded')).toBe('false')
    fireEvent.mouseEnter(screen.getByText('묶음'))
    expect(menus()).toHaveLength(2)
    expect(opener.getAttribute('aria-expanded')).toBe('true')
    expect(row('kid').getAttribute('role')).toBe('menuitem')
  })

  it('execution argument = coordinates + slot content (content) — neither other fields added to ctx nor a caller marker are passed', () => {
    const withState = { ...slotCtx, menuState: true } // A case where the assembler added a menu state field
    render(<SlotContextMenu x={0} y={0} items={[item('toggle.x')]} ctx={withState} onClose={vi.fn()} />)
    fireEvent.click(screen.getByText('toggle.x'))
    const [id, args] = dispatchMock.fireAndForget.mock.calls[0] as [string, Record<string, unknown>]
    expect(id).toBe('toggle.x')
    expect(args).toStrictEqual({ viewId: 'v1', slotId: 's1', agentId: null, content: slotCtx.content })
  })

  it('enabled false → aria-disabled on a line with a role · clicking does not run · does not close the menu — enabled receives ctx', () => {
    const onClose = vi.fn()
    const enabled = vi.fn(() => false)
    render(<SlotContextMenu x={0} y={0} items={[item('gated.x', { enabled })]} ctx={slotCtx} onClose={onClose} />)
    expect(row('gated.x').getAttribute('role')).toBe('menuitem')
    expect(row('gated.x').getAttribute('aria-disabled')).toBe('true')
    fireEvent.click(row('gated.x'))
    expect(dispatchMock.fireAndForget).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()
    expect(enabled).toHaveBeenCalledWith(slotCtx)
  })

  it('enabled true (or absent) → runs as usual', () => {
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('gated.x', { enabled: () => true }), item('plain')]}
        ctx={slotCtx}
        onClose={vi.fn()}
      />,
    )
    expect(row('gated.x').hasAttribute('aria-disabled')).toBe(false)
    expect(row('plain').hasAttribute('aria-disabled')).toBe(false)
    fireEvent.click(row('gated.x'))
    expect(dispatchMock.fireAndForget).toHaveBeenCalledTimes(1)
  })

  it('if ctx changes the open menu re-judges in place', () => {
    const enabled = (ctx: { agentId?: string | null }) => ctx.agentId != null
    const menu = (agentId: string | null) => (
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('gated.x', { enabled })]}
        ctx={{ ...slotCtx, agentId }}
        onClose={vi.fn()}
      />
    )
    const { rerender } = render(menu('a1'))
    expect(row('gated.x').hasAttribute('aria-disabled')).toBe(false)
    rerender(menu(null))
    expect(row('gated.x').getAttribute('aria-disabled')).toBe('true')
  })

  it('checked · enabled throw → rendering survives and draws off · disabled', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const boom = () => {
      throw new Error('boom')
    }
    render(
      <SlotContextMenu
        x={0}
        y={0}
        items={[item('a', { checked: boom }), item('b', { enabled: boom })]}
        ctx={slotCtx}
        onClose={vi.fn()}
      />,
    )
    expect(row('a').getAttribute('aria-checked')).toBe('false')
    expect(row('b').getAttribute('aria-disabled')).toBe('true')
    // It is decided on every redraw, so look per item rather than per count.
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/checked verdict failed — drawing "a"/), expect.any(Error))
    expect(error).toHaveBeenCalledWith(expect.stringMatching(/enabled verdict failed — drawing "b"/), expect.any(Error))
  })

  it('submenu children are drawn as the same line (checked · enabled)', () => {
    const container = item('container:묶음', {
      title: '묶음',
      children: [item('kid', { checked: () => true, enabled: () => false })],
    })
    render(<SlotContextMenu x={0} y={0} items={[container]} ctx={slotCtx} onClose={vi.fn()} />)
    fireEvent.mouseEnter(screen.getByText('묶음'))
    expect(row('kid').getAttribute('aria-checked')).toBe('true')
    expect(row('kid').getAttribute('aria-disabled')).toBe('true')
  })
})

// ── ★Viewport clamp (Bug1)★: the pure helper clampMenuPosition is unit-tested broadly — jsdom's getBoundingClientRect returns 0, so the component path cannot overflow; the component only calls this helper (GUI measurement is separate). ──
describe('clampMenuPosition — flips on overflow (the anchor stays at the gap outside the menu)', () => {
  const VW = 1000
  const VH = 800
  const W = 150
  const H = 300

  /**
   * ★The required value is written apart from the implementation constant (ANCHOR_GAP)★: using `ANCHOR_GAP` in the assertion would neutralize it if the constant became 0. This is a *lower bound* derived from the OS — Windows' default double-click tolerance rectangle is 4px (radius 2px), so twice that is required. Passes if the implementation shrinks from 8 to 6, fails below the radius.
   */
  const REQUIRED_GAP = 4

  /**
   * The *larger* of the per-axis distances from the anchor to the menu rectangle. The second click lands on the menu only if both axes are inside the rectangle, so one axis this far off does not reach it within the tolerance radius. 0 = the anchor is inside the rectangle (an item under the cursor).
   */
  function anchorClearance(x: number, y: number, pos: { top: number; left: number }, w: number, h: number) {
    const dx = Math.max(pos.left - x, x - (pos.left + w), 0)
    const dy = Math.max(pos.top - y, y - (pos.top + h), 0)
    return Math.max(dx, dy)
  }

  it('if it does not overflow a position the gap away from the anchor (below and right of the anchor)', () => {
    expect(clampMenuPosition(100, 100, W, H, VW, VH)).toEqual({
      top: 100 + ANCHOR_GAP,
      left: 100 + ANCHOR_GAP,
    })
  })

  it('bottom overflow → flips up and also keeps the gap in that direction (menu bottom is above the anchor)', () => {
    const pos = clampMenuPosition(100, 700, W, H, VW, VH)
    expect(pos.top).toBe(700 - ANCHOR_GAP - H)
    expect(pos.top + H).toBeLessThanOrEqual(VH)
    expect(anchorClearance(100, 700, pos, W, H)).toBeGreaterThanOrEqual(REQUIRED_GAP)
  })

  it('right overflow → flips left and also keeps the gap in that direction', () => {
    const pos = clampMenuPosition(950, 100, W, H, VW, VH)
    expect(pos.left).toBe(950 - ANCHOR_GAP - W)
    expect(pos.left + W).toBeLessThanOrEqual(VW)
    expect(anchorClearance(950, 100, pos, W, H)).toBeGreaterThanOrEqual(REQUIRED_GAP)
  })

  it('bottom-right corner overflow → flips both axes and keeps the gap on both axes', () => {
    const pos = clampMenuPosition(950, 700, W, H, VW, VH)
    expect(pos).toEqual({ top: 700 - ANCHOR_GAP - H, left: 950 - ANCHOR_GAP - W })
    expect(anchorClearance(950, 700, pos, W, H)).toBeGreaterThanOrEqual(REQUIRED_GAP)
  })

  it('the menu is larger than the viewport (h>vh) → it does not fit even flipped, so push fallback (top fixed, prevents negative)', () => {
    // In a degenerate case "it is inside the screen" beats the anchor rule (including the gap) — the anchor is allowed to come inside.
    const { top } = clampMenuPosition(100, 700, W, 900, VW, VH)
    expect(top).toBe(4)
  })

  it('an exact fit including the gap (y+GAP+h===vh) is not an overflow → it does not flip', () => {
    // Pins that the overflow decision boundary includes the gap (deciding without the gap lets the menu go 1..GAP px off screen).
    const y = VH - H - ANCHOR_GAP
    expect(clampMenuPosition(0, y, W, H, VW, VH).top).toBe(VH - H)
  })

  // ★Property assertion★: pins "the anchor is at least the required gap from the menu + the menu is on screen" rather than a number, so touching the placement formula again is caught (the whole normal range where menu ≤ viewport). ★Strengthened (ADR-0143 decision 4)★: a placement flush against a corner passed the old "does not go inside" assertion, yet a double-click's second click executed an item.
  it('from any anchor inside the viewport the anchor stays at least the required gap from the menu and the menu is on screen', () => {
    for (const x of [0, 1, 100, 500, 841, 842, 843, 950, 999, VW]) {
      for (const y of [0, 1, 100, 400, 491, 492, 493, 700, 799, VH]) {
        const pos = clampMenuPosition(x, y, W, H, VW, VH)
        expect(
          anchorClearance(x, y, pos, W, H),
          `the anchor (${x},${y}) is too close to the menu`,
        ).toBeGreaterThanOrEqual(REQUIRED_GAP)
        expect(pos.left >= 0 && pos.left + W <= VW, `left off screen (${x},${y})`).toBe(true)
        expect(pos.top >= 0 && pos.top + H <= VH, `top off screen (${x},${y})`).toBe(true)
      }
    }
  })
})

// ── ★Submenu flyout placement (ADR-0065)★ ──
describe('flyoutPosition — submenu right/left expansion', () => {
  const VW = 1000
  const VH = 800
  const FW = 150 // flyout width
  const FH = 200 // flyout height

  it('enough room on the right → opens to the right from the parent\'s right edge (anchorRight)', () => {
    expect(flyoutPosition(100, 250, 100, FW, FH, VW, VH)).toEqual({ top: 100, left: 250 })
  })

  it('right overflow + room on the left → flips left (anchorLeft - FW)', () => {
    const { left } = flyoutPosition(900, 980, 100, FW, FH, VW, VH)
    expect(left).toBe(900 - FW)
    expect(left + FW).toBeLessThanOrEqual(VW)
  })

  it('bottom overflow → pushes top up so the whole flyout is inside the viewport', () => {
    const { top } = flyoutPosition(100, 250, 700, FW, FH, VW, VH)
    expect(top).toBe(VH - FH - 4)
    expect(top + FH).toBeLessThanOrEqual(VH)
  })

  it('the flyout is larger than the viewport (FH>VH) → top is fixed at the minimum margin (prevents negative)', () => {
    const { top } = flyoutPosition(100, 250, 700, FW, 900, VW, VH)
    expect(top).toBe(4)
  })
})

// ── Component path ──
describe('SlotContextMenu — viewport placement applied after mount (Bug1)', () => {
  const origRect = HTMLElement.prototype.getBoundingClientRect
  afterEach(() => {
    HTMLElement.prototype.getBoundingClientRect = origRect
  })

  it('opening near the bottom flips the menu up and the anchor stays outside the menu\'s lower gap', () => {
    HTMLElement.prototype.getBoundingClientRect = function () {
      return { width: 150, height: 300, top: 0, left: 0, right: 150, bottom: 300, x: 0, y: 0, toJSON() {} } as DOMRect
    }
    const vh = window.innerHeight
    render(
      <SlotContextMenu
        x={10}
        y={vh - 50}
        items={[item('slot.close')]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={vi.fn()}
      />,
    )
    const fixed = document.querySelector('div[style*="fixed"]') as HTMLElement
    expect(fixed).toBeTruthy()
    const topPx = parseInt(fixed.style.top, 10)
    expect(topPx).toBe(vh - 50 - ANCHOR_GAP - 300)
    expect(topPx + 300).toBeLessThanOrEqual(vh)
    // The menu bottom is ANCHOR_GAP above the anchor (y) = no item under the cursor, and the second click of a double-click cannot reach it either.
    expect(topPx + 300).toBe(vh - 50 - ANCHOR_GAP)
  })
})

// ADR-0373: a language switch changes the titles' width, so the open menu and its flyout are placed again.
describe('SlotContextMenu — placement after a language switch', () => {
  const origRect = HTMLElement.prototype.getBoundingClientRect
  afterEach(() => {
    HTMLElement.prototype.getBoundingClientRect = origRect
    act(() => {
      setLanguage('ko')
    })
  })

  it('measures the menu and the open flyout again', () => {
    // The sizes the titles would give in each language.
    let size = { menuH: 300, flyW: 100 }
    HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
      const w = this.hasAttribute('data-slot-menu-flyout') ? size.flyW : 150
      const h = this.hasAttribute('data-slot-menu-flyout') ? 50 : size.menuH
      return { width: w, height: h, top: 0, left: 0, right: w, bottom: h, x: 0, y: 0, toJSON() {} } as DOMRect
    }
    const vh = window.innerHeight
    const vw = window.innerWidth
    render(
      <SlotContextMenu
        x={10}
        y={vh - 50}
        items={[item('container:c', { title: 'c', children: [item('kid')] })]}
        ctx={{ viewId: 'v1', slotId: 's1', agentId: null }}
        onClose={vi.fn()}
      />,
    )
    fireEvent.mouseEnter(document.querySelector('[data-slot-menu-container="container:c"]') as HTMLElement)
    const menu = document.querySelector('[role="menu"][data-engram-overlay]') as HTMLElement
    const flyout = () => document.querySelector('[data-slot-menu-flyout="container:c"]') as HTMLElement
    expect(parseInt(menu.style.top, 10)).toBe(vh - 50 - ANCHOR_GAP - 300)
    expect(parseInt(flyout().style.left, 10)).toBe(150)

    size = { menuH: 200, flyW: vw - 100 }
    act(() => {
      setLanguage('en')
    })

    expect(parseInt(menu.style.top, 10)).toBe(vh - 50 - ANCHOR_GAP - 200)
    expect(parseInt(flyout().style.left, 10)).not.toBe(150)
  })
})
