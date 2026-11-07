// DaemonControl — the daemon lifecycle control surface (ADR-0021 §5: the same handle for LLM · UI · tray).
//
// Human UI clicks · the tray · the (future) backend-side LLM · cdp.mjs all go through this surface (§5 LLM-first control).
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

/** discover_daemon/daemon_start return (mirror of DaemonInfoDto). The token is exposed but must not be logged. */
export interface DaemonInfo {
  pid: number
  host: string
  port: number
  token: string
  protocol_version: number
}

export interface DaemonControl {
  /** Explicit start (ensure). Attach if already alive, spawn if not. With console=true it comes with a console window (debug). */
  start(opts?: { console?: boolean; timeoutMs?: number }): Promise<DaemonInfo>
  /** Stop. If the connection is alive, StopDaemon (graceful, exits on its own after cleaning up children) first, then a fallback kill. */
  stop(opts?: { force?: boolean }): Promise<void>
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

  // ★The single point where a failure reason reaches the screen (ADR-0134 decision 4)★: an explicit start goes through here from boot bootstrap · the UI ·
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

  async stop(opts?: { force?: boolean }): Promise<void> {
    // 1) Graceful first — if the connection is alive, use a StopDaemon AgentCommand to have the daemon clean up its child PTYs and
    //    bring itself down.
    let gracefulOk = false
    if (this.client.connectionState === 'connected') {
      try {
        await this.client.stopDaemon(opts?.force ?? false)
        gracefulOk = true
      } catch {
        // Graceful failure (active agents rejection, etc.) — force termination with the fallback kill below (force intent).
      }
    }
    // 2) note3 (reconnect noise removed): right after graceful, client.disconnect() sets closedByUser=true →
    //    settles to 'down' immediately. Without it, when the daemon closes the socket after the Ack, attach-only reconnect makes 5 futile attempts.
    //    ★No respawn★: disconnect only does transport.close (no spawn). Recovery only through an explicit start.
    this.client.disconnect()
    // 3) M-1 (eases the graceful/taskkill race): if graceful was Acked, the daemon is exiting on its own —
    //    do not taskkill right away; check still-alive with daemon_status and fall back only if it is alive.
    //    If there was no graceful (no connection) or it failed, force termination goes straight to the fallback kill.
    if (gracefulOk) {
      let stillAlive = true
      try {
        const s = await invoke<DaemonStatus>('daemon_status')
        stillAlive = s.alive
      } catch {
        // Status query failed — conservatively assume alive (proceed to the fallback).
      }
      if (!stillAlive) return // Graceful already brought it down — no taskkill needed (avoids the race).
    }
    // Fallback kill — taskkill the pid in daemon.json. If graceful worked it is already dead and this is a no-op.
    await invoke<number | null>('daemon_stop')
  }

  status(): Promise<DaemonStatus> {
    return invoke<DaemonStatus>('daemon_status')
  }
}
