// TRD S21-slot-docking §2-4: 셸 `pick` 과 **같은 골든 파일**을 먹는다 — 두 판이 갈라지면 한쪽이 빨개진다.
// ★골든 사본을 두지 않는다★ — 사본이면 Rust 쪽 골든만 고쳐도 이 시험이 초록으로 남는다. 파일 형식의 정본은 그 파일
//   머리의 `_format` 이다.

import { describe, expect, it } from 'vitest'

import goldenSource from '../../../../src-tauri/src/dock/pick_golden.json?raw'
import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { Picked } from '../../../../src-tauri/bindings/Picked'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { TabHeader } from '../../../../src-tauri/bindings/TabHeader'
import { pickTarget, reorderIndex, reorderLeft } from './dockPick'

interface GoldenCase {
  name: string
  regions: string
  point: PointCss
  /** 없음 = 스크롤을 주지 않는다(`null`). */
  scroll?: number
  expect: Picked
  /** 순서 이동 사례의 끄는 머리 왼쪽. */
  left?: number
}

interface PickGolden {
  source_view: string
  regions: Record<string, DockRegions>
  cases: GoldenCase[]
}

const golden = JSON.parse(goldenSource) as PickGolden

describe('pickTarget — 공유 골든(셸 pick 과 한 파일)', () => {
  it('골든에 사례가 있고 모든 사례의 영역 이름이 있다(빈 파일 · 오타가 초록으로 통과하지 않게)', () => {
    expect(golden.cases.length).toBeGreaterThan(0)
    for (const c of golden.cases) expect(golden.regions, c.name).toHaveProperty(c.regions)
  })

  for (const c of golden.cases) {
    it(`골든: [${c.regions}] ${c.name}`, () => {
      const r = golden.regions[c.regions]
      expect(pickTarget(r, c.point, c.scroll ?? null, golden.source_view)).toEqual(c.expect)
      const re = c.expect.type === 'reorder' ? r.reorder : null
      const left =
        re === null ? undefined : reorderLeft(re.heads, re.dragged, re.grab_x, c.point.x + (c.scroll ?? re.scroll))
      expect(left).toBe(c.left)
    })
  }
})

// 골든이 못 박는 규칙 — 셸 `regions_for` 가 만들지 않는 영역(끄는 탭이 든 영역 · 작은 캔버스 · 칸 밖으로 삐진 띠 · 캔버스와
//   겹친 탭바)에서만 갈리는 규칙이라 골든 사례로는 안 갈린다. 아래는 셸 `hit.rs` 의 같은 규칙 시험(이름을 각 사례에 단다)의
//   TS 쪽 짝이고, 골든과 달리 두 판이 한 파일을 먹지 않는다 — 규칙을 고치면 양쪽 시험을 손으로 함께 고친다.
const S = 'source'
const A = 'tab-a'
const regions = (over: Partial<DockRegions>): DockRegions => ({
  drag_id: 'd',
  rev: 1,
  bar_rev: 1,
  blocked: false,
  view_id: A,
  canvas_origin: { x: 0, y: 28 },
  canvas: { w: 400, h: 300 },
  outer: [],
  slots: [],
  tab_bar: null,
  reorder: null,
  ...over,
})

