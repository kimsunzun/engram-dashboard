//! ControlIngress seam(ADR-0086 스텝 2) — 듀얼 입구(MCP + CLI)의 공통 파이프라인.
//!
//! ★역할★: 두 입구(MCP `send_message` 툴 · `/control/send` HTTP 라우트)가 각자 요청을 정규화한
//!   `ControlCommand` 로 만들어 **이 모듈의 단일 핸들러**(`handle_send`)를 부른다. 그 아래(Validator·
//!   Relay·ACK)는 어느 입구로 들어왔는지 모른다(entrance-agnostic) — 입구별 코드 중복·표류를 막는다.
//!
//! ★불변식(ADR-0086)★:
//!   - `from`(발신자 신원)은 **토큰/세션에서만 파생**된다 — 페이로드가 아니라. 두 어댑터 모두
//!     BoundIdentity(auth 미들웨어/세션 바인딩이 검증한 신원)를 넣어 ControlCommand 를 만든다(사칭 차단).
//!   - ACK/에러 JSON 은 **두 입구에서 동일 shape** 다(같은 코드가 만든다) — 자기교정 로스터(RECIPIENT_*
//!     hint)도 동일.
//!   - "enqueued" 워딩은 미래 장부(ledger)와의 forward-compat 로 유지한다 — 이 최소 버전은 즉시 배달
//!     (relay)이지만 ACK 문구는 바꾸지 않는다.
//!
//! ★봉투(ADR-0096/0103)★: 봉투 조립은 **단일 wrap point**(`wrap_message`/`wrap_notice` — ADR-0096)에만
//!   있다. S18 메시징 v1(ADR-0103)이 기본 포맷을 XML 로 flip 하고 `<message>` 속성(id/type/reply-by/
//!   in-reply-to/to)과 `<notice>`(데몬 전용, from 없음) 렌더를 이 seam 에 얹었다(`EnvelopeFields`). colon 은
//!   잔존 스위치(속성 미지원). 그룹 해석(`@`)은 **후속 increment C4**(여기 범위 밖).
//!
//! ★회신 계약 인자(C3 · spec §3 · ADR-0103 결정 2/3)★: 두 입구가 `SendContract`(request/reply_by/reply_to)를
//!   실어 오고, **구문 검증은 전부 여기서**(서비스 위임 전에) 한다 — 상호배타·기간 표기·빈 값. 통과한 인자는
//!   `engram_dashboard_messaging::service::SendMeta`(파싱된 Duration 포함)로 정규화돼 서비스로 내려간다.
//!
//! ★`type="notice"` 밀반입 불가(구조적 — ADR-0103 불변식)★: 발신 인자에는 **타입 문자열이 없다**(`request:
//!   bool` 뿐). `<notice>` 는 데몬이 `wrap_notice` 로만 만들고 그 호출부는 MessagingService 의 타임아웃 통지
//!   경로뿐이다 — 에이전트가 어떤 입구로도 notice 를 발신할 수 없다.
//!
//! tauri import 0(daemon crate).

use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::types::{AgentId, AgentStatus};
// ADR-0110: 봉투 계층·관측 어휘·논리 메시지 id 는 메시징 커널 crate 소유(옛 자리는 이 파일이었다).
use engram_dashboard_messaging::envelope::{new_msg_id, Entrance, EnvelopeFormat};

use super::registry::{BoundIdentity, ControlRegistry};

/// body 상한(64 KiB). 최소 버전의 방어적 상한 — 초과 시 BODY_TOO_LARGE 로 교정 에러(같은 shape).
/// (MCP 라우트의 전송 계층 상한(RequestBodyLimitLayer 1MB)과 별개 — 여기선 body **문자열** 자체의 상한.)
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 정규화된 제어 커맨드(ADR-0086) — 두 입구가 이 형태로 만들어 `handle_send` 에 넘긴다.
///
/// ★from = 토큰/세션 파생 신원★: 페이로드가 아니라 어댑터가 검증된 BoundIdentity 를 넣는다(사칭 차단).
/// 이 최소 버전은 커맨드 종류가 send 하나뿐이라 별도 cmd 태그 없이 send 전용 필드만 담는다(spawn/창이동
/// 등은 후속 additive — ADR-0086 §커맨드=의도별 전용 툴).
#[derive(Debug, Clone)]
pub struct ControlCommand {
    /// 발신자 신원 — 토큰/세션에서 파생(페이로드 아님). 사칭 차단의 단일 출처.
    pub from: BoundIdentity,
    /// 수신자 지목 — 에이전트 이름(profile name) 또는 정확한 AgentId 문자열. 미래 그룹(@) 예약.
    pub to: String,
    /// 메시지 본문(텍스트). 최소 버전은 순수 텍스트(첨부·구조화는 범위 밖).
    pub body: String,
    /// ★회신 계약 인자(C3, spec §3)★ — 전부 선택이라 별도 struct 로 묶는다(`Default` = 통보 = 기존 동작).
    pub contract: SendContract,
}

/// ★회신 계약 발송 인자(C3 · spec §6 `send_message { …, request?, reply_by?, reply_to? }`)★.
///
/// ★왜 별도 struct 인가★: 세 인자는 전부 **선택**이고 기본값(`Default`)이 곧 "통보"(기존 v1 이전 동작)다.
///   `ControlCommand` 에 평평하게 늘어놓으면 plain 발송을 만드는 모든 자리(테스트·스모크 bin 포함)가 세
///   필드를 매번 써야 한다 — 묶어 두면 `contract: SendContract::default()` 한 줄이면 된다.
/// ★검증은 여기가 아니라 `handle_send`(입구 공통)★: 이 struct 는 **날것 그대로**를 나른다(파싱 안 함).
///   상호배타·기간 표기 유효성은 `validate_contract` 가 서비스 위임 전에 한 번만 본다(양 입구 동일 반려).
/// ★표기 매핑(spec §1)★: 여기 필드는 snake_case(`reply_by`/`reply_to`) — 봉투 XML 속성은 kebab-case
///   (`reply-by`/`in-reply-to`)로 렌더된다(`EnvelopeFields` 주석).
// ADR-0103 (결정 2/3 — request 타입 + 회신 계약)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendContract {
    /// `true` = 회신 요구(장부에 미회신 오픈 + 봉투 `type="request"`). 그룹 주소엔 v1 금지(spec §4).
    pub request: bool,
    /// 회신 기한 **기간 표기**(`"5m"`/`"10m"`/`"1h"`, 최소 1분). `request` 전용 — 단독 지정은 반려.
    pub reply_by: Option<String>,
    /// 어느 request 의 회신인가(원본 메시지 id). `request` 와 **상호배타**(spec §6).
    pub reply_to: Option<String>,
}

/// `reply_by` 기간 표기 상한 — 30일. 초과는 반려한다.
///
/// ★왜 상한이 필요한가(load-bearing — 데몬 안정성)★: 장부는 기한을 `created_at + reply_by`(`Instant +
///   Duration`)로 계산하는데, 이 덧셈은 **오버플로 시 패닉**한다. 상한이 없으면 에이전트가
///   `--reply-by 99999999999999s` 하나로 sweep task 를 죽일 수 있다(가용성 결함). 30일은 인메모리 단계의
///   현실적 상한이다 — 파킹 TTL 이 24h 이고 데몬 재시작이면 장부 자체가 소멸하므로 그보다 긴 기한은 의미가 없다.
const MAX_REPLY_BY_SECS: u64 = 30 * 24 * 60 * 60;

/// `reply_by` 하한 — 1분. 미만은 반려한다.
///
/// ★왜 하한이 필요한가(리뷰 fix 7 · load-bearing — 계약 정직성)★: 기한 **초과 판정은 sweep 주기(60초)에서만**
///   일어난다(lib.rs). 그래서 `30s` 를 받아 주면 실제 통지는 60~120초 뒤에 나간다 — 파서가 표현할 수 있는
///   정밀도와 데몬이 지킬 수 있는 정밀도가 어긋나, 발신자는 "30초 뒤 알려준다" 는 약속을 받고 두 배 넘게
///   기다린다. 지킬 수 없는 기한은 받지 않는 게 맞다(모호하면 반려 — parse_reply_by 의 기조와 동일).
///   `s` 단위 자체는 계속 받는다(`120s` 처럼 60 이상이면 유효) — 막는 건 **값의 크기**지 표기가 아니다.
/// ★sweep 주기와의 결합★: 이 값은 lib.rs 의 `SWEEP_INTERVAL`(60s)과 짝이다. 주기를 바꾸면 이 하한도 함께
///   봐야 한다(더 촘촘해지면 낮출 수 있다).
const MIN_REPLY_BY_SECS: u64 = 60;

/// ★`reply_by` 기간 표기 파서(C3 · spec §3 "기간 표기 10m/1h — 데몬이 절대시각 환산")★ — 엄격.
///
/// 허용 형태는 **정수 + 단위 1글자**뿐이다: `<digits>(s|m|h)` — 공백·부호·소수점·복합 표기(`1h30m`)·
/// 대문자 단위 전부 거부한다. 왜 관대하게 받지 않나: 이 값은 발신 LLM 이 자유롭게 쓰는 자리라 "받아는
/// 줬는데 뜻이 어긋나는" 해석(예: `"10"` 을 분으로 추측)이 조용한 계약 오차가 된다 — 모호하면 반려하고
/// 힌트로 형태를 알려 자기교정시킨다.
///
/// 반려: 빈 값 · 단위 없음/모름 · 숫자 아님 · `0` · u64 곱셈 오버플로 · 하한(1분) 미만 · 상한(30일) 초과.
/// (`0` 을 막는 이유: 기한 0 = 발송 즉시 초과라 notice 를 즉발한다 — 계약이 아니라 오타로 보는 게 안전하다.)
/// (하한 1분의 근거: `MIN_REPLY_BY_SECS` 주석 — 판정 해상도가 sweep 주기라 더 짧은 약속은 지킬 수 없다.)
// ADR-0103
pub(crate) fn parse_reply_by(s: &str) -> Result<std::time::Duration, String> {
    let bytes = s.as_bytes();
    // 최소 2바이트(숫자 1 + 단위 1). 바이트 단위로 자르므로 멀티바이트 문자는 단위 매치에서 자연히 걸린다.
    if bytes.len() < 2 {
        return Err(format!(
            "reply_by '{s}' is not a duration; use an integer with a unit like 5m, 10m, or 1h (minimum 1 minute)."
        ));
    }
    let (digits, unit) = bytes.split_at(bytes.len() - 1);
    let unit_secs: u64 = match unit[0] {
        b's' => 1,
        b'm' => 60,
        b'h' => 3600,
        _ => {
            return Err(format!(
                "reply_by '{s}' has an unknown unit; use s (seconds), m (minutes), or h (hours) — e.g. 10m."
            ))
        }
    };
    if !digits.iter().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "reply_by '{s}' must be a plain integer plus one unit (no spaces or signs) — e.g. 5m, 10m, 1h."
        ));
    }
    let n: u64 = std::str::from_utf8(digits)
        .expect("ascii digits")
        .parse()
        .map_err(|_| {
            format!("reply_by '{s}' is out of range; use at most 30d worth (e.g. 24h).")
        })?;
    if n == 0 {
        return Err(format!(
            "reply_by '{s}' must be greater than zero; drop the flag if you don't want a deadline."
        ));
    }
    let secs = n.checked_mul(unit_secs).ok_or_else(|| {
        format!("reply_by '{s}' is out of range; use at most 30d worth (e.g. 24h).")
    })?;
    if secs < MIN_REPLY_BY_SECS {
        return Err(format!(
            "reply_by '{s}' is shorter than the 1-minute minimum; deadlines are checked once a minute, so anything shorter cannot be honoured — use 1m or longer (60s is fine)."
        ));
    }
    if secs > MAX_REPLY_BY_SECS {
        return Err(format!(
            "reply_by '{s}' exceeds the 30-day maximum; use a shorter deadline (e.g. 24h)."
        ));
    }
    Ok(std::time::Duration::from_secs(secs))
}

