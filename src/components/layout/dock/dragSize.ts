// TRD S21-slot-docking §4-2 「크기」: 출처 dragstart 가 싣는 크기 셈 — 새 창 안쪽 크기(R22 · Chrome `CalculateDraggedWindowSize`) ·
//   끄는 탭의 원래 크기(R7 — 대상의 붙는 몫 재료) · 새 창에서 커서가 올 점 `grab`. 단위 = 논리(DIP) px = CSS px. 순수하다.
// ★셸 `src-tauri/src/layout/drag_size.rs` 의 거울이다★ — LLM 명령(T-65)의 기본값 · 놓을 때 검증이 셸 판을 쓴다. 둘은 골든
//   `bands_golden.json` 의 `drag_size` 표를 함께 먹는다(`dragSize.test.ts`). 여백 숫자는 생성물 `DOCK_CONSTANTS` 한 벌이다.

import type { Insets } from '../../../../src-tauri/bindings/Insets'
import { DOCK_CONSTANTS } from '../../../../src-tauri/bindings/DockConstants'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import { tabStripLayout } from '../tabStrip/layout'

const { MAXIMIZED_INSET_DIP } = DOCK_CONSTANTS

/** 끄기 시작 때 출처 창의 사실(셸 `SourceWindow`). */
export interface SourceWindow {
  /** 복원(보통) 안쪽 크기(`window:frame.restored`) — `null` = 모름(지금 크기로 친다). */
  restored: SizeCss | null
  maximized: boolean
  /** 창 틀 = 바깥 − 안쪽(`window:frame.insets`). */
  insets: Insets
  /** 지금 안쪽 크기(뷰포트) — 최대화면 최대화 안쪽 크기다. */
  viewport: SizeCss
  /** 그 창 모니터의 작업 영역(`screen.availWidth` · `availHeight` — 바깥 크기의 한계). */
  work: SizeCss
  /** 크롬 = 뷰포트 − 탭 캔버스. */
  chrome: SizeCss
  /** 탭 하나뿐인 팝아웃 — 새 창 없이 그 창을 옮긴다(Q34). */
  single: boolean
}

export interface DragSizes {
  /** 새 창 안쪽 크기 — `null` = 탭 하나뿐인 팝아웃(그 창을 크기 그대로 옮긴다). */
  size: SizeCss | null
  /** 끄는 탭의 원래 크기 = 캔버스 크기(음수는 0). */
  original: SizeCss
}

/**
 * 셸 `dragged_size` — `restored` 에서 ① 작업 영역 `work` 보다 큰 변은 `max(0, work − 2 × MAXIMIZED_INSET_DIP)` ② 출처가 최대화
 * (`maximized` = 그 크기)면 각 변을 그 절반 이상으로. ★②가 마지막이라 이긴다(C5)★ — 순서를 바꾸지 말 것. 세 크기는 같은 쪽
 * (셋 다 바깥 · 셋 다 안쪽)이어야 한다.
 */
// ADR-0295
export function draggedSize(restored: SizeCss, maximized: SizeCss | null, work: SizeCss): SizeCss {
  let { w, h } = restored
  if (w > work.w) w = fmax(work.w - 2 * MAXIMIZED_INSET_DIP, 0)
  if (h > work.h) h = fmax(work.h - 2 * MAXIMIZED_INSET_DIP, 0)
  if (maximized !== null) {
    w = fmax(w, maximized.w / 2)
    h = fmax(h, maximized.h / 2)
  }
  return { w, h }
}

/**
 * 셸 `drag_sizes` — 탭이 둘 이상인 창: `size` = `draggedSize` 를 안쪽 크기로 옮긴 것(복원 크기 · 없으면 지금 크기 / 작업 영역 −
 * 틀 / 최대화면 그 안쪽 크기) · `original` = `size` − 크롬. 탭 하나뿐인 팝아웃: `size` 없음 · `original` = (최대화면 복원 크기 ·
 * 아니면 지금 크기) − 크롬.
 */
export function dragSizes(s: SourceWindow): DragSizes {
  const canvas = (v: SizeCss): SizeCss => ({ w: fmax(v.w - s.chrome.w, 0), h: fmax(v.h - s.chrome.h, 0) })
  if (s.single) {
    return { size: null, original: canvas(s.maximized && s.restored !== null ? s.restored : s.viewport) }
  }
  const work = { w: s.work.w - (s.insets.l + s.insets.r), h: s.work.h - (s.insets.t + s.insets.b) }
  const size = draggedSize(s.restored ?? s.viewport, s.maximized ? s.viewport : null, work)
  return { size, original: canvas(size) }
}

/** `dragGrab` 의 입력 — 좌표는 출처 뷰포트 CSS px. */
export interface GrabInput {
  /** 누른 점. */
  pressed: PointCss
  /** 누른 때 끄는 머리의 가로 범위. */
  head: { x0: number; x1: number }
  /** 탭 끌기 영역 왼쪽(첫 탭 왼쪽). */
  stripLeft: number
  /**
   * 새 창 탭바에서 머리들이 쓸 수 있는 폭(`layout.ts` 의 `available`) — `null` = 모름(표준 폭). 탭이 둘 이상이면 새 창 폭,
   * 최대화된 탭 하나뿐인 팝아웃이면 복원 창 폭에서 센다(TRD §3-5).
   */
  headRoom: number | null
  single: boolean
  maximized: boolean
}

/**
 * 새 창 뷰포트에서 커서가 올 점(`DropTo.new_window.grab` · Chrome `GetTabOffsetForDetachedWindow`) = (탭바 왼쪽 + r × W1, 누른 y) —
 * r = 누른 점의 머리 안 폭 비율 · W1 = 새 창 탭바의 탭 하나 머리 폭. 최대화되지 않은 탭 하나뿐인 팝아웃은 누른 점 그대로다(그 탭이
 * 이미 첫 탭이고 W1 = 지금 머리 폭 — 셸 `Reuse` 가 창을 끈 만큼 옮긴다 · Q34).
 */
// 새 창 탭바의 왼쪽 · 세로 자리는 출처 탭바와 같다고 친다 — 새 창은 팝아웃이라 main 이 출처면 그 크롬이 다를 수 있다([미검] ·
//   어긋나면 새 창이 커서에서 그만큼 밀려 뜬다).
export function dragGrab(g: GrabInput): PointCss {
  if (g.single && !g.maximized) return { x: g.pressed.x, y: g.pressed.y }
  const width = g.head.x1 - g.head.x0
  const r = width > 0 ? (g.pressed.x - g.head.x0) / width : 0
  const w1 = tabStripLayout(g.headRoom, 1, 0).widths[0]
  return { x: g.stripLeft + r * w1, y: g.pressed.y }
}

/** 셸 `f64::max` — NaN 이 아닌 쪽을 돌려준다(`Math.max` 는 NaN). */
function fmax(a: number, b: number): number {
  if (Number.isNaN(a)) return b
  if (Number.isNaN(b)) return a
  return Math.max(a, b)
}
