// TRD S21-slot-docking §8-2 `judge`: re-emits every point of the shell golden (tab bar zone · chosen band · zone lines — `points` of `bands_golden.json`) as decisions
//   + tables outside the golden (order · tab bar zone boundary · source window = Chrome `insertionIndex` · other window = `dropIndex` · the dragged tab's own canvas
//   · blocked bands).

import { describe, expect, it } from 'vitest'

import { canvasZone, windowBands } from './bands'
import { judge, type JudgeStrip, type JudgeWindow } from './judge'
import { bandsGolden, bandsInputOf, goldenCase, type GoldenBandsCase } from './testing/bandsGolden'
import { idealBounds } from '../tabStrip/reorderGeometry'

const NO_HEADS: JudgeStrip = { role: 'other', left: 0, bounds: { tabs: [], visible: [], lastVisible: -1 } }

function windowOf(c: GoldenBandsCase, strip: JudgeStrip = NO_HEADS): JudgeWindow {
  const input = bandsInputOf(c)
  return {
    canvasOrigin: c.canvas_origin,
    tabBar: input.zone === null ? null : { zone: input.zone, strip },
    bands: windowBands(input),
  }
}

describe('golden points — one file with the shell bands.rs', () => {
  it('there are over a thousand points (so an empty · truncated file does not pass green)', () => {
    expect(bandsGolden.bands.reduce((n, c) => n + c.points.length, 0)).toBeGreaterThan(1000)
  })

  for (const c of bandsGolden.bands) {
    it(`golden window: ${c.name} — ${c.points.length} points`, () => {
      expect(c.points.length).toBeGreaterThan(0)
      const w = windowOf(c)
      const got = c.points.map(([x, y]) => {
        // Golden points are canvas coordinates — the decision takes viewport points (a quarter-px grid + integer origin make the round trip exact).
        const j = judge(w, { x: x + c.canvas_origin.x, y: y + c.canvas_origin.y })
        return [x, y, j.type === 'strip' ? 'strip' : j.type === 'band' ? j.band : null, j.type === 'strip' ? null : j.zones]
      })
      expect(got).toEqual(c.points)
    })
  }
})

// 「left-right halves」 — canvas 1200 × 800 · origin (0, 28) · tab drag area (0, 0)–(600, 28) · area 1 | area 2.
const HALVES = goldenCase('left-right halves')

describe('order — tab bar zone > outer band > area band > otherwise', () => {
  const w = windowOf(HALVES)
  const at = (x: number, y: number) => judge(w, { x, y })
  const bandAt = (x: number, y: number) => {
    const j = at(x, y)
    if (j.type !== 'band') return null
    const b = w.bands!.bands[j.band]
    return `${b.anchor.type === 'outer' ? 'outer' : 'slot'}/${b.side}`
  }

  it('the tab bar zone covers the outer top band · area band', () => {
    expect(at(300, 40).type).toBe('strip')
    expect(at(300, 42.75).type).toBe('strip')
  })

  it('outside the zone outer top band → the area top band beneath → area center (zone lines only) → outside the canvas', () => {
    expect(bandAt(300, 43)).toBe('outer/Top')
    expect(bandAt(900, 30)).toBe('outer/Top')
    expect(bandAt(300, 48)).toBe('slot/Top')
    expect(at(300, 428)).toEqual({ type: 'elsewhere', zones: 0 })
    expect(at(300, 900)).toEqual({ type: 'elsewhere', zones: null })
  })

  it('zone lines are not drawn over an outer band', () => {
    const j = at(5, 400)
    expect(j.type === 'band' && j.zones).toBe(null)
  })
})

