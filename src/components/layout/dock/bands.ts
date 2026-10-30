// TRD S21-slot-docking §4-5: 도킹 띠 — 셸 `src-tauri/src/layout/bands.rs` 의 거울. 창 하나의 캔버스 위 탭바 구역 · 바깥 띠 ·
//   칸 띠 · 그 유효 영역 · 붙는 몫 · 붙을 사각형(미리보기) · 점의 띠 고르기 · 칸 구역 선. 좌표 = 그 창 캔버스의 CSS px(캔버스
//   왼쪽 위 원점) · 띠 · 영역 · 붙을 사각형은 반열림 정수 사각형. 순수하다.
// ★셸 판과 한 벌이다★ — 끄는 동안 끌기는 판정을 셸과 맞추지 않아(ADR-0296 · ADR-0297) 웹이 같은 규칙을 혼자 센다. 둘은 골든
//   `src-tauri/src/layout/bands_golden.json` 을 함께 먹는다 — 규칙을 고치면 셸 · 골든 · 이 파일을 같은 변경에서 고친다.
// ★숫자는 생성물 `DOCK_CONSTANTS` 한 벌이다★ — 띠 · 구역 · 상한 숫자를 이 파일에 손으로 적지 않는다(TRD §4-5). 분할 비율
//   한계는 스냅숏 `ratio_min` · `ratio_max` 로 받는다(ADR-0227).
// 진입점: `canvasZone`(탭바 구역) → `windowBands`(dragenter · 보이는 탭 · 스냅숏이 바뀔 때만 — 칸 틀 · 띠 · 유효성 · 구역 선) →
//   dragover 마다 `choose` · `zonesAt`(점 고르기만). 앞서는 차례(R6a) = 탭바 구역 > 바깥 띠 > 칸 띠 — 탭바 구역은 이 파일 앞
//   (`judge.ts`)에서 걸러진다. 막힌 띠(R8)는 없는 것으로 친다 — 칠하기와 고르기가 같은 이 목록을 쓰고, 막힌 바깥 띠는 밑 칸
//   띠를 덮지 않는다(AC36).

import type { LayoutNode } from '../../../api/layoutTypes'
import { DOCK_CONSTANTS } from '../../../../src-tauri/bindings/DockConstants'
import type { DockSide } from '../../../../src-tauri/bindings/DockSide'
import type { DropAnchor } from '../../../../src-tauri/bindings/DropAnchor'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import {
  computeSlots,
  dockedTree,
  framePx,
  incomingRect,
  pxEdge,
  slotIds,
  toU32,
  type CanvasPx,
  type PxRect,
  type RatioLimits,
} from './dockedTree'
import type { RectCss, SlotZones } from './overlayTypes'

const { BAND_SHARE, BAND_MIN_PX, OUTER_BAND_PX, STRIP_ZONE_PX, ATTACH_SHARE_MAX, SIDE_ORDER } = DOCK_CONSTANTS

/** 띠 하나 — 고른 띠의 판정 · 그림 · 놓기 재료(셸 `BandHit`). */
export interface BandHit {
  /** 놓기 `DropTo.seam.anchor` 그대로 — `outer` = 바깥 띠 · `slot` = 그 칸의 띠. */
  anchor: DropAnchor
  side: DockSide
  /** 띠 사각형 전체(앞선 영역을 빼기 전) — `choose` 의 정규화 거리 재료. */
  band: PxRect
  /** 붙는 몫 — `landing` 이 이 값으로 셈했고 놓기의 `DropTo.seam.share` 도 이 값이다(다시 셈하지 않는다). */
  share: number
  /** 놓으면 끼운 탭이 차지할 사각형(미리보기 — R4). */
  landing: PxRect
  /** 유효 영역 — 띠에서 앞서는 영역을 뺀 것(비지 않는다 · 서로 겹치지 않는다). `choose` 가 짚는 곳. */
  areas: PxRect[]
}

/** `windowBands` 의 입력 — 창 하나 몫(셸 `WindowTargets`). */
export interface BandsInput {
  canvas: CanvasPx
  /** 그 창에 보이는 탭의 배치 — 칸 틀도 이것으로 센다(셸 `frames`). */
  layout: LayoutNode
  ratio: RatioLimits
  /** 끄는 탭의 배치 — `null` = 모양 칸이 없거나 깨짐 → 칸 하나로 친다(셸이 놓을 때 `ZeroArea` 로 다시 막는다). */
  incoming: LayoutNode | null
  /** 끄는 탭의 원래 크기(R7) — `null` = 모름 → 몫이 `ATTACH_SHARE_MAX`. */
  original: SizeCss | null
  /** 탭바 구역(`canvasZone` 의 값) — `null` = 탭바를 모름(띠에서 빼지 않는다). */
  zone: RectCss | null
}

