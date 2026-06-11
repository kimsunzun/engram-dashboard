import { create } from 'zustand'

import type { AgentInfo, AgentStatus } from '../api/types'

// AgentTree가 직접 참조하는 더미 데이터 — 3c에서 실제 트리 연결 전까지 유지.
export const dummyAgents = [
  { id: '1', name: '비서', status: 'running', cost: '$0.12' },
  { id: '2', name: '코더', status: 'idle', cost: '$0.21' },
  { id: '3', name: '리뷰어', status: 'error', cost: '$0.08' },
]

export const dummyGroups = [{ id: 'g1', name: '코딩룰', members: ['1', '2', '3'] }]

interface AgentState {
  /** 백엔드 실제 에이전트 목록. agent-list-updated가 권위 있는 교체 기준(T-4). */
  agents: AgentInfo[]
  groups: typeof dummyGroups
  selectedAgentId: string | null
  setSelectedAgent: (id: string | null) => void
  /** agent-list-updated 수신 시 전체 교체. 존재/제거 판정은 이것만. */
  setAgents: (agents: AgentInfo[]) => void
  /**
   * agent-status-changed 수신 시 해당 agent의 status만 갱신(뱃지 표시용).
   * T-4: Killed/Exited를 받아도 목록에서 제거하지 않는다.
   * 실제 제거는 kill 완료 후 manager가 보내는 agent-list-updated가 담당.
   */
  onStatusChanged: (id: string, status: AgentStatus) => void
}

export const useAgentStore = create<AgentState>(set => ({
  agents: [],
  groups: dummyGroups,
  selectedAgentId: null,
  setSelectedAgent: id => set({ selectedAgentId: id }),
  setAgents: agents => set({ agents }),
  onStatusChanged: (id, status) =>
    set(state => ({
      agents: state.agents.map(a => (a.id === id ? { ...a, status } : a)),
    })),
}))
