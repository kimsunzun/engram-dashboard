//! 도메인 타입(wire 표현). 현 `agent::types` / `agent::profile` 의 직렬화 형태를 미러.
//! ★이 중복을 합치려 `agent` 가 이 crate 를 의존하게 만들지 말 것★ — 그 crate 의 protocol-무의존이
//! 불변식이라 미러가 그 대가다(정본 = `crates/engram-dashboard-agent/Cargo.toml` `[dependencies]` 주석).
//! ★단 사용량 한도 타입(`Usage*`)은 미러가 아니다★ — 데몬 답의 모양이라 agent `usage` 의 관측 타입과
//!   칸이 일부러 다르다(부호·나이·만료). 맞추려 들지 말 것.

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
// agent 는 protocol 무의존(§1 불변)이라 agent 타입을 여기 쓸 수 없다 — 그래서 같은 JSON 형태의
// 독립 타입을 두고, agent↔wire 명시 변환은 데몬이 한다(reflection 왕복 금지 — agent_info_to_wire 패턴).
// 프론트 `src/api/types.ts` 의 AgentProfile/AgentCommand/RestartPolicy 와 글자 그대로 일치.

/// Terminal=PTY 대화형, StreamJson=헤드리스 NDJSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentOutputFormat {
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
/// enum(AgentOutputFormat 등)은 PascalCase 지만, 이 타입은 invoke 표면에 직접 노출되므로 lowercase 로 둔다.
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

/// agent `profile::AgentCommand` 의 wire 미러.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum AgentSpawnCommand {
    /// extra_args 는 세션 인자를 제외한 사용자 추가 인자.
    /// output_format 은 `#[serde(default)]` 라 이 칸이 없는 옛 패킷은 Terminal 로 흡수된다.
    Claude {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: AgentOutputFormat,
    },
    Shell {
        program: String,
        args: Vec<String>,
    },
    /// extra_args 는 대화형 인자(`--cd`·`-s`·`-a`)를 제외한 사용자 추가 인자 — 그 조립은 코어의
    /// codex backend 가 하고 이 wire 는 그 목록을 그대로 나른다.
    /// output_format 은 형제 `Claude` 와 같은 계약(`#[serde(default)]` → 없으면 Terminal)이지만
    /// **가르는 것이 다르다**: codex 에서는 이 값이 대화형 TUI 와 상주 JSON 서버(`codex app-server`)를
    /// 가른다(코어 `backend::codex::is_app_server`).
    Codex {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: AgentOutputFormat,
    },
}

/// 스폰 패킷이 **어느 백엔드를 띄울지** 고르는 칸. `AgentSpawnCommand` 가 「무엇을 어떤 인자로」라면
/// 이것은 「어느 프로그램인가」 하나만 고르는 좁은 어휘다 — 인자를 아직 못 정하는 입구(`SpawnByCwd`)가
/// 쓴다.
///
/// ★부재의 뜻 = 오류다(사용자 결정 2026-09-07)★: 이 칸을 안 채운 패킷은 데몬이 거절한다. 기본값을 두면
/// 새 스폰 입구가 생길 때마다 **고르지 않은 것**과 **claude 를 고른 것**이 구별되지 않고, 그 조용한
/// 기본값이 곧 「요청한 것과 다른 에이전트가 떴다」가 된다.
/// ★철자가 lowercase 인 이유★: 이 값은 invoke 표면(`spawn_into` 의 `backend` 인자)에서 오는 문자열과
/// 같은 낱말이어야 하고 그 자리는 이미 `"claude"` 로 적혀 있었다(형제 `EnvelopeFormat` 과 같은 사유).
/// ★받는 철자는 대소문자를 가리지 않는다 · **내보내는 철자는 lowercase 하나뿐이다**★: 이 칸을 채우는
/// 것은 사람·LLM 이 손으로 친 낱말이고(`agent.spawnInto` 의 `backend`, 프론트의 `SpawnByCwd`) 거기서
/// `Claude` 는 오타가 아니라 같은 뜻이다. 반대로 내보내는 쪽을 넓히면 `agents.json` 과 ts-rs 유니온이
/// 갈리므로 [`Serialize`](serde::Serialize) 는 derive 그대로 둔다.
// `Ord` = 선언 순서 — 뜻이 없고 정렬된 집합(셸의 사용량 관심 `BTreeSet`)의 열쇠로만 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AgentBackendKind {
    Claude,
    Codex,
}