/** 창 하나의 띠 셈 — dragenter 에 한 번 짓고 dragover 는 읽기만 한다. */
export interface WindowBands {
  canvas: CanvasPx
  /** 칸 틀 — 전위 순. */
  frames: Array<{ id: string; frame: PxRect }>
  /** 유효 띠 전부 — 바깥 띠(`SIDE_ORDER`) 뒤에 칸마다(`frames` 순) 그 칸의 띠(`SIDE_ORDER`). */
  bands: BandHit[]
  /** `frames` 와 같은 차례 — 커서가 그 칸 위일 때 그릴 구역 선. `null` = 그 칸에 유효한 칸 띠가 없다. */
  zones: Array<SlotZones | null>
}

// 모양 칸이 없을 때의 끄는 탭 — id 는 `tabDragData.parseShape` 의 칸 이름 꼴이라 보이는 탭의 칸 id(uuid)와 겹치지 않는다.
const ONE_SLOT: LayoutNode = { type: 'slot', id: 'shape-slot-0', content: { type: 'empty' } }

// ── 탭바 구역 ─────────────────────────────────────────────────────────────────

/**
 * 셸 `strip_zone` — 바탕 사각형 `area`(뷰포트 CSS px — `localDrag.StripGeometry.zoneViewport` 의 바탕)의 가로 그대로 · 세로 위아래
 * `STRIP_ZONE_PX`.
 */
// ADR-0295
export function stripZone(area: RectCss): RectCss {
  return { x0: area.x0, y0: area.y0 - STRIP_ZONE_PX, x1: area.x1, y1: area.y1 + STRIP_ZONE_PX }
}

/** 셸 `contains` — 반열림(왼쪽 · 위 포함). NaN 점은 어디에도 없다. */
export function contains(r: RectCss, p: PointCss): boolean {
  return r.x0 <= p.x && p.x < r.x1 && r.y0 <= p.y && p.y < r.y1
}

/** 셸 `zone_in_canvas` — 구역을 캔버스 좌표로 옮기고 정수 경계로 바깥으로 넓힌다. 탭바 판정 · 띠 빼기가 이 경계 하나를 쓴다. */
// ADR-0295
export function zoneInCanvas(zone: RectCss, canvasOrigin: PointCss): RectCss {
  return {
    x0: Math.floor(zone.x0 - canvasOrigin.x),
    y0: Math.floor(zone.y0 - canvasOrigin.y),
    x1: Math.ceil(zone.x1 - canvasOrigin.x),
    y1: Math.ceil(zone.y1 - canvasOrigin.y),
  }
}

/** 바탕 사각형 `area`(뷰포트 · `null` = 탭바를 모름 — `stripZone`)의 탭바 구역(캔버스 좌표) — `BandsInput.zone` · 판정의 구역. */
export function canvasZone(area: RectCss | null, canvasOrigin: PointCss): RectCss | null {
  return area === null ? null : zoneInCanvas(stripZone(area), canvasOrigin)
}

// ── 몫 · 띠 ──────────────────────────────────────────────────────────────────

/**
 * 셸 `attach_share` — 칸(바깥 띠면 캔버스) `target` 의 `side` 쪽에 붙을 때의 몫 = `min(L, T × ATTACH_SHARE_MAX) ÷ T` 를
 * `[ratio.min, ATTACH_SHARE_MAX]` 로 자른 것(`L` = 원래 크기의 나누는 방향 길이 · `T` = 칸의 같은 방향 길이). 어떤 입력에도 유한한
 * 그 범위 값이다 — 칸 길이 0 · `L` NaN · +∞ = 상한 · 0 이하 = 하한.
 */
// ADR-0295
export function attachShare(original: SizeCss, side: DockSide, target: PxRect, ratio: RatioLimits): number {
  const horizontal = side === 'Left' || side === 'Right'
  const l = horizontal ? original.w : original.h
  const t = horizontal ? satSub(target.x1, target.x0) : satSub(target.y1, target.y0)
  if (t === 0) return ATTACH_SHARE_MAX
  const cap = t * ATTACH_SHARE_MAX
  // 셸 `f64::min` 은 NaN 이 아닌 쪽을 돌려준다(`Math.min` 은 NaN) — NaN `L` 은 상한으로 간다.
  const share = (Number.isNaN(l) ? cap : Math.min(l, cap)) / t
  return Math.min(Math.max(share, ratio.min), ATTACH_SHARE_MAX)
}

