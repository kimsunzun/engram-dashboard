// TRD S21-slot-docking §4-5: docking bands — mirror of shell `src-tauri/src/layout/bands.rs`: tab bar zone · outer bands · area bands · effective regions · attach share · landing rectangle (preview) · band choice for a point · area zone lines of one window canvas. Coordinates = CSS px of the window canvas (top-left origin); bands · regions · landings are half-open integer rectangles. Pure.
// ★One set with the shell version★: the drag does not align its decision with the shell (ADR-0296 · ADR-0297), so the web counts the same rules alone; both read the golden `src-tauri/src/layout/bands_golden.json` — change shell · golden · this file together.
// ★Numbers come from the generated `DOCK_CONSTANTS`★ (TRD §4-5), never hand-written here; split ratio limits arrive as snapshot `ratio_min` · `ratio_max` (ADR-0227).
// Entry points: `canvasZone` → `windowBands` (on dragenter · visible tab · snapshot change) → per dragover `choose` · `zonesAt`. Precedence (R6a) = tab bar zone > outer bands > area bands; the tab bar zone is filtered before this file (`judge.ts`). Blocked bands (R8) are treated as absent — painting and choosing use the same list, and a blocked outer band does not cover the area band beneath (AC36).

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

/** One band — the material for deciding · drawing · dropping with the chosen band (shell `BandHit`). */
export interface BandHit {
  /** The drop `DropTo.seam.anchor` as is — `outer` = outer band · `slot` = that area's band. */
  anchor: DropAnchor
  side: DockSide
  /** The whole band rectangle (before subtracting the preceding regions) — the material for the normalized distance of `choose`. */
  band: PxRect
  /** Attach share — `landing` computed with this value and the drop's `DropTo.seam.share` is this value too (not recomputed). */
  share: number
  /** The rectangle the inserted tab will occupy when dropped (preview — R4). */
  landing: PxRect
  /** Effective region — the band minus the preceding regions (never empty · never overlapping each other). Where `choose` points. */
  areas: PxRect[]
}

/** Input of `windowBands` — one window's share (shell `WindowTargets`). */
export interface BandsInput {
  canvas: CanvasPx
  /** Layout of the tab visible in that window — area frames are also computed from this (shell `frames`). */
  layout: LayoutNode
  ratio: RatioLimits
  /** Layout of the dragged tab — `null` = no shape field or broken → treated as one area (the shell blocks again at drop with `ZeroArea`). */
  incoming: LayoutNode | null
  /** Original size of the dragged tab (R7) — `null` = unknown → share is `ATTACH_SHARE_MAX`. */
  original: SizeCss | null
  /** Tab bar zone (value of `canvasZone`) — `null` = tab bar unknown (not subtracted from bands). */
  zone: RectCss | null
}

/** Band computation of one window — built once at dragenter and dragover only reads. */
export interface WindowBands {
  canvas: CanvasPx
  /** Area frames — pre-order. */
  frames: Array<{ id: string; frame: PxRect }>
  /** All effective bands — outer bands (`SIDE_ORDER`) followed by each area's bands (`SIDE_ORDER`) per area (in `frames` order). */
  bands: BandHit[]
  /** Same order as `frames` — the zone lines to draw when the cursor is over that area. `null` = that area has no effective area band. */
  zones: Array<SlotZones | null>
}

// Stand-in for a dragged tab with no shape field; its id mirrors a field name of `tabDragData.parseShape`, so it cannot collide with a visible area uuid.
const ONE_SLOT: LayoutNode = { type: 'slot', id: 'shape-slot-0', content: { type: 'empty' } }

// ── tab bar zone ─────────────────────────────────────────────────────────────────

/** Shell `strip_zone` — same horizontal extent as base rectangle `area` (viewport CSS px, base of `localDrag.StripGeometry.zoneViewport`), `STRIP_ZONE_PX` above and below. */
// ADR-0295
export function stripZone(area: RectCss): RectCss {
  return { x0: area.x0, y0: area.y0 - STRIP_ZONE_PX, x1: area.x1, y1: area.y1 + STRIP_ZONE_PX }
}

/** Shell `contains` — half-open (left · top included). A NaN point is nowhere. */
export function contains(r: RectCss, p: PointCss): boolean {
  return r.x0 <= p.x && p.x < r.x1 && r.y0 <= p.y && p.y < r.y1
}

/** Shell `zone_in_canvas` — zone in canvas coordinates, widened outward to integer boundaries; the tab bar decision and band subtraction share this boundary. */
// ADR-0295
export function zoneInCanvas(zone: RectCss, canvasOrigin: PointCss): RectCss {
  return {
    x0: Math.floor(zone.x0 - canvasOrigin.x),
    y0: Math.floor(zone.y0 - canvasOrigin.y),
    x1: Math.ceil(zone.x1 - canvasOrigin.x),
    y1: Math.ceil(zone.y1 - canvasOrigin.y),
  }
}

/** Tab bar zone in canvas coordinates for base rectangle `area` (`null` = tab bar unknown) — `BandsInput.zone`. */
export function canvasZone(area: RectCss | null, canvasOrigin: PointCss): RectCss | null {
  return area === null ? null : zoneInCanvas(stripZone(area), canvasOrigin)
}

// ── share · band ──────────────────────────────────────────────────────────────────