describe('tab bar zone boundary', () => {
  const w = windowOf(HALVES)
  const isStrip = (x: number, y: number): boolean => judge(w, { x, y }).type === 'strip'

  it('vertical = the background up and down by STRIP_ZONE_PX (half-open) · horizontal = the background as is (here background = the tab drag area)', () => {
    expect(isStrip(100, -15)).toBe(true)
    expect(isStrip(100, -15.25)).toBe(false)
    expect(isStrip(100, 42.75)).toBe(true)
    expect(isStrip(100, 43)).toBe(false)
    expect(isStrip(0, 10)).toBe(true)
    expect(isStrip(599.75, 10)).toBe(true)
    expect(isStrip(600, 10)).toBe(false)
  })

  it('without the tab bar there is no zone', () => {
    expect(judge({ ...w, tabBar: null }, { x: 300, y: 10 }).type).toBe('elsewhere')
  })
})

describe('tab bar position', () => {
  // Three tabs · 160 each · drag area left 4.
  const bounds = idealBounds({ widths: [160, 160, 160], visible: [true, true, true] }, 0)
  const zone = canvasZone({ x0: 4, y0: 0, x1: 604, y1: 28 }, HALVES.canvas_origin)!
  const withStrip = (strip: JudgeStrip): JudgeWindow => ({ ...windowOf(HALVES), tabBar: { zone, strip } })

  it('source window — Chrome insertionIndex (the dragged head\'s left = draggedLeft)', () => {
    const w = withStrip({ role: 'source', left: 4, bounds, dragged: 0, grabOffset: 80, available: 600 })
    // Cursor tab bar x 400 − grabbed 80 = left 320 → after the third (the end of the list without the dragged tab).
    expect(judge(w, { x: 404, y: 10 })).toEqual({ type: 'strip', index: 2, left: 320, marker: null })
    expect(judge(w, { x: 84, y: 10 })).toEqual({ type: 'strip', index: 0, left: 0, marker: null })
    // Even dragging into the empty space after the tabs (480..600), it stops at the last position (320).
    expect(judge(w, { x: 584, y: 10 })).toEqual({ type: 'strip', index: 2, left: 320, marker: null })
  })

  it('another window — dropIndex · vertical line (an x equal to the center = right)', () => {
    const w = withStrip({ role: 'other', left: 4, bounds })
    expect(judge(w, { x: 254, y: 10 })).toEqual({ type: 'strip', index: 2, left: null, marker: 320 })
    expect(judge(w, { x: 244, y: 10 })).toEqual({ type: 'strip', index: 1, left: null, marker: 160 })
    expect(judge(w, { x: 4, y: 10 })).toEqual({ type: 'strip', index: 0, left: null, marker: 0 })
  })

  for (const dragged of [-1, bounds.tabs.length]) {
    it(`for the source window, the dragged tab index ${dragged} is outside the list (left the tab bar) — does not throw and takes the other-window path (dropIndex · vertical line)`, () => {
      const w = withStrip({ role: 'source', left: 4, bounds, dragged, grabOffset: 80, available: 600 })
      expect(judge(w, { x: 254, y: 10 })).toEqual({ type: 'strip', index: 2, left: null, marker: 320 })
      expect(judge(w, { x: 4, y: 10 })).toEqual({ type: 'strip', index: 0, left: null, marker: 0 })
    })
  }
})

describe('where there is no band', () => {
  it('the canvas where the dragged tab itself is visible (SelfTarget) has neither band nor zone lines · the tab bar as is', () => {
    const w: JudgeWindow = { ...windowOf(HALVES), bands: null }
    expect(judge(w, { x: 5, y: 400 })).toEqual({ type: 'elsewhere', zones: null })
    expect(judge(w, { x: 300, y: 428 })).toEqual({ type: 'elsewhere', zones: null })
    expect(judge(w, { x: 300, y: 10 }).type).toBe('strip')
  })

  it('a blocked band is not picked — dropping gives a new window (R8 · AC15)', () => {
    const w = windowOf(goldenCase('a dragged tab that would leave a zero-area slot on the sides'))
    expect(judge(w, { x: 1150, y: 428 }).type).toBe('elsewhere')
  })
})
