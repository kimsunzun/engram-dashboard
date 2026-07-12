// Rust 백엔드 타입 미러 — LLD §3 / frontend-integration-lld.md §1
// 백엔드 #[serde(tag="type")]와 정확히 일치하는 discriminated union.

/** 에이전트 생명주기 상태 — `status.type`으로 분기 */
export type AgentStatus =
  | { type: 'Running' }
  | { type: 'Exiting' }
  | { type: 'Exited'; code: number | null }
  | { type: 'Failed'; message: string }
  | { type: 'Killed' }

/** PTY 출력 Channel 페이로드 — data_b64는 base64 인코딩된 raw bytes */
export interface PtyEvent {
  agent_id: string
  seq: number
  /** 세션 epoch — WS binary frame 헤더와 동형(BLOCKER 1). InProc 이 이 값으로 epoch 가드를 통과시킨다. */
  epoch: number
  data_b64: string
}

// ── Capabilities (Rust Capabilities 미러, snake_case) ──────────────────────────

/** PTY 입력 채널 지원 여부 */
export interface InputCaps {
  raw: boolean
  message: boolean
  attachment: boolean
}

/** 출력 포맷 지원 여부 */
export interface OutputCaps {
  terminal_bytes: boolean
  /** 구조화 스트림(NDJSON) 여부 — 렌더러 분기(xterm vs RichSlot) 근거 (ADR-0044) */
  structured: boolean
  markdown: boolean
  tool_events: boolean
  usage: boolean
}

/** 제어 동작 지원 여부 */
export interface ControlCaps {
  resize: boolean
  interrupt: boolean
  cancel: boolean
  graceful_shutdown: boolean
}

/** 세션 연속성 지원 여부 */
export interface SessionCaps {
  resume: boolean
  snapshot: boolean
  cwd_env: boolean
}

/** 모델 파라미터 제어 지원 여부 */
export interface ModelCaps {
  select: boolean
  temperature: boolean
  max_tokens: boolean
}

/** transport 종류별 영역별 capability — AgentInfo에 포함되어 프론트 UI 분기에 사용 */
export interface Capabilities {
  input: InputCaps
  output: OutputCaps
  control: ControlCaps
  session: SessionCaps
  model: ModelCaps
}

/** 에이전트 메타데이터 스냅샷 */
export interface AgentInfo {
  id: string
  /** 표시용 이름. 백엔드 ProfileRegistry에서 채움(없으면 id 앞 8자). */
  name: string
  cwd: string
  status: AgentStatus
  cols: number
  rows: number
  /** 재spawn마다 +1. [agentId, epoch]로 재구독 트리거 (S9 §18) */
  epoch: number
  /** transport 종류별 지원 영역 — UI 분기용 */
  capabilities: Capabilities
}

/** agent-status-changed Tauri event 페이로드 */
export interface AgentStatusChanged {
  id: string
  status: AgentStatus
  /** 재spawn epoch — 옛 세션의 지연 알림을 버리는 데 사용 (S9 §18-d) */
  epoch: number
}

// ── S9: 프로필 + 복원 ──────────────────────────────────────────────────────────

/** claude 출력 포맷 — Terminal=PTY(xterm) / StreamJson=헤드리스 NDJSON(RichSlot). (ADR-0044) */
export type ClaudeOutputFormat = 'Terminal' | 'StreamJson'

/** 에이전트 실행 명령 — 백엔드 #[serde(tag="kind")]와 일치 */
export type AgentCommand =
  | { kind: 'Claude'; extra_args: string[]; output_format: ClaudeOutputFormat }
  | { kind: 'Shell'; program: string; args: string[] }

export type RestartPolicy = 'Never' | 'OnCrash' | 'Always'

/** 영속 프로필 — agents.json 단위. env에 자격증명 금지(평문 저장) */
export interface AgentProfile {
  id: string
  name: string
  /**
   * 사용자 지정 표시명 override(ADR-0061 리치화 — 트리 rename). Some → 그대로 표시, null → cwd basename
   * 파생(기존 동작 불변). 트리는 `name`(CreateProfile 이름/ad-hoc cwd 문자열, 표시명 부적합) 대신 이 값을
   * 우선한다. wire `AgentProfile.display_name` 미러(#[serde(default)] — 옛 agents.json 은 null).
   */
  display_name: string | null
  command: AgentCommand
  cwd: string
  env: [string, string][]
  claude_session_id: string | null
  old_session_ids: string[]
  epoch: number
  auto_restore: boolean
  restart_policy: RestartPolicy
  /** 크래시 가드 카운터(수동 재시작 시 0 리셋 — 동작 TODO) */
  restart_count: number
  /** Failed(자동복원 suspend) 사유 — 콜드부팅 넘어 영속(ADR-0016). 동작 TODO */
  failed_reason: string | null
  created_at: number
  last_active: number
  /** 마지막 프로세스 기동 시각(기록·디버깅용, 리셋 판정엔 미사용) */
  last_start_at: number | null
}

// ── 프리셋(ADR-0061) ────────────────────────────────────────────────────────────

/**
 * 영속 프리셋 — presets.json 단위(데몬 소유, ADR-0061). wire `Preset` 미러(protocol/bindings/Preset.ts).
 * ★이름은 저장하지 않는다★ — cwd basename 을 프론트가 파생한다(ADR-0061). 프로필과 별개 축:
 * 프리셋 = 자주 쓰는 cwd 북마크(스폰 안 함), 프로필 = 실제 에이전트 저장 단위.
 */
export interface Preset {
  id: string
  /** 정규화된 cwd(PathBuf 의 JSON 표현 = 문자열). name override 가 없으면 이 값의 basename 으로 파생. */
  cwd: string
  /**
   * 사용자 지정 표시명 override(ADR-0061 리치화). Some → 그대로 표시, null → cwd basename 파생(기존
   * 동작 불변). wire `Preset.name` 미러(#[serde(default)] — 옛 presets.json 은 null). rename command 가 set/clear.
   */
  name: string | null
}

/** 복원 결말 — agent-restore-result event, #[serde(tag="type")] */
export type RestoreOutcome =
  | { type: 'Resumed' }
  | { type: 'Started' }
  | { type: 'FreshFallback'; old_sid: string | null; new_sid: string; reason: string }
  | { type: 'Blocked'; reason: string }
  | { type: 'Failed'; reason: string }

/** agent-restore-result Tauri event 페이로드 */
export interface RestoreReport {
  agent_id: string
  epoch: number
  outcome: RestoreOutcome
}
