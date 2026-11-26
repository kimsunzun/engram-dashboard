import { useEffect } from 'react'
import { HashRouter, Routes, Route } from 'react-router-dom'
import { installUiSettings } from './theme/uiSettings'
import AppLayout from './components/layout/AppLayout'
import PopoutPage from './pages/PopoutPage'
import { initEventBus } from './store/eventBus'
import { agentClient, bootstrapDaemonIfNeeded } from './api/clientFactory'
import { retryAsync } from './util/retryInvoke'
import { useAgentStore } from './store/agentStore'
// ADR-0055/0064: a single import of the command + slot menu contribution registration manifest — at boot every register(...)·
//   registerSlotMenu(...) runs and fills the registry and the slot menu contributions (scattered imports unified, ADR-0064 §4).
import './commands/contributions'
import { installKeybindings } from './commands/keybindings'
// ADR-0155: tells the shell about this window's commands and passes the envelopes the shell sends down into the same registry (TRD §6 Step 4).
import { installViewCommandBridge } from './commands/viewCommandBridge'
// ADR-0053: applies the same scrollbar visibility rule as the seam to native scrollers outside the seam (ScrollArea) (= the xterm viewport).
//   It is style wiring, so it is installed once at the app root, not in the slot components (this wiring comes up in every tab window).
import { installNativeScrollActivity } from './components/ui/nativeScrollActivity'
import { RootErrorBoundary } from './components/ui/RootErrorBoundary'
import { useLanguage } from './i18n/react'

function bootPull<T>(name: string, fetch: () => Promise<T>, apply: (value: T) => void): void {
  retryAsync(fetch, {
    onRetry: (err, attempt) => {
      console.warn(`[App] retrying ${name} #${attempt}:`, err)
    },
  })
    .then(apply)
    .catch(err => console.warn(`[App] ${name} finally failed:`, err))
}

function App() {
  // ADR-0373: a language change re-renders the whole tree from here (no remount — component state stays). A `memo` boundary
  //   below that renders `t()` text reads the language itself.
  useLanguage()

  // The theme is decided by the shell — the window theme (`window.setTheme`) ?? the shell setting `theme.default` (ADR-0265). Until the boot query
  // comes back, the theme the shell stamped on the window when creating it shows (`theme/bootTheme.ts`).
  // ★Only the value is swapped — no remount (ADR-0149)★.
  useEffect(() => installUiSettings(), [])

  // ADR-0055: the returned disposer keeps listeners from piling up on unmount/HMR.
  useEffect(() => installKeybindings(), [])

  // ★The `contributions` import above has already run and the registry is filled★ (module side effects are evaluated before
  //   this component) — so the list reported here is not empty. The same disposer discipline.
  useEffect(() => installViewCommandBridge(), [])

  // ADR-0053: see the import comment above. The same disposer discipline (prevents HMR duplicates).
  useEffect(() => installNativeScrollActivity(), [])

  useEffect(() => {
    // ADR-0021 §1: one explicit ensure at boot — in daemon mode it launches the daemon (an explicit start, not a side effect of a
    // command). The command path (ensureReady) is attach-only, so without this the daemon does not come up at boot.
    // Only after start fills the cache (host:port) can the later getAgents/subscription attaches connect, so it is awaited first.
    void (async () => {
      await bootstrapDaemonIfNeeded()
      void initEventBus()
      // ADR-0102: the boot pull is wrapped in retries — the same pattern as bootstrapDaemonIfNeeded.
      // Right after the daemon connects, IPC can be momentarily not ready, so a one-shot failure would turn straight into a blank.
      // ★Profiles are retried too★: the tree stays empty until it receives the first list (AgentList), so a single failure means a stuck empty area.
      bootPull('getAgents', () => agentClient.getAgents(), agents => useAgentStore.getState().setAgents(agents))
      // ADR-0018: needed for the tree to draw reserved (empty-shell) nodes.
      bootPull('listProfiles', () => agentClient.listProfiles(), p => useAgentStore.getState().setProfiles(p))
    })()
  }, [])

  // The boundary sits inside this component and outside the router — wrapping this component would make 「Redraw」 remount it and rerun the boot effect
  //   above (daemon ensure · list · profile queries). Only the routed screen needs redrawing.
  return (
    <RootErrorBoundary>
      <HashRouter>
        <div style={{ height: '100vh' }}>
          <Routes>
            <Route path="/" element={<AppLayout />} />
            {/* Runtime windows (popout · new empty window) — windows with tabs at ?window=<label> (ADR-0057). */}
            <Route path="/popup" element={<PopoutPage />} />
          </Routes>
        </div>
      </HashRouter>
    </RootErrorBoundary>
  )
}

export default App