impl AgentBackendKind {
    /// wire 낱말 전량 — ★철자가 적히는 곳은 여기 하나다★. 반려 문구의 기대값이 이 배열 그대로이고
    /// (`parse_backend` 가 그 문구를 실어 나른다) 아래 `ALL` 도 여기서 낱말을 꺼낸다.
    /// `#[serde(rename_all = "lowercase")]` 이 내보내는 철자와 같아야 하며, 그 일치를 재는 자리 =
    /// `tests::backend_kind_serializes_exactly_the_declared_words`.
    const WORDS: &'static [&'static str] = &["claude", "codex"];

    /// (변형, wire 표기) 짝 — 역직렬화가 낱말에서 변형으로 되돌아오는 유일한 표.
    const ALL: &'static [(AgentBackendKind, &'static str)] = &[
        (AgentBackendKind::Claude, Self::WORDS[0]),
        (AgentBackendKind::Codex, Self::WORDS[1]),
    ];
}

/// ★`String` 으로 받는다(`&str` 로 좁히지 말 것)★ — 형제 [`AgentFailureKind`] 와 같은 사유: 이스케이프가
/// 하나라도 있으면 serde_json 이 scratch 경로로 빠져 빌린 `&str` 이 실패하고 **메시지 전체**가 깨진다.
/// ★모르는 낱말은 흡수하지 않고 반려한다★ — 그 형제와 갈리는 지점이다: 여기서 접으면 요청한 것과 다른
/// 백엔드가 조용히 뜬다(이 타입 doc 의 「부재의 뜻 = 오류」와 같은 결정).
impl<'de> serde::Deserialize<'de> for AgentBackendKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(d)?;
        let folded = raw.to_ascii_lowercase();
        AgentBackendKind::ALL
            .iter()
            .find(|(_, word)| *word == folded)
            .map(|(kind, _)| *kind)
            .ok_or_else(|| serde::de::Error::unknown_variant(&raw, AgentBackendKind::WORDS))
    }
}

