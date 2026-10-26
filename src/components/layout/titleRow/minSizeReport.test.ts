// TRD S26-window-chrome §7-7 의 최소 창 크기 보고 — 한 번에 하나 · 마지막 값만 · 같은 값 안 보냄 · 실패 뒤 다시 맡음 · 답 없는
//   시도의 시한 · 보낼 때마다 새 순번(`seq`) · 페이지마다 `epoch` · 첫 화면 알림이 기다리는 끝(상한 포함). invoke 는 가짜다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.hoisted(() => vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args) }))

import {
  __resetMinSizeReportForTest,
  MIN_SIZE_ATTEMPT_TIMEOUT_MS,
  MIN_SIZE_WAIT_CAP_MS,
  minWindowSizeSettled,
  reportMinWindowSize,
} from './minSizeReport'

// `retryAsync` 기본의 첫 backoff — 시한에 놓인 시도 뒤 다음 차례는 이만큼 더 늦게 나간다.
const FIRST_BACKOFF_MS = 150

interface SentArgs {
  width: number
  height: number
  seq: number
  epoch: number
}

const args = (): SentArgs[] => invokeMock.mock.calls.map(c => c[1] as SentArgs)
const sent = () => args().map(({ width, height }) => ({ width, height }))
const seqs = () => args().map(a => a.seq)
const flush = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

/** 다음 보고 하나를 붙든다 — 돌려준 함수가 답한다(`fail` 이면 거절). */
function holdNext(): (fail?: boolean) => Promise<void> {
  let answer: (fail: boolean) => void = () => {}
  invokeMock.mockImplementationOnce(
    () => new Promise((resolve, reject) => (answer = fail => (fail ? reject(new Error('거절(시험)')) : resolve(undefined)))),
  )
  return async (fail = false) => {
    answer(fail)
    await flush()
  }
}

// 이 페이지의 `performance.timeOrigin` 을 갈아 끼운다 — `afterEach` 가 원래 속성으로 돌린다.
const ownTimeOrigin = Object.getOwnPropertyDescriptor(performance, 'timeOrigin')
function setTimeOrigin(ms: number): void {
  Object.defineProperty(performance, 'timeOrigin', { value: ms, configurable: true })
}

beforeEach(() => {
  __resetMinSizeReportForTest()
  invokeMock.mockReset()
  invokeMock.mockImplementation(async () => undefined)
})

