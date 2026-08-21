//! 도메인 타입(wire 표현). 현 `core::agent::types` / `core::agent::profile` 의 직렬화 형태를 미러.
//! phase 1 에서 core 가 이 crate 에 의존하며 단일 진실원으로 합쳐진다(중복 제거).

use ts_rs::TS;

use crate::ids::{AgentId, PresetId, ProfileId};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

/// 영역별 capability(bool 폭증 방지).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct InputCaps {
    pub raw: bool,
    pub message: bool,
    pub attachment: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct OutputCaps {
    pub terminal_bytes: bool,
    /// 구조화 스트림(NDJSON) 여부(ADR-0044).
    /// `#[serde(default)]`(FIX 3): M1 에서 새로 추가된 필드라, 이 필드가 없는 옛 wire(구 데몬/프론트)를
    /// 받아도 관용적으로 false 로 역직렬화한다(sibling `output_format` 과 같은 additive·tolerant 접근 —
    /// PROTOCOL_VERSION 유지). ts-rs 는 serde(default) 를 optional 로 표기하지 않으므로 TS 는 여전히
    /// `structured: boolean`(non-optional) — 프론트는 손댈 필요 없다.
    #[serde(default)]
    pub structured: bool,
    pub markdown: bool,
    pub tool_events: bool,
    pub usage: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ControlCaps {
    pub resize: bool,
    pub interrupt: bool,
    pub cancel: bool,
    pub graceful_shutdown: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SessionCaps {
    pub resume: bool,
    pub snapshot: bool,
    pub cwd_env: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ModelCaps {
    pub select: bool,
    pub temperature: bool,
    pub max_tokens: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct AgentInfo {
    #[ts(type = "string")]
    pub id: AgentId,
    /// 표시용 이름(ProfileRegistry 단일 진실원, 없으면 id 앞 8자).
    pub name: String,
    pub cwd: String,
    pub status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    /// ★화신(incarnation) 하나를 가리키는 **불투명 표식**★ — 화신마다 새로 뽑은 난수라 **순서에 뜻이
    /// 없다**. 비교는 일치/불일치만 쓴다(대소로 "더 새 것" 을 유도하지 말 것, ADR-0163). 받는 쪽은 이
    /// 값으로 "지금 읽는 출력 스트림이 아까 그 스트림인가" 를 판정한다 — 재구독 계기·deps 는 이
    /// 필드가 아니라 권위 명부 관측이다(ADR-0164 결정 8).
    /// 데몬 프로세스를 넘겨 살지 않는다 — 재기동하면 같은 에이전트도 다른 표식으로 돌아온다.
    pub epoch: u32,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum RestoreOutcome {
    Resumed,
    Started,
    FreshFallback {
        old_sid: Option<String>,
        new_sid: String,
        reason: String,
    },
    Blocked {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct RestoreReport {
    #[ts(type = "string")]
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
}

// ── 프로필 wire 미러(phase4 1단계) ──────────────────────────────────────────────
//
// core 는 protocol 무의존(§1 불변)이라 core 타입을 여기 쓸 수 없다 — 그래서 같은 JSON 형태의
// 독립 타입을 두고, core↔wire 명시 변환은 데몬이 한다(reflection 왕복 금지 — agent_info_to_wire 패턴).
// 프론트 `src/api/types.ts` 의 AgentProfile/AgentCommand/RestartPolicy 와 글자 그대로 일치.

/// Terminal=PTY 대화형, StreamJson=헤드리스 NDJSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum ClaudeOutputFormat {
    #[default]
    Terminal,
    StreamJson,
}

/// 봉투 포맷 wire 타입(ADR-0096/0103) — 데몬이 A→B 메시지를 감쌀 때 쓰는 형식 스위치의 값.
/// 렌더 규칙(정확한 문자열·속성)은 데몬 `control::ingress` 단독 소유 — 이 wire 타입은 스위치
/// 값만 나른다(설계·조립은 데몬, 이 crate 는 순수 wire 계약). 실제 렌더 enum(데몬측)과 이름은 같으나 별개 타입.
///
/// ★serde lowercase(load-bearing)★: `#[serde(rename_all="lowercase")]` 라 wire JSON 이 `"colon"`/`"xml"`
/// (variant 이름 소문자)로 직렬화된다 — `set_envelope_format({format:"xml"})` invoke JSON 이 그대로
/// 역직렬화되게 하는 계약(오퍼레이터/LLM 이 손으로 부르는 표면이라 소문자가 자연스럽다). 다른 wire
/// enum(ClaudeOutputFormat 등)은 PascalCase 지만, 이 타입은 invoke 표면에 직접 노출되므로 lowercase 로 둔다.
/// ★기본 = Xml★: `#[default]` — 데몬 전역 상태 초기값(ADR-0103 기본 flip)과 정합. wire default 자체는
/// SetEnvelopeFormat.format 이 `#[serde(default)]` 아님(항상 명시)이라 배선상 안 쓰이나, 운영 기본과 어긋나면
/// `EnvelopeFormat::default()` 를 부르는 미래 코드가 오해하므로 데몬 기본과 동일하게 맞춘다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum EnvelopeFormat {
    /// `<message from="{sender}" ...>{body}</message>` — 구조 봉투, 운영 기본(ADR-0103).
    #[default]
    Xml,
    /// `{sender}: {body}` — 인간 채팅 관례, 잔존 스위치(ADR-0103 — 삭제 아님).
    Colon,
}

/// core `profile::AgentCommand` 와 동일.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum AgentSpawnCommand {
    /// extra_args 는 세션 인자를 제외한 사용자 추가 인자.
    /// output_format 은 `#[serde(default)]` 라 옛 프로필은 Terminal.
    Claude {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: ClaudeOutputFormat,
    },
    Shell {
        program: String,
        args: Vec<String>,
    },
}

/// **예약(reserved) — 죽은 필드 아님.** 동작 미구현이나 ADR-0016 "추후 재검토" 유효(2026-06-18 결정).
/// 제거 시 core·ts-rs 바인딩·프론트 동반 + PROTOCOL_VERSION bump 유발 → 제거 금지.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum RestartPolicy {
    Never,
    OnCrash,
    Always,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct AgentProfile {
    #[ts(type = "string")]
    pub id: ProfileId,
    pub name: String,
    /// 사용자 지정 표시명 override(ADR-0061 리치화 — 트리 rename). `Some` → 그대로 표시, `None` → cwd
    /// basename 파생(기존 동작 불변). 프론트 트리가 `name` 대신
    /// 이 값을 우선 표시명으로 쓴다(`name` 은 CreateProfile 이름/ad-hoc cwd 문자열이라 표시명 부적합).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub display_name: Option<String>,
    /// 트리 계층 부모 프로필 id(ADR-0072). `Some` → 이 프로필은 해당 부모의 자식(트리 들여쓰기), `None` →
    /// 최상위(루트). 1단 중첩·부모삭제=루트승격 규칙은 데몬(reparent).
    /// `#[serde(default)]` 라 이 필드 없는 옛 wire → None(루트, PROTOCOL_VERSION 유지 — display_name 과 동형).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub parent_id: Option<ProfileId>,
    pub command: AgentSpawnCommand,
    /// 정규화된 cwd.
    pub cwd: String,
    /// ※자격증명 금지(평문 persist).
    pub env: Vec<(String, String)>,
    #[ts(type = "string | null")]
    pub claude_session_id: Option<String>,
    #[ts(type = "string[]")]
    pub old_session_ids: Vec<String>,
    pub epoch: u32,
    pub auto_restore: bool,
    pub restart_policy: RestartPolicy,
    /// 크래시 가드 카운터(수동 재시작 시 0 리셋). **예약(reserved)** — 동작 미구현, ADR-0016 유효.
    pub restart_count: u32,
    /// Failed(자동복원 suspend) 사유 — 콜드부팅 넘어 영속, 수동 깨우기 전까지 자동복원 제외(ADR-0016).
    /// **예약(reserved)** — 동작 미구현이나 ADR-0016에서 유효, 제거 금지(버전 bump 유발).
    #[ts(type = "string | null")]
    pub failed_reason: Option<String>,
    pub created_at: i64,
    pub last_active: i64,
    /// 마지막 프로세스 기동 시각(기록·디버깅용, 리셋 판정엔 미사용).
    #[ts(type = "number | null")]
    pub last_start_at: Option<i64>,
}

// ── 프리셋 wire 미러(ADR-0061) ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Preset {
    #[ts(type = "string")]
    pub id: PresetId,
    /// 정규화된 cwd.
    pub cwd: String,
    /// 사용자 지정 표시명 override(ADR-0061 리치화). `Some` → 그대로 표시, `None` → cwd basename 파생
    /// (기존 동작 불변).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub name: Option<String>,
}

/// core `types::OutputChunk` 와 일치.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SnapshotChunk {
    #[ts(type = "number")]
    pub seq: u64,
    #[serde(with = "serde_bytes")]
    #[ts(type = "number[]")]
    pub data: Vec<u8>,
}
