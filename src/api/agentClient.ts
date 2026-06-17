// AgentClient — 프론트가 의존하는 단일 제어 표면(S12 phase 1b, daemon-design §3-a).
//
// 컴포넌트·스토어는 invoke/Channel/WS 를 직접 부르지 않고 이 인터페이스만 의존한다.
// 단일 구현 ProtocolClient(프로토콜 의미론 1벌) + transport 2개(InProc/Ws, ADR-0020 Stage 3~4a).
// transport(Tauri Channel / WS binary frame)와 base64/디코딩은 transport 내부에 숨긴다 —
// 인터페이스는 "디코드된 바이트 청크"만 노출(§3-a 손발/두뇌 분리: 프론트=순수 I/O).

import type { AgentInfo, AgentProfile, AgentStatus, RestoreReport } from './types'

/** 클라↔백엔드 연결 상태. Embedded 는 항상 connected. Daemon 만 reconnecting/down 발생. */
export type ConnectionState = 'connected' | 'reconnecting' | 'down'

/** 디코드된 출력 청크 — transport 무관(base64/binary frame 은 클라 내부에서 이미 풀림). */
export interface OutputChunk {
  /** core OutputCore 발급 seq(단조 증가). 클라가 재연결 경계 dedup, 컴포넌트도 방어적 dedup. */
  seq: number
  /** raw 바이트(터미널 write 용). */
  bytes: Uint8Array
}

/** 구독 해제 핸들 — 반드시 호출(unmount/재구독 시). 내부에서 transport 정리까지 수행. */
export interface OutputSubscription {
  unsubscribe: () => void
}

/**
 * 에이전트 제어/구독 단일 표면. 사람 UI 클릭과 (미래) LLM 호출이 같은 진입점을 거친다(§5).
 * 모든 side-effect 메서드는 idempotency·재시도 정책을 구현체가 책임진다.
 */
export interface AgentClient {
  // ── 연결 상태 ──────────────────────────────────────────────────────────────
  readonly connectionState: ConnectionState
  /** 상태 변화 구독. 반환은 해제 함수. */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void

  // ── 출력 구독 ──────────────────────────────────────────────────────────────
  /** 출력 구독. onChunk 로 디코드된 바이트 전달. 반환 핸들의 unsubscribe 로 해제. */
  subscribeOutput(
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
  ): Promise<OutputSubscription>

  // ── 상태/목록/복원 이벤트 ─────────────────────────────────────────────────────
  // 두 모드 공통 표면 — eventBus 가 소비해 store 에 연결한다(모드 무관).
  // Embedded 는 Tauri listen 래핑, Daemon 은 WS 이벤트 라우팅으로 동일 의미를 제공한다.
  // 각 메서드는 sync disposer 를 반환(호출 시 구독 해제). connectionState 패턴과 동일.
  /** 권위 있는 에이전트 목록 교체(존재/제거 판정 기준). */
  onAgentListUpdated(cb: (agents: AgentInfo[]) => void): () => void
  /** 개별 status 갱신(뱃지 표시용, 목록 제거 안 함). */
  onStatusChanged(cb: (id: string, status: AgentStatus, epoch: number) => void): () => void
  /** 부팅 복원 결과(S9). */
  onRestoreResult(cb: (report: RestoreReport) => void): () => void
  /**
   * 프로필 목록 라이브 갱신(깡통/예약 에이전트 — ADR-0018 후속, §5).
   * 백엔드가 프로필 변경(create/delete/activate)을 broadcast 하면 store 미러를 갱신한다.
   * daemon 모드는 AgentEvent::ProfileListUpdated 라우팅으로 동작, embedded 는 후속 backend
   * broadcast 흡수 자리(현재 백엔드 미도달 — 인터페이스·프론트 배선은 지금 깐다).
   */
  onProfileListUpdated(cb: (profiles: AgentProfile[]) => void): () => void

  // ── 명령 ──────────────────────────────────────────────────────────────────
  spawnAgent(cwd: string): Promise<AgentInfo>
  killAgent(agentId: string): Promise<void>
  interruptAgent(agentId: string): Promise<void>
  writeStdin(agentId: string, data: Uint8Array): Promise<void>
  resizePty(agentId: string, cols: number, rows: number): Promise<void>
  getAgents(): Promise<AgentInfo[]>
  getSnapshot(agentId: string): Promise<unknown[]>

  // ── 프로필 CRUD ────────────────────────────────────────────────────────────
  listProfiles(): Promise<AgentProfile[]>
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
  ): Promise<AgentProfile>
  deleteProfile(agentId: string): Promise<void>
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo>
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void>
}
