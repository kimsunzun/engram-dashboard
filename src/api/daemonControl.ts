// DaemonControl — the frontend daemon start/status surface (ADR-0021 §5). Boot bootstrap and window.__ENGRAM_DAEMON__ (LLM · cdp.mjs) go through it (§5 LLM-first control). Daemon-only (ADR-0029): clientFactory always exposes DaemonDaemonControl.
// ADR-0350: no stop method here — the shell owns the one stop path (tray Stop / Quit); the frontend only asks it through `shutdownDaemon` below (the LLM `daemon.stop`). A frontend stop calling client.disconnect() races that path.
// ★ensure/reconnect separation (ADR-0021 invariant)★: only start triggers a spawn; the wsTransport reconnect loop never calls this surface (attach-only). If the daemon is stopped, reconnect cannot attach and the state stays 'down' until the user starts again.

import { invoke } from '@tauri-apps/api/core'

import type { DaemonShutdownReply } from '../../src-tauri/bindings/DaemonShutdownReply'
import type { AgentClient } from './agentClient'

/** daemon_status Tauri command return (mirror of DaemonStatusDto). */
export interface DaemonStatus {
  alive: boolean
  pid: number | null
  port: number | null
}

/** discover_daemon/daemon_start result (mirrors DaemonInfoDto). Carries no daemon token — the WebView never receives it. */
export interface DaemonInfo {
  pid: number
  host: string
  port: number
  protocol_version: number
}

export interface DaemonControl {
  /** Explicit start (ensure). Attach if already alive, spawn if not. With console=true it comes with a console window (debug). */
  start(opts?: { console?: boolean; timeoutMs?: number }): Promise<DaemonInfo>
  status(): Promise<DaemonStatus>
}

/** An invoke rejection arrives as a string (Rust `Err(String)`) but may also be an Error instance — unfold both into a sentence. */
function describeError(err: unknown): string {
  if (typeof err === 'string') return err
  if (err instanceof Error) return err.message
  return String(err)
}

export class DaemonDaemonControl implements DaemonControl {
  private readonly client: AgentClient

  constructor(client: AgentClient) {
    this.client = client
  }

  // ★The single point where a start's failure cause reaches the screen (ADR-0134 decision 4)★: boot bootstrap, window.__ENGRAM_DAEMON__, the band's "Reconnect" and the LLM `connection.reconnect` all start here; scattered report calls would leave some path failing silently.
  async start(opts?: { console?: boolean; timeoutMs?: number }): Promise<DaemonInfo> {
    // TRD S21 trd-3-2-u5 R14: a new attempt drops the previous attempt's cause.
    this.client.reportConnectionError(null)
    try {
      // 1) Spawn the daemon if absent (daemon_start, WMI/CREATE_NO_WINDOW); attach if alive. console is a spawn-time parameter (runtime toggle not implemented — M-2).
      const info = await invoke<DaemonInfo>('daemon_start', {
        console: opts?.console ?? false,
        timeoutMs: opts?.timeoutMs ?? null,
      })
      // 2) ★Explicit connect★ (ADR-0021 §1): client.connect → transport.start (discover allowed, cache filled, closedByUser/attempt reset) — the only client path that can spawn. The command path (ensureReady) is attach-only and cannot wake the daemon, so only boot/explicit start brings it up (B-1).
      await this.client.connect()
      this.client.reportConnectionError(null)
      return info
    } catch (err) {
      // Carry Rust's cause (e.g. cannot write to the data folder) onto the connection state surface; the throw continues so the caller's retry and logging do not change.
      this.client.reportConnectionError(describeError(err))
      throw err
    }
  }

  status(): Promise<DaemonStatus> {
    return invoke<DaemonStatus>('daemon_status')
  }
}

/**
 * Stops the daemon through the shell's one stop path (`daemon_shutdown` — graceful, then an identity-checked kill; joins a stop already in flight). No confirm: the caller is the LLM `daemon.stop` (TRD S21 trd-3-2-u5 D5).
 * ★Never `client.disconnect()`★ — the shell's link stays open through a stop and reports the next band itself (ADR-0352); a frontend close would leave the window with no band. A stop sets no band and no cause (TRD S21 trd-3-2-u5 U6). A rejection = the command did not run (e.g. no shell client).
 */
export function shutdownDaemon(): Promise<DaemonShutdownReply> {
  return invoke<DaemonShutdownReply>('daemon_shutdown')
}
