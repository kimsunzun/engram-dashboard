// TRD S21-slot-docking §8-2 `dragSize`: the new window size table of the **same golden file** as the shell `layout/drag_size.rs` + Chrome C5 order · no restore size
//   · single-tab popout · `grab` (r × W1 · single tab → the pressed point).

import { describe, expect, it } from 'vitest'

import { DOCK_CONSTANTS } from '../../../../src-tauri/bindings/DockConstants'
import { TAB_STANDARD_W, TAB_MIN_ACTIVE_W } from '../tabStrip/layout'
import { dragGrab, draggedSize, dragSizes, type GrabInput, type SourceWindow } from './dragSize'
import { bandsGolden } from './testing/bandsGolden'

describe('golden — one file with the shell drag_size.rs', () => {
  it('there are eight or more size lines', () => {
    expect(bandsGolden.drag_size.length).toBeGreaterThanOrEqual(8)
  })

  for (const c of bandsGolden.drag_size) {
    it(`golden: ${c.name}`, () => {
      const { name: _, size, original, ...input } = c
      expect(dragSizes(input)).toEqual({ size, original })
    })
  }
})

const FRAME = { t: 31, r: 8, b: 8, l: 8 }
const BASE: SourceWindow = {
  restored: { w: 1000, h: 700 },
  maximized: false,
  insets: FRAME,
  viewport: { w: 1000, h: 700 },
  work: { w: 1920, h: 1032 },
  chrome: { w: 0, h: 36 },
  single: false,
}

describe('new window size (R22 · Chrome C5)', () => {
  it('a side exceeding the work area becomes work area − margins on both sides → then the half-maximized lower bound wins', () => {
    const inset = 2 * DOCK_CONSTANTS.MAXIMIZED_INSET_DIP
    expect(draggedSize({ w: 3000, h: 3000 }, null, { w: 800, h: 600 })).toEqual({ w: 800 - inset, h: 600 - inset })
    expect(draggedSize({ w: 3000, h: 3000 }, { w: 2560, h: 1369 }, { w: 800, h: 600 })).toEqual({ w: 1280, h: 684.5 })
    expect(draggedSize({ w: 5, h: 5 }, null, { w: 4, h: 4 })).toEqual({ w: 0, h: 0 })
  })

  it('if the restore size is unknown from the current size · original size = new window − chrome', () => {
    expect(dragSizes({ ...BASE, restored: null, viewport: { w: 900, h: 500 } })).toEqual({
      size: { w: 900, h: 500 },
      original: { w: 900, h: 464 },
    })
  })

  it('popout with only one tab = no size (that window is moved) · original size = (restore if maximized · otherwise current) − chrome', () => {
    expect(dragSizes({ ...BASE, single: true, viewport: { w: 640, h: 480 } })).toEqual({
      size: null,
      original: { w: 640, h: 444 },
    })
    expect(dragSizes({ ...BASE, single: true, maximized: true, viewport: { w: 1920, h: 1001 } })).toEqual({
      size: null,
      original: { w: 1000, h: 664 },
    })
  })

  it('the frame is the inset of window:frame — compared after subtracting the frame from the work area', () => {
    const r = dragSizes({ ...BASE, restored: { w: 1910, h: 1000 } })
    expect(r.size).toEqual({ w: 1920 - 16 - 2 * DOCK_CONSTANTS.MAXIMIZED_INSET_DIP, h: 1032 - 39 - 2 * DOCK_CONSTANTS.MAXIMIZED_INSET_DIP })
  })
})

describe('grab — where the cursor lands in the new window', () => {
  // Press the middle of the dragged head 100..200 (r = 1/2) · drag area left 4.
  const G: GrabInput = {
    pressed: { x: 150, y: 14 },
    head: { x0: 100, x1: 200 },
    stripLeft: 4,
    headRoom: 1000,
    single: false,
    maximized: false,
  }

  it('= (tab bar left + r × W1, pressed y) — W1 = the head width of one tab in the new window', () => {
    expect(dragGrab(G)).toEqual({ x: 4 + TAB_STANDARD_W / 2, y: 14 })
    expect(dragGrab({ ...G, headRoom: null })).toEqual({ x: 4 + TAB_STANDARD_W / 2, y: 14 })
    expect(dragGrab({ ...G, headRoom: TAB_MIN_ACTIVE_W })).toEqual({ x: 4 + TAB_MIN_ACTIVE_W / 2, y: 14 })
  })

  it('popout with only one tab = the pressed point · if maximized it counts with W1 of the restored window width', () => {
    expect(dragGrab({ ...G, single: true })).toEqual({ x: 150, y: 14 })
    expect(dragGrab({ ...G, single: true, maximized: true })).toEqual({ x: 4 + TAB_STANDARD_W / 2, y: 14 })
  })
})
