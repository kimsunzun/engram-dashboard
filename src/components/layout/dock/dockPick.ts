// TRD S21-slot-docking §2-4: 셸 `dock::regions::pick`(규칙 정본 = `src-tauri/src/dock/hit.rs`)의 거울. 웹뷰가 끄는 동안
//   가이드를 그릴 판정이고, 놓으면 셸이 같은 영역 위에서 같은 규칙으로 다시 판정해 대조한다 — 갈라지면 놓기가
//   「보인 것과 다름 = 무변경」으로 떨어진다.
// ★두 판은 골든 `src-tauri/src/dock/pick_golden.json` 을 함께 먹는다★ — 규칙을 고치면 Rust · 골든 · 이 파일을 같은
//   변경에서 고친다(`dockPick.test.ts`).
// 부동소수 비교만 하고 반올림하지 않는다(R6a ③) — 셸과 같은 f64 연산을 같은 순서로 해야 경계 점이 같은 쪽에 떨어진다.

import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { DockSide } from '../../../../src-tauri/bindings/DockSide'
import type { Picked } from '../../../../src-tauri/bindings/Picked'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { PxRectWire } from '../../../../src-tauri/bindings/PxRectWire'
import type { RectCss } from '../../../../src-tauri/bindings/RectCss'
import type { SlotRegion } from '../../../../src-tauri/bindings/SlotRegion'

// 바깥 띠 두께(CSS px) — 셸 `hit::OUTER_BAND_PX` 의 사본. 갈라지면 골든 시험이 빨개진다.
const OUTER_BAND_PX = 32

// 동률 깨기 순서 — 셸 `hit::SIDE_ORDER`.
const SIDE_ORDER: readonly DockSide[] = ['Left', 'Right', 'Top', 'Bottom']

const NONE: Picked = { type: 'none' }

/**
 * 점 `point`(그 창 뷰포트 CSS px)가 영역 `regions` 의 어디인가. `sourceView` = 끄는 탭.
 * 셸 `pick` 과 같은 입력이면 같은 답이다(골든). 영역의 `blocked` 는 보지 않는다 — 막힌 영역은 몸이 비어 늘 `none`
 * 이고, 받을지(`dropEffect`)는 부르는 쪽이 `blocked` 로 가른다.
 */
export function pickTarget(regions: DockRegions, point: PointCss, sourceView: string): Picked {
  const css = { x: point.x, y: point.y }

  const bar = regions.tab_bar
  if (bar !== null && containsCss(bar.rect, css)) {
    // 셸이 이미 뺐어도 다시 거른다 — 셸 `locate` 가 그렇게 한다.
    const heads = bar.heads.filter(h => h.view_id !== sourceView)
    return {
      type: 'tab_bar',
      index: heads.filter(h => centerX(h.rect) <= css.x).length,
      over: heads.find(h => containsCss(h.rect, css))?.view_id ?? null,
    }
  }

  const p = { x: css.x - regions.canvas_origin.x, y: css.y - regions.canvas_origin.y }
  const { w, h } = regions.canvas
  if (!(0 <= p.x && p.x < w) || !(0 <= p.y && p.y < h)) return NONE
  // 보이는 탭이 없으면 끄는 탭으로 친다 — 셸 `pick` 과 같은 자리채움이라 캔버스 위가 가이드 없음이 된다.
  if ((regions.view_id ?? sourceView) === sourceView) return NONE

  const under = regions.slots.find(s => containsPx(s.frame, p)) ?? null
  const sides = outerSides(p, regions.canvas)
  if (sides.length > 0) {
    const valid = sides.find(side => regions.outer.some(o => o.side === side))
    if (valid !== undefined) return { type: 'seam', anchor: null, side: valid }
  }
  if (under === null) return NONE
  // 띠는 id 로 다시 찾는다 — 셸 `pick` 이 칸 틀은 점을 품은 칸에서, 띠는 같은 id 의 첫 칸에서 꺼낸다.
  const bands = regions.slots.find(s => s.slot_id === under.slot_id)?.bands ?? []
  const side = chooseBand(p, under.frame, bands)
  return side === null ? NONE : { type: 'seam', anchor: under.slot_id, side }
}

function chooseBand(p: Pt, frame: PxRectWire, bands: SlotRegion['bands']): DockSide | null {
  let best: { side: DockSide; d: number } | null = null
  for (const side of SIDE_ORDER) {
    for (const band of bands) {
      if (band.side !== side) continue
      if (!within(band.rect, frame) || !containsPx(band.rect, p)) continue
      const d = edgeGap(p, frame, side) / thickness(band.rect, side)
      if (best === null || d < best.d) best = { side, d }
    }
  }
  return best?.side ?? null
}

// 셸 `hit::outer_strips` 를 캔버스에서 다시 세운다 — 영역의 `outer[].strip` 이 아니라. 셸 `pick` 도 변의 유효 여부만
//   `outer` 에서 읽고 띠 사각형은 캔버스에서 만든다.
function outerSides(p: Pt, canvas: DockRegions['canvas']): DockSide[] {
  const { w, h } = canvas
  const tw = Math.min(OUTER_BAND_PX, w)
  const th = Math.min(OUTER_BAND_PX, h)
  const full: PxRectWire = { x0: 0, y0: 0, x1: w, y1: h }
  const strips: Array<[DockSide, PxRectWire]> = [
    ['Left', { x0: 0, y0: 0, x1: tw, y1: h }],
    ['Right', { x0: w - tw, y0: 0, x1: w, y1: h }],
    ['Top', { x0: 0, y0: 0, x1: w, y1: th }],
    ['Bottom', { x0: 0, y0: h - th, x1: w, y1: h }],
  ]
  return (
    strips
      .filter(([, strip]) => containsPx(strip, p))
      .map(([side]) => ({ side, d: edgeGap(p, full, side) }))
      // Array.prototype.sort 는 안정 정렬이라 동률은 SIDE_ORDER 그대로 남는다 — 셸의 안정 정렬과 같다.
      .sort((a, b) => a.d - b.d)
      .map(({ side }) => side)
  )
}

interface Pt {
  x: number
  y: number
}

const centerX = (r: RectCss): number => (r.x0 + r.x1) / 2

const containsCss = (r: RectCss, p: Pt): boolean => r.x0 <= p.x && p.x < r.x1 && r.y0 <= p.y && p.y < r.y1

const containsPx = (r: PxRectWire, p: Pt): boolean => r.x0 <= p.x && p.x < r.x1 && r.y0 <= p.y && p.y < r.y1

const within = (inner: PxRectWire, outer: PxRectWire): boolean =>
  outer.x0 <= inner.x0 && inner.x1 <= outer.x1 && outer.y0 <= inner.y0 && inner.y1 <= outer.y1

function edgeGap(p: Pt, frame: PxRectWire, side: DockSide): number {
  switch (side) {
    case 'Left':
      return p.x - frame.x0
    case 'Right':
      return frame.x1 - p.x
    case 'Top':
      return p.y - frame.y0
    case 'Bottom':
      return frame.y1 - p.y
  }
}

// 점을 품은 띠만 부른다 — 그러면 두께가 양수라 나눗셈이 선다.
function thickness(r: PxRectWire, side: DockSide): number {
  return side === 'Left' || side === 'Right' ? r.x1 - r.x0 : r.y1 - r.y0
}
