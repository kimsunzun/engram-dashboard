// ADR-0227: 창 캔버스 크기 보고의 보내는 쪽. 관측·디바운스는 `WindowLayout.tsx` 의 훅이 하고, 여기서는
//   무엇을 언제 `report_window_canvas` 로 보낼지 정하고, 셸이 마지막으로 받아들인 값을 내놓는다
//   (`subscribeCanvasReport` — 구분선 드래그의 허용 범위가 셸과 같은 입력으로 읽는다).
// ★invoke 는 웹뷰당(= 창당) 한 번에 하나만 날린다★: 겹친 두 요청을 셸이 도착 역순으로 처리하면 저장된
//   캔버스가 다음 리사이즈까지 낡은 채 남는다. WebView2 가 겹친 요청을 JS 발행 순서대로 올리는지는 문서에
//   없다(모름). 하나씩이면 응답 순서가 곧 셸 적용 순서라, 취소된 작업의 성공도 셸의 현재 값으로 믿을 수 있다.
//   대가: 응답이 끝내 안 오면 이 창의 보고는 거기서 멈춘다.
// 상태가 모듈 범위인 이유: 같은 웹뷰에서도 훅 인스턴스는 바뀐다(RootErrorBoundary 다시 그리기·HMR 재마운트).
//   인스턴스 상태면 새 인스턴스가 옛 인스턴스의 날아가는 invoke 를 모른 채 하나를 더 날린다.
// 컴포넌트 파일과 떼어 둔 이유: HMR 은 고친 모듈만 다시 실행하므로 `WindowLayout.tsx` 를 고쳐도 이 상태는
//   산다. 컴포넌트 파일이 컴포넌트 아닌 것(아래 테스트용 초기화)을 내보내면 Fast Refresh 경계도 깨진다.
//   한계: 이 파일 자체를 고치면 상태가 새로 생긴다(개발 중에만).

import { invoke } from '@tauri-apps/api/core'

import { retryAsync, RetryCancelledError } from '../../util/retryInvoke'

/** 창 캔버스 크기(CSS px 정수) — 셸 `report_window_canvas` 의 인자. */
export interface CanvasPx {
  w: number
  h: number
}

/** 한 웹뷰(= 한 창)가 공유하는 보고 상태. */
export interface CanvasReportState {
  /** 디바운스를 마치고 보낼 차례를 기다리는 값. 하나만 두고 새 값이 덮는다. */
  want: CanvasPx | null
  /** 셸이 성공 응답한 마지막 값. */
  lastSent: CanvasPx | null
  /** 지금 보내는 작업(재시도 대기 포함). 취소되면 null. */
  job: { size: CanvasPx; gen: number } | null
  /** invoke 가 응답을 기다리는 중인가 — 재시도의 backoff 대기와는 따로 센다. */
  inAir: boolean
  /** 작업 세대 — 바뀌면 그 전 작업의 재시도는 더 시도하지 않는다. */
  gen: number
}

const sameCanvas = (a: CanvasPx, b: CanvasPx): boolean => a.w === b.w && a.h === b.h
const isEmptyCanvas = (s: CanvasPx): boolean => s.w === 0 || s.h === 0

const createCanvasReportState = (): CanvasReportState => ({
  want: null,
  lastSent: null,
  job: null,
  inAir: false,
  gen: 0,
})

let canvasReport = createCanvasReportState()

// 구독 하나마다 항목을 따로 둔다 — 같은 함수를 두 번 걸어도 한쪽 해제가 다른 쪽을 지우지 않는다.
const canvasListeners = new Set<{ cb: (size: CanvasPx | null) => void }>()

/** 훅이 관측을 붙일 때 잡는 이 웹뷰의 보고 상태. */
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

// 알림은 invoke 시도 안에서 나간다 — 구독자의 throw 가 새면 보고 실패로 셈돼 같은 값을 다시 보낸다.
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
  canvasReport = createCanvasReportState()
  if (had) notifyCanvas(null)
}

