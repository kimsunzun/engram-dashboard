// TRD S21-slot-docking §8-2 `bands` · `dockedTree`: 셸 `layout/bands.rs` 와 **같은 골든 파일**(창마다 탭바 구역 · 막힌 띠 · 유효
//   띠 · 칸 틀 · 구역 선 — 점 고르기는 `judge.test.ts`)을 먹는다 + 골든 밖 표(셸 `layout/bands/tests.rs` 의 같은 이름 시험의 짝 —
//   두 판이 한 파일을 먹지 않으므로 규칙을 고치면 손으로 함께 고친다).

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

// ── 골든 ──────────────────────────────────────────────────────────────────────

describe('골든 — 셸 bands.rs 와 한 파일', () => {
  it('머리 _constants = 생성물 DOCK_CONSTANTS(상수를 바꾸고 골든을 다시 뜨지 않으면 빨갛다)', () => {
    expect(bandsGolden._constants).toEqual(DOCK_CONSTANTS)
  })

  it('창이 열둘 이상이다(빈 파일 · 잘린 파일이 초록으로 통과하지 않게)', () => {
    expect(bandsGolden.bands.length).toBeGreaterThanOrEqual(12)
  })

  for (const c of bandsGolden.bands) {
    describe(`골든 창: ${c.name}`, () => {
      const input = bandsInputOf(c)
      const wb = windowBands(input)

      it('모양 문자열 왕복', () => {
        expect(input.incoming).not.toBeNull()
        expect(shapeOf(input.incoming!)).toBe(c.shape)
      })

      it('탭바 구역', () => {
        expect(input.zone).toEqual(c.zone)
      })

      it('막힌 띠 — 끼우면 새로 면적 0 칸이 생기는 자리 전부(바깥 먼저 · 칸 전위 순 · SIDE_ORDER)', () => {
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

      it('유효 띠 — 차례 · 띠 · 영역 · 몫 · 미리보기', () => {
        expect(wb.bands.map(b => ({ ...b, anchor: goldenAnchor(b.anchor) }))).toEqual(c.bands)
      })

      it('칸 틀 · 구역 선', () => {
        expect(wb.frames.map(({ id, frame }, i) => ({ id, frame, zones: wb.zones[i] }))).toEqual(c.slots)
      })
    })
  }
})

// ── 탭바 구역 (R2) ────────────────────────────────────────────────────────────

// 끌기 영역 = 첫 탭 왼쪽 4 부터 폭 500 · 탭바 줄 0..28.
const AREA = { x0: 4, y0: 0, x1: 504, y1: 28 }

describe('탭바 구역', () => {
  it('끌기 영역을 위아래로만 STRIP_ZONE_PX 넓힌다(the_zone_is_the_drag_area_widened_15_up_and_down_only)', () => {
    expect(stripZone(AREA)).toEqual({ x0: 4, y0: -15, x1: 504, y1: 43 })
  })

  it('두 축 모두 반열림 · NaN 은 밖(the_zone_is_half_open_on_both_axes)', () => {
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

  it('캔버스 좌표로 옮기며 정수 경계로 넓힌다(the_zone_in_the_canvas_widens_to_whole_pixels)', () => {
    const zone = stripZone({ x0: 0, y0: 0, x1: 600.5, y1: 28.125 })
    expect(zoneInCanvas(zone, { x: 0.25, y: 28 })).toEqual({ x0: -1, y0: -43, x1: 601, y1: 16 })
    expect(canvasZone(null, { x: 0, y: 0 })).toBeNull()
  })
})

// ── 띠 두께 · 틀 (R3) ─────────────────────────────────────────────────────────

describe('띠 두께 · 띠 틀', () => {
  it('칸 길이의 BAND_SHARE · .5 는 위로 · 최소 BAND_MIN_PX · 칸보다 두껍지 않다(AC39 — 폭 50 칸)', () => {
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
    for (const [len, want] of table) expect(bandThickness(len), `칸 길이 ${len}`).toBe(want)
  })

  it('모든 길이에서 정수 반올림과 같다(the_band_thickness_is_exact_integer_rounding_for_every_length)', () => {
    for (let len = 0; len <= 20000; len += 1) {
      const want = Math.min(Math.max(Math.floor((len + 5) / 10), DOCK_CONSTANTS.BAND_MIN_PX), len)
      if (bandThickness(len) !== want) expect(bandThickness(len), `칸 길이 ${len}`).toBe(want)
    }
  })

  it('캔버스 변에 닿은 칸의 띠는 바깥 띠 안쪽에 선다 · 막힌 변은 밀지 않는다(a_slot_on_the_canvas_edge_moves_its_band_inside_the_outer_band)', () => {
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

  it('바깥 띠는 작은 캔버스에서 캔버스로 준다(outer_strips_are_20_px_and_shrink_to_a_tiny_canvas)', () => {
    const tiny = px(0, 0, 12, 10)
    expect(outerStrips({ w: 12, h: 10 })).toEqual([
      ['Left', tiny],
      ['Right', tiny],
      ['Top', tiny],
      ['Bottom', tiny],
    ])
  })

  it('사각형 빼기는 덮이지 않은 곳만 남긴다(subtracting_keeps_exactly_the_uncovered_pixels)', () => {
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

// ── 붙는 몫 (R7) ──────────────────────────────────────────────────────────────

describe('붙는 몫', () => {
  const len = (w: number, h: number, side: DockSide, t: PxRect): number =>
    attachShare({ w, h }, side, t, RATIO) * (side === 'Left' || side === 'Right' ? t.x1 - t.x0 : t.y1 - t.y0)

  it('원래 길이를 칸 절반에서 자른다 — AC2 · AC39 · AC44', () => {
    const table: Array<[string, number, number, DockSide, PxRect, number]> = [
      ['AC2 — 칸 1200 에 원래 400', 400, 300, 'Right', px(0, 0, 1200, 800), 400],
      ['AC2 — 칸 600 에 원래 400 = 절반', 400, 300, 'Right', px(600, 0, 1200, 800), 300],
      ['AC39 — 600 에 200', 200, 999, 'Right', px(0, 0, 600, 800), 200],
      ['AC39 — 600 에 400', 400, 999, 'Left', px(0, 0, 600, 800), 300],
      ['AC39 — 작은 칸 50 에 200', 200, 999, 'Right', px(0, 0, 50, 800), 25],
      ['AC39 — 작은 칸 50 에 20', 20, 999, 'Left', px(0, 0, 50, 800), 20],
      ['AC44 — 바깥 위 띠 · 높이 300', 999, 300, 'Top', px(0, 0, 1200, 800), 300],
      ['AC44 — 레이아웃 높이 500 이면 절반', 999, 300, 'Top', px(0, 0, 1200, 500), 250],
      ['아래 = 높이', 999, 120, 'Bottom', px(0, 100, 400, 700), 120],
    ]
    for (const [why, w, h, side, t, want] of table) expect(len(w, h, side, t), why).toBeCloseTo(want, 9)
  })

  it('끝 입력 — 칸 0 · NaN(크기 모름) · +∞ = 상한 · 0 · 음수 · −∞ = 하한 · 뒤집힌 칸 = 길이 0(edge_inputs_go_to_the_documented_ends)', () => {
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

  it('나누는 방향 길이만 센다(only_the_split_axis_length_counts)', () => {
    const t = px(0, 0, 1000, 600)
    expect(attachShare({ w: 200, h: 10 }, 'Left', t, RATIO)).toBe(attachShare({ w: 200, h: 5000 }, 'Left', t, RATIO))
  })
})

// ── 창 하나 (골든 밖) ─────────────────────────────────────────────────────────

describe('창 하나', () => {
  it('모양 칸이 없으면 칸 하나로 친다', () => {
    const input = bandsInputOf(goldenCase('left-right halves'))
    expect(windowBands({ ...input, incoming: null })).toEqual(windowBands({ ...input, incoming: parseShape('l') }))
  })

  it('원래 크기를 모르면 몫이 전부 상한이다', () => {
    const wb = windowBands(bandsInputOf(goldenCase('an unknown original size')))
    expect(wb.bands.length).toBeGreaterThan(0)
    for (const b of wb.bands) expect(b.share).toBe(ATTACH_SHARE_MAX)
  })

  it('유효성(R8 · AC36) — 면적 0 이 생길 띠는 목록에 없고, 칠하는 목록 = 고르는 목록이다', () => {
    const c = goldenCase('a dragged tab that would leave a zero-area slot on the sides')
    const wb = windowBands(bandsInputOf(c))
    expect(c.blocked.length).toBeGreaterThan(0)
    const listed = new Set(wb.bands.map(b => `${goldenAnchor(b.anchor)}/${b.side}`))
    for (const b of c.blocked) expect(listed.has(`${b.anchor}/${b.side}`), `${b.anchor} ${b.side}`).toBe(false)
    expect(listed.has(`${uuid(1)}/Top`)).toBe(true)
    // 칸 오른쪽 띠 자리를 짚어도 고를 것이 없다 — 놓으면 새 창(R8 · AC15).
    expect(choose(wb.bands, { x: 1150, y: 400 })).toBeNull()
  })

  it('막힌 바깥 위 띠 밑의 칸 띠가 그 자리에서 칠해지고 골린다(AC36 마지막)', () => {
    const wb = windowBands(bandsInputOf(goldenCase('a blocked outer top band over a deep chain')))
    expect(wb.bands.some(b => b.anchor.type === 'outer' && b.side === 'Top')).toBe(false)
    const top = wb.bands.findIndex(b => b.anchor.type === 'slot' && b.anchor.id === uuid(1) && b.side === 'Top')
    expect(top).toBeGreaterThanOrEqual(0)
    expect(wb.bands[top].band).toEqual(px(20, 0, 1180, 40))
    expect(wb.bands[top].areas).toContainEqual(px(20, 15, 1180, 40))
    expect(choose(wb.bands, { x: 300, y: 20 })).toBe(top)
  })

  it('미리보기 = 놓은 뒤 끼운 칸들이 그려질 자리(the_landing_is_exactly_where_the_incoming_tab_will_be_drawn)', () => {
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

  it('동률은 SIDE_ORDER 앞의 것 · 바깥 띠가 칸 띠보다 앞선다', () => {
    const band = (anchor: DropAnchor, side: DockSide, r: PxRect): BandHit => ({
      anchor,
      side,
      band: r,
      share: ATTACH_SHARE_MAX,
      landing: r,
      areas: [r],
    })
    const slot: DropAnchor = { type: 'slot', id: 'a' }
    // 모서리 (5, 5) — 왼쪽 · 위 띠가 같은 정규화 거리(5 ÷ 20).
    const left = band(slot, 'Left', px(0, 0, 20, 100))
    const top = band(slot, 'Top', px(0, 0, 100, 20))
    expect(choose([left, top], { x: 5, y: 5 })).toBe(0)
    expect(choose([top, left], { x: 5, y: 5 })).toBe(0)
    const outer = band({ type: 'outer' }, 'Bottom', px(0, 0, 100, 100))
    expect(choose([left, outer], { x: 1, y: 1 })).toBe(1)
    expect(choose([left], { x: 50, y: 50 })).toBeNull()
  })
})
