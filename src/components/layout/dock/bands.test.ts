// TRD S21-slot-docking §8-2 `bands` · `dockedTree`: takes the **same golden file** as the shell `layout/bands.rs` (per window the tab bar zone · blocked bands · effective
//   bands · area frames · zone lines — point choosing is in `judge.test.ts`) + tables outside the golden (the counterpart of the same-named tests in the shell `layout/bands/tests.rs` —
//   the two versions do not read one file, so if a rule is changed it is changed by hand together).

import { describe, expect, it } from 'vitest'

import { DOCK_CONSTANTS } from '../../../../src-tauri/bindings/DockConstants'
import type { DockSide } from '../../../../src-tauri/bindings/DockSide'
import type { DropAnchor } from '../../../../src-tauri/bindings/DropAnchor'
import {
  attachShare,
  bandFrame,
  bandThickness,
  canvasZone,
  choose,
  contains,
  landingRect,
  outerStrips,
  slotBand,
  stripZone,
  subtract,
  windowBands,
  zoneInCanvas,
  type BandHit,
} from './bands'
import { computeSlots, dockedTree, framePx, type PxRect } from './dockedTree'
import { parseShape, shapeOf } from './tabDragData'
import { bandsGolden, bandsInputOf, goldenAnchor, goldenCase } from './testing/bandsGolden'

const { SIDE_ORDER, ATTACH_SHARE_MAX } = DOCK_CONSTANTS
const RATIO = bandsGolden.ratio_limits

const px = (x0: number, y0: number, x1: number, y1: number): PxRect => ({ x0, y0, x1, y1 })
const uuid = (n: number): string => `00000000-0000-0000-0000-${n.toString(16).padStart(12, '0')}`
const NAN_SIZE = { w: Number.NaN, h: Number.NaN }

// ── golden ──────────────────────────────────────────────────────────────────────

describe('golden — one file with the shell bands.rs', () => {
  it('head _constants = the generated DOCK_CONSTANTS (red if a constant is changed and the golden is not re-taken)', () => {
    expect(bandsGolden._constants).toEqual(DOCK_CONSTANTS)
  })

  it('there are twelve or more windows (so an empty · truncated file does not pass green)', () => {
    expect(bandsGolden.bands.length).toBeGreaterThanOrEqual(12)
  })

  for (const c of bandsGolden.bands) {
    describe(`golden window: ${c.name}`, () => {
      const input = bandsInputOf(c)
      const wb = windowBands(input)

      it('shape string round trip', () => {
        expect(input.incoming).not.toBeNull()
        expect(shapeOf(input.incoming!)).toBe(c.shape)
      })

      it('tab bar zone', () => {
        expect(input.zone).toEqual(c.zone)
      })

      it('blocked bands — every place where inserting would newly make a zero-area area (outer first · area pre-order · SIDE_ORDER)', () => {
        const whole = px(0, 0, c.canvas.w, c.canvas.h)
        const targets: Array<[DropAnchor, PxRect]> = [
          [{ type: 'outer' }, whole],
          ...wb.frames.map(({ id, frame }): [DropAnchor, PxRect] => [{ type: 'slot', id }, frame]),
        ]
        const blocked = targets.flatMap(([anchor, target]) =>
          SIDE_ORDER.filter(side => {
            const share = attachShare(c.original ?? NAN_SIZE, side, target, RATIO)
            return landingRect(c.layout, anchor, side, input.incoming!, c.canvas, share, RATIO) === null
          }).map(side => ({ anchor: goldenAnchor(anchor), side })),
        )
        expect(blocked).toEqual(c.blocked)
      })

      it('valid bands — order · band · region · share · preview', () => {
        expect(wb.bands.map(b => ({ ...b, anchor: goldenAnchor(b.anchor) }))).toEqual(c.bands)
      })

      it('area frame · zone lines', () => {
        expect(wb.frames.map(({ id, frame }, i) => ({ id, frame, zones: wb.zones[i] }))).toEqual(c.slots)
      })
    })
  }
})

// ── tab bar zone (R2) ────────────────────────────────────────────────────────────

// Drag area = from 4 left of the first tab, width 500 · tab bar row 0..28.
const AREA = { x0: 4, y0: 0, x1: 504, y1: 28 }