/// ★C3 인자 정합 검증(spec §6 상호배타 · 양 입구 동일 반려)★ — 통과하면 서비스용 `SendMeta` 로 정규화한다.
///
/// 규칙(첫 위반에서 반려, 코드는 전부 `INVALID_SEND_ARGS`):
///   1. `request` + `reply_to` 동시 지정 → 반려. 한 메시지가 "새 요청" 이면서 "남의 요청에 대한 회신" 일 수
///      없다(spec §6 상호배타). 실제로 회신하며 새 요청을 걸고 싶으면 메시지 2건으로 나눈다.
///   2. `reply_by` 는 `request` 전용 — 단독 지정은 반려(기한만 있고 추적할 계약이 없다 = 조용한 무시가 되므로).
///   3. `reply_by` 표기 파싱 실패 → 반려(허용 형태를 hint 로).
///   4. `reply_to` 빈 문자열/공백만 → 반려(장부 매칭 키가 못 된다). 앞뒤 공백은 **잘라서** 받는다.
fn validate_contract(
    c: &SendContract,
) -> Result<engram_dashboard_messaging::service::SendMeta, String> {
    if c.request && c.reply_to.is_some() {
        return Err(
            "request and reply_to are mutually exclusive: a message is either a new request or a reply to one. Send two messages if you need both."
                .to_string(),
        );
    }
    if c.reply_by.is_some() && !c.request {
        return Err(
            "reply_by is only meaningful with request=true (it is the deadline of the reply contract you are opening). Add request, or drop reply_by."
                .to_string(),
        );
    }
    let reply_by = match &c.reply_by {
        Some(raw) => Some(parse_reply_by(raw)?),
        None => None,
    };
    let reply_to = match &c.reply_to {
        Some(raw) => {
            // ★입구 정규화로서의 trim(의도적 — 리뷰에서 제기됐으나 유지)★: 이 값은 발신 LLM 이 봉투에서
            //   눈으로 옮겨 적는 id 라 앞뒤 공백이 섞이기 쉽다. 여기서 한 번 다듬은 값이 **봉투 렌더와 장부
            //   엄격 매칭 양쪽에 그대로** 쓰이므로(SendMeta.reply_to 하나), 두 곳이 서로 다른 문자열을 볼
            //   여지가 없다 — 즉 trim 은 매칭 규칙을 느슨하게 만드는 게 아니라 **입구에서 표준형을 정하는**
            //   것이다. 다듬지 않으면 `" m-7f3k"` 가 같은 id 를 가리키는데도 NoMatch 로 조용히 빗나간다
            //   (엄격 매칭의 취지는 "틀린 id 는 안 닫는다" 이지 "공백 하나로 빗나간다" 가 아니다).
            let t = raw.trim();
            if t.is_empty() {
                return Err(
                    "reply_to must be the id of the request you are answering (e.g. m-7f3k9q2d); it cannot be empty."
                        .to_string(),
                );
            }
            Some(t.to_string())
        }
        None => None,
    };
    Ok(engram_dashboard_messaging::service::SendMeta {
        request: c.request,
        reply_by_raw: c.reply_by.clone(),
        reply_by,
        reply_to,
        // 그룹 라벨은 발신 인자가 아니다(수신자 지목 `to` 에서 파생) — 그룹 갈래는 `handle_group_send` 가
        //   자기 meta 를 만든다. 단일 발송 경로의 meta 는 항상 라벨 없음(= 봉투에 `to` 속성 없음).
        group: None,
    })
}

/// ★계약 필드(request/reply_to)가 **현재 봉투 렌더 경로에서 표현 불가**인가(C3 리뷰 fix 1 · load-bearing)★.
/// 불가면 교정 hint(`INVALID_SEND_ARGS` 용)를 돌려주고, 가능하면 `None`.
///
/// ★왜 반려까지 하나(조용한 열화 거부)★: 봉투 포맷 스위치는 두 갈래로 살아 있다 — 런타임
///   `SetEnvelopeFormat`(Colon, connection_core 경유로 실시간 전환) 과 스파이크용 `ENGRAM_WRAP_FORMAT`
///   템플릿(`wrap_message` 가 **format 인자보다 먼저** 본다). 두 갈래 모두 렌더가 `sender/id/body` 수준이라
///   **id·type·reply-by·in-reply-to 속성을 통째로 버린다**(`EnvelopeFields` 주석 "Colon 변형 미지원",
///   `apply_wrap_template` 의 플레이스홀더 집합).
///   그 상태에서 request 를 받아 주면 결말이 **정해져 있다**: 수신자는 회신에 쓸 id 를 본 적이 없는데 장부는
///   엄격 매칭을 요구하므로(spec §2) 회신이 구조적으로 불가능하고, 기한이 지나면 발신자에게 "회신 없음"
///   통지가 **반드시** 간다 — 거짓 타임아웃이 보장된 계약이다. 회신(`reply_to`)도 `in-reply-to` 가 사라져
///   수신자가 무엇에 대한 답인지 모른다. 그래서 열화 배달 대신 입구에서 반려한다.
/// ★통보는 영향 없음★: 속성이 없는 발송이라 어느 포맷에서도 그대로 나간다(기존 동작 불변).
/// ★env 를 여기서 읽는 이유★: 템플릿은 `wrap_message` 가 **최우선**으로 보는 전역 스위치라, 이 판정이
///   registry 포맷만 보면 템플릿이 켜진 프로세스에서 그대로 새 나간다. 판정과 렌더가 같은 입력을 봐야 한다.
// ADR-0103
fn contract_unsupported_by_envelope(
    meta: &engram_dashboard_messaging::service::SendMeta,
    format: EnvelopeFormat,
) -> Option<String> {
    if !(meta.request || meta.reply_to.is_some()) {
        return None;
    }
    let template_active = std::env::var("ENGRAM_WRAP_FORMAT").is_ok_and(|v| !v.is_empty());
    if format == EnvelopeFormat::Xml && !template_active {
        return None;
    }
    Some(
        "Reply contracts (request / reply_to) need the XML envelope: the envelope format in effect drops the id / type / reply-by / in-reply-to attributes, so the recipient could never reply and you would get a false timeout notice. Send this as a plain notification instead, or switch the envelope format back to xml."
            .to_string(),
    )
}

/// 한 수신자에 대한 발송 결과(spec §6 `results[]` 원소). status = delivered|pending|skipped, hint 선택.
///
/// ★spec §6 shape★: 발송 성공 응답은 `{ id, results: [{to, status, hint?}] }` 다. 단일 발송은 길이 1,
///   그룹 방송(C4)은 **멤버당 한 줄**이라 길이 N 이다(그룹 단위 요약이 아니라 멤버별 회계 — spec §6).
#[derive(Debug, Clone)]
pub struct SendResult {
    /// 수신자 이름 — 단일 발송이면 발신자가 지목한 이름, 그룹 방송이면 **멤버 이름**(그룹 이름 아님).
    pub to: String,
    /// `"delivered"`(실제 주입) · `"pending"`(파킹) · `"skipped"`(그룹 방송에서 배달 안 함 — 부재/동명
    ///   다수/보관함 가득) — spec §4·§5 상태 어휘. `skipped` 는 그룹 방송에서만 나온다.
    pub status: &'static str,
    /// 자기교정용 힌트(파킹 사유 등). None 이면 응답에서 생략.
    pub hint: Option<String>,
}

/// 제어 커맨드 처리 결과 — 성공(수신자별 결과 배열) 또는 교정 에러(반려). 두 입구 모두 이 값을 그대로
/// JSON 직렬화해 열린 요청에 돌려준다(동일 shape 보장, spec §6). `to_json` 이 wire JSON 을 만든다.
///
/// ★spec §6 응답 계약(ADR-0103 — 옛 "enqueued" 폐기)★: 성공 = `{ id, results: [{to, status, hint?}] }`,
///   반려 = `{ status:"error", code, hint }`. S18 메시징 v1 이 파킹(pending)을 도입하며 옛 단일-상태
///   "enqueued" ACK 를 이 다중-결과 shape 로 교체했다(성공에 delivered/pending 이 섞일 수 있으므로).
#[derive(Debug, Clone)]
pub enum ControlResult {
    /// 발송 접수 성공 — 논리 메시지 id + 수신자별 결과 배열(spec §6). delivered/pending 이 섞일 수 있다.
    Ok {
        id: String,
        results: Vec<SendResult>,
    },
    /// 교정 에러(반려) — code + hint(자기교정용). 발신자가 이걸 보고 재시도한다.
    Error { code: &'static str, hint: String },
}

impl ControlResult {
    /// wire JSON(serde_json::Value). 두 입구가 이 값을 직렬화해 응답 body/툴 결과로 쓴다(spec §6 shape).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ControlResult::Ok { id, results } => {
                let arr: Vec<serde_json::Value> = results
                    .iter()
                    .map(|r| {
                        let mut obj = serde_json::json!({ "to": r.to, "status": r.status });
                        if let Some(h) = &r.hint {
                            obj["hint"] = serde_json::Value::String(h.clone());
                        }
                        obj
                    })
                    .collect();
                serde_json::json!({ "id": id, "results": arr })
            }
            ControlResult::Error { code, hint } => serde_json::json!({
                "status": "error",
                "code": code,
                "hint": hint,
            }),
        }
    }

    /// 발송 접수 성공(반려 아님)인가 — CLI 가 exit code(0/1) 매핑에 쓴다. delivered·pending 모두 성공.
    /// ★pending 도 성공★: 파킹은 "데몬이 접수해 배달 보장"(등장 시 flush)이라 발신자에겐 성공이다 —
    ///   반려(MAILBOX_FULL/AMBIGUOUS 등)만 실패로 본다(spec §5·§6).
    pub fn is_accepted(&self) -> bool {
        matches!(self, ControlResult::Ok { .. })
    }
}

/// 수신자 해석 결과(Validator 내부). 성공 시 산 세션의 (id, 표시이름), 실패 시 교정 에러.
///
/// ★C1 이후 Ok 필드는 handle_send 비-테스트 경로에서 안 읽힌다★: handle_send 는 이제 `Resolution::Err`
///   의 AMBIGUOUS 여부만 보고(부재·해석은 MessagingService 가 재수행), Ok 의 id/name 을 소비하지 않는다.
///   그 필드는 ingress 단위 테스트(`resolve_by_unique_name` 등)가 여전히 읽으므로 유지한다 — 비-테스트
///   빌드의 dead_code 경고만 억제한다(로직 삭제가 아니라 관측 표면 유지).
#[allow(dead_code)]
enum Resolution {
    Ok { id: AgentId, name: String },
    Err(ControlResult),
}

