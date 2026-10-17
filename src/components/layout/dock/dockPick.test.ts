// TRD S21-slot-docking §2-4: 셸 `pick` 과 **같은 골든 파일**을 먹는다 — 두 판이 갈라지면 한쪽이 빨개진다.
// ★골든 사본을 두지 않는다★ — 사본이면 Rust 쪽 골든만 고쳐도 이 시험이 초록으로 남는다. 파일 형식의 정본은 그 파일
//   머리의 `_format` 이다.

import { describe, expect, it } from 'vitest'

import goldenSource from '../../../../src-tauri/src/dock/pick_golden.json?raw'
import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { Picked } from '../../../../src-tauri/bindings/Picked'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import { pickTarget } from './dockPick'

interface GoldenCase {
  name: string
  regions: string
  point: PointCss
  expect: Picked
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
      expect(pickTarget(golden.regions[c.regions], c.point, golden.source_view)).toEqual(c.expect)
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
    expect(pickTarget(r, { x: 4, y: 128 }, S)).toEqual({ type: 'seam', anchor: null, side: 'Left' })
    expect(pickTarget(r, { x: 15, y: 128 }, S)).toEqual({ type: 'seam', anchor: null, side: 'Right' })
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
    expect(pickTarget(r, { x: 120, y: 178 }, S)).toEqual({ type: 'none' })
  })

  it('탭바는 캔버스와 겹쳐도 이긴다(the_tab_bar_is_half_open_and_wins_over_the_canvas)', () => {
    const r = regions({
      canvas_origin: { x: 0, y: 0 },
      outer: [{ side: 'Top', strip: { x0: 0, y0: 0, x1: 400, y1: 32 }, landing: { x0: 0, y0: 0, x1: 400, y1: 100 } }],
      tab_bar: { rect: { x0: 0, y0: 0, x1: 400, y1: 28 }, heads: [], insert_xs: [0] },
    })
    expect(pickTarget(r, { x: 200, y: 10 }, S)).toEqual({ type: 'tab_bar', index: 0, over: null })
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
      },
    })
    expect(pickTarget(r, { x: 60, y: 10 }, S)).toEqual({ type: 'tab_bar', index: 0, over: null })
    expect(pickTarget(r, { x: 150, y: 10 }, S)).toEqual({ type: 'tab_bar', index: 1, over: A })
  })

  it('보이는 탭이 끄는 탭이면 캔버스 위는 가이드 없음이다', () => {
    const r = regions({
      view_id: S,
      outer: [{ side: 'Left', strip: { x0: 0, y0: 0, x1: 32, y1: 300 }, landing: { x0: 0, y0: 0, x1: 133, y1: 300 } }],
    })
    expect(pickTarget(r, { x: 10, y: 100 }, S)).toEqual({ type: 'none' })
    expect(pickTarget({ ...r, view_id: A }, { x: 10, y: 100 }, S)).toEqual({
      type: 'seam',
      anchor: null,
      side: 'Left',
    })
  })
})
