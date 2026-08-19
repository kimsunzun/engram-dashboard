//! 계열을 가리지 않는 제어 입구 둘 — **발견**(`/control/commands`)과 **전체 이름 호출**(`/control/call`).
//!
//! ★역할★: 이웃 `agent.rs` 가 `agent` 한 계열의 어휘(`{verb}`)를 지는 어댑터라면, 여기는 **이름을 그대로
//!   받는** 어댑터다. 동사 표도 계열 지식도 없고, 검문·실행은 **1단계 포트**를 거쳐 이웃 `commands` 의 공통
//!   입구(`call_daemon_command`)가 그대로 한다(그 포트를 쓰는 이유 = [`handle_call`] 의 그 문단) — 이 파일이
//!   더하는 것은 셋이다: (a) 두 출처를 한 목록으로 접는 병합 규칙 (b) 이 표면의 어휘로 쓰는 회복 안내
//!   (c) 표의 결말 → wire 봉투.
//!
//! ★`help` 는 열어보지 않는다(하드 제약)★: 병합도 목록도 그 문자열을 **바이트 그대로** 나른다 —
//!   파싱·검증·분기 금지(ADR-0156). 데몬은 그 안에 무엇이 들었는지 알 필요가 없고, 알면 주인이 모양을
//!   바꿀 때마다 데몬이 함께 깨진다.
//!
//! ★목록의 도달성 칸(`callable`)과 [`handle_call`] 의 결말은 **같은 사실**에서 나와야 한다★: 그 칸이
//!   기계가 분기할 수 있는 유일한 자리라, 갈리는 순간 발견은 부를 수 없는 이름을 부를 수 있다고
//!   가르치거나 그 반대가 된다. 그 사실이 무엇인지는 [`handle_list`].
//!
//! ★남의 이름은 **버스로 넘긴다**★(ADR-0160): 데몬 자기 표에 없고 주인이 붙어 있으면 [`CallOutcome::Relay`]
//!   가 되어 부르는 쪽이 배달 기계에 태운다. 여기서 배달을 하지 않는 이유는 이 파일이 blocking 이라서다 —
//!   왕복은 async 다.
//!
//! tauri import 0(daemon crate).
// ADR-0155
// ADR-0156
// ADR-0160

use std::collections::BTreeSet;

use engram_dashboard_command::{
    CommandDecl, CommandError, CommandReply, ErrorCode, OwnerLookup, OwnerLookupSource, RequestId,
    RosterEntry,
};
use engram_dashboard_protocol::CommandListEntry;

use super::agent::{preview, preview_within, CommandArgs};
use super::ingress::ControlQueryResult;
use crate::command_delivery::LocalCommands;