/// ★듀얼 입구 공통 핸들러(ADR-0086 · S18 메시징 v1 C1)★: 정규화된 ControlCommand → Validator → 발송
/// 3분기(MessagingService) → 응답(spec §6). 두 어댑터(MCP 툴 · HTTP 라우트)가 유일하게 부르는 진입점이다
/// — 이 아래는 입구를 모른다(entrance-agnostic).
///
/// 검사 순서(첫 실패에서 교정 에러 반환 — 같은 shape 양 입구):
///   0. ★C3 회신 계약 인자 정합★ → INVALID_SEND_ARGS(상호배타·reply_by 단독·표기 오류·빈 reply_to).
///      주소보다 **먼저** 본다: 순수 구문 오류라 로스터 상태와 무관하게 항상 같은 답이 나와야 한다.
///   1. ★그룹 주소(`@`) → fan-out 위임(C4)★. 단 계약 필드는 금지다: request → GROUP_REQUEST_UNSUPPORTED
///      (spec §4 v1 영구 금지) · reply_to → INVALID_SEND_ARGS(전체회신 없음 — 회신은 발신자 1인에게).
///      해석 실패는 GROUP_NOT_FOUND / GROUP_EMPTY(`handle_group_send`).
///   2. body 상한(64 KiB) → BODY_TOO_LARGE.
///   3. ★동명 다수 → RECIPIENT_AMBIGUOUS(유지, spec §5 주의)★ — 산 로스터에 같은 이름이 여럿이면
///      파킹/배달 이전에 반려한다(발신자가 exact id 로 재지목). 파킹 대상이 모호하면 안 되므로 여기서 먼저.
///   4. ★그 외 전부 MessagingService 위임(spec §5 3분기)★:
///      - 산·도달 수신자 해석 + inject 성공 → `delivered`.
///      - 부재("없는 이름" 포함)·inject 실패 → **파킹** = `pending`(RECIPIENT_NOT_FOUND 소멸, spec §5 주의).
///      - cap 초과 → `MAILBOX_FULL` 반려.
///
/// ★발신자 생존 관측(기록용만 — 게이트 아님, 사용자 결정 2026-07-19)★: relay 직전에 발신자가 아직 산
///   신원인지 registry 로 조회하되 죽었어도 거부하지 않는다(작성 시점 인증으로 유효). 죽은 발신자 배달은
///   forensic 로그만 남긴다. 이 관측은 delivered 갈래에만 의미가 있어 여기(위임 전)서 한 번 남긴다.
/// ★self-send 허용★: to == 발신자여도 특수 처리 없이 정상 배달(테스트·자가 메시지 유용 — ADR-0086 §7).
/// ★락 규율(ADR-0006)★: 여기선 manager.list_agents(동명 검사)만 직접 부른다(내부에서 sessions lock 을
///   clone 후 즉시 해제). 파킹/주입/장부는 MessagingService 가 자기 단일 락 규율로 처리한다(그 락을 든 채
///   manager 를 부르지 않음 — service.rs 헤더).
// ADR-0086
// ADR-0103
pub fn handle_send(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    entrance: Entrance,
    cmd: ControlCommand,
) -> ControlResult {
    // ★trim 은 **그룹 축에만** 적용한다(round-3 fix 4 — C4 의 무조건 trim 을 좁힘 · load-bearing)★.
    //
    // 원래 고치려던 결함은 **판정들 사이의 불일치** 하나였다: 그룹 갈래는 `starts_with('@')` 로 raw 를 보고
    //   그룹 이름 정규화(groups::normalize_group_name)는 trim 한 값을 본다 → `" @all"` 이 "@ 로 시작하지
    //   않으니 단일 발송" 으로 흘러 **그런 이름 없음 → 부재 파킹**이 됐다(발신자에겐 `pending` 성공인데
    //   아무도 못 받고 TTL 에 소멸 = 공백 한 칸 뒤에 숨은 조용한 유실). 그 불일치는 **그룹 감지와 그룹
    //   해석이 같은 문자열을 보게** 하면 사라진다.
    // ★그런데 `cmd.to` 자체를 덮어쓰면 과교정이다★: 단일 수신자 주소는 **바이트 그대로의 이름 네임스페이스**
    //   다(WYSIWYA — ADR-0101). 앞뒤 공백이 붙은 canonical 이름이 실재하면 무조건 trim 은 그 수신자를 **다른
    //   이름으로 재지목**해 버리고(극단적으로는 trim 한 값이 산 AgentId 문자열이면 엉뚱한 에이전트로 배달),
    //   파킹 키·장부 키·관측 레코드의 `to` 까지 발신자가 쓰지 않은 값으로 바뀐다. 주소 정규화는 이 입구가
    //   임의로 내릴 결정이 아니다.
    // 그래서: **trim 한 값은 ① `@` 감지 ② 그룹 해석 ③ 봉투 그룹 라벨에만** 쓰고, 단일 수신자 해석·관측·
    //   장부 키·응답 `to` 는 **원본 `cmd.to`** 를 그대로 쓴다(= C4 이전 동작 그대로).
    // ★body 는 trim 하지 않는다★: 본문의 앞뒤 공백은 발신자가 의도한 내용일 수 있다(코드 블록 들여쓰기 등).
    // ADR-0101
    let to_trimmed = cmd.to.trim();

    // 0. C3 회신 계약 인자 정합(순수 구문 — 로스터 무관). 통과분은 파싱된 SendMeta 로 정규화된다.
    let meta = match validate_contract(&cmd.contract) {
        Ok(m) => m,
        Err(hint) => {
            return ControlResult::Error {
                code: "INVALID_SEND_ARGS",
                hint,
            }
        }
    };

    // 0-b. ★계약 필드는 XML 봉투 전용(리뷰 fix 1)★ — 판정 정본은 `contract_unsupported_by_envelope`.
    if let Some(hint) = contract_unsupported_by_envelope(&meta, registry.envelope_format()) {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint,
        };
    }

    // 1. 그룹 주소(@) — 계약 필드는 금지, 그 외는 fan-out(C4).
    // ★두 금지는 C4 이후에도 남는다(load-bearing)★:
    //   - **request** — 완료 판정(any/all/quorum) 시맨틱이 미정이라 v1 에서 **영구 금지**(spec §4 · ADR-0103
    //     거부 대안 "그룹 request(v1)"). 방송을 켠다고 함께 열리면 안 되는 갈래다.
    //   - **reply_to** — "회신은 항상 발신자 1인에게(전체회신 없음)"(spec §4). 그룹 주소로 회신을 보내면
    //     그게 곧 reply-all 이라 계약이 뒤집힌다. 조용히 무시(라벨만 떼고 방송)하면 발신자는 회신했다고
    //     믿는데 원 요청자는 아무 것도 못 닫으므로, 입구에서 반려해 발신자가 1:1 로 다시 보내게 한다.
    //   두 반려는 **배달·장부 부작용 0** 지점(body 상한·로스터 조회 전)에서 끝난다.
    // 그룹 감지·해석·라벨은 **trim 한 값**을 본다(위 정규화 규약 — 감지와 해석이 같은 문자열을 봐야 한다).
    if to_trimmed.starts_with('@') {
        if meta.request {
            return ControlResult::Error {
                code: "GROUP_REQUEST_UNSUPPORTED",
                hint: format!(
                    "Requests must have exactly one recipient; '{to_trimmed}' is a group address. Send the request to a single agent name (a plain broadcast to the group is a separate message)."
                ),
            };
        }
        if meta.reply_to.is_some() {
            return ControlResult::Error {
                code: "INVALID_SEND_ARGS",
                hint: format!(
                    "Replies always go to one agent — the sender of the request; '{to_trimmed}' is a group address (there is no reply-all). Send the reply to that agent's name, and broadcast any follow-up separately."
                ),
            };
        }
        // body 상한은 그룹도 같이 받는다(수신자 수와 무관한 순수 구문 상한) — 아래 단일 경로와 같은 코드.
        if cmd.body.len() > MAX_BODY_BYTES {
            return ControlResult::Error {
                code: "BODY_TOO_LARGE",
                hint: format!(
                    "Message body exceeds the {MAX_BODY_BYTES}-byte limit; shorten it and retry."
                ),
            };
        }
        return handle_group_send(
            manager, registry, messaging, entrance, &cmd, to_trimmed, &meta,
        );
    }

    // 2. body 상한.
    if cmd.body.len() > MAX_BODY_BYTES {
        return ControlResult::Error {
            code: "BODY_TOO_LARGE",
            hint: format!(
                "Message body exceeds the {MAX_BODY_BYTES}-byte limit; shorten it and retry."
            ),
        };
    }

    // 3. 동명 다수 → RECIPIENT_AMBIGUOUS(파킹/배달 전에 — spec §5 "AMBIGUOUS 유지"). 산 로스터 스냅샷 1회.
    //    ★왜 여기서(위임 전)★: 파킹은 이름 기반이라 동명이 여럿이면 어느 큐로 갈지 모호하다 — 반려로 발신
    //    자가 exact id 로 재지목하게 한다. `to` 가 exact AgentId 이거나 유일 이름이면 통과(service 가 처리).
    let agents = manager.list_agents();
    if let Resolution::Err(e) = resolve_recipient(&cmd.to, &agents) {
        // resolve_recipient 는 부재를 NOT_FOUND 로 내지만, C1 은 부재를 **파킹**으로 처리한다 —
        //   그래서 여기선 **AMBIGUOUS 만** 조기 반환하고, NOT_FOUND(부재)는 아래 위임으로 흘려 파킹시킨다.
        if let ControlResult::Error { code, .. } = &e {
            if *code == "RECIPIENT_AMBIGUOUS" {
                return e;
            }
        }
    }

    // ★발신자 생존 관측(기록용만 — 게이트 아님, 사용자 결정 2026-07-19)★: 죽은 발신자여도 배달·파킹은
    //   진행한다("결과 보내고 종료" 유언 패턴 + 파킹 커밋 시맨틱). body/토큰 미로깅(보안).
    // ★id = `m-` + base36 8자(C3, spec §1)★ — 수신 LLM 이 회신에 되받아 적는 값이라 짧은 불투명 id 다
    //   (옛 UUID 폐기 — new_msg_id 주석). 봉투 `id` 속성·장부 키·응답 `id`·관측 레코드가 전부 이 값이다.
    let mut msg_id = new_msg_id();
    if !registry.is_identity_live(cmd.from) {
        tracing::warn!(
            from = %cmd.from.agent_id,
            from_epoch = cmd.from.epoch,
            msg_id = %msg_id,
            entrance = entrance.as_str(),
            "제어 채널 메시지 발송 — 발신자가 relay 시점에 더 이상 산 신원 아님(작성 시점 인증으로 유효, 게이트 아님·기록용 관측, ADR-0086·사용자 결정 2026-07-19)"
        );
    }

    // 봉투 sender 표시 이름 = canonical(WYSIWYA ADR-0101) — 라우팅/로스터가 보는 이름과 byte-identical.
    let sender_name = sender_display_name(manager, cmd.from);

    // 4. 발송 3분기 위임(spec §5). MessagingService 가 resolve/inject/park/ledger 를 소유한다.
    //    ★mid-send yield-seam(ADR-0088)은 이제 MessagingService.inject 경로가 아니라 이 위임 직전이
    //      resolve↔inject 갭의 관측 지점이다★ — test-harness 전용이라 운영 빌드엔 컴파일 안 됨. hook 을
    //      여기서 발화해 위임 안 inject 착지 incarnation 을 결정적으로 관측한다(동작 byte-identical when OFF).
    #[cfg(feature = "test-harness")]
    registry.fire_mid_send_hook();

    // ★id 충돌 재시도(C3 — new_msg_id 주석의 "새 id 로 1회 재시도")★: request 발송은 장부에 계약을 여는데,
    //   그 예약이 `DuplicateId` 로 튕기면(2.8×10^12 공간에서 사실상 불가) 서비스는 **부작용 없이**
    //   `IdCollision` 만 돌려준다(예약이 첫 부작용이라 배달·파킹은 아직 없다). 그러니 여기서 새 id 를 뽑아
    //   딱 한 번 더 시도하고, 두 번째도 걸리면 내부 결함으로 보고 반려한다(무한 재시도 금지 — 그 지점이면
    //   충돌이 아니라 장부/난수 배선 버그다).
    let mut outcome = messaging.handle_single_send(
        &msg_id,
        cmd.from.into(),
        &sender_name,
        &cmd.to,
        &cmd.body,
        entrance,
        &meta,
    );
    if matches!(
        outcome,
        Err(engram_dashboard_messaging::service::SendReject::IdCollision)
    ) {
        // ★로그는 **충돌한 id** 를 찍는다(리뷰 fix 10)★: 예전엔 재생성 **뒤** 찍어서 대체 id(=아직 아무
        //   일도 없던 값)만 남았다 — 장부에서 무엇과 부딪혔는지 추적할 단서가 사라진다. 조사 가치는 옛
        //   id 에 있으므로 그걸 먼저 붙들고, 대체 id 는 상관용으로 함께 남긴다.
        let collided = std::mem::replace(&mut msg_id, new_msg_id());
        tracing::error!(
            collided = %collided,
            replacement = %msg_id,
            entrance = entrance.as_str(),
            "메시지 id 충돌 — 새 id 로 1회 재시도(ADR-0103 · 사실상 불가한 경로라 난수/장부 배선을 의심할 것)"
        );
        outcome = messaging.handle_single_send(
            &msg_id,
            cmd.from.into(),
            &sender_name,
            &cmd.to,
            &cmd.body,
            entrance,
            &meta,
        );
    }

    match outcome {
        Ok(engram_dashboard_messaging::service::SendOutcome::Delivered) => ControlResult::Ok {
            id: msg_id,
            results: vec![SendResult {
                to: cmd.to,
                status: "delivered",
                hint: None,
            }],
        },
        Ok(engram_dashboard_messaging::service::SendOutcome::Parked { hint }) => ControlResult::Ok {
            id: msg_id,
            results: vec![SendResult {
                to: cmd.to,
                status: "pending",
                hint: Some(hint),
            }],
        },
        Err(engram_dashboard_messaging::service::SendReject::MailboxFull) => ControlResult::Error {
            code: "MAILBOX_FULL",
            hint: format!(
                "Recipient '{}' mailbox is full; oldest parked messages expire by TTL — retry later.",
                cmd.to
            ),
        },
        Err(engram_dashboard_messaging::service::SendReject::IdCollision) => ControlResult::Error {
            code: "INTERNAL_ID_COLLISION",
            hint: "The daemon could not allocate a unique message id; retry the send.".to_string(),
        },
        // ★오픈 계약 상한(리뷰 fix 3)★ — request 만 받는 반려. 코드는 안정 계약이므로 새 이름을 만들지
        //   말고 이 값을 쓴다(발신 LLM 이 코드로 분기한다).
        Err(engram_dashboard_messaging::service::SendReject::RequestCapacity) => ControlResult::Error {
            code: "REQUEST_CAPACITY",
            hint: "Too many replies are still outstanding; the daemon is not tracking new requests right now. Send this as a plain notification, or wait for earlier requests to be answered or to time out."
                .to_string(),
        },
    }
}

/// ★그룹 방송 입구(C4 · spec §4·§6)★ — `handle_send` 의 `@` 갈래가 계약 필드 금지·body 상한을 통과시킨 뒤
/// 부른다. 여기 책임은 **얇다**: id 부여 + 서비스 위임 + 멤버별 결과를 wire JSON shape 으로 옮기는 것뿐이고,
/// 스냅샷·해석·회계는 전부 `MessagingService::handle_group_send` 가 소유한다(입구는 정책을 모른다).
///
/// ★응답 = 멤버당 한 줄(spec §6)★: `{ id, results: [{to: 멤버이름, status, hint?}] }` — `to` 는 그룹 이름이
///   아니라 **멤버 이름**이다. 그룹 단위 반려(NOT_FOUND/EMPTY)는 성공 축이 아니라 `{status:"error"}` 다.
/// ★에러 코드 어휘는 spec §4 고정★: 이름 규약 위반(`@`·`@@x`)도 새 코드를 만들지 않고 `GROUP_NOT_FOUND` 로
///   답한다 — 발신자에게 맞는 사실은 "그런 그룹은 없다" 이고, 교정 방법은 hint 가 알려 준다.
/// ★id 충돌 재시도★: 단일 발송과 **같은 규율**(새 id 로 1회 재시도 후 반려) — 서비스가 부작용 0 상태에서
///   `IdCollision` 을 돌려주기 때문에 그대로 다시 부를 수 있다.
// ADR-0103 (결정 4 — 그룹 방송)
fn handle_group_send(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    entrance: Entrance,
    cmd: &ControlCommand,
    // 그룹 주소 — 호출자(`handle_send`)가 `cmd.to` 를 trim 한 값이다. ★`cmd.to` 를 여기서 다시 읽지 않는
    //   이유★: 그러면 감지(trim 한 값)와 해석(raw)이 다시 갈려 원래 결함이 되살아난다. 그룹 축의 단일
    //   문자열은 이 인자다(정규화 자체는 서비스의 `normalize_group_name` 이 한 번 더 한다 — 라벨 단일 출처).
    group: &str,
    // 검증된 메타 — 그룹 갈래에선 **계약 필드가 비어 있음이 이미 확인된 값**이다. 서비스에 그대로 넘겨
    //   그쪽 debug_assert 가 배선 실수를 잡게 한다(단일 발송 guard 와 대칭 — service.rs handle_group_send).
    meta: &engram_dashboard_messaging::service::SendMeta,
) -> ControlResult {
    use engram_dashboard_messaging::service::{GroupMemberStatus, GroupReject};

    // 발신자 생존 관측(기록용만 — 단일 발송과 같은 규율, 게이트 아님).
    let mut msg_id = new_msg_id();
    if !registry.is_identity_live(cmd.from) {
        tracing::warn!(
            from = %cmd.from.agent_id,
            from_epoch = cmd.from.epoch,
            msg_id = %msg_id,
            entrance = entrance.as_str(),
            "그룹 방송 발송 — 발신자가 relay 시점에 더 이상 산 신원 아님(작성 시점 인증으로 유효, 기록용 관측)"
        );
    }
    let sender_name = sender_display_name(manager, cmd.from);

    let mut outcome = messaging.handle_group_send(
        &msg_id,
        cmd.from.into(),
        &sender_name,
        group,
        &cmd.body,
        entrance,
        meta,
    );
    if matches!(outcome, Err(GroupReject::IdCollision)) {
        // 로그는 **충돌한 id** 를 찍는다(단일 발송 재시도와 같은 규율 — 조사 단서는 옛 id 쪽에 있다).
        let collided = std::mem::replace(&mut msg_id, new_msg_id());
        tracing::error!(
            collided = %collided,
            replacement = %msg_id,
            entrance = entrance.as_str(),
            "그룹 메시지 id 충돌 — 새 id 로 1회 재시도(ADR-0103 · 사실상 불가한 경로)"
        );
        outcome = messaging.handle_group_send(
            &msg_id,
            cmd.from.into(),
            &sender_name,
            group,
            &cmd.body,
            entrance,
            meta,
        );
    }

    match outcome {
        Ok(members) => ControlResult::Ok {
            id: msg_id,
            results: members
                .into_iter()
                .map(|m| SendResult {
                    to: m.to,
                    status: match m.status {
                        GroupMemberStatus::Delivered => "delivered",
                        GroupMemberStatus::Pending => "pending",
                        GroupMemberStatus::Skipped => "skipped",
                    },
                    hint: m.hint,
                })
                .collect(),
        },
        Err(GroupReject::NotFound { name }) => ControlResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: format!(
                "No group named '{name}' is registered. Create it first (group add), or use the built-in @all."
            ),
        },
        Err(GroupReject::Empty { name }) => ControlResult::Error {
            code: "GROUP_EMPTY",
            hint: format!(
                "Group '{name}' resolved to no members right now — nothing was sent. Add members to it, or wait until the agents you want are running (for @all, that means someone other than you is live and reachable)."
            ),
        },
        // 이름 규약 위반도 "그런 그룹 없음" 으로 답한다(spec §4 어휘 고정 — 새 코드 금지). hint 가 규약을 알려준다.
        Err(GroupReject::InvalidName { name }) => ControlResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: format!(
                "'{name}' is not a valid group address: a group is exactly one leading '@' plus a name (e.g. @coders). Use @all to reach everyone live."
            ),
        },
        Err(GroupReject::IdCollision) => ControlResult::Error {
            code: "INTERNAL_ID_COLLISION",
            hint: "The daemon could not allocate a unique message id; retry the send.".to_string(),
        },
    }
}

