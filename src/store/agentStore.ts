import { create } from 'zustand'

export const dummyAgents = [
  { id: '1', name: '비서',   status: 'running', cost: '$0.12' },
  { id: '2', name: '코더',   status: 'idle',    cost: '$0.21' },
  { id: '3', name: '리뷰어', status: 'error',   cost: '$0.08' },
]

export const dummyGroups = [
  { id: 'g1', name: '코딩룰', members: ['1', '2', '3'] },
]

interface AgentState {
  agents: typeof dummyAgents
  groups: typeof dummyGroups
  selectedAgentId: string | null
  setSelectedAgent: (id: string | null) => void
}

export const useAgentStore = create<AgentState>((set) => ({
  agents: dummyAgents,
  groups: dummyGroups,
  selectedAgentId: null,
  setSelectedAgent: (id) => set({ selectedAgentId: id }),
}))