/// **예약(reserved) — 죽은 필드 아님.** 동작 미구현이나 ADR-0016 "추후 재검토" 유효(2026-06-18 결정).
/// 제거 시 agent·ts-rs 바인딩·프론트 동반 + PROTOCOL_VERSION bump 유발 → 제거 금지.
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
// ADR-0172
macro_rules! declare_failure_kinds {
    (
        $( $(#[$vmeta:meta])* $variant:ident ),+ $(,)?
        ; absorbing = $absorbing:ident
    ) => {
        /// agent `failure::AgentFailureKind` 와 동일. 「마지막 실패」의 종류 어휘 — **화면 문구는 여기
        /// 없다**: 종류 → {다시 해볼 가치 · 문구 · 권하는 행동} 표는 프론트가 진다
        /// (ADR-0172 결정 5 · ADR-0173).
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
    pub backend_session_id: Option<String>,
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
    /// 이 항목이 마지막으로 활성화에 실패한 종류(ADR-0172). `null` = 실패 기록 없음.
    ///
    /// ★데몬 메모리에만 산다★ — agent 쪽 원본이 `#[serde(skip)]` 이라 `agents.json` 에 없고, 데몬을
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

/// agent `types::OutputChunk` 와 일치.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SnapshotChunk {
    #[ts(type = "number")]
    pub seq: u64,
    #[serde(with = "serde_bytes")]
    #[ts(type = "number[]")]
    pub data: Vec<u8>,
}

// ── 사용량 한도 wire(구독형 — 클라이언트가 벤더를 구독하고 데몬이 바뀔 때마다 보낸다) ──────────────

/// 한 벤더·계정의 사용량 상태 — 데몬이 그 벤더를 구독한 연결에 보내는 한 장
/// ([`AgentEvent::UsageLimitsUpdated`](crate::AgentEvent::UsageLimitsUpdated)).
///
/// ★시간 칸 규칙★: 절대 시각은 [`UsageWindow::resets_at`](서버가 준 epoch 초) 하나뿐이다. 나머지 시간
///   칸(`age_secs`·`next_attempt_in_secs`·`retry_in_secs`)은 **이 한 장을 뜬 순간 기준 상대 초**라, 받는 쪽은
///   받은 순간부터 흐른 만큼 더해 읽는다 — 양쪽 벽시계가 어긋나거나 되감겨도 흔들리지 않게.
/// ★상태 문구는 받는 쪽이 번역 키로 만든다★ — 상태는 코드 + 수로 나른다. 예외 = `detail`(분류 낱말 `kind`·
///   상류 원문 `upstream` 은 번역 없이 그대로 보인다).
/// ★`null` 창·`null` 수는 「모른다」이지 0 이 아니다★ — 0% 로 그리면 안 된다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageLimitSnapshot {
    pub vendor: AgentBackendKind,
    /// 같은 벤더 안의 계정 — 지금은 늘 `"default"`(데몬 env 의 기본 로그인).
    pub account_key: String,
    pub five_hour: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
    /// 모델별 주간 창. 빈 배열 = 없음.
    pub model_scoped: Vec<UsageScopedWindow>,
    pub plan: Option<String>,
    /// 조회가 아직 진행 중이다 — 이 한 장의 값은 그 조회 전의 것이고, 결과는 조회가 끝나면 구독한 연결에 새
    /// 한 장으로 간다.
    pub in_flight: bool,
    /// `Ready` 가 아니어도 위 값들은 유효하다 — 실패는 마지막으로 알던 값을 지우지 않는다.
    /// 예외는 `Unavailable` 이다 — 값을 싣지 않는다.
    pub state: UsageVendorState,
    /// 이 칸(벤더·계정)의 단조 번호 — 무엇이든 바뀌면 +1. 받는 쪽은 같은 연결(소켓) 안에서 칸마다 최댓값만
    /// 남기고 더 작은 것은 버린다. 새 소켓이 서면 그 기억을 잊는다 — 데몬이 재시작하면 0 부터 다시 센다.
    #[ts(type = "number")]
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageWindow {
    /// **쓴** 양의 백분율 0–100(남은 양이 아니다). 생산자는 소수 넷째 자리로 반올림해 싣는다(agent
    /// `usage/normalize.rs` 의 `PCT_SCALE`) — 받는 쪽은 남은 양을 보정 없이 `floor(100 − used_pct)` 로 보여 주므로,
    /// 이진 부동소수 꼬리(0.55 × 100 = 55.00000000000001)가 정수 경계를 넘지 않는 것이 그 반올림에 기댄다.
    pub used_pct: Option<f64>,
    /// 리셋 시각, epoch 초.
    #[ts(type = "number | null")]
    pub resets_at: Option<u64>,
    /// 이 값을 관측한 뒤 흐른 초.
    #[ts(type = "number")]
    pub age_secs: u64,
    /// 데몬이 리셋 경과를 확인했다 — 한 번 서면 이 창에 새 값이 올 때까지 유지된다(벽시계가 되감겨도).
    pub expired: bool,
}

/// 모델별 주간 창 하나. `label` = 벤더가 준 표시용 이름.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageScopedWindow {
    pub label: String,
    pub window: UsageWindow,
}

/// 벤더 조회의 상태 코드. 비정상 다섯의 `detail` = 왜 그 상태인가(없으면 `null` — 원인을 모른다).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum UsageVendorState {
    Ready,
    /// 그 벤더의 CLI 가 없다.
    NotInstalled {
        detail: Option<UsageStateDetail>,
    },
    /// 인증 오류 — 로그인이 필요하다.
    NeedsLogin {
        detail: Option<UsageStateDetail>,
    },
    /// 조회는 됐지만 이 계정엔 한도 정보가 없다(로그아웃 · API 키 계정 · profile 권한이 없는 토큰 등)
    /// — 스냅숏에 창 값이 없다.
    Unavailable {
        detail: Option<UsageStateDetail>,
    },
    /// 그 밖의 실패. `next_attempt_in_secs` = 다음 자동 조회까지 남은 초 — 늘 정의된다(기준점이 없으면 거절
    /// 끝 또는 지금), 이미 지났으면 0.
    Failed {
        #[ts(type = "number")]
        next_attempt_in_secs: u64,
        detail: Option<UsageStateDetail>,
    },
    /// 상류가 조회를 거절했다. 이 초가 지나기 전에는 강제 새로고침도 조회를 내보내지 않는다.
    Rejected {
        #[ts(type = "number")]
        retry_in_secs: u64,
        detail: Option<UsageStateDetail>,
    },
}

/// 비정상 상태의 원인.
///
/// `kind` = 분류 낱말(예 `rate_limits_null`·`claude_error`·`rpc_error`·`limits_unavailable`·`timeout`).
/// ★enum 이 아니라 문자열이다★ — 분류가 늘어도 옛 셸이 스냅숏 전체를 못 읽는 일이 없게. 받는 쪽은
///   번역하지 않고 그대로 보인다.
/// `code` = 상류가 준 수(Codex JSON-RPC `error.code`). `upstream` = 상류 원문 — 보내는 쪽이 이 순서로
///   다듬은 것이다: 비밀 가림 → 공백류 제어(탭·개행·CR·VT·FF·NEL·U+2028/2029)는 공백, 그 밖의 제어·
///   보이지 않는 서식 문자는 U+FFFD → 200자로 자름. 받는 쪽이 다시 정화할 필요는 없다.
/// ★`Debug` 는 `upstream` 을 글자 수로만 찍는다★ — 어디서 `{:?}` 로 찍혀도 원문이 로그에 안 나가게.
///   원문은 wire(`Serialize`)와 비교(`PartialEq`)만 본다.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageStateDetail {
    pub kind: String,
    #[ts(type = "number | null")]
    pub code: Option<i64>,
    pub upstream: Option<String>,
}

