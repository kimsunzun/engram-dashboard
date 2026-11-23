// App-wide agent event wiring, consuming agentClient's event subscriptions (no direct Tauri listening).

import { agentClient } from '../api/clientFactory'
import { list as cmdList, run as cmdRun } from '../commands/registry'
import { useAgentStore } from './agentStore'
import { usePeerStore } from './peerStore'
import { useUsageStore } from './usageStore'
import { initMainWindowFromBackend, subscribeViewEvents } from './viewStore'

let unlistenFns: (() => void)[] = []
// Prevents the StrictMode double-mount race
let initPromise: Promise<void> | null = null

/**
 * Profile list refresh (ADR-0018). Live updates come from the ProfileListUpdated broadcast; this pull runs at boot, on reconnect resync and right after create/delete/activate (spawnProfile) in case a broadcast is lost. Only the reserved list is fetched (the switch to running is agent-list-updated's).
 */
export async function refreshProfiles(): Promise<void> {
  try {
    const profiles = await agentClient.listProfiles()
    useAgentStore.getState().setProfiles(profiles)
  } catch (err) {
    console.warn('[eventBus] refreshProfiles failed:', err)
  }
}

/** Preset list refresh (ADR-0061, mirror of refreshProfiles): pulled at boot + reconnect resync; post-create/delete updates come from the PresetListUpdated broadcast. */
export async function refreshPresets(): Promise<void> {
  try {
    const presets = await agentClient.listPresets()
    useAgentStore.getState().setPresets(presets)
  } catch (err) {
    console.warn('[eventBus] refreshPresets failed:', err)
  }
}

/**
 * Resync right after a reconnect (Q2): only on a connected *re*-transition (first connection excluded — `lastState` guard in initEventBus). Pulls the authoritative lists and rewrites the store.
 * ★This getAgents is **the main path** of output recovery (ADR-0046 amend)★: a disconnected view settles into detached and re-attaches only when its agent shows up in that roster (protocolClient.observeRoster). Without the roster pull, slots never attach after a reconnect — do not remove it as "just a list redraw".
 * ★Failures are swallowed★: no retry ladder; a manual reconnect reruns this path.
 */
async function resyncAfterReconnect(): Promise<void> {
  // ★Usage is pulled again too★: the same snapshot twice is harmless (store merge rule filters it), and it fills the gap if the boot pull exhausted its retries or broadcasts were missed while disconnected. Runs after the usage broadcast hookup, so "hook up first, pull later" holds.
  void useUsageStore.getState().pull()
  try {
    const agents = await agentClient.getAgents()
    useAgentStore.getState().setAgents(agents)
  } catch (err) {
    console.warn('[eventBus] resync getAgents failed:', err)
  }
  await refreshProfiles()
  await refreshPresets()
}

