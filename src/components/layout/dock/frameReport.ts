// TRD S21-slot-docking §4: 창 틀 보고(`report_dock_chrome`)의 보내는 쪽 — 셸은 이 값으로 그 창의 탭바 구역(붙기 판정) · 캔버스
//   원점(칸 띠)을 안다. 측정이라 version · 알림 · 저장이 없다(ADR-0227). 끌기 시작 때 재면 첫 판정이 늦으므로 평소에 보고한다.
//   언제 잴지는 부르는 쪽이 정하고 여기서는 무엇을 보낼지만 정한다 — 보내기 규칙은 `util/latestValueReporter.ts`.
// ★모양은 U8 까지 옛 `DockChrome` 그대로다★ — `tabs: []` · `scroll: null` 을 싣는다. 셸 `DockChrome` 이 그때까지 `tabs` 를
//   요구해(serde 기본값 없음) 칸을 빼면 보고가 전부 역직렬화에 실패한다(TRD §13 B8). 뜻만 바뀐다: `tab_bar` = 탭 끌기 영역.
// 상태가 모듈 범위인 이유는 `windowCanvasReport.ts` 와 같다(훅 인스턴스가 바뀌어도 날아가는 invoke 를 안다 · HMR).

import { invoke } from '@tauri-apps/api/core'

import type { DockChrome } from '../../../../src-tauri/bindings/DockChrome'
import type { RectCss } from '../../../../src-tauri/bindings/RectCss'
import { LatestValueReporter } from '../../../util/latestValueReporter'
import { dragAreaWidth, type StripBounds } from '../tabStrip/reorderGeometry'

/** 탭 끌기 영역의 재료 — 좌표는 뷰포트 CSS px. */
export interface DragAreaMeasure {
  /** 첫 탭 왼쪽 = 끌기 영역 왼쪽. */
  left: number
  /** 탭바 줄의 위 · 아래. */
  top: number
  bottom: number
  /** `dragAreaWidth` 의 `available` — 잰 「머리들이 쓸 수 있는 폭」 · 재지 못하면 `null`(보이는 탭 폭 합으로 대신한다). */
  available: number | null
  /** 그 탭바의 이상적 경계(`idealBounds`). */
  bounds: StripBounds
}

/** 한 창 틀의 측정. `canvas` = 탭 캔버스 영역의 왼쪽 위(뷰포트 — `getBoundingClientRect()` 를 그대로 넘겨도 된다). */
export interface FrameMeasure {
  canvas: { left: number; top: number }
  area: DragAreaMeasure
}

/**
 * 탭 끌기 영역 사각형(C3) — x = 첫 탭 왼쪽 ~ 그 + 끌기 영역 폭 · y = 탭바 줄. `tab_drag_begin` 의 `area` 와 이 보고의
 * `tab_bar` 가 같은 사각형이다(셸이 둘에서 같은 탭바 구역을 셈한다).
 */
export function dragAreaRect(m: DragAreaMeasure): RectCss {
  return { x0: m.left, y0: m.top, x1: m.left + dragAreaWidth(m.available, m.bounds), y1: m.bottom }
}

/** 측정 → 보고 값. */
export function frameChromeOf(m: FrameMeasure): DockChrome {
  return {
    canvas_origin: { x: m.canvas.left, y: m.canvas.top },
    tab_bar: dragAreaRect(m.area),
    tabs: [],
    scroll: null,
  }
}

// `tabs` · `scroll` 은 늘 빈 칸이라 보지 않는다.
const sameFrame = (a: DockChrome, b: DockChrome): boolean =>
  a.canvas_origin.x === b.canvas_origin.x &&
  a.canvas_origin.y === b.canvas_origin.y &&
  a.tab_bar.x0 === b.tab_bar.x0 &&
  a.tab_bar.y0 === b.tab_bar.y0 &&
  a.tab_bar.x1 === b.tab_bar.x1 &&
  a.tab_bar.y1 === b.tab_bar.y1

const createFrameReport = (): LatestValueReporter<DockChrome> =>
  new LatestValueReporter<DockChrome>({
    send: chrome => invoke<void>('report_dock_chrome', { chrome }),
    same: sameFrame,
    log: 'frameReport',
    describe: () => 'report_dock_chrome',
  })

let report = createFrameReport()

/** 잰 값을 곧바로 보낼 차례에 올린다(디바운스 없음) — 늦으면 안 되는 자리에 쓴다. 셸이 마지막으로 받은 값과 같으면 보내지 않는다. */
export function submitFrame(m: FrameMeasure): void {
  report.submit(frameChromeOf(m))
}

/**
 * `delayMs` 뒤 `measure` 를 불러 그 값을 보낸다 — 그 사이 다시 부르면 타이머를 다시 잡고 마지막 `measure` 만 쓴다. `measure` 가
 * `null` 을 주면(잴 요소가 아직 없다) 보내지 않는다.
 */
export function scheduleFrameReport(measure: () => FrameMeasure | null, delayMs?: number): void {
  report.schedule(() => {
    const m = measure()
    return m === null ? null : frameChromeOf(m)
  }, delayMs)
}

/** 관측을 뗄 때(창 레이아웃 언마운트) — 디바운스 · 대기 값을 버리고 재시도를 멈춘다(`LatestValueReporter.release`). */
export function cancelFrameReport(): void {
  report.release()
}

/** 테스트 전용 — 모듈 상태 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetFrameReportForTest(): void {
  report.release()
  report = createFrameReport()
}
