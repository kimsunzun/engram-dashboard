// The factory creates the singleton (agentClient) at module load → isolate each case with vi.resetModules and then
// a dynamic import. @tauri-apps/api/core is mocked (invoke is not called at instantiation time).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))

type Win = Window & {
  __ENGRAM_AGENT__?: unknown
  __ENGRAM_DAEMON__?: unknown
}

function clearEnv() {
  const w = window as Win
  delete w.__ENGRAM_AGENT__
  delete w.__ENGRAM_DAEMON__
}

beforeEach(() => {
  vi.resetModules()
  clearEnv()
})
afterEach(() => {
  clearEnv()
})

describe('clientFactory (daemon-only)', () => {
  it('getAgentClient → ProtocolClient over Ws (lazy connect, so down before a command)', async () => {
    const factory = await import('./clientFactory')
    const { ProtocolClient } = await import('./protocolClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(ProtocolClient)
    expect(client.connectionState).toBe('down')
  })

  it('getAgentClient is a singleton (two calls give the same instance) + exposes window.__ENGRAM_AGENT__', async () => {
    const factory = await import('./clientFactory')
    const a = factory.getAgentClient()
    const b = factory.getAgentClient()
    expect(a).toBe(b)
    expect((window as Win).__ENGRAM_AGENT__).toBe(a)
  })

  it('instantiation alone does not call invoke(discover) — lazy connect', async () => {
    const core = await import('@tauri-apps/api/core')
    const factory = await import('./clientFactory')
    const client = factory.getAgentClient()
    expect(client.connectionState).toBe('down')
    expect(core.invoke).not.toHaveBeenCalled()
  })

  // ── ADR-0021: DaemonControl exposure (always the real implementation) ──────────────────────────────────
  it('getDaemonControl → DaemonDaemonControl (real) + exposes window.__ENGRAM_DAEMON__', async () => {
    const factory = await import('./clientFactory')
    const { DaemonDaemonControl } = await import('./daemonControl')
    const ctrl = factory.getDaemonControl()
    expect(ctrl).toBeInstanceOf(DaemonDaemonControl)
    expect((window as Win).__ENGRAM_DAEMON__).toBe(ctrl)
  })
})