describe('pickTarget — 골든 밖의 규칙', () => {
  it('띠보다 좁은 캔버스에서는 마주 보는 두 띠가 겹치고 가까운 변이 이긴다(an_outer_corner_carries_every_strip_it_is_in_nearest_first)', () => {
    const strip = { x0: 0, y0: 0, x1: 20, y1: 300 }
    const r = regions({
      canvas: { w: 20, h: 300 },
      outer: [
        { side: 'Left', strip, landing: strip },
        { side: 'Right', strip, landing: strip },
      ],
    })
    expect(pickTarget(r, { x: 4, y: 128 }, null, S)).toEqual({ type: 'seam', anchor: null, side: 'Left' })
    expect(pickTarget(r, { x: 15, y: 128 }, null, S)).toEqual({ type: 'seam', anchor: null, side: 'Right' })
  })

  it('칸 틀 밖으로 삐진 띠는 보지 않는다(a_band_that_spills_out_of_the_frame_is_ignored)', () => {
    const r = regions({
      slots: [
        {
          slot_id: 'slot-1',
          frame: { x0: 100, y0: 100, x1: 300, y1: 200 },
          bands: [{ side: 'Left', anchor: 'slot-1', rect: { x0: 90, y0: 100, x1: 150, y1: 200 } }],
        },
      ],
    })
    expect(pickTarget(r, { x: 120, y: 178 }, null, S)).toEqual({ type: 'none' })
  })

  it('탭바는 캔버스와 겹쳐도 이긴다(the_tab_bar_is_half_open_and_wins_over_the_canvas)', () => {
    const r = regions({
      canvas_origin: { x: 0, y: 0 },
      outer: [{ side: 'Top', strip: { x0: 0, y0: 0, x1: 400, y1: 32 }, landing: { x0: 0, y0: 0, x1: 400, y1: 100 } }],
      tab_bar: { rect: { x0: 0, y0: 0, x1: 400, y1: 28 }, heads: [], insert_xs: [0], scroll: 0 },
    })
    expect(pickTarget(r, { x: 200, y: 10 }, null, S)).toEqual({ type: 'tab_bar', index: 0, over: null })
  })
})

describe('pickTarget — 끄는 탭', () => {

  it('탭바 머리에 끄는 탭이 남아 있어도 자리 · over 에서 뺀다', () => {
    const r = regions({
      tab_bar: {
        rect: { x0: 0, y0: 0, x1: 400, y1: 28 },
        heads: [
          { view_id: S, rect: { x0: 0, y0: 0, x1: 100, y1: 28 } },
          { view_id: A, rect: { x0: 100, y0: 0, x1: 200, y1: 28 } },
        ],
        insert_xs: [0, 100, 200],
        scroll: 0,
      },
    })
    expect(pickTarget(r, { x: 60, y: 10 }, null, S)).toEqual({ type: 'tab_bar', index: 0, over: null })
    expect(pickTarget(r, { x: 150, y: 10 }, null, S)).toEqual({ type: 'tab_bar', index: 1, over: A })
  })

  it('보이는 탭이 끄는 탭이면 캔버스 위는 가이드 없음이다', () => {
    const r = regions({
      view_id: S,
      outer: [{ side: 'Left', strip: { x0: 0, y0: 0, x1: 32, y1: 300 }, landing: { x0: 0, y0: 0, x1: 133, y1: 300 } }],
    })
    expect(pickTarget(r, { x: 10, y: 100 }, null, S)).toEqual({ type: 'none' })
    expect(pickTarget({ ...r, view_id: A }, { x: 10, y: 100 }, null, S)).toEqual({
      type: 'seam',
      anchor: null,
      side: 'Left',
    })
  })
})

// 순서 이동 규칙 — 셸 `hit.rs` 의 reorder_* 시험(이름을 각 사례에 단다)의 TS 쪽 짝. 골든이 못 싣는 값(17 자리 소수 —
//   셸 serde_json 이 1 ULP 어긋나게 읽을 수 있다)과 영역이 싣지 않는 입력을 여기서 잰다.
const head = (viewId: string, x0: number, x1: number): TabHeader => ({
  view_id: viewId,
  rect: { x0, y0: 0, x1, y1: 28 },
})
// A [40,140) · 끄는 탭 [140,260) · B [260,340) · C [340,500) — 가운데 A 90 · B 300 · C 420 · 끄는 머리 폭 120.
const STRIP = [head(A, 40, 140), head(S, 140, 260), head('tab-b', 260, 340), head('tab-c', 340, 500)]

function nextUp(x: number): number {
  const f = new Float64Array([x])
  const bits = new BigInt64Array(f.buffer)
  bits[0] += x >= 0 ? 1n : -1n
  return f[0]
}
function nextDown(x: number): number {
  const f = new Float64Array([x])
  const bits = new BigInt64Array(f.buffer)
  bits[0] += x > 0 ? -1n : 1n
  return f[0]
}

