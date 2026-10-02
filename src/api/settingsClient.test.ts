// settingsClient — 구독 먼저 · 키마다 `rev` 로 적용 · 쓰기 통과(ADR-0265 · TRD S21-storage §8 「프론트 설정」).

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

/** 가짜 IPC — `listen` 등록과 `settings_get` 답을 시험이 붙잡았다 놓는다. */
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
      throw new Error(`예상 밖 명령 ${cmd}`)
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
      if (!handler) throw new Error('리스너가 아직 없다')
      handler(payload)
    },
    hasHandler: () => handler !== undefined,
  }
}

/** 마이크로태스크 사슬(등록 → 당기기 → 적용)을 흘려보낸다. */
async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

function item(key: string, value: string) {
  return { key, value, is_default: false }
}

/** 구독자가 받은 변경을 키 → 값으로 누적한다. */
function recorder() {
  const seen: SettingChange[][] = []
  const listener = (changes: readonly SettingChange[]) => {
    seen.push([...changes])
  }
  return { seen, listener }
}

describe('settingsClient — 구독 먼저, 당기기 나중', () => {
  it('listen 등록이 끝나기 전에는 settings_get 을 내지 않는다', async () => {
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

describe('settingsClient — 키마다 rev', () => {
  it('처음 보는 키는 rev 0 답이어도 적용한다', async () => {
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

  it('그 키에 적용한 rev 이하의 알림은 버리고, 더 큰 rev 만 적용한다', async () => {
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

  it('rev 는 키마다 따로다 — 다른 키의 큰 rev 가 이 키의 작은 rev 를 막지 않는다', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()

    f.push({ rev: 9, items: [item('a', 'a9')] })
    f.push({ rev: 3, items: [item('b', 'b3')] })
    expect(client.get('b')).toBe('b3')
  })

  it('새 알림 뒤에 온 당기기 답은 그 키를 덮지 않지만, 답의 다른 키는 적용한다', async () => {
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

  it('모양이 계약과 다른 짐은 버린다', async () => {
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

describe('settingsClient — 구독자', () => {
  it('구독하자마자 지금까지의 값을 받고, 그 뒤로는 값이 달라진 키만 받는다', async () => {
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

describe('settingsClient — 쓰기 통과', () => {
  it('set 은 settings_set {key,value} 로 가고, 바뀐 답은 같은 규칙으로 적용한다', async () => {
    const f = fakeIpc()
    f.writeAnswers.set('settings_set', { rev: 4, key: 'a', value: '15px', changed: true })
    const client = createSettingsClient(f.ipc)

    const out = await client.set('a', ' 15PX ')
    expect(out).toEqual({ rev: 4, key: 'a', value: '15px', changed: true })
    expect(f.invokeArgs).toEqual([['settings_set', { key: 'a', value: ' 15PX ' }]])
    expect(client.get('a')).toBe('15px')
  })

  it('changed:false 답도 더 큰 rev 면 놓친 알림 탓에 낡은 값을 바로잡는다', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    client.install()
    f.openListen()
    await flush()
    f.push({ rev: 2, items: [item('a', 'old')] })

    // rev 3–5 의 알림을 이 창이 놓쳤다 — 셸의 현재 값은 이미 'new' 라 쓰기는 무변경으로 답한다.
    f.writeAnswers.set('settings_set', { rev: 5, key: 'a', value: 'new', changed: false })
    await client.set('a', 'new')
    expect(client.get('a')).toBe('new')
  })

  it('changed:false 답이라도 그 키에 적용한 rev 이하면 덮지 않는다', async () => {
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

  it('reset 은 settings_reset {key} 로 가고, changed 항목을 적용한다', async () => {
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

  it('셸의 거절(쓰기 미개방 등)은 부른 쪽으로 그대로 reject 한다', async () => {
    const f = fakeIpc()
    const client = createSettingsClient(f.ipc)
    f.ipc.invoke = async () => {
      throw '설정 쓰기가 아직 열리지 않았다'
    }
    await expect(client.set('a', '1px')).rejects.toBe('설정 쓰기가 아직 열리지 않았다')
  })
})

describe('settingsClient — 수명', () => {
  it('dispose 는 unlisten 을 부르고, 그 뒤 알림 · 당기기 답을 칠하지 않는다', async () => {
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

  it('등록이 끝나기 전에 dispose 되면 등록을 받자마자 풀고 당기지 않는다', async () => {
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
