// restoreClient — subscribe first · the later-issued pull wins · pull again after the answer (TRD S21-storage §6-7 · §8 「frontend modal」).

import { afterEach, describe, expect, it, vi } from 'vitest'

import { BOOT_REPULL_INTERVAL_MS, createRestoreClient, EVT_RESTORE_CHANGED } from './restoreClient'
import { fakeRestoreIpc, flush, view } from './testing/fakeRestoreIpc'

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('restoreClient — subscribe first, pull later', () => {
  it('does not issue restore_status before listen registration finishes', async () => {
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

  it('when disposed before registration finishes, releases the received handle right away and does not pull (StrictMode first pass)', async () => {
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

describe('restoreClient — answers of a torn-down install', () => {
  it('a pull that resolves after dispose does not paint', async () => {
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

  it("remount — even when the previous install's answer comes before the new install's, it does not paint; only the new install's answer remains", async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    f.setHoldStatus(true)
    const client = createRestoreClient(f.ipc)
    const first = client.install()
    await flush()
    first()
    client.install()
    await flush()
    expect(f.heldStatus).toHaveLength(2)

    // A request issued by the earlier install resolves late — meanwhile the shell has already received the answer.
    f.heldStatus[0](view('awaiting'))
    await flush()
    expect(client.status()).toBeNull()

    f.heldStatus[1](view('answered'))
    await flush()
    expect(client.status()?.crash_copy).toBe('answered')
  })
})

describe('restoreClient — notices', () => {
  it('pulls again on every restore:changed and notifies subscribers', async () => {
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

  it('even when answers come in a different order than issued, the answer of the later-issued pull remains', async () => {
    const f = fakeRestoreIpc(view('awaiting'))
    const client = createRestoreClient(f.ipc)
    f.setHoldStatus(true)
    client.install()
    await flush()
    // While the first pull was in flight the answer finished and a notification arrived → a second pull.
    f.push('answered')
    await flush()
    expect(f.heldStatus).toHaveLength(2)

    f.heldStatus[1](view('answered'))
    await flush()
    f.heldStatus[0](view('awaiting'))
    await flush()
    expect(client.status()?.crash_copy).toBe('answered')
  })

  it('a notice delivered late after dispose does not pull', async () => {
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

  it('durable is carried as is when a boolean, and as null when missing or of a different shape', async () => {
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

  it('carries saves, and it stays the same after the answer', async () => {
    const f = fakeRestoreIpc(view('awaiting', { saves: false }))
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()?.saves).toBe(false)

    await client.answer(false)
    expect(client.status()?.crash_copy).toBe('answered')
    expect(client.status()?.saves).toBe(false)
  })

  // When the shell and frontend editions are out of step — if the payload is dropped, even the asking modal does not show.
  it.each([
    ['when missing', undefined],
    ['when null', null],
    ['when not a boolean', 'false'],
  ])('saves %s does not drop the first payload — only saves is left unknown (undefined) and the warning comes once', async (_label, saves) => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { saves: _omit, ...withoutSaves } = view('awaiting')
    const f = fakeRestoreIpc((saves === undefined ? withoutSaves : { ...withoutSaves, saves }) as never)
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()?.crash_copy).toBe('awaiting')
    expect(client.status()?.tabs).toBe(5)
    expect(client.status()?.saves).toBeUndefined()
    expect(warn).toHaveBeenCalledTimes(1)

    f.push('awaiting')
    await flush()
    expect(f.statusCalls()).toBe(2)
    expect(warn).toHaveBeenCalledTimes(1)
  })

  it('an unknown state_file value does not drop the first payload — only state_file is undefined, the warning comes once', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const f = fakeRestoreIpc({ ...view('awaiting'), state_file: 'from_a_newer_shell' as never })
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()?.crash_copy).toBe('awaiting')
    expect(client.status()?.state_file).toBeUndefined()
    expect(client.status()?.saves).toBe(true)
    expect(warn).toHaveBeenCalledTimes(1)

    f.push('awaiting')
    await flush()
    expect(f.statusCalls()).toBe(2)
    expect(warn).toHaveBeenCalledTimes(1)
  })

  // ADR-0291: the shell's `newer` is a known value — lowering it to unknown would lose that notice.
  it('carries newer of state_file as is and does not warn', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const f = fakeRestoreIpc({ ...view('none'), saves: false, state_file: 'newer' })
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()?.state_file).toBe('newer')
    expect(client.status()?.saves).toBe(false)
    expect(warn).not.toHaveBeenCalled()
  })

  it('when both notice fields are off, one warning carries both', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { saves: _omit, ...withoutSaves } = view('none')
    const f = fakeRestoreIpc({ ...withoutSaves, state_file: 7 } as never)
    const client = createRestoreClient(f.ipc)
    client.install()
    await flush()
    expect(client.status()?.crash_copy).toBe('none')
    expect(warn).toHaveBeenCalledTimes(1)
    expect(String(warn.mock.calls[0][0])).toContain('saves · state_file')
  })
})

describe('restoreClient — answer', () => {
  it('carries accept to restore_answer, and resolves after pulling again after the answer', async () => {
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

  it("pulls again even on failure, and rejects with the shell's error as is", async () => {
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

// Enough time for the bounded retry (4 attempts spaced 150 + 300 + 600 ms) to run out.
const RETRIES_DONE_MS = 1100

describe('restoreClient — the boot pull does not let go until it receives', () => {
  function quietConsole() {
    return {
      error: vi.spyOn(console, 'error').mockImplementation(() => {}),
      warn: vi.spyOn(console, 'warn').mockImplementation(() => {}),
    }
  }

  it('when all retries fail, it pulls again every interval and stops once it receives — the log comes once', async () => {
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
    // After the 3rd attempt of the bounded retry, not every attempt leaves a log.
    expect(log.error).toHaveBeenCalledTimes(1)
    expect(log.warn).toHaveBeenCalledTimes(3)

    f.setFailStatus(false)
    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS)
    expect(f.statusCalls()).toBe(8)
    expect(client.status()?.crash_copy).toBe('awaiting')

    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 5)
    expect(f.statusCalls()).toBe(8)
  })

  it('dispose stops pulling again', async () => {
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

  it('a StrictMode-style install → dispose → install runs only one loop', async () => {
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

  it("installing again mid-loop (remount) stops the previous install's loop and only the new install's runs", async () => {
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

  it('the boot pull goes out even when the subscription fails through all its retries', async () => {
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

  it('a payload that could not read crash_copy is a failed pull — it pulls again every interval and stops on receiving a valid payload', async () => {
    vi.useFakeTimers()
    const log = quietConsole()
    const f = fakeRestoreIpc({ ...view('awaiting'), crash_copy: 'Awaiting' as never })
    const client = createRestoreClient(f.ipc)
    client.install()

    await vi.advanceTimersByTimeAsync(0)
    expect(f.statusCalls()).toBe(1)
    expect(client.status()).toBeNull()
    expect(log.error).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 2)
    expect(f.statusCalls()).toBe(3)
    expect(client.status()).toBeNull()
    expect(log.error).toHaveBeenCalledTimes(1)

    f.server.view = view('awaiting')
    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS)
    expect(f.statusCalls()).toBe(4)
    expect(client.status()?.crash_copy).toBe('awaiting')

    await vi.advanceTimersByTimeAsync(BOOT_REPULL_INTERVAL_MS * 5)
    expect(f.statusCalls()).toBe(4)
  })

  it('when a pull a notice called receives first, the loop stops there', async () => {
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