describe('reorderLeft · reorderIndex — 셸 hit::reorder_* 의 거울', () => {
  it('끄는 머리는 커서를 따르고 양 끝에 고정된다(the_dragged_head_follows_the_cursor_and_stops_at_both_ends)', () => {
    expect(reorderLeft(STRIP, 1, 30, 170)).toBe(140)
    expect(reorderLeft(STRIP, 1, 30, 200.5)).toBe(170.5)
    expect(reorderLeft(STRIP, 1, 30, 0)).toBe(40)
    expect(reorderLeft(STRIP, 1, 30, 70)).toBe(40)
    expect(reorderLeft(STRIP, 1, 30, 409)).toBe(379)
    expect(reorderLeft(STRIP, 1, 30, 410)).toBe(380)
    expect(reorderLeft(STRIP, 1, 30, 5000)).toBe(380)
  })

  it('이웃은 끄는 머리 변이 가운데를 넘어야 비킨다 — 같으면 그대로 · 1 ULP 넘으면 비킴(a_neighbour_moves_only_once_the_dragged_edge_passes_its_center)', () => {
    expect(reorderIndex(STRIP, 1, 140)).toBe(1)
    expect(reorderIndex(STRIP, 1, 90)).toBe(1)
    expect(reorderIndex(STRIP, 1, nextDown(90))).toBe(0)
    expect(reorderIndex(STRIP, 1, 180)).toBe(1)
    const past = nextUp(300) - 120
    expect(past + 120).toBe(nextUp(300))
    expect(reorderIndex(STRIP, 1, past)).toBe(2)
    expect(reorderIndex(STRIP, 1, 300)).toBe(2)
    expect(reorderIndex(STRIP, 1, 300.5)).toBe(3)
    expect(reorderIndex(STRIP, 1, 40)).toBe(0)
    expect(reorderIndex(STRIP, 1, 380)).toBe(3)
  })

  it('맨 왼쪽 · 맨 오른쪽 머리도 양 끝에 닿는다(the_first_and_last_heads_reach_both_ends_too)', () => {
    expect(reorderLeft(STRIP, 0, 10, 0)).toBe(40)
    expect(reorderIndex(STRIP, 0, 40)).toBe(0)
    expect(reorderLeft(STRIP, 0, 10, 1000)).toBe(400)
    expect(reorderIndex(STRIP, 0, 400)).toBe(3)
    expect(reorderLeft(STRIP, 3, 0, 1000)).toBe(340)
    expect(reorderIndex(STRIP, 3, 340)).toBe(3)
    expect(reorderIndex(STRIP, 3, 40)).toBe(0)
  })

  it('머리 하나는 늘 제자리다(a_lone_head_always_stays_put)', () => {
    const lone = [head(S, 40, 160)]
    for (const x of [-100, 0, 60, 300, 5000]) {
      expect(reorderLeft(lone, 0, 20, x)).toBe(40)
      expect(reorderIndex(lone, 0, reorderLeft(lone, 0, 20, x))).toBe(0)
    }
  })

  it('폭 0 머리도 가운데와 문턱이 있다(zero_width_heads_still_have_a_center_and_a_threshold)', () => {
    const h = [head(A, 0, 100), head(S, 100, 100), head('tab-b', 100, 200), head('tab-c', 200, 200)]
    expect(reorderLeft(h, 1, 0, 500)).toBe(200)
    expect(reorderIndex(h, 1, 50)).toBe(1)
    expect(reorderIndex(h, 1, 49.999)).toBe(0)
    expect(reorderIndex(h, 1, 150)).toBe(1)
    expect(reorderIndex(h, 1, 150.001)).toBe(2)
    expect(reorderIndex(h, 1, 200)).toBe(2)
    expect(reorderIndex(h, 1, nextUp(200))).toBe(3)
  })

  it('끄는 머리가 없는 목록은 폭 0 으로 친다(heads_without_the_dragged_one_count_it_as_zero_width)', () => {
    expect(reorderLeft([], 0, 30, 100)).toBe(70)
    expect(reorderLeft(STRIP, 9, 0, 1000)).toBe(500)
    expect(reorderIndex(STRIP, 9, 500)).toBe(4)
    expect(reorderIndex([], 0, 10)).toBe(0)
  })
})

