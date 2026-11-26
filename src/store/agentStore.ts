import { create } from 'zustand'

import type { AgentInfo, AgentProfile, AgentStatus } from '../api/types'

export const dummyAgents = [
  { id: '1', name: 'Assistant', status: 'running', cost: '$0.12' },
  { id: '2', name: 'Coder', status: 'idle', cost: '$0.21' },
  { id: '3', name: 'Reviewer', status: 'error', cost: '$0.08' },
]

export const dummyGroups = [{ id: 'g1', name: 'Coding rules', members: ['1', '2', '3'] }]

interface AgentState {
  /** The real agent list of the backend. agent-list-updated is the authoritative replacement basis (T-4). */
  agents: AgentInfo[]
  /**
   * Whether the authoritative roster has been received even once (ADR-0148). `agents: []` alone cannot tell "not received yet" (right after launch/reconnect) from "really empty" — the slot of an agent reaped by exit showed "Connecting to agent…". connectionState looks only at the transport layer, so it cannot stand in. setAgents sets it and it never comes down — boot pull, reconnect resync and broadcast all pass that one place.
   */
  agentsLoaded: boolean
  /**
   * All saved agent profiles (running ones included). No profile change event (ADR-0018): loaded once at boot + replaced by a listProfiles refetch right after create/delete/activate. The tree synthesizes profiles ∖ agents into "Reserved" nodes (mergeTreeNodes).
   */
  profiles: AgentProfile[]
  /**
   * Whether the profile list has been received even once (ADR-0148 — counterpart of agentsLoaded). The profile pull can be late or fail (the boot pull can finally fail after retries — `App.tsx` `bootPull`; other re-reads are one-shots), and reading `profiles: []` as "no profiles" would **misclassify a living corpse slot as "no target" and unmount early a view preserving the conversation**. So "no profile" is decided only after this is set (else it stays at "connecting").
   */
  profilesLoaded: boolean
  groups: typeof dummyGroups
  selectedAgentId: string | null
  setSelectedAgent: (id: string | null) => void
  /** Full replacement on agent-list-updated. Existence/removal is decided only by this. */
  setAgents: (agents: AgentInfo[]) => void
  /** Full replacement of the profiles with the listProfiles refetch result. */
  setProfiles: (profiles: AgentProfile[]) => void
  /**
   * On agent-status-changed, only that agent's status is updated (badge). T-4: Killed/Exited is not removed from the list; removal comes from the agent-list-updated the manager sends after the kill completes.
   */
  onStatusChanged: (id: string, status: AgentStatus) => void
}

export const useAgentStore = create<AgentState>(set => ({
  agents: [],
  agentsLoaded: false,
  profiles: [],
  profilesLoaded: false,
  groups: dummyGroups,
  selectedAgentId: null,
  setSelectedAgent: id => set({ selectedAgentId: id }),
  setAgents: agents => set({ agents, agentsLoaded: true }),
  setProfiles: profiles => set({ profiles, profilesLoaded: true }),
  onStatusChanged: (id, status) =>
    set(state => ({
      agents: state.agents.map(a => (a.id === id ? { ...a, status } : a)),
    })),
}))
