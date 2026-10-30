// ADR-0227: 창 캔버스 크기 보고의 보내는 쪽. 관측·디바운스는 `WindowLayout.tsx` 의 훅이 하고, 여기서는 무엇을
//   `report_window_canvas` 로 보낼지 정하고(보내기 규칙 = `util/latestValueReporter.ts`), 셸이 마지막으로 받아들인 값을
//   내놓는다(`subscribeCanvasReport` — 구분선 드래그의 허용 범위가 셸과 같은 입력으로 읽는다).
// 상태가 모듈 범위인 이유: 같은 웹뷰에서도 훅 인스턴스는 바뀐다(RootErrorBoundary 다시 그리기·HMR 재마운트).
//   인스턴스 상태면 새 인스턴스가 옛 인스턴스의 날아가는 invoke 를 모른 채 하나를 더 날린다.
// 컴포넌트 파일과 떼어 둔 이유: HMR 은 고친 모듈만 다시 실행하므로 `WindowLayout.tsx` 를 고쳐도 이 상태는
//   산다. 컴포넌트 파일이 컴포넌트 아닌 것(아래 테스트용 초기화)을 내보내면 Fast Refresh 경계도 깨진다.
//   한계: 이 파일 자체를 고치면 상태가 새로 생긴다(개발 중에만).

import { invoke } from '@tauri-apps/api/core'

import { LatestValueReporter } from '../../util/latestValueReporter'

/** 창 캔버스 크기(CSS px 정수) — 셸 `report_window_canvas` 의 인자. */
export interface CanvasPx {
  w: number
  h: number
}

/** 한 웹뷰(= 한 창)가 공유하는 보고기. `lastSent` = 셸이 성공 응답한 마지막 값. */
export type CanvasReportState = LatestValueReporter<CanvasPx>

const sameCanvas = (a: CanvasPx, b: CanvasPx): boolean => a.w === b.w && a.h === b.h
const isEmptyCanvas = (s: CanvasPx): boolean => s.w === 0 || s.h === 0

function createCanvasReport(): CanvasReportState {
  const reporter: CanvasReportState = new LatestValueReporter<CanvasPx>({
    send: size => invoke<void>('report_window_canvas', { w: size.w, h: size.h }),
    same: sameCanvas,
    log: 'windowCanvasReport',
    describe: size => `report_window_canvas(${size.w}x${size.h})`,
    // 테스트 초기화로 버려진 보고기의 늦은 응답은 지금 보고기의 구독자에게 알리지 않는다.
    onAccepted: size => {
      if (reporter === canvasReport) notifyCanvas(size)
    },
  })
  return reporter
}

let canvasReport = createCanvasReport()

// 구독 하나마다 항목을 따로 둔다 — 같은 함수를 두 번 걸어도 한쪽 해제가 다른 쪽을 지우지 않는다.
const canvasListeners = new Set<{ cb: (size: CanvasPx | null) => void }>()

/** 훅이 관측을 붙일 때 잡는 이 웹뷰의 보고기. */
export function getCanvasReport(): CanvasReportState {
  return canvasReport
}

/**
 * `getCanvasReport().lastSent` 가 바뀔 때마다 새 값으로 부른다 — 셸이 지금과 다른 크기를 성공 응답했을 때, 그리고
 * 테스트 초기화가 값을 비웠을 때(null). 실패·같은 값의 재성공에는 부르지 않는다.
 * 반환 = 구독 해제(여러 번 불러도 된다). 구독자가 던져도 보고는 그대로 이어진다(오류 로그만 남는다).
 */
export function subscribeCanvasReport(cb: (size: CanvasPx | null) => void): () => void {
  const entry = { cb }
  canvasListeners.add(entry)
  return () => {
    canvasListeners.delete(entry)
  }
}

// 알림은 invoke 응답 처리 안에서 나간다 — 구독자의 throw 가 새면 보고 실패로 셈돼 같은 값을 다시 보낸다.
function notifyCanvas(size: CanvasPx | null): void {
  for (const { cb } of [...canvasListeners]) {
    try {
      cb(size)
    } catch (err) {
      console.error('[windowCanvasReport] 캔버스 구독자 오류:', err)
    }
  }
}

/** 테스트 전용 — 모듈 상태 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetCanvasReportForTest(): void {
  const had = canvasReport.lastSent !== null
  canvasReport.release()
  canvasReport = createCanvasReport()
  if (had) notifyCanvas(null)
}

/** 반올림한 관측 하나를 알린다(디바운스 전). */
export function observeCanvasSize(st: CanvasReportState, size: CanvasPx): void {
  // 0 크기는 보낼 후보가 아니라 옛 값을 낡게 만들지 않는다.
  if (isEmptyCanvas(size)) return
  st.observe(size)
}

/** 디바운스를 마친 측정을 보낼 차례에 올린다. */
export function submitCanvasSize(st: CanvasReportState, size: CanvasPx): void {
  // 0×0 RO 콜백은 실재한다(TerminalSlot.tsx 도 0×0 을 거른다). 셸도 무시하지만 화면이 보내지 않는다.
  if (isEmptyCanvas(size)) return
  st.submit(size)
}

/** 관측을 뗄 때 — 대기 값을 버리고 재시도를 멈춘다(날아가던 invoke 는 그대로 — `LatestValueReporter.release`). */
export function releaseCanvasReport(st: CanvasReportState): void {
  st.release()
}