describe('tab bar zone', () => {
  it('widens the drag area only up and down by STRIP_ZONE_PX (the_zone_is_the_drag_area_widened_15_up_and_down_only)', () => {
    expect(stripZone(AREA)).toEqual({ x0: 4, y0: -15, x1: 504, y1: 43 })
  })

  it('half-open on both axes · NaN is outside (the_zone_is_half_open_on_both_axes)', () => {
    const z = stripZone(AREA)
    const table: Array<[number, number, boolean]> = [
      [4, 10, true],
      [3.999, 10, false],
      [503.999, 10, true],
      [504, 10, false],
      [200, -15, true],
      [200, -15.001, false],
      [200, 42.999, true],
      [200, 43, false],
      [Number.NaN, 10, false],
    ]
    for (const [x, y, inside] of table) expect(contains(z, { x, y }), `(${x}, ${y})`).toBe(inside)
  })

  it('moving to canvas coordinates widens to integer bounds (the_zone_in_the_canvas_widens_to_whole_pixels)', () => {
    const zone = stripZone({ x0: 0, y0: 0, x1: 600.5, y1: 28.125 })
    expect(zoneInCanvas(zone, { x: 0.25, y: 28 })).toEqual({ x0: -1, y0: -43, x1: 601, y1: 16 })
    expect(canvasZone(null, { x: 0, y: 0 })).toBeNull()
  })
})

// ── band thickness · frame (R3) ─────────────────────────────────────────────────────────

describe('band thickness · band frame', () => {
  it('BAND_SHARE of the area length · .5 rounds up · minimum BAND_MIN_PX · not thicker than the area (AC39 — 50-wide area)', () => {
    const table: Array<[number, number]> = [
      [600, 60],
      [1205, 121],
      [125, 13],
      [135, 14],
      [124, 12],
      [115, 12],
      [50, 12],
      [8, 8],
      [0, 0],
    ]
    for (const [len, want] of table) expect(bandThickness(len), `area length ${len}`).toBe(want)
  })

  it('equals integer rounding at every length (the_band_thickness_is_exact_integer_rounding_for_every_length)', () => {
    for (let len = 0; len <= 20000; len += 1) {
      const want = Math.min(Math.max(Math.floor((len + 5) / 10), DOCK_CONSTANTS.BAND_MIN_PX), len)
      if (bandThickness(len) !== want) expect(bandThickness(len), `area length ${len}`).toBe(want)
    }
  })

  it('the band of an area touching the canvas edge stands inside the outer band · a blocked side does not push (a_slot_on_the_canvas_edge_moves_its_band_inside_the_outer_band)', () => {
    const canvas = { w: 882, h: 544 }
    const frame = px(0, 272, 882, 544)
    const room = bandFrame(frame, canvas, SIDE_ORDER)
    expect(room).toEqual(px(20, 272, 862, 524))
    expect(slotBand(frame, room, 'Bottom')).toEqual(px(20, 497, 862, 524))
    expect(slotBand(frame, room, 'Left')).toEqual(px(20, 272, 108, 524))
    expect(slotBand(frame, room, 'Top')).toEqual(px(20, 272, 862, 299))
    expect(bandFrame(frame, canvas, ['Left', 'Right'])).toEqual(px(20, 272, 862, 544))
    const sliver = bandFrame(px(0, 0, 15, 544), canvas, SIDE_ORDER)
    expect(sliver.x0 >= sliver.x1 || sliver.y0 >= sliver.y1).toBe(true)
  })

  it('outer bands give to the canvas on a small canvas (outer_strips_are_20_px_and_shrink_to_a_tiny_canvas)', () => {
    const tiny = px(0, 0, 12, 10)
    expect(outerStrips({ w: 12, h: 10 })).toEqual([
      ['Left', tiny],
      ['Right', tiny],
      ['Top', tiny],
      ['Bottom', tiny],
    ])
  })

  it('rectangle subtraction leaves only the uncovered places (subtracting_keeps_exactly_the_uncovered_pixels)', () => {
    const r = px(10, 10, 30, 30)
    expect(subtract(r, px(40, 40, 50, 50))).toEqual([r])
    expect(subtract(r, px(30, 10, 40, 30))).toEqual([r])
    expect(subtract(r, px(0, 0, 100, 100))).toEqual([])
    expect(subtract(r, px(0, 0, 100, 15))).toEqual([px(10, 15, 30, 30)])
    expect(subtract(r, px(15, 15, 25, 25))).toEqual([
      px(10, 10, 30, 15),
      px(10, 25, 30, 30),
      px(10, 15, 15, 25),
      px(25, 15, 30, 25),
    ])
  })
})