/// 발견 목록의 병합 규칙 — ★이 규칙이 사는 자리는 여기 하나다★.
///
/// 두 표면(WS 의 `ListCommands` 와 HTTP 의 `/control/commands`)이 이 함수를 부른다. 사본을 두면 두
/// 목록이 갈리고, 갈린 순간 발견은 **어느 한쪽에게 거짓말**이 된다.
///
/// 규칙 셋:
/// - `mine`(데몬 자기 표)과 `registered`(붙어 있는 주인들의 명부)를 합친다. 자기 표를 빼면 `agent.*` 는
///   배달되는데 발견에는 안 보여, 부를 수 있는 이름을 물어본 LLM 이 그 목록만 믿고 영영 안 부른다.
/// - 같은 이름이 양쪽에 있으면 **데몬 것이 이긴다** — 배달이 그렇게 정해져 있기 때문이다(내 표가 1단계,
///   명부가 2단계 — `command_delivery::deliver`). 목록이 반대로 말하면 호출자는 남의 `help` 로 인자를
///   맞춘 뒤 데몬 핸들러에게 반려당한다.
/// - 두 출처를 이어 붙였으므로 마지막에 이름순으로 한 번 정렬한다(각각은 이름순인데 합치면 아니다).
///
/// ★이 dedup 은 **오늘 도달 불가한 상태**를 위한 것이다 — 그래도 지운다는 뜻이 아니다★: 겹침을 만들려면
///   데몬 표가 비어 있는 사이에 등록이 도착해야 하는데(`refuse_names_i_answer` 는 그 순간의 표를 본다),
///   조립된 서버 둘은 **연결을 받기 전에** 슬롯을 채운다. 표를 늦게 꽂는 조립이 **불가능하지는** 않고
///   (슬롯은 정의상 늦은 주입이다) 그때 이 자리가 유일한 그물이다. 이 판정은 두 출처를 합치는 함수의
///   **자기 정합성**이지 특정 조립 순서에 기댄 것이 아니다.
/// ★반환 타입은 **WS 투영**이다 — 그 `available` 은 WS 의 뜻(주인이 붙어 있다)으로만 참이다★: 주인이
///   끊기면 그 이름이 명부에서 사라지므로(ADR-0150 결정 3) 실려 오는 항목은 전부 살아 있는 등록이다(그
///   칸의 정본 주석 = `CommandListEntry`). **HTTP 목록은 그 칸을 다시 내보내지 않는다** — 그쪽이 말하는
///   것은 「이 입구가 실행할 수 있나」라 재료가 하나 더 있고(데몬 자기 표), 같은 이름의 칸을 두 뜻으로
///   재사용하면 그 칸으로 분기한 호출자가 어느 뜻인지 못 가린다([`handle_list`]).
pub fn merge(mine: Vec<CommandDecl>, registered: Vec<RosterEntry>) -> Vec<CommandListEntry> {
    let mine: Vec<CommandListEntry> = mine
        .into_iter()
        .map(|decl| CommandListEntry {
            name: decl.name,
            help: decl.help,
            available: true,
        })
        .collect();
    let mut entries: Vec<CommandListEntry> = registered
        .into_iter()
        .filter(|entry| !mine.iter().any(|held| held.name == entry.name))
        .map(|entry| CommandListEntry {
            name: entry.name,
            help: entry.help,
            available: true,
        })
        .collect();
    entries.extend(mine);
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// 발견 응답 — `{ "commands": [ { "name", "help", "callable" }, … ] }`.
///
/// ★`callable` = **이 입구가 지금 그 이름을 실행할 수 있다**★(계약): 참이면 `/control/call` 이 그 이름을
///   실제로 돈다 — 데몬 자기 표의 이름은 그 자리에서 돌고, 남의 이름은 그 주인에게 중계된다(ADR-0160).
///   ★도달성을 말하는 칸은 이것 하나여야 한다★ — WS 투영의 `available` 을 여기 다시 실으면 같은 이름의
///   칸이 표면마다 다른 뜻을 갖고, 그 칸으로 분기한 호출자는 부를 수 없는 이름을 부를 수 있다고 읽는다.
/// ★판정 재료는 [`handle_call`] 과 **같은 사실 둘**이다★ — 그 이름이 데몬 자기 표에 있거나(여기서 `mine`,
///   저기서 `CommandTable::contains` — 같은 표의 같은 명단이다) 명부가 주인을 답하거나(여기서
///   `registered`, 저기서 `OwnerLookupSource::lookup`). 재료를 갈라 두면 목록과 실행이 서로 다른 답을 한다.
/// ★그래서 오늘 이 칸은 [`merge`] 가 낸 모든 행에서 참이다 — 그렇다고 상수로 접지 말 것★: 참인 이유는
///   합치는 두 출처가 **둘 다 도달 가능해서**이지 이 칸이 뜻을 잃어서가 아니다. 세 번째 출처(주인 없이
///   선언만 아는 이름 따위)가 [`merge`] 에 들어오는 날, 파생해 두었으면 그 행만 거짓이 되고 상수로
///   박아 두었으면 발견이 없는 도달성을 광고한다.
///
/// ★인자가 값인 이유(슬롯·핸들이 아니라)★: 두 출처를 **어디서 읽었는지**를 이 함수가 모르게 해야 WS 쪽과
///   같은 [`merge`] 를 태울 수 있다. 읽는 시점은 부르는 쪽이 정한다.
/// ★반쪽 목록도 내보낸다(표 슬롯이 비어 있으면 `mine` 이 빈 벡터)★ — 그것이 같은 상태에서 WS 목록이
///   내는 답이고, 두 표면이 **같은 상태에서 다른 답**을 내지 않는 것이 이 라우트의 요점이다. 조립된 서버
///   둘은 연결을 받기 전에 표를 꽂으므로 운영에서 이 갈래는 나지 않는다.
pub fn handle_list(mine: Vec<CommandDecl>, registered: Vec<RosterEntry>) -> ControlQueryResult {
    // 이름만 뜬다 — `help` 는 복사하지 않는다(이 판정에 필요 없고, 그 블롭이 이 목록에서 가장 큰 것이다).
    let callable: BTreeSet<String> = mine
        .iter()
        .map(|decl| decl.name.clone())
        .chain(registered.iter().map(|entry| entry.name.clone()))
        .collect();
    // ★`json!` 을 쓰지 않는 이유는 **복사**다★: 그 매크로는 리터럴이 아닌 자리를 `to_value(&expr)` 로 펼치고
    //   (`serialize_str` 이 `to_owned` 한다) `help` 블롭이 행마다 한 번 더 복제된다 — 명부 상한(4096 × 4 KiB)
    //   에서 요청마다 나는 그 사본이 이 응답의 가장 큰 비용이다. 손으로 조립하면 아래 String 은 **옮겨진다**.
    //   ★덤으로 실패 경로가 사라진다★: `to_value` 를 안 거치므로 `json!` 이 숨기는 `.unwrap()` 자체가 없다
    //   (릴리스는 `panic = "abort"` 라 그 패닉은 500 이 아니라 데몬 종료다).
    let commands: Vec<serde_json::Value> = merge(mine, registered)
        .into_iter()
        .map(|entry| {
            let callable = callable.contains(&entry.name);
            let mut row = serde_json::Map::with_capacity(3);
            row.insert("name".to_string(), serde_json::Value::String(entry.name));
            row.insert("help".to_string(), serde_json::Value::String(entry.help));
            row.insert("callable".to_string(), serde_json::Value::Bool(callable));
            serde_json::Value::Object(row)
        })
        .collect();
    let mut root = serde_json::Map::with_capacity(1);
    root.insert("commands".to_string(), serde_json::Value::Array(commands));
    ControlQueryResult::Ok(serde_json::Value::Object(root))
}

/// `/control/call` 요청 바디 — `{ "name": "<전체 이름>", "args": { … } }`.
///
/// ★인자를 `args` 안에 넣는다(꼭대기에 펼치지 않는다)★: 이웃 `/control/agent` 는 `verb` 만 자기 어휘라
///   나머지를 통째로 인자로 펼치지만, 여기 꼭대기 어휘는 `name` 이고 **`name` 은 실제 인자 이름이기도
///   하다**(`agent.rename` 이 그 칸을 쓴다). 펼치면 그 동사를 이 입구로 부를 수 없다.
/// ★`args` 부재 = 인자 없음★ — 인자가 하나도 없는 명령(`agent.list`)이 빈 객체를 실을 의무를 지지 않는다.
/// ★중복 키는 꼭대기에서도 `args` 안에서도 반려된다★ — 어느 값을 고를 근거가 없으므로 고르지 않는다
///   (ADR-0157). 안쪽 규율의 정본은 [`CommandArgs`].
/// ★모르는 꼭대기 칸은 **무시한다**(`args` 안과 규율이 다르다)★: 홉 간 additive 진화를 살리려면 이 봉투에
///   칸이 하나 늘어도 옛 데몬이 받아야 한다. 인자를 `args` 에 안 넣고 꼭대기에 실은 요청은 **조용히 통과하지
///   않는다** — 그 명령의 필수 칸이 비어 `INVALID_ARGUMENT` 로 선언된 칸 전량과 함께 반려된다.
/// ★`request_id` 는 **선택**이다 — 있으면 호출자 것을 쓰고 없으면 데몬이 낸다★(ADR-0161 결정 1).
///
/// 이 칸이 사는 이유는 하나다: 배달의 중복 방지는 **같은 번호가 다시 왔을 때만** 걸리므로, 데몬이 요청마다
/// 새 번호를 만들면 그 장치는 구조적으로 한 번도 안 걸린다. 재시도가 같은 번호를 되보내야 걸릴 기회가 생긴다.
///
/// ## ★오늘 이 칸이 실제로 사는 보장 = **왕복이 열려 있는 동안뿐**이다(알려진 갭)★
///
/// 이 문으로 온 번호가 지금 막는 것은 「진행 중인 왕복과 겹친 재질의」뿐이다 — 그 갈래는 자리 표가
/// [`Seat::Taken`](crate::command_delivery)·[`Seat::Conflict`](crate::command_delivery) 로 반려하거나
/// 합친다. **왕복이 끝난 뒤에 온 재질의는 다시 적용된다.** 사유가 둘이고 둘 다 이 문에 걸린다:
///
/// 1. **중계된 이름**(대시보드 것)은 답장 프레임이 나가는 순간 자리를 놓는다
///    ([`CommandDeliveries::release`](crate::command_delivery)) — 번호를 붙드는
///    [`SeatState::Retained`](crate::command_delivery) 는 데몬 **자기** 1단계 자리에만 붙는다.
/// 2. **데몬 자기 이름**(`agent.*`)은 [`handle_call`] 이 그 자리에서 답하므로 자리 표를 아예 안 거친다 —
///    즉 그 보유 장치에 닿을 경로가 이 입구에는 없다.
///
/// ★그래서 「완료 후 재질의가 `ALREADY_APPLIED` 로 막힌다」로 적지 말 것★ — 이 입구에서는 거짓이다.
/// 완료분을 되돌려주는 진짜 dedup 저장소가 없는 것이 그 뿌리이고(그 부재의 정본 = `command_delivery` 의
/// `no_retry`), 이 문의 좌석을 완료 뒤에도 붙들게 할지는 **미결**이다(사용자 결정 대기).
///
/// ★없는 요청도 계속 받는다★ — 하위 호환이고, 그 경로는 위 in-flight 보장조차 안 걸린다(번호가 매번
/// 새것이라 겹칠 것이 없다 — ADR-0161 영향 절).
/// ★봉투가 아니라 이 HTTP 바디만 늘어난다★ — ADR-0160 「봉투 무변경」이 그대로 유효하다.
#[derive(Debug, Default)]
pub struct CallRequest {
    /// ★필수인데 부재를 허용하는 이유는 이웃과 같다★ — 없으면 역직렬화가 실패해 **사유 없는 반려**로
    ///   끝난다. 빈 문자열로 받아 아래 dispatch 가 「모르는 명령」을 내면 호출자가 발견 목록으로 갈 수 있다.
    pub name: String,
    pub args: CommandArgs,
    /// 호출자가 실어 보낸 요청 번호의 **원문**. `None` = 안 실었다.
    ///
    /// ★문자열로 받아 [`handle_call`] 에서 판정한다 — 역직렬화 단계에서 파싱하지 않는다★: 여기서 실패하면
    /// 바디 전체가 [`malformed_body`] 로 접혀 「JSON 모양이 틀렸다」가 되는데, 틀린 것은 **이 칸 하나**다.
    /// 호출자는 그 문구를 읽고 멀쩡한 봉투 모양을 의심한다.
    pub request_id: Option<String>,
}

impl<'de> serde::Deserialize<'de> for CallRequest {
    /// ★객체만 받는다 — `#[derive(Deserialize)]` 로 되돌리지 마라★: 파생 구현은 **JSON 배열**도 필드
    /// 순서열로 읽어들여, `["agent.list"]` 가 `agent.list` **실행**이 된다(실측 — 이 자리를 손으로 쓰게 만든
    /// 사고 그대로다). 이웃 `AgentRequest` 가 그 함정을 안 밟는 것은 `#[serde(flatten)]` 이 map 전용
    /// 역직렬화를 강제하기 때문이라, 그 형태를 안 쓰는 여기서는 벽을 직접 세워야 한다.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CallRequest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object like {\"name\":\"agent.list\",\"args\":{}}")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<CallRequest, M::Error> {
                let mut name: Option<String> = None;
                let mut args: Option<CommandArgs> = None;
                let mut request_id: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => {
                            if name.is_some() {
                                return Err(serde::de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        "args" => {
                            if args.is_some() {
                                return Err(serde::de::Error::duplicate_field("args"));
                            }
                            args = Some(map.next_value()?);
                        }
                        CALLER_REQUEST_ID_FIELD => {
                            if request_id.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    CALLER_REQUEST_ID_FIELD,
                                ));
                            }
                            request_id = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(CallRequest {
                    name: name.unwrap_or_default(),
                    args: args.unwrap_or_default(),
                    request_id,
                })
            }
        }

        de.deserialize_map(Visitor)
    }
}

