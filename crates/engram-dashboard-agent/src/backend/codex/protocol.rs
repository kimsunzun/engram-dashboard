//! codex app-server 와이어 타입 — 봉투 · 메서드 이름 · 우리가 보내고 받는 payload.
//!
//! ★출처와 재생성 방법(다시 추론하지 말 것)★: 이 파일의 필드 이름·필수 여부·enum 값은 전부
//!   codex-cli **0.154.0** 이 낸 JSON Schema 에서 읽은 것이다. 다시 뽑는 명령 =
//!   `codex app-server generate-json-schema --out <dir>` — `v2/` 아래에 타입별 파일 266 개가
//!   떨어진다. 상류가 바뀌었나를 확인할 때는 이 파일을 읽지 말고 그 명령으로 다시 뽑아 대조한다.
//!
//! ★봉투가 JSON-RPC 2.0 이 아니다★ — `jsonrpc` 필드가 스키마 전체에 0 회다. 보내지도 말고
//!   받을 때 요구하지도 말 것.
//!
//! ★인바운드 구조체는 **우리가 읽는 칸만** 싣는다★ — serde 는 모르는 칸을 무시하므로, 안 읽는
//!   필수 칸까지 옮겨 적으면 얻는 것 없이 역직렬화 실패 지점만 는다. 반대로 **아웃바운드**는
//!   스키마가 required 로 표시한 칸을 전부 싣는다 — 빠지면 서버가 거절한다.
//!
//! ★`deny_unknown_fields` 를 어디에도 달지 않는다 — 취향이 아니라 정확성 요구다★(실측 0.154.0):
//!   실 응답은 스키마에 **아예 없는** 칸을 싣고 온다. `thread/start` 응답에서 관측된 것만도
//!   `environments`·`runtimeWorkspaceRoots`·`extra`·`canAcceptDirectInput`·`daybreakEnabled`
//!   ·`multiAgentMode`·`activePermissionProfile`·`historyMode` 다. 모르는 칸을 거부하는 순간
//!   정상 응답이 해독 실패가 된다.
//!
//! ★crate 전역 `rename_all` 을 쓰지 않는다★ — 이 프로토콜은 자기 안에서 케이싱이 섞인다
//!   (`input_text` 옆에 `inputText`, `approved_for_session` 옆에 `acceptForSession`).
//!   `rename_all` 은 **그 타입의 칸이 전부 camelCase 라고 스키마에서 확인한 타입에만** 단다.
//!
//! 이 파일은 I/O 를 하지 않는다 — 문자열 in, 타입 out.

// chunk 1 은 타입과 번역기만 세우고 배선은 하나도 하지 않는다. 아래 표면의 소비자(통로 구현체)는
//   chunk 2 가 만들고, 그때 이 allow 를 걷는다.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── 봉투 ──────────────────────────────────────────────────────────────────────

/// JSON-RPC id. 스키마는 `anyOf[string, i64]` 이고 string 을 먼저 적는다(`RequestId.json`).
///
/// ★서버가 낸 id 는 **글자 그대로** 되돌려 줘야 한다★ — 그래서 숫자로 정규화하지 않는다.
/// i64 인 것도 의도다: 스키마가 `format: int64` 라 JS 쪽 피어가 2^53 을 넘는 id 를 보낼 수 있다.
///
/// ★`null` 로 직렬화될 수 있는 변형이 없는 것이 이 타입의 불변식이다★(실측 0.154.0):
/// `id: null` 을 보내면 서버의 untagged 봉투 union 이 그 줄을 **알림으로 재분류해 조용히
/// 버린다** — 오류도, stderr 한 줄도 없다. `Option` 으로 감싸거나 Null 변형을 더하면 그 침묵이
/// 되살아난다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RequestId {
    Str(String),
    Num(i64),
}

/// `JSONRPCErrorError` — 오류 봉투의 본문.
///
/// ★`code` 는 불투명한 i64 다★: 스키마에 enum·const·min/max 가 없고 `-32xxx` 대역이 **한 번도**
/// 안 나온다(`-32001` 포함 — grep 0 히트). 과부하·레이트리밋은 이 봉투가 아니라 `error`
/// **알림**의 `codexErrorInfo`(`serverOverloaded`·`rateLimitExceeded`)로 온다.
/// ★런타임에 `-32001` 같은 코드가 실제로 오는지는 미검증★ — 숫자로 분기하는 코드를 여기 두지
/// 않는다. 특정 코드 취급이 필요해지면 실 서버 관측이 먼저다.
///
/// 실측 0.154.0: 모르는 메서드에 대한 답은 `-32600` 이고 `data` 칸은 **오지 않았다**(그래도
/// optional 로 둔다 — 스키마가 허용한다). ★`message` 가 4KB 에 이를 수 있다★ — 모르는 메서드
/// 오류의 본문이 유효 메서드 160 개를 전부 열거한다. 이 문자열을 로그·화면으로 옮기는 쪽은
/// 반드시 잘라 쓴다.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) data: Option<Value>,
}

/// 서버에서 들어온 한 줄의 봉투 모양 넷.
///
/// ★판별은 **칸의 유무**로 한다 — id 값으로는 원리상 못 가른다★: 서버가 낸 id 를 그대로
/// 되돌려 줘야 하므로 두 id 공간은 겹칠 수밖에 없다(TRD §4-4).
/// `#[serde(untagged)]` 로 만들지 않는다 — 네 모양의 required 집합이 겹치고 어느 것도 여분 칸을
/// 금하지 않아, 순서가 의미를 결정하는 함정이 된다(`JSONRPCMessage` 가 그 모양이다).
#[derive(Debug, Clone)]
pub(crate) enum Inbound {
    /// `method` + `id` — 서버가 우리에게 묻는다. ★답하지 않으면 그 에이전트는 영구 정지다★.
    Request {
        id: RequestId,
        method: String,
        params: Option<Value>,
    },
    /// `method` 만 — 아무도 답을 기다리지 않는다. 번역기가 받는 것은 이것뿐이다.
    ///
    /// ★실 알림 봉투에는 `method`·`params` 와 나란히 `emittedAtMs` 가 하나 더 붙는다★
    /// (실측 0.154.0 — 알림 12/12 에 있었고 응답 8/8 에는 없었다. 스키마에는 없는 칸이다).
    /// 여기서는 **의도적으로 안 싣는다** — 이 단계의 번역은 순서에 기대지 않는다. 필요해지면
    /// 원본 줄에서 그 키를 읽으면 된다(판별은 그 칸이 있어도 그대로 선다).
    Notification {
        method: String,
        params: Option<Value>,
    },
    /// `id` + `result` — 우리가 낸 요청의 답.
    Response { id: RequestId, result: Value },
    /// `id` + `error` — 우리가 낸 요청의 실패.
    Error { id: RequestId, error: RpcError },
}

