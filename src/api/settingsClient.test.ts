// settingsClient — subscribe first · apply by `rev` per key · writes pass through (ADR-0265 · TRD S21-storage §8 「frontend settings」).

import { describe, expect, it, vi } from 'vitest'

import { createSettingsClient, type SettingChange, type SettingsIpc } from './settingsClient'

const EVT = 'settings:changed'

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void } {
  let resolve: (v: T) => void = () => {}
  const promise = new Promise<T>(r => {
    resolve = r
  })
  return { promise, resolve }
}

/** Fake IPC — the test holds and releases the `listen` registration and the `settings_get` answer. */
function fakeIpc() {
  const calls: string[] = []
  const listenGate = deferred<void>()
  const getAnswer = deferred<unknown>()
  let handler: ((payload: unknown) => void) | undefined
  const unlisten = vi.fn()
  const invokeArgs: Array<[string, Record<string, unknown> | undefined]> = []
  const writeAnswers = new Map<string, unknown>()

  const ipc: SettingsIpc = {
    async listen(event, h) {
      calls.push(`listen:${event}`)
      await listenGate.promise
      handler = h
      return unlisten
    },
    async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
      calls.push(`invoke:${cmd}`)
      invokeArgs.push([cmd, args])
      if (cmd === 'settings_get') return (await getAnswer.promise) as T
      if (writeAnswers.has(cmd)) return writeAnswers.get(cmd) as T
      throw new Error(`unexpected command ${cmd}`)
    },
  }

  return {
    ipc,
    calls,
    invokeArgs,
    unlisten,
    writeAnswers,
    openListen: () => listenGate.resolve(),
    answerGet: (v: unknown) => getAnswer.resolve(v),
    push: (payload: unknown) => {
      if (!handler) throw new Error('no listener yet')
      handler(payload)
    },
    hasHandler: () => handler !== undefined,
  }
}

/** Flush the microtask chain (registration → pull → apply). */
async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

function item(key: string, value: string) {
  return { key, value, is_default: false }
}

/** Accumulate the changes the subscriber received as key → value. */
function recorder() {
  const seen: SettingChange[][] = []
  const listener = (changes: readonly SettingChange[]) => {
    seen.push([...changes])
  }
  return { seen, listener }
}

describe('settingsClient — subscribe first, pull later', () => {
  it('does not issue settings_get before listen registration finishes', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    await flush()
    expect(f.calls).toEqual([`listen:${EVT}`])

    f.openListen()
    await flush()
    expect(f.calls).toEqual([`listen:${EVT}`, 'invoke:settings_get'])
  })
})

describe('settingsClient — rev per key', () => {
  it('applies a key seen for the first time even with a rev 0 answer', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    const r = recorder()
    client.subscribe(r.listener)
    client.install()
    f.openListen()
    await flush()
    f.answerGet({ rev: 0, items: [item('chat.style.fontSize', '13px')] })
    await flush()

    expect(client.get('chat.style.fontSize')).toBe('13px')
    expect(r.seen).toEqual([[{ key: 'chat.style.fontSize', value: '13px' }]])
  })

  it('drops notices at or below the rev applied to that key, and applies only a larger rev', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()

    f.push({ rev: 5, items: [item('a', 'x')] })
    f.push({ rev: 5, items: [item('a', 'same-rev')] })
    f.push({ rev: 4, items: [item('a', 'older')] })
    expect(client.get('a')).toBe('x')

    f.push({ rev: 6, items: [item('a', 'newer')] })
    expect(client.get('a')).toBe('newer')
  })

  it("rev is separate per key — a large rev of another key does not block this key's small rev", async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()

    f.push({ rev: 9, items: [item('a', 'a9')] })
    f.push({ rev: 3, items: [item('b', 'b3')] })
    expect(client.get('b')).toBe('b3')
  })

  it('a pull answer that came after a new notice does not overwrite that key, but applies the other keys of the answer', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()

    f.push({ rev: 3, items: [item('a', 'new')] })
    f.answerGet({ rev: 2, items: [item('a', 'old'), item('b', 'b2')] })
    await flush()

    expect(client.get('a')).toBe('new')
    expect(client.get('b')).toBe('b2')
  })

  it('drops a payload whose shape differs from the contract', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()

    f.push({ rev: '7', items: [item('a', 'x')] })
    f.push(null)
    f.push({ rev: 1, items: [{ key: 'a', value: 7 }, item('b', 'ok')] })
    expect(client.get('a')).toBeUndefined()
    expect(client.get('b')).toBe('ok')
    warn.mockRestore()
  })
})