/// 전체 이름 호출을 이 자리에서 끝냈나, 아니면 버스로 넘겨야 하나.
///
/// ★두 갈래를 한 타입으로 가르는 이유★: 넘기는 갈래는 **async 왕복**이라 이 blocking 함수 안에서 끝낼 수
/// 없다. 산문으로 「빈손이면 넘겨라」를 남기면 넘기지 않는 호출자가 표현 가능하고, 그때 증상은 대시보드
/// 명령이 조용히 다시 `UNSUPPORTED` 가 되는 것이다.
// ADR-0160
#[derive(Debug)]
pub enum CallOutcome {
    /// 이 데몬이 답했다(실행했거나 반려했거나 그런 이름을 모른다) — 그대로 나간다.
    Answered(ControlQueryResult),
    /// 남의 이름이고 주인이 붙어 있다 — 부르는 쪽이 배달 기계에 태운다.
    ///
    /// ★인자를 그대로 넘긴다★: 이 데몬의 표에 없는 이름은 대조할 선언이 없어 검문받지 않고(그게 옳다 —
    /// 홉 간 additive 진화가 그 관용에 걸려 있다) 손도 대지 않는다(`call_daemon_command` 의 미스 계약).
    Relay {
        name: String,
        args: serde_json::Value,
        /// 이 왕복의 번호 — 호출자가 실어 보낸 것이거나(ADR-0161 결정 1) 여기서 발급한 것이다.
        ///
        /// ★발급을 어댑터로 미루지 않는 이유★: 미루면 「호출자 것을 쓴다」와 「데몬이 낸다」의 갈림이
        /// 판정부 밖에 남아, 검증을 통과한 값이 쓰이는지 어댑터를 읽어야 알 수 있다.
        request_id: RequestId,
    },
}

/// 호출자가 자기 요청 번호를 싣는 칸의 이름 — 역직렬화와 안내 문구가 같은 철자를 써야 한다.
const CALLER_REQUEST_ID_FIELD: &str = "request_id";

/// 호출자가 준 번호의 원문 → [`RequestId`].
///
/// ★UUID 문법을 요구하는 것이 곧 길이·문자셋 검증이다★(ADR-0161 결정 5): [`RequestId`] 가 UUID 한 개를
/// 감싼 타입이라(도구 crate `envelope.rs`) 다른 문법을 받아들이려면 **봉투의 타입**을 바꿔야 하고, 그건
/// ADR-0160 「봉투 무변경」이 막는다. 그래서 바깥에서 온 임의 길이 문자열이 자리 표의 키가 되는 길이 없다.
/// ★단 받는 모양이 하이픈 36자 하나는 아니다★ — `Uuid::parse_str` 는 hyphenated 말고 simple(32 hex)·
/// braced(`{…}`)·urn(`urn:uuid:…`) 도 받는다. 그래도 위 문장이 서는 이유는 그 넷이 **전부 유계**라서다
/// (가장 긴 urn 형이 45자). 안내 문구는 그 사실대로 적는다 — 「36자」로 적으면 통과하는 값을 두고 호출자가
/// 자기 봉투를 의심한다.
/// ★값의 **내용**은 검사하지 않는다★ — nil 이나 v4 아닌 값도 통과한다. 그 사실에 기대는 자리가 없어야 한다
/// (그 전제의 정본 = `command_delivery` 의 `CommandDeliveries::complete`).
/// ★반려 문구에 호출자 원문을 [`preview`] 로 줄여 싣는다★ — 그 값이 요청만큼 클 수 있다.
fn caller_request_id(raw: &str) -> Result<RequestId, ControlQueryResult> {
    uuid::Uuid::parse_str(raw).map(RequestId).map_err(|_| {
        ControlQueryResult::Error {
            code: ErrorCode::InvalidArgument.as_str(),
            hint: format!(
                "'{CALLER_REQUEST_ID_FIELD}' must be a UUID — hyphenated (8-4-4-4-12), 32 hex digits with no dashes, braced, or urn:uuid: form — so a retry can reuse it; got: {}",
                preview(raw)
            ),
        }
    })
}

