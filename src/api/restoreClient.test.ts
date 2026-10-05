// restoreClient — 구독 먼저 · 나중에 낸 당기기가 이김 · 답 뒤 다시 당김(TRD S21-storage §6-7 · §8 「프론트 모달」).

import { afterEach, describe, expect, it, vi } from 'vitest'

import { BOOT_REPULL_INTERVAL_MS, createRestoreClient, EVT_RESTORE_CHANGED } from './restoreClient'
import { fakeRestoreIpc, flush, view } from './testing/fakeRestoreIpc'

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('restoreClient — 구독 먼저, 당기기 나중', () => {
  it('listen 등록이 끝나기 전에는 restore_status 를 내지 않는다', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    const open = f.holdListen()
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(f.calls).toEqual([`listen:${EVT_RESTORE_CHANGED}`])
    expect(client.status()).toBeNull()

    open()
    await flush()
    expect(f.calls).toEqual([`listen:${EVT_RESTORE_CHANGED}`, 'invoke:restore_status'])
    expect(client.status()?.crash_copy).toBe('awaiting')
  })

  it('등록이 끝나기 전에 dispose 되면 받은 핸들을 바로 풀고 당기지 않는다(StrictMode 첫 회)', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    const open = f.holdListen()
    const client = createRestoreClient(f.ipc)
    const dispose = client.install()
    dispose()
    open()
    await flush()
    expect(f.unlistenCalls).toBe(1)
    expect(f.hasListener()).toBe(false)
    expect(f.calls).not.toContain('invoke:restore_status')
  })
})

describe('restoreClient — 내린 설치의 답', () => {
  it('dispose 뒤에 풀린 당기기는 칠하지 않는다', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    f.setHoldStatus(true)
    const client = createRestoreClient(f.ipc)
    const dispose = client.install()
    await flush()
    expect(f.heldStatus).toHaveLength(1)

    dispose()
    f.heldStatus[0](view('awaiting'))
    await flush()
    expect(client.status()).toBeNull()
  })

  it('재마운트 — 앞 설치의 답이 새 설치의 답보다 먼저 와도 칠하지 않고, 새 설치의 답만 남는다', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    f.setHoldStatus(true)
    const client = createRestoreClient(f.ipc)
    const first = client.install()
    await flush()
    first()
    client.install()
    await flush()
    expect(f.heldStatus).toHaveLength(2)

    // 앞 설치가 낸 요청이 늦게 풀린다 — 그사이 셸은 이미 답을 받았다.
    f.heldStatus[0](view('awaiting'))
    await flush()
    expect(client.status()).toBeNull()

    f.heldStatus[1](view('answered'))
    await flush()
    expect(client.status()?.crash_copy).toBe('answered')
  })
})

describe('restoreClient — 알림', () => {
  it('restore:changed 마다 다시 당기고 구독자에게 알린다', async () => {
    const f = fakeRestoreIpc(view('none'))
    const client = createRestoreClient(f.ipc)
    const seen = vi.fn()
    client.subscribe(seen)
    client.install()
    await flush()
    expect(client.status()?.crash_copy).toBe('none')

    f.server.view = view('awaiting')
    f.push('awaiting')
    await flush()
    expect(client.status()?.crash_copy).toBe('awaiting')
    expect(client.status()?.tabs).toBe(5)
    expect(seen).toHaveBeenCalledTimes(2)
  })

  it('답이 낸 순서와 다르게 와도 나중에 낸 당기기의 답이 남는다', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    const client = createRestoreClient(f.ipc)
    f.setHoldStatus(true)
    client.install()
    await flush()
    // 첫 당기기가 떠 있는 동안 답이 끝나 알림이 왔다 → 둘째 당기기.
    f.push('answered')
    await flush()
    expect(f.heldStatus).toHaveLength(2)

    f.heldStatus[1](view('answered'))
    await flush()
    f.heldStatus[0](view('awaiting'))
    await flush()
    expect(client.status()?.crash_copy).toBe('answered')
  })

  it('dispose 뒤에 늦게 배달된 알림은 당기지 않는다', async () => {
    const f = fakeRestoreIpc(view('none'))
    const client = createRestoreClient(f.ipc)
    const dispose = client.install()
    await flush()
    const late = f.currentHandler()
    const before = f.calls.length
    dispose()
    expect(f.unlistenCalls).toBe(1)
    late?.('answered')
    await flush()
    expect(f.calls.length).toBe(before)
  })

  it('durable 은 불리언이면 그대로, 없거나 모양이 다르면 null 로 싣는다', async () => {
    const f = fakeRestoreIpc(view('awaiting', { durable: false }))
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()?.durable).toBe(false)

    const { durable: _omit, ...withoutDurable } = view('awaiting')
    f.server.view = withoutDurable as never
    f.push('awaiting')
    await flush()
    expect(client.status()?.durable).toBeNull()
  })

  it('모양이 계약과 다른 답은 버린다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const f = fakeRestoreIpc({ ...view('awaiting'), crash_copy: 'Awaiting' as never })
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })
})

