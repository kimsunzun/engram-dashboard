// Value mirror of the usage slot (TRD S21 usage-limit-slot §1-8) — the last snapshot per company, and the memory of the rule that accepted it.
//
// ★Receiving is only the one place `merge`★ — the shell broadcast (eventBus connects it) and the shell cache pull come in by the same rule. The two paths
//   guarantee no order between them, so an old copy that arrived late is sorted out here. The ⟳ answer (`Ack`) carries no value and only releases the wait.
// ★Interest (which company one wants to see) is not here★ — the shell computes it from the layout. So broadcasts of companies not turned on
//   enter here too — filtering is the drawing side's job.
// The store calls only `agentClient` (ADR-0011) — the pull, which is a shell command, is also behind the transport seam (ADR-0036).

import { create } from 'zustand'

import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { UsageVendorState } from '../../crates/engram-dashboard-protocol/bindings/UsageVendorState'
import { agentClient } from '../api/clientFactory'
import type { AgentBackendKind } from '../api/types'
import { retryAsync } from '../util/retryInvoke'

/**
 * One company's stored value. The key is one company — the snapshot's `account_key` is always `"default"` for now so it is not told apart (if accounts
 * grow, the revision is per company · account field so the key must be widened too).
 */
export interface UsageVendorEntry {
  snapshot: UsageLimitSnapshot
  /** `performance.now()` (ms) at the moment of acceptance. The snapshot's relative time fields (`age_secs` etc.) are based on this moment. */
  receivedAt: number
  /**
   * The highest revision accepted on the current socket. `null` = forgotten (a larger `socketEpoch` was received) — the next one sheet is accepted regardless of
   * revision. Even when forgotten, `snapshot` is left as is.
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
   * Pull the shell cache and `merge` it — bounded retry, and a final failure leaves the held value as is (does not throw).
   * ★Order stands only after broadcast reception is up★ — eventBus calls it once right after connecting, and the slot calls it on mount.
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
      // ADR-0195: the socket marker is not rewound inside the shell process, so larger means later socket order. A smaller one belongs to a pushed-out socket.
      //   ★The place that forgets revision is this one only★ — it is not forgotten on a connection state transition (leaving `connected`): that transition
      //   comes by another path and may arrive later than the new socket's payload, and if the floor is erased then, a late old copy of the same socket would be
      //   accepted and the screen rewinds. 0 = the marker is unknown — do not change the held marker.
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
        onRetry: (err, attempt) => console.warn(`[usageStore] getUsageSnapshot 재시도 #${attempt}:`, err),
      })
      for (const snapshot of snapshots) get().merge(snapshot, socketEpoch)
    } catch (err) {
      console.warn('[usageStore] getUsageSnapshot 최종 실패 — 들고 있는 값 유지:', err)
    }
  },

  refresh: async vendor => {
    set(state => ({ pending: addPending(state.pending, vendor, 1) }))
    try {
      await agentClient.refreshUsageLimits(vendor)
    } catch (err) {
      // On a disconnect ProtocolClient refuses all pending requests — that refusal is the path that releases the wait here.
      console.warn(`[usageStore] refreshUsageLimits(${vendor}) 실패:`, err)
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
 * Refusal that blocks ⟳ = the visible `Rejected` state. The deactivation of the two ⟳ (small display · per-company in the popup) and the target
 * selection of `usageSlot.refresh` read this one. A refusal while a value has come and shows as `Ready` is not caught here — then ⟳ is answered by the daemon
 * from cache without a query (TRD §3 #88).
 */
export function blocksRefresh(state: UsageVendorState | undefined): boolean {
  return state?.kind === 'Rejected'
}

/**
 * Among the turned-on companies, those that can send ⟳ — those not caught by [`blocksRefresh`]. It also includes a company that has not yet received a value (⟳ is the way to call for the first
 * value). `usageSlot.refresh` looks back at the store when pressed and picks targets with this.
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
