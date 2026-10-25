// TRD S26-window-chrome §7-7: 최소 창 크기 보고(`report_window_min_width`) — 이 창의 제목 줄이 센 값(`metrics.ts`
//   `minWindowSize`)을 셸에 보낸다. 셸은 그 값을 OS 최소 크기로 입히고 열 때 한 번 넓힌다 — 그래서 첫 화면 알림이 이 보고를
//   기다린다(`minWindowSizeSettled` — 첫 화면이 보이기 전에 넓히기가 끝나게). 지금 셸은 적기만 한다 — 입히기 · 넓히기는
//   U8b(`chrome::report_min_size` 의 할 일)라 그때까지 이 기다림은 배선뿐이다.
// 상태가 모듈 범위인 이유: 보고는 웹뷰(창) 하나에 하나이고, 다시 읽기가 모듈을 새로 실행하며 함께 초기화된다(`uiMetricsReport.ts`
//   와 같다).
// ★한 번에 하나 · 마지막 값만★ — 보내는 동안 맡은 값은 그 응답 뒤에 보내고, 그사이 더 새 값을 맡으면 옛 값은 보내지 않는다.
// ★응답이 끝내 오지 않는 시도는 시한(`MIN_SIZE_ATTEMPT_TIMEOUT_MS`)에 놓는다★ — 놓지 않으면 그 뒤 보고가 전부 그 뒤에 막힌다.
//   놓은 시도는 다음 시도보다 늦게 셸에 닿을 수 있다(`util/latestValueReporter.ts` 머리) — 그래서 보낼 때마다(재시도 포함) 새
//   순번 `seq` 와 이 페이지의 `epoch` 를 싣고, 셸은 더 옛 `epoch` 이거나 같은 `epoch` 에서 받아들인 순번보다 새 것이
//   아니면 버린다(`chrome::book` `record_min`). 닿는 차례와 무관하게 셸에는 가장 새 페이지의 가장 큰 순번 = 마지막으로 보낸
//   값이 남는다.
// 셸의 지금 값으로 믿는 것 = 가장 큰 순번이 성공 응답한 그 값뿐이다 — 새 순번을 보내는 순간 모름이 되고, 더 옛 순번의 답은
//   (시한 뒤에 왔어도) 아무것도 바꾸지 않는다.

import { invoke } from '@tauri-apps/api/core'

import { retryAsync, RetryCancelledError } from '../../../util/retryInvoke'

export interface MinWindowSize {
  width: number
  height: number
}

// 첫 화면 알림이 보고를 기다리는 상한 — 셸의 보이기 시한(1500 ms)보다 넉넉히 짧다. 보통 답은 몇 ms 이고, 첫 실패 뒤 재시도
//   한 번(150 ms 뒤 — `retryAsync` 기본)까지는 기다린다. 넘기면 첫 화면을 알리고 보고는 뒤에서 마저 한다 — 그 창은 보인 뒤
//   넓혀질 수 있다(TRD T1 로 받아들인 것과 같은 모습).
export const MIN_SIZE_WAIT_CAP_MS = 300

// 한 시도가 답을 기다리는 상한 — 넘기면 그 시도를 실패로 치고 재시도한다(더 새 값을 맡았으면 재시도 대신 그 값을 보낸다). 보통
//   답은 몇 ms 라, 이 값은 막힌 응답에서 보고 줄을 되찾는 데만 쓰인다. 짧으면 느린 응답 하나에도 같은 값을 겹쳐 보낸다.
export const MIN_SIZE_ATTEMPT_TIMEOUT_MS = 1000

interface MinSizeReportState {
  /**
   * 이 페이지의 표식 = `Math.floor(performance.timeOrigin)`(페이지를 읽은 벽시계 ms) — 다시 읽을수록 커져 셸이 페이지의
   * 앞뒤를 가른다(옛 페이지의 늦은 보고가 새 페이지 값을 덮지 못한다). 더 큰 `epoch` 가 오면 셸은 순번을 새로 센다(다시 읽으면
   * `seq` 가 1 부터 다시 간다). 두 페이지의 `timeOrigin` 이 같은 ms 면 한 페이지로 섞인다 — 다루지 않는다.
   */
  epoch: number
  /**
   * 마지막으로 보낸 순번 — 보낼 때마다(재시도 포함) 하나 오른다. 첫 순번 = 1. u32 를 넘기는 경우는 다루지 않는다 — 한
   * 페이지에서 42억 번 보내지 않는다.
   */
  seq: number
  /** 마지막으로 맡은 값 — 같은 값은 다시 맡지 않는다. 최종 실패하면 비워 같은 값을 다시 맡는다. */
  latest: MinWindowSize | null
  /** 맡은 값의 세대 — 값을 맡을 때마다 오른다. 세대가 지난 보고는 더 시도하지 않는다. */
  gen: number
  /** 셸의 지금 값으로 믿는 것 = 순번 `seq` 가 성공 응답한 값. 그 순번의 답을 아직 못 받았으면 모름(`null`). */
  delivered: MinWindowSize | null
  /** 맡은 보고가 다 끝나면(성공 · 최종 실패 · 더 새 값에 밀림) 풀린다. 거부하지 않는다. */
  chain: Promise<void>
}

