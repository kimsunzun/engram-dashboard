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

describe('fireAndForget (ADR-0055 fire-and-forget safety net)', () => {
  it('runs a synchronous handler and passes the argument (single bag)', () => {
    const spy = vi.fn()
    register({ id: 'x', title: () => 'x', run: spy })
    fireAndForget('x', { a: 1 })
    expect(spy).toHaveBeenCalledWith({ a: 1 })
  })

  it('swallows a synchronous throw (no propagation) + warn', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    register({ id: 'boom', title: () => 'boom', run: () => { throw new Error('nope') } })
    expect(() => fireAndForget('boom')).not.toThrow()
    expect(warn).toHaveBeenCalled()
  })

  it('swallows an unknown id (run throws) too (so the listener does not die)', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    expect(() => fireAndForget('missing.id')).not.toThrow()
    expect(warn).toHaveBeenCalled()
  })

  it('returns a Promise that rejects: swallowed without an unhandled rejection + warn', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    register({ id: 'async.reject', title: () => 'r', run: () => Promise.reject(new Error('async-nope')) })
    fireAndForget('async.reject')
    // Wait for the microtask flush so the .catch runs.
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
  })

  it('swallows a thenable (non-native Promise) reject too (FIX-3: not narrowed by instanceof)', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const thenable = { then: (_res: unknown, rej: (e: unknown) => void) => rej(new Error('thenable-nope')) }
    register({ id: 'thenable.reject', title: () => 't', run: () => thenable })
    fireAndForget('thenable.reject')
    await Promise.resolve()
    await Promise.resolve()
    expect(warn).toHaveBeenCalled()
  })
})