// ── 조회·관리 입구(D · spec §6 `messages` / `group`) ──────────────────────────────────────
//
// ★같은 규율, 다른 동사(ADR-0086 entrance-agnostic)★: `handle_send` 와 마찬가지로 MCP 툴과 CLI HTTP 라우트가
//   **이 함수들만** 부른다 — 두 입구의 응답 JSON 이 같은 코드에서 나오므로 갈릴 수 없다(spec §6 "두 입구 동일
//   JSON"). 신원 역시 두 입구 모두 토큰/세션 파생 `BoundIdentity` 다(payload 신원 금지).
// ★읽기 전용/부작용 최소★: `messages` 는 장부를 **바꾸지 않는다**(조회 전용 — 전이·닫기 없음). `group` 은
//   명단만 바꾸고 메시지·장부·큐를 건드리지 않는다(스냅샷 원칙 — service.rs 그룹 관리 섹션).
// ★spawn_blocking 을 쓰지 않는 이유★: 이 경로엔 자식 stdin blocking write 가 없다(inject 없음). 잡는 락은
//   messaging state 하나이고 그 임계구역은 순수 자료구조 조작뿐이라(port 호출은 락 밖) 짧다 — async 워커를
//   붙들지 않는다. `handle_send` 가 blocking 풀로 가는 이유(막힌 파이프)가 여기엔 성립하지 않는다.

/// 조회·관리 커맨드의 결과 — 성공(임의 JSON 객체) 또는 교정 에러. 발송(`ControlResult`)과 **에러 shape 이
/// 동일**하다(`{status:"error", code, hint}` — spec §6): 발신 LLM·CLI 가 성공/실패를 한 규칙으로 읽는다.
///
/// ★왜 발송과 다른 타입인가★: 발송 성공은 `{id, results[]}` 로 **shape 이 고정**돼 있지만(계약), 조회 성공은
///   동사마다 모양이 다르다(메시지 상태 / 미결 목록 / 그룹 목록 / 멤버 목록). 억지로 한 enum 에 넣으면
///   variant 가 동사마다 늘어 계약이 흐려지므로, 성공 payload 는 만든 쪽이 통째로 싣고 **에러 축만** 공유한다.
// PartialEq(Eq 아님 — serde_json::Value 가 f64 를 담아 Eq 를 못 준다)는 단위 테스트가 결과를 통째로
//   비교하려고 붙였다. wire 계약엔 영향 없다.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlQueryResult {
    /// 조회/관리 성공 — 동사별 payload(아래 각 핸들러 doc-comment 가 shape 정본).
    Ok(serde_json::Value),
    /// 교정 에러(반려) — 발송과 같은 code/hint 규약.
    Error { code: &'static str, hint: String },
}

impl ControlQueryResult {
    /// wire JSON. 성공은 payload 그대로, 에러는 `{status:"error", code, hint}`(발송과 동일 shape).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ControlQueryResult::Ok(v) => v.clone(),
            ControlQueryResult::Error { code, hint } => serde_json::json!({
                "status": "error",
                "code": code,
                "hint": hint,
            }),
        }
    }

    /// 성공인가 — CLI 가 exit code(0/1) 매핑에 쓴다.
    pub fn is_ok(&self) -> bool {
        matches!(self, ControlQueryResult::Ok(_))
    }
}

/// ★`messages { id? }` 공통 핸들러(D · spec §6)★ — **읽기 전용**. 장부를 조회만 하고 어떤 상태도 바꾸지 않는다.
///
/// ★응답 shape(계약 — 두 입구 동일)★
///
/// `id` 지정 = 그 메시지의 배달 장부. 그룹 방송은 **수신자별 1행**(spec §4 1 msg_id : N 배달기록):
/// ```json
/// { "id": "m-7f3k", "from": "alice", "awaiting_reply": false, "may_be_truncated": false,
///   "rows": [ { "to": "bob", "status": "delivered", "age_secs": 42, "updated_secs_ago": 40 } ] }
/// ```
/// - `status` 어휘 = 발송 응답과 **같은 집합**(`pending|delivered|replied|expired|skipped`).
/// - `awaiting_reply` = 이 메시지가 request 인데 아직 회신이 안 왔다(통보면 항상 false).
/// - ★`may_be_truncated`(리뷰 B2)★ = `rows` 가 **그 메시지의 전부라는 보장이 없다**(인메모리 이력 링이
///   밀려 앞쪽 행이 사라졌을 수 있다). `false` 면 확실히 전부다. `true` 일 때는 `hint` 도 함께 실어
///   사람/LLM 이 읽을 문장으로도 알린다 — 이 필드가 없으면 조회자가 남은 행을 전체로 오독한다(10인
///   방송의 앞 6행이 밀려나면 "4명에게만 나갔다" 로 읽힌다).
/// - 없는 id → `{status:"error", code:"MESSAGE_NOT_FOUND", hint}`. 단 이력이 통째로 밀려났어도 **회신
///   계약이 살아 있으면** 행 0줄 + `awaiting_reply:true` + `may_be_truncated:true` 로 답한다(무인자
///   조회가 미결로 보여 주는 id 를 여기선 "없다" 고 하는 자기모순 제거).
///
/// `id` 없음 = **호출자의 미결**(세 갈래를 한 목록으로, 오래된 순):
/// ```json
/// { "me": "alice", "open": [
///     { "direction": "outbound_pending",     "id": "m-1", "from": "alice", "to": "ghost", "age_secs": 90 },
///     { "direction": "awaiting_their_reply", "id": "m-2", "from": "alice", "to": "bob",   "age_secs": 30,
///       "reply_by": "10m", "timed_out": false },
///     { "direction": "reply_owed_by_me",     "id": "m-3", "from": "carol", "to": "alice", "age_secs": 5 } ] }
/// ```
/// - `direction` = 이 줄이 무엇인지(안정 토큰). 세 값의 **할 일이 정반대**라 이 태그가 필수다:
///   `outbound_pending`(내 발송이 아직 안 꽂힘 — 기다림) · `awaiting_their_reply`(내 요청의 회신 대기 —
///   기다림) · `reply_owed_by_me`(**내가 지금 답해야 함**).
/// - `reply_by`·`timed_out` 은 request 줄에만 실린다(통보 줄에선 생략 — 노출 원칙과 같은 정신).
/// - 미결이 없으면 `open: []`.
///
/// ★시각을 절대시각이 아니라 경과 초로 주는 이유★: 장부 시각은 단조 시계(`Instant`)라 벽시계 값이 없다
///   (spec §5 — 상태 전이 시각은 상대 비교용). 절대시각을 내려면 장부에 새 시간 축을 들여야 하는데 v1 범위
///   밖이다. "3분 전" 은 수신 LLM 에게도 타임스탬프보다 바로 쓸모 있다(`MessagingService::message_state` 주석).
/// ★호출자 이름 = canonical(WYSIWYA — ADR-0101)★: 장부는 이름으로 기록되므로 신원(BoundIdentity)을 발송
///   봉투와 **같은 계산**으로 표시 이름으로 바꿔 매칭한다(`sender_display_name` 재사용 — 두 곳이 갈리면
///   "내 미결" 이 남의 것으로 보이거나 통째로 비어 버린다).
// ADR-0086 (듀얼 입구 공통 핸들러)
// ADR-0103 (spec §6 messages)
pub fn handle_messages(
    manager: &Arc<AgentManager>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    from: BoundIdentity,
    id: Option<&str>,
) -> ControlQueryResult {
    let now = std::time::Instant::now();
    match id {
        Some(raw) => {
            // ★trim 은 여기서 한다(입구 정규화 — reply_to 와 같은 규율)★: 이 값도 발신 LLM 이 봉투에서 눈으로
            //   옮겨 적는 id 라 앞뒤 공백이 섞이기 쉽다. 조회는 부작용이 없으므로 관대해도 안전하다.
            let msg_id = raw.trim();
            if msg_id.is_empty() {
                return ControlQueryResult::Error {
                    code: "MESSAGE_NOT_FOUND",
                    hint: "Pass the message id you want to inspect (e.g. m-7f3k9q2d), or call this tool with no arguments to list your own open items.".to_string(),
                };
            }
            let Some(view) = messaging.message_state(msg_id, now) else {
                return ControlQueryResult::Error {
                    code: "MESSAGE_NOT_FOUND",
                    hint: format!(
                        "No message '{msg_id}' in the ledger. Ids look like m-7f3k9q2d; very old messages fall out of the in-memory history, and the ledger is cleared when the daemon restarts."
                    ),
                };
            };
            let rows: Vec<serde_json::Value> = view
                .rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "to": r.to,
                        "status": r.status,
                        "age_secs": r.age_secs,
                        "updated_secs_ago": r.updated_secs_ago,
                    })
                })
                .collect();
            let mut out = serde_json::json!({
                "id": view.id,
                "from": view.from,
                "awaiting_reply": view.awaiting_reply,
                // ★항상 싣는다(리뷰 B2)★: `false` 는 "확실히 전부" 라는 **적극적 완전성 단언**이라 그 자체로
                //   정보다. 참일 때만 실으면 필드의 부재가 완전성으로 읽혀 같은 오독이 남는다.
                "may_be_truncated": view.may_be_truncated,
                "rows": rows,
            });
            if view.may_be_truncated {
                // 기계 판독용 불리언 옆에 사람/LLM 이 읽을 문장을 붙인다(발송 반려의 hint 와 같은 역할).
                out["hint"] = serde_json::Value::String(
                    "The in-memory ledger has rotated, so some delivery rows for this message may already be gone — treat the list below as partial, not as the full set of recipients.".to_string(),
                );
            }
            ControlQueryResult::Ok(out)
        }
        None => {
            let me = sender_display_name(manager, from);
            let open: Vec<serde_json::Value> = messaging
                // 의무 귀속은 이름이 아니라 신원(AgentId)으로 가른다(리뷰 B1 — 동명 다수 오귀속 차단).
                .open_items_for(&me, from.agent_id, now)
                .iter()
                .map(|i| {
                    let mut obj = serde_json::json!({
                        "direction": i.direction.as_str(),
                        "id": i.id,
                        "from": i.from,
                        "to": i.to,
                        "age_secs": i.age_secs,
                    });
                    // 계약 축은 request 줄에만 — 통보 줄에 `reply_by: null` 을 실으면 노출 원칙이 흐려진다.
                    if let Some(rb) = &i.reply_by {
                        obj["reply_by"] = serde_json::Value::String(rb.clone());
                    }
                    if i.reply_by.is_some() || i.timed_out {
                        obj["timed_out"] = serde_json::Value::Bool(i.timed_out);
                    }
                    obj
                })
                .collect();
            ControlQueryResult::Ok(serde_json::json!({ "me": me, "open": open }))
        }
    }
}

/// `group` 커맨드 인자(D · spec §6 `group { group?, add?, remove?, delete? }`). 두 입구가 이 형태로 정규화한다.
///
/// ★신원 없음(사용자 결정 2026-07-26)★: ACL 도 행위자 기록도 없다 — 누구나 어떤 그룹이든 고치고 지운다.
///   그래서 이 struct 는 발신자를 담지 않는다(안 쓰는 값을 받으면 "검사하겠지" 라는 잘못된 기대를 남긴다).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupCommand {
    /// 대상 그룹 이름(`@`로 시작). 없으면 = 목록 조회.
    pub group: Option<String>,
    /// 추가할 멤버 이름들(없는 그룹이면 **암묵 생성**).
    pub add: Option<Vec<String>>,
    /// 제거할 멤버 이름들.
    pub remove: Option<Vec<String>>,
    /// 그룹 자체 삭제. `add`/`remove` 와 **함께 쓸 수 없다**(아래 handle_group 규칙 3).
    pub delete: Option<bool>,
}