/**
 * 셸 `band_frame` — 칸 `frame` 의 띠 틀. 칸 틀에서 유효한 바깥 띠가 선 변(`outer`)의 캔버스 변 안쪽 `OUTER_BAND_PX` 를 뺀다 —
 * 바깥 띠가 막힌 변은 빼지 않는다(그 몫은 칸 띠 차지 — AC36). 칸이 그 몫 안에 다 들면 빈 사각형이다.
 */
// ADR-0295
export function bandFrame(frame: PxRect, canvas: CanvasPx, outer: readonly DockSide[]): PxRect {
  const cut = (side: DockSide): number => (outer.includes(side) ? OUTER_BAND_PX : 0)
  const x0 = Math.min(Math.max(frame.x0, cut('Left')), frame.x1)
  const y0 = Math.min(Math.max(frame.y0, cut('Top')), frame.y1)
  return {
    x0,
    y0,
    x1: Math.max(Math.min(frame.x1, satSub(canvas.w, cut('Right'))), x0),
    y1: Math.max(Math.min(frame.y1, satSub(canvas.h, cut('Bottom'))), y0),
  }
}

/** 셸 `band_thickness` — `max(round(BAND_SHARE × len), BAND_MIN_PX)` 를 `len` 으로 자른 것. */
export function bandThickness(len: number): number {
  return Math.min(Math.max(toU32(Math.round(len * BAND_SHARE)), BAND_MIN_PX), len)
}

// (좌우 띠 두께, 위아래 띠 두께) — 칸 틀 길이로 세고 띠 틀 안으로 자른다.
function roomThickness(frame: PxRect, room: PxRect): [number, number] {
  return [
    Math.min(bandThickness(satSub(frame.x1, frame.x0)), satSub(room.x1, room.x0)),
    Math.min(bandThickness(satSub(frame.y1, frame.y0)), satSub(room.y1, room.y0)),
  ]
}

/** 셸 `slot_band` — 칸 `frame` 의 `side` 띠. 띠 틀 `room` 의 `side` 변에 붙여 `room` 안으로 자른다. */
// ADR-0295
export function slotBand(frame: PxRect, room: PxRect, side: DockSide): PxRect {
  const [tw, th] = roomThickness(frame, room)
  switch (side) {
    case 'Left':
      return { ...room, x1: room.x0 + tw }
    case 'Right':
      return { ...room, x0: room.x1 - tw }
    case 'Top':
      return { ...room, y1: room.y0 + th }
    case 'Bottom':
      return { ...room, y0: room.y1 - th }
  }
}

/**
 * 셸 `slot_centre` — 칸 띠 넷 어디에도 들지 않는 가운데(소수). `room` 모서리와 이 사각형의 같은 쪽 모서리를 잇는 선이 이웃한
 * 두 칸 띠의 `choose` 경계다. 좌우(위아래) 띠가 `room` 절반을 넘는 좁은 칸이면 그 축 폭 0 인 선분까지 줄인다.
 */
export function slotCentre(frame: PxRect, room: PxRect): RectCss {
  const [tw, th] = roomThickness(frame, room)
  const reach = (a: number, b: number, t: number): number => (t > 0 ? satSub(b, a) / (2 * t) : 1)
  const s = Math.min(reach(room.x0, room.x1, tw), reach(room.y0, room.y1, th), 1)
  return { x0: room.x0 + s * tw, y0: room.y0 + s * th, x1: room.x1 - s * tw, y1: room.y1 - s * th }
}

/** 셸 `outer_strips` — 캔버스 네 변의 바깥 띠(`SIDE_ORDER` 순) · 두께 `OUTER_BAND_PX` · 캔버스를 넘지 않는다. */
export function outerStrips(canvas: CanvasPx): Array<[DockSide, PxRect]> {
  const { w, h } = canvas
  const tw = Math.min(OUTER_BAND_PX, w)
  const th = Math.min(OUTER_BAND_PX, h)
  const strip: Record<DockSide, PxRect> = {
    Left: { x0: 0, y0: 0, x1: tw, y1: h },
    Right: { x0: w - tw, y0: 0, x1: w, y1: h },
    Top: { x0: 0, y0: 0, x1: w, y1: th },
    Bottom: { x0: 0, y0: h - th, x1: w, y1: h },
  }
  return SIDE_ORDER.map(side => [side, strip[side]])
}

