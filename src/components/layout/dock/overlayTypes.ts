// TRD S21-slot-docking §4-1: 끌기 판정 · 안내가 쓰는 TS 고유 타입.
// ★생성 바인딩의 `RectCss` 를 import 하지 않는다★ — 셸 `RectCss` 는 ts 내보내기를 떼기로 한 타입이라(§5-4) 이으면 그 바인딩을
//   걷을 때 이 파일을 쓰는 곳이 함께 깨진다(그 걷기의 끝 조건 rg 가 경로 문자열을 세므로 주석에도 그 경로를 적지 않는다). 모양은
//   그 바인딩과 같고, 셸 판과는 골든 `src-tauri/src/layout/bands_golden.json` 이 묶는다.

import type { PxRect } from './dockedTree'

/**
 * 사각형 — CSS px 경계 꼴(`getBoundingClientRect` 의 left · top · right · bottom). `x0 <= x1` · `y0 <= y1` · 반열림(왼쪽 · 위
 * 포함). 원점(뷰포트 · 그 창 캔버스)은 쓰는 자리가 정한다.
 */
export interface RectCss {
  x0: number
  y0: number
  x1: number
  y1: number
}

/**
 * 칸 하나의 구역 선 — 그릴 것 = `centre` 테두리 + `slot` 과 `centre` 의 같은 쪽 모서리를 잇는 선 넷(이웃한 두 칸 띠의 경계).
 * 좌표 = 그 창 캔버스. `slot` = 칸 띠가 서는 틀(캔버스 변에 닿은 칸이면 칸 틀보다 작다) · `centre` 는 `slot` 안이고 폭이나 높이가
 * 0 일 수 있다(선분).
 */
export interface SlotZones {
  slot: PxRect
  centre: RectCss
}

/**
 * 안내에 그릴 것(`DockOverlay` 의 `DockOverlayView`) — 좌표 = 안내 층 CSS px(층이 대상 창 캔버스를 덮으므로 그 캔버스 좌표).
 * 사각형은 반열림이다. U4b 까지 옛 안내 창 페이지도 이 타입으로 셸이 민 몸을 받는다 — 그 판은 `slot` 을 보내지 않는다.
 */
export interface OverlayView {
  /** 커서 밑 칸의 유효 칸 띠 영역 — 그리지 않는다(칸 띠의 경계는 `slot` 의 구역 선이 보인다 · 사용자 결정 2026-10-19). */
  bands: RectCss[]
  /**
   * 유효 바깥 띠 영역 — 커서가 바깥 띠에 들었을 때만 싣고 그 밖은 빈 목록이다(바깥 틀은 닿을 때만 — 사용자 결정 2026-10-19). 비지
   * 않으면 레이아웃 틀(층 테두리 = 캔버스 틀)을 그린다 · 영역 자체는 칠하지 않는다.
   */
  outer: RectCss[]
  /** 고른 띠의 붙을 사각형 — `null` = 고른 띠가 없다. */
  preview: RectCss | null
  /** 커서 밑 칸의 구역 선 — 없음 · `null` = 그리지 않는다(커서가 바깥 띠에 들었다 · 그 칸에 유효 칸 띠가 없다 · 옛 셸 판). */
  slot?: SlotZones | null
}
