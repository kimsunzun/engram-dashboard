// DaemonControl 단위테스트 — ADR-0021 §5 lifecycle 제어 표면.
//
// daemon_start/stop/status Tauri command(invoke) mock + fake AgentClient 로 graceful→fallback
// 순서·연결상태별 분기를 검증한다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, ..._rest: unknown[]) => undefined as unknown)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, ...rest: unknown[]) => invokeMock(cmd, ...rest),
}))

import { DaemonDaemonControl } from './daemonControl'
import type { AgentClient, ConnectionState } from './agentClient'

// stopDaemon/connect/disconnect/connectionState 만 쓰는 최소 fake client.
function fakeClient(
  state: ConnectionState,
  overrides: {
    stopDaemon?: ReturnType<typeof vi.fn>
    connect?: ReturnType<typeof vi.fn>
    disconnect?: ReturnType<typeof vi.fn>
  } = {},
): AgentClient {
  return {
    connectionState: state,
    stopDaemon: overrides.stopDaemon ?? vi.fn(async () => undefined),
    connect: overrides.connect ?? vi.fn(async () => undefined),
    disconnect: overrides.disconnect ?? vi.fn(() => undefined),
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
  it('start → daemon_start invoke(console/timeout 전달) + client.connect(명시 spawn 연결)', async () => {
    const connect = vi.fn(async () => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { connect }))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, token: 't', protocol_version: 1 })
    await ctrl.start({ console: true, timeoutMs: 3000 })
    expect(invokeMock).toHaveBeenCalledWith('daemon_start', { console: true, timeoutMs: 3000 })
    // ADR-0021 §1: start 만 client.connect(=transport.start, spawn 허용)를 호출한다.
    expect(connect).toHaveBeenCalledTimes(1)
  })

  it('start 기본값 → console=false, timeoutMs=null', async () => {
    const ctrl = new DaemonDaemonControl(fakeClient('down'))
    invokeMock.mockResolvedValueOnce({ pid: 1, host: '127.0.0.1', port: 5, token: 't', protocol_version: 1 })
    await ctrl.start()
    expect(invokeMock).toHaveBeenCalledWith('daemon_start', { console: false, timeoutMs: null })
  })

  it('stop(연결 있음) → graceful StopDaemon → disconnect → still-alive 확인 → fallback', async () => {
    const stop = vi.fn(async () => undefined)
    const disconnect = vi.fn(() => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('connected', { stopDaemon: stop, disconnect }))
    // graceful 이 Ack 됐으므로 M-1: daemon_status 로 still-alive 확인. 죽어있다고 응답 → fallback 스킵.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'daemon_status') return { alive: false, pid: null, port: null }
      return undefined
    })
    await ctrl.stop()
    expect(stop).toHaveBeenCalledWith(false) // graceful 우선.
    expect(disconnect).toHaveBeenCalledTimes(1) // note3: 재연결 노이즈 제거.
    expect(invokeMock).toHaveBeenCalledWith('daemon_status') // M-1: still-alive 확인.
    // 데몬이 graceful 로 이미 내려갔으므로 taskkill(daemon_stop) 안 함(race 회피).
    expect(invokeMock).not.toHaveBeenCalledWith('daemon_stop')
  })

  it('stop(연결 있음) graceful 후에도 살아있으면 → fallback daemon_stop(M-1)', async () => {
    const stop = vi.fn(async () => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('connected', { stopDaemon: stop }))
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'daemon_status') return { alive: true, pid: 9, port: 1 } // 아직 살아있음
      return undefined
    })
    await ctrl.stop()
    expect(invokeMock).toHaveBeenCalledWith('daemon_status')
    expect(invokeMock).toHaveBeenCalledWith('daemon_stop') // still-alive → fallback kill.
  })

  it('stop(연결 없음) → graceful 스킵, disconnect 후 곧장 fallback daemon_stop(status 확인 안 함)', async () => {
    const stop = vi.fn(async () => undefined)
    const disconnect = vi.fn(() => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('down', { stopDaemon: stop, disconnect }))
    await ctrl.stop()
    expect(stop).not.toHaveBeenCalled() // 연결 없으면 StopDaemon 전송 안 함.
    expect(disconnect).toHaveBeenCalledTimes(1)
    // graceful 이 없었으므로 status 확인 없이 곧장 fallback kill.
    expect(invokeMock).not.toHaveBeenCalledWith('daemon_status')
    expect(invokeMock).toHaveBeenCalledWith('daemon_stop')
  })

  it('stop graceful 거부(active agents)해도 status 확인 없이 fallback kill 로 진행', async () => {
    const stop = vi.fn(async () => {
      throw new Error('active agents present')
    })
    const ctrl = new DaemonDaemonControl(fakeClient('connected', { stopDaemon: stop }))
    await expect(ctrl.stop({ force: false })).resolves.toBeUndefined()
    expect(stop).toHaveBeenCalledWith(false)
    // graceful 실패 → gracefulOk=false → status 확인 건너뛰고 곧장 fallback.
    expect(invokeMock).not.toHaveBeenCalledWith('daemon_status')
    expect(invokeMock).toHaveBeenCalledWith('daemon_stop')
  })

  it('stop(force) → graceful 에 force=true 전달', async () => {
    const stop = vi.fn(async () => undefined)
    const ctrl = new DaemonDaemonControl(fakeClient('connected', { stopDaemon: stop }))
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'daemon_status') return { alive: false, pid: null, port: null }
      return undefined
    })
    await ctrl.stop({ force: true })
    expect(stop).toHaveBeenCalledWith(true)
  })

  it('status → daemon_status invoke 결과 반환', async () => {
    const ctrl = new DaemonDaemonControl(fakeClient('connected'))
    invokeMock.mockResolvedValueOnce({ alive: true, pid: 42, port: 9999 })
    const s = await ctrl.status()
    expect(invokeMock).toHaveBeenCalledWith('daemon_status')
    expect(s).toEqual({ alive: true, pid: 42, port: 9999 })
  })
})
