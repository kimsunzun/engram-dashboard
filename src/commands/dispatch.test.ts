// ADR-0055 (FIX-3/FIX-4)
// ★run() itself is not touched (no loss at await call sites)★ — registry.test.ts guards that contract.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { fireAndForget } from './dispatch'
import { __resetRegistryForTest, register } from './registry'

beforeEach(() => {
  __resetRegistryForTest()
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('fireAndForget (ADR-0055 fire-and-forget 안전망)', () => {
  it('동기 handler 를 실행하고 인자(단일 가방)를 전달한다', () => {
    const spy = vi.fn()
    register({ id: 'x', title: 'x', run: spy })
    fireAndForget('x', { a: 1 })
    expect(spy).toHaveBeenCalledWith({ a: 1 })
  })

  it('동기 throw 를 삼킨다(전파 안 함) + warn', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    register({ id: 'boom', title: 'boom', run: () => { throw new Error('nope') } })
    expect(() => fireAndForget('boom')).not.toThrow()
    expect(warn).toHaveBeenCalled()
  })

  it('모르는 id(run throw)도 삼킨다(리스너가 죽지 않게)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    expect(() => fireAndForget('missing.id')).not.toThrow()
    expect(warn).toHaveBeenCalled()
  })

  it('reject 되는 Promise 반환: unhandled rejection 없이 삼킨다 + warn', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    register({ id: 'async.reject', title: 'r', run: () => Promise.reject(new Error('async-nope')) })
    fireAndForget('async.reject')
    // Wait for the microtask flush so the .catch runs.
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
  })

  it('thenable(비 네이티브 Promise) reject 도 삼킨다(FIX-3: instanceof 로 안 좁힘)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const thenable = { then: (_res: unknown, rej: (e: unknown) => void) => rej(new Error('thenable-nope')) }
    register({ id: 'thenable.reject', title: 't', run: () => thenable })
    fireAndForget('thenable.reject')
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
  })
})