/// [`classify`] 가 한 줄을 네 봉투 중 어느 것으로도 못 읽은 사유.
///
/// ★전부 비-치명이다★ — 호출자는 그 줄 하나를 로그로 남기고 버리며, 스트림을 끊지 않는다
/// (TRD §4-6: 비-JSON stdout 라인으로 죽지 않는다).
#[derive(Debug, thiserror::Error)]
pub(crate) enum ParseError {
    #[error("JSON 이 아니다: {0}")]
    NotJson(serde_json::Error),
    #[error("JSON 최상위가 객체가 아니다")]
    NotObject,
    #[error("`method` 가 문자열이 아니다")]
    MethodNotString,
    #[error("`id` 가 문자열도 i64 도 아니다")]
    BadId,
    #[error("`error` 가 {{code,message}} 모양이 아니다: {0}")]
    BadError(serde_json::Error),
    #[error("`{0}` 봉투에 `id` 가 없다")]
    MissingId(&'static str),
    /// `result` 와 `error` 가 한 줄에 함께 왔다. 성공과 실패를 동시에 주장하는 줄이라 어느 쪽으로
    /// 읽어도 틀릴 수 있다 — ★먼저 나오는 칸을 이기게 두면 실패를 성공으로 읽는다★.
    #[error("`result` 와 `error` 가 한 봉투에 함께 있다")]
    ConflictingEnvelope,
    #[error("네 봉투 모양 중 어느 것도 아니다 — method·result·error 가 전부 없다")]
    UnknownShape,
}

/// 한 **줄**을 봉투 하나로 읽는다. 줄 분할은 호출자 몫이고 이 함수는 개행을 다루지 않는다.
///
/// 판별 순서가 계약이다: `method` 가 있으면 요청/알림이고(그때 `id` 유무가 둘을 가른다),
/// 없을 때만 `result`·`error` 를 본다. ★`method` 와 `id` 가 함께 온 줄은 언제나 요청이다★ —
/// 그 줄을 알림으로 읽으면 답해야 할 요청을 버리게 된다.
///
/// ★모르는 형제 칸이 있어도 판별은 그대로 선다★ — 실 알림에는 `emittedAtMs` 가 늘 함께 오고
/// (실측 0.154.0), 상류는 스키마에 없는 칸을 언제든 더한다. "여분 칸이 있으니 네 모양 중 아무
/// 것도 아니다" 로 읽으면 정상 줄이 통째로 버려진다.
///
/// JSON 이 아닌 줄, JSON 이지만 네 모양 어디에도 안 맞는 줄은 **오류**로 돌아온다 — panic 도
/// 조용한 버림도 아니다.
///
/// ★호출자에게 거는 요구 하나 — 나간 요청마다 마감 시한을 둘 것★(실측 0.154.0): 서버는 자기가
/// **해독하지 못한 봉투에는 `id` 가 실려 있어도 아무 답도 하지 않는다**(`{"id":90}` 한 줄에
/// stderr 로그만 남고 stdout 은 영영 조용했다). 답이 오는 것은 봉투가 읽힌 뒤의 *파라미터* 오류
/// 뿐이다. 그래서 "답이 올 때까지 기다린다" 는 대기표는 언젠가 반드시 영구히 매달린다.
///
/// ★알려진 한계 — 중복 키를 거르지 않는다★: `{"id":1,"id":2,"result":{}}` 같은 줄에서 serde_json
/// 은 **마지막 값을 취한다**. 그래서 그런 줄이 오면 우리는 `id=2` 로 읽고, 그 응답은 **엉뚱한
/// 대기자에게 배달된다**(원래 요청은 영영 안 풀리고 남의 요청이 남의 답을 받는다). 거르려면 맵을
/// 거치지 않는 전용 visitor 가 필요해 여기서는 하지 않았다 — 미검증이고, 실 서버가 그런 줄을 내는
/// 것을 본 적은 없다.
pub(crate) fn classify(line: &str) -> Result<Inbound, ParseError> {
    let value: Value = serde_json::from_str(line).map_err(ParseError::NotJson)?;
    let obj = value.as_object().ok_or(ParseError::NotObject)?;

    // ★method 분기보다 먼저 본다★ — 모순은 봉투 종류와 무관하고, 나중에 보면 `result` 를 먼저
    //   집는 분기가 실패를 성공으로 읽어 버린다.
    if obj.contains_key("result") && obj.contains_key("error") {
        return Err(ParseError::ConflictingEnvelope);
    }

    if let Some(raw_method) = obj.get("method") {
        let method = raw_method
            .as_str()
            .ok_or(ParseError::MethodNotString)?
            .to_string();
        let params = obj.get("params").cloned();
        return match obj.get("id") {
            Some(raw_id) => Ok(Inbound::Request {
                id: parse_id(raw_id)?,
                method,
                params,
            }),
            None => Ok(Inbound::Notification { method, params }),
        };
    }

    if let Some(result) = obj.get("result") {
        let raw_id = obj.get("id").ok_or(ParseError::MissingId("result"))?;
        return Ok(Inbound::Response {
            id: parse_id(raw_id)?,
            result: result.clone(),
        });
    }

    if let Some(raw_error) = obj.get("error") {
        let raw_id = obj.get("id").ok_or(ParseError::MissingId("error"))?;
        let error: RpcError =
            serde_json::from_value(raw_error.clone()).map_err(ParseError::BadError)?;
        return Ok(Inbound::Error {
            id: parse_id(raw_id)?,
            error,
        });
    }

    Err(ParseError::UnknownShape)
}

fn parse_id(raw: &Value) -> Result<RequestId, ParseError> {
    match raw {
        Value::String(s) => Ok(RequestId::Str(s.clone())),
        Value::Number(n) => n.as_i64().map(RequestId::Num).ok_or(ParseError::BadId),
        _ => Err(ParseError::BadId),
    }
}

// ── 메서드 이름 ───────────────────────────────────────────────────────────────

/// 와이어 메서드 이름. ★버전 접두가 없다★ — 스키마 트리의 `v1/`·`v2/` 는 소스 정리용
/// 디렉터리일 뿐이고 메서드는 `thread/start` 처럼 맨 이름으로 나간다.
pub(crate) mod method {
    // 우리 → 서버 요청
    pub(crate) const INITIALIZE: &str = "initialize";
    pub(crate) const THREAD_START: &str = "thread/start";
    pub(crate) const THREAD_RESUME: &str = "thread/resume";
    pub(crate) const TURN_START: &str = "turn/start";
    pub(crate) const TURN_INTERRUPT: &str = "turn/interrupt";

    /// 이어받은 스레드의 지난 item 을 **페이지로** 받는다(ADR-0203).
    ///
    /// ★쌍둥이 `thread/turns/list` 를 쓰지 않는다★ — 우리가 화면에 그리는 단위는 turn 이 아니라
    /// item 이고, 그쪽은 같은 item 을 turn 봉투 안에 한 겹 더 싸서 준다. 벗길 봉투만 하나 는다.
    pub(crate) const THREAD_ITEMS_LIST: &str = "thread/items/list";

