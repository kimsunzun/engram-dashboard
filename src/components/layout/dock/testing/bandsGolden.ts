// 테스트 전용 — 셸 도킹 띠 · 새 창 크기 규칙과 공유하는 골든(`bands_golden.json`)을 **경로에서** 읽는다(TRD S21-slot-docking
//   §4-5 · §8-2). ★사본을 두지 않는다★ — 사본이면 Rust 쪽 골든만 고쳐도 TS 시험이 초록으로 남아 두 판이 조용히 갈라진다.
// 파일 형식의 정본은 그 파일 머리의 `_format`(같은 글 = 셸 `src-tauri/src/layout/bands/golden.rs` 머리)이다.

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

/** 골든의 띠 줄 — `anchor` = `"outer"` 또는 칸 id. */
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

/** `[x, y, pick, zones]` — 캔버스 CSS px · `pick` = `"strip"` | 띠 자리 | `null` · `zones` = 칸 자리 | `null`. */
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

/** 골든 한 창 → `windowBands` 의 입력(셸 골든 `Eval::new` 의 짝). */
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

/** 골든 줄의 `anchor` 꼴. */
export function goldenAnchor(a: DropAnchor): string {
  return a.type === 'outer' ? 'outer' : a.id
}

export function goldenCase(name: string): GoldenBandsCase {
  const c = bandsGolden.bands.find(b => b.name === name)
  if (c === undefined) throw new Error(`골든에 창 「${name}」 이 없다`)
  return c
}