// ── attach share (R7) ──────────────────────────────────────────────────────────────

describe('attach share', () => {
  const len = (w: number, h: number, side: DockSide, t: PxRect): number =>
    attachShare({ w, h }, side, t, RATIO) * (side === 'Left' || side === 'Right' ? t.x1 - t.x0 : t.y1 - t.y0)

  it('cuts the original length at half of the area — AC2 · AC39 · AC44', () => {
    const table: Array<[string, number, number, DockSide, PxRect, number]> = [
      ['AC2 — original 400 on a 1200 area', 400, 300, 'Right', px(0, 0, 1200, 800), 400],
      ['AC2 — original 400 on a 600 area = half', 400, 300, 'Right', px(600, 0, 1200, 800), 300],
      ['AC39 — 200 on 600', 200, 999, 'Right', px(0, 0, 600, 800), 200],
      ['AC39 — 400 on 600', 400, 999, 'Left', px(0, 0, 600, 800), 300],
      ['AC39 — 200 on a small area 50', 200, 999, 'Right', px(0, 0, 50, 800), 25],
      ['AC39 — 20 on a small area 50', 20, 999, 'Left', px(0, 0, 50, 800), 20],
      ['AC44 — outer top band · height 300', 999, 300, 'Top', px(0, 0, 1200, 800), 300],
      ['AC44 — half if the layout height is 500', 999, 300, 'Top', px(0, 0, 1200, 500), 250],
      ['bottom = height', 999, 120, 'Bottom', px(0, 100, 400, 700), 120],
    ]
    for (const [why, w, h, side, t, want] of table) expect(len(w, h, side, t), why).toBeCloseTo(want, 9)
  })

  it('edge inputs — area 0 · NaN (size unknown) · +∞ = upper bound · 0 · negative · −∞ = lower bound · a flipped area = length 0 (edge_inputs_go_to_the_documented_ends)', () => {
    const t = px(0, 0, 600, 400)
    expect(attachShare({ w: 100, h: 100 }, 'Left', px(0, 0, 0, 400), RATIO)).toBe(ATTACH_SHARE_MAX)
    expect(attachShare({ w: 100, h: 100 }, 'Left', px(9, 9, 3, 3), RATIO)).toBe(ATTACH_SHARE_MAX)
    expect(attachShare({ w: Number.NaN, h: 0 }, 'Left', t, RATIO)).toBe(ATTACH_SHARE_MAX)
    expect(attachShare({ w: Number.POSITIVE_INFINITY, h: 0 }, 'Left', t, RATIO)).toBe(ATTACH_SHARE_MAX)
    expect(attachShare({ w: 0, h: 0 }, 'Left', t, RATIO)).toBe(RATIO.min)
    expect(attachShare({ w: -1, h: 0 }, 'Left', t, RATIO)).toBe(RATIO.min)
    expect(attachShare({ w: Number.NEGATIVE_INFINITY, h: 0 }, 'Left', t, RATIO)).toBe(RATIO.min)
    expect(attachShare({ w: 3, h: 3 }, 'Left', px(0, 0, 1000, 600), RATIO)).toBe(RATIO.min)
  })

  it('only the split-direction length counts (only_the_split_axis_length_counts)', () => {
    const t = px(0, 0, 1000, 600)
    expect(attachShare({ w: 200, h: 10 }, 'Left', t, RATIO)).toBe(attachShare({ w: 200, h: 5000 }, 'Left', t, RATIO))
  })
})

// ── a single window (outside the golden) ─────────────────────────────────────────────────────────