/// 전체 이름 호출 — HTTP 어댑터가 유일하게 부르는 진입점.
///
/// ★blocking 함수다(호출자 계약)★: 표의 핸들러는 전부 blocking 이다(이웃 `commands` 의 공통 입구 doc) —
///   그래서 async 런타임 스레드가 아니라 blocking 풀에서 불러야 한다.
/// ★`registered` 는 **미스 갈래에서만** 읽는다★: 이름이 이 표의 것이면 명부를 볼 이유가 없고, 명부 조회는
///   공유 잠금을 건드린다. 잠금은 그 호출 안에서 끝난다(ADR-0006 — [`OwnerLookupSource`] 의 존재 이유).
/// ★조회와 배달 사이에 주인이 끊길 수 있다 — 그 경합은 여기서 못 닫는다★: 배달이 그 상태를 자기 어휘로
///   답한다(`route` 의 명부 재조회 · `OwnerLink::send` 의 찢어진 창). 여기서 잠금을 배달까지 들고 가면
///   느린 명령 하나가 등록·연결 정리·다른 배달을 전부 세운다.
/// ★호출자 번호는 **무엇을 하기 전에** 판정한다★: 뒤로 미루면 같은 잘못된 값이 데몬 자기 이름에는 통하고
///   남의 이름에만 반려되어, 이 입구의 계약이 이름에 따라 갈린다.
///
/// ## ★1단계 표를 **버스가 든 그 한 부**로 받는다 — `&CommandTable` 로 되돌리지 말 것★
///
/// 이 함수의 판정(내가 답하나 / 중계하나)과 배달 기계의 판정([`LocalCommands::claim`] — 자리의 `local`
/// 표식과 `CommandBus::invoke` 의 중계 거절이 그 값을 쓴다)은 **같은 사실**이어야 한다. 표를 인자로 따로
/// 받으면 어댑터가 버스와 **다른 부**를 넘기는 조립이 표현 가능해지고, 그 상태의 증상이 조용하다: 이 함수는
/// 「내 이름이 아니다」라며 중계로 보내는데 버스는 그 이름을 자기 것이라며 `INTERNAL` 로 반려한다(또는 그
/// 반대로, 거절이 조용한 no-op 이 되어 취소 한 번에 자리가 영구히 남는 갈래가 열린다). 포트로 받으면 어댑터가
/// 넘길 수 있는 것이 **버스의 그 한 부**뿐이라 그 어긋남이 표현되지 않는다.
/// ★그래서 검문·실행도 이 포트를 거친다★ — 실물 구현이 이웃 `commands` 의 공통 입구로 몰기 때문에
/// (`commands::DaemonLocalCommands::run`) ADR-0157 검문은 그대로 걸린다. [`CommandTable::call`] 을 여기서
/// 직접 부르는 두 번째 경로를 만들지 말 것.
// ADR-0157
// ADR-0160
pub fn handle_call(
    locals: &dyn LocalCommands,
    registered: &dyn OwnerLookupSource,
    req: CallRequest,
) -> CallOutcome {
    let CallRequest {
        name,
        args,
        request_id,
    } = req;
    let mut args = args.into_value();
    let request_id = match request_id.as_deref().map(caller_request_id) {
        Some(Err(refusal)) => return CallOutcome::Answered(refusal),
        Some(Ok(id)) => id,
        None => RequestId::new(),
    };

    // ★라우팅 권위는 `claim` 하나다★([`LocalCommands::claim`] doc) — 그 값이 자리의 `local` 표식을 정하므로,
    //   여기서 다른 술어로 갈래를 고르면 이 함수와 배달이 서로 다른 답을 한다.
    if locals.claim(&name).is_none() {
        return match registered.lookup(&name) {
            OwnerLookup::Available(_) => CallOutcome::Relay {
                name,
                args,
                request_id,
            },
            OwnerLookup::Unknown => CallOutcome::Answered(unknown_name(&name)),
        };
    }
    CallOutcome::Answered(match locals.run(&name, &mut args, "cli") {
        Some(Ok(payload)) => ControlQueryResult::Ok(payload),
        Some(Err(e)) => refused(e),
        // ★계약 위반이라 **다음 단계로 흘리지 않는다**★: `claim` 이 내 것이라 답했으므로 이 이름은 중계
        //   대상이 아니고, 중계로 떨어뜨리면 버스가 같은 근거로 그것을 다시 거절한다(무한한 것은 아니지만
        //   호출자에게는 무의미한 왕복이다). 배달이 같은 어긋남을 같은 코드로 드러낸다
        //   (`command_delivery` 의 `fold_local`).
        None => {
            tracing::error!(
                entrance = "cli",
                command = %crate::connection_core::sanitize_for_log(&name),
                "1단계 표가 자기 이름이라 해 놓고 빈손을 냈다 — 라우팅 권위(`claim`)와 실행이 갈렸다"
            );
            refused(CommandError::of(
                ErrorCode::Internal,
                format!(
                    "this daemon claims '{}' but its own table produced no answer for it",
                    preview(&name)
                ),
            ))
        }
    })
}

/// 배달이 돌려준 결말 → wire 봉투.
///
/// ★성공 payload 는 주인이 만든 것이라 **열어보지 않는다**★ — 이 데몬은 `args` 도 결과도 파싱하지 않는다
/// (ADR-0081 「데몬 opaque」). 실패는 이 표면의 다른 반려와 **같은 함수**를 탄다([`refused`]) — 갈라 두면
/// 같은 코드가 입구 안에서 자리마다 다른 문구·다른 회복 안내를 달고 나간다.
// ADR-0160
pub fn relayed(reply: CommandReply) -> ControlQueryResult {
    match reply.outcome {
        Ok(payload) => ControlQueryResult::Ok(payload),
        Err(e) => refused(e),
    }
}

/// 표의 실패 → wire 봉투. 코드는 타입드 어휘 그대로 나간다(TRD §4-⑦).
///
/// ★회복 안내는 **고칠 수 있는 실패에만** 붙인다★: 대상 부재·상태 충돌·내부 실패에 「목록을 보라」를 달면
///   호출자는 없는 오타를 찾는다. 이 입구는 계열을 모르므로 계열별 조회 동사를 제안할 수 없고, 그래서
///   나머지 코드에는 표가 준 문구만 싣는다(지어낸 경로보다 침묵이 정직하다).
/// ★문구는 **양 끝을 남기고** 줄인다★: 표의 인자 반려는 호출자가 친 것을 앞에, 선언된 칸 전량을 뒤에
///   두므로 머리만 남기면 고칠 재료가 사라진다(그 레이아웃과 상한 선택의 근거 = [`MESSAGE_PREVIEW_CHARS`]).
fn refused(e: CommandError) -> ControlQueryResult {
    let message = preview_within(e.message(), MESSAGE_PREVIEW_CHARS);
    let hint = match e.code() {
        ErrorCode::InvalidArgument | ErrorCode::UnknownCommand => {
            format!("{message} — {CATALOG_RECOVERY}.")
        }
        _ => message,
    };
    ControlQueryResult::Error {
        code: e.code().as_str(),
        hint,
    }
}