describe('restoreClient — 답', () => {
  it('restore_answer 에 accept 를 싣고, 답 뒤 다시 당긴 다음에 풀린다', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()

    const reply = await client.answer(true)
    expect(reply).toEqual({ restored_windows: 1, durable: true })
    expect(f.answerArgs).toEqual([true])
    expect(f.calls.slice(-2)).toEqual(['invoke:restore_answer', 'invoke:restore_status'])
    expect(client.status()?.crash_copy).toBe('answered')
  })

  it('실패해도 다시 당기고, 셸의 오류를 그대로 reject 한다', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    f.setOnAnswer(async () => {
      throw 'CONFLICT'
    })
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()

    await expect(client.answer(false)).rejects.toBe('CONFLICT')
    expect(f.calls.slice(-2)).toEqual(['invoke:restore_answer', 'invoke:restore_status'])
    expect(client.status()?.crash_copy).toBe('awaiting')
  })
})

// 유계 재시도(150 + 300 + 600 ms 사이 4회)가 다 끝나는 데 충분한 시간.
const RETRIES_DONE_MS = 1100

describe('restoreClient — 부팅 당기기는 받을 때까지 놓지 않는다', () => {
  function quietConsole() {
    return {
      error: vi.spyOn(console, 'error').mockImplementation(() => {}),
      warn: vi.spyOn(console, 'warn').mockImplementation(() => {}),
    }
  }

  it('재시도가 다 실패하면 간격마다 다시 당기고, 받으면 멈춘다 — 로그는 한 번', async () => {
    vi.useFakeTimers()
    const log = quietConsole()
    const f = fakeRestoreIpc(view('awaiting'))
    f.setFailStatus(true)
    const client = createRestoreClient(f.ipc)
    client.install()

    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS)
    expect(f.statusCalls()).toBe(4)
    expect(log.error).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 3)
    expect(f.statusCalls()).toBe(7)
    expect(client.status()).toBeNull()
    // 유계 재시도의 3회 뒤로는 시도마다 남기지 않는다.
    expect(log.error).toHaveBeenCalledTimes(1)
    expect(log.warn).toHaveBeenCalledTimes(3)

    f.setFailStatus(false)
    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS)
    expect(f.statusCalls()).toBe(8)
    expect(client.status()?.crash_copy).toBe('awaiting')

    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 5)
    expect(f.statusCalls()).toBe(8)
  })

  it('dispose 하면 다시 당기기를 멈춘다', async () => {
    vi.useFakeTimers()
    quietConsole()
    const f = fakeRestoreIpc(view('awaiting'))
    f.setFailStatus(true)
    const dispose = createRestoreClient(f.ipc).install()

    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS + BOOT_REPULL_INTERVAL_MS)
    expect(f.statusCalls()).toBe(5)
    dispose()
    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 5)
    expect(f.statusCalls()).toBe(5)
  })

  it('StrictMode 식 install → dispose → install 은 루프를 하나만 돌린다', async () => {
    vi.useFakeTimers()
    quietConsole()
    const f = fakeRestoreIpc(view('awaiting'))
    f.setFailStatus(true)
    const client = createRestoreClient(f.ipc)
    client.install()()
    client.install()

    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS)
    expect(f.statusCalls()).toBe(4)
    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 3)
    expect(f.statusCalls()).toBe(7)
  })

  it('루프 도중 다시 설치해도(재마운트) 앞 설치의 루프는 멈추고 새 설치의 것만 돈다', async () => {
    vi.useFakeTimers()
    quietConsole()
    const f = fakeRestoreIpc(view('awaiting'))
    f.setFailStatus(true)
    const client = createRestoreClient(f.ipc)
    const first = client.install()
    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS)
    expect(f.statusCalls()).toBe(4)

    first()
    client.install()
    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS)
    expect(f.statusCalls()).toBe(8)
    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 2)
    expect(f.statusCalls()).toBe(10)
  })

  it('구독이 재시도까지 다 실패해도 부팅 당기기는 나간다', async () => {
    vi.useFakeTimers()
    const log = quietConsole()
    const f = fakeRestoreIpc(view('awaiting'))
    f.setFailListen(true)
    const client = createRestoreClient(f.ipc)
    client.install()

    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS)
    expect(f.calls.filter(c => c === `listen:${EVT_RESTORE_CHANGED}`)).toHaveLength(4)
    expect(f.hasListener()).toBe(false)
    expect(f.statusCalls()).toBe(1)
    expect(client.status()?.crash_copy).toBe('awaiting')
    expect(log.error).toHaveBeenCalledTimes(1)
  })

  it('알림이 부른 당기기가 먼저 받으면 루프는 거기서 멈춘다', async () => {
    vi.useFakeTimers()
    quietConsole()
    const f = fakeRestoreIpc(view('none'))
    f.setFailStatus(true)
    const client = createRestoreClient(f.ipc)
    client.install()
    await vi.advanceTimersByTimeAsync(RETRIES_DONE_MS)
    expect(f.statusCalls()).toBe(4)

    f.setFailStatus(false)
    f.server.view = view('awaiting')
    f.push('awaiting')
    await vi.advanceTimersByTimeAsync(0)
    expect(client.status()?.crash_copy).toBe('awaiting')
    expect(f.statusCalls()).toBe(5)

    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 5)
    expect(f.statusCalls()).toBe(5)
  })
})
