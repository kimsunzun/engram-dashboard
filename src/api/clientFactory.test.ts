// clientFactory 단위테스트 — 위치 모드(Embedded/Daemon) 선택 + window 노출(§5).
//
// factory 는 모듈 로드 시점에 싱글톤(agentClient)을 만들고 resolveMode 가 window/localStorage
// 를 읽는다 → 각 케이스마다 vi.resetModules + 환경 셋업 후 dynamic import 로 격리한다.
// @tauri-apps/api/core 는 mock(EmbeddedClient 가 import 하나, 인스턴스화 시점엔 호출 안 함).

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
}

function clearEnv() {
  const w = window as Win
  delete w.__ENGRAM_MODE__
  delete w.__ENGRAM_AGENT__
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
  it('기본(설정 없음) → EmbeddedClient(connectionState 항상 connected)', async () => {
    const factory = await import('./clientFactory')
    const { EmbeddedClient } = await import('./embeddedClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(EmbeddedClient)
    // Embedded 는 항상 connected.
    expect(client.connectionState).toBe('connected')
  })

  it('window.__ENGRAM_MODE__ = "daemon" → DaemonClient', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'daemon'
    const factory = await import('./clientFactory')
    const { DaemonClient } = await import('./daemonClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(DaemonClient)
  })

  it('localStorage engram_client_mode = "daemon" → DaemonClient', async () => {
    window.localStorage.setItem('engram_client_mode', 'daemon')
    const factory = await import('./clientFactory')
    const { DaemonClient } = await import('./daemonClient')
    const client = factory.getAgentClient()
    expect(client).toBeInstanceOf(DaemonClient)
  })

  it('전역(__ENGRAM_MODE__)이 localStorage 보다 우선', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'embedded'
    window.localStorage.setItem('engram_client_mode', 'daemon')
    const factory = await import('./clientFactory')
    const { EmbeddedClient } = await import('./embeddedClient')
    expect(factory.getAgentClient()).toBeInstanceOf(EmbeddedClient)
  })

  it('getAgentClient 는 싱글톤(두 번 호출 동일 인스턴스) + window.__ENGRAM_AGENT__ 노출', async () => {
    const factory = await import('./clientFactory')
    const a = factory.getAgentClient()
    const b = factory.getAgentClient()
    expect(a).toBe(b)
    // §5 LLM-우선 제어: 제어 표면이 window 에 노출돼야 함.
    expect((window as Win).__ENGRAM_AGENT__).toBe(a)
  })

  it('DaemonClient 는 lazy connect — 인스턴스화만으로 invoke(discover) 호출 안 함', async () => {
    ;(window as Win).__ENGRAM_MODE__ = 'daemon'
    const core = await import('@tauri-apps/api/core')
    const factory = await import('./clientFactory')
    const client = factory.getAgentClient()
    expect(client.connectionState).toBe('down')
    // 명령 호출 전이므로 discover_daemon 미호출.
    expect(core.invoke).not.toHaveBeenCalled()
  })
})