/// 표가 준 반려 문구 **하나 전체**가 차지해도 되는 몫(문자 수).
///
/// ★설계된 최악을 **안 자르는** 값이어야 한다★: 인자 검문의 최악 문구는 도구 crate 가 이미 캡한다 — 이름
/// 하나가 128 B(`quoted_input`), 개수가 여덟(`MAX_NAMED_UNKNOWN`)이라 실측 최악이 약 1.4 KB다. 그 아래로
/// 잡으면 **설계된 경우에** 가운데가 잘려 나가고, 그건 상한이 지키려던 것과 무관한 손실이다.
/// ★그래도 상한을 두는 이유★: 이 자리에 오는 문구 전부가 그렇게 캡되지는 않는다 —
/// `commands::drive_to_completion` 은 호출자가 보낸 명령 이름을 원문 그대로 넣는다. 이 수는 그 갈래의
/// **꼬리를 유한하게** 만드는 것이지 평균 크기를 줄이는 장치가 아니다.
/// 아래 수는 그 1.4 KB 위의 여유이지 계약이 아니다.
const MESSAGE_PREVIEW_CHARS: usize = 2048;

/// 이 입구가 아는 **유일한** 회복 경로 — 부를 수 있는 이름과 그 칸을 아는 것은 발견 목록뿐이다.
///
/// ★CLI 동사를 지어내지 않는다★: 데몬이 소유한 것은 **라우트**이지 CLI 철자가 아니고, 이 목록을 렌더할
///   CLI 동사는 아직 없다. 그래서 우리가 가진 이름(그 라우트)만 적고 화면 이름은 CLI 가 자기 것을 붙인다.
/// ★`engram help` 로 되돌리지 마라★: 그 화면은 **정적**이라(bin/engram.rs `render_help`) 계열 이름 둘만
///   내고 칸은 하나도 안 낸다 — `agent.*` 밖의 이름은 원리상 거기 나올 수 없다. 즉 이 안내가 존재하는
///   바로 그 경우(모르는 이름 · 틀린 칸)를 답할 수 없는 곳으로 보내는 것이 된다.
/// ★ADR-0132 결정 4(발견은 help 로만)를 이 자리의 근거로 쓰지 말 것★ — 그 결정은 명령을 나열하는 대신
///   **정적 계열 화면**을 고른 것이고, 여기가 가리키는 것은 런타임 목록이라 방향이 반대다.
/// ★경로 문자열은 라우트 상수와 갈리면 안 된다★ — 아래 테스트가 그 상수를 태워 대조한다.
// ADR-0156 (칸을 나르는 것은 불투명 `help` 블롭이고, 그 블롭을 내려 주는 표면이 이 목록이다)
const CATALOG_RECOVERY: &str =
    "list the command catalog (POST /control/commands) for every callable name and its fields";

/// 아무도 그 이름을 선언하지 않았다 — 데몬 표에도 없고 붙어 있는 주인도 없다.
///
/// ★「남의 이름이라 못 닿는다」(`UNSUPPORTED`)를 여기 되살리지 마라★: 그 갈래는 이제 배달로 간다
///   ([`CallOutcome::Relay`] · ADR-0160). 되살리면 방금 목록에서 `tab.create` 를 배운 호출자가 그 이름을
///   부를 수 없게 되고, 목록의 도달성 칸([`handle_list`])도 함께 거짓이 된다.
/// ★주인이 방금 끊긴 이름도 여기로 온다★ — 끊긴 주인의 이름은 명부에서 사라지므로(ADR-0150 결정 3)
///   「한 번도 없던 이름」과 구분할 재료가 이 데몬에 없다. 그 구분 손실은 그 결정이 감수한 것이다.
fn unknown_name(name: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::UnknownCommand.as_str(),
        hint: format!(
            "unknown command '{}' — neither this daemon nor any connected client declares that name; {CATALOG_RECOVERY}.",
            preview(name)
        ),
    }
}

