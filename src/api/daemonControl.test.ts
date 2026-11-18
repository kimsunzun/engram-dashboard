import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

import { DaemonDaemonControl, shutdownDaemon } from './daemonControl'
import type { AgentClient, ConnectionState } from './agentClient'

function fakeClient(
  state: ConnectionState,
  overrides: {
    connect?: ReturnType<typeof vi.fn>
    reportConnectionError?: ReturnType<typeof vi.fn>
  } = {},
): AgentClient {
  return {
    connectionState: state,
    connectionError: null,
    connect: overrides.connect ?? vi.fn(async () => undefined),
    reportConnectionError: overrides.reportConnectionError ?? vi.fn(() => undefined),
  } as unknown as AgentClient
}

beforeEach(() => {
  invokeMock.mockClear()
  invokeMock.mockImplementation(async () => undefined)
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('DaemonDaemonControl (daemon mode)', () => {
  // ★R7★: if an explicit start fails, its reason must be carried on the connection state surface — boot · UI ·
  //   window.__ENGRAM_DAEMON__ all go through this single path, so recording it here covers all of them.
  it('start failure → carries the reason on the connection state surface and throws it as is', async () => {
    const report = vi.fn(() => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { reportConnectionError: report }))
    invokeMock.mockRejectedValueOnce('cannot write to the data folder (C:\\x) — unzip it to a writable location')
    await expect(ctrl.start()).rejects.toBeDefined()
    expect(report).toHaveBeenCalledWith(
      'cannot write to the data folder (C:\\x) — unzip it to a writable location',
    )
  })

  it('start success → clears the reason left behind', async () => {
    const report = vi.fn((_reason: string | null) => undefined)
    // A cause written while this start was connecting (e.g. a concurrent attempt that failed) — the success clears it.
    const connect = vi.fn(async () => {
      report('a reason from another attempt')
    })
    const ctrl = new DaemonDaemonControl(fakeClient('down', { connect, reportConnectionError: report }))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 })
    await ctrl.start()
    expect(report).toHaveBeenLastCalledWith(null)
  })

  it('start → daemon_start invoke (console/timeout passed) + client.connect (explicit spawn connection)', async () => {
    const connect = vi.fn(async () => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { connect }))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 })
    await ctrl.start({ console: true, timeoutMs: 3000 })
    expect(invokeMock).toHaveBeenCalledWith('daemon_start', { console: true, timeoutMs: 3000 })
    // ADR-0021 §1: only start calls client.connect (= transport.start, spawn allowed).
    expect(connect).toHaveBeenCalledTimes(1)
  })

  it('start defaults → console=false, timeoutMs=null', async () => {
    const ctrl = new DaemonDaemonControl(fakeClient('down'))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 })
    await ctrl.start()
    expect(invokeMock).toHaveBeenCalledWith('daemon_start', { console: false, timeoutMs: null })
  })

  // TRD S36 §3-5 (PRD R7): a new attempt clears the previous attempt's cause before it runs.
  it.each([
    ['succeeds', true],
    ['fails', false],
  ])('start clears the old cause before daemon_start when the attempt %s', async (_name, ok) => {
    const order: string[] = []
    const report = vi.fn((reason: string | null) => {
      order.push(`report:${String(reason)}`)
    })
    const ctrl = new DaemonDaemonControl(fakeClient('down', { reportConnectionError: report }))
    invokeMock.mockImplementationOnce(async (cmd: string) => {
      order.push(cmd)
      if (!ok) throw new Error('new reason')
      return { pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 }
    })
    await ctrl.start().catch(() => undefined)
    expect(order.slice(0, 2)).toEqual(['report:null', 'daemon_start'])
  })

  it('status → returns the daemon_status invoke result', async () => {
    const ctrl = new DaemonDaemonControl(fakeClient('connected'))
    invokeMock.mockResolvedValueOnce({ alive: true, pid: 42, port: 9999 })
    const s = await ctrl.status()
    expect(invokeMock).toHaveBeenCalledWith('daemon_status')
    expect(s).toEqual({ alive: true, pid: 42, port: 9999 })
  })
})

describe('shutdownDaemon', () => {
  it('invokes daemon_shutdown and returns its reply as is', async () => {
    invokeMock.mockResolvedValueOnce({ outcome: 'closed', reason: null })
    await expect(shutdownDaemon()).resolves.toEqual({ outcome: 'closed', reason: null })
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('daemon_shutdown')
  })

  it('passes a rejection through (the command did not run)', async () => {
    invokeMock.mockRejectedValueOnce('no shell client')
    await expect(shutdownDaemon()).rejects.toBe('no shell client')
  })
})