/**
 * 셸 `landing_rect` — 대상 나무 `target` 의 `anchor` 의 `side` 에 `incoming` 을 몫 `share` 로 끼웠을 때 끼운 것이 차지할 캔버스
 * px 사각형(미리보기 = 놓은 뒤 그려질 끼운 칸들의 외접 사각형 — R4). 결과에 새로 면적 0 칸이 생기거나(R8) 기준 칸이 없으면 `null`.
 */
// ADR-0227
export function landingRect(
  target: LayoutNode,
  anchor: DropAnchor,
  side: DockSide,
  incoming: LayoutNode,
  canvas: CanvasPx,
  share: number,
  ratio: RatioLimits,
): PxRect | null {
  const result = dockedTree(target, anchor, side, incoming, share, ratio)
  if (!result.ok) return null
  const r = incomingRect(computeSlots(result.tree, ratio), slotIds(incoming))
  if (r === null) return null
  return { x0: pxEdge(canvas.w, r.x0), y0: pxEdge(canvas.h, r.y0), x1: pxEdge(canvas.w, r.x1), y1: pxEdge(canvas.h, r.y1) }
}

/** 셸 `subtract` — `r` 에서 `cut` 을 뺀 나머지(위 · 아래 통째 · 그 사이 왼쪽 · 오른쪽 — 이 차례). 빈 조각은 싣지 않는다. */
export function subtract(r: PxRect, cut: PxRect): PxRect[] {
  if (isEmpty(r)) return []
  if (isEmpty(cut) || !overlaps(r, cut)) return [r]
  const y0 = Math.max(r.y0, cut.y0)
  const y1 = Math.min(r.y1, cut.y1)
  return [
    { ...r, y1: cut.y0 },
    { ...r, y0: cut.y1 },
    { ...r, x1: cut.x0, y0, y1 },
    { ...r, x0: cut.x1, y0, y1 },
  ].filter(p => !isEmpty(p))
}

function subtractAll(r: PxRect, cuts: readonly PxRect[]): PxRect[] {
  return cuts.reduce<PxRect[]>((pieces, cut) => pieces.flatMap(p => subtract(p, cut)), [r])
}

/** `bands` 에 든 바깥 띠의 변 — `bandFrame` 의 `outer`. */
export function outerSides(bands: readonly BandHit[]): DockSide[] {
  return bands.filter(b => b.anchor.type === 'outer').map(b => b.side)
}

// ── 창 하나 ───────────────────────────────────────────────────────────────────

/** 창 하나의 띠 셈(셸 `effective_bands` + 칸마다 구역 선) — 막힌 띠와 앞선 영역에 전부 덮인 띠는 없다. */
// ADR-0295
export function windowBands(input: BandsInput): WindowBands {
  const { canvas } = input
  const incoming = input.incoming ?? ONE_SLOT
  const original = input.original ?? { w: Number.NaN, h: Number.NaN }
  const frames = computeSlots(input.layout, input.ratio).map(r => ({ id: r.slot_id, frame: framePx(canvas, r) }))
  const hit = (anchor: DropAnchor, side: DockSide, band: PxRect, target: PxRect, cuts: readonly PxRect[]): BandHit | null => {
    const areas = subtractAll(band, cuts)
    if (areas.length === 0) return null
    const share = attachShare(original, side, target, input.ratio)
    const landing = landingRect(input.layout, anchor, side, incoming, canvas, share, input.ratio)
    return landing === null ? null : { anchor, side, band, share, landing, areas }
  }

  const zone = zoneCut(input.zone)
  const whole: PxRect = { x0: 0, y0: 0, x1: canvas.w, y1: canvas.h }
  const bands: BandHit[] = []
  for (const [side, strip] of outerStrips(canvas)) {
    const b = hit({ type: 'outer' }, side, strip, whole, zone === null ? [] : [zone])
    if (b !== null) bands.push(b)
  }
  // 칸 띠에서 빼는 것 = 탭바 구역 + 유효 바깥 띠 영역 전부(R6a ① · 막힌 바깥 띠는 이미 빠졌다 — AC36).
  const cuts = [...(zone === null ? [] : [zone]), ...bands.flatMap(b => b.areas)]
  const outer = outerSides(bands)
  for (const { id, frame } of frames) {
    const room = bandFrame(frame, canvas, outer)
    for (const side of SIDE_ORDER) {
      const b = hit({ type: 'slot', id }, side, slotBand(frame, room, side), frame, cuts)
      if (b !== null) bands.push(b)
    }
  }

  const zones = frames.map(({ id, frame }) => {
    if (!bands.some(b => b.anchor.type === 'slot' && b.anchor.id === id)) return null
    const room = bandFrame(frame, canvas, outer)
    return { slot: room, centre: slotCentre(frame, room) }
  })
  return { canvas, frames, bands, zones }
}

