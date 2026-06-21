// clientFactory 단위테스트 — daemon-only(ADR-0029) + window 노출(§5).
//
// factory 는 항상 ProtocolClient over WsTransport(데몬 attach) + DaemonDaemonControl 을 만든다.
// 모드 개념 없음 — carrier 는 WsTransport 고정(lazy connect 라 명령 전 'down').
// factory 는 모듈 로드 시점에 싱글톤(agentClient)을 만든다 → 각 케이스마다 vi.resetModules 후
// dynamic import 로 격리한다. @tauri-apps/api/core 는 mock(인스턴스화 시점엔 invoke 호출 안 함).

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
  it('getAgentClient → ProtocolClient over Ws(lazy connect 라 명령 전 down)', async () => {
    const factory = await import('./clientFactory')
    const { ProtocolClient } = await import('./protocolClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(ProtocolClient)
    // Ws carrier 는 lazy connect — 명령 전 'down'.
    expect(client.connectionState).toBe('down')
  })

  it('getAgentClient 는 싱글톤(두 번 호출 동일 인스턴스) + window.__ENGRAM_AGENT__ 노출', async () => {
    const factory = await import('./clientFactory')
    const a = factory.getAgentClient()
    const b = factory.getAgentClient()
    expect(a).toBe(b)
    // §5 LLM-우선 제어: 제어 표면이 window 에 노출돼야 함.
    expect((window as Win).__ENGRAM_AGENT__).toBe(a)
  })

  it('인스턴스화만으로 invoke(discover) 호출 안 함 — lazy connect', async () => {
    const core = await import('@tauri-apps/api/core')
    const factory = await import('./clientFactory')
    const client = factory.getAgentClient()
    expect(client.connectionState).toBe('down')
    // 명령 호출 전이므로 discover_daemon 미호출.
    expect(core.invoke).not.toHaveBeenCalled()
  })

  // ── ADR-0021: DaemonControl 노출(항상 실제 구현) ──────────────────────────────────
  it('getDaemonControl → DaemonDaemonControl(실제) + window.__ENGRAM_DAEMON__ 노출', async () => {
    const factory = await import('./clientFactory')
    const { DaemonDaemonControl } = await import('./daemonControl')
    const ctrl = factory.getDaemonControl()
    expect(ctrl).toBeInstanceOf(DaemonDaemonControl)
    expect((window as Win).__ENGRAM_DAEMON__).toBe(ctrl)
  })
})
