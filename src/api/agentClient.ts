// AgentClient — 프론트가 의존하는 단일 제어 표면(S12 phase 1b, daemon-design §3-a).
//
// 컴포넌트·스토어는 invoke/Channel/WS 를 직접 부르지 않고 이 인터페이스만 의존한다.
// 단일 구현 ProtocolClient(프로토콜 의미론 1벌) + transport 2개(InProc/Ws, ADR-0020 Stage 3~4a).
// transport(Tauri Channel / WS binary frame)와 base64/디코딩은 transport 내부에 숨긴다 —
// 인터페이스는 "디코드된 바이트 청크"만 노출(§3-a 손발/두뇌 분리: 프론트=순수 I/O).

import type {
  AgentInfo,
  AgentProfile,
  AgentStatus,
  ClaudeOutputFormat,
  RestoreReport,
} from './types'

/** 클라↔백엔드 연결 상태. Embedded 는 항상 connected. Daemon 만 reconnecting/down 발생. */
export type ConnectionState = 'connected' | 'reconnecting' | 'down'

/** 디코드된 출력 청크 — transport 무관(base64/binary frame 은 클라 내부에서 이미 풀림). */
export interface OutputChunk {
  /** core OutputCore 발급 seq(단조 증가). 클라가 재연결 경계 dedup, 컴포넌트도 방어적 dedup. */
  seq: number
  /**
   * frame 종류(wsFrame.ts): 0=터미널 raw 바이트(xterm write) / 1=StructuredEvent JSON(ADR-0045 tag1).
   * 단일 콜백에 tag 를 실어 소비자가 렌더 경로를 가른다(TerminalSlot=tag0 무시하고 bytes / RichSlot=
   * tag1 이면 JSON.parse). 별도 콜백 대신 tag 필드로 통합한 이유: seq dedup·epoch 가드·pre-subscribe
   * 버퍼가 tag 를 몰라도 되게(한 seq 공간·한 배달 경로) — 콜백 이중화는 그 규율을 두 벌로 쪼갠다.
   */
  tag: number
  /** raw payload — tag0 이면 터미널 바이트, tag1 이면 StructuredEvent JSON UTF-8 바이트. */
  bytes: Uint8Array
}

/** 구독 해제 핸들 — 반드시 호출(unmount/재구독 시). 내부에서 transport 정리까지 수행. */
export interface OutputSubscription {
  unsubscribe: () => void
}

/**
 * 뷰별 replay 상태 스냅샷(§5 LLM 제어 표면 — ADR-0046). getViewOutputState 가 반환한다.
 * error(재요청 사다리 소진) 등을 LLM/자동화가 타입으로 발견·재구동 판단에 쓴다. 최소 노출.
 */
export interface ViewOutputState {
  agentId: string
  /** buffering(축적 중) / live(직행 배달) / error(재요청 소진). */
  phase: 'buffering' | 'live' | 'error'
  /** buffering 중 축적 프레임 수(디버그·관측). */
  buffered: number
  /** 재요청 사다리 시도 횟수(0=아직 재요청 안 함). */
  attempts: number
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

  /**
   * **명시 연결(spawn 허용)** — ADR-0021 §1. transport.start 위임. 부팅 1회 / 사용자 daemon_start 가
   * 부른다(DaemonControl.start). 데몬이 없으면 여기서만 spawn 한다. 명령 경로(ensureReady)는
   * attach-only 라 spawn 못 하므로, 데몬을 띄우는 유일한 의도적 진입점이다. daemon 모드만 의미 있고
   * embedded(InProc)는 Channel 등록(no-op spawn). 재연결로 멈췄던 상태(closedByUser/attempt)를 리셋.
   */
  connect(): Promise<void>
  /**
   * **명시 연결 해제(재연결 중단, ADR-0021 note3)** — transport.close 위임. graceful daemon_stop
   * 후 부른다: closedByUser=true 로 즉시 'down' 정착해 5회 재연결 헛시도를 없앤다. ProtocolClient
   * 자체(구독 라우터/콜백 레지스트리)는 유지하므로, 이후 connect 로 다시 살릴 수 있다(close 와 다름).
   */
  disconnect(): void

  // ── 출력 구독 ──────────────────────────────────────────────────────────────
  /**
   * 뷰(slot) 단위 출력 구독(ADR-0046). viewId = 슬롯 id — 같은 agentId 를 N 뷰가 봐도 각자 독립 진도
   * (버그 B 구조 해소). onChunk 로 디코드된 바이트 전달. onState(옵션)로 replay 상태(buffering/live/error)
   * 통지 — 슬롯이 error·streaming 표면화에 쓴다. 반환 핸들의 unsubscribe 로 해제.
   */
  subscribeOutput(
    viewId: string,
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
    onState?: (state: 'buffering' | 'live' | 'error') => void,
  ): Promise<OutputSubscription>

  /**
   * 뷰(slot)별 replay 상태 조회(§5 LLM 제어 표면 — ADR-0046). error 소진(재요청 3회 실패)·buffering
   * 고착 등을 LLM/자동화가 관측·재구동 판단에 쓴다. 없는 viewId 면 null. (구현 = ProtocolClient.)
   */
  getViewOutputState(viewId: string): ViewOutputState | null

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
  /**
   * 데몬 graceful 종료(ADR-0021 §5). StopDaemon AgentCommand 전송 — 데몬이 자식 PTY 를 정리하고
   * 스스로 내려간다. force=false 면 실활성 에이전트가 있을 때 데몬이 거부(Error). embedded 모드는
   * in-proc 라 무의미(데몬 없음) → carrier 가 무시(앱 안 내림). DaemonControl.stop 이 이걸 graceful
   * 단계로 부르고, 실패/연결없음 시 daemon_stop(fallback kill)로 보강한다.
   */
  stopDaemon(force: boolean): Promise<void>

  // ── 프로필 CRUD ────────────────────────────────────────────────────────────
  listProfiles(): Promise<AgentProfile[]>
  /**
   * claude 프로필 생성. outputFormat 은 렌더 모드를 가른다(ADR-0044): 'Terminal'=PTY(xterm),
   * 'StreamJson'=헤드리스 NDJSON(RichSlot). 기본 'Terminal'(기존 호출자 동작 불변 — wire 는
   * `#[serde(default)]`). §5 제어 표면(cdp/console)이 이 인자로 json 에이전트를 스폰한다.
   */
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat?: ClaudeOutputFormat,
  ): Promise<AgentProfile>
  deleteProfile(agentId: string): Promise<void>
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo>
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void>
}
