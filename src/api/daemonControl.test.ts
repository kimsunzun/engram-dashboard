import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

import { DaemonDaemonControl } from './daemonControl'
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

describe('DaemonDaemonControl (daemon 모드)', () => {
  // ★R7★: if an explicit start fails, its reason must be carried on the connection state surface — boot · UI ·
  //   window.__ENGRAM_DAEMON__ all go through this single path, so recording it here covers all of them.
  it('start 실패 → 이유를 연결 상태 표면에 싣고 그대로 throw', async () => {
    const report = vi.fn(() => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { reportConnectionError: report }))
    invokeMock.mockRejectedValueOnce('데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요')
    await expect(ctrl.start()).rejects.toBeDefined()
    expect(report).toHaveBeenCalledWith(
      '데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요',
    )
  })

  it('start 성공 → 남아 있던 이유를 지운다', async () => {
    const report = vi.fn(() => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { reportConnectionError: report }))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 })
    await ctrl.start()
    expect(report).toHaveBeenCalledWith(null)
  })

  it('start → daemon_start invoke(console/timeout 전달) + client.connect(명시 spawn 연결)', async () => {
    const connect = vi.fn(async () => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { connect }))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 })
    await ctrl.start({ console: true, timeoutMs: 3000 })
    expect(invokeMock).toHaveBeenCalledWith('daemon_start', { console: true, timeoutMs: 3000 })
    // ADR-0021 §1: only start calls client.connect (= transport.start, spawn allowed).
    expect(connect).toHaveBeenCalledTimes(1)
  })

  it('start 기본값 → console=false, timeoutMs=null', async () => {
    const ctrl = new DaemonDaemonControl(fakeClient('down'))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, protocol_version: 1 })
    await ctrl.start()
    expect(invokeMock).toHaveBeenCalledWith('daemon_start', { console: false, timeoutMs: null })
  })

  it('status → daemon_status invoke 결과 반환', async () => {
    const ctrl = new DaemonDaemonControl(fakeClient('connected'))
    invokeMock.mockResolvedValueOnce({ alive: true, pid: 42, port: 9999 })
    const s = await ctrl.status()
    expect(invokeMock).toHaveBeenCalledWith('daemon_status')
    expect(s).toEqual({ alive: true, pid: 42, port: 9999 })
  })
})