/// ★`group` 공통 핸들러(D · spec §4·§6)★ — 그룹 명단 조회·증감·삭제.
///
/// ★인자 조합 규칙(결정 — 사용자 결정 2026-07-26 기반)★
///   1. 인자 없음 → **목록**: `{ "groups": ["@all", "@coders", …] }`(`@all` 먼저, 나머지 사전순).
///   2. `group` 만 → **멤버 조회**: `{ "group": "@coders", "members": ["alice","bob"] }`.
///      `@all` 은 지금 살아 있는 수신 가능 전원(발송 때와 같은 스냅샷 규칙 — 로스터 이름 verbatim).
///   3. `group` + `add`/`remove` → **증감**(없는 그룹에 add 하면 **암묵 생성** — 별도 create 동사 없음).
///      응답은 2번과 같은 shape(적용 후 명단) — 호출자가 결과를 한 번에 확인한다.
///   4. `group` + `delete:true` → **삭제**: `{ "group": "@coders", "deleted": true }`.
///   5. `delete` 와 `add`/`remove` **동시 지정은 반려**(`INVALID_GROUP_ARGS`) — "지우면서 멤버를 넣는다" 는
///      의미가 없고, 어느 쪽을 먼저 적용하느냐로 결과가 갈린다(둘 중 무엇을 원했는지 데몬이 추측하면 안 된다).
///      ★"delete 단독이면 delete 가 이긴다"★ = 4번, 즉 조합이 아닌 단독일 때만 삭제가 수행된다.
///   6. `group` 없이 `add`/`remove`/`delete` → 반려(`INVALID_GROUP_ARGS`) — 대상이 없다.
///   7. `delete:false` 는 "삭제 안 함" 이라 **무시**한다(false 를 명시했다고 반려하지 않는다 — 관대해도
///      모호하지 않은 유일한 자리다. 5번 판정도 `delete:true` 일 때만 발동한다).
///
/// ★에러 어휘★: `INVALID_GROUP_NAME`(선행 `@` 없음·`@` 단독·중복 `@` — 기존 `INVALID_SEND_ARGS` 의
///   `INVALID_*` 계열) · `INVALID_GROUP_ARGS`(위 5·6번 조합 오류) · `GROUP_NOT_FOUND`(없는 그룹 조회/증감/
///   삭제 — 발송 갈래와 **같은 코드**) · `GROUP_BUILTIN`(`@all` 증감·삭제 시도 — `GROUP_*` 계열).
/// ★왜 발송의 `GROUP_NOT_FOUND` 처럼 이름 규약 위반을 NOT_FOUND 로 접지 않나★: 발송에서는 발신자에게 맞는
///   사실이 "그런 그룹은 없다" 였다(보낼 곳이 없다는 게 핵심). 관리에서는 반대다 — 사용자가 **만들려고**
///   하는 중이라 "그 이름은 규약 위반" 과 "그 그룹이 아직 없다" 의 처방이 완전히 다르다(전자는 이름을 고쳐라,
///   후자는 add 로 만들어라). 두 사실을 한 코드로 접으면 `group coders --add a` 가 "없으니 만들어라" 로
///   읽혀 무한 재시도가 된다.
/// ★알림 없음★: 멤버 증감·삭제는 조용하다(사용자 결정 2026-07-26) — notice 를 만들지 않는다.
// ADR-0086 (듀얼 입구 공통 핸들러)
// ADR-0103 (spec §4·§6 group)
pub fn handle_group(
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    cmd: GroupCommand,
) -> ControlQueryResult {
    let plan = match validate_group_args(cmd) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match plan {
        GroupPlan::List => {
            ControlQueryResult::Ok(serde_json::json!({ "groups": messaging.group_list() }))
        }
        GroupPlan::Delete { group } => match messaging.group_delete(&group) {
            Ok(()) => ControlQueryResult::Ok(serde_json::json!({
                "group": group,
                "deleted": true,
            })),
            Err(e) => group_error_to_result(&group, e),
        },
        // 규칙 2·3 — 조회와 증감은 **한 출구**로 흐른다(응답 shape 이 같아야 하므로 분기를 늘리지 않는다).
        //   `@all` 은 증감이 금지돼 있어 변경 인자가 있으면 service 가 `Builtin` 으로 거절하고, 없으면 live
        //   스냅샷 조회로 답한다.
        GroupPlan::Members {
            group,
            add,
            remove,
            mutating,
        } => {
            let outcome = if mutating {
                messaging.group_update(&group, &add, &remove)
            } else {
                messaging.group_members(&group)
            };
            match outcome {
                Ok(members) => ControlQueryResult::Ok(serde_json::json!({
                    "group": group,
                    "members": members,
                })),
                Err(e) => group_error_to_result(&group, e),
            }
        }
    }
}

/// 검증을 통과한 `group` 커맨드의 **실행 계획**(순수 — 부작용 없음).
///
/// ★왜 계획으로 한 번 접나★: 인자 조합 규칙(handle_group doc-comment 1~7)은 순수 판정인데, 그걸
///   `handle_group` 안에 인라인으로 두면 `MessagingService`(= 실 로스터·실 큐) 없이는 단위 테스트가 불가능하다.
///   판정을 값으로 뽑아 두면 조합 규칙 전수를 순수 테스트로 못 박고, 실행부는 통합 테스트가 본다(seam 분리).
#[derive(Debug, Clone, PartialEq, Eq)]
enum GroupPlan {
    /// 인자 없음 → 그룹 이름 목록.
    List,
    /// 조회 또는 증감(둘의 응답 shape 이 같아 한 variant). `mutating=false` 면 순수 조회.
    Members {
        group: String,
        add: Vec<String>,
        remove: Vec<String>,
        mutating: bool,
    },
    /// 그룹 삭제(단독 지정일 때만 — 규칙 5).
    Delete { group: String },
}

/// `group` 인자 조합 검증 + 이름 정규화(순수 — handle_group doc-comment 의 규칙 1~7 정본 구현).
///
/// ★이름 정규화를 여기서 하는 이유(라벨 단일 출처)★: 응답의 `group` 필드는 **레지스트리가 실제로 쓴 이름**과
///   같아야 한다(발송 봉투의 그룹 라벨과 같은 원칙 — groups.rs `normalize_group_name` 주석). 입구가 raw 를
///   그대로 되돌리면 `" @coders"` 같은 입력에서 응답 라벨과 저장 키가 갈려, 호출자가 자기가 만든 그룹을
///   목록에서 못 찾는다. 그래서 seam 함수로 한 번 정규화하고 그 값을 아래로도 위로도 쓴다.
fn validate_group_args(cmd: GroupCommand) -> Result<GroupPlan, ControlQueryResult> {
    // ★멤버 이름 정규화는 **여기**(공용 핸들러)가 정본이다(D 리뷰 A1)★ — 아래 함수 주석 참조.
    let add_raw = cmd.add.unwrap_or_default();
    let remove_raw = cmd.remove.unwrap_or_default();
    let add = normalize_member_names(&add_raw);
    let remove = normalize_member_names(&remove_raw);
    // ★"뭔가 주긴 했는데 쓸 이름이 하나도 안 남았다" 는 반려한다★: 조용히 빈 목록으로 접으면 변경 의도가
    //   **순수 조회로 강등**돼 호출자가 "적용됐다" 고 오독한다(응답이 성공 shape 이라 구분이 안 된다).
    if !add_raw.is_empty() && add.is_empty() {
        return Err(ControlQueryResult::Error {
            code: "INVALID_GROUP_ARGS",
            hint: "add contained no usable agent names (only blanks or separators). Pass the teammate names you want in the group, e.g. add = [\"alice\", \"bob\"].".to_string(),
        });
    }
    if !remove_raw.is_empty() && remove.is_empty() {
        return Err(ControlQueryResult::Error {
            code: "INVALID_GROUP_ARGS",
            hint: "remove contained no usable agent names (only blanks or separators). Pass the names you want out of the group.".to_string(),
        });
    }
    // ★중첩 그룹 거절(round-2 리뷰 F5)★: `@` 로 시작하는 이름은 그룹 네임스페이스라 **에이전트 이름일 수
    //   없다**. 그대로 등록하면 어떤 방송에서도 매치되지 않아 영원히 skipped 되는데, 응답엔 멤버로 보이므로
    //   발신자는 중첩 그룹이 동작한다고 믿는다. `remove` 도 함께 막는다 — 애초에 등록될 수 없는 이름을
    //   지우라는 요청은 호출자가 중첩을 기대하고 있다는 신호라, 조용한 no-op 보다 교정 hint 가 낫다.
    if let Some(bad) = add.iter().chain(remove.iter()).find(|n| n.starts_with('@')) {
        return Err(invalid_member_name(bad));
    }
    // 규칙 7 — `delete:false` 는 "삭제 안 함" 이라 없는 것과 같게 취급한다.
    let delete = cmd.delete.unwrap_or(false);
    let mutating = !add.is_empty() || !remove.is_empty();

    // 규칙 5 — delete + 증감은 모호. 대상 유무보다 **먼저** 본다: 순수 인자 오류라 그룹 상태와 무관하게
    //   항상 같은 답이어야 한다(handle_send 의 "구문 먼저" 규율과 동일).
    if delete && mutating {
        return Err(ControlQueryResult::Error {
            code: "INVALID_GROUP_ARGS",
            hint: "delete cannot be combined with add/remove — deleting the group discards the membership you are editing. Send one call to change members, or one call with delete alone.".to_string(),
        });
    }

    let Some(group) = cmd.group else {
        // 규칙 6 — 대상 없는 변경은 반려. 인자가 아예 없으면 목록(규칙 1).
        if delete || mutating {
            return Err(ControlQueryResult::Error {
                code: "INVALID_GROUP_ARGS",
                hint: "add / remove / delete need a group to act on — pass group = \"@name\". Call with no arguments to list the groups that exist.".to_string(),
            });
        }
        return Ok(GroupPlan::List);
    };

    let group = engram_dashboard_messaging::groups::normalize_group_name(&group).map_err(|e| {
        // 이름 규약 위반은 여기서 끝난다 — service 까지 내려가지 않는다(부작용 0 지점에서 반려).
        group_error_to_result(&group, e)
    })?;

    if delete {
        return Ok(GroupPlan::Delete { group });
    }
    Ok(GroupPlan::Members {
        group,
        add,
        remove,
        mutating,
    })
}

/// ★멤버 이름 목록 정규화 — **양 입구 공용 정본**(D 리뷰 A1 · load-bearing)★. 콤마 분해 + 각 조각 trim +
/// 빈 조각 제거.
///
/// ★왜 CLI 가 아니라 여기인가★: 예전엔 이 정리가 CLI 파서에만 있었다. 그래서 **MCP 로 들어온 같은 표기가
///   그대로 저장**됐다 — 프라이밍이 콤마 형태(`--add alice,bob`)를 가르치므로 MCP 호출자도 자연히
///   `add:["alice,bob"]` 을 보내는데, 그러면 `"alice,bob"` 이라는 **이름 하나짜리 유령 멤버**가 등록된다.
///   그 이름과 일치하는 에이전트는 영원히 없으므로 모든 방송에서 `skipped` 로 새고, 발신자는 두 명에게
///   보냈다고 믿는다. 같은 이유로 `" bob"`(앞 공백)은 별개 멤버가 되고, `""`(빈 이름)은 CLI 로 지울 수조차
///   없었다(`--remove ""` 가 CLI 단계에서 걸러져 순수 조회로 바뀐다). 검증·정규화는 데몬 ingress 단독이라는
///   이 모듈의 원칙(헤더)을 그대로 적용해 정본을 여기 하나로 옮긴다 — CLI 는 argv 를 그대로 실어 보낸다.
/// ★왜 거부가 아니라 분해인가★: 콤마 형태는 우리가 프라이밍에서 **가르친** 표기다. 그걸 에러로 돌려주면
///   문서와 구현이 싸우는 꼴이고, 호출자는 자기가 배운 대로 썼는데 반려당한다. 분해가 의도에 맞는 해석이다.
///   (콤마를 품은 실제 에이전트 이름은 그 대가로 그룹에 넣을 수 없다 — 이름은 display_name/cwd basename 에서
///   오므로 병적인 경우이고, 유령 멤버가 조용히 생기는 쪽이 훨씬 해롭다.)
/// ★결과가 빈 목록이 될 수 있다★: 호출자가 뭔가를 주긴 했는데 전부 걸러졌다는 뜻이라, 호출부
///   (`validate_group_args`)가 그 경우를 `INVALID_GROUP_ARGS` 로 반려한다(조용한 강등 금지).
// 리뷰 A1
// ADR-0109 (의미 검증·정규화 = 데몬 단일점)
fn normalize_member_names(raw: &[String]) -> Vec<String> {
    raw.iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 그룹 연산 에러 → wire 코드/hint(관리 표면 전용 매핑 — 발송 갈래는 `handle_group_send` 가 따로 매핑한다).
///
/// ★두 매핑이 갈리는 건 의도적이다(handle_group doc-comment 의 근거)★: 발송은 "보낼 곳이 없다" 가 요점이라
///   이름 규약 위반까지 `GROUP_NOT_FOUND` 로 접었지만, 관리는 "무엇을 고쳐야 하나" 가 요점이라 사유를 가른다.
fn group_error_to_result(
    requested: &str,
    e: engram_dashboard_messaging::groups::GroupError,
) -> ControlQueryResult {
    use engram_dashboard_messaging::groups::GroupError;
    match e {
        GroupError::InvalidName { name } => ControlQueryResult::Error {
            code: "INVALID_GROUP_NAME",
            hint: format!(
                "'{name}' is not a valid group name: exactly one leading '@' plus a name, e.g. @coders."
            ),
        },
        GroupError::Builtin => ControlQueryResult::Error {
            code: "GROUP_BUILTIN",
            hint: "@all is built in — it always means everyone live and reachable right now, so it cannot be edited or deleted. Create your own group instead (e.g. group @coders --add alice).".to_string(),
        },
        GroupError::NotFound { name } => ControlQueryResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: format!(
                "No group named '{name}'. Adding a member creates the group, so `group {name} --add <name>` is how you make it."
            ),
        },
        // 관리 경로는 "아는데 멤버 0명" 을 정상 조회 결과(빈 목록)로 답하므로 여기 오면 배선 결함이다 —
        //   조용히 성공으로 접지 않고 사실대로 알린다(발송 갈래의 GROUP_EMPTY 와 같은 어휘).
        GroupError::Empty { name } => ControlQueryResult::Error {
            code: "GROUP_EMPTY",
            hint: format!("Group '{name}' has no members right now (requested: '{requested}')."),
        },
        // 입구가 먼저 거르므로(validate_group_args) 여기 오면 구조적 guard 가 잡은 것 — 같은 코드·같은 문구.
        GroupError::InvalidMemberName { name } => invalid_member_name(&name),
    }
}

