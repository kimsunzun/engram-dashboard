// latestValueReporter 단위테스트 — 한 번에 하나 · 마지막 값이 덮음 · 같은 값 안 보냄 · 관측이 낡은 값을 거둠 · 재시도 · 떼기 ·
//   디바운스. 캔버스 크기 보고의 흐름은 `WindowLayout.test.tsx` 가 잰다.

import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'

import { LatestValueReporter, REPORT_DEBOUNCE_MS } from './latestValueReporter'

interface V {
  n: number
}
const v = (n: number): V => ({ n })

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}

let send: Mock<(value: V) => Promise<unknown>>
let accepted: V[]
let rep: LatestValueReporter<V>

const sentNs = (): number[] => send.mock.calls.map(c => c[0].n)

/** 다음 send 하나를 시험이 손수 끝낸다. */
function holdSend(): { finish: () => void; fail: (e: unknown) => void } {
  const h = { finish: () => {}, fail: (_e: unknown) => {} }
  send.mockImplementationOnce(
    () =>
      new Promise<void>((res, rej) => {
        h.finish = res
        h.fail = rej
      }),
  )
  return h
}

beforeEach(() => {
  send = vi.fn(async (_value: V): Promise<unknown> => undefined)
  accepted = []
  rep = new LatestValueReporter<V>({
    send: value => send(value),
    same: (a, b) => a.n === b.n,
    log: 'test',
    describe: value => `report(${value.n})`,
    onAccepted: value => accepted.push(value),
  })
})
afterEach(() => {
  rep.release()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('LatestValueReporter', () => {
  it('submit 은 곧바로 보내고, 성공하면 lastSent · onAccepted — 같은 값은 다시 보내지 않는다', async () => {
    const a = v(1)
    rep.submit(a)
    await ticks()
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toBe(a)
    expect(accepted).toEqual([a])

    rep.submit(v(1))
    await ticks()
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toBe(a)
    expect(accepted).toHaveLength(1)
  })

  it('한 번에 하나 — 날아가는 동안 온 값은 마지막 하나만 응답 뒤에 보낸다', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(2))
    rep.submit(v(3))
    await ticks()
    expect(sentNs()).toEqual([1])

    first.finish()
    await ticks()
    expect(sentNs()).toEqual([1, 3])
    expect(rep.lastSent).toEqual(v(3))
    expect(accepted).toEqual([v(1), v(3)])
  })

  it('날아가는 값과 같은 값이 차례를 기다리면 응답 뒤에 다시 보내지 않는다', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(1))
    first.finish()
    await ticks()
    expect(sentNs()).toEqual([1])
  })

  it('취소된 값의 성공도 lastSent 가 된다 — 응답 순서가 곧 셸 적용 순서다', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.observe(v(2))
    first.finish()
    await ticks()
    expect(rep.lastSent).toEqual(v(1))
    expect(sentNs()).toEqual([1])
  })

  it('실패하면 스스로 재시도하고 성공한 값만 lastSent 가 된다 — 소진되면 console.error 한 번', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    send.mockRejectedValue(new Error('refused'))

    rep.submit(v(1))
    await vi.advanceTimersByTimeAsync(150 + 300 + 600 + 10)
    expect(sentNs()).toEqual([1, 1, 1, 1])
    expect(rep.lastSent).toBeNull()
    expect(err).toHaveBeenCalledTimes(1)
    expect(err).toHaveBeenCalledWith('[test] report(1) 최종 실패(재시도 소진):', expect.any(Error))

    // 실패한 값은 lastSent 가 아니므로 같은 값이라도 다시 나간다.
    send.mockReset()
    send.mockImplementation(async () => undefined)
    rep.submit(v(1))
    await ticks()
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toEqual(v(1))
  })

  it('observe 가 다른 값이면 옛 값의 재시도를 멈추고 대기 값을 버린다 — 취소는 실패 로그가 아니다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    send.mockRejectedValueOnce(new Error('refused'))

    rep.submit(v(1))
    await ticks()
    rep.observe(v(2))
    await vi.advanceTimersByTimeAsync(3000)
    expect(sentNs()).toEqual([1])
    expect(err).not.toHaveBeenCalled()

    const held = holdSend()
    rep.submit(v(3))
    rep.submit(v(4))
    rep.observe(v(5))
    held.finish()
    await ticks()
    expect(sentNs()).toEqual([1, 3])
  })

  it('observe 가 같은 값이면 아무것도 거두지 않는다', async () => {
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(2))
    rep.observe(v(2))
    first.finish()
    await ticks()
    expect(sentNs()).toEqual([1, 2])
  })

  it('release — 디바운스 · 대기 값 · 재시도를 멈추고, 날아가던 응답은 lastSent 를 맞춘다', async () => {
    vi.useFakeTimers()
    const first = holdSend()
    rep.submit(v(1))
    rep.submit(v(2))
    rep.schedule(() => v(3))
    rep.release()
    first.finish()
    await vi.advanceTimersByTimeAsync(3000)
    expect(sentNs()).toEqual([1])
    expect(rep.lastSent).toEqual(v(1))
  })

  it('schedule — 디바운스 동안 다시 부르면 마지막 measure 만 타이머 끝에 부르고, null 이면 보내지 않는다', async () => {
    vi.useFakeTimers()
    const early = vi.fn(() => v(1))
    rep.schedule(early)
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS - 1)
    rep.schedule(() => v(2))
    await vi.advanceTimersByTimeAsync(REPORT_DEBOUNCE_MS - 1)
    expect(sentNs()).toEqual([])
    await vi.advanceTimersByTimeAsync(1)
    expect(early).not.toHaveBeenCalled()
    expect(sentNs()).toEqual([2])

    rep.schedule(() => null, 10)
    await vi.advanceTimersByTimeAsync(10)
    expect(sentNs()).toEqual([2])
  })
})