    /// 우리 → 서버 알림. ★클라이언트가 보낼 수 있는 알림은 이것 하나뿐이고 params 칸이 아예
    /// 없다★(`ClientNotification.json`).
    /// ★보내는 것이 의무가 아니다★(실측 0.154.0 — 보낸 경우와 안 보낸 경우 둘 다에서
    /// `thread/start` 가 똑같이 성공했다). 보내도 해롭지 않아 핸드셰이크 모양을 맞추는 쪽으로 둔다.
    pub(crate) const INITIALIZED: &str = "initialized";

    // 서버 → 우리 알림 중 번역기가 이름으로 아는 것
    pub(crate) const ITEM_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
    pub(crate) const ITEM_STARTED: &str = "item/started";
    pub(crate) const ITEM_COMPLETED: &str = "item/completed";
    pub(crate) const THREAD_TOKEN_USAGE_UPDATED: &str = "thread/tokenUsage/updated";
    pub(crate) const ERROR: &str = "error";
    pub(crate) const DEPRECATION_NOTICE: &str = "deprecationNotice";

    /// ★이 이름 하나에 소비자가 둘이다★ — 번역기가 턴 경계로 옮기고(`decoder`), 통로가 큐 해제의
    /// 상태 기계 입력으로 읽는다(`transport`). 둘은 같은 줄을 각자 본다.
    pub(crate) const TURN_COMPLETED: &str = "turn/completed";
}

/// `TurnStatus` 의 네 값 전량(스키마 0.154.0 `definitions.TurnStatus` — 닫힌 `enum`).
///
/// ★문자열 상수로 두고 Rust `enum` 으로 받지 않는다★: 닫힌 타입으로 역직렬화하면 상류가 다섯째 값을
/// 더한 날 `turn/completed` 줄 **전체**가 실패하고, 그러면 턴 경계가 통째로 사라져 화면의 대기
/// 인디케이터가 영영 돈다. 그 결말이 이 상수들이 고치는 결함 그 자체다.
pub(crate) mod turn_status {
    pub(crate) const COMPLETED: &str = "completed";
    pub(crate) const INTERRUPTED: &str = "interrupted";
    pub(crate) const FAILED: &str = "failed";
    pub(crate) const IN_PROGRESS: &str = "inProgress";

    /// 등급을 가르는 용도 — 여기 **없는** 값은 상류 드리프트이고, 여기 있는데 결말로 안 옮겨지는 값
    /// (오늘 [`IN_PROGRESS`] 하나)은 「뜻을 아직 모른다」다. 어느 쪽이든 번역은 그래도 한다.
    pub(crate) const ALL: &[&str] = &[COMPLETED, INTERRUPTED, FAILED, IN_PROGRESS];
}

/// 우리가 모르는 인바운드 **요청**을 거절할 때 싣는 코드.
///
/// ★스키마는 코드 대역을 정하지 않는다★ — `JSONRPCErrorError.code` 는 제약 없는 i64 이고
/// `-32xxx` 가 스키마에 한 번도 안 나온다. 우리가 봉투를 정의하는 쪽이므로 값은 우리가 고르고,
/// JSON-RPC 2.0 관례의 "method not found" 를 빌려 쓴다(읽는 사람에게 뜻이 통한다).
/// ★app-server 가 이 응답을 어떻게 받아들이는지는 미검증★ — 그 턴을 실패로 접는지 무시하는지
/// 본 적이 없다. 그래도 버리는 것보다 낫다: 버리면 그 에이전트가 영구 정지한다(TRD §6-2).
///
/// 참고로 **codex 자신은 모르는 메서드에 `-32600` 을 돌려준다**(실측 0.154.0) — 즉 상대는
/// "모르는 메서드" 를 따로 세지 않는다. 그래도 우리 쪽 값은 관례 뜻이 분명한 -32601 로 둔다:
/// 이 숫자를 읽는 첫 독자는 로그를 여는 사람이다.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;

// ── 직렬화 헬퍼 ───────────────────────────────────────────────────────────────

/// 요청 한 줄을 만든다. ★돌려주는 문자열은 **개행까지 포함한 완성된 줄**이다★ — 호출자가
/// `\n` 을 따로 붙이면 빈 줄이 하나 더 나간다.
///
/// `params` 직렬화가 실패하면 오류다(우리 타입은 실패할 수 없지만 제네릭이라 계약상 열어 둔다).
pub(crate) fn request_line<P: Serialize>(
    id: &RequestId,
    method: &str,
    params: &P,
) -> Result<String, serde_json::Error> {
    let mut obj = serde_json::Map::new();
    obj.insert("id".to_string(), serde_json::to_value(id)?);
    obj.insert("method".to_string(), Value::String(method.to_string()));
    obj.insert("params".to_string(), serde_json::to_value(params)?);
    Ok(format!("{}\n", Value::Object(obj)))
}

/// params 없는 알림 한 줄(개행 포함). `initialized` 가 유일한 소비자다.
pub(crate) fn notification_line(method: &str) -> String {
    format!("{}\n", serde_json::json!({ "method": method }))
}

/// 서버 요청을 거절하는 오류 응답 한 줄(개행 포함).
///
/// ★`id` 는 받은 것을 그대로 싣는다★ — 서버의 대기표 키라서 정규화하면 그 요청이 영영 안 풀린다.
/// ★성공을 위장하면 안 된다★ — 승인 요청에 성공 응답을 돌려주면 그것이 자동 승인이 되어
/// 승인 정책을 우회한다(TRD §6-2).
pub(crate) fn error_response_line(id: &RequestId, code: i64, message: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "id": id,
            "error": { "code": code, "message": message },
        })
    )
}

// ── initialize ────────────────────────────────────────────────────────────────

/// ★한 번만 보낼 수 있다★(실측 0.154.0) — 두 번째 `initialize` 는 `-32600` `"Already
/// initialized"` 로 돌아온다. 재시도 경로가 이것을 다시 보내지 않게 할 책임은 호출자에 있다.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeParams {
    pub(crate) client_info: ClientInfo,
    // ★`capabilities` 를 싣지 않는 것은 의도다★ — 모든 칸이 optional 이고 우리가 원하는 값이
    //   전부 기본값이다. 특히 `requestAttestation` 이 기본 false 라 답할 수 없는
    //   `attestation/generate` 요청이 오지 않는다(그 요청엔 거절 모양이 아예 없다). 켜려면
    //   그 결과를 감당할 코드를 함께 들여야 한다.
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClientInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    // `title` 은 optional 이라 안 싣는다.
}

