// AgentClient 팩토리 — daemon-only 단일 경로 (ADR-0029: embedded 표면 제거).
//
// T7c: WsTransport → TauriTransport 교체. 창이 몇 개든 데몬엔 Rust DaemonClient 연결 1개
// (ADR-0036 목표). 프론트는 Rust app.emit 으로 broadcast를 수신하고, invoke 로 명령을 전달한다.
//
// §5 LLM-우선 제어: 제어 표면(AgentClient·DaemonControl)을 window 에 노출해 cdp.mjs eval /
// (미래) 백엔드측 LLM 이 사람 클릭과 동일 진입점을 호출할 수 있게 한다(임시 경로, 정식 command 버스 전까지).

import type { AgentClient } from './agentClient'
import { type DaemonControl, DaemonDaemonControl } from './daemonControl'
import { ProtocolClient } from './protocolClient'
import { TauriTransport } from './tauriTransport'

let instance: AgentClient | null = null
let daemonControlInstance: DaemonControl | null = null

// ★T7c: TauriTransport 는 async init(리스너 등록)이 필요 — 싱글톤 초기화를 async 로 보호한다.
let initPromise: Promise<void> | null = null

/** 단일 AgentClient 인스턴스. 컴포넌트·스토어·(미래)LLM 이 모두 이걸 통한다. */
export function getAgentClient(): AgentClient {
  if (!instance) {
    // daemon-only: T7c — TauriTransport(Rust DaemonClient 연결 단일화) 위의 ProtocolClient.
    const transport = new TauriTransport()
    instance = new ProtocolClient(transport)
    // ADR-0021 §5: 데몬 lifecycle 제어 표면(start/stop/status).
    daemonControlInstance = new DaemonDaemonControl(instance)
    // §5 LLM-우선 제어: 제어 표면을 window 에 노출 — cdp.mjs eval / (미래) 백엔드측 LLM 이
    // 사람 클릭과 동일 진입점을 호출할 수 있게 한다.
    ;(window as unknown as { __ENGRAM_AGENT__?: AgentClient }).__ENGRAM_AGENT__ = instance
    // 데몬 제어(start/stop/status)도 동일하게 노출 — 트레이(#2)·LLM·cdp 가 같은 핸들을 흔든다.
    ;(window as unknown as { __ENGRAM_DAEMON__?: DaemonControl }).__ENGRAM_DAEMON__ =
      daemonControlInstance
    // ★T7c: TauriTransport Tauri 이벤트 리스너 등록(async). 리스너가 등록되기 전 도착하는 이벤트는
    // 유실될 수 있으나, 부팅 직후 bootstrapDaemonIfNeeded 가 connect 를 보장하므로 그 이후 이벤트는
    // 안전하다. 리스너 등록 완료 전엔 connection 상태가 'down' 이므로 ProtocolClient 가 명령을 보내지
    // 않는다(ensureReady reject).
    initPromise = transport.init().catch((e: unknown) => {
      console.warn('[clientFactory] TauriTransport 리스너 등록 실패:', e)
    })
  }
  return instance
}

/** TauriTransport 초기화(리스너 등록) 완료 대기. 부팅 시 bootstrapDaemonIfNeeded 전에 호출 권장. */
export async function waitForTransportInit(): Promise<void> {
  getAgentClient() // 싱글톤 생성 보장.
  if (initPromise) await initPromise
}

/** 단일 DaemonControl 인스턴스. getAgentClient 와 동일 시점에 구성된다. */
export function getDaemonControl(): DaemonControl {
  if (!daemonControlInstance) getAgentClient() // 동시 초기화 보장.
  return daemonControlInstance!
}

/**
 * 부팅 시 **명시 ensure 1회**(ADR-0021 §1: spawn=명시 시점만). daemonControl.start() 로 데몬을
 * 띄운다(tmux attach 가 서버를 띄우는 것과 동치). 명령 경로(ensureReady)는 attach-only 라 데몬을
 * 못 깨우므로, 이 부팅 start 가 없으면 부팅 시 데몬이 안 뜬다.
 * 멱등 — start 는 이미 connected 면 즉시 resolve. 실패(데몬 spawn 불가)는 삼켜 부팅을 막지 않는다.
 */
export async function bootstrapDaemonIfNeeded(): Promise<void> {
  try {
    await getDaemonControl().start()
  } catch (err) {
    console.warn('[clientFactory] 부팅 daemon start 실패(수동 daemon_start 필요):', err)
  }
}

/** 모듈 로드 시점 싱글톤(대부분 컴포넌트는 이걸 import). */
export const agentClient = getAgentClient()
