// TRD S21-slot-docking §1-5: 창 크롬(탭바 상자 · 탭 머리 · 캔버스 원점) 보고의 보내는 쪽 — `report_dock_chrome`.
//   셸은 이 값으로 끌기 영역을 만든다(측정이라 version · 알림 · 저장 없음 — ADR-0227). 끌기 시작 때 보고를 기다리면 첫
//   영역이 늦으므로 평소에 보고한다. 언제 잴지(관측 · 탭 목록 · 떼어진 표시로 머리를 접은 뒤)는 부르는 쪽(`WindowLayout`)이
//   정하고, 여기서는 무엇을 언제 보낼지만 정한다.
// ★invoke 는 웹뷰당(= 창당) 한 번에 하나만 날린다★ — 이유는 `windowCanvasReport.ts` 와 같다(겹친 요청을 셸이 도착
//   역순으로 처리하면 낡은 크롬이 남는다 · WebView2 가 발행 순서대로 올리는지 모름). 날아가는 동안 온 값은 대기 칸
//   하나에 두고 새 값이 덮는다 — 응답 뒤 그 마지막 값만 보낸다. 대가: 응답이 끝내 안 오면 이 창의 보고는 거기서 멈춘다.
// 상태가 모듈 범위인 이유도 `windowCanvasReport.ts` 와 같다(훅 인스턴스가 바뀌어도 날아가는 invoke 를 안다 · HMR).

import { invoke } from '@tauri-apps/api/core'

import type { DockChrome } from '../../../../src-tauri/bindings/DockChrome'
import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

/** 뷰포트 CSS px 사각형 — `getBoundingClientRect()` 의 `DOMRect` 를 그대로 넘겨도 된다. */
export interface RectLike {
  left: number
  top: number
  right: number
  bottom: number
}

/** 탭 머리 하나의 측정. `collapsed` = 떼어진 표시로 접힌 머리(끄는 탭) — 보고에서 뺀다(TRD §1-5). */
export interface HeadMeasure {
  viewId: string
  rect: RectLike
  collapsed: boolean
}

/** 한 창의 크롬 측정 — `canvas` = 탭 캔버스 영역(캔버스 크기 보고와 같은 요소) · `tabBar` = 탭바 상자. */
export interface ChromeMeasure {
  canvas: RectLike
  tabBar: RectLike
  heads: HeadMeasure[]
}

/** 디바운스 기본값 — 캔버스 크기 보고(`WindowLayout.tsx` `CANVAS_REPORT_DEBOUNCE_MS`)와 같은 값. */
export const DOCK_CHROME_DEBOUNCE_MS = 100

/**
 * 측정 → 보고 값. 접힌 머리는 뺀다. 수 하나라도 유한하지 않으면 `null`(보내지 않는다 — JSON 이 `null` 로 실어 셸
 * 역직렬화가 거절한다). 탭바 밖으로 스크롤된 머리도 그대로 싣는다(셸 계약 — `DockChrome` 문서).
 */
export function dockChromeOf(m: ChromeMeasure): DockChrome | null {
  const heads = m.heads.filter(h => !h.collapsed)
  const rects = [m.canvas, m.tabBar, ...heads.map(h => h.rect)]
  if (!rects.every(r => [r.left, r.top, r.right, r.bottom].every(Number.isFinite))) return null
  return {
    canvas_origin: { x: m.canvas.left, y: m.canvas.top },
    tab_bar: rectCss(m.tabBar),
    tabs: heads.map(h => ({ view_id: h.viewId, rect: rectCss(h.rect) })),
  }
}

const rectCss = (r: RectLike): DockChrome['tab_bar'] => ({ x0: r.left, y0: r.top, x1: r.right, y1: r.bottom })

interface DockChromeReportState {
  /** 보낼 차례를 기다리는 값. 하나만 두고 새 값이 덮는다. */
  pending: DockChrome | null
  /** 셸이 성공 응답한 마지막 값 — 같은 값은 다시 보내지 않는다. */
  lastSent: DockChrome | null
  /** 지금 보내는 작업(재시도 대기 포함). 취소되면 null. */
  job: { chrome: DockChrome; gen: number } | null
  /** invoke 가 응답을 기다리는 중인가 — 재시도의 backoff 대기와는 따로 센다. */
  inAir: boolean
  /** 작업 세대 — 바뀌면 그 전 작업의 재시도는 더 시도하지 않는다. */
  gen: number
  /** `scheduleDockChromeReport` 의 디바운스 타이머와 그때 잴 함수. */
  timer: ReturnType<typeof setTimeout> | null
  measure: (() => DockChrome | null) | null
}

const createState = (): DockChromeReportState => ({
  pending: null,
  lastSent: null,
  job: null,
  inAir: false,
  gen: 0,
  timer: null,
  measure: null,
})

let report = createState()