// 캔버스 밖(음수)을 잘라 정수 사각형으로 — 정수가 아닌 값이 와도 바깥으로 넓혀 구역 안 점을 품은 영역이 남지 않게.
function zoneCut(zone: RectCss | null): PxRect | null {
  if (zone === null) return null
  const r = {
    x0: toU32(Math.floor(zone.x0)),
    y0: toU32(Math.floor(zone.y0)),
    x1: toU32(Math.ceil(zone.x1)),
    y1: toU32(Math.ceil(zone.y1)),
  }
  return isEmpty(r) ? null : r
}

// ── 점 ────────────────────────────────────────────────────────────────────────

/**
 * 셸 `choose` — 점 `p`(캔버스 CSS px)에서 고르는 `bands` 의 자리. ① 점을 품은 바깥 띠 영역 중 정규화 거리(띠 변까지 거리 ÷ 두께)
 * 가 가장 작은 것 ② 아니면 칸 띠 영역 중 같은 기준 ③ 같으면 앞의 것(그래서 `SIDE_ORDER`) ④ 없으면 `null`(칸 가운데 · 캔버스 밖 — R6).
 */
// ADR-0295
export function choose(bands: readonly BandHit[], p: PointCss): number | null {
  const nearest = (outer: boolean): number | null => {
    let best: number | null = null
    let bestD = 0
    for (let i = 0; i < bands.length; i += 1) {
      const b = bands[i]
      if ((b.anchor.type === 'outer') !== outer || !b.areas.some(a => containsPx(a, p))) continue
      const d = edgeGap(p, b.band, b.side) / thickness(b.band, b.side)
      if (best === null || d < bestD) {
        best = i
        bestD = d
      }
    }
    return best
  }
  return nearest(true) ?? nearest(false)
}

/** 점 `p`(캔버스 CSS px)를 품은 칸의 `frames` 자리 — `null` = 캔버스 밖. 이웃 칸 경계의 점은 오른쪽 · 아래 칸이다. */
export function frameAt(wb: WindowBands, p: PointCss): number | null {
  const i = wb.frames.findIndex(f => containsPx(f.frame, p))
  return i < 0 ? null : i
}

/**
 * 셸 `slot_zones` — 점 `p` 에서 구역 선을 그릴 칸의 `frames` 자리. `null` = 고른 띠가 바깥 띠(그때 안내는 레이아웃 틀) · 캔버스
 * 밖 · 점 밑 칸에 유효한 칸 띠가 없다. 탭바 구역은 이 함수 앞에서 걸러진다(탭바에서는 선을 그리지 않는다).
 */
export function zonesAt(wb: WindowBands, p: PointCss, picked: number | null): number | null {
  if (picked !== null && wb.bands[picked].anchor.type === 'outer') return null
  const i = frameAt(wb, p)
  return i === null || wb.zones[i] === null ? null : i
}

function satSub(a: number, b: number): number {
  return a > b ? a - b : 0
}

function isEmpty(r: PxRect): boolean {
  return r.x0 >= r.x1 || r.y0 >= r.y1
}

function overlaps(a: PxRect, b: PxRect): boolean {
  return a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

function containsPx(r: PxRect, p: PointCss): boolean {
  return r.x0 <= p.x && p.x < r.x1 && r.y0 <= p.y && p.y < r.y1
}

/** 점에서 `band` 의 `side` 변까지 — 안쪽이면 0 이상. */
function edgeGap(p: PointCss, band: PxRect, side: DockSide): number {
  switch (side) {
    case 'Left':
      return p.x - band.x0
    case 'Right':
      return band.x1 - p.x
    case 'Top':
      return p.y - band.y0
    case 'Bottom':
      return band.y1 - p.y
  }
}

// 점을 품은 띠만 부른다 — 그러면 두께가 양수라 나눗셈이 선다.
function thickness(r: PxRect, side: DockSide): number {
  return side === 'Left' || side === 'Right' ? r.x1 - r.x0 : r.y1 - r.y0
}