describe('pickTarget — 순서 이동 구역(셸 regions.rs 의 손으로 세운 영역 시험 짝)', () => {
  // 바깥 띠가 없어 구역 밑에 칸 1 의 위 띠가 드러난 출처 창 영역(hand_source).
  const heads = [head(A, 0, 100), head(S, 100, 220), head('tab-b', 220, 300)]
  const bar = { x0: 0, y0: 0, x1: 600, y1: 28 }
  const hand = (): DockRegions =>
    regions({
      canvas: { w: 600, h: 400 },
      slots: [
        {
          slot_id: 'slot-1',
          frame: { x0: 0, y0: 0, x1: 600, y1: 400 },
          bands: [{ side: 'Top', anchor: 'slot-1', rect: { x0: 0, y0: 0, x1: 600, y1: 133 } }],
        },
      ],
      tab_bar: { rect: bar, heads: heads.filter(h => h.view_id !== S), insert_xs: [0, 220, 300], scroll: 0 },
      reorder: { zone: { x0: 0, y0: -15, x1: 600, y1: 43 }, heads, dragged: 1, grab_x: 30, scroll: 0 },
    })
  const topOf1: Picked = { type: 'seam', anchor: 'slot-1', side: 'Top' }

  it('구역이 탭바 · 칸 위 띠보다 먼저다(the_reorder_zone_is_picked_before_the_tab_bar_and_every_band)', () => {
    const r = hand()
    expect(pickTarget(r, { x: 150, y: 10 }, null, S)).toEqual({ type: 'reorder', index: 1 })
    expect(pickTarget(r, { x: 150, y: 42.999 }, null, S)).toEqual({ type: 'reorder', index: 1 })
    expect(pickTarget(r, { x: 150, y: 43 }, null, S)).toEqual(topOf1)
    const plain = { ...r, reorder: null }
    expect(pickTarget(plain, { x: 150, y: 10 }, null, S)).toEqual({ type: 'tab_bar', index: 1, over: null })
    expect(pickTarget(plain, { x: 150, y: 42.999 }, null, S)).toEqual(topOf1)
  })

  it('스크롤은 자리만 옮기고 구역 · 탭바 상자 · 캔버스는 옮기지 않는다(the_scroll_moves_the_index_but_not_the_zone_the_bar_box_or_the_canvas)', () => {
    const r = hand()
    expect(pickTarget(r, { x: 150, y: 10 }, null, S)).toEqual({ type: 'reorder', index: 1 })
    expect(pickTarget(r, { x: 150, y: 10 }, 100, S)).toEqual({ type: 'reorder', index: 2 })
    const scrolled = { ...r, reorder: { ...r.reorder!, scroll: 100 } }
    expect(pickTarget(scrolled, { x: 150, y: 10 }, null, S)).toEqual({ type: 'reorder', index: 2 })
    expect(pickTarget(scrolled, { x: 150, y: 10 }, 0, S)).toEqual({ type: 'reorder', index: 1 })
    const plain: DockRegions = { ...r, reorder: null }
    expect(pickTarget(plain, { x: 60, y: 10 }, null, S)).toEqual({ type: 'tab_bar', index: 1, over: A })
    expect(pickTarget(plain, { x: 60, y: 10 }, 200, S)).toEqual({ type: 'tab_bar', index: 2, over: 'tab-b' })
    const barScrolled: DockRegions = { ...plain, tab_bar: { ...plain.tab_bar!, scroll: 200 } }
    expect(pickTarget(barScrolled, { x: 60, y: 10 }, null, S)).toEqual({ type: 'tab_bar', index: 2, over: 'tab-b' })
    expect(pickTarget(barScrolled, { x: 60, y: 10 }, 0, S)).toEqual({ type: 'tab_bar', index: 1, over: A })
    expect(pickTarget(r, { x: 150, y: 43 }, 500, S)).toEqual(topOf1)
    expect(pickTarget(r, { x: 150, y: 42.999 }, -500, S)).toEqual({ type: 'reorder', index: 0 })
    expect(pickTarget(plain, { x: 600, y: 10 }, -100, S)).toEqual({ type: 'none' })
  })
})
