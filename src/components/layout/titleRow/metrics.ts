// TRD S26-window-chrome §7-6: 제목 줄 크기 값의 정본(CSS px = DIP · 확대 1). 셸은 최소 창 폭 보고(§7-7)로만 받는다.

import { MIN_PANE_PX } from '../splitPreview'
import { TAB_ADD_GAP, TAB_BUTTON_SIZE, TAB_MIN_ACTIVE_W } from '../tabStrip/layout'

export const TAB_BAR_H = 28

// 활성 · 맴돎 면은 머리 위 이만큼 내려앉는다 — 그 위로 바가 보여야 탭이 바에서 솟은 조각으로 읽힌다(Chrome). 머리 상자는 줄
//   높이 그대로다. 머리 내용 칸과 `+` 상자도 이 높이부터 줄 바닥까지 세워 × 원과 `+` 원의 가운데 높이를 맞추고, 번호 표식도
//   같은 칸에 글자를 세워 탭 글자와 가운데 높이를 맞춘다. 이 4 px 는 끌 자리가 아니다 — 머리 누름이 이긴다(§7-2 · R19).
export const SURFACE_TOP = 4

// 보통 상태 위 여백 — Chrome 의 「최대화 아닐 때 비활성 탭 위 6 px」(슬롯 도킹 R25)이자, 빠른 위쪽 끌기가 OS 끌기 시작 전에
//   창을 벗어나지 않게 그 길을 출처 창 안에 두는 몫이다(W13). 값은 S13 이 정한다.
export const TOP_GAP = 6
// Chrome 은 최대화(condensed) 때 끌 손잡이를 탭 위로 늘리지 않는다(Chromium `TabStrip::CanExtendDragHandle`).
export const TOP_GAP_MAXIMIZED = 0

// 끌 빈 자리 최소 = Chromium `FrameGrabHandle::CalculatePreferredSize` — 탭줄이 차도 잡을 자리를 남긴다. 번호 표식으로
//   갈음하지 않는다(메인 결정 M10).
export const GRAB_MIN_W = 42

/** 번호 표식 폭 = max(`MARKER_MIN_W`, 글자 폭 + 2 × `MARKER_PAD`) — 한 자리 = 28(메인 결정). 자리 수만큼 넓어지고 자르지 않는다. */
export const MARKER_MIN_W = 28
export const MARKER_PAD = 8

// Windows 11 캡션 단추 폭(관행 — 미검 · S11). 높이 = 줄 높이.
export const CAPTION_BUTTON_W = 46

export function titleRowHeight(maximized: boolean): number {
  return (maximized ? TOP_GAP_MAXIMIZED : TOP_GAP) + TAB_BAR_H
}

export interface MinWindowSizeInput {
  /** 탭 줄 왼 여백 — 크롬 모양의 `leading_inset`. */
  leadingInset: number
  /** 번호 표식의 잰 폭 — `null` = 표식을 그리지 않는다(대기 모드). */
  markerW: number | null
  /** 창 단추 셋을 그리나. */
  buttons: boolean
}

/**
 * 최소 창 크기(TRD §7-7 · 정수 CSS px — 올림). 폭 = 왼 여백 + 활성 탭 하나 + `+` 상자 + 끌 빈 자리 + 표식 + 단추 셋 · 높이 =
 * 보통 상태 줄 높이 + 칸 최소. 보통 상태 기준이다 — 셸은 보통 크기 창에만 OS 최소 크기로 입히고 열 때 한 번 넓혀 당긴다
 * (`src-tauri/src/chrome` — 최대화 동안은 걷지 않고 둔다 · 최대화 사각형은 작업 영역이다 · R20). 표식 폭은 `MARKER_MIN_W` 아래로 치지 않는다
 * (재지 못한 0 폭도 그 값이다).
 */
export function minWindowSize(input: MinWindowSizeInput): { width: number; height: number } {
  const marker = input.markerW === null ? 0 : Math.max(MARKER_MIN_W, input.markerW)
  const width =
    input.leadingInset +
    TAB_MIN_ACTIVE_W +
    TAB_ADD_GAP +
    TAB_BUTTON_SIZE +
    GRAB_MIN_W +
    marker +
    (input.buttons ? 3 * CAPTION_BUTTON_W : 0)
  return { width: Math.ceil(width), height: titleRowHeight(false) + MIN_PANE_PX }
}