/**
 * 잰 값을 곧바로 보낼 차례에 올린다(디바운스 없음) — 떼어진 표시로 머리를 접은 뒤처럼 늦으면 안 되는 자리에 쓴다.
 * 날아가는 invoke 가 있으면 그 응답 뒤에 마지막 값만 간다. 셸이 마지막으로 받은 값과 같으면 보내지 않는다.
 * 실패는 유계 재시도(`retryAsync`)하고, 그 사이 다른 값이 오면 옛 값의 재시도를 그친다.
 */
export function submitDockChrome(chrome: DockChrome): void {
  const st = report
  st.pending = chrome
  if (st.job && !sameChrome(st.job.chrome, chrome)) cancelJob(st)
  pump(st)
}

/**
 * `delayMs` 뒤 `measure` 를 불러 그 값을 보낸다 — 그 사이 다시 부르면 타이머를 다시 잡고 마지막 `measure` 만 쓴다.
 * 재는 것을 타이머 끝으로 미루므로 그때의 DOM 을 잰다. `measure` 가 `null` 을 주면 보내지 않는다.
 */
export function scheduleDockChromeReport(
  measure: () => DockChrome | null,
  delayMs: number = DOCK_CHROME_DEBOUNCE_MS,
): void {
  const st = report
  st.measure = measure
  if (st.timer !== null) clearTimeout(st.timer)
  st.timer = setTimeout(() => {
    st.timer = null
    const m = st.measure
    st.measure = null
    if (st !== report || m === null) return
    let chrome: DockChrome | null
    try {
      chrome = m()
    } catch (err) {
      console.error('[dockChromeReport] 크롬 측정 실패:', err)
      return
    }
    if (chrome !== null) submitDockChrome(chrome)
  }, delayMs)
}

/**
 * 관측을 뗄 때(창 레이아웃 언마운트) — 디바운스 · 대기 값을 버리고 재시도를 멈춘다. 날아가던 invoke 와 `lastSent` 는
 * 남긴다 — 그 응답이 셸의 현재 값을 맞추고, 다시 붙은 쪽도 그 응답을 기다려 한 번에 하나가 지켜진다.
 */
export function cancelDockChromeReport(): void {
  const st = report
  if (st.timer !== null) clearTimeout(st.timer)
  st.timer = null
  st.measure = null
  st.pending = null
  cancelJob(st)
}

/** 테스트 전용 — 모듈 상태 초기화(테스트 간 격리). 프로덕션 코드에서 호출 금지. */
export function __resetDockChromeReportForTest(): void {
  if (report.timer !== null) clearTimeout(report.timer)
  report = createState()
}

function cancelJob(st: DockChromeReportState): void {
  st.gen += 1
  st.job = null
}

function pump(st: DockChromeReportState): void {
  if (st.inAir || st.pending === null) return
  const chrome = st.pending
  st.pending = null
  if (st.job) {
    if (sameChrome(st.job.chrome, chrome)) return
    cancelJob(st)
  }
  if (st.lastSent !== null && sameChrome(st.lastSent, chrome)) return
  startJob(st, chrome)
}

function startJob(st: DockChromeReportState, chrome: DockChrome): void {
  const gen = ++st.gen
  st.job = { chrome, gen }
  const attempt = async (): Promise<void> => {
    st.inAir = true
    try {
      await invoke<void>('report_dock_chrome', { chrome })
      // 취소된 작업의 성공도 적는다 — 한 번에 하나라 이 응답 순서가 곧 셸 적용 순서이고, 셸은 지금 이 값을 쥐고 있다.
      st.lastSent = chrome
    } finally {
      st.inAir = false
      pump(st)
    }
  }
  // 크롬은 드물게 바뀌어 실패 뒤 다음 측정을 기다리면 영영 안 갈 수 있다(예: 부팅 직후 셸 탭 모델에 창이 아직 없다 —
  //   `report_dock_chrome` 의 `Err`). 그러면 끄는 동안 이 창에 탭바 · 칸 영역이 없다 — 스스로 재시도한다.
  void retryAsync(attempt, {
    isCancelled: () => st.gen !== gen,
    onRetry: (err, n) => {
      if (st.gen === gen) console.warn(`[dockChromeReport] report_dock_chrome 재시도 ${n}:`, err)
    },
  })
    .then(() => {
      if (st.gen === gen) st.job = null
    })
    .catch(err => {
      if (err instanceof RetryCancelledError || st.gen !== gen) return
      st.job = null
      console.error('[dockChromeReport] report_dock_chrome 최종 실패(재시도 소진):', err)
    })
}

function sameChrome(a: DockChrome, b: DockChrome): boolean {
  return (
    a.canvas_origin.x === b.canvas_origin.x &&
    a.canvas_origin.y === b.canvas_origin.y &&
    sameRect(a.tab_bar, b.tab_bar) &&
    a.tabs.length === b.tabs.length &&
    a.tabs.every((t, i) => t.view_id === b.tabs[i].view_id && sameRect(t.rect, b.tabs[i].rect))
  )
}

const sameRect = (a: DockChrome['tab_bar'], b: DockChrome['tab_bar']): boolean =>
  a.x0 === b.x0 && a.y0 === b.y0 && a.x1 === b.x1 && a.y1 === b.y1