/** 반올림한 관측 하나를 알린다(디바운스 전). */
export function observeCanvasSize(st: CanvasReportState, size: CanvasPx): void {
  // 새 관측이 보내는 값·대기 값을 낡게 만들면 디바운스를 기다리지 않고 거둔다 — 기다리면 그 사이 옛
  //   재시도나 응답 뒤 차례가 낡은 값을 한 번 더 보낸다. 0 크기는 보낼 후보가 아니라 옛 값을 낡게 만들지 않는다.
  if (isEmptyCanvas(size)) return
  if (st.job && !sameCanvas(st.job.size, size)) cancelCanvasJob(st)
  if (st.want && !sameCanvas(st.want, size)) st.want = null
}

/** 디바운스를 마친 측정을 보낼 차례에 올린다. */
export function submitCanvasSize(st: CanvasReportState, size: CanvasPx): void {
  // 0×0 RO 콜백은 실재한다(TerminalSlot.tsx 도 0×0 을 거른다). 셸도 무시하지만 화면이 보내지 않는다.
  if (isEmptyCanvas(size)) return
  st.want = size
  pumpCanvasReport(st)
}

/** 관측을 뗄 때 — 대기 값을 버리고 재시도를 멈춘다. */
export function releaseCanvasReport(st: CanvasReportState): void {
  st.want = null
  cancelCanvasJob(st)
  // inAir·lastSent 는 남긴다 — 날아가던 invoke 는 떼어도 셸에 닿고 그 응답이 lastSent 를 맞춘다. 다시
  //   붙거나 새 인스턴스가 붙어도 그 응답을 기다리므로 한 번에 하나가 지켜진다.
}

function cancelCanvasJob(st: CanvasReportState): void {
  st.gen += 1
  st.job = null
}

/** 보낼 차례를 기다리는 값을 내보낸다. 날아가는 invoke 가 있으면 그 응답 뒤(`startCanvasJob`)에 다시 불린다. */
function pumpCanvasReport(st: CanvasReportState): void {
  if (st.inAir || !st.want) return
  const size = st.want
  st.want = null
  if (st.job) {
    if (sameCanvas(st.job.size, size)) return
    cancelCanvasJob(st)
  }
  if (st.lastSent && sameCanvas(st.lastSent, size)) return
  startCanvasJob(st, size)
}

function startCanvasJob(st: CanvasReportState, size: CanvasPx): void {
  const gen = ++st.gen
  st.job = { size, gen }
  const attempt = async (): Promise<void> => {
    st.inAir = true
    try {
      await invoke<void>('report_window_canvas', { w: size.w, h: size.h })
      // 같은 값이면 객체를 갈지 않는다 — 구독자가 읽는 값의 참조가 알림 없이 바뀌지 않게.
      if (st.lastSent === null || !sameCanvas(st.lastSent, size)) {
        st.lastSent = size
        // 테스트 초기화로 버려진 상태의 늦은 응답은 지금 상태의 구독자에게 알리지 않는다.
        if (st === canvasReport) notifyCanvas(size)
      }
    } finally {
      st.inAir = false
      pumpCanvasReport(st)
    }
  }
  // RO 는 크기가 바뀔 때만 울리므로 실패 뒤 다음 관측을 기다리면 영영 안 갈 수 있다 — 스스로 재시도한다.
  void retryAsync(attempt, {
    isCancelled: () => st.gen !== gen,
    onRetry: (err, n) => {
      if (st.gen === gen) {
        console.warn(`[WindowLayout] report_window_canvas(${size.w}x${size.h}) 재시도 ${n}:`, err)
      }
    },
  })
    .then(() => {
      if (st.gen === gen) st.job = null
    })
    .catch(err => {
      if (err instanceof RetryCancelledError || st.gen !== gen) return
      st.job = null
      console.error(
        `[WindowLayout] report_window_canvas(${size.w}x${size.h}) 최종 실패(재시도 소진):`,
        err,
      )
    })
}
