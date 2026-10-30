// TRD S21-slot-docking §4-3: 모든 우리 창 뿌리 dragover 의 판정 — 점 + 그 창 맥락 → 탭바 자리 · 띠 · 그 밖. 순수하다.
// 앞서는 차례(R6a) = 탭바 구역 > 바깥 띠 > 칸 띠 > 그 밖. 띠는 `bands.ts` 가 dragenter 에 미리 센 유효한 것만 본다 — 막힌 띠는
//   칠하지도 고르지도 않는다(R8). 탭바 구역 · 띠의 규칙 · 숫자는 셸 `layout/bands.rs` 와 한 벌이고 골든이 묶는다(`judge.test.ts`).

import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import {
  draggedLeft,
  dropIndex,
  dropMarkerX,
  insertionIndex,
  reorderAreaWidth,
  type StripBounds,
} from '../tabStrip/reorderGeometry'
import { choose, contains, zonesAt, type WindowBands } from './bands'
import type { RectCss } from './overlayTypes'

/**
 * 그 창 탭바 — 탭바 좌표 0 = 탭 끌기 영역 왼쪽(`left` · 뷰포트 x). `source` = 끄는 탭이 든 출처 창 자기 탭바(Chrome 순서 이동 —
 * R23) · `other` = 다른 창 탭바(세로선 — W13).
 */
export type JudgeStrip =
  | {
      role: 'source'
      left: number
      /** 지금 순서의 이상적 경계(끄는 탭 포함 — `idealBounds(layout, dragged)`). */
      bounds: StripBounds
      /** 끄는 탭의 `bounds.tabs` 자리 — 목록 밖(−1 · 길이 이상 = 끄는 탭이 이 탭바를 떠남)이면 다른 창 탭바처럼 센다. */
      dragged: number
      /** `grabOffset` — dragstart 에 한 번 셈한 값. */
      grabOffset: number
      /** `reorderAreaWidth` 의 `available` — 잰 값(`null` = 재지 못함). */
      available: number | null
    }
  | { role: 'other'; left: number; bounds: StripBounds }

/** 판정의 창 맥락 — dragenter · 보이는 탭 · 스냅숏이 바뀔 때만 다시 짓는다. */
export interface JudgeWindow {
  /** 캔버스 왼쪽 위(뷰포트 CSS px). */
  canvasOrigin: PointCss
  /**
   * 탭바 — `zone` = 탭바 구역(`bands.canvasZone` 의 값 · 캔버스 좌표 — 띠 셈의 `BandsInput.zone` 과 같은 값이어야 띠 영역과 맞물린다)
   * · `strip` = 그 자리를 셀 탭바. `null` = 탭바를 모름.
   */
  tabBar: { zone: RectCss; strip: JudgeStrip } | null
  /** 보이는 탭의 띠 — `null` = 끄는 탭 자신이 보이는 캔버스(탭 하나뿐인 출처 창 · `SelfTarget` — 띠 없음). */
  bands: WindowBands | null
}

export type Judgement =
  /**
   * 탭바 구역. `index` = 출처면 Chrome `insertionIndex`(끄는 탭을 뺀 목록의 자리) · 다른 창이면 `dropIndex`(그 창 목록에 끼울 자리).
   * `left` = 출처의 끄는 머리 왼쪽(탭바 좌표 · `draggedLeft`) · `marker` = 다른 창의 세로선 x(탭바 좌표 · `dropMarkerX`).
   */
  | { type: 'strip'; index: number; left: number | null; marker: number | null }
  /** 띠 — `band` = `bands.bands` 의 자리(미리보기 = 그 `landing`) · `zones` = 구역 선을 그릴 `bands.frames` 의 자리(바깥 띠면 `null`). */
  | { type: 'band'; band: number; zones: number | null }
  /** 그 밖(칸 가운데 · 캔버스 밖 우리 UI) — drop 이면 새 창. 칸 가운데면 그 칸의 구역 선(`zones`)은 그린다. */
  | { type: 'elsewhere'; zones: number | null }

/** 점 `point`(그 창 뷰포트 CSS px)의 판정. */
export function judge(w: JudgeWindow, point: PointCss): Judgement {
  const p = { x: point.x - w.canvasOrigin.x, y: point.y - w.canvasOrigin.y }
  if (w.tabBar !== null && contains(w.tabBar.zone, p)) return stripAt(w.tabBar.strip, point.x - w.tabBar.strip.left)
  if (w.bands === null) return { type: 'elsewhere', zones: null }
  const band = choose(w.bands.bands, p)
  const zones = zonesAt(w.bands, p, band)
  return band === null ? { type: 'elsewhere', zones } : { type: 'band', band, zones }
}

function stripAt(s: JudgeStrip, x: number): Judgement {
  const head = s.role === 'source' ? s.bounds.tabs[s.dragged] : undefined
  if (s.role === 'other' || head === undefined) {
    const index = dropIndex(s.bounds, x, null)
    return { type: 'strip', index, left: null, marker: dropMarkerX(s.bounds, index, null) }
  }
  const left = draggedLeft(x, s.grabOffset, head.x1 - head.x0, reorderAreaWidth(s.available, s.bounds))
  return { type: 'strip', index: insertionIndex(s.bounds, s.dragged, left), left, marker: null }
}
