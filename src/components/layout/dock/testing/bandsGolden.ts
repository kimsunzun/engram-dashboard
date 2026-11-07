// Test only — reads the golden (`bands_golden.json`) shared with the shell docking band · new window size rules **from its path** (TRD S21-slot-docking
//   §4-5 · §8-2). ★Do not keep a copy★ — with a copy, fixing only the Rust-side golden would leave the TS test green and the two versions would diverge silently.
// The canonical file format is `_format` at the head of that file (same text = the head of the shell `src-tauri/src/layout/bands/golden.rs`).

import goldenSource from '../../../../../src-tauri/src/layout/bands_golden.json?raw'
import type { LayoutNode } from '../../../../api/layoutTypes'
import type { DockSide } from '../../../../../src-tauri/bindings/DockSide'
import type { DropAnchor } from '../../../../../src-tauri/bindings/DropAnchor'
import type { Insets } from '../../../../../src-tauri/bindings/Insets'
import type { PointCss } from '../../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../../src-tauri/bindings/SizeCss'
import { canvasZone, type BandsInput } from '../bands'
import type { CanvasPx, PxRect, RatioLimits } from '../dockedTree'
import type { RectCss, SlotZones } from '../overlayTypes'
import { parseShape } from '../tabDragData'

/** Band line of the golden — `anchor` = `"outer"` or an area id. */
export interface GoldenBand {
  anchor: string
  side: DockSide
  band: PxRect
  areas: PxRect[]
  share: number
  landing: PxRect
}

export interface GoldenSlot {
  id: string
  frame: PxRect
  zones: SlotZones | null
}

/** `[x, y, pick, zones]` — canvas CSS px · `pick` = `"strip"` | band position | `null` · `zones` = area position | `null`. */
export type GoldenPoint = [number, number, 'strip' | number | null, number | null]

export interface GoldenBandsCase {
  name: string
  canvas: CanvasPx
  canvas_origin: PointCss
  tab_bar: RectCss | null
  layout: LayoutNode
  shape: string
  original: SizeCss | null
  zone: RectCss | null
  blocked: Array<{ anchor: string; side: DockSide }>
  bands: GoldenBand[]
  slots: GoldenSlot[]
  points: GoldenPoint[]
}

export interface GoldenDragSizeCase {
  name: string
  restored: SizeCss | null
  maximized: boolean
  insets: Insets
  viewport: SizeCss
  work: SizeCss
  chrome: SizeCss
  single: boolean
  size: SizeCss | null
  original: SizeCss
}

export interface BandsGolden {
  _format: string
  _constants: unknown
  ratio_limits: RatioLimits
  bands: GoldenBandsCase[]
  drag_size: GoldenDragSizeCase[]
}

export const bandsGolden = JSON.parse(goldenSource) as BandsGolden

/** One golden window → input of `windowBands` (counterpart of shell golden `Eval::new`). */
export function bandsInputOf(c: GoldenBandsCase): BandsInput {
  const incoming = parseShape(c.shape)
  if (incoming === null) throw new Error(`${c.name}: 모양 ${c.shape} 이 읽히지 않는다`)
  return {
    canvas: c.canvas,
    layout: c.layout,
    ratio: bandsGolden.ratio_limits,
    incoming,
    original: c.original,
    zone: canvasZone(c.tab_bar, c.canvas_origin),
  }
}

/** The `anchor` form of a golden line. */
export function goldenAnchor(a: DropAnchor): string {
  return a.type === 'outer' ? 'outer' : a.id
}

export function goldenCase(name: string): GoldenBandsCase {
  const c = bandsGolden.bands.find(b => b.name === name)
  if (c === undefined) throw new Error(`골든에 창 「${name}」 이 없다`)
  return c
}
