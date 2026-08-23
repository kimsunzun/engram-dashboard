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

/// epoch 는 재구독 트리거(`[agentId,epoch]`).
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

/// 「마지막 실패」 어휘를 **한 목록에서** 선언한다 — 열거형 · wire 표기 · 전수 목록 · 양방향 직렬화가
/// 전부 이 매크로 한 번의 전개에서 나온다.
///
/// ★존재 이유 = 반쪽 수정이 컴파일되지 않게 하는 것★: 손으로 쓴 표가 둘이면(예전 형태: exhaustive match
///   하나 + 고정 길이 배열 하나) 변형을 늘릴 때 한쪽만 고쳐도 **빌드가 통과한다**. 그러면 같은 버전 피어
///   둘이 serialize 는 새 문자열을 내보내고 deserialize 는 그걸 못 찾아 `Other` 로 접는다 — 왕복이 깨지고
///   화면엔 그 종류의 문구 대신 일반 문구가 영원히 뜬다. 목록이 하나뿐이면 그 반쪽 수정 자체가 불가능하다.
/// ★wire 문자열은 `stringify!` 로 **변형 이름에서 파생**된다★: ts-rs 가 내는 TS 유니온도 변형 이름에서
///   나오므로, 손으로 문자열을 적지 않는 한 그 둘은 구조적으로 같다(그 사실은 `messages.rs` 의 golden 이
///   생성된 `.ts` 를 실제로 읽어 다시 확인한다).
// ADR-0161
macro_rules! declare_failure_kinds {
    (
        $( $(#[$vmeta:meta])* $variant:ident ),+ $(,)?
        ; absorbing = $absorbing:ident
    ) => {
        /// core `failure::AgentFailureKind` 와 동일. 「마지막 실패」의 종류 어휘 — **화면 문구는 여기
        /// 없다**: 종류 → {다시 해볼 가치 · 문구 · 권하는 행동} 표는 프론트가 진다
        /// (ADR-0161 결정 5 · ADR-0162).
        ///
        /// ★상태(`AgentStatus`)와 별개 축이라 합치지 않는다★ — "도는 중인데 마지막 실패를 든" 조합이
        ///   표현돼야 화면의 「도는 중이 이긴다」 규칙이 성립한다.
        /// ★변형을 늘리려면 `declare_failure_kinds!` 호출 한 곳만 고친다★ — 여기 직접 쓸 수 없다.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, TS)]
        #[ts(export)]
        pub enum AgentFailureKind {
            $( $(#[$vmeta])* $variant, )+
        }

        impl AgentFailureKind {
            /// 어휘 전수 = (변형, wire 표기). 매크로가 열거형과 **같은 목록**에서 만든다.
            const ALL: &'static [(AgentFailureKind, &'static str)] =
                &[ $( (AgentFailureKind::$variant, stringify!($variant)), )+ ];

            /// 어휘 전수 — golden 이 생성된 TS 유니온과 대조하는 축(테스트가 어휘를 다시 적지 않게).
            pub fn all() -> impl Iterator<Item = &'static (AgentFailureKind, &'static str)> {
                AgentFailureKind::ALL.iter()
            }

            /// 위와 같은 목록의 이름만.
            pub fn wire_names() -> impl Iterator<Item = &'static str> {
                AgentFailureKind::ALL.iter().map(|(_, name)| *name)
            }

            /// wire 표기 — 변형 이름 그대로(`stringify!`).
            fn as_wire_str(self) -> &'static str {
                match self {
                    $( AgentFailureKind::$variant => stringify!($variant), )+
                }
            }
        }

        impl serde::Serialize for AgentFailureKind {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_wire_str())
            }
        }

        /// 모르는 종류를 흡수 변형으로 접는다 — **어휘가 늘어날 때 옛 피어를 지키는 유일한 장치**.
        ///
        /// ★막는 사고★: 이 타입은 프로필 구조체 **안**에 있어서, 그냥 파생하면 새 변형 문자열 하나가
        ///   `AgentProfile` **메시지 전체**의 디코드를 실패시킨다 — 필드만 비는 게 아니라 그 프로필이
        ///   통째로 사라진다. 옛 빌드의 src-tauri 셸이 새 데몬에 붙는 조합이 실제 경로이고, 그쪽
        ///   수신부는 디코드 실패를 **조용히 버린다**(증상이 "트리가 안 바뀐다" 뿐이다).
        /// ★프론트와 같은 계약의 러스트 쪽 절반★: `src/components/agent/failureKinds.ts` 가
        ///   `table[kind] ?? Other` 로 같은 fail-open 을 이미 한다.
        /// ★흡수 범위는 **모르는 문자열** 하나뿐이다★: `42`·`[]`·`{}` 같은 비-문자열은 그대로 오류가
        ///   된다. 전부 삼키면 망가진 값이 흡수 변형으로 둔갑해 **실패가 없는 항목에 실패 문구가 뜬다** —
        ///   그건 흡수가 아니라 날조다.
        /// ★`String` 으로 받는다(`&str` 로 좁히지 말 것)★: 빌린 `&str` 은 이스케이프 없는 in-memory
        ///   버퍼에서만 성공한다 — `"Other"` 처럼 이스케이프가 하나만 있어도 serde_json 이 scratch
        ///   경로로 빠져 `invalid type: string, expected a borrowed string` 을 내고 **메시지 전체**가
        ///   깨진다. `from_value`/`from_reader` 경로도 같다.
        impl<'de> serde::Deserialize<'de> for AgentFailureKind {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = <String as serde::Deserialize>::deserialize(d)?;
                Ok(AgentFailureKind::ALL
                    .iter()
                    .find(|(_, name)| *name == raw)
                    .map(|(kind, _)| *kind)
                    .unwrap_or(AgentFailureKind::$absorbing))
            }
        }
    };
}

declare_failure_kinds! {
    /// 이어받을 대화 실물이 없다 — 한 마디도 주고받지 않고 죽은 항목.
    NoConversationToResume,
    /// 프로세스를 띄우지 못했다.
    SpawnFailed,
    /// 이어받기로 떴으나 관측 창 안에 종료했다.
    EarlyExitAfterResume,
    /// 생산자가 분류하지 못했다 — **그리고** 소비자가 모르는 어휘를 흡수하는 자리다.
    Other,
    ; absorbing = Other
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
    /// 이 항목이 마지막으로 활성화에 실패한 종류(ADR-0161). `null` = 실패 기록 없음.
    ///
    /// ★데몬 메모리에만 산다★ — core 쪽 원본이 `#[serde(skip)]` 이라 `agents.json` 에 없고, 데몬을
    ///   재기동하면 사라진다(앱 창 재시작은 견딘다).
    /// `#[serde(default)]` 라 이 필드 없는 옛 wire → None(PROTOCOL_VERSION 유지 — display_name·
    /// parent_id 와 동형 additive). 모르는 **값**은 `Other` 로 흡수된다(`AgentFailureKind` 의
    /// `Deserialize` 구현 — 그 doc 이 왜 필요한지의 정본).
    #[serde(default)]
    pub last_failure: Option<AgentFailureKind>,
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
