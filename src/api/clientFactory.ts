// AgentClient 팩토리 — 위치 모드(embedded/daemon) 선택 지점(ADR-0020 Stage 3).
// 부팅 시 1회 모드를 고른다 — 라이브 핫스왑 안 함. 기본 'embedded'(회귀 0, 기존 동작 불변).
// daemon 은 명시 opt-in: window.__ENGRAM_MODE__ 전역 또는 localStorage 'engram_client_mode'.
//
// ★Stage 3 전환★: mode → transport(InProc/Ws) 선택 → new ProtocolClient(transport). 단일
// ProtocolClient(프로토콜 의미론 1벌) + carrier 2개. EmbeddedClient/DaemonClient 클래스는
// Stage 4 에서 ptyApi·옛 Tauri command 와 함께 제거(현재 파일은 잔류, factory 만 전환).

import type { AgentClient } from './agentClient'
import { InProcTransport } from './inProcTransport'
import { ProtocolClient } from './protocolClient'
import type { Transport } from './transport'
import { WsTransport } from './wsTransport'

type ClientMode = 'embedded' | 'daemon'

/** 부팅 시 1회 모드 결정. 전역(__ENGRAM_MODE__) 우선, 없으면 localStorage, 그래도 없으면 embedded. */
function resolveMode(): ClientMode {
  const fromGlobal = (window as unknown as { __ENGRAM_MODE__?: string }).__ENGRAM_MODE__
  if (fromGlobal === 'daemon' || fromGlobal === 'embedded') return fromGlobal
  try {
    const stored = window.localStorage.getItem('engram_client_mode')
    if (stored === 'daemon' || stored === 'embedded') return stored
  } catch {
    // localStorage 접근 불가(예: 일부 컨텍스트) — 기본값 fallback.
  }
  return 'embedded'
}

let instance: AgentClient | null = null

/** 단일 AgentClient 인스턴스. 컴포넌트·스토어·(미래)LLM 이 모두 이걸 통한다. */
export function getAgentClient(): AgentClient {
  if (!instance) {
    const mode = resolveMode()
    // mode → carrier 선택. 프로토콜 의미론은 ProtocolClient 한 곳(carrier 무관).
    const transport: Transport = mode === 'daemon' ? new WsTransport() : new InProcTransport()
    instance = new ProtocolClient(transport)
    // §5 LLM-우선 제어: 제어 표면을 window 에 노출 — cdp.mjs eval / (미래) 백엔드측 LLM 이
    // 사람 클릭과 동일 진입점을 호출할 수 있게 한다(임시 경로, 정식 command 버스 전까지).
    ;(window as unknown as { __ENGRAM_AGENT__?: AgentClient }).__ENGRAM_AGENT__ = instance
  }
  return instance
}

/** 모듈 로드 시점 싱글톤(대부분 컴포넌트는 이걸 import). */
export const agentClient = getAgentClient()