/// `@` 로 시작하는 멤버 이름 반려(round-2 리뷰 F5) — 입구 검증과 레지스트리 guard 가 **같은 답**을 내게
/// 하나로 모은다(두 곳이 다른 문구를 내면 호출자가 원인을 두 가지로 오해한다).
fn invalid_member_name(name: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: "INVALID_MEMBER_NAME",
        hint: format!(
            "'{name}' cannot be a group member: members are agent names, and a group cannot contain another group (nesting is not supported). Pass the individual agent names instead."
        ),
    }
}

/// `to`(이름 또는 AgentId 문자열) → 산 에이전트 해석. 매치 규칙(ADR-0086 §6):
///   - ★정확한 AgentId 문자열 우선(F2)★. 이름과 별개 축이며 **이름 매치보다 먼저** 시도한다.
///   - 그 다음 이름(AgentInfo.name = profile name) 정확 일치. 여러 개면 RECIPIENT_AMBIGUOUS(후보 name+id 나열).
///   - 없으면 RECIPIENT_NOT_FOUND(산 에이전트 이름 나열 = 미니 로스터, 자기교정용).
///
/// ★왜 ID 를 먼저 보나(F2)★: 어떤 에이전트의 *이름*이 우연히 다른 에이전트의 UUID 문자열과 같으면,
///   이름 매치를 먼저 하면 ID 로 지목한 메일이 엉뚱한(이름=UUID) 에이전트에게 잡힌다. AgentId 는
///   시스템이 부여하는 안정적·유일한 주소축이므로 ID 형태의 `to` 는 항상 ID 로 먼저 해석한다(이름
///   충돌이 ID 지목을 가로채지 못하게). 그래서 exact-ID 매치가 name 매치를 **선행**한다.
///
/// ★산(live) 판정★: 종료된 세션은 reaper 가 곧 맵에서 제거하나, 스냅샷 순간에 terminal 상태가 남아 있을
///   수 있어 명시적으로 non-terminal(Running/Exiting)만 후보로 본다.
///
/// ★도달성(structured) 정렬(finding 6 · load-bearing)★: 후보 집합은 산(live) **AND 도달 가능(structured)**
///   이다 — MessagingService::live_reachable_agents 와 **정확히 같은** 판정이다. 예전엔 여기서 is_live 만
///   봐서, 같은 이름의 structured 1개 + TUI(비-structured) 1개가 있으면 AMBIGUOUS 로 반려됐다 — 실제로는
///   service resolver 가 도달 가능(structured) 후보 1개만 보므로 **유일하게 배달 가능**한데도 막힌 것이다.
///   두 곳의 후보 집합을 일치시켜 그 위양성 반려를 없앤다.
/// ★남은 check-then-act TOCTOU(accepted)★: 이 AMBIGUOUS 사전 검사(ingress)와 이후 service resolve 는 서로
///   다른 로스터 스냅샷을 본다 — 그 사이 같은 이름의 도달 후보가 새로 등장하면 ingress 는 통과시켰는데 service
///   가 동명 다수를 만날 수 있다(반대도 성립). 이 창은 극히 좁고(사람 대화 수준 메시지율) 최악이라도 파킹/
///   재지목으로 수렴하므로(유실 없음) v1 에선 **의도적으로 수용**한다 — 두 조회를 하나의 원자 스냅샷으로
///   묶는 건 seam 을 넘는 과설계라 v2 로 미룬다.
fn resolve_recipient(
    to: &str,
    agents: &[engram_dashboard_core::agent::types::AgentInfo],
) -> Resolution {
    // 산 AND 도달 가능(structured) — service resolver 와 동일 후보 집합(finding 6).
    let live: Vec<&engram_dashboard_core::agent::types::AgentInfo> = agents
        .iter()
        .filter(|a| is_live(&a.status) && a.capabilities.output.structured)
        .collect();

    // ★F2: AgentId 문자열 정확 일치를 이름보다 **먼저** 시도★ — 이름=UUID 충돌이 ID 지목을 가로채지 못하게.
    if let Some(a) = live.iter().find(|a| a.id.to_string() == to) {
        return Resolution::Ok {
            id: a.id,
            name: a.name.clone(),
        };
    }

    // 이름 정확 일치 후보(ID 매치 실패 후).
    let by_name: Vec<&&engram_dashboard_core::agent::types::AgentInfo> =
        live.iter().filter(|a| a.name == to).collect();

    match by_name.len() {
        1 => {
            let a = by_name[0];
            return Resolution::Ok {
                id: a.id,
                name: a.name.clone(),
            };
        }
        n if n > 1 => {
            // 동명 다수 → 후보를 name+id 쌍으로 나열해 발신자가 id 로 재지목하게 한다.
            let candidates = by_name
                .iter()
                .map(|a| format!("{}(id:{})", a.name, a.id))
                .collect::<Vec<_>>()
                .join(", ");
            return Resolution::Err(ControlResult::Error {
                code: "RECIPIENT_AMBIGUOUS",
                hint: format!(
                    "Multiple live agents named '{to}': {candidates}. Re-send using the exact agent id."
                ),
            });
        }
        _ => {}
    }

    // 아무 매치 없음 → 산 에이전트 이름 나열(미니 로스터, 자기교정).
    let roster = live
        .iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let roster = if roster.is_empty() {
        "(none)".to_string()
    } else {
        roster
    };
    Resolution::Err(ControlResult::Error {
        code: "RECIPIENT_NOT_FOUND",
        hint: format!("No live agent matches '{to}'. Live agents: {roster}."),
    })
}

/// non-terminal(산) 상태인가. Running/Exiting = 산, Exited/Failed/Killed = terminal.
fn is_live(status: &AgentStatus) -> bool {
    matches!(status, AgentStatus::Running | AgentStatus::Exiting)
}

