// ADR-0227: 칸 틀 기본 지표 보고(TRD §2d) — 웹뷰당 한 번, 잎이 마운트될 때 그 잎의 테두리 요소를 재서
//   `report_ui_metrics` 로 보낸다. 셸은 이 값과 창 캔버스로 칸 px 를 계산하고 화면에 묻지 않는다.
// 상태가 모듈 범위인 이유: 「보냈음」은 웹뷰 하나에 하나이고, 웹뷰 재로드가 모듈을 새로 실행하며 함께 초기화된다.
//   부팅 때 잎 N 개가 한꺼번에 마운트되므로 진행 중인 보고 하나를 모두가 공유한다.
// 컴포넌트 파일과 떼어 둔 이유는 `windowCanvasReport.ts` 와 같다(HMR 은 고친 모듈만 다시 실행 · Fast Refresh 경계).

import { invoke } from '@tauri-apps/api/core'

import type { UiMetrics } from '../../api/layoutTypes'
import { retryAsync } from '../../util/retryInvoke'
import { MIN_PANE_PX } from './splitPreview'

interface UiMetricsReportState {
  /** 셸이 성공 응답했다. 실패로는 세우지 않는다. */
  reported: boolean
  /** 지금 보내는 작업(재시도 대기 포함). 끝나면 성공·실패와 무관하게 비운다. */
  inFlight: Promise<void> | null
}

// 초기화가 상태 객체를 통째로 갈아 끼우므로, 그 전에 떠난 작업의 응답은 버려진 객체에만 쓴다.
let uiMetricsReport: UiMetricsReportState = { reported: false, inFlight: null }

/** 칸 테두리 요소의 계산된 테두리 폭 넷(CSS px) + 정책 상수. 폭 하나라도 유한한 수로 읽히지 않으면 null. */
export function measureUiMetrics(borderEl: Element): UiMetrics | null {
  const cs = getComputedStyle(borderEl)
  const t = parseFloat(cs.borderTopWidth)
  const r = parseFloat(cs.borderRightWidth)
  const b = parseFloat(cs.borderBottomWidth)
  const l = parseFloat(cs.borderLeftWidth)
  if (![t, r, b, l].every(Number.isFinite)) return null
  return { frame_insets: { t, r, b, l }, min_pane_px: MIN_PANE_PX }
}

/**
 * 이 웹뷰가 아직 지표를 성공적으로 보내지 않았으면 `borderEl` 을 재서 보낸다. 보내는 중이면 그 작업을 함께 기다린다.
 * 유계 재시도(`retryAsync`) 끝에 실패하면 오류를 남기고 비워 두어 다음 호출(다음 잎 마운트)이 다시 시도한다.
 * 반환 프라미스는 거부하지 않는다.
 */
export function reportUiMetrics(borderEl: Element): Promise<void> {
  const st = uiMetricsReport
  if (st.reported) return Promise.resolve()
  if (st.inFlight !== null) return st.inFlight
  const metrics = measureUiMetrics(borderEl)
  if (metrics === null) {
    // 셸은 유한하지 않은 inset 을 거절한다 — 보내 봐야 재시도만 소진한다.
    console.error('[uiMetricsReport] 칸 테두리 폭을 수로 읽지 못해 보고하지 않는다:', borderEl)
    return Promise.resolve()
  }
  const job = retryAsync(() => invoke<void>('report_ui_metrics', { metrics }), {
    onRetry: (err, attempt) => console.warn(`[uiMetricsReport] report_ui_metrics 재시도 ${attempt}:`, err),
  }).then(
    () => {
      st.reported = true
      st.inFlight = null
    },
    err => {
      st.inFlight = null
      console.error('[uiMetricsReport] report_ui_metrics 최종 실패(재시도 소진):', err)
    },
  )
  st.inFlight = job
  return job
}

/** 테스트 전용 — 모듈 상태 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetUiMetricsReportForTest(): void {
  uiMetricsReport = { reported: false, inFlight: null }
}
