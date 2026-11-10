// connection.status · connection.reconnect · daemon.stop — the band's LLM path (TRD S36 §3-6).
//
// The registration is checked through the manifest (`./contributions`, the reason = the head of
// renderModeCommands.test.ts) in a main and in a popout window; the replies with fake dependencies.

import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Band } from '../../src-tauri/bindings/Band'
import type { DaemonShutdownReply } from '../../src-tauri/bindings/DaemonShutdownReply'
import type { AgentClient, ConnectionState } from '../api/agentClient'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main', close: vi.fn(async () => undefined) }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))
vi.mock('../api/clientFactory', () => ({
  agentClient: new Proxy({}, { get: () => vi.fn(async () => undefined) }),
  getAgentClient: vi.fn(),
  getDaemonControl: vi.fn(),
  bootstrapDaemonIfNeeded: vi.fn(async () => undefined),
}))

const IDS = ['connection.status', 'connection.reconnect', 'daemon.stop']
const PLAIN: Band = { kind: 'not_connected', refused: false }
const VERSION: Band = { kind: 'version', can_stop: false }

const origHash = window.location.hash

afterEach(() => {
  window.location.hash = origHash
})

describe('connection commands — main window only', () => {
  async function registeredIn(hash: string) {
    vi.resetModules()
    window.location.hash = hash
    await import('./contributions')
    const { list } = await import('./registry')
    return list()
  }

  it('come in through the manifest in the main window, off the bus (no `help`)', async () => {
    const cmds = (await registeredIn('#/')).filter(c => IDS.includes(c.id))
    expect(cmds.map(c => c.id).sort()).toEqual([...IDS].sort())
    for (const c of cmds) expect(c.help, c.id).toBeUndefined()
  })

  it('are not registered in a popout or the spare window', async () => {
    const ids = (await registeredIn('#/popup?window=slot-popup-1')).map(c => c.id)
    for (const id of IDS) expect(ids).not.toContain(id)
  })
})

describe('connection commands — replies', () => {
  let registry: typeof import('./registry')

  type Status = { state: ConnectionState; band: Band | null; error?: string | null }

  function fakeClient(initial: Status) {
    return {
      connectionState: initial.state,
      connectionBand: initial.band,
      connectionError: initial.error ?? null,
      disconnect: vi.fn(),
    }
  }

  async function install(
    client: ReturnType<typeof fakeClient>,
    deps: { start?: () => Promise<unknown>; shutdown?: () => Promise<DaemonShutdownReply> } = {},
  ) {
    vi.resetModules()
    window.location.hash = '#/popup?window=slot-popup-1' // keep the module's own registration out
    registry = await import('./registry')
    const { registerConnectionCommands } = await import('./connectionCommands')
    const start = vi.fn(deps.start ?? (async () => undefined))
    const closed = async (): Promise<DaemonShutdownReply> => ({ outcome: 'closed', reason: null })
    const shutdown = vi.fn(deps.shutdown ?? closed)
    registerConnectionCommands({ client: client as unknown as AgentClient, start, shutdown })
    return { start, shutdown }
  }

  it.each<[string, Status, { state: ConnectionState; band: Band | null; text: string | null }]>([
    ['connected', { state: 'connected', band: null }, { state: 'connected', band: null, text: null }],
    [
      'band ① with a start cause',
      { state: 'down', band: PLAIN, error: '폴더 문제' },
      { state: 'down', band: PLAIN, text: '데몬에 연결하지 못했습니다. 폴더 문제' },
    ],
    [
      'band ① refused',
      { state: 'down', band: { kind: 'not_connected', refused: true } },
      {
        state: 'down',
        band: { kind: 'not_connected', refused: true },
        text: '데몬에 연결하지 못했습니다. 데몬이 이 앱의 인증을 거절했습니다',
      },
    ],
    [
      'band ②',
      { state: 'reconnecting', band: VERSION, error: '폴더 문제' },
      {
        state: 'reconnecting',
        band: VERSION,
        text: '데몬에 연결하지 못했습니다. 실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요',
      },
    ],
    ['no band while down (boot hold)', { state: 'down', band: null, error: '폴더 문제' }, { state: 'down', band: null, text: null }],
  ])('connection.status replies {state, band, text} — %s', async (_name, status, reply) => {
    await install(fakeClient(status))
    expect(registry.run('connection.status')).toEqual(reply)
  })

  it('connection.reconnect waits for the start, then replies the status', async () => {
    const client = fakeClient({ state: 'down', band: PLAIN })
    let finish!: () => void
    const { start } = await install(client, {
      start: () =>
        new Promise<void>(resolve => {
          finish = resolve
        }),
    })
    const reply = registry.run('connection.reconnect') as Promise<unknown>
    let settled = false
    void reply.then(() => (settled = true))
    for (let i = 0; i < 10; i++) await Promise.resolve()
    expect(settled).toBe(false)

    client.connectionState = 'connected'
    client.connectionBand = null
    finish()
    await expect(reply).resolves.toEqual({ state: 'connected', band: null, text: null })
    expect(start).toHaveBeenCalledTimes(1)
  })

  it('connection.reconnect rethrows the start error', async () => {
    const boom = new Error('데이터 폴더에 쓸 수 없음')
    await install(fakeClient({ state: 'down', band: PLAIN }), {
      start: async () => {
        throw boom
      },
    })
    await expect(registry.run('connection.reconnect')).rejects.toBe(boom)
  })

  it('daemon.stop passes the shutdown reply through and never disconnects', async () => {
    const client = fakeClient({ state: 'down', band: VERSION })
    const { shutdown } = await install(client, {
      shutdown: async (): Promise<DaemonShutdownReply> => ({ outcome: 'timeout', reason: 'still_running' }),
    })
    await expect(registry.run('daemon.stop')).resolves.toEqual({ outcome: 'timeout', reason: 'still_running' })
    expect(shutdown).toHaveBeenCalledTimes(1)
    expect(client.disconnect).not.toHaveBeenCalled()
  })

  it('daemon.stop passes a failed command through', async () => {
    await install(fakeClient({ state: 'down', band: null }), {
      shutdown: async () => {
        throw 'state not managed'
      },
    })
    await expect(registry.run('daemon.stop')).rejects.toBe('state not managed')
  })
})