/// ★여기엔 프로토콜 버전이 없다★ — 스키마 전체에 `protocolVersion` 이 0 회다.
/// `initialize` 가 협상하는 것은 버전이 아니라 capability 집합이고, 우리는 그것도 안 보낸다.
/// 상류 드리프트를 사후에 가르는 유일한 값이 `user_agent` 와 `Thread.cliVersion` 이라 기동 시
/// 한 번 기록한다(TRD §4-7 의 1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResponse {
    pub(crate) codex_home: String,
    pub(crate) platform_family: String,
    pub(crate) platform_os: String,
    pub(crate) user_agent: String,
}

// ── thread/start · thread/resume ──────────────────────────────────────────────

/// 승인 정책. 스키마는 `oneOf[문자열 3 종, GranularAskForApproval]` 인데 ★우리는 문자열 갈래만
/// 보낸다★ — granular 쪽은 우리가 쓰지 않는 칸이라 모양을 옮겨 적지 않았다.
/// 값 철자가 kebab-case 인 것에 주의: 같은 프로토콜의 다른 승인 enum 은 camelCase 다.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum AskForApproval {
    #[serde(rename = "untrusted")]
    Untrusted,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "never")]
    Never,
}

/// `thread/start`·`thread/resume` 의 `sandbox` 칸 타입.
///
/// ★같은 이름의 다른 타입과 헷갈리지 말 것★ — `TurnStartParams.sandboxPolicy` 와
/// `ThreadStartResponse.sandbox` 는 구조화 union(`SandboxPolicy`)이고 이 문자열 enum 이 아니다.
/// 응답의 `sandbox` 를 그대로 이 칸에 되돌려 넣을 수 없다.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum SandboxMode {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

/// ★스키마에 `required` 배열이 아예 없다 — `{}` 가 유효한 params 다★. 그래서 모든 칸이
/// `Option` + `skip_serializing_if` 여야 한다: 그러지 않으면 codex 가 "부재" 를 기대한 자리에서
/// 명시적 `null` 을 받는다.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    /// codex 기본 지시문 **뒤에 덧붙는** 지시문. 부재 = 아무것도 덧붙이지 않는다.
    ///
    /// ★대체가 아니라 덧붙이기다★ — 이 칸에 무엇을 실어도 codex 자신의 기본 지시문은 그대로 남는다
    ///   (실측 0.155.0: 본문이 `input[0]` 의 `developer` 역할 메시지로 앞에 서고 기본 프롬프트는 무손상).
    /// ★터미널 모드의 `-c developer_instructions=…` 와 **같은 것을 나르지만 같은 통로가 아니다**★:
    ///   이쪽은 JSON 본문이라 명령줄 길이 상한도, `%VAR%` 치환도, 줄바꿈 절단도 없다. 그래서 값을
    ///   **손대지 않고 그대로** 싣는다 — 그쪽의 줄바꿈 변환·문자 가드를 여기로 가져오지 말 것(가져오면
    ///   아무 위험도 막지 못한 채 에이전트가 읽는 문서만 망가진다).
    // ADR-0215
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

/// ★여기엔 `developerInstructions` 짝이 **없다 — 없는 채로 두는 것이 결정이다**★: 그 칸이 이 요청의
/// 스키마에도 있는지를 재 보지 않았고, 모르는 칸을 JSON-RPC params 에 얹는 대가는 「거절당한 핸드셰이크」
/// 라 이어받기 스폰이 통째로 죽는다. 그래서 **이어받은 app-server 스레드는 프라이밍을 못 받는다** —
/// 터미널 모드는 argv 라 이어받기에도 그대로 실리므로, 갭은 이 한 갈래뿐이다. 실측으로 칸의 존재가
/// 확인되면 그때 더한다.
// ADR-0215
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeParams {
    pub(crate) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    /// true 면 `thread.turns` 를 안 채워 보낸다 — 전체 이력 수화는 상류가 deprecated 로 적었다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exclude_turns: Option<bool>,
}

/// 핸드셰이크의 **둘째 요청** — 새 스레드를 여나, 저장된 스레드를 이어받나.
///
/// ★고르는 자리는 `backend/codex/mod.rs` 의 [`crate::backend::AgentBackend::open_spawn`] 하나다★ —
/// 통로는 받은 것을 그대로 낸다. 통로가 자기 상태를 보고 다시 판정하면 가르는 자리가 둘이 된다
/// (ADR-0191 이 통로 선택에서 걷어낸 것과 같은 모양).
// ADR-0185
pub(crate) enum ThreadOpen {
    Start(ThreadStartParams),
    Resume(ThreadResumeParams),
}

/// ★thread id 가 여기 산다 — 응답 최상위가 아니라 `thread.id` 다★.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ThreadStartResponse {
    pub(crate) thread: Thread,
}

/// `thread/resume` 응답. 칸 구성은 `thread/start` 응답에 커서 둘이 더 붙은 것이다.
///
/// ★그 커서 중 하나를 이제 읽는다 — 그것이 화면 복원의 **진입점**이다★(ADR-0203). 쓰는 법은 스키마
/// (0.154.0)가 그 칸의 설명에 직접 적어 둔다: "Pass this as `cursor` to `thread/items/list` with
/// `sortDirection: \"desc\"`. The first page includes the item identified by the cursor."
/// ★`default` 로 두는 것은 스키마가 그렇기 때문이다★ — `required` 목록에 없고 `default: null` 이다.
/// 부재 = 되돌아갈 이력이 없다는 뜻이고, **실패가 아니다**(복원을 건너뛴다).
/// ★`thread/start` 응답에는 이 칸이 아예 없다★ — 그래서 새 대화는 이 경로를 탈 재료가 없다.
/// ★나머지 커서(`turnsBackwardsCursor`)는 여전히 안 읽는다★ — 위 [`method::THREAD_ITEMS_LIST`] 가
/// 그 쌍둥이를 안 쓰는 사유를 진다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeResponse {
    pub(crate) thread: Thread,
    #[serde(default)]
    pub(crate) items_backwards_cursor: Option<String>,
}

/// 페이지를 어느 방향으로 걷나 — 스키마의 닫힌 `enum` 두 값 전량(0.154.0 `SortDirection`).
///
/// ★[`SortDirection::Asc`] 는 오늘 아무도 안 쓴다 — 그래도 적어 둔다★: 이것은 **나가는** 칸이라 값을
/// 지어낼 수 없고, 둘을 함께 적어 두면 방향을 뒤집는 변경이 상수를 새로 만들지 않는다.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum SortDirection {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

/// `thread/items/list` 요청. ★`threadId` 만 required 다★(스키마 0.154.0) — 나머지는 전부 생략 가능이라
/// `Option` + `skip_serializing_if` 로 둔다([`ThreadStartParams`] 와 같은 사유: 부재를 기대하는 자리에
/// 명시적 `null` 을 보내지 않는다).
/// ★한 턴으로 좁히는 `turnId` 는 옮겨 적지 않았다★ — 우리는 스레드 전체를 끝에서부터 걷는다.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemsListParams {
    pub(crate) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sort_direction: Option<SortDirection>,
}

