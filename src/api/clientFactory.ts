// AgentClient factory — daemon-only single path (ADR-0029: embedded surface removed).
//
// T7c: WsTransport → TauriTransport swap. However many windows there are, the daemon sees one Rust DaemonClient connection
// (ADR-0036 goal). The frontend receives broadcasts through Rust app.emit and sends commands through invoke.
//
// §5 LLM-first control: expose the control surface (AgentClient · DaemonControl) on window so that cdp.mjs eval /
// a (future) backend-side LLM can call the same entry points as a human click (temporary path, until the real command bus).

import type { AgentClient } from './agentClient'
import { type DaemonControl, DaemonDaemonControl } from './daemonControl'
import { ProtocolClient } from './protocolClient'
import { TauriTransport } from './tauriTransport'
import { retryAsync } from '../util/retryInvoke'

let instance: AgentClient | null = null
let daemonControlInstance: DaemonControl | null = null

// ★T7c: TauriTransport needs an async init (listener registration) — guard the singleton initialization with async.
let initPromise: Promise<void> | null = null

/** The single AgentClient instance. Components · stores · the (future) LLM all go through it. */
export function getAgentClient(): AgentClient {
  if (!instance) {
    const transport = new TauriTransport()
    instance = new ProtocolClient(transport)
    daemonControlInstance = new DaemonDaemonControl(instance)
    // §5 LLM-first control: expose the control surface on window — so that cdp.mjs eval / a (future) backend-side LLM
    // can call the same entry points as a human click.
    ;(window as unknown as { __ENGRAM_AGENT__?: AgentClient }).__ENGRAM_AGENT__ = instance
    // Expose daemon control (start/status) the same way — the LLM and cdp drive the same handle as boot.
    ;(window as unknown as { __ENGRAM_DAEMON__?: DaemonControl }).__ENGRAM_DAEMON__ =
      daemonControlInstance
    // ★T7c: register the TauriTransport Tauri event listeners (async). Events arriving before the listeners are registered
    // may be lost, but bootstrapDaemonIfNeeded guarantees connect right after boot, so events after that
    // are safe. Until registration completes the connection state is 'down', so ProtocolClient does not send
    // commands (ensureReady rejects).
    //
    // ★Retry safety★: on partial failure, registerListeners() inside init() cleans up what it registered right away and throws
    // (leaving with unlisten emptied). Then on the next attempt registerListeners() re-registers from scratch, so there is
    // no double-listener risk. This extends the ADR-0102 pattern to the init path.
    initPromise = retryAsync(() => transport.init(), {
      onRetry: (err, attempt) => {
        console.warn(`[clientFactory] TauriTransport 리스너 등록 재시도 #${attempt}:`, err)
      },
    }).catch((e: unknown) => {
      console.error('[clientFactory] TauriTransport 리스너 등록 최종 실패 — 이벤트 수신 불가:', e)
    })
  }
  return instance
}

/** Wait for TauriTransport initialization (listener registration) to finish. Recommended to call before bootstrapDaemonIfNeeded at boot. */
export async function waitForTransportInit(): Promise<void> {
  getAgentClient() // Guarantees singleton creation.
  if (initPromise) await initPromise
}

export function getDaemonControl(): DaemonControl {
  if (!daemonControlInstance) getAgentClient() // Ensures both are initialized together.
  return daemonControlInstance!
}

/**
 * **Explicit ensure once** at boot (ADR-0021 §1: spawn only at an explicit point). daemonControl.start() brings the daemon
 * up (equivalent to tmux attach starting the server). The command path (ensureReady) is attach-only and cannot
 * wake the daemon, so without this boot start the daemon would not come up at boot.
 * Idempotent — start resolves immediately if already connected. Failure (daemon spawn impossible) is swallowed so it does not block boot.
 */
export async function bootstrapDaemonIfNeeded(): Promise<void> {
  // ADR-0102: self-heal a transient failure caused by a boot race with retries (guards against an instant rejection before initial IPC is ready).
  // The final failure is surfaced with console.error — if this function threw, every later getAgents/subscription would be blocked, so
  // it is swallowed to let boot continue, and the log escalates from warn (transient failure) → error (manual recovery needed).
  try {
    await retryAsync(() => getDaemonControl().start(), {
      onRetry: (err, attempt) => {
        console.warn(`[clientFactory] daemon start 재시도 #${attempt}:`, err)
      },
    })
  } catch (err) {
    // ★The reason is recorded not here but inside DaemonControl.start★: every explicit start path goes through there, so
    //   it is recorded in one place. Recording it here too would only make the two places diverge.
    console.error('[clientFactory] 부팅 daemon start 최종 실패 — 수동 daemon_start 필요:', err)
  }
}

/** Module-load-time singleton (most components import this). */
export const agentClient = getAgentClient()
