// AgentClient 팩토리 — 위치 모드(Embedded/Daemon) 선택 지점(daemon-design §8-d #7, phase 4).
// 지금은 Embedded 만. phase 4 에서 startup config(mode=embedded|daemon)로 DaemonClient 분기.
// 라이브 핫스왑 안 함 — 부팅 시 한 번 고른다.

import type { AgentClient } from './agentClient'
import { EmbeddedClient } from './embeddedClient'

let instance: AgentClient | null = null

/** 단일 AgentClient 인스턴스. 컴포넌트·스토어·(미래)LLM 이 모두 이걸 통한다. */
export function getAgentClient(): AgentClient {
  if (!instance) {
    instance = new EmbeddedClient()
    // §5 LLM-우선 제어: 제어 표면을 window 에 노출 — cdp.mjs eval / (미래) 백엔드측 LLM 이
    // 사람 클릭과 동일 진입점을 호출할 수 있게 한다(임시 경로, 정식 command 버스 전까지).
    ;(window as unknown as { __ENGRAM_AGENT__?: AgentClient }).__ENGRAM_AGENT__ = instance
  }
  return instance
}

/** 모듈 로드 시점 싱글톤(대부분 컴포넌트는 이걸 import). */
export const agentClient = getAgentClient()
