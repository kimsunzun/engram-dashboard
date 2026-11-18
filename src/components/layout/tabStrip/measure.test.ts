// measure unit test — measured values (row rectangle · left of the first tab · usable width = row width − `+` width) and the ideal bounds · base of the tab bar zone.

import { describe, expect, it } from 'vitest'

import { tabStripLayout } from './layout'
import { measureStrip, stripBounds, stripZoneArea, tabsEnd } from './measure'
import { idealBounds } from './reorderGeometry'

function box(left: number, top: number, width: number, height: number): Element {
  const el = document.createElement('div')
  el.getBoundingClientRect = () =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect
  return el
}

describe('measureStrip', () => {
  it('row rectangle · first tab left = row left · usable width = row width − + width', () => {
    const m = measureStrip({ row: box(100, 40, 700, 28), add: box(760, 40, 40, 28) })
    expect(m).toEqual({
      row: { x0: 100, y0: 40, x1: 800, y1: 68 },
      left: 100,
      available: 660,
      addWidth: 40,
    })
  })

  it('with no + button the row width as is · + width 0', () => {
    const m = measureStrip({ row: box(0, 0, 500, 28), add: null })
    expect(m.available).toBe(500)
    expect(m.addWidth).toBe(0)
  })
})

// User decision 2026-10-26 (T-66) — only where tabs are (first tab ~ `+`) is the tab bar zone.
describe('stripZoneArea — tab bar zone background = first tab left ~ + right end × title row (the tab bar row if absent)', () => {
  // Tab bar row x 100..800 · y 40..68 · + box width 26.
  const m = measureStrip({ row: box(100, 40, 700, 28), add: box(0, 40, 26, 28) })
  const three = idealBounds({ widths: [160, 160, 160], visible: [true, true, true] })
  const TITLE = { top: 34, bottom: 68 }

  it('+ position = right end of the last visible head · 0 if there is no visible head', () => {
    expect(tabsEnd(three)).toBe(480)
    expect(tabsEnd(idealBounds({ widths: [52, 32, 32], visible: [true, true, false] }))).toBe(84)
    expect(tabsEnd(idealBounds({ widths: [], visible: [] }))).toBe(0)
  })

  it('horizontal = first tab ~ + right end — the empty place to its right · drag spot · number marker · window buttons are outside the zone · vertical = tab bar row', () => {
    expect(stripZoneArea(m, three, null)).toEqual({ x0: 100, y0: 40, x1: 606, y1: 68 })
  })

  it('if inside the title row, only vertical is the title row (including the top margin) — horizontal is the same', () => {
    expect(stripZoneArea(m, three, TITLE)).toEqual({ x0: 100, y0: 34, x1: 606, y1: 68 })
  })

  it('hidden heads are not counted — up to the visible end where + stands', () => {
    const narrow = idealBounds({ widths: [52, 32, 32, 32], visible: [true, true, false, false] })
    expect(stripZoneArea(m, narrow, null).x1).toBe(100 + 84 + 26)
  })

  it('with no tabs only + · with no + either width 0 (no zone)', () => {
    const none = idealBounds({ widths: [], visible: [] })
    expect(stripZoneArea(m, none, null)).toEqual({ x0: 100, y0: 40, x1: 126, y1: 68 })
    const bare = measureStrip({ row: box(100, 40, 700, 28), add: null })
    expect(stripZoneArea(bare, none, null)).toEqual({ x0: 100, y0: 40, x1: 100, y1: 68 })
  })

  it('with no + up to the right end of the last visible head', () => {
    const bare = measureStrip({ row: box(100, 40, 700, 28), add: null })
    expect(stripZoneArea(bare, three, null).x1).toBe(580)
  })

  it('clips at the right end of the tab bar row (when heads are wider than the row because they could not be measured — where the tab bar clips horizontally and it cannot be seen) · row width 0 = width 0', () => {
    const short = measureStrip({ row: box(100, 40, 300, 28), add: box(0, 40, 26, 28) })
    expect(stripZoneArea(short, three, null).x1).toBe(400)
    const zero = measureStrip({ row: box(100, 40, 0, 28), add: box(0, 40, 26, 28) })
    expect(stripZoneArea(zero, idealBounds({ widths: [], visible: [] }), null).x1).toBe(100)
  })
})

describe('stripBounds', () => {
  it('ideal bounds = layout.ts width · accumulation of hiding (closing mode frozen width · including the dragged tab)', () => {
    expect(stripBounds({ available: 300, count: 5, active: 1, dragged: 4, widthCap: 250 })).toEqual(
      idealBounds(tabStripLayout(300, 5, 1, 250), 4),
    )
  })
})