/// 페이지 한 장.
///
/// ★`nextCursor` 가 없거나 `null` 이면 그 방향으로 더 볼 것이 없다★ — 스키마가 그렇게 적고, 실측으로도
/// 마지막 페이지에서 `null` 이 왔다(2026-09-16, 실 0.154.0 · 28 item 스레드가 25+3 으로 끝났다).
/// ★방향을 뒤집을 때 쓰는 `backwardsCursor` 는 옮겨 적지 않았다★ — 우리는 한 방향으로만 걷는다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemsListResponse {
    pub(crate) data: Vec<ThreadItemEntry>,
    #[serde(default)]
    pub(crate) next_cursor: Option<String>,
}

/// 페이지에 실린 item 한 개 + 그것이 속한 turn.
///
/// ★`item` 이 [`ItemNotification`] 의 그 칸과 **같은 타입**이라는 것이 ADR-0203 의 근거다★ — 벤더
/// 스키마에서 `item/completed` 알림·resume 응답·페이지 응답의 항목 정의 해시가 전부 같다(19 variants).
/// 그래서 번역기를 새로 짜지 않고 봉투만 벗긴다. [`Value`] 로 받는 사유도 그쪽과 같다 — 모르는 변형
/// 하나가 페이지 **전체**를 죽이면 안 된다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemEntry {
    pub(crate) turn_id: String,
    pub(crate) item: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Thread {
    /// codex 가 발급한다 — ★호출자가 정할 수 없다★. 값은 UUIDv7 문자열이고 `thread.sessionId`
    /// 가 같은 값을 담는다(실측 0.154.0). 자리는 응답 최상위가 아니라 `result.thread.id` 다.
    pub(crate) id: String,
    /// ★스키마는 required 로 적지만 우리는 부재를 허용한다★ — 진단용 값 하나가 사라졌다고
    /// 스레드 기동 전체를 실패시키는 것이 더 나쁘다. 상류 드리프트를 사후에 가르는 값이다.
    #[serde(default)]
    pub(crate) cli_version: Option<String>,
}

// ── turn/start · turn/interrupt ───────────────────────────────────────────────

/// 턴 입력 한 조각. 스키마는 7 변형(text·image·localImage·audio·localAudio·skill·mention)인데
/// ★우리가 보내는 것은 `text` 하나뿐★이라 나머지를 옮겨 적지 않았다.
///
/// ★`text_elements` 를 싣지 않는 것도 의도다★ — optional 이고 기본 `[]` 이며, 철자가 이
/// camelCase 타입 안에서 혼자 snake_case 다. 필요해지면 그 철자 그대로 써야 한다.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum UserInput {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnStartParams {
    pub(crate) thread_id: String,
    pub(crate) input: Vec<UserInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TurnStartResponse {
    pub(crate) turn: Turn,
}

/// 턴 — ★`turn/start` **응답**을 읽는 타입이고 `turn/completed` 알림은 이것으로 읽지 않는다★.
/// 그 알림의 `turn` 은 같은 스키마 타입이지만 번역기가 [`Value`] 로 직접 훑는다(사유 정본은
/// [`crate::backend::codex::decoder`] 의 `turn/completed` 자리 — 한 칸이 어긋나도 턴 경계는 서야 한다).
/// 여기서 필요한 것은 `turn/interrupt` 에 실을 id 하나뿐이라 나머지 칸(`status`·`items`·`error`
/// ·`startedAt`·`completedAt`·`durationMs`·`itemsView`)을 옮겨 적지 않았다.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Turn {
    pub(crate) id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnInterruptParams {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

/// `turn/interrupt` 의 결과 — ★스키마상 properties 도 required 도 없는 진짜 빈 객체다★.
/// 우리는 이것을 **받기만 한다**(우리가 내는 유일한 응답은 거절 오류다).
///
/// 중괄호를 단 struct 로 쓴 것은 의도다 — unit struct 는 `{}` 가 아니라 `null` 로 직렬화된다.
/// 지금은 그 방향으로 나가는 줄이 없지만, 이 타입이 언젠가 나가는 줄에 실릴 때 모양이 조용히
/// 바뀌지 않게 못 박아 둔다(회귀망이 그 직렬화를 잰다).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TurnInterruptResponse {}

// ── 번역하는 알림의 params ────────────────────────────────────────────────────

/// `item/agentMessage/delta` — 어시스턴트 본문 스트림. 네 칸 전부 required 이고 그중 셋을 읽는다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentMessageDeltaNotification {
    pub(crate) turn_id: String,
    pub(crate) item_id: String,
    pub(crate) delta: String,
}

/// `item/started` · `item/completed` 의 공통 읽기 칸.
///
/// ★두 알림의 차이는 `startedAtMs` 냐 `completedAtMs` 냐뿐이고 `item` union 은 같다★ — 그래서
/// 타입 하나로 읽고 분기는 `method` 로 한다.
/// `item` 을 타입으로 안 받고 [`Value`] 로 받는 것은 의도다 — internally-tagged enum 은 모르는
/// `type` 을 만나면 **줄 전체가 역직렬화 실패**가 되는데, 이 union 은 오늘 19 변형이고 상류가
/// 계속 늘린다. 모르는 변형 하나가 그 줄을 죽이면 안 된다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemNotification {
    pub(crate) turn_id: String,
    pub(crate) item: Value,
}

/// `thread/tokenUsage/updated`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadTokenUsageUpdatedNotification {
    pub(crate) turn_id: String,
    pub(crate) token_usage: ThreadTokenUsage,
}

/// ★`last` 와 `total` 이 무엇을 세는지 스키마가 설명하지 않는다★ — 설명 문자열이 없고 둘 다
/// required 다. 이름만 보고 `last` = 방금 끝난 모델 요청, `total` = 누적으로 읽었고, 번역기는
/// `last` 를 쓴다(사유는 [`crate::backend::codex::decoder`] 의 usage 번역 자리).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ThreadTokenUsage {
    pub(crate) last: TokenUsageBreakdown,
}

/// ★우리 `Usage` 어휘에 칸이 둘뿐이라 나머지는 잃는다★ — `cachedInputTokens`
/// ·`reasoningOutputTokens`·`totalTokens`·`cacheWriteInputTokens` 가 그것이다. 어휘를 늘리는
/// 것은 wire 변경이라 이 단계의 범위가 아니다.
///
/// ★부호 있는 `i64` 인 것은 스키마가 `format: int64` 로 선언하기 때문이다★ — `u64` 로 좁히면
/// 음수·범위 밖 값 하나에 **알림 전체가 역직렬화 실패**하고 그 턴의 `Usage` 가 통째로 사라진다.
/// 우리 어휘 칸이 `u64` 라 음수는 번역 자리에서 0 으로 눌린다(그 자리 주석이 정본).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TokenUsageBreakdown {
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
}