/// 발신자 표시 이름 — canonical 표시명(display_name ?? basename(session.cwd)). 없으면 id 앞 8자.
///
/// ADR-0101 (WYSIWYA): 봉투 sender 이름은 수신자가 로스터·트리·라우팅에서 보는 이름과 **byte-identical**
///   해야 한다 — 안 그러면 "A: 안녕" 봉투를 받고도 로스터엔 A 가 다른 문자열로 떠 지목이 어긋난다.
///
/// ★단일 출처 = manager.canonical_name(session.cwd 기반)★: 예전엔 여기서 profile.cwd(raw)로 재파생해
///   agent_info(session.cwd 기반)와 갈릴 수 있었다. 이제 라우팅(resolve_recipient)이 쓰는 AgentInfo.name
///   과 **정확히 같은 계산**을 manager 한 곳에서 얻어 로직 복제·어긋남을 없앤다. 산 세션이면 그 값을 쓰고,
///   relay 시점에 세션이 이미 수거됐으면(발신자 terminal — line 328 케이스) 프로필+공유 fallback 으로
///   best-effort 표시(이 봉투는 이미 인증된 발신자의 표시용이라 라우팅 어긋남과 무관).
fn sender_display_name(manager: &Arc<AgentManager>, from: BoundIdentity) -> String {
    // 산 세션이면 AgentInfo.name 과 byte-identical 한 canonical 이름(session.cwd 기반).
    if let Some(name) = manager.canonical_name(from.agent_id) {
        return name;
    }
    // 세션 수거됨(발신자 terminal) → 프로필 있으면 공유 fallback 으로 best-effort. profile.cwd 는 raw 라
    //   canonical 과 다를 수 있으나, 이 경로는 산 세션이 없어 라우팅 대상도 아니다(표시 전용).
    if let Some(p) = manager.profiles().get(from.agent_id) {
        return engram_dashboard_core::agent::name::canonical_name_or_id_fallback(
            p.display_name.as_deref(),
            &p.cwd.to_string_lossy(),
            from.agent_id,
        );
    }
    let s = from.agent_id.to_string();
    s[..8.min(s.len())].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    // ADR-0110: 봉투 조립 seam 은 커널 crate — 이 아래 테스트는 "계약 인자 → 봉투 속성" 매핑을 본다.
    use engram_dashboard_messaging::envelope::wrap_message;

    use engram_dashboard_core::agent::types::{
        AgentInfo, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps, SessionCaps,
    };

    /// ENGRAM_WRAP_FORMAT 은 프로세스 전역 env — set/remove·미설정 단언 테스트끼리 직렬화한다
    /// (병렬 실행 시 한 테스트의 set 이 다른 테스트의 "미설정" 단언을 짓밟지 않게).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn ident(id: AgentId) -> BoundIdentity {
        BoundIdentity {
            agent_id: id,
            epoch: 0,
        }
    }

    /// 테스트용 AgentInfo — 이름·structured(도달성)·상태를 지정한다.
    fn info(id: AgentId, name: &str, structured: bool, status: AgentStatus) -> AgentInfo {
        AgentInfo {
            id,
            name: name.to_string(),
            cwd: ".".to_string(),
            status,
            cols: 80,
            rows: 24,
            epoch: 0,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: !structured,
                    structured,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
        }
    }

    // ── Validator: resolve_recipient ────────────────────────────────────────────
    // (구 `resolve_recipients_*`(ADR-0092 다중수신 seam) 제거 — C1 이 부재를 파킹으로 옮기며 그 1:1 목록
    //  승격 seam 은 MessagingService 로 이사했다. 부재/파킹 로직 회귀는 service.rs 단위 테스트가 커버한다.)
    #[test]
    fn resolve_by_unique_name() {
        let id = AgentId::new_v4();
        let agents = vec![info(id, "alice", true, AgentStatus::Running)];
        match resolve_recipient("alice", &agents) {
            Resolution::Ok { id: got, name } => {
                assert_eq!(got, id);
                assert_eq!(name, "alice");
            }
            Resolution::Err(_) => panic!("이름 유일 매치는 성공해야"),
        }
    }

    #[test]
    fn resolve_by_exact_agent_id() {
        let id = AgentId::new_v4();
        let agents = vec![info(id, "alice", true, AgentStatus::Running)];
        match resolve_recipient(&id.to_string(), &agents) {
            Resolution::Ok { id: got, .. } => assert_eq!(got, id),
            Resolution::Err(_) => panic!("정확한 AgentId 문자열도 수용해야"),
        }
    }

    #[test]
    fn resolve_not_found_lists_roster() {
        let agents = vec![info(AgentId::new_v4(), "alice", true, AgentStatus::Running)];
        match resolve_recipient("bob", &agents) {
            Resolution::Err(ControlResult::Error { code, hint }) => {
                assert_eq!(code, "RECIPIENT_NOT_FOUND");
                assert!(hint.contains("alice"), "미니 로스터에 산 이름 나열: {hint}");
            }
            _ => panic!("없는 수신자는 NOT_FOUND"),
        }
    }

    #[test]
    fn resolve_ambiguous_lists_candidates() {
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let agents = vec![
            info(a, "dup", true, AgentStatus::Running),
            info(b, "dup", true, AgentStatus::Running),
        ];
        match resolve_recipient("dup", &agents) {
            Resolution::Err(ControlResult::Error { code, hint }) => {
                assert_eq!(code, "RECIPIENT_AMBIGUOUS");
                assert!(
                    hint.contains(&a.to_string()) && hint.contains(&b.to_string()),
                    "후보 name+id 쌍 나열: {hint}"
                );
            }
            _ => panic!("동명 다수는 AMBIGUOUS"),
        }
    }

    #[test]
    fn resolve_exact_id_precedes_name_match_f2() {
        // ★F2 회귀★: 에이전트 X 의 이름이 우연히 에이전트 Y 의 UUID 문자열과 같을 때, Y 의 UUID 로 지목하면
        //   ID 로 먼저 해석돼 Y 에게 가야 한다(이름=UUID 인 X 가 가로채면 안 됨).
        let y = AgentId::new_v4();
        let x = AgentId::new_v4();
        let agents = vec![
            // X 의 name = Y 의 UUID 문자열(악의/우연 충돌).
            info(x, &y.to_string(), true, AgentStatus::Running),
            info(y, "yankee", true, AgentStatus::Running),
        ];
        match resolve_recipient(&y.to_string(), &agents) {
            Resolution::Ok { id, name } => {
                assert_eq!(
                    id, y,
                    "ID 지목은 그 ID 의 에이전트(Y)로 — 이름=UUID 인 X 가 가로채면 안 됨"
                );
                assert_eq!(name, "yankee");
            }
            Resolution::Err(_) => panic!("exact-ID 매치가 이름 매치를 선행해야(F2)"),
        }
    }

    #[test]
    fn resolve_name_with_structured_and_tui_is_not_ambiguous() {
        // ★finding 6 회귀★: 같은 이름의 structured 1개 + TUI(비-structured) 1개는 AMBIGUOUS 가 아니다 —
        //   도달 가능(structured) 후보가 유일하므로 그것으로 해석된다(service resolver 와 후보 집합 일치).
        let s = AgentId::new_v4();
        let t = AgentId::new_v4();
        let agents = vec![
            info(s, "dup", true, AgentStatus::Running),  // 도달 가능
            info(t, "dup", false, AgentStatus::Running), // TUI(비-도달)
        ];
        match resolve_recipient("dup", &agents) {
            Resolution::Ok { id, name } => {
                assert_eq!(
                    id, s,
                    "도달 가능(structured) 후보로 해석 — TUI 는 후보 아님"
                );
                assert_eq!(name, "dup");
            }
            Resolution::Err(_) => {
                panic!("structured 유일이면 AMBIGUOUS 아님(finding 6)")
            }
        }
    }

    #[test]
    fn resolve_two_structured_same_name_is_still_ambiguous() {
        // finding 6 경계: 도달 가능(structured) 후보가 **둘** 이면 여전히 AMBIGUOUS(정상 반려).
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let agents = vec![
            info(a, "dup", true, AgentStatus::Running),
            info(b, "dup", true, AgentStatus::Running),
        ];
        assert!(
            matches!(
                resolve_recipient("dup", &agents),
                Resolution::Err(ControlResult::Error {
                    code: "RECIPIENT_AMBIGUOUS",
                    ..
                })
            ),
            "도달 후보 2개는 AMBIGUOUS 유지"
        );
    }

    #[test]
    fn resolve_only_tui_same_name_is_not_found_not_ambiguous() {
        // finding 6 경계: 같은 이름이 전부 TUI(비-도달)면 도달 후보 0 → NOT_FOUND(부재 → 상위가 파킹).
        //   AMBIGUOUS 가 아니다(모호할 도달 후보가 없음).
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        let agents = vec![
            info(a, "dup", false, AgentStatus::Running),
            info(b, "dup", false, AgentStatus::Running),
        ];
        assert!(
            matches!(
                resolve_recipient("dup", &agents),
                Resolution::Err(ControlResult::Error {
                    code: "RECIPIENT_NOT_FOUND",
                    ..
                })
            ),
            "TUI 만 있는 이름은 도달 후보 0 → NOT_FOUND(AMBIGUOUS 아님)"
        );
    }

    #[test]
    fn resolve_skips_terminal_agents() {
        // terminal 상태(Killed)는 산 후보에서 제외 → NOT_FOUND.
        let id = AgentId::new_v4();
        let agents = vec![info(id, "ghost", true, AgentStatus::Killed)];
        assert!(matches!(
            resolve_recipient("ghost", &agents),
            Resolution::Err(ControlResult::Error {
                code: "RECIPIENT_NOT_FOUND",
                ..
            })
        ));
    }

    // ── ControlResult wire shape(양 입구 동일 — spec §6) ──────────────────────────────
    #[test]
    fn ok_delivered_json_shape() {
        // spec §6: 성공 = `{ id, results: [{to, status}] }`. delivered 는 hint 없음.
        let r = ControlResult::Ok {
            id: "mid".to_string(),
            results: vec![SendResult {
                to: "bob".to_string(),
                status: "delivered",
                hint: None,
            }],
        };
        let v = r.to_json();
        assert_eq!(v["id"], "mid");
        assert_eq!(v["results"][0]["to"], "bob");
        assert_eq!(v["results"][0]["status"], "delivered");
        assert!(
            v["results"][0].get("hint").is_none(),
            "delivered 는 hint 생략"
        );
        assert!(
            v.get("status").is_none(),
            "성공 응답엔 최상위 status 없음(spec §6)"
        );
        assert!(r.is_accepted());
    }

    #[test]
    fn ok_pending_json_shape_includes_hint() {
        // spec §6: 파킹 = `{ id, results: [{to, status:"pending", hint}] }`.
        let r = ControlResult::Ok {
            id: "mid".to_string(),
            results: vec![SendResult {
                to: "ghost".to_string(),
                status: "pending",
                hint: Some("parked".to_string()),
            }],
        };
        let v = r.to_json();
        assert_eq!(v["results"][0]["status"], "pending");
        assert_eq!(v["results"][0]["hint"], "parked");
        assert!(r.is_accepted(), "pending 도 접수 성공(반려 아님)");
    }

    #[test]
    fn error_json_shape() {
        let r = ControlResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: "h".to_string(),
        };
        let v = r.to_json();
        assert_eq!(v["status"], "error");
        assert_eq!(v["code"], "GROUP_NOT_FOUND");
        assert_eq!(v["hint"], "h");
        assert!(!r.is_accepted(), "반려는 접수 성공 아님");
    }

    // ── ControlCommand 정규화: from 은 값(신원)으로만 들어온다(페이로드 아님) ──────────────
    #[test]
    fn control_command_carries_identity_not_payload_from() {
        // ControlCommand 는 from 을 BoundIdentity 값으로만 담는다 — payload 에 from 필드가 없다(구조적 보장).
        let id = AgentId::new_v4();
        let cmd = ControlCommand {
            from: ident(id),
            to: "bob".to_string(),
            body: "hi".to_string(),
            contract: Default::default(),
        };
        assert_eq!(cmd.from.agent_id, id);
    }

    // ── C3: reply_by 기간 표기 파서 ──────────────────────────────────────────────────────────

    #[test]
    fn parse_reply_by_accepts_integer_plus_unit() {
        use std::time::Duration;
        assert_eq!(
            parse_reply_by("10m").expect("10m"),
            Duration::from_secs(600)
        );
        assert_eq!(parse_reply_by("1h").expect("1h"), Duration::from_secs(3600));
        // ★`s` 단위는 계속 유효하다(리뷰 fix 7)★ — 하한은 **값**에 걸리는 것이지 표기에 거는 게 아니다.
        assert_eq!(parse_reply_by("60s").expect("60s"), Duration::from_secs(60));
        assert_eq!(
            parse_reply_by("120s").expect("120s"),
            Duration::from_secs(120)
        );
        // 상한 경계(30일)는 수용.
        assert_eq!(
            parse_reply_by("720h").expect("720h = 30d"),
            Duration::from_secs(30 * 24 * 3600)
        );
    }

    #[test]
    fn parse_reply_by_rejects_below_one_minute_floor() {
        // ★리뷰 fix 7★: 기한 판정 해상도 = sweep 주기(60s)라 1분 미만은 지킬 수 없는 약속이다 —
        //   `30s` 를 받아 주면 실제 통지는 60~120초 뒤에 나간다(계약 문구가 거짓말이 된다).
        for bad in ["1s", "30s", "59s"] {
            let err = parse_reply_by(bad).expect_err("1분 미만은 반려");
            assert!(
                err.contains("1-minute minimum"),
                "hint 가 하한을 알려야: {err}"
            );
        }
        // 경계: 정확히 60초는 수용(`>=` 아님 — `< MIN` 만 반려).
        assert!(parse_reply_by("60s").is_ok(), "정확히 1분은 수용");
        assert!(parse_reply_by("1m").is_ok(), "1m 도 같은 값이라 수용");
    }

    #[test]
    fn parse_reply_by_rejects_malformed_forms() {
        // 엄격 — 모호하면 반려하고 hint 로 형태를 알린다(조용한 오해석 금지).
        for bad in [
            "", "s", "m", "10", "10 m", " 10m", "10m ", "10M", "1H", "10x", "-5m", "1.5h", "1h30m",
            "0s", "0m", "0h", "십분", "10초",
        ] {
            assert!(
                parse_reply_by(bad).is_err(),
                "'{bad}' 는 반려돼야(엄격 파서)"
            );
        }
    }

    #[test]
    fn parse_reply_by_rejects_overflow_and_beyond_cap() {
        // ★가용성 방어★: 상한이 없으면 `Instant + Duration` 오버플로 패닉으로 sweep task 가 죽는다.
        assert!(parse_reply_by("721h").is_err(), "30일 초과는 반려");
        assert!(
            parse_reply_by("18446744073709551615s").is_err(),
            "u64 최대치도 상한에서 반려"
        );
        assert!(
            parse_reply_by("99999999999999999999s").is_err(),
            "u64 파싱 초과도 반려"
        );
        assert!(
            parse_reply_by("18446744073709551615h").is_err(),
            "곱셈 오버플로도 반려"
        );
    }

    // ── C3 리뷰 fix 1: 계약 필드는 XML 봉투 전용 ─────────────────────────────────────────────

    /// request 발송 메타(검증 통과분 모양).
    fn req_meta() -> engram_dashboard_messaging::service::SendMeta {
        engram_dashboard_messaging::service::SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            reply_by: Some(std::time::Duration::from_secs(600)),
            reply_to: None,
            group: None,
        }
    }
    fn reply_meta() -> engram_dashboard_messaging::service::SendMeta {
        engram_dashboard_messaging::service::SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
            group: None,
        }
    }

    #[test]
    fn contract_fields_are_rejected_under_colon_envelope() {
        // ★fix 1★: colon 렌더는 id/type/reply-by/in-reply-to 를 통째로 버린다 — 그 상태의 request 는
        //   회신이 구조적으로 불가능하고 거짓 타임아웃이 **보장**된다. 조용히 열화시키지 않고 반려한다.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var("ENGRAM_WRAP_FORMAT").is_err(),
            "전제: 템플릿 env 미설정"
        );
        for meta in [req_meta(), reply_meta()] {
            let hint = contract_unsupported_by_envelope(&meta, EnvelopeFormat::Colon)
                .expect("colon 에선 계약 필드 반려");
            assert!(hint.contains("XML envelope"), "교정 hint: {hint}");
        }
        // 대조군: 같은 colon 이라도 **통보**는 통과한다(속성이 없으니 열화될 게 없다).
        assert_eq!(
            contract_unsupported_by_envelope(
                &engram_dashboard_messaging::service::SendMeta::default(),
                EnvelopeFormat::Colon
            ),
            None,
            "통보는 포맷과 무관하게 허용(기존 동작 불변)"
        );
        // 대조군: xml 이면 계약 필드도 통과.
        assert_eq!(
            contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml),
            None
        );
    }

    #[test]
    fn contract_fields_are_rejected_when_wrap_template_env_is_active() {
        // ★fix 1 의 두 번째 갈래★: `ENGRAM_WRAP_FORMAT` 템플릿은 wrap_message 가 **format 인자보다 먼저**
        //   보는 전역 스위치라, 포맷이 Xml 이어도 실제 렌더는 템플릿(sender/id/body)이다 — 판정도 같은
        //   입력을 봐야 새지 않는다.
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var("ENGRAM_WRAP_FORMAT").is_err(),
            "전제: 다른 테스트가 남긴 값이 없어야"
        );
        std::env::set_var("ENGRAM_WRAP_FORMAT", "<{sender}#{id}> {body}");
        let under_template = contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml);
        let plain_under_template = contract_unsupported_by_envelope(
            &engram_dashboard_messaging::service::SendMeta::default(),
            EnvelopeFormat::Xml,
        );
        // 빈 값은 "미설정" 과 같게 취급한다(wrap_message 의 판정과 동일 규칙).
        std::env::set_var("ENGRAM_WRAP_FORMAT", "");
        let under_empty = contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml);
        std::env::remove_var("ENGRAM_WRAP_FORMAT"); // 반드시 제거(다른 테스트로 새지 않게).

        assert!(
            under_template.is_some(),
            "템플릿이 켜져 있으면 xml 포맷이어도 계약 필드는 반려"
        );
        assert_eq!(
            plain_under_template, None,
            "템플릿 아래서도 통보는 통과(기존 스파이크 경로 불변)"
        );
        assert_eq!(under_empty, None, "빈 값 = 미설정 취급");
    }

    // ── C3: 발송 인자 정합(spec §6 상호배타) ──────────────────────────────────────────────────

    #[test]
    fn contract_default_is_plain_notice_message() {
        let m = validate_contract(&SendContract::default()).expect("통보는 항상 유효");
        assert!(!m.request);
        assert_eq!(m.reply_by, None);
        assert_eq!(m.reply_to, None);
    }

    #[test]
    fn contract_request_and_reply_to_together_is_rejected() {
        let c = SendContract {
            request: true,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
        };
        let e = validate_contract(&c).expect_err("상호배타(spec §6)");
        assert!(
            e.contains("mutually exclusive"),
            "hint 가 사유를 말해야: {e}"
        );
    }

    #[test]
    fn contract_reply_by_without_request_is_rejected() {
        let c = SendContract {
            request: false,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        };
        let e = validate_contract(&c).expect_err("reply_by 는 request 전용");
        assert!(e.contains("request"), "hint 가 교정법을 말해야: {e}");
    }

    #[test]
    fn contract_bad_reply_by_notation_is_rejected_with_form_hint() {
        let c = SendContract {
            request: true,
            reply_by: Some("ten minutes".to_string()),
            reply_to: None,
        };
        let e = validate_contract(&c).expect_err("표기 오류");
        assert!(e.contains("10m"), "허용 형태 예시: {e}");
        // ★hint 는 **지금 유효한** 예시만 든다(리뷰 fix 7 의 짝)★: 하한 도입 후에도 `30s` 를 예시로 남기면
        //   그 예시를 따라 고친 발신자가 다시 반려당한다(자기교정 루프). 예시는 반드시 통과하는 값이어야.
        for example in ["30s", "1s", "59s"] {
            assert!(
                !e.contains(example),
                "hint 가 반려되는 값을 예시로 들면 안 된다('{example}'): {e}"
            );
        }
    }

    #[test]
    fn contract_empty_reply_to_is_rejected_and_whitespace_is_trimmed() {
        let empty = SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("   ".to_string()),
        };
        assert!(
            validate_contract(&empty).is_err(),
            "공백만 reply_to 는 반려"
        );

        let padded = SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("  m-7f3k  ".to_string()),
        };
        let m = validate_contract(&padded).expect("trim 후 유효");
        assert_eq!(
            m.reply_to.as_deref(),
            Some("m-7f3k"),
            "앞뒤 공백은 잘라 받는다"
        );
    }

    #[test]
    fn valid_request_contract_carries_both_raw_and_parsed_deadline() {
        // 봉투는 raw 표기("10m")를, 장부는 파싱값(600s)을 쓴다(SendMeta 주석).
        let c = SendContract {
            request: true,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        };
        let m = validate_contract(&c).expect("valid");
        assert!(m.request);
        assert_eq!(m.reply_by_raw.as_deref(), Some("10m"));
        assert_eq!(m.reply_by, Some(std::time::Duration::from_secs(600)));
    }

    // ── C3: 봉투 속성 조립(노출 원칙 — spec §1) ────────────────────────────────────────────────

    #[test]
    fn request_envelope_exposes_id_type_reply_by_only() {
        // ★노출 원칙★: id 는 request 에만. 통보/회신 봉투에 id 가 새면 수신 LLM 의 회신 판단이 흐려진다.
        let _g = ENV_LOCK.lock().unwrap();
        let m = validate_contract(&SendContract {
            request: true,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        })
        .expect("valid");
        let w = wrap_message(
            "qa-alpha",
            "m-7f3k",
            "코드 짜고 회신해",
            EnvelopeFormat::Xml,
            &m.envelope_fields("m-7f3k"),
        );
        assert_eq!(
            w,
            r#"<message from="qa-alpha" id="m-7f3k" type="request" reply-by="10m">코드 짜고 회신해</message>"#
        );
    }

    #[test]
    fn reply_envelope_exposes_only_in_reply_to() {
        let _g = ENV_LOCK.lock().unwrap();
        let m = validate_contract(&SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
        })
        .expect("valid");
        let w = wrap_message(
            "qa-bravo",
            "m-other",
            "다 짰음",
            EnvelopeFormat::Xml,
            &m.envelope_fields("m-other"),
        );
        assert_eq!(
            w, r#"<message from="qa-bravo" in-reply-to="m-7f3k">다 짰음</message>"#,
            "회신엔 id/type 이 없다(노출 원칙)"
        );
    }

    #[test]
    fn plain_envelope_has_no_attributes() {
        let _g = ENV_LOCK.lock().unwrap();
        let m = validate_contract(&SendContract::default()).expect("valid");
        let w = wrap_message(
            "alice",
            "m-zz",
            "빌드 끝났음",
            EnvelopeFormat::Xml,
            &m.envelope_fields("m-zz"),
        );
        assert_eq!(w, r#"<message from="alice">빌드 끝났음</message>"#);
    }

    // ── D: `group` 인자 조합 규칙(순수 — handle_group doc-comment 1~7) ────────────────────────

    /// 인자 조합 오류의 code 만 뽑는다(hint 문구는 계약이 아니라 교정 텍스트라 단언하지 않는다).
    fn group_err_code(r: &ControlQueryResult) -> &'static str {
        match r {
            ControlQueryResult::Error { code, .. } => code,
            ControlQueryResult::Ok(v) => panic!("에러여야 하는데 성공: {v}"),
        }
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn group_args_no_arguments_is_a_list() {
        assert_eq!(
            validate_group_args(GroupCommand::default()),
            Ok(GroupPlan::List)
        );
        // 규칙 7 — delete:false 는 "삭제 안 함" 이라 없는 것과 같다(명시했다고 반려하지 않는다).
        assert_eq!(
            validate_group_args(GroupCommand {
                delete: Some(false),
                ..Default::default()
            }),
            Ok(GroupPlan::List)
        );
    }

    #[test]
    fn group_args_name_only_is_a_pure_query() {
        assert_eq!(
            validate_group_args(GroupCommand {
                group: Some("@coders".to_string()),
                ..Default::default()
            }),
            Ok(GroupPlan::Members {
                group: "@coders".to_string(),
                add: vec![],
                remove: vec![],
                mutating: false,
            })
        );
    }

    #[test]
    fn group_args_add_or_remove_is_a_mutation() {
        assert_eq!(
            validate_group_args(GroupCommand {
                group: Some("@coders".to_string()),
                add: Some(names(&["alice"])),
                remove: Some(names(&["bob"])),
                ..Default::default()
            }),
            Ok(GroupPlan::Members {
                group: "@coders".to_string(),
                add: names(&["alice"]),
                remove: names(&["bob"]),
                mutating: true,
            })
        );
        // 빈 배열은 변경이 아니다(부작용 0) — `--add` 없이 호출한 것과 같게 접는다.
        assert_eq!(
            validate_group_args(GroupCommand {
                group: Some("@coders".to_string()),
                add: Some(vec![]),
                remove: Some(vec![]),
                ..Default::default()
            })
            .map(|p| matches!(
                p,
                GroupPlan::Members {
                    mutating: false,
                    ..
                }
            )),
            Ok(true)
        );
    }

    #[test]
    fn group_args_delete_alone_wins_but_combined_with_edits_is_rejected() {
        // 규칙 4 — delete 단독은 삭제.
        assert_eq!(
            validate_group_args(GroupCommand {
                group: Some("@coders".to_string()),
                delete: Some(true),
                ..Default::default()
            }),
            Ok(GroupPlan::Delete {
                group: "@coders".to_string()
            })
        );
        // 규칙 5 — delete + add/remove 는 모호하므로 반려(둘 중 무엇을 원했는지 추측 금지).
        for cmd in [
            GroupCommand {
                group: Some("@coders".to_string()),
                add: Some(names(&["a"])),
                delete: Some(true),
                ..Default::default()
            },
            GroupCommand {
                group: Some("@coders".to_string()),
                remove: Some(names(&["a"])),
                delete: Some(true),
                ..Default::default()
            },
        ] {
            let err = validate_group_args(cmd).expect_err("조합 반려");
            assert_eq!(group_err_code(&err), "INVALID_GROUP_ARGS");
        }
    }

    #[test]
    fn group_args_edits_without_a_target_are_rejected() {
        // 규칙 6 — 대상 없는 변경. 조합 오류는 그룹 상태와 무관하게 항상 같은 코드다.
        for cmd in [
            GroupCommand {
                add: Some(names(&["a"])),
                ..Default::default()
            },
            GroupCommand {
                remove: Some(names(&["a"])),
                ..Default::default()
            },
            GroupCommand {
                delete: Some(true),
                ..Default::default()
            },
        ] {
            let err = validate_group_args(cmd).expect_err("대상 없음 반려");
            assert_eq!(group_err_code(&err), "INVALID_GROUP_ARGS");
        }
    }

    #[test]
    fn group_args_reject_names_that_break_the_at_namespace() {
        // ★관리 표면은 발송 갈래와 **다른 코드**를 쓴다★: 발송은 "그런 그룹 없다"(GROUP_NOT_FOUND)가 맞는
        //   사실이지만, 관리는 사용자가 만들려는 중이라 "이름이 규약 위반" 과 "아직 없다" 의 처방이 다르다.
        for bad in ["coders", "@", "  @  ", "@@x", "@a@b", ""] {
            let err = validate_group_args(GroupCommand {
                group: Some(bad.to_string()),
                ..Default::default()
            })
            .expect_err("이름 규약 반려");
            assert_eq!(group_err_code(&err), "INVALID_GROUP_NAME", "입력: {bad:?}");
        }
    }

    // ── D 리뷰 A1: 멤버 이름 정규화가 **공용 핸들러**에 있다 ────────────────────────────────

    #[test]
    fn group_args_split_and_trim_member_names_regardless_of_entrance() {
        // ★핵심★: 프라이밍이 가르치는 콤마 표기가 MCP 로 들어와도 CLI 와 **같은 최종 상태**가 돼야 한다.
        //   옛 구현은 CLI 파서에만 분해가 있어 MCP 호출이 `"alice,bob"` 이라는 유령 멤버 하나를 만들었다.
        let plan = validate_group_args(GroupCommand {
            group: Some("@coders".to_string()),
            add: Some(names(&["alice,bob", " carol ", "dave"])),
            remove: Some(names(&["eve, frank"])),
            ..Default::default()
        })
        .expect("정규화 통과");
        assert_eq!(
            plan,
            GroupPlan::Members {
                group: "@coders".to_string(),
                add: names(&["alice", "bob", "carol", "dave"]),
                remove: names(&["eve", "frank"]),
                mutating: true,
            },
            "콤마 분해 + trim + 빈 조각 제거가 입구와 무관하게 적용"
        );
    }

    #[test]
    fn group_args_never_register_a_blank_member_name() {
        // 빈 이름 멤버는 어떤 에이전트와도 매치되지 않아 영원히 skipped 되고, CLI 로 지울 수도 없었다.
        //   애초에 등록되지 않게 막는다(생성 경로를 닫으면 제거 불가 상태도 사라진다).
        let plan = validate_group_args(GroupCommand {
            group: Some("@t".to_string()),
            add: Some(names(&["a", "", "  ", "b"])),
            ..Default::default()
        })
        .expect("정규화 통과");
        assert_eq!(
            plan,
            GroupPlan::Members {
                group: "@t".to_string(),
                add: names(&["a", "b"]),
                remove: vec![],
                mutating: true,
            }
        );
    }

    #[test]
    fn group_args_reject_at_prefixed_member_names_as_nested_groups() {
        // ★round-2 리뷰 F5★: `@` 로 시작하는 이름을 멤버로 받으면 어떤 에이전트와도 매치되지 않아 영원히
        //   skipped 되는데, 응답엔 멤버로 보여 호출자가 "중첩 그룹이 된다" 고 믿는다. 등록 시점에 막는다.
        for cmd in [
            GroupCommand {
                group: Some("@t".to_string()),
                add: Some(names(&["alice", "@coders"])),
                ..Default::default()
            },
            GroupCommand {
                group: Some("@t".to_string()),
                add: Some(names(&["@all"])),
                ..Default::default()
            },
            // remove 도 막는다 — 등록될 수 없는 이름을 지우라는 요청은 중첩 기대의 신호라, 조용한
            //   no-op 보다 교정 hint 가 낫다.
            GroupCommand {
                group: Some("@t".to_string()),
                remove: Some(names(&["@coders"])),
                ..Default::default()
            },
            // 콤마 표기 안에 섞여 들어와도 정규화 **뒤** 검사라 잡힌다.
            GroupCommand {
                group: Some("@t".to_string()),
                add: Some(names(&["alice,@coders"])),
                ..Default::default()
            },
        ] {
            let err = validate_group_args(cmd).expect_err("중첩 그룹 반려");
            assert_eq!(group_err_code(&err), "INVALID_MEMBER_NAME");
        }
        // 대조군: 평범한 이름은 통과한다(과잉 차단 아님).
        assert!(validate_group_args(GroupCommand {
            group: Some("@t".to_string()),
            add: Some(names(&["alice", "bob-2", "e@mail"])),
            ..Default::default()
        })
        .is_ok());
    }

    #[test]
    fn group_args_reject_edits_whose_names_all_normalize_away() {
        // ★조용한 강등 금지★: 전부 걸러지면 변경이 순수 조회가 돼 "적용됐다" 로 오독된다 — 반려한다.
        for cmd in [
            GroupCommand {
                group: Some("@t".to_string()),
                add: Some(names(&[",,", "   "])),
                ..Default::default()
            },
            GroupCommand {
                group: Some("@t".to_string()),
                remove: Some(names(&[""])),
                ..Default::default()
            },
        ] {
            let err = validate_group_args(cmd).expect_err("쓸 이름 없음 반려");
            assert_eq!(group_err_code(&err), "INVALID_GROUP_ARGS");
        }
    }

    #[test]
    fn group_args_normalize_the_label_so_response_and_registry_agree() {
        // 라벨 단일 출처 — 응답의 group 필드는 레지스트리가 실제로 쓴 이름이어야 한다(공백 낀 입력에서
        //   응답 라벨과 저장 키가 갈리면 호출자가 자기 그룹을 목록에서 못 찾는다).
        assert_eq!(
            validate_group_args(GroupCommand {
                group: Some("  @coders  ".to_string()),
                delete: Some(true),
                ..Default::default()
            }),
            Ok(GroupPlan::Delete {
                group: "@coders".to_string()
            })
        );
    }

    #[test]
    fn query_result_error_shape_matches_the_send_entrance() {
        // 두 입구·두 동사가 같은 규칙으로 읽히려면 에러 shape 이 발송과 동일해야 한다(spec §6).
        let e = ControlQueryResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: "h".to_string(),
        };
        assert_eq!(
            e.to_json(),
            serde_json::json!({"status":"error","code":"GROUP_NOT_FOUND","hint":"h"})
        );
        assert!(!e.is_ok());
        let ok = ControlQueryResult::Ok(serde_json::json!({"groups":["@all"]}));
        assert_eq!(ok.to_json(), serde_json::json!({"groups":["@all"]}));
        assert!(ok.is_ok());
    }
}