describe('settingsClient — subscribers', () => {
  it('gets the values so far right on subscribing, and after that only the keys whose values changed', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()
    f.push({ rev: 1, items: [item('a', 'x')] })

    const r = recorder()
    const off = client.subscribe(r.listener)
    expect(r.seen).toEqual([[{ key: 'a', value: 'x' }]])

    f.push({ rev: 2, items: [item('a', 'x'), item('b', 'y')] })
    expect(r.seen[1]).toEqual([{ key: 'b', value: 'y' }])

    off()
    f.push({ rev: 3, items: [item('a', 'z')] })
    expect(r.seen).toHaveLength(2)
  })
})

describe('settingsClient — writes pass through', () => {
  it('set goes as settings_set {key,value}, and a changed answer is applied by the same rule', async () => {
    const f = fakeIpc()
    f.writeAnswers.set('settings_set', { rev: 4, key: 'a', value: '15px', changed: true })
    const client = createSettingsClient(f.ipc)

    const out = await client.set('a', ' 15PX ')
    expect(out).toEqual({ rev: 4, key: 'a', value: '15px', changed: true })
    expect(f.invokeArgs).toEqual([['settings_set', { key: 'a', value: ' 15PX ' }]])
    expect(client.get('a')).toBe('15px')
  })

  it('even a changed:false answer corrects a stale value caused by a missed notice when its rev is larger', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()
    f.push({ rev: 2, items: [item('a', 'old')] })

    // This window missed the notices for rev 3–5 — the shell's current value is already 'new', so the write answers with no change.
    f.writeAnswers.set('settings_set', { rev: 5, key: 'a', value: 'new', changed: false })
    await client.set('a', 'new')
    expect(client.get('a')).toBe('new')
  })

  it('even a changed:false answer does not overwrite at or below the rev applied to that key', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()
    f.push({ rev: 7, items: [item('a', 'newer')] })

    f.writeAnswers.set('settings_set', { rev: 6, key: 'a', value: 'older', changed: false })
    await client.set('a', 'older')
    expect(client.get('a')).toBe('newer')
  })

  it('reset goes as settings_reset {key}, and applies the changed entries', async () => {
    const f = fakeIpc()
    f.writeAnswers.set('settings_reset', {
      rev: 8,
      reset: ['chat.style.a', 'chat.style.b'],
      changed: [{ key: 'chat.style.a', value: '1rem', is_default: true }],
    })
    const client = createSettingsClient(f.ipc)

    await client.reset('chat.style.')
    expect(f.invokeArgs).toEqual([['settings_reset', { key: 'chat.style.' }]])
    expect(client.get('chat.style.a')).toBe('1rem')
    expect(client.get('chat.style.b')).toBeUndefined()
  })

  it('a shell refusal (writes not open yet etc.) is rejected to the caller as is', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    f.ipc.invoke = async () => {
      throw 'settings writes are not open yet (before shell setup)'
    }
    await expect(client.set('a', '1px')).rejects.toBe('settings writes are not open yet (before shell setup)')
  })
})

describe('settingsClient — lifetime', () => {
  it('dispose calls unlisten, and after that notices and pull answers do not paint', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    const dispose = client.install()
    f.openListen()
    await flush()

    dispose()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
    f.push({ rev: 1, items: [item('a', 'x')] })
    f.answerGet({ rev: 1, items: [item('b', 'y')] })
    await flush()
    expect(client.get('a')).toBeUndefined()
    expect(client.get('b')).toBeUndefined()
  })

  it('when disposed before registration finishes, it releases the registration as soon as it arrives and does not pull', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    const dispose = client.install()
    await flush()
    dispose()

    f.openListen()
    await flush()
    expect(f.unlisten).toHaveBeenCalledTimes(1)
    expect(f.calls).not.toContain('invoke:settings_get')
  })
})
