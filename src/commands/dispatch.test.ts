// ADR-0055: fireAndForget 단위테스트(FIX-3/FIX-4) — 사람 클릭·키바인딩용 안전 실행 경로.
//   핵심: sync throw·async reject(thenable 포함)를 모두 삼켜 warn 만 남긴다(리스너 안 죽음).
//   ★run() 자체는 손대지 않는다(await 호출부 무손실)★ — 그 계약은 registry.test.ts 가 지킨다.

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
    // microtask flush 를 기다려 .catch 가 돌게 한다.
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
  })

  it('thenable(비 네이티브 Promise) reject 도 삼킨다(FIX-3: instanceof 로 안 좁힘)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // Promise.resolve().catch 로 정규화되므로 thenable 도 커버된다.
    const thenable = { then: (_res: unknown, rej: (e: unknown) => void) => rej(new Error('thenable-nope')) }
    register({ id: 'thenable.reject', title: 't', run: () => thenable })
    fireAndForget('thenable.reject')
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
  })
})