impl std::fmt::Debug for UsageStateDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct CharCount(usize);
        impl std::fmt::Debug for CharCount {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "<{}자>", self.0)
            }
        }
        f.debug_struct("UsageStateDetail")
            .field("kind", &self.kind)
            .field("code", &self.code)
            .field(
                "upstream",
                &self.upstream.as_ref().map(|s| CharCount(s.chars().count())),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 이 칸이 없는 옛 패킷 — 데몬이 명부를 보낼 때 `output_format` 을 안 싣던 빌드가 이 모양이다.
    /// ★`#[serde(default)]` 는 **옛 데몬 → 새 셸** 한 방향만 진다★: 이것이 빠지면 그 데몬의
    /// `ProfileList` 가 **한 행이 아니라 응답 전체** 역직렬화 실패로 무너진다(그 실패 단위는
    /// [`crate::PROTOCOL_VERSION`] 의 v4 항목이 정본). 반대 방향(새 데몬 → 옛 셸)을 떠받치는 것은 이
    /// attribute 가 아니라 serde 의 **미지 필드 관용**이라 여기서 재지 않는다.
    #[test]
    fn codex_spawn_command_without_output_format_defaults_to_terminal() {
        let legacy = r#"{ "kind": "Codex", "extra_args": ["--foo"] }"#;
        let cmd: AgentSpawnCommand = serde_json::from_str(legacy).expect("옛 패킷 역직렬화");
        assert_eq!(
            cmd,
            AgentSpawnCommand::Codex {
                extra_args: vec!["--foo".to_string()],
                output_format: AgentOutputFormat::Terminal,
            }
        );
    }

    /// 실린 값은 그대로 돌아온다 — 위 항목 혼자면 이 타입이 `Terminal` 을 **고정**해도 초록이다.
    #[test]
    fn codex_spawn_command_roundtrips_both_modes() {
        for mode in [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson] {
            let cmd = AgentSpawnCommand::Codex {
                extra_args: vec![],
                output_format: mode,
            };
            let json = serde_json::to_string(&cmd).expect("직렬화");
            assert_eq!(
                serde_json::from_str::<AgentSpawnCommand>(&json).expect("역직렬화"),
                cmd,
                "{json}"
            );
        }
    }

    /// 손으로 친 낱말은 대소문자를 가리지 않는다 — `agent.spawnInto` 의 `backend` 와 프론트의
    /// `SpawnByCwd` 가 사람·LLM 이 친 문자열을 그대로 싣는다.
    #[test]
    fn backend_kind_accepts_any_casing() {
        for (raw, want) in [
            ("claude", AgentBackendKind::Claude),
            ("Claude", AgentBackendKind::Claude),
            ("CLAUDE", AgentBackendKind::Claude),
            ("codex", AgentBackendKind::Codex),
            ("Codex", AgentBackendKind::Codex),
            ("CODEX", AgentBackendKind::Codex),
        ] {
            let got: AgentBackendKind =
                serde_json::from_value(serde_json::Value::String(raw.to_string()))
                    .unwrap_or_else(|e| panic!("'{raw}' 는 받아야 한다: {e}"));
            assert_eq!(got, want, "{raw}");
        }
    }

    /// 관용은 **철자**에만 든다 — 모르는 낱말은 그대로 반려하고, 문구가 기대 낱말을 나열한다.
    #[test]
    fn backend_kind_still_rejects_a_word_it_does_not_know() {
        let err = serde_json::from_value::<AgentBackendKind>(serde_json::Value::String(
            "codx".to_string(),
        ))
        .expect_err("모르는 낱말은 반려");
        let text = err.to_string();
        assert!(text.contains("codx"), "{text}");
        assert!(text.contains("claude") && text.contains("codex"), "{text}");
    }

    /// 내보내는 철자는 안 바뀐다 — `agents.json` 과 ts-rs 유니온이 이 값을 읽는다.
    #[test]
    fn backend_kind_serializes_exactly_the_declared_words() {
        for (kind, word) in AgentBackendKind::ALL {
            assert_eq!(
                serde_json::to_value(kind).expect("직렬화"),
                serde_json::Value::String((*word).to_string())
            );
        }
        assert_eq!(
            AgentBackendKind::ALL.len(),
            AgentBackendKind::WORDS.len(),
            "낱말 표와 짝 표가 갈렸다"
        );
    }

    // ── 사용량 한도 wire ──

    fn usage_window(used_pct: Option<f64>) -> UsageWindow {
        UsageWindow {
            used_pct,
            resets_at: Some(1_900_000_000),
            age_secs: 42,
            expired: false,
        }
    }

    fn usage_snapshot(state: UsageVendorState) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor: AgentBackendKind::Codex,
            account_key: "default".to_string(),
            five_hour: Some(usage_window(Some(37.5))),
            weekly: None,
            model_scoped: vec![UsageScopedWindow {
                label: "opus".to_string(),
                window: usage_window(None),
            }],
            plan: Some("pro".to_string()),
            in_flight: true,
            state,
            revision: 7,
        }
    }

    fn detail(code: Option<i64>, upstream: Option<&str>) -> UsageStateDetail {
        UsageStateDetail {
            kind: "rpc_error".to_string(),
            code,
            upstream: upstream.map(str::to_string),
        }
    }

    /// 전체 모양을 한 번에 잰다 — 부분 단언은 검사 안 한 칸의 이름·표기 변경을 통과시킨다.
    #[test]
    fn usage_snapshot_serializes_to_the_exact_wire_shape() {
        let snap = usage_snapshot(UsageVendorState::Failed {
            next_attempt_in_secs: 840,
            detail: Some(detail(Some(-32603), Some("internal error"))),
        });
        assert_eq!(
            serde_json::to_value(&snap).expect("직렬화"),
            serde_json::json!({
                "vendor": "codex",
                "account_key": "default",
                "five_hour": {
                    "used_pct": 37.5, "resets_at": 1_900_000_000u64, "age_secs": 42,
                    "expired": false
                },
                "weekly": null,
                "model_scoped": [{
                    "label": "opus",
                    "window": {
                        "used_pct": null, "resets_at": 1_900_000_000u64, "age_secs": 42,
                        "expired": false
                    }
                }],
                "plan": "pro",
                "in_flight": true,
                "state": {
                    "kind": "Failed",
                    "next_attempt_in_secs": 840,
                    "detail": { "kind": "rpc_error", "code": -32603, "upstream": "internal error" }
                },
                "revision": 7
            })
        );
    }

    /// 상태 여섯이 코드 + 상대 초 + 원인만으로 왕복한다 — 비정상 다섯은 `detail` 있음·없음을, 칸이 있는
    /// 둘은 값·`null` 을 다 태운다. 태그 `kind`(상태)와 `detail.kind`(분류)가 한 JSON 안에 함께 산다.
    #[test]
    fn every_usage_vendor_state_roundtrips_inside_a_snapshot() {
        let full = || Some(detail(Some(429), Some("slow down")));
        let bare = || {
            Some(UsageStateDetail {
                kind: "timeout".to_string(),
                code: None,
                upstream: None,
            })
        };
        let full_json =
            serde_json::json!({ "kind": "rpc_error", "code": 429, "upstream": "slow down" });
        let bare_json = serde_json::json!({ "kind": "timeout", "code": null, "upstream": null });
        let cases = [
            (
                UsageVendorState::Ready,
                serde_json::json!({ "kind": "Ready" }),
            ),
            (
                UsageVendorState::NotInstalled { detail: None },
                serde_json::json!({ "kind": "NotInstalled", "detail": null }),
            ),
            (
                UsageVendorState::NotInstalled { detail: bare() },
                serde_json::json!({ "kind": "NotInstalled", "detail": bare_json }),
            ),
            (
                UsageVendorState::NeedsLogin { detail: None },
                serde_json::json!({ "kind": "NeedsLogin", "detail": null }),
            ),
            (
                UsageVendorState::NeedsLogin { detail: full() },
                serde_json::json!({ "kind": "NeedsLogin", "detail": full_json }),
            ),
            (
                UsageVendorState::Unavailable { detail: None },
                serde_json::json!({ "kind": "Unavailable", "detail": null }),
            ),
            (
                UsageVendorState::Unavailable { detail: bare() },
                serde_json::json!({ "kind": "Unavailable", "detail": bare_json }),
            ),
            (
                UsageVendorState::Failed {
                    next_attempt_in_secs: 0,
                    detail: None,
                },
                serde_json::json!({ "kind": "Failed", "next_attempt_in_secs": 0, "detail": null }),
            ),
            (
                UsageVendorState::Failed {
                    next_attempt_in_secs: 90,
                    detail: full(),
                },
                serde_json::json!({
                    "kind": "Failed", "next_attempt_in_secs": 90, "detail": full_json
                }),
            ),
            (
                UsageVendorState::Rejected {
                    retry_in_secs: 600,
                    detail: None,
                },
                serde_json::json!({ "kind": "Rejected", "retry_in_secs": 600, "detail": null }),
            ),
            (
                UsageVendorState::Rejected {
                    retry_in_secs: 600,
                    detail: full(),
                },
                serde_json::json!({ "kind": "Rejected", "retry_in_secs": 600, "detail": full_json }),
            ),
        ];
        for (state, want) in cases {
            let snap = usage_snapshot(state);
            let json = serde_json::to_value(&snap).expect("직렬화");
            assert_eq!(json["state"], want);
            assert_eq!(
                serde_json::from_value::<UsageLimitSnapshot>(json).expect("역직렬화"),
                snap
            );
        }
    }

    /// `detail` 키가 없는 패킷(이 칸 전의 데몬)은 `None` 으로 읽힌다 — 상태 하나 때문에 스냅숏 전체가
    /// 역직렬화 실패로 무너지지 않게.
    #[test]
    fn usage_vendor_state_without_detail_key_reads_as_none() {
        let cases = [
            (
                r#"{ "kind": "NotInstalled" }"#,
                UsageVendorState::NotInstalled { detail: None },
            ),
            (
                r#"{ "kind": "NeedsLogin" }"#,
                UsageVendorState::NeedsLogin { detail: None },
            ),
            (
                r#"{ "kind": "Unavailable" }"#,
                UsageVendorState::Unavailable { detail: None },
            ),
            (
                r#"{ "kind": "Failed", "next_attempt_in_secs": 5 }"#,
                UsageVendorState::Failed {
                    next_attempt_in_secs: 5,
                    detail: None,
                },
            ),
            (
                r#"{ "kind": "Rejected", "retry_in_secs": 7 }"#,
                UsageVendorState::Rejected {
                    retry_in_secs: 7,
                    detail: None,
                },
            ),
        ];
        for (raw, want) in cases {
            assert_eq!(
                serde_json::from_str::<UsageVendorState>(raw).expect(raw),
                want
            );
        }
    }

    /// `{:?}` 는 원문을 안 낸다 — 글자 수(바이트 아님)만 낸다. wire·비교는 원문 그대로다.
    #[test]
    fn usage_state_detail_debug_hides_upstream_text() {
        let secret = "token=sk-비밀값";
        let d = detail(Some(1), Some(secret));
        let dbg = format!("{d:?}");
        assert!(!dbg.contains("sk-") && !dbg.contains("비밀"), "{dbg}");
        assert!(
            dbg.contains(&format!("<{}자>", secret.chars().count())),
            "{dbg}"
        );
        assert!(
            dbg.contains("rpc_error") && dbg.contains("Some(1)"),
            "{dbg}"
        );
        let nested = format!(
            "{:#?}",
            UsageVendorState::NeedsLogin {
                detail: Some(d.clone())
            }
        );
        assert!(!nested.contains("sk-"), "{nested}");
        assert_eq!(
            serde_json::to_value(&d).expect("직렬화")["upstream"],
            secret
        );
        assert_eq!(
            format!("{:?}", detail(None, None)),
            r#"UsageStateDetail { kind: "rpc_error", code: None, upstream: None }"#
        );
    }

    /// 프론트가 받는 TS 모양 — u64·i64 칸이 `bigint` 로 새면 JSON number 를 받는 코드와 타입이 갈린다.
    #[test]
    fn usage_ts_shapes_use_number_for_every_time_field() {
        let decls = [
            UsageLimitSnapshot::decl(),
            UsageWindow::decl(),
            UsageScopedWindow::decl(),
            UsageVendorState::decl(),
            UsageStateDetail::decl(),
        ];
        for decl in &decls {
            assert!(!decl.contains("bigint"), "{decl}");
        }
        let snap = UsageLimitSnapshot::decl();
        assert!(snap.contains("revision: number,"), "{snap}");
        // `served` 는 데몬 안에만 산다 — wire 모양에 칸도 타입 참조도 없어야 한다.
        assert!(
            !snap.contains("served") && !snap.contains("UsageServed"),
            "{snap}"
        );
        let window = UsageWindow::decl();
        // 값의 출처(줍기·능동 조회)는 사용자에게 보이지 않는다 — wire 창에 칸이 없어야 한다.
        assert!(!window.contains("source"), "{window}");
        assert!(window.contains("resets_at: number | null"), "{window}");
        assert!(window.contains("age_secs: number,"), "{window}");
        assert_eq!(
            UsageStateDetail::inline(),
            "{ kind: string, code: number | null, upstream: string | null, }"
        );
        // 상태는 전량을 잰다 — 문구(`string`) 칸은 `UsageStateDetail` 하나만 허용하고 여기선 이름으로만
        // 보인다. 상태 변형에 맨 `string` 칸이 끼어들면 여기서 깨진다.
        assert_eq!(
            UsageVendorState::inline(),
            concat!(
                r#"{ "kind": "Ready" } | "#,
                r#"{ "kind": "NotInstalled", detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "NeedsLogin", detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "Unavailable", detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "Failed", next_attempt_in_secs: number, "#,
                r#"detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "Rejected", retry_in_secs: number, detail: UsageStateDetail | null, }"#,
            )
        );
    }
}