export function initEventBus(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      // §5: command registry control surface (ADR-0055) — the LLM (cdp eval) calls the same single entry point as human clicks and global keydown.
      // ★Do not make new global handles★: layout, tabs, windows and render modes are called through tab.*·window.*·slot.*·layout.setSlotContent registered here; side-by-side handles give one action two surfaces that diverge.
      // ★It is not a single surface yet★: global handles outside the bus remain. Canonical policy = CLAUDE.md 「LLM-우선 제어」 · gap = `docs/tracking.md` T-77; live assignments: `rg "\)\.__ENGRAM_|\)\.__engram" src/ -g '!*.test.*'` (the `).__NAME` anchor avoids type/comment matches). Do not list them here (goes stale) and do not judge them by their definition comments — T-77 decides which are side doors. // hangul-ok
      // ★The registry is not a state authority★ (ADR-0035): handlers route to existing store actions/invoke. run passes the handler's return (sometimes a Promise) through, so cdp eval can await it.
      //   window.__engramCmd.run('slot.empty', { viewId, slotId })  // an unknown id throws
      // ★Exposing every command on window is intentional (WONTFIX)★: required by CLAUDE.md 「LLM-우선 제어」 (every feature LLM-controllable) / ADR-0055; an allowlist was rejected as it collides with that. This is a control contract, not a security hole. // hangul-ok
      ;(globalThis as Record<string, unknown>).__engramCmd = {
        list: cmdList,
        run: cmdRun,
      }

      // On HMR re-evaluation, release the existing subscriptions first
      if (unlistenFns.length > 0) {
        unlistenFns.forEach(fn => fn())
        unlistenFns = []
      }

      // Layout emit subscription (layout:updated / window:tabs-updated) — src-tauri authority, so @tauri-apps/api listen is used directly (viewStore.subscribeViewEvents).
      // ★Push dispose immediately, without await★ (leak guard): put it in unlistenFns first so cleanup (HMR dispose/re-init) running while the ready await below is pending includes it and a late registration does not leak.
      const viewSub = subscribeViewEvents()
      unlistenFns.push(viewSub.dispose)

      // ★Register the HMR dispose callback *before* the ready await★ (last link of the leak guard): registered after `await viewSub.ready`, an HMR during the wait would find no callback and never call viewSub.dispose → leaked layout listener.
      // unlistenFns/initPromise are module-scope lets, so the closure reads the latest values.
      if (import.meta.hot) {
        import.meta.hot.dispose(() => {
          unlistenFns.forEach(fn => fn())
          unlistenFns = []
          initPromise = null
          // ★__engramCmd is not removed here★: the reinstall happens in initEventBus, and App awaits bootstrapDaemonIfNeeded first (App.tsx), so removing it would make cdp calls in between hit undefined for as long as the retry delay. A dev-only HMR quirk; a full refresh clears it.
        })
      }

      // ★Await registration completion★ (F-listen): listen() is async, so an init pull result or backend emit arriving before registration finishes would be lost. initMainWindowFromBackend runs only after ready; ready ends normally even if dispose comes first or registration fails (no hang).
      //
      // ★Isolate a layout subscription failure from the agentClient subscriptions★: if ready rejects, only the layout (ADR-0035 authority) failed; agentClient (ADR-0011 authority) must stay unaffected — without the catch the registrations below would all be skipped. The catch's viewSub.dispose() cleans up partial registrations (idempotent, also in unlistenFns).
      try {
        await viewSub.ready
      } catch (err) {
        console.warn('[eventBus] layout subscription (subscribeViewEvents) failed — agentClient subscriptions continue:', err)
        viewSub.dispose()
      }

      // Boot init: the backend's default View exists before boot and no emit announces it (emits come only after changes), so list_tabs("main")/get_view pull the main window's tabs + active layout. ★Called after the subscriptions are set up★: emits during init are not missed, and the window/cache version guard lets a newer emit overwrite the pull. (A popup window pulls its own tabs per WindowLayout on mount.)
      // ADR-0102: a final failure (bounded retries exhausted) is surfaced as an error, not a quiet warn — main has no event recovery path, so a silent failure would leave a stuck loading of unknown cause. (The visible UI error is rendered by the main WindowLayout.)
      void initMainWindowFromBackend().catch(err => {
        console.error('[eventBus] initMainWindowFromBackend finally failed (retries exhausted):', err)
      })

      unlistenFns.push(
        agentClient.onAgentListUpdated(agents => {
          useAgentStore.getState().setAgents(agents)
        }),
      )

      unlistenFns.push(
        agentClient.onStatusChanged((id, status) => {
          useAgentStore.getState().onStatusChanged(id, status)
        }),
      )

      unlistenFns.push(
        agentClient.onRestoreResult(report => {
          console.info('[restore]', report.agent_id, report.outcome.type, report.outcome)
        }),
      )

      unlistenFns.push(
        agentClient.onProfileListUpdated(profiles => {
          useAgentStore.getState().setProfiles(profiles)
        }),
      )

      unlistenFns.push(
        agentClient.onPresetListUpdated(presets => {
          useAgentStore.getState().setPresets(presets)
        }),
      )

      // Usage (TRD S21 usage-limit-slot §1-8): ★hook up the broadcast first, pull later★ — an inversion is resolved by the store's revision. ★Do not remove this pull★: this function runs after the daemon ensure (App.tsx), so a usage slot can mount earlier and its mount pull misses broadcasts before the hookup.
      unlistenFns.push(
        agentClient.onUsageLimitsUpdated((snapshot, socketEpoch) => {
          useUsageStore.getState().merge(snapshot, socketEpoch)
        }),
      )
      void useUsageStore.getState().pull()

      // ADR-0394: the PC list slot's peer state — the slot reads `peer.list` itself on mount, so no pull here.
      unlistenFns.push(
        agentClient.onPeerStateUpdated(state => {
          usePeerStore.getState().receive(state)
        }),
      )

      // Reconnect resync (Q2) + re-attach trigger for output views (see resyncAfterReconnect): without it the agent tree and profile list go stale on spawn/kill/profile broadcasts missed while disconnected. ★The first connected is skipped★ to avoid duplicating the App.tsx boot load. Same lastState guard as ProtocolClient (prev!=='connected' && cur==='connected'); the initial value is the current state so a first connected notification is not a re-transition.
      let lastConn = agentClient.connectionState
      unlistenFns.push(
        agentClient.onConnectionStateChange(state => {
          const prev = lastConn
          lastConn = state
          if (state === 'connected' && prev !== 'connected') {
            void resyncAfterReconnect()
          }
        }),
      )
    } catch (err) {
      console.error('[eventBus] init failed:', err)
      initPromise = null // prevents getting stuck — allows a retry on the next call
      throw err
    }
  })()

  return initPromise
}
