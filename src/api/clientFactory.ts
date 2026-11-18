// AgentClient factory — daemon-only single path (ADR-0029). However many windows there are, the daemon sees one Rust DaemonClient connection (ADR-0036); the frontend receives broadcasts via Rust app.emit and sends commands via invoke.
// §5 LLM-first control: the control surface (AgentClient · DaemonControl) is exposed on window so cdp.mjs eval / a (future) backend-side LLM calls the same entry points as a human click (temporary path, until the real command bus).

import type { AgentClient } from './agentClient'
import { type DaemonControl, DaemonDaemonControl } from './daemonControl'
import { ProtocolClient } from './protocolClient'
import { TauriTransport } from './tauriTransport'
import { retryAsync } from '../util/retryInvoke'

let instance: AgentClient | null = null
let daemonControlInstance: DaemonControl | null = null

// ★TauriTransport needs an async init (listener registration) — the singleton initialization is guarded with async.
let initPromise: Promise<void> | null = null

/** The single AgentClient instance. Components · stores · the (future) LLM all go through it. */
export function getAgentClient(): AgentClient {
  if (!instance) {
    const transport = new TauriTransport()
    instance = new ProtocolClient(transport)
    daemonControlInstance = new DaemonDaemonControl(instance)
    ;(window as unknown as { __ENGRAM_AGENT__?: AgentClient }).__ENGRAM_AGENT__ = instance
    // Expose daemon control (start/status) the same way — the LLM and cdp drive the same handle as boot.
    ;(window as unknown as { __ENGRAM_DAEMON__?: DaemonControl }).__ENGRAM_DAEMON__ =
      daemonControlInstance
    // ★Register the TauriTransport Tauri event listeners (async)★: events arriving before registration may be lost, but bootstrapDaemonIfNeeded guarantees connect right after boot. Until registration completes the connection state is 'down', so ProtocolClient sends nothing (ensureReady rejects).
    // ★Retry safety★: on partial failure registerListeners() inside init() cleans up what it registered and throws (unlisten emptied), so the next attempt re-registers from scratch — no double-listener risk (the ADR-0102 pattern on the init path).
    initPromise = retryAsync(() => transport.init(), {
      onRetry: (err, attempt) => {
        console.warn(`[clientFactory] retrying TauriTransport listener registration #${attempt}:`, err)
      },
    }).catch((e: unknown) => {
      console.error('[clientFactory] TauriTransport listener registration finally failed — events cannot be received:', e)
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
 * **Explicit ensure once** at boot (ADR-0021 §1: spawn only at an explicit point). daemonControl.start() brings the daemon up; the command path (ensureReady) is attach-only and cannot wake it, so without this boot start the daemon would not come up. Idempotent. Failure (spawn impossible) is swallowed so it does not block boot.
 */
export async function bootstrapDaemonIfNeeded(): Promise<void> {
  // ADR-0102: self-heals a transient boot-race failure with retries. The final failure is logged with console.error and swallowed — a throw would block every later getAgents/subscription — escalating warn (transient) → error (manual recovery needed).
  try {
    await retryAsync(() => getDaemonControl().start(), {
      onRetry: (err, attempt) => {
        console.warn(`[clientFactory] retrying daemon start #${attempt}:`, err)
      },
    })
  } catch (err) {
    // ★The reason is recorded inside DaemonControl.start, not here★: every explicit start path goes through there; recording it here too would make the two diverge.
    console.error('[clientFactory] boot daemon start finally failed — manual daemon_start required:', err)
  }
}

/** Module-load-time singleton (most components import this). */
export const agentClient = getAgentClient()