afterEach(() => {
  if (ownTimeOrigin) Object.defineProperty(performance, 'timeOrigin', ownTimeOrigin)
  else delete (performance as { timeOrigin?: number }).timeOrigin
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('reportMinWindowSize', () => {
  it('report_window_min_width { width, height, seq, epoch } 로 보낸다 — 첫 순번 1 · epoch = 페이지 timeOrigin 의 정수 ms · 같은 값은 다시 보내지 않는다', async () => {
    setTimeOrigin(1_761_000_000_123.75)
    __resetMinSizeReportForTest()
    reportMinWindowSize({ width: 286, height: 64 })
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    expect(invokeMock.mock.calls).toEqual([
      ['report_window_min_width', { width: 286, height: 64, seq: 1, epoch: 1_761_000_000_123 }],
    ])
  })

  it('epoch 는 페이지마다 하나 — 다시 읽으면(초기화) 그 페이지의 epoch 에서 순번이 1 부터 다시 간다', async () => {
    setTimeOrigin(1_761_000_000_000.5)
    __resetMinSizeReportForTest()
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()

    setTimeOrigin(1_761_000_005_000.25)
    __resetMinSizeReportForTest()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    expect(args().map(a => [a.epoch, a.seq])).toEqual([
      [1_761_000_000_000, 1],
      [1_761_000_000_000, 2],
      [1_761_000_005_000, 1],
    ])
  })

  it('한 번에 하나 — 날아가는 동안 맡은 값은 마지막 하나만 응답 뒤에 보낸다', async () => {
    const answer = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    reportMinWindowSize({ width: 288, height: 64 })
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    expect(sent()).toEqual([{ width: 286, height: 64 }])

    await answer()
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2])
  })

  it('닿은 값과 같은 값은 — 그 사이 맡은 다른 값을 보내기 전에 밀렸으면 — 보내지 않는다', async () => {
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    reportMinWindowSize({ width: 295, height: 64 })
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    expect(sent()).toEqual([{ width: 286, height: 64 }])
  })

  it('앞서 닿은 값으로 돌아와도 그 사이 다른 값을 보냈으면 다시 보낸다 — 그 값의 답(닿음 · 거절)을 따지지 않는다', async () => {
    reportMinWindowSize({ width: 286, height: 64 })
    await flush()
    let answer = holdNext()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    reportMinWindowSize({ width: 286, height: 64 })
    await answer()
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2, 3])

    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockClear()
    answer = holdNext()
    reportMinWindowSize({ width: 295, height: 64 })
    await flush()
    reportMinWindowSize({ width: 286, height: 64 })
    await answer(true)
    await minWindowSizeSettled()
    expect(sent()).toEqual([
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([4, 5])
  })

  it('답이 오지 않는 시도는 시한에 놓는다 — 더 새 값이 유계 시간 안에 새 순번으로 나가고 · 늦게 온 옛 순번의 답은 「닿음」을 바꾸지 않는다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const late = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    reportMinWindowSize({ width: 295, height: 64 })
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS - 1)
    expect(sent()).toEqual([{ width: 286, height: 64 }])
    await vi.advanceTimersByTimeAsync(1 + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2])

    await late()
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS * 10)
    expect(sent()).toHaveLength(2)

    // 늦은 286 이 「닿음」으로 적혔으면 이 보고를 건너뛴다.
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2, 3])
  })

  it('늦은 답이 섞여도 가장 큰 순번이 마지막 값을 싣고 · 그 순번의 답만 「닿음」이 된다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const lateFirst = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    const lateSecond = holdNext()
    reportMinWindowSize({ width: 288, height: 64 })
    reportMinWindowSize({ width: 295, height: 64 })
    // 286 이 시한에 놓이고 295 가 나간다(288 은 밀렸다) — 그 295 도 시한에 놓여 다시 나간다.
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 295, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2, 3])

    await lateSecond()
    await lateFirst()
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS * 10)
    expect(sent()).toHaveLength(3)

    // 「닿음」 = 순번 3 의 295 그대로 — 늦은 순번 1 의 286 으로 되돌아갔으면 이 보고를 건너뛴다.
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    expect(sent()).toHaveLength(4)
    expect(args()[3]).toMatchObject({ width: 286, height: 64, seq: 4 })
  })

  it('시한에 놓인 시도의 답이 재시도 전에 오면 — 그 순번이 아직 마지막이라 「닿음」으로 치고 다시 보내지 않는다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const late = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS)
    await late()
    await vi.advanceTimersByTimeAsync(FIRST_BACKOFF_MS * 10)
    expect(sent()).toEqual([{ width: 286, height: 64 }])
  })

  it('더 새 값이 없으면 시한에 놓인 같은 값을 새 순번으로 다시 보낸다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 286, height: 64 },
    ])
    expect(seqs()).toEqual([1, 2])
  })

  it('시한을 넘긴 시도 뒤에는 셸의 값을 모름으로 친다 — 앞서 닿은 값으로 돌아와도 다시 보낸다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    holdNext()
    reportMinWindowSize({ width: 295, height: 64 })
    await vi.advanceTimersByTimeAsync(0)
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.advanceTimersByTimeAsync(MIN_SIZE_ATTEMPT_TIMEOUT_MS + FIRST_BACKOFF_MS)
    expect(sent()).toEqual([
      { width: 286, height: 64 },
      { width: 295, height: 64 },
      { width: 286, height: 64 },
    ])
  })

  it('재시도가 소진되면 오류를 남기고 · 같은 값을 다시 맡으면 다시 보낸다 — 재시도마다 새 순번', async () => {
    vi.useFakeTimers()
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockImplementation(async () => {
      throw new Error('거절(시험)')
    })
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.runAllTimersAsync()
    expect(seqs()).toEqual([1, 2, 3, 4])
    expect(error).toHaveBeenCalledTimes(1)

    invokeMock.mockImplementation(async () => undefined)
    reportMinWindowSize({ width: 286, height: 64 })
    await vi.runAllTimersAsync()
    expect(seqs()).toEqual([1, 2, 3, 4, 5])
  })
})

describe('minWindowSizeSettled — 첫 화면 알림이 기다리는 끝', () => {
  it('맡은 보고가 없으면 곧바로 · 있으면 그 응답 뒤에 풀린다', async () => {
    let settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await flush()
    expect(settled).toBe(true)

    const answer = holdNext()
    reportMinWindowSize({ width: 286, height: 64 })
    settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await flush()
    expect(settled).toBe(false)
    await answer()
    expect(settled).toBe(true)
  })

  it('응답이 오지 않아도 상한 뒤에는 풀린다 — 첫 화면 알림을 막지 않는다', async () => {
    vi.useFakeTimers()
    invokeMock.mockImplementation(() => new Promise(() => {}))
    reportMinWindowSize({ width: 286, height: 64 })
    let settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await vi.advanceTimersByTimeAsync(MIN_SIZE_WAIT_CAP_MS - 1)
    expect(settled).toBe(false)
    await vi.advanceTimersByTimeAsync(1)
    expect(settled).toBe(true)
  })

  it('보고가 거절되면 재시도를 다 기다리지 않는다 — 상한 안에 풀린다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.spyOn(console, 'error').mockImplementation(() => {})
    invokeMock.mockImplementation(async () => {
      throw new Error('거절(시험)')
    })
    reportMinWindowSize({ width: 286, height: 64 })
    let settled = false
    void minWindowSizeSettled().then(() => (settled = true))
    await vi.advanceTimersByTimeAsync(MIN_SIZE_WAIT_CAP_MS)
    expect(settled).toBe(true)
  })
})