/// 바디 자체가 JSON 계약을 못 지킨 경우의 반려 — 어댑터가 부른다.
///
/// ★빈 body 400 을 쓰지 않는 이유는 이웃과 같다★: 이 라우트의 호출자는 자기교정하는 LLM 이라 사유가
///   실려야 한다. serde 의 문구(어느 필드가 어떤 타입이어야 하는지·중복 키·JSON 구문 위치)를 그대로 싣고,
///   바디 원문은 [`preview`] 로 줄인다.
/// ★[`CATALOG_RECOVERY`] 를 여기 붙이지 마라★: 여기서 틀린 것은 **바디의 JSON 모양**이지 명령 이름도 칸도
///   아니다. 카탈로그를 보라고 하면 호출자는 멀쩡한 이름을 의심하며 목록을 뒤지고, 정작 고칠 봉투 모양은
///   그대로 둔 채 다시 보낸다. 고칠 재료는 이 문구가 이미 들고 있다 — serde 의 사유와 기대하는 모양.
pub fn malformed_body(reason: &str, raw: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "the request body is not a valid command call: {reason} — it must be a JSON object like {{\"name\":\"agent.list\",\"args\":{{}}}}; got: {}",
            preview(raw)
        ),
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_command::{Effect, OwnerToken, RequestId};

    use super::*;
    use crate::command_delivery::NoLocalCommands;

    /// 이름 **하나**를 자기 것이라 답하는 1단계 표 — 실물(`commands::DaemonLocalCommands`)의 성질만 흉내
    /// 낸다: `claim` 과 `run` 이 **같은 명단**을 본다(그 어댑터가 `OnceLock` 표 하나를 보므로 그렇다).
    ///
    /// ★실물을 끌어오지 않는 이유★: `agent.*` 표는 `AgentManager` 를 쥐고 그 뒤에 디스크·프로필 락이 딸려
    /// 온다(`commands::make_daemon_table`). 이 파일이 재는 것은 **판정**(내가 답하나 / 중계하나 / 모르나)이라
    /// 그 실물이 필요 없고, 끌어오면 이 시험이 재는 것이 판정에서 조립으로 옮겨간다(ADR-0012).
    struct Holds(&'static str);

    impl LocalCommands for Holds {
        fn claim(&self, name: &str) -> Option<Effect> {
            (name == self.0).then_some(Effect::Read)
        }

        fn run(
            &self,
            name: &str,
            _args: &mut serde_json::Value,
            _entrance: &'static str,
        ) -> Option<Result<serde_json::Value, CommandError>> {
            (name == self.0).then(|| Ok(serde_json::json!({ "ran": name })))
        }

        fn decls(&self) -> Vec<CommandDecl> {
            vec![decl(self.0, "{}")]
        }
    }

    fn decl(name: &str, help: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: help.to_string(),
        }
    }

    fn entry(name: &str, help: &str) -> RosterEntry {
        RosterEntry {
            name: name.to_string(),
            help: help.to_string(),
            owner: OwnerToken::new("conn-1"),
        }
    }

    fn names(entries: &[CommandListEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// ★안내가 가리키는 곳이 **실재하고, 그 질문에 답할 수 있는 곳**이어야 한다★: 상수는 포맷으로 조립할
    /// 수 없어 경로를 손으로 적었고, 그 손이 미끄러지면 없는 주소를 치게 한다. 라우트 상수를 태워 대조한다.
    #[test]
    fn the_recovery_points_at_the_route_that_can_actually_answer_it() {
        assert!(
            CATALOG_RECOVERY.contains(super::super::mcp_server::CONTROL_COMMANDS_PATH),
            "회복 안내가 발견 라우트와 갈렸다: {CATALOG_RECOVERY}"
        );
    }

    #[test]
    fn the_merge_sorts_both_sources_into_one_list_and_the_daemon_wins_a_clash() {
        let merged = merge(
            vec![decl("agent.list", "mine"), decl("agent.new", "mine")],
            vec![entry("tab.create", "theirs"), entry("agent.list", "stolen")],
        );

        assert_eq!(
            names(&merged),
            vec!["agent.list", "agent.new", "tab.create"]
        );
        assert_eq!(
            merged
                .iter()
                .find(|e| e.name == "agent.list")
                .map(|e| e.help.as_str()),
            Some("mine"),
            "겹치면 데몬 표의 help 가 남는다"
        );
    }

    /// ★`help` 는 불투명 문자열 한 칸이다★ — JSON 이 아닌 값도 바이트 그대로 나가야 한다. 데몬이
    /// 파싱·검증을 끼우면 여기서 깨진다.
    #[test]
    fn the_listing_carries_the_help_bytes_untouched() {
        let opaque = "not json at all — 임의의 바이트 {[(";
        let result = handle_list(vec![decl("agent.list", opaque)], vec![]);

        let json = result.to_json();
        assert_eq!(json["commands"][0]["name"], "agent.list");
        assert_eq!(json["commands"][0]["help"], opaque);
    }

    /// ★목록이 가르친 이름은 [`handle_call`] 이 전부 알아본다 — 그리고 그 밖의 이름은 모른다★.
    ///
    /// ★`callable == true` 만 단언하지 않는 이유★: 그 칸은 오늘 [`merge`] 의 모든 행에서 참이라(두 출처가
    /// 둘 다 도달 가능하다) 그 단언은 **상수를 단언한 것**과 구분되지 않는다. 실제로 지켜야 하는 성질은
    /// 「목록과 실행이 같은 두 사실에서 나온다」이고, 그것은 두 함수를 같은 입력에 태워야 관측된다.
    /// ★음성 대조를 함께 둔다★ — 없으면 아래 루프는 어떤 구현에서도 초록이다.
    #[test]
    fn every_name_the_listing_teaches_is_a_name_the_call_route_recognises() {
        let registered = Registered("tab.create");
        let json = handle_list(
            vec![decl("agent.list", "mine")],
            vec![entry("tab.create", "theirs")],
        )
        .to_json();
        let rows = json["commands"].as_array().expect("배열");
        assert_eq!(rows.len(), 2, "전제: 두 출처가 다 실렸다: {json}");

        // 1단계 표는 비워 둔다 — 여기서 재는 것은 **알아보나**이지 결말이 아니다.
        let table = NoLocalCommands;
        for row in rows {
            let name = row["name"].as_str().expect("이름");
            assert_eq!(row["callable"], true, "{json}");
            // `agent.list` 는 이 빈 표에 없으므로 명부로 떨어진다 — 그래서 이 루프가 재는 것은 명부 쪽
            //   절반이고, 표 쪽 절반은 실제 표를 든 통합 시험이 잰다.
            let known = !matches!(
                handle_call(&table, &registered, call_of(name)),
                CallOutcome::Answered(ControlQueryResult::Error { code, .. }) if code == "UNKNOWN_COMMAND"
            );
            assert!(
                known || name == "agent.list",
                "목록이 가르친 이름을 호출 판정이 모른다: {name}"
            );
        }

        assert!(
            matches!(
                handle_call(&table, &registered, call_of("nobody.declares.this")),
                CallOutcome::Answered(ControlQueryResult::Error { ref code, .. }) if code == &"UNKNOWN_COMMAND"
            ),
            "음성 대조 — 목록에 없는 이름은 모르는 명령이어야 한다"
        );

        // WS 투영의 칸은 이 표면에 새어 나오지 않는다 — 도달 규칙이 달라 뜻이 둘이 된다.
        assert!(
            rows.iter().all(|r| r["available"].is_null()),
            "WS 의 available 이 이 목록에 실렸다: {json}"
        );
    }

    /// ★호출자가 준 번호를 그대로 쓴다 — 그리고 문법에 안 맞으면 **그 칸을 지목해** 반려한다★
    /// (ADR-0161 결정 1·5).
    ///
    /// 번호를 여기서 새로 만들면 배달의 중복 방지가 구조적으로 안 걸린다 — 그 성질이 이 칸의 존재 이유
    /// 전부다. ★그 성질이 오늘 어디까지 서는지는 [`CallRequest::request_id`] 가 적는다(왕복이 열려 있는
    /// 동안뿐)★ — 이 시험이 재는 것은 **번호가 그대로 쓰이는가** 하나다.
    #[test]
    fn a_caller_supplied_request_id_is_used_as_is_and_a_malformed_one_names_that_field() {
        let table = NoLocalCommands;
        let registered = Registered("tab.create");
        let mine = RequestId::new();

        let req = CallRequest {
            name: "tab.create".to_string(),
            request_id: Some(mine.to_string()),
            ..CallRequest::default()
        };
        match handle_call(&table, &registered, req) {
            CallOutcome::Relay { request_id, .. } => {
                assert_eq!(request_id, mine, "호출자가 준 번호를 그대로 써야 한다")
            }
            other => panic!("중계여야: {other:?}"),
        }

        // 안 실으면 데몬이 낸다 — 하위 호환(그 경로는 중복 방지가 하나도 안 걸린다는 것이 계약이다).
        match handle_call(&table, &registered, call_of("tab.create")) {
            CallOutcome::Relay { .. } => {}
            other => panic!("번호 없는 요청도 받아야: {other:?}"),
        }

        // ★바깥에서 온 값이라 문법을 검증한다★ — 그리고 그 반려는 **바디 전체가 틀렸다**가 아니어야 한다.
        for bad in ["", "not-a-uuid", &"9".repeat(4096)] {
            let req = CallRequest {
                name: "tab.create".to_string(),
                request_id: Some(bad.to_string()),
                ..CallRequest::default()
            };
            match handle_call(&table, &registered, req) {
                CallOutcome::Answered(ControlQueryResult::Error { code, hint }) => {
                    assert_eq!(code, "INVALID_ARGUMENT", "{hint}");
                    assert!(
                        hint.contains(CALLER_REQUEST_ID_FIELD),
                        "어느 칸이 틀렸는지 말해야: {hint}"
                    );
                    assert!(hint.len() < 4096, "문구가 요청만큼 커졌다: {}", hint.len());
                }
                other => panic!("반려여야: {other:?}"),
            }
        }
    }

    /// 잘못된 번호는 **이름을 가리지 않고** 같은 답을 낸다 — 안 그러면 이 입구의 계약이 이름에 따라 갈린다.
    ///
    /// ★그래서 1단계 표가 그 이름을 **실제로 쥐고 있어야** 이 시험이 자기 이름을 잰다★: 빈 표를 넘기면
    /// `agent.list` 도 중계 갈래로 떨어져, 시험 이름이 주장하는 축(자기 이름에서도 같은 반려)을 한 번도
    /// 태우지 않는다. 형제([`a_caller_supplied_request_id_is_used_as_is_and_a_malformed_one_names_that_field`])
    /// 는 빈 표로 중계 갈래를 재므로, 둘이 합쳐 두 갈래를 덮는다.
    #[test]
    fn a_malformed_request_id_is_refused_even_for_a_name_this_daemon_holds() {
        let req = CallRequest {
            name: "agent.list".to_string(),
            request_id: Some("nope".to_string()),
            ..CallRequest::default()
        };
        match handle_call(&Holds("agent.list"), &Registered("tab.create"), req) {
            CallOutcome::Answered(ControlQueryResult::Error { code, .. }) => {
                assert_eq!(code, "INVALID_ARGUMENT")
            }
            other => panic!("반려여야: {other:?}"),
        }
    }

    /// ★1단계 표가 **자기 것이라 해 놓고 빈손**을 내면 중계로 흘리지 않는다★
    ///
    /// 흘리면 그 이름은 명부에 주인이 없어 「모르는 명령」으로 나가거나(있으면) 버스가 같은 근거로 다시
    /// 거절한다 — 어느 쪽이든 호출자는 **자기가 고칠 수 없는 배선 결함**을 자기 잘못으로 읽는다. 배달도 같은
    /// 어긋남을 같은 코드로 드러낸다(`command_delivery` 의 `fold_local`).
    /// ★이 상태는 실물 어댑터에서는 날 수 없다★(`claim`·`run` 이 같은 `OnceLock` 표를 본다) — 그래서 여기
    /// 이중인격 표를 손으로 세운다. 다른 구현을 꽂는 날 그 성질을 함께 가져와야 한다는 것이 이 시험의 값이다.
    #[test]
    fn a_table_that_claims_a_name_but_answers_nothing_is_not_relayed() {
        struct ClaimsButAnswersNothing;
        impl LocalCommands for ClaimsButAnswersNothing {
            fn claim(&self, _name: &str) -> Option<Effect> {
                Some(Effect::Write)
            }
            fn run(
                &self,
                _name: &str,
                _args: &mut serde_json::Value,
                _entrance: &'static str,
            ) -> Option<Result<serde_json::Value, CommandError>> {
                None
            }
            fn decls(&self) -> Vec<CommandDecl> {
                Vec::new()
            }
        }

        // 주인이 붙어 있는 이름으로 태운다 — 중계로 흘렸다면 여기가 `Relay` 가 된다.
        match handle_call(
            &ClaimsButAnswersNothing,
            &Registered("tab.create"),
            call_of("tab.create"),
        ) {
            CallOutcome::Answered(ControlQueryResult::Error { code, hint }) => {
                assert_eq!(code, "INTERNAL", "{hint}");
                assert!(hint.contains("tab.create"), "어느 이름인지 말해야: {hint}");
            }
            other => panic!("INTERNAL 반려여야: {other:?}"),
        }
    }

    #[test]
    fn an_absent_args_object_is_the_same_as_an_empty_one() {
        let with: CallRequest =
            serde_json::from_str(r#"{"name":"agent.list","args":{}}"#).expect("역직렬화");
        let without: CallRequest =
            serde_json::from_str(r#"{"name":"agent.list"}"#).expect("역직렬화");
        assert_eq!(with.args.len(), without.args.len());
        assert_eq!(without.name, "agent.list");
    }

    /// ★`name` 은 꼭대기 어휘이면서 실제 인자 이름이기도 하다★ — 인자를 꼭대기에 펼치는 형태로 되돌리면
    /// `agent.rename` 을 이 입구로 부를 수 없게 된다.
    #[test]
    fn a_command_argument_called_name_survives_alongside_the_command_name() {
        let req: CallRequest =
            serde_json::from_str(r#"{"name":"agent.rename","args":{"target":"a","name":"b"}}"#)
                .expect("역직렬화");

        assert_eq!(req.name, "agent.rename");
        assert_eq!(req.args["name"], "b");
        assert_eq!(req.args["target"], "a");
    }

    /// ★배열 바디는 **호출이 아니다**★ — 파생 역직렬화로 되돌리면 `["agent.list"]` 가 필드 순서열로 읽혀
    /// 그 명령이 **실행된다**(실측). 부작용 있는 입구에서 그 관용은 사고다.
    #[test]
    fn a_json_array_is_not_a_command_call() {
        for body in [r#"["agent.list"]"#, r#"["agent.new",{"cwd":"C:/x"}]"#] {
            assert!(
                serde_json::from_str::<CallRequest>(body).is_err(),
                "객체가 아닌 바디는 반려여야: {body}"
            );
        }
    }

    /// 모르는 꼭대기 칸은 무시하되(홉 간 additive 진화) **인자를 잃은 요청이 조용히 성공하지는 않는다** —
    /// 그 판정은 표의 입구 검문이 지므로 여기서는 칸이 `args` 로 안 새는 것만 못박는다.
    #[test]
    fn an_unknown_top_level_field_is_ignored_and_never_becomes_an_argument() {
        let req: CallRequest =
            serde_json::from_str(r#"{"name":"agent.move","target":"a","parent":"lead"}"#)
                .expect("역직렬화");

        assert_eq!(req.name, "agent.move");
        assert!(
            req.args.is_empty(),
            "꼭대기 칸이 인자로 새면 `args` 규율이 무의미해진다: {:?}",
            *req.args
        );
    }

    /// ★꼭대기 칸만 본다★ — `args` 안의 중복은 [`CommandArgs`] 소유이고 그쪽 자리에서 이미 재고 있다.
    /// 여기서 겹쳐 재면 이 파일이 안 가진 규율을 이 파일이 지키는 것처럼 읽힌다(그 합성이 실제로 도는지는
    /// 라우트를 태우는 통합 시험이 본다).
    #[test]
    fn a_duplicate_name_is_refused() {
        assert!(
            serde_json::from_str::<CallRequest>(r#"{"name":"agent.list","name":"agent.new"}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<CallRequest>(
            r#"{"name":"agent.list","args":{},"args":{}}"#
        )
        .is_err());
    }

    /// 이름 하나에 주인이 붙어 있다고 답하는 명부.
    struct Registered(&'static str);

    impl OwnerLookupSource for Registered {
        fn lookup(&self, name: &str) -> OwnerLookup {
            if name == self.0 {
                OwnerLookup::Available(OwnerToken::new("conn-1"))
            } else {
                OwnerLookup::Unknown
            }
        }
    }

    fn call_of(name: &str) -> CallRequest {
        CallRequest {
            name: name.to_string(),
            ..CallRequest::default()
        }
    }

    /// ★목록이 가르친 이름은 **버스로 간다**★ — 여기서 반려로 접으면 대시보드 명령 열다섯이 다시 닫힌다
    /// (ADR-0160). 그리고 아무도 선언하지 않은 이름은 여전히 「모르는 명령」이어야 한다 — 두 사실을 한
    /// 갈래로 뭉치면 호출자는 실재하는 이름을 계속 바꿔 가며 재시도한다.
    #[test]
    fn a_name_with_a_connected_owner_goes_to_the_bus_and_an_unknown_one_is_refused() {
        let table = NoLocalCommands;
        let registered = Registered("tab.create");

        match handle_call(&table, &registered, call_of("tab.create")) {
            CallOutcome::Relay { name, args, .. } => {
                assert_eq!(name, "tab.create");
                assert!(args.is_object(), "인자는 그대로 넘어간다: {args}");
            }
            CallOutcome::Answered(other) => panic!("주인이 붙은 이름은 중계여야: {other:?}"),
        }

        match handle_call(&table, &registered, call_of("nope.nope")) {
            CallOutcome::Answered(ControlQueryResult::Error { code, hint }) => {
                assert_eq!(code, "UNKNOWN_COMMAND");
                assert!(hint.contains("nope.nope"), "{hint}");
                assert!(hint.contains(CATALOG_RECOVERY), "{hint}");
            }
            other => panic!("모르는 이름은 반려여야: {other:?}"),
        }
    }

    /// 중계된 결말은 **주인이 만든 것**이다 — 성공 payload 는 열어보지 않고, 실패는 이 표면의 다른 반려와
    /// 같은 함수를 탄다(회복 안내 규율까지 같이 걸린다).
    #[test]
    fn a_relayed_outcome_keeps_the_owners_answer() {
        let id = RequestId::new();

        let ok = relayed(CommandReply::ok(id, serde_json::json!({ "tab": "t-1" })));
        assert!(
            matches!(&ok, ControlQueryResult::Ok(v) if v["tab"] == "t-1"),
            "{ok:?}"
        );

        let failed = relayed(CommandReply::err(
            id,
            CommandError::invalid_argument("no such window 'ghost'"),
        ));
        match failed {
            ControlQueryResult::Error { code, hint } => {
                assert_eq!(code, "INVALID_ARGUMENT");
                assert!(hint.contains("ghost"), "{hint}");
                assert!(
                    hint.contains(CATALOG_RECOVERY),
                    "고칠 수 있는 실패다: {hint}"
                );
            }
            other => panic!("반려여야: {other:?}"),
        }
    }

    /// 고칠 인자가 없는 실패에 「목록을 보라」를 달면 호출자는 없는 오타를 찾는다.
    #[test]
    fn the_recovery_is_attached_only_to_faults_the_catalog_can_fix() {
        let fixable = refused(CommandError::invalid_argument("bad field"));
        let unfixable = refused(CommandError::not_found("no agent called 'ghost'"));

        match (fixable, unfixable) {
            (
                ControlQueryResult::Error { hint: fixable, .. },
                ControlQueryResult::Error {
                    code,
                    hint: unfixable,
                },
            ) => {
                assert!(fixable.contains(CATALOG_RECOVERY), "{fixable}");
                assert!(fixable.contains("bad field"), "{fixable}");
                assert_eq!(code, "NOT_FOUND");
                assert!(!unfixable.contains(CATALOG_RECOVERY), "{unfixable}");
                assert!(unfixable.contains("ghost"), "{unfixable}");
            }
            other => panic!("둘 다 반려여야: {other:?}"),
        }
    }

    /// ★반려 문구는 요청만큼 커지지 않는다 — **어느 갈래로 나가든**★: 호출자 문자열은 이름으로도
    /// (`unknown_name`) 표가 준 문구 안으로도(`refused` — `check_args` 가 친 키를 원문 그대로 넣는다)
    /// 들어온다. 한 갈래만 재면 다른 갈래는 1 MiB 바디 상한이 유일한 방벽인 채로 남고, 그 상태에서 이
    /// 테스트는 **파일 전체의 성질처럼 읽혀** 없는 보장을 광고한다.
    ///
    /// 아래 수는 **여유이지 계약이 아니다** — 계약은 「요청 크기와 무관」이고, 그것을 세우는 것은 갈래마다의
    /// `preview_within`·`preview` 다(몫이 다르므로 한 수로 좁게 못 잡는다).
    #[test]
    fn every_refusal_branch_is_reported_within_a_bound() {
        let huge = "k".repeat(500_000);
        let branches = [
            unknown_name(&huge),
            // 중계된 실패도 이 표면의 반려로 나간다 — 주인이 보낸 문구가 요청만큼 클 수 있다.
            relayed(CommandReply::err(
                RequestId::new(),
                CommandError::of(ErrorCode::Conflict, format!("owner said: {huge}")),
            )),
            // 표의 문구는 자기 입력을 캡하지만(`quoted_input`), 캡하지 않는 생산자도 있다
            //   (`commands::drive_to_completion`) — 그쪽이 오는 모양으로 잰다.
            refused(CommandError::invalid_argument(format!(
                "not an argument of '{huge}': … — declared arguments: target, name"
            ))),
            refused(CommandError::not_found(format!("no agent called '{huge}'"))),
            malformed_body("expected a string", &huge),
        ];
        for branch in branches {
            match branch {
                ControlQueryResult::Error { code, hint } => {
                    assert!(
                        hint.contains("truncated"),
                        "잘랐다고 말한다({code}): {hint}"
                    );
                    assert!(
                        hint.len() < 4096,
                        "문구가 요청만큼 커졌다({code}): {} 바이트",
                        hint.len()
                    );
                }
                other => panic!("반려여야: {other:?}"),
            }
        }
    }

    /// ★자르기는 **양 끝을 남긴다**★ — 머리에는 호출자가 친 것이, 꼬리에는 다음에 할 일이 앉는다.
    /// 머리만 남기는 형태로 되돌리면 정확히 그 「할 일」이 사라진다(표의 인자 반려는 선언된 칸 전량을
    /// 문구 **뒤**에 단다).
    ///
    /// 여기 입력은 어떤 상한도 안 통과한 **가상의 최악**이다 — 설계된 최악은 이보다 훨씬 작아 안 잘린다
    /// (그 경우는 통합 시험 `an_unknown_field_is_refused_with_the_declared_argument_list` 계열이 실제
    /// 라우트로 잰다). 여기서 재는 것은 「잘릴 때 무엇이 남는가」다.
    #[test]
    fn truncation_keeps_both_ends_so_the_next_step_survives() {
        let refusal = refused(CommandError::invalid_argument(format!(
            "not an argument of 'agent.rename': \"{}\" — declared arguments: target, name",
            "n".repeat(500_000)
        )));

        match refusal {
            ControlQueryResult::Error { hint, .. } => {
                assert!(hint.contains("truncated"), "잘랐다고 말한다: {hint}");
                assert!(
                    hint.contains("not an argument of 'agent.rename'"),
                    "머리(무엇이 틀렸나)가 남아야: {hint}"
                );
                for declared in ["declared arguments", "target", "name"] {
                    assert!(
                        hint.contains(declared),
                        "꼬리(무엇으로 고치나)가 잘려 나갔다({declared}): {hint}"
                    );
                }
            }
            other => panic!("반려여야: {other:?}"),
        }
    }
}
