// DaemonControl — 데몬 lifecycle 제어 표면 (ADR-0021 §5: LLM·UI·트레이 동일 핸들).
//
// daemon_start(명시 ensure=spawn 허용) / daemon_stop(graceful 우선, fallback kill) / daemon_status.
// 사람 UI 클릭·트레이·(미래) 백엔드측 LLM·cdp.mjs 가 모두 이 표면을 통한다(§5 LLM-우선 제어).
//
// ★ensure/reconnect 분리(ADR-0021 불변식)★: start 만 spawn 을 유발한다. wsTransport 의 재연결
// 루프는 이 표면을 절대 호출하지 않는다(attach-only). 데몬을 stop 하면 재연결이 못 붙어 'down'
// 유지 — 사용자가 다시 start 해야 살아난다.
//
// daemon-only(ADR-0029): clientFactory 가 항상 DaemonDaemonControl 을 노출한다.

import { invoke } from '@tauri-apps/api/core'

import type { AgentClient } from './agentClient'

/** daemon_status Tauri command 반환(DaemonStatusDto 미러). */
export interface DaemonStatus {
  alive: boolean
  pid: number | null
  port: number | null
}

/** discover_daemon/daemon_start 반환(DaemonInfoDto 미러). token 은 노출하나 로그 금지. */
export interface DaemonInfo {
  pid: number
  host: string
  port: number
  token: string
  protocol_version: number
}

/** 데몬 lifecycle 제어(start/stop/status). */
export interface DaemonControl {
  /** 명시 시작(ensure). 이미 살아있으면 attach, 없으면 spawn. console=true 면 콘솔 창과 함께(디버그). */
  start(opts?: { console?: boolean; timeoutMs?: number }): Promise<DaemonInfo>
  /** 종료. 연결이 살아있으면 StopDaemon(graceful, 자식 정리 후 자진 종료) 먼저, 그 뒤 fallback kill. */
  stop(opts?: { force?: boolean }): Promise<void>
  /** 상태 조회(alive/pid/port). */
  status(): Promise<DaemonStatus>
}

/** 실제 구현 — Tauri command + StopDaemon graceful. */
export class DaemonDaemonControl implements DaemonControl {
  private readonly client: AgentClient

  constructor(client: AgentClient) {
    this.client = client
  }

  async start(opts?: { console?: boolean; timeoutMs?: number }): Promise<DaemonInfo> {
    // 1) 데몬 spawn(없으면) — daemon_start Tauri command(WMI/CREATE_NO_WINDOW). console 은
    //    spawn-time 파라미터(런타임 토글은 후속 — set_daemon_console 미구현, M-2). 이미 살아있으면 attach.
    const info = await invoke<DaemonInfo>('daemon_start', {
      console: opts?.console ?? false,
      timeoutMs: opts?.timeoutMs ?? null,
    })
    // 2) ★명시 연결★(ADR-0021 §1): client.connect → transport.start(discover 허용 + 캐시 채움 +
    //    closedByUser/attempt 리셋). 이게 spawn 을 유발할 수 있는 유일한 클라 경로다. 명령 경로
    //    (ensureReady)는 attach-only 라 데몬을 못 깨운다 → 부팅/명시 start 만 데몬을 띄운다(B-1).
    await this.client.connect()
    return info
  }

  async stop(opts?: { force?: boolean }): Promise<void> {
    // 1) graceful 우선 — 연결이 살아있으면 StopDaemon AgentCommand 로 데몬이 자식 PTY 를 정리하고
    //    스스로 내려가게 한다. 연결이 없으면(이미 끊김) 이 단계는 건너뛴다.
    let gracefulOk = false
    if (this.client.connectionState === 'connected') {
      try {
        await this.client.stopDaemon(opts?.force ?? false)
        gracefulOk = true
      } catch {
        // graceful 실패(active agents 거부 등) — 아래 fallback kill 로 강제 종료(force 의도).
      }
    }
    // 2) note3(재연결 노이즈 제거): graceful 직후 client.disconnect() 로 closedByUser=true →
    //    즉시 'down' 정착. 안 하면 데몬이 Ack 후 소켓을 닫을 때 attach-only 재연결이 5회 헛시도한다.
    //    ★respawn 금지★: disconnect 는 transport.close 만(spawn 안 함). 복구는 명시 start 로만.
    this.client.disconnect()
    // 3) M-1(graceful/taskkill race 완화): graceful 이 Ack 됐으면 데몬이 자진 종료 중 —
    //    바로 taskkill 하지 말고 daemon_status 로 still-alive 확인 후 살아있을 때만 fallback.
    //    graceful 이 없었거나(연결 없음) 실패했으면 곧장 fallback kill 로 강제 종료.
    if (gracefulOk) {
      let stillAlive = true
      try {
        const s = await invoke<DaemonStatus>('daemon_status')
        stillAlive = s.alive
      } catch {
        // status 조회 실패 — 살아있다고 보수적으로 가정(fallback 진행).
      }
      if (!stillAlive) return // graceful 이 이미 내렸다 — taskkill 불필요(race 회피).
    }
    // fallback kill — daemon.json 의 pid 를 taskkill. graceful 이 먹었으면 이미 죽어 no-op.
    await invoke<number | null>('daemon_stop')
  }

  status(): Promise<DaemonStatus> {
    return invoke<DaemonStatus>('daemon_status')
  }
}
