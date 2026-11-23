// Value mirror of the PC list slot — the daemon's peer state: this PC's ID and addresses, the trusted PCs with their link
//   state, refused PCs, waiting requests and the network settings. ★All behaviour is the daemon's★ — this store only holds what came.
// Two ways in, one shape: the broadcast (`AgentEvent::PeerStateUpdated`, relayed while the shell reports a PC list slot — eventBus hooks it)
//   and the slot's own `peer.list` read on mount (which also makes this PC's ID the first time — ADR-0389 d3). A read answer that comes back
//   after a broadcast arrived is dropped: the broadcast is newer (copies of one connection come in order). The read leaves the agent
//   rows out (`--agents false`), as the broadcast does: the slot does not show them, and every PC's rows could grow one answer past
//   the shell's message cap.
// The store calls only `agentClient` (ADR-0011).
// ADR-0394

import { create } from 'zustand'

import type { PeerState } from '../../crates/engram-dashboard-protocol/bindings/PeerState'
import { agentClient } from '../api/clientFactory'

interface PeerStoreState {
  /** The last state received — `null` until the first one. */
  state: PeerState | null
  /** Broadcasts received so far — a read started before the latest one does not overwrite it. */
  broadcasts: number
  /** The last read failed (connection down, daemon refused) — shown as a line; the held state stays. */
  readFailed: boolean
  receive(state: PeerState): void
  /** Reads `peer.list` (the human path) and keeps the answer unless a broadcast came meanwhile. Does not throw. */
  read(): Promise<void>
}

export const usePeerStore = create<PeerStoreState>()((set, get) => ({
  state: null,
  broadcasts: 0,
  readFailed: false,

  receive: state => set(s => ({ state, broadcasts: s.broadcasts + 1, readFailed: false })),

  read: async () => {
    const before = get().broadcasts
    try {
      const answer = (await agentClient.peerCommand('peer.list', { agents: false })) as PeerState
      if (get().broadcasts === before) set({ state: answer, readFailed: false })
    } catch (e) {
      console.warn('[peerStore] peer.list failed:', e)
      set({ readFailed: true })
    }
  },
}))
