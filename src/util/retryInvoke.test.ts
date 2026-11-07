import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { retryAsync, RetryCancelledError } from './retryInvoke'

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

// ★unhandled-rejection avoidance★: attach the assertion (handler) as soon as the promise is made, and then advance the fake
//   timer. If the promise is made ahead and passed as an argument, the timer fires before the handler is attached and leaks as an unhandled
//   rejection, so here we await (handler-attached promise + runAllTimersAsync) together.
async function settleRejects(p: Promise<unknown>, matcher: unknown): Promise<void> {
  const assertion = expect(p).rejects
  const check =
    matcher instanceof Error || typeof matcher === 'string'
      ? assertion.toThrow(matcher as string | Error)
      : assertion.toBeInstanceOf(matcher as new (...a: never[]) => unknown)
  await Promise.all([check, vi.runAllTimersAsync()])
}

async function settleResolves<T>(p: Promise<T>): Promise<T> {
  const [result] = await Promise.all([p, vi.runAllTimersAsync()])
  return result
}

describe('retryAsync — 재시도 성공/실패/취소(ADR-0102)', () => {
  it('2번 reject 후 resolve → 성공값 반환(재시도가 회수)', async () => {
    let calls = 0
    const fn = vi.fn(async () => {
      calls += 1
      if (calls <= 2) throw new Error(`boom ${calls}`)
      return 'ok'
    })
    const result = await settleResolves(retryAsync(fn, { baseDelayMs: 10 }))
    expect(result).toBe('ok')
    expect(fn).toHaveBeenCalledTimes(3)
  })

  it('모든 시도 실패 → 마지막 에러를 throw(조용히 삼키지 않음)', async () => {
    let calls = 0
    const fn = vi.fn(async () => {
      calls += 1
      throw new Error(`fail ${calls}`)
    })
    await settleRejects(retryAsync(fn, { baseDelayMs: 5 }), 'fail 4')
    expect(fn).toHaveBeenCalledTimes(4)
  })

  it('attempts=1 → 재시도 없이 즉시 실패 throw', async () => {
    const fn = vi.fn(async () => {
      throw new Error('once')
    })
    await settleRejects(retryAsync(fn, { attempts: 1 }), 'once')
    expect(fn).toHaveBeenCalledTimes(1)
  })

  it('첫 시도부터 성공 → 재시도·대기 없음', async () => {
    const fn = vi.fn(async () => 42)
    const result = await settleResolves(retryAsync(fn))
    expect(result).toBe(42)
    expect(fn).toHaveBeenCalledTimes(1)
  })

  it('isCancelled → RetryCancelledError(정상 실패와 구분)', async () => {
    let cancelled = false
    const fn = vi.fn(async () => {
      cancelled = true
      throw new Error('boom')
    })
    await settleRejects(
      retryAsync(fn, { baseDelayMs: 5, isCancelled: () => cancelled }),
      RetryCancelledError,
    )
    expect(fn).toHaveBeenCalledTimes(1)
  })

  it('마지막 실패 시도 도중 취소 → 소진 throw 대신 RetryCancelledError(FIX-4)', async () => {
    // The old code looked at isCancelled only at the top of the loop, so it threw the backend error ('fail 2') as is → the call site
    //   logged a pointless final failure. FIX-4 re-checks right before the exhaustion throw and turns it into RetryCancelledError.
    let calls = 0
    let cancelled = false
    const fn = vi.fn(async () => {
      calls += 1
      if (calls === 2) cancelled = true // Imitates unmount
      throw new Error(`fail ${calls}`)
    })
    await settleRejects(
      retryAsync(fn, { attempts: 2, baseDelayMs: 5, isCancelled: () => cancelled }),
      RetryCancelledError,
    )
    expect(fn).toHaveBeenCalledTimes(2)
  })

  it('onRetry 는 실패 시도마다(마지막 제외) 호출된다', async () => {
    const onRetry = vi.fn()
    const fn = vi.fn(async () => {
      throw new Error('x')
    })
    await settleRejects(retryAsync(fn, { attempts: 3, baseDelayMs: 5, onRetry }), 'x')
    expect(onRetry).toHaveBeenCalledTimes(2)
    expect(onRetry).toHaveBeenNthCalledWith(1, expect.any(Error), 1)
    expect(onRetry).toHaveBeenNthCalledWith(2, expect.any(Error), 2)
  })
})
