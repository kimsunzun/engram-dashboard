// DaemonControl — the frontend daemon start/status surface (ADR-0021 §5).
//
// Boot bootstrap and window.__ENGRAM_DAEMON__ (LLM · cdp.mjs) go through this surface (§5 LLM-first control).
//
// ADR-0350: no stop here — the shell owns the one stop path (tray Stop / Quit; the LLM `daemon.stop` command comes
// with U5b-2). A frontend stop that calls client.disconnect() or the `daemon_stop` pid kill races that path.
//
// ★ensure/reconnect separation (ADR-0021 invariant)★: only start triggers a spawn. The wsTransport reconnect
// loop never calls this surface (attach-only). If the daemon is stopped, reconnect cannot attach and the state stays 'down'
// — the user must start again to bring it back.
//
// daemon-only (ADR-0029): clientFactory always exposes DaemonDaemonControl.

import { invoke } from '@tauri-apps/api/core'

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

  // ★The single point where a failure reason reaches the screen (ADR-0134 decision 4)★: an explicit start goes through here from boot bootstrap and
  //   window.__ENGRAM_DAEMON__ alike, so the reason is recorded in this one place too. If report calls are
  //   scattered across callers, some path fails silently (the retry after the user fixed the folder and pressed again
  //   was such a path).
  async start(opts?: { console?: boolean; timeoutMs?: number }): Promise<DaemonInfo> {
    try {
      // 1) Spawn the daemon (if absent) — daemon_start Tauri command (WMI/CREATE_NO_WINDOW). console is a
      //    spawn-time parameter (a runtime toggle is follow-up — set_daemon_console not implemented, M-2). Attach if already alive.
      const info = await invoke<DaemonInfo>('daemon_start', {
        console: opts?.console ?? false,
        timeoutMs: opts?.timeoutMs ?? null,
      })
      // 2) ★Explicit connect★ (ADR-0021 §1): client.connect → transport.start (discover allowed + cache filled +
      //    closedByUser/attempt reset). This is the only client path that can trigger a spawn. The command path
      //    (ensureReady) is attach-only and cannot wake the daemon → only boot/explicit start brings the daemon up (B-1).
      await this.client.connect()
      this.client.reportConnectionError(null)
      return info
    } catch (err) {
      // Carry the cause Rust attached (e.g. cannot write to the data folder) onto the connection state surface. The throw
      // continues as is so the caller's retry and logging do not change.
      this.client.reportConnectionError(describeError(err))
      throw err
    }
  }

  status(): Promise<DaemonStatus> {
    return invoke<DaemonStatus>('daemon_status')
  }
}
