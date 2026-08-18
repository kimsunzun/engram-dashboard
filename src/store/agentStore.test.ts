// agentsLoaded 배선 회귀(ADR-0148).
//
// ★왜 이 한 줄에 테스트가 붙나★: 이 플래그가 "권위 명부를 아직 못 받음"과 "정말 비었음"을 가르는 유일한
//   신호다. 안 서면 종료로 수거된 에이전트의 슬롯이 영구히 「에이전트 연결 중…」으로 보이고(부재 표현이
//   영영 안 뜬다), 반대로 초기값이 true 면 기동 직후 모든 슬롯이 부재로 판정돼 죽은 것처럼 보인다.
//   두 오작동 모두 화면을 봐야 드러나므로 여기서 못 박는다.
//
// 부팅 pull(App.tsx) · 재연결 재동기(eventBus) · broadcast 가 전부 setAgents 한 곳을 지나므로 그 한 곳만 본다.

import { beforeEach, describe, expect, it } from 'vitest'

import type { AgentInfo } from '../api/types'
import { useAgentStore } from './agentStore'

function agent(id: string): AgentInfo {
  return {
    id,
    name: id,
    cwd: 'C:/x',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch: 0,
    capabilities: {
      input: { raw: true, message: false, attachment: false },
      output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
      control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: false },
      session: { resume: true, snapshot: false, cwd_env: true },
      model: { select: false, temperature: false, max_tokens: false },
    },
  }
}

beforeEach(() => {
  useAgentStore.setState({ agents: [], agentsLoaded: false })
})

describe('agentStore — agentsLoaded', () => {
  it('초기값은 false(아직 못 받음)', () => {
    expect(useAgentStore.getState().agentsLoaded).toBe(false)
  })

  it('setAgents 가 서면 다시 내려가지 않는다 — 빈 목록으로 받아도 "받았음" 이다', () => {
    // ★빈 배열이 핵심 케이스★: 마지막 에이전트를 종료하면 명부는 []로 온다. 그때도 플래그가 서야
    //   "정말 비었음" 으로 판정된다.
    useAgentStore.getState().setAgents([])
    expect(useAgentStore.getState().agentsLoaded).toBe(true)

    useAgentStore.getState().setAgents([agent('a1')])
    expect(useAgentStore.getState().agentsLoaded).toBe(true)

    useAgentStore.getState().setAgents([])
    expect(useAgentStore.getState().agentsLoaded).toBe(true)
    expect(useAgentStore.getState().agents).toEqual([])
  })
})