/// `error` 알림.
///
/// ★`threadId`·`turnId`·`willRetry` 는 스키마가 required 로 적지만 여기서는 부재를 허용한다★ —
/// 해독 못 한 오류를 통째로 버리는 것이 이 프로토콜에서 가장 나쁜 결말이라, 오류 본문만 있으면
/// 흘릴 수 있게 둔다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorNotification {
    pub(crate) error: TurnError,
    #[serde(default)]
    pub(crate) will_retry: bool,
}

/// ★이 타입을 나르는 칸이 둘이다★ — `error` 알림의 `error`, 그리고 `Turn.error`. 스키마가 후자에
/// 「Only populated when the Turn's status is failed」를 적는다(0.154.0). 그래서 실패한 턴의 사유는
/// 이 타입 하나로 읽히고, 화면 문자열을 만드는 자리도 하나다.
///
/// ★`codexErrorInfo` 를 [`Value`] 로 받는 것은 의도다★ — 문자열 enum 13 종과 래퍼 객체 5 종이
/// 섞인 union(= `oneOf` 멤버 6)이고, 우리가 쓰는 것은 사람이 읽을 라벨 하나뿐이다. 이 안에
/// `serverOverloaded`·`rateLimitExceeded` 가 있다 — 스키마가 가진 유일한 백프레셔 어휘이고,
/// 숫자 오류 코드가 아니라 **이 알림**으로 온다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnError {
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) additional_details: Option<String>,
    #[serde(default)]
    pub(crate) codex_error_info: Option<Value>,
    #[serde(default)]
    pub(crate) misalignment: Option<MisalignmentErrorDetails>,
}

