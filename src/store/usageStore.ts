// Value mirror of the usage slot (TRD S21 usage-limit-slot §1-8) — the last snapshot per company and the memory of the rule that accepted it.
// ★Receiving is only the one place `merge`★ — the shell broadcast (eventBus connects it) and the shell cache pull share the rule; the two paths guarantee no order, so a late old copy is sorted out here. The ⟳ answer (`Ack`) carries no value and only releases the wait.
// ★Interest (which company one wants to see) is not here★ — the shell computes it from the layout, so broadcasts of companies not turned on enter here too; filtering is the drawing side's job.
// The store calls only `agentClient` (ADR-0011); the pull, a shell command, is also behind the transport seam (ADR-0036).

import { create } from 'zustand'

import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { UsageVendorState } from '../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import { agentClient } from '../api/clientFactory'
import type { AgentBackendKind } from '../api/types'
import { retryAsync } from '../util/retryInvoke'

/**
 * One company's stored value. The key is one company — `account_key` is always `"default"` for now, so it is not told apart (if accounts grow, revision becomes per company · account and the key must widen).
 */
export interface UsageVendorEntry {
  snapshot: UsageLimitSnapshot
  /** `performance.now()` (ms) at the moment of acceptance. The snapshot's relative time fields (`age_secs` etc.) are based on this moment. */
  receivedAt: number
  /**
   * The highest revision accepted on the current socket. `null` = forgotten (a larger `socketEpoch` was received) — the next sheet is accepted regardless of revision. `snapshot` is left as is even when forgotten.
   */
  revision: number | null
}

interface UsageState {
  vendors: Partial<Record<AgentBackendKind, UsageVendorEntry>>
  /** The largest socket marker accepted (`UsageSnapshotPull.socketEpoch`). 0 = none yet. */
  socketEpoch: number
  /** ⟳ pending count (per company — shared among slots inside the webview). To read it use [`useUsagePending`]. */
  pending: Partial<Record<AgentBackendKind, number>>
  merge(snapshot: UsageLimitSnapshot, socketEpoch: number): void
  /**
   * Pull the shell cache and `merge` it — bounded retry; a final failure leaves the held value (does not throw). ★Call only after broadcast reception is up★ — eventBus calls it once right after connecting, the slot on mount.
   */
  pull(): Promise<void>
  /** ⟳ — when it ends (answer · failure · refusal on disconnect) release the wait. The value comes by broadcast. Does not throw. */
  refresh(vendor: AgentBackendKind): Promise<void>
}

function forgetRevisions(vendors: UsageState['vendors']): UsageState['vendors'] {
  const next: UsageState['vendors'] = {}
  for (const [vendor, entry] of Object.entries(vendors) as [AgentBackendKind, UsageVendorEntry][]) {
    next[vendor] = { ...entry, revision: null }
  }
  return next
}

function addPending(
  pending: UsageState['pending'],
  vendor: AgentBackendKind,
  delta: number,
): UsageState['pending'] {
  return { ...pending, [vendor]: Math.max(0, (pending[vendor] ?? 0) + delta) }
}

export const useUsageStore = create<UsageState>()((set, get) => ({
  vendors: {},
  socketEpoch: 0,
  pending: {},

  merge: (snapshot, socketEpoch) =>
    set(state => {
      let vendors = state.vendors
      let heldEpoch = state.socketEpoch
      // ADR-0195: the socket marker is not rewound inside the shell process, so larger = later socket; a smaller one belongs to a pushed-out socket. ★Revision is forgotten here only★ — not on a connection state transition (leaving `connected`), which comes by another path and may arrive after the new socket's payload; erasing the floor then would accept a late old copy of the same socket and rewind the screen. 0 = marker unknown — keep the held marker.
      if (socketEpoch !== 0) {
        if (socketEpoch < heldEpoch) return state
        if (socketEpoch > heldEpoch) {
          heldEpoch = socketEpoch
          vendors = forgetRevisions(vendors)
        }
      }
      const prev = vendors[snapshot.vendor]
      // The same revision is accepted — it is a copy of the same state taken later, so the relative time fields are fresher.
      if (prev && prev.revision !== null && snapshot.revision < prev.revision) return state
      return {
        socketEpoch: heldEpoch,
        vendors: {
          ...vendors,
          [snapshot.vendor]: { snapshot, receivedAt: performance.now(), revision: snapshot.revision },
        },
      }
    }),

  pull: async () => {
    try {
      const { socketEpoch, snapshots } = await retryAsync(() => agentClient.getUsageSnapshot(), {
        onRetry: (err, attempt) => console.warn(`[usageStore] retrying getUsageSnapshot #${attempt}:`, err),
      })
      for (const snapshot of snapshots) get().merge(snapshot, socketEpoch)
    } catch (err) {
      console.warn('[usageStore] getUsageSnapshot finally failed — keeping the held value:', err)
    }
  },

  refresh: async vendor => {
    set(state => ({ pending: addPending(state.pending, vendor, 1) }))
    try {
      await agentClient.refreshUsageLimits(vendor)
    } catch (err) {
      // On a disconnect ProtocolClient refuses all pending requests — that refusal is the path that releases the wait here.
      console.warn(`[usageStore] refreshUsageLimits(${vendor}) failed:`, err)
    } finally {
      set(state => ({ pending: addPending(state.pending, vendor, -1) }))
    }
  },
}))

/** That company's stored value — `undefined` if none (not a single one received yet). */
export function useUsageVendor(vendor: AgentBackendKind): UsageVendorEntry | undefined {
  return useUsageStore(state => state.vendors[vendor])
}

/** Has that company's ⟳ not yet finished. */
export function useUsagePending(vendor: AgentBackendKind): boolean {
  return useUsageStore(state => (state.pending[vendor] ?? 0) > 0)
}

/**
 * Refusal that blocks ⟳ = the visible `Rejected` state. Both ⟳ (small display · per-company in the popup) and the target selection of `usageSlot.refresh` read this. A refusal while a value shows as `Ready` is not caught — then the daemon answers ⟳ from cache without a query (TRD §3 #88).
 */
export function blocksRefresh(state: UsageVendorState | undefined): boolean {
  return state?.kind === 'Rejected'
}

/**
 * Among the turned-on companies, those that can send ⟳ — not caught by [`blocksRefresh`]; includes a company with no value yet (⟳ is how to call for the first value). `usageSlot.refresh` re-reads the store when pressed and picks targets with this.
 */
export function refreshableVendors(
  shows: { show_claude: boolean; show_codex: boolean },
  vendors: UsageState['vendors'],
): AgentBackendKind[] {
  const shown: AgentBackendKind[] = []
  if (shows.show_claude) shown.push('claude')
  if (shows.show_codex) shown.push('codex')
  return shown.filter(vendor => !blocksRefresh(vendors[vendor]?.snapshot.state))
}
