// Rust 백엔드 타입 미러 — LLD §3 / frontend-integration-lld.md §1
// 백엔드 #[serde(tag="type")]와 정확히 일치하는 discriminated union.

export type AgentStatus =
  | { type: 'Running' }
  | { type: 'Exiting' }
  | { type: 'Exited'; code: number | null }
  | { type: 'Failed'; message: string }
  | { type: 'Killed' }

export interface PtyEvent {
  agent_id: string
  seq: number
  /** 세션 epoch — WS binary frame 헤더와 동형(BLOCKER 1). */
  epoch: number
  data_b64: string
}

// ── Capabilities (Rust Capabilities 미러, snake_case) ──────────────────────────

export interface InputCaps {
  raw: boolean
  message: boolean
  attachment: boolean
}

export interface OutputCaps {
  terminal_bytes: boolean
  /** 구조화 스트림(NDJSON) 여부 — 렌더러 분기(xterm vs RichSlot) 근거 (ADR-0044) */
  structured: boolean
  markdown: boolean
  tool_events: boolean
  usage: boolean
}

export interface ControlCaps {
  resize: boolean
  interrupt: boolean
  cancel: boolean
  graceful_shutdown: boolean
}

export interface SessionCaps {
  resume: boolean
  snapshot: boolean
  cwd_env: boolean
}

export interface ModelCaps {
  select: boolean
  temperature: boolean
  max_tokens: boolean
}

export interface Capabilities {
  input: InputCaps
  output: OutputCaps
  control: ControlCaps
  session: SessionCaps
  model: ModelCaps
}

export interface AgentInfo {
  id: string
  /** 표시용 canonical 이름 — 백엔드가 `display_name ?? cwd basename` 으로 채움(ADR-0101). profile.name 이 아니다. */
  name: string
  cwd: string
  status: AgentStatus
  cols: number
  rows: number
  /**
   * ★화신(incarnation) 하나를 가리키는 **불투명 표식**★ — 화신마다 새로 뽑은 난수라 **순서에 뜻이 없다**.
   * 비교는 일치/불일치만 쓴다(대소로 "더 새 것" 을 유도하지 말 것). 프론트는 이 값을 "지금 읽는 출력
   * 스트림이 아까 그 스트림인가" 의 판정 축으로만 쓴다 — ★재구독·재마운트 트리거로 쓰지 말 것★:
   * 종료가 곧 명부에서 사라지는 것이라, 이 값을 컴포넌트 prop 으로 내려보내면 종료 순간 값이 떨어지며
   * **replay 가 오기도 전에** 화면이 지워진다(데몬 ring 은 이미 없어 복구 불가). (ADR-0007)
   */
  epoch: number
  capabilities: Capabilities
}

export interface AgentStatusChanged {
  id: string
  status: AgentStatus
  /** 이 알림을 낸 화신의 표식 — 옛 세션의 지연 알림을 버리는 데 쓴다(일치/불일치만, AgentInfo.epoch). */
  epoch: number
}

// ── S9: 프로필 + 복원 ──────────────────────────────────────────────────────────

/** claude 출력 포맷 — Terminal=PTY(xterm) / StreamJson=헤드리스 NDJSON(RichSlot). (ADR-0044) */
export type ClaudeOutputFormat = 'Terminal' | 'StreamJson'

/**
 * 에이전트 실행 명령 — 백엔드 #[serde(tag="kind")]와 일치.
 *
 * ★이름 충돌★ — 러스트 `protocol` 의 동명 타입은 뜻이 다르다(데몬에 보내는 wire 명령).
 * 이 타입의 러스트 짝은 `AgentSpawnCommand` 다. 생성 바인딩을 물릴 때 `AgentCommand.ts`
 * 가 아니라 `AgentSpawnCommand.ts` 를 가져올 것 — 전자는 모양이 달라 tsc 가 잡지만,
 * 그 방어는 이 값을 짓는 테스트 픽스처가 있을 때만 작동한다(프로덕션 코드는 안 만든다).
 */
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
  /**
   * 트리 계층의 부모 프로필 id(ADR-0072). Some(pid) → pid 의 자식(들여쓰기), null → 최상위(루트).
   * 1단 중첩만 허용(부모는 반드시 루트) — 검증은 백엔드 ProfileRegistry::reparent + 쓰기 경계 정규화.
   * wire `AgentProfile.parent_id` 미러(#[serde(default)] — 옛 agents.json 은 null).
   */
  parent_id: string | null
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
 * 프로필과 별개 축: 프리셋 = 자주 쓰는 cwd 북마크(스폰 안 함), 프로필 = 실제 에이전트 저장 단위.
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

/** 복원 결말 — restore-result event, #[serde(tag="type")] */
export type RestoreOutcome =
  | { type: 'Resumed' }
  | { type: 'Started' }
  | { type: 'FreshFallback'; old_sid: string | null; new_sid: string; reason: string }
  | { type: 'Blocked'; reason: string }
  | { type: 'Failed'; reason: string }

/** restore-result Tauri event 페이로드의 `result` 필드 */
export interface RestoreReport {
  agent_id: string
  epoch: number
  outcome: RestoreOutcome
}
