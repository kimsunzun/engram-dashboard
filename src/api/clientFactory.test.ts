// clientFactory 단위테스트 — 위치 모드(embedded/daemon) 선택 + window 노출(§5).
//
// ★Stage 3★: factory 는 mode → transport(InProc/Ws) → ProtocolClient 를 만든다. 반환은 항상
// ProtocolClient(단일 프로토콜 의미론). carrier 차이는 connectionState 로 관측한다 —
//   embedded(InProc) = 'connected'(항상), daemon(Ws) = 'down'(lazy connect 전).
// factory 는 모듈 로드 시점에 싱글톤(agentClient)을 만들고 resolveMode 가 window/localStorage
// 를 읽는다 → 각 케이스마다 vi.resetModules + 환경 셋업 후 dynamic import 로 격리한다.
// @tauri-apps/api/core 는 mock(transport 가 import 하나, 인스턴스화 시점엔 invoke 호출 안 함).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))

type Win = Window & {
  __ENGRAM_MODE__?: string
  __ENGRAM_AGENT__?: unknown
  __ENGRAM_DAEMON__?: unknown
}

function clearEnv() {
  const w = window as Win
  delete w.__ENGRAM_MODE__
  delete w.__ENGRAM_AGENT__
  delete w.__ENGRAM_DAEMON__
  try {
    window.localStorage.clear()
  } catch {
    /* ignore */
  }
}

beforeEach(() => {
  vi.resetModules()
  clearEnv()
})
afterEach(() => {
  clearEnv()
})

describe('clientFactory.resolveMode / getAgentClient', () => {
  it('기본(설정 없음) → ProtocolClient over InProc(connectionState 항상 connected)', async () => {
    const factory = await import('./clientFactory')
    const { ProtocolClient } = await import('./protocolClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(ProtocolClient)
    // InProc carrier 는 항상 connected(프로세스 수명=연결 수명).
    expect(client.connectionState).toBe('connected')
  })

  it('window.__ENGRAM_MODE__ = "daemon" → ProtocolClient over Ws(lazy, down)', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'daemon'
    const factory = await import('./clientFactory')
    const { ProtocolClient } = await import('./protocolClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(ProtocolClient)
    // Ws carrier 는 lazy connect — 명령 전 'down'.
    expect(client.connectionState).toBe('down')
  })

  it('localStorage engram_client_mode = "daemon" → Ws carrier(down)', async () => {
    window.localStorage.setItem('engram_client_mode', 'daemon')
    const factory = await import('./clientFactory')
    const { ProtocolClient } = await import('./protocolClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(ProtocolClient)
    expect(client.connectionState).toBe('down')
  })

  it('전역(__ENGRAM_MODE__)이 localStorage 보다 우선', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'embedded'
    window.localStorage.setItem('engram_client_mode', 'daemon')
    const factory = await import('./clientFactory')
    // embedded 우선 → InProc carrier → connected.
    expect(factory.getAgentClient().connectionState).toBe('connected')
  })

  it('getAgentClient 는 싱글톤(두 번 호출 동일 인스턴스) + window.__ENGRAM_AGENT__ 노출', async () => {
    const factory = await import('./clientFactory')
    const a = factory.getAgentClient()
    const b = factory.getAgentClient()
    expect(a).toBe(b)
    // §5 LLM-우선 제어: 제어 표면이 window 에 노출돼야 함.
    expect((window as Win).__ENGRAM_AGENT__).toBe(a)
  })

  it('daemon 모드 ProtocolClient 는 lazy connect — 인스턴스화만으로 invoke(discover) 호출 안 함', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'daemon'
    const core = await import('@tauri-apps/api/core')
    const factory = await import('./clientFactory')
    const client = factory.getAgentClient()
    expect(client.connectionState).toBe('down')
    // 명령 호출 전이므로 discover_daemon 미호출.
    expect(core.invoke).not.toHaveBeenCalled()
  })

  // ── ADR-0021: DaemonControl 노출 + mode별 구현 ───────────────────────────────────
  it('daemon 모드 → window.__ENGRAM_DAEMON__ = DaemonDaemonControl(실제)', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'daemon'
    const factory = await import('./clientFactory')
    const { DaemonDaemonControl } = await import('./daemonControl')
    const ctrl = factory.getDaemonControl()
    expect(ctrl).toBeInstanceOf(DaemonDaemonControl)
    expect((window as Win).__ENGRAM_DAEMON__).toBe(ctrl)
  })

  it('embedded 모드 → DaemonControl 은 no-op(EmbeddedDaemonControl), 노출은 동일', async () => {
    const factory = await import('./clientFactory')
    const { EmbeddedDaemonControl } = await import('./daemonControl')
    const ctrl = factory.getDaemonControl()
    expect(ctrl).toBeInstanceOf(EmbeddedDaemonControl)
    expect((window as Win).__ENGRAM_DAEMON__).toBe(ctrl)
  })
})