/// 모델 정렬 위반으로 턴이 막힌 경우의 상세. ★`message` 만으로는 "왜 막혔나" 도 "무엇을 하면
/// 되나" 도 알 수 없다★ — 이 셋이 그 둘을 나른다.
///
/// 스키마에 `required` 배열이 없다 — 세 칸 모두 optional 이고 셋 다 nullable 이다.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MisalignmentErrorDetails {
    /// 이미 지역화된 설명이 온다(스키마 설명: "A substantive localized explanation").
    #[serde(default)]
    pub(crate) detailed_explanation: Option<String>,
    /// ★닫힌 enum 이 아니다★ — 스키마가 "clients must accept categories added by Responses" 로
    /// 열어 두었으므로 문자열 그대로 받는다.
    #[serde(default)]
    pub(crate) error_type: Option<String>,
    /// 계속하기로 한다면 **다음 턴 입력으로 그대로 제출할 문장**이다 — 우리가 지어내는 문장이
    /// 아니라 codex 가 준 것이다.
    #[serde(default)]
    pub(crate) steer: Option<MisalignmentSteer>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MisalignmentSteer {
    pub(crate) message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        v.get(key)
            .unwrap_or_else(|| panic!("필드 `{key}` 가 없다: {v}"))
    }

    // ── 봉투 판별 ────────────────────────────────────────────────────────────

    #[test]
    fn notification_has_method_and_no_id() {
        let line = r#"{"method":"item/agentMessage/delta","params":{"delta":"hi"}}"#;
        match classify(line).expect("알림으로 읽혀야 한다") {
            Inbound::Notification { method, params } => {
                assert_eq!(method, "item/agentMessage/delta");
                assert_eq!(field(&params.unwrap(), "delta"), "hi");
            }
            other => panic!("알림이 아니다: {other:?}"),
        }
    }

    #[test]
    fn method_plus_id_is_a_request_never_a_notification() {
        let line = r#"{"id":7,"method":"item/fileChange/requestApproval","params":{}}"#;
        match classify(line).expect("요청으로 읽혀야 한다") {
            Inbound::Request { id, method, .. } => {
                assert_eq!(id, RequestId::Num(7));
                assert_eq!(method, "item/fileChange/requestApproval");
            }
            other => panic!("요청이 아니다: {other:?}"),
        }
    }

    #[test]
    fn id_plus_result_is_a_response_never_a_request() {
        let line = r#"{"id":"abc","result":{"thread":{"id":"t-1"}}}"#;
        match classify(line).expect("응답으로 읽혀야 한다") {
            Inbound::Response { id, result } => {
                assert_eq!(id, RequestId::Str("abc".to_string()));
                assert_eq!(field(field(&result, "thread"), "id"), "t-1");
            }
            other => panic!("응답이 아니다: {other:?}"),
        }
    }

    #[test]
    fn id_plus_error_is_an_error_envelope() {
        let line = r#"{"id":3,"error":{"code":-1,"message":"nope"}}"#;
        match classify(line).expect("오류로 읽혀야 한다") {
            Inbound::Error { id, error } => {
                assert_eq!(id, RequestId::Num(3));
                assert_eq!(error.code, -1);
                assert_eq!(error.message, "nope");
                assert!(error.data.is_none());
            }
            other => panic!("오류 봉투가 아니다: {other:?}"),
        }
    }

    #[test]
    fn string_ids_survive_verbatim() {
        let line = r#"{"id":"req-42","method":"x","params":null}"#;
        match classify(line).unwrap() {
            Inbound::Request { id, .. } => assert_eq!(id, RequestId::Str("req-42".to_string())),
            other => panic!("요청이 아니다: {other:?}"),
        }
    }

    #[test]
    fn non_json_is_an_error_not_a_panic() {
        assert!(matches!(
            classify("Warning: something on stderr"),
            Err(ParseError::NotJson(_))
        ));
    }

    #[test]
    fn json_that_is_not_an_object_is_an_error() {
        assert!(matches!(classify("[1,2,3]"), Err(ParseError::NotObject)));
        assert!(matches!(classify("\"bare\""), Err(ParseError::NotObject)));
    }

    #[test]
    fn json_object_fitting_no_envelope_is_an_error() {
        assert!(matches!(
            classify(r#"{"hello":"world"}"#),
            Err(ParseError::UnknownShape)
        ));
    }

    /// 성공과 실패를 동시에 주장하는 줄 — 둘 중 하나를 골라 읽으면 실패를 성공으로 읽을 수 있다.
    #[test]
    fn result_and_error_together_is_an_error_not_a_response() {
        assert!(matches!(
            classify(r#"{"id":7,"result":{},"error":{"code":-1,"message":"failed"}}"#),
            Err(ParseError::ConflictingEnvelope)
        ));
        // method 가 함께 있어도 모순이 이긴다 — 분기 순서에 기대지 않는다.
        assert!(matches!(
            classify(r#"{"method":"x","result":{},"error":{"code":-1,"message":"f"}}"#),
            Err(ParseError::ConflictingEnvelope)
        ));
    }

    #[test]
    fn result_without_id_is_an_error() {
        assert!(matches!(
            classify(r#"{"result":{}}"#),
            Err(ParseError::MissingId("result"))
        ));
    }

    #[test]
    fn non_string_method_is_an_error() {
        assert!(matches!(
            classify(r#"{"method":42,"id":1}"#),
            Err(ParseError::MethodNotString)
        ));
    }

    #[test]
    fn non_scalar_id_is_an_error() {
        assert!(matches!(
            classify(r#"{"method":"x","id":{"nested":true}}"#),
            Err(ParseError::BadId)
        ));
    }

    /// 실측 0.154.0 — 알림 봉투에는 `emittedAtMs` 가 늘 함께 온다. 여분 형제 칸 때문에 판별이
    /// 흔들리면 정상 알림이 통째로 버려진다.
    #[test]
    fn an_extra_sibling_key_does_not_break_classification() {
        let line =
            r#"{"method":"item/started","params":{"turnId":"u"},"emittedAtMs":1764000000000}"#;
        match classify(line).expect("알림으로 읽혀야 한다") {
            Inbound::Notification { method, .. } => assert_eq!(method, "item/started"),
            other => panic!("알림이 아니다: {other:?}"),
        }
    }

    /// 실측 0.154.0 — 응답에 스키마에 없는 칸이 여럿 실려 온다. 모르는 칸을 거부하면 정상 응답이
    /// 해독 실패가 된다.
    #[test]
    fn unknown_fields_on_a_response_are_tolerated() {
        let v = serde_json::json!({
            "thread": {"id": "t-1", "historyMode": "compact", "extra": {"a": 1}},
            "environments": [], "runtimeWorkspaceRoots": [], "canAcceptDirectInput": true,
            "daybreakEnabled": false, "multiAgentMode": "off", "activePermissionProfile": null
        });
        let r: ThreadStartResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.thread.id, "t-1");
    }

    #[test]
    fn missing_params_is_not_an_error() {
        match classify(r#"{"method":"thread/closed"}"#).unwrap() {
            Inbound::Notification { params, .. } => assert!(params.is_none()),
            other => panic!("알림이 아니다: {other:?}"),
        }
    }

    // ── 직렬화 헬퍼 ──────────────────────────────────────────────────────────

    #[test]
    fn request_line_is_one_newline_terminated_line_that_classifies_back() {
        let params = TurnInterruptParams {
            thread_id: "t-1".to_string(),
            turn_id: "u-9".to_string(),
        };
        let line = request_line(&RequestId::Num(5), method::TURN_INTERRUPT, &params).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        match classify(line.trim_end()).unwrap() {
            Inbound::Request { id, method, params } => {
                assert_eq!(id, RequestId::Num(5));
                assert_eq!(method, "turn/interrupt");
                let p = params.unwrap();
                assert_eq!(field(&p, "threadId"), "t-1");
                assert_eq!(field(&p, "turnId"), "u-9");
            }
            other => panic!("요청이 아니다: {other:?}"),
        }
    }

    #[test]
    fn request_line_carries_no_jsonrpc_field() {
        let line = request_line(
            &RequestId::Str("a".to_string()),
            method::TURN_INTERRUPT,
            &TurnInterruptParams {
                thread_id: "t".to_string(),
                turn_id: "u".to_string(),
            },
        )
        .unwrap();
        assert!(!line.contains("jsonrpc"), "{line}");
    }

    #[test]
    fn notification_line_has_method_only() {
        let line = notification_line(method::INITIALIZED);
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(field(&v, "method"), "initialized");
        assert!(v.get("params").is_none());
        assert!(v.get("id").is_none());
    }

    #[test]
    fn error_response_line_echoes_the_id_verbatim() {
        let line = error_response_line(
            &RequestId::Str("srv-1".to_string()),
            METHOD_NOT_FOUND,
            "unsupported",
        );
        assert!(line.ends_with('\n'));
        match classify(line.trim_end()).unwrap() {
            Inbound::Error { id, error } => {
                assert_eq!(id, RequestId::Str("srv-1".to_string()));
                assert_eq!(error.code, -32601);
                assert_eq!(error.message, "unsupported");
            }
            other => panic!("오류 봉투가 아니다: {other:?}"),
        }
    }

    /// 실측 0.154.0 — `id: null` 인 요청은 서버가 **알림으로 재분류해 조용히 버린다**(오류도
    /// stderr 도 없다). 그래서 우리 id 가 어느 경로로도 `null` 이 되면 안 된다.
    #[test]
    fn ids_never_serialize_as_null() {
        for id in [RequestId::Str(String::new()), RequestId::Num(0)] {
            assert!(!serde_json::to_value(&id).unwrap().is_null());
            let req = request_line(&id, method::INITIALIZE, &serde_json::json!({})).unwrap();
            let v: Value = serde_json::from_str(req.trim_end()).unwrap();
            assert!(!field(&v, "id").is_null(), "{req}");
            let err = error_response_line(&id, METHOD_NOT_FOUND, "m");
            let v: Value = serde_json::from_str(err.trim_end()).unwrap();
            assert!(!field(&v, "id").is_null(), "{err}");
        }
    }

    #[test]
    fn error_response_line_keeps_numeric_ids_numeric() {
        let line = error_response_line(&RequestId::Num(9_007_199_254_740_993), 1, "m");
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(field(&v, "id").as_i64(), Some(9_007_199_254_740_993));
    }

    // ── 아웃바운드 params 의 와이어 철자 ─────────────────────────────────────

    #[test]
    fn initialize_params_field_names() {
        let v = serde_json::to_value(InitializeParams {
            client_info: ClientInfo {
                name: "engram-dashboard".to_string(),
                version: "0.1.0".to_string(),
            },
        })
        .unwrap();
        let info = field(&v, "clientInfo");
        assert_eq!(field(info, "name"), "engram-dashboard");
        assert_eq!(field(info, "version"), "0.1.0");
        assert!(v.get("capabilities").is_none());
    }

    #[test]
    fn thread_start_params_omit_absent_fields_entirely() {
        let v = serde_json::to_value(ThreadStartParams::default()).unwrap();
        assert_eq!(v, serde_json::json!({}));
    }

    #[test]
    fn thread_start_params_field_names_and_enum_spellings() {
        let v = serde_json::to_value(ThreadStartParams {
            cwd: Some("C:/w".to_string()),
            approval_policy: Some(AskForApproval::Never),
            sandbox: Some(SandboxMode::WorkspaceWrite),
            developer_instructions: Some("be brief".to_string()),
        })
        .unwrap();
        assert_eq!(field(&v, "cwd"), "C:/w");
        assert_eq!(field(&v, "approvalPolicy"), "never");
        assert_eq!(field(&v, "sandbox"), "workspace-write");
        // ★camelCase 로 나가는 것이 계약이다★ — snake_case 로 새면 codex 가 모르는 칸으로 읽어 조용히
        //   버리고, 증상은 「프라이밍이 안 먹는다」 하나다(오류가 없다).
        assert_eq!(field(&v, "developerInstructions"), "be brief");
    }

    #[test]
    fn approval_and_sandbox_enums_use_the_schema_spellings() {
        assert_eq!(
            serde_json::to_value(AskForApproval::OnRequest).unwrap(),
            "on-request"
        );
        assert_eq!(
            serde_json::to_value(AskForApproval::Untrusted).unwrap(),
            "untrusted"
        );
        assert_eq!(
            serde_json::to_value(SandboxMode::ReadOnly).unwrap(),
            "read-only"
        );
        assert_eq!(
            serde_json::to_value(SandboxMode::DangerFullAccess).unwrap(),
            "danger-full-access"
        );
    }

    #[test]
    fn thread_resume_params_field_names() {
        let v = serde_json::to_value(ThreadResumeParams {
            thread_id: "t-1".to_string(),
            cwd: None,
            approval_policy: None,
            sandbox: None,
            exclude_turns: Some(true),
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        assert_eq!(field(&v, "excludeTurns"), true);
        assert!(v.get("cwd").is_none());
        assert!(v.get("approvalPolicy").is_none());
        assert!(v.get("sandbox").is_none());
    }

    #[test]
    fn turn_start_params_field_names_and_input_tagging() {
        let v = serde_json::to_value(TurnStartParams {
            thread_id: "t-1".to_string(),
            input: vec![UserInput::Text {
                text: "hello".to_string(),
            }],
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        let item = &field(&v, "input").as_array().unwrap()[0];
        assert_eq!(field(item, "type"), "text");
        assert_eq!(field(item, "text"), "hello");
    }

    #[test]
    fn turn_interrupt_params_field_names() {
        let v = serde_json::to_value(TurnInterruptParams {
            thread_id: "t-1".to_string(),
            turn_id: "u-1".to_string(),
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        assert_eq!(field(&v, "turnId"), "u-1");
    }

    #[test]
    fn turn_interrupt_response_serializes_as_an_empty_object_not_null() {
        let s = serde_json::to_string(&TurnInterruptResponse::default()).unwrap();
        assert_eq!(s, "{}");
    }

    // ── 인바운드 payload ─────────────────────────────────────────────────────

    #[test]
    fn thread_start_response_reads_the_nested_thread_id() {
        let v = serde_json::json!({
            "thread": {"id": "0199-7ab", "sessionId": "0199-7ab", "cliVersion": "0.154.0",
                       "cwd": "C:/w", "createdAt": 1, "updatedAt": 2, "ephemeral": false,
                       "modelProvider": "openai", "preview": "", "projectId": null,
                       "source": {}, "status": {"type": "idle"}, "turns": []},
            "cwd": "C:/w", "model": "gpt-5", "modelProvider": "openai",
            "approvalPolicy": "never", "approvalsReviewer": "user",
            "sandbox": {"type": "dangerFullAccess"}
        });
        let r: ThreadStartResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.thread.id, "0199-7ab");
        assert_eq!(r.thread.cli_version.as_deref(), Some("0.154.0"));
    }

    #[test]
    fn thread_survives_a_missing_cli_version() {
        let t: Thread = serde_json::from_value(serde_json::json!({"id": "x"})).unwrap();
        assert_eq!(t.id, "x");
        assert!(t.cli_version.is_none());
    }

    #[test]
    fn turn_start_response_reads_the_nested_turn_id() {
        let v = serde_json::json!({"turn": {"id": "u-7", "items": [], "status": "inProgress"}});
        let r: TurnStartResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.turn.id, "u-7");
    }

    #[test]
    fn initialize_response_reads_all_four_required_fields() {
        let v = serde_json::json!({
            "codexHome": "C:/Users/x/.codex", "platformFamily": "windows",
            "platformOs": "windows", "userAgent": "codex/0.154.0"
        });
        let r: InitializeResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.user_agent, "codex/0.154.0");
        assert_eq!(r.platform_os, "windows");
        assert_eq!(r.platform_family, "windows");
        assert_eq!(r.codex_home, "C:/Users/x/.codex");
    }

    #[test]
    fn error_notification_reads_with_only_the_message_present() {
        let n: ErrorNotification =
            serde_json::from_value(serde_json::json!({"error": {"message": "boom"}})).unwrap();
        assert_eq!(n.error.message, "boom");
        assert!(!n.will_retry);
        assert!(n.error.codex_error_info.is_none());
    }

    /// 스키마가 `int64` 로 선언한 칸을 `u64` 로 좁히면 이 알림이 통째로 사라진다.
    #[test]
    fn token_counts_accept_the_full_declared_int64_range() {
        let n: ThreadTokenUsageUpdatedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t", "turnId": "u",
            "tokenUsage": {
                "last": {"inputTokens": -1, "cachedInputTokens": 0, "outputTokens": 9_007_199_254_740_993i64,
                         "reasoningOutputTokens": 0, "totalTokens": 0},
                "total": {"inputTokens": 0, "cachedInputTokens": 0, "outputTokens": 0,
                          "reasoningOutputTokens": 0, "totalTokens": 0}
            }
        }))
        .unwrap();
        assert_eq!(n.token_usage.last.input_tokens, -1);
        assert_eq!(n.token_usage.last.output_tokens, 9_007_199_254_740_993);
    }

    #[test]
    fn misalignment_details_survive_the_read() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "error": {
                "message": "blocked",
                "misalignment": {
                    "detailedExplanation": "이 요청은 정책을 벗어난다",
                    "errorType": "somethingBrandNew",
                    "steer": {"message": "다르게 물어보세요"}
                }
            }
        }))
        .unwrap();
        let m = n.error.misalignment.expect("misalignment 가 읽혀야 한다");
        assert_eq!(
            m.detailed_explanation.as_deref(),
            Some("이 요청은 정책을 벗어난다")
        );
        assert_eq!(m.error_type.as_deref(), Some("somethingBrandNew"));
        assert_eq!(m.steer.unwrap().message, "다르게 물어보세요");
    }

    #[test]
    fn misalignment_is_optional_and_its_members_are_too() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "error": {"message": "m", "misalignment": {}}
        }))
        .unwrap();
        let m = n.error.misalignment.unwrap();
        assert!(m.detailed_explanation.is_none() && m.error_type.is_none() && m.steer.is_none());
    }

    #[test]
    fn error_notification_keeps_the_codex_error_label() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t", "turnId": "u", "willRetry": true,
            "error": {"message": "overloaded", "codexErrorInfo": "serverOverloaded"}
        }))
        .unwrap();
        assert!(n.will_retry);
        assert_eq!(
            n.error.codex_error_info.unwrap().as_str(),
            Some("serverOverloaded")
        );
    }
}