const fresh = (): MinSizeReportState => ({
  epoch: Math.floor(performance.timeOrigin),
  seq: 0,
  latest: null,
  gen: 0,
  delivered: null,
  chain: Promise.resolve(),
})

// 초기화가 상태 객체를 통째로 갈아 끼우므로, 그 전에 맡은 보고의 응답은 버려진 객체에만 쓴다.
let report: MinSizeReportState = fresh()

const same = (a: MinWindowSize, b: MinWindowSize): boolean => a.width === b.width && a.height === b.height

/** 지금 제목 줄의 최소 창 크기를 맡긴다 — 마지막으로 맡은 값과 같으면 아무것도 하지 않는다. */
export function reportMinWindowSize(size: MinWindowSize): void {
  const st = report
  if (st.latest !== null && same(st.latest, size)) return
  st.latest = size
  const gen = ++st.gen
  st.chain = st.chain.then(() => send(st, size, gen))
}

async function send(st: MinSizeReportState, size: MinWindowSize, gen: number): Promise<void> {
  if (st.gen !== gen) return
  try {
    await retryAsync(() => attempt(st, size), {
      isCancelled: () => st.gen !== gen,
      onRetry: (err, n) => {
        if (st.gen === gen) console.warn(`[minSizeReport] report_window_min_width(${size.width}x${size.height}) 재시도 ${n}:`, err)
      },
    })
  } catch (err) {
    if (err instanceof RetryCancelledError) return
    if (st.gen === gen) st.latest = null
    console.error(`[minSizeReport] report_window_min_width(${size.width}x${size.height}) 최종 실패(재시도 소진):`, err)
  }
}

/**
 * 한 번 보낸다 — 셸의 지금 값이 이미 `size` 면 보내지 않는다. 새 순번을 싣고, `MIN_SIZE_ATTEMPT_TIMEOUT_MS` 안에 답이 없으면
 * 거절한다. 성공 답은 그 순번이 아직 마지막 순번일 때만 「닿음」으로 적는다(시한 뒤에 온 답도 같다).
 */
function attempt(st: MinSizeReportState, size: MinWindowSize): Promise<void> {
  if (st.delivered !== null && same(st.delivered, size)) return Promise.resolve()
  const seq = ++st.seq
  st.delivered = null
  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`응답 없음(${MIN_SIZE_ATTEMPT_TIMEOUT_MS} ms)`)), MIN_SIZE_ATTEMPT_TIMEOUT_MS)
    const fail = (err: unknown): void => {
      clearTimeout(timer)
      reject(err)
    }
    try {
      invoke<void>('report_window_min_width', { width: size.width, height: size.height, seq, epoch: st.epoch }).then(() => {
        if (st.seq === seq) st.delivered = size
        clearTimeout(timer)
        resolve()
      }, fail)
    } catch (err) {
      fail(err)
    }
  })
}

/**
 * 지금까지 맡은 보고가 다 끝나면 — 늦어도 `MIN_SIZE_WAIT_CAP_MS` 뒤 — 풀린다(거부하지 않는다). 맡은 것이 없으면 곧바로 풀린다.
 * 첫 화면 알림(`theme/firstPaint.ts`)을 이 뒤에 보낸다.
 */
export function minWindowSizeSettled(): Promise<void> {
  const chain = report.chain
  return new Promise(resolve => {
    const timer = setTimeout(resolve, MIN_SIZE_WAIT_CAP_MS)
    void chain.then(() => {
      clearTimeout(timer)
      resolve()
    })
  })
}

/** 테스트 전용 — 모듈 상태 초기화(페이지 다시 읽기와 같다 — `epoch` 를 `performance.timeOrigin` 에서 다시 읽음 · 순번 0 · 앞 상태의 재시도도 멈춘다). 프로덕션 코드에서 호출 금지. */
export function __resetMinSizeReportForTest(): void {
  report.gen += 1
  report = fresh()
}
