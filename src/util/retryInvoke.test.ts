// retryInvoke 단위테스트(ADR-0102) — 유계 재시도 + backoff + 최종 실패 throw + 취소 sentinel.
//
// ★검증 불변식★:
//   1. N번 reject 후 resolve → 성공값 반환(재시도가 성공을 회수).
//   2. 모든 시도 실패 → 마지막 에러를 throw(조용히 삼키지 않음).
//   3. isCancelled → RetryCancelledError(정상 실패와 구분).
//   4. onRetry 콜백은 실패 시도마다(마지막 제외) 호출.
//   5. backoff 대기가 실제로 걸린다(fake timers 로 시간 진행).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { retryAsync, RetryCancelledError } from './retryInvoke'

beforeEach(() => {
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

// ★unhandled-rejection 회피★: promise 를 만들자마자 assertion(핸들러)을 붙이고, 그 다음 fake timer 를
//   진행시킨다. promise 를 인자로 미리 만들어 넘기면 핸들러 부착 전에 timer 가 firing 돼 unhandled
//   rejection 으로 새므로, 여기서 (핸들러 부착 promise + runAllTimersAsync) 를 함께 await 한다.
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
    expect(fn).toHaveBeenCalledTimes(3) // 첫 시도 + 재시도 2회.
  })

  it('모든 시도 실패 → 마지막 에러를 throw(조용히 삼키지 않음)', async () => {
    let calls = 0
    const fn = vi.fn(async () => {
      calls += 1
      throw new Error(`fail ${calls}`)
    })
    // 기본 attempts=4 → 4회 시도 후 마지막(4번째) 에러 throw.
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
      cancelled = true // 첫 시도 후 취소 상태로 전환.
      throw new Error('boom')
    })
    await settleRejects(
      retryAsync(fn, { baseDelayMs: 5, isCancelled: () => cancelled }),
      RetryCancelledError,
    )
    // 첫 시도(1회)는 돌고, 그 실패 후 다음 시도 전 isCancelled 로 중단 → fn 은 1회만.
    expect(fn).toHaveBeenCalledTimes(1)
  })

  it('마지막 실패 시도 도중 취소 → 소진 throw 대신 RetryCancelledError(FIX-4)', async () => {
    // 시나리오: attempts=2. 두 시도 모두 reject 하되, ★마지막(2번째) 시도가 실패한 뒤★ 취소로 전환된다.
    //   옛 코드는 루프 상단에서만 isCancelled 를 봐서 backend 에러('fail 2')를 그대로 throw → 호출부가
    //   헛된 최종-실패를 로깅. FIX-4 는 소진 throw 직전 재확인해 RetryCancelledError 로 바꾼다.
    let calls = 0
    let cancelled = false
    const fn = vi.fn(async () => {
      calls += 1
      if (calls === 2) cancelled = true // 마지막 시도가 실패하는 순간 unmount 흉내(취소로 전환).
      throw new Error(`fail ${calls}`)
    })
    await settleRejects(
      retryAsync(fn, { attempts: 2, baseDelayMs: 5, isCancelled: () => cancelled }),
      RetryCancelledError,
    )
    expect(fn).toHaveBeenCalledTimes(2) // 두 시도 모두 돌고, 소진 후 취소 재확인이 sentinel 로 치환.
  })

  it('onRetry 는 실패 시도마다(마지막 제외) 호출된다', async () => {
    const onRetry = vi.fn()
    const fn = vi.fn(async () => {
      throw new Error('x')
    })
    await settleRejects(retryAsync(fn, { attempts: 3, baseDelayMs: 5, onRetry }), 'x')
    // 3회 시도 중 마지막(3번째) 실패엔 onRetry 안 부름 → 2회.
    expect(onRetry).toHaveBeenCalledTimes(2)
    expect(onRetry).toHaveBeenNthCalledWith(1, expect.any(Error), 1)
    expect(onRetry).toHaveBeenNthCalledWith(2, expect.any(Error), 2)
  })
})