/**
 * Shell `attach_share`: share when attaching to `side` of area `target` (the canvas for an outer band) = `min(L, T × ATTACH_SHARE_MAX) ÷ T` clamped to `[ratio.min, ATTACH_SHARE_MAX]` (`L` = original size along the split, `T` = area length along it). Always finite and in range — area length 0 · `L` NaN · +∞ = upper bound; ≤ 0 = lower bound.
 */
// ADR-0295
export function attachShare(original: SizeCss, side: DockSide, target: PxRect, ratio: RatioLimits): number {
  const horizontal = side === 'Left' || side === 'Right'
  const l = horizontal ? original.w : original.h
  const t = horizontal ? satSub(target.x1, target.x0) : satSub(target.y1, target.y0)
  if (t === 0) return ATTACH_SHARE_MAX
  const cap = t * ATTACH_SHARE_MAX
  // Shell `f64::min` returns the non-NaN side (`Math.min` gives NaN) — NaN `L` goes to the upper bound.
  const share = (Number.isNaN(l) ? cap : Math.min(l, cap)) / t
  return Math.min(Math.max(share, ratio.min), ATTACH_SHARE_MAX)
}

/**
 * Shell `band_frame`: area `frame` minus `OUTER_BAND_PX` inside the canvas edge for each side (`outer`) with an effective outer band — not for a side whose outer band is blocked (that share belongs to the area band — AC36). Entirely inside that share → empty rectangle.
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

/** Shell `band_thickness` — `max(round(BAND_SHARE × len), BAND_MIN_PX)` clamped to `len`. */
export function bandThickness(len: number): number {
  return Math.min(Math.max(toU32(Math.round(len * BAND_SHARE)), BAND_MIN_PX), len)
}

// (left/right band thickness, top/bottom band thickness) — computed from the area frame length and clamped inside the band frame.
function roomThickness(frame: PxRect, room: PxRect): [number, number] {
  return [
    Math.min(bandThickness(satSub(frame.x1, frame.x0)), satSub(room.x1, room.x0)),
    Math.min(bandThickness(satSub(frame.y1, frame.y0)), satSub(room.y1, room.y0)),
  ]
}

/** Shell `slot_band` — the `side` band of area `frame`. Attached to the `side` edge of the band frame `room` and clamped inside `room`. */
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
 * Shell `slot_centre`: the center (fractional) outside all four area bands. Lines joining `room` corners to this rectangle's same-side corners are the `choose` boundaries of adjacent area bands. When left/right (top/bottom) bands exceed half of `room` it shrinks to width 0 on that axis.
 */
export function slotCentre(frame: PxRect, room: PxRect): RectCss {
  const [tw, th] = roomThickness(frame, room)
  const reach = (a: number, b: number, t: number): number => (t > 0 ? satSub(b, a) / (2 * t) : 1)
  const s = Math.min(reach(room.x0, room.x1, tw), reach(room.y0, room.y1, th), 1)
  return { x0: room.x0 + s * tw, y0: room.y0 + s * th, x1: room.x1 - s * tw, y1: room.y1 - s * th }
}

/** Shell `outer_strips` — outer bands on the four sides of the canvas (in `SIDE_ORDER`) · thickness `OUTER_BAND_PX` · not exceeding the canvas. */
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
 * Shell `landing_rect`: canvas px rectangle the inserted part occupies when `incoming` is inserted with share `share` at `side` of `anchor` in `target` (preview = bounding rectangle of the inserted areas after the drop — R4). `null` if a zero-area area would appear (R8) or the reference area is missing.
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

/** Shell `subtract` — what remains of `r` after removing `cut` (top · whole bottom · left · right in between — in this order). Empty pieces are not carried. */
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

/** The side of the outer band contained in `bands` — the `outer` of `bandFrame`. */
export function outerSides(bands: readonly BandHit[]): DockSide[] {
  return bands.filter(b => b.anchor.type === 'outer').map(b => b.side)
}

// ── a single window ───────────────────────────────────────────────────────────────────

/** Band computation of one window (shell `effective_bands` + zone lines per area) — a band blocked or fully covered by preceding regions does not exist. */
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
  // What is subtracted from area bands = the tab bar zone + all effective outer band regions (R6a ① · blocked outer bands are already out — AC36).
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

// Clip outside the canvas (negative) to an integer rectangle — even if a non-integer value arrives, widen outward so no region containing a point in the zone remains.
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

// ── point ────────────────────────────────────────────────────────────────────────

/**
 * Shell `choose`: index in `bands` chosen at point `p`: ① among outer band regions containing it, the smallest normalized distance (distance to band edge ÷ thickness); ② else among area band regions by the same criterion; ③ ties → earlier (hence `SIDE_ORDER`); ④ none → `null` (area center · outside canvas — R6).
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

/** The `frames` index of the area containing point `p` (canvas CSS px) — `null` = outside the canvas. A point on a boundary of neighboring areas goes to the right · lower area. */
export function frameAt(wb: WindowBands, p: PointCss): number | null {
  const i = wb.frames.findIndex(f => containsPx(f.frame, p))
  return i < 0 ? null : i
}

/**
 * Shell `slot_zones`: `frames` index of the area to draw zone lines for at `p`; `null` = chosen band is an outer band (the guide is then the layout frame) · outside the canvas · no effective area band under the point. The tab bar zone is filtered out before this function.
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

/** From the point to the `side` edge of `band` — 0 or more if inside. */
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

// Called only for a band containing the point — then the thickness is positive and the division holds.
function thickness(r: PxRect, side: DockSide): number {
  return side === 'Left' || side === 'Right' ? r.x1 - r.x0 : r.y1 - r.y0
}
