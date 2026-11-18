// Asking for the tab drag key (`getIdentifier`) — bounded retry · asks again on the next call after exhaustion · empty answer · repeated calls while asking (ADR-0102).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { createAppIdentifier } from './appIdentifier'

describe('createAppIdentifier', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('holds the answer in lowercase · null before the answer · while asking · repeated calls after the answer do not ask again', async () => {
    const get = vi.fn(async () => 'com.Engram.App')
    const app = createAppIdentifier(get)
    expect(app.current()).toBeNull()
    app.ask()
    app.ask()
    await vi.advanceTimersByTimeAsync(0)
    expect(app.current()).toBe('com.engram.app')
    app.ask()
    expect(get).toHaveBeenCalledTimes(1)
  })

  it('on failure it asks again within the same call — it does not wait for the next mount', async () => {
    const get = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error('not ready'))
      .mockRejectedValueOnce(new Error('not ready'))
      .mockResolvedValue('com.engram.app')
    const app = createAppIdentifier(get)
    app.ask()
    await vi.advanceTimersByTimeAsync(1000)
    expect(get).toHaveBeenCalledTimes(3)
    expect(app.current()).toBe('com.engram.app')
  })

  it('if retries all fail it stays null · the next call asks again', async () => {
    const get = vi.fn<() => Promise<string>>().mockRejectedValue(new Error('down'))
    const app = createAppIdentifier(get)
    app.ask()
    await vi.advanceTimersByTimeAsync(5000)
    const tries = get.mock.calls.length
    expect(tries).toBeGreaterThan(1)
    expect(app.current()).toBeNull()
    expect(console.error).toHaveBeenCalledTimes(1)

    get.mockResolvedValue('com.engram.app')
    app.ask()
    await vi.advanceTimersByTimeAsync(0)
    expect(get).toHaveBeenCalledTimes(tries + 1)
    expect(app.current()).toBe('com.engram.app')
  })

  it('an empty answer → null as is · it does not ask again', async () => {
    const get = vi.fn(async () => '')
    const app = createAppIdentifier(get)
    app.ask()
    await vi.advanceTimersByTimeAsync(5000)
    app.ask()
    await vi.advanceTimersByTimeAsync(5000)
    expect(get).toHaveBeenCalledTimes(1)
    expect(app.current()).toBeNull()
  })
})