describe('one window', () => {
  it('with no shape field it counts as one area', () => {
    const input = bandsInputOf(goldenCase('left-right halves'))
    expect(windowBands({ ...input, incoming: null })).toEqual(windowBands({ ...input, incoming: parseShape('l') }))
  })

  it('if the original size is unknown all shares are the upper bound', () => {
    const wb = windowBands(bandsInputOf(goldenCase('an unknown original size')))
    expect(wb.bands.length).toBeGreaterThan(0)
    for (const b of wb.bands) expect(b.share).toBe(ATTACH_SHARE_MAX)
  })

  it('validity (R8 · AC36) — a band that would make zero area is not in the list, and the painted list = the picked list', () => {
    const c = goldenCase('a dragged tab that would leave a zero-area slot on the sides')
    const wb = windowBands(bandsInputOf(c))
    expect(c.blocked.length).toBeGreaterThan(0)
    const listed = new Set(wb.bands.map(b => `${goldenAnchor(b.anchor)}/${b.side}`))
    for (const b of c.blocked) expect(listed.has(`${b.anchor}/${b.side}`), `${b.anchor} ${b.side}`).toBe(false)
    expect(listed.has(`${uuid(1)}/Top`)).toBe(true)
    // Pointing at the right band position of an area leaves nothing to choose — dropping gives a new window (R8 · AC15).
    expect(choose(wb.bands, { x: 1150, y: 400 })).toBeNull()
  })

  it('the area band under a blocked outer top band is painted and picked in that place (AC36 last)', () => {
    const wb = windowBands(bandsInputOf(goldenCase('a blocked outer top band over a deep chain')))
    expect(wb.bands.some(b => b.anchor.type === 'outer' && b.side === 'Top')).toBe(false)
    const top = wb.bands.findIndex(b => b.anchor.type === 'slot' && b.anchor.id === uuid(1) && b.side === 'Top')
    expect(top).toBeGreaterThanOrEqual(0)
    expect(wb.bands[top].band).toEqual(px(20, 0, 1180, 40))
    expect(wb.bands[top].areas).toContainEqual(px(20, 15, 1180, 40))
    expect(choose(wb.bands, { x: 300, y: 20 })).toBe(top)
  })

  it('preview = where the areas inserted after the drop will be drawn (the_landing_is_exactly_where_the_incoming_tab_will_be_drawn)', () => {
    const c = goldenCase('three slots, a split dragged tab')
    const input = bandsInputOf(c)
    const ids = computeSlots(input.incoming!, RATIO).map(r => r.slot_id)
    for (const b of windowBands(input).bands) {
      const result = dockedTree(c.layout, b.anchor, b.side, input.incoming!, b.share, RATIO)
      if (!result.ok) throw new Error(result.error)
      const drawn = computeSlots(result.tree, RATIO)
        .filter(r => ids.includes(r.slot_id))
        .map(r => framePx(c.canvas, r))
      const hull = drawn.reduce((a, r) => px(Math.min(a.x0, r.x0), Math.min(a.y0, r.y0), Math.max(a.x1, r.x1), Math.max(a.y1, r.y1)))
      expect(b.landing, `${goldenAnchor(b.anchor)} ${b.side}`).toEqual(hull)
    }
  })

  it('ties go to the one earlier in SIDE_ORDER · outer bands come before area bands', () => {
    const band = (anchor: DropAnchor, side: DockSide, r: PxRect): BandHit => ({
      anchor,
      side,
      band: r,
      share: ATTACH_SHARE_MAX,
      landing: r,
      areas: [r],
    })
    const slot: DropAnchor = { type: 'slot', id: 'a' }
    // Corner (5, 5) — the left · top bands have the same normalized distance (5 ÷ 20).
    const left = band(slot, 'Left', px(0, 0, 20, 100))
    const top = band(slot, 'Top', px(0, 0, 100, 20))
    expect(choose([left, top], { x: 5, y: 5 })).toBe(0)
    expect(choose([top, left], { x: 5, y: 5 })).toBe(0)
    const outer = band({ type: 'outer' }, 'Bottom', px(0, 0, 100, 100))
    expect(choose([left, outer], { x: 1, y: 1 })).toBe(1)
    expect(choose([left], { x: 50, y: 50 })).toBeNull()
  })
})
