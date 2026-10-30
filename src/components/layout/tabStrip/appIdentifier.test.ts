// 탭 끌기 열쇠(`getIdentifier`) 묻기 — 유계 재시도 · 소진 뒤 다음 부름에 다시 · 빈 답 · 묻는 중 거듭 부름(ADR-0102).

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

  it('답을 소문자로 쥔다 · 답 전은 null · 묻는 중 · 답 뒤의 거듭 부름은 다시 묻지 않는다', async () => {
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

  it('실패하면 같은 부름 안에서 다시 묻는다 — 다음 마운트를 기다리지 않는다', async () => {
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

  it('재시도가 다 실패하면 null 그대로 · 다음 부름은 다시 묻는다', async () => {
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

  it('빈 답 → null 그대로 · 다시 묻지 않는다', async () => {
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
