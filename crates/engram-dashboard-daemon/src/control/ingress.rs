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
//!   - 발신 인자의 **구문 검증은 전부 여기 소유**다 — 서비스 위임 전에 끝나고, 양 입구가 같은 반려를 받는다.
//!
//! tauri import 0(daemon crate).

use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_messaging::envelope::{new_msg_id, Entrance, EnvelopeFormat};

use super::registry::{BoundIdentity, ControlRegistry};

/// body **문자열** 자체의 상한 — MCP 라우트의 전송 계층 상한(`mcp_server` `RequestBodyLimitLayer`)과 별개다.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 정규화된 제어 커맨드(ADR-0086) — 두 입구가 이 형태로 만들어 `handle_send` 에 넘긴다.
#[derive(Debug, Clone)]
pub struct ControlCommand {
    pub from: BoundIdentity,
    /// 각 원소 = 에이전트 이름 · 정확한 AgentId 문자열 · `@`주소(혼용 가능). **콤마 분해는 CLI 입구
    ///   전용**이라 어댑터가 이 목록을 만들 때 이미 끝나 있다(MCP 배열 원소는 절대 쪼개지 않는다 — spec §6).
    pub to: Vec<String>,
    /// 순수 텍스트(첨부·구조화는 범위 밖).
    pub body: String,
    pub contract: SendContract,
}

/// ★회신 계약 발송 인자(C3 · spec §6 `send_message { …, request?, reply_by?, reply_to? }`)★.
///
/// ★왜 별도 struct 인가★: 세 인자는 전부 **선택**이고 기본값(`Default`)이 곧 "통보"다. `ControlCommand` 에
///   평평하게 늘어놓으면 plain 발송을 만드는 모든 자리(테스트·스모크 bin 포함)가 세 필드를 매번 써야 한다.
/// ★어댑터는 파싱하지 않는다★: 이 struct 는 **날것 그대로**를 나르고 검증은 `validate_contract` 한 곳뿐이다.
// ADR-0103 (결정 2/3 — request 타입 + 회신 계약)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendContract {
    /// `true` = 회신 요구(장부에 미회신 오픈 + 봉투 `type="request"`). ★다중 수신자·`@all` 도 허용★ —
    ///   수신자마다 **독립 계약 1건**이 열린다(ADR-0111 결정 5).
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
/// ★왜 하한이 필요한가(load-bearing — 계약 정직성)★: 기한 **초과 판정은 sweep 주기(60초)에서만** 일어난다
///   (lib.rs `SWEEP_INTERVAL`). `30s` 를 받아 주면 실제 통지는 60~120초 뒤에 나가, 발신자는 "30초 뒤
///   알려준다" 는 약속을 받고 두 배 넘게 기다린다 — 지킬 수 없는 기한은 받지 않는다.
///   `s` 단위 자체는 계속 받는다(`120s` 처럼 60 이상이면 유효) — 막는 건 **값의 크기**지 표기가 아니다.
const MIN_REPLY_BY_SECS: u64 = 60;

/// ★`reply_by` 기간 표기 파서(spec §3 "기간 표기 10m/1h — 데몬이 절대시각 환산")★ — 엄격.
///
/// ★왜 관대하게 받지 않나★: 이 값은 발신 LLM 이 자유롭게 쓰는 자리라 "받아는 줬는데 뜻이 어긋나는"
///   해석(예: `"10"` 을 분으로 추측)이 조용한 계약 오차가 된다 — 모호하면 반려하고 힌트로 형태를 알려
///   자기교정시킨다.
/// (`0` 을 막는 이유: 기한 0 = 발송 즉시 초과라 notice 를 즉발한다 — 계약이 아니라 오타로 보는 게 안전하다.)
pub(crate) fn parse_reply_by(s: &str) -> Result<std::time::Duration, String> {
    let bytes = s.as_bytes();
    // 바이트 단위로 자르므로 멀티바이트 문자는 단위 매치에서 자연히 걸린다.
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

/// 인자 정합 검증 — 첫 위반에서 반려하고, 반려 코드는 전부 `INVALID_SEND_ARGS` 다(호출부가 붙인다).
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
            // ★입구 정규화로서의 trim(의도적 — 리뷰에서 제기됐으나 유지)★: 다듬은 값이 **봉투 렌더와 장부
            //   엄격 매칭 양쪽에 그대로** 쓰이므로(`SendMeta.reply_to` 하나) 매칭이 느슨해지지 않는다. 안
            //   다듬으면 `" m-7f3k"` 가 같은 id 를 가리키는데도 NoMatch 로 조용히 빗나간다.
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
        to_attr: None,
    })
}

/// ★`reply_to` 표기 규칙(spec §3 항목 7-①)★ — 회신은 **수신자가 정확히 1명**일 때만 유효하다.
///
/// ★왜 "표기 단계" 인가(load-bearing)★: `@`토큰이 섞이면 **펼침 결과가 1명이어도** 거부한다. 펼침 뒤에 세면
///   같은 발송이 로스터 상태에 따라 어떤 날은 통과하고 어떤 날은 반려돼(비결정) 발신 LLM 이 규칙을 배울 수
///   없다 — 표기만 보면 답이 항상 같다.
fn reply_to_has_single_recipient(recipients: &[String]) -> bool {
    recipients.len() == 1 && !recipients[0].starts_with('@')
}

/// ★계약 필드(request/reply_to)가 **현재 봉투 렌더 경로에서 표현 불가**인가(load-bearing)★.
///
/// ★왜 반려까지 하나(조용한 열화 거부)★: colon 렌더와 `ENGRAM_WRAP_FORMAT` 템플릿은 둘 다 id·type·
///   reply-by·in-reply-to 속성을 버린다. 그 상태의 request 는 결말이 **정해져 있다** — 수신자는 회신에 쓸
///   id 를 본 적이 없는데 장부는 엄격 매칭을 요구하므로(spec §2) 회신이 구조적으로 불가능하고, 기한이
///   지나면 "회신 없음" 통지가 **반드시** 간다(거짓 타임아웃이 보장된 계약). 회신도 `in-reply-to` 가 사라져
///   수신자가 무엇에 대한 답인지 모른다.
/// ★env 를 여기서 읽는 이유★: 템플릿은 `wrap_message` 가 **최우선**으로 보는 전역 스위치라, 이 판정이
///   registry 포맷만 보면 템플릿이 켜진 프로세스에서 그대로 새 나간다. 판정과 렌더가 같은 입력을 봐야 한다.
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

/// 한 수신자에 대한 발송 결과 — spec §6 `results[]` 원소(수신자 1명당 1행, 그룹 단위 요약이 아니다).
#[derive(Debug, Clone)]
pub struct SendResult {
    pub to: String,
    pub status: &'static str,
    pub code: Option<&'static str>,
    pub hint: Option<String>,
}

/// 제어 커맨드 처리 결과 — `to_json` 이 wire JSON 을 만든다.
///
/// ★파서 분기 기준(수용된 비대칭 — 명시)★: 성공 응답엔 top-level `status` 가 없고, 반려 응답엔 `results` 가
///   없다 — 수신 측은 `status == "error"` 존재 여부로 분기한다(shape 통일은 하지 않는다, 기존 계약 유지).
#[derive(Debug, Clone)]
pub enum ControlResult {
    Ok {
        id: String,
        results: Vec<SendResult>,
    },
    Error { code: &'static str, hint: String },
}

impl ControlResult {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ControlResult::Ok { id, results } => {
                let arr: Vec<serde_json::Value> = results
                    .iter()
                    .map(|r| {
                        let mut obj = serde_json::json!({ "to": r.to, "status": r.status });
                        if let Some(c) = r.code {
                            obj["code"] = serde_json::Value::String(c.to_string());
                        }
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

    /// ★행 실패는 여기서 실패가 아니다★: 부분 진행이 정상 경로라(ADR-0111 결정 3) "일부가 못 받았다" 는
    ///   발신자가 `results[]` 를 읽고 판단할 사실이지 호출 자체의 실패가 아니다. **전원이 실패해도 접수**다
    ///   — 발송 단위 반려(인자 오류·주소 공간 오류)만 실패로 본다.
    pub fn is_accepted(&self) -> bool {
        matches!(self, ControlResult::Ok { .. })
    }
}

/// ★듀얼 입구 공통 핸들러★ — 두 어댑터(MCP 툴 · HTTP 라우트)가 유일하게 부르는 진입점이다.
///
/// 검사 순서(첫 실패에서 발송 단위 반려):
///   0. 회신 계약 인자 정합 → `INVALID_SEND_ARGS`. 주소보다 **먼저** 본다: 순수 구문 오류라 로스터 상태와
///      무관하게 항상 같은 답이 나와야 한다.
///   0-b. 계약 필드는 XML 봉투 전용 → `INVALID_SEND_ARGS`.
///   1. 수신자 목록 정규화 — 남은 게 없으면 `INVALID_SEND_ARGS`.
///   2. `reply_to` 는 수신자 정확히 1명 → 아니면 `INVALID_SEND_ARGS`.
///   3. body 상한 → `BODY_TOO_LARGE`.
///   4. 그 외 전부 MessagingService 위임. 주소 공간 오류(`GROUP_NOT_FOUND`/`GROUP_EMPTY`)와 id 충돌만
///      발송 단위 반려로 되돌아온다.
///
/// ★부재 수신자는 여기서 걸러지지 않는다(ADR-0111 결정 1)★: 존재·동명 판정은 **발송 순간 로스터 스냅샷 한
///   장**으로 서비스가 수신자마다 한다. 스냅샷을 두 번 뜨면 반쪽 판정이 재발한다 — 옛 입구 사전검사의
///   TOCTOU 가 그 자리였다. 그래서 입구는 로스터를 보지 않는다.
/// ★self-send 허용★: `to` 에 자기 이름을 명시하면 정상 배달된다(spec §4).
// ADR-0086
// ADR-0111
pub fn handle_send(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    entrance: Entrance,
    cmd: ControlCommand,
) -> ControlResult {
    use engram_dashboard_messaging::service::{SendReject, SendStatus};

    let meta = match validate_contract(&cmd.contract) {
        Ok(m) => m,
        Err(hint) => {
            return ControlResult::Error {
                code: "INVALID_SEND_ARGS",
                hint,
            }
        }
    };

    if let Some(hint) = contract_unsupported_by_envelope(&meta, registry.envelope_format()) {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint,
        };
    }

    // ★트림은 하되 그 이상의 보정은 없다★: 이름 자체는 바이트 그대로의 주소 네임스페이스다
    //   (WYSIWYA — ADR-0101).
    let recipients: Vec<String> = cmd
        .to
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    if recipients.is_empty() {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint: "to must name at least one recipient — pass an agent name, an agent id, @here (everyone live except you), or @all (every agent in the tree, including ones that are not running, except you). A list is allowed."
                .to_string(),
        };
    }

    if meta.reply_to.is_some() && !reply_to_has_single_recipient(&recipients) {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint: "A reply goes to exactly one recipient — the agent that sent the request. Drop the extra recipients (and any @address) and send the reply to that one agent; broadcast any follow-up as a separate message."
                .to_string(),
        };
    }

    if cmd.body.len() > MAX_BODY_BYTES {
        return ControlResult::Error {
            code: "BODY_TOO_LARGE",
            hint: format!(
                "Message body exceeds the {MAX_BODY_BYTES}-byte limit; shorten it and retry."
            ),
        };
    }

    let mut msg_id = new_msg_id();
    if !registry.is_identity_live(cmd.from) {
        // ★이 로그에 body/토큰을 싣지 않는다(보안)★.
        tracing::warn!(
            from = %cmd.from.agent_id,
            from_epoch = cmd.from.epoch,
            msg_id = %msg_id,
            entrance = entrance.as_str(),
            "제어 채널 메시지 발송 — 발신자가 relay 시점에 더 이상 산 신원 아님(작성 시점 인증으로 유효, 게이트 아님·기록용 관측, ADR-0086·사용자 결정 2026-07-19)"
        );
    }

    let sender_name = sender_display_name(manager, cmd.from);

    #[cfg(feature = "test-harness")]
    registry.fire_mid_send_hook();

    let mut outcome = messaging.handle_send(
        &msg_id,
        cmd.from.into(),
        &sender_name,
        &recipients,
        &cmd.body,
        entrance,
        &meta,
    );
    if matches!(outcome, Err(SendReject::IdCollision)) {
        let collided = std::mem::replace(&mut msg_id, new_msg_id());
        tracing::error!(
            collided = %collided,
            replacement = %msg_id,
            entrance = entrance.as_str(),
            "메시지 id 충돌 — 새 id 로 1회 재시도(ADR-0103 · 사실상 불가한 경로라 난수/장부 배선을 의심할 것)"
        );
        outcome = messaging.handle_send(
            &msg_id,
            cmd.from.into(),
            &sender_name,
            &recipients,
            &cmd.body,
            entrance,
            &meta,
        );
    }

    match outcome {
        Ok(rows) => ControlResult::Ok {
            id: msg_id,
            results: rows
                .into_iter()
                .map(|r| SendResult {
                    to: r.to,
                    status: match r.status {
                        SendStatus::Delivered => "delivered",
                        SendStatus::Pending => "pending",
                        SendStatus::Failed => "failed",
                    },
                    code: r.code.map(|c| c.as_str()),
                    hint: r.hint,
                })
                .collect(),
        },
        Err(SendReject::GroupNotFound { name }) => ControlResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: format!(
                "'{name}' is not an address the broker knows. The only group addresses are @here (everyone live except you) and @all (every agent in the tree, including ones that are not running, except you); everything else must be an agent name or agent id. Fix the address and send again."
            ),
        },
        Err(SendReject::GroupEmpty) => ControlResult::Error {
            code: "GROUP_EMPTY",
            hint: "That send resolved to no recipients at all — nothing was sent. For @here it means nobody other than you is live right now; for @all it means the team tree holds nobody but you. Name an agent directly (including yourself) if you meant to reach someone specific."
                .to_string(),
        },
        Err(SendReject::IdCollision) => ControlResult::Error {
            code: "INTERNAL_ID_COLLISION",
            hint: "The daemon could not allocate a unique message id; retry the send.".to_string(),
        },
    }
}

// ── 조회 입구(D · spec §6 `messages`) ─────────────────────────────────────────────────────
//
// ★spawn_blocking 을 쓰지 않는 이유★: 이 경로엔 자식 stdin blocking write 가 없다(inject 없음). 잡는 락은
//   messaging state 하나이고 그 임계구역은 순수 자료구조 조작뿐이라(port 호출은 락 밖) 짧다 — async 워커를
//   붙들지 않는다. `handle_send` 가 blocking 풀로 가는 이유(막힌 파이프)가 여기엔 성립하지 않는다.

/// 조회 커맨드의 결과 — 발송(`ControlResult`)과 **에러 shape 이 동일**하다(`{status:"error", code, hint}`):
/// 발신 LLM·CLI 가 성공/실패를 한 규칙으로 읽는다.
///
/// ★왜 발송과 다른 타입인가★: 발송 성공은 `{id, results[]}` 로 **shape 이 고정**돼 있지만(계약), 조회 성공은
///   동사마다 모양이 다르다. 억지로 한 enum 에 넣으면 variant 가 동사마다 늘어 계약이 흐려지므로, 성공
///   payload 는 만든 쪽이 통째로 싣고 **에러 축만** 공유한다.
// Eq 가 아니라 PartialEq 인 것은 `serde_json::Value` 가 f64 를 담아 Eq 를 못 주기 때문이다.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlQueryResult {
    Ok(serde_json::Value),
    Error { code: &'static str, hint: String },
}

impl ControlQueryResult {
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

    pub fn is_ok(&self) -> bool {
        matches!(self, ControlQueryResult::Ok(_))
    }
}

/// ★`messages { id? }` 공통 핸들러(D · spec §6)★ — **읽기 전용**. 장부를 조회만 하고 어떤 상태도 바꾸지 않는다.
///
/// ★응답 shape(계약 — 두 입구 동일)★
///
/// `id` 지정 = 그 메시지의 배달 장부. 다중 수신자·`@all` 발송은 **수신자별 1행**(1 msg_id : N 배달기록):
/// ```json
/// { "id": "m-7f3k", "from": "alice", "awaiting_reply": false, "may_be_truncated": false,
///   "rows": [ { "to": "bob", "status": "delivered", "age_secs": 42, "updated_secs_ago": 40 } ] }
/// ```
/// - `status` = **장부 어휘**(`pending|delivered|replied|expired|failed|skipped`). 발송 응답 어휘
///   (`delivered|pending|failed`)와 **같은 집합이 아니다** — 둘은 spec §6 대응표가 잇는 **다른 축**이다:
///   응답은 "그 발송 호출이 어떻게 끝났나" 의 순간 스냅샷, 조회는 "그 배달기록이 지금 어디까지 갔나" 다.
/// - `code`/`hint` 는 **행에 있을 때만** 실린다 — 사후에 종결된 행의 사유다(지금 나타나는 값은
///   `RECIPIENT_DELETED` 하나).
/// - `awaiting_reply` = 이 메시지가 request 인데 아직 회신이 안 왔다(통보면 항상 false).
/// - ★`may_be_truncated`★ = `rows` 가 **그 메시지의 전부라는 보장이 없다**(인메모리 이력 링이 밀려 앞쪽
///   행이 사라졌을 수 있다). `false` 면 확실히 전부다 — 그래서 **항상 싣는다**: 참일 때만 실으면 필드의
///   부재가 완전성으로 읽혀 조회자가 남은 행을 전체로 오독한다(10인 방송의 앞 6행이 밀려나면 "4명에게만
///   나갔다" 로 읽힌다). `true` 면 `hint` 도 함께 실어 사람/LLM 이 읽을 문장으로도 알린다.
/// - 없는 id → `{status:"error", code:"MESSAGE_NOT_FOUND", hint}`.
///
/// `id` 없음 = **호출자의 미결**(세 갈래를 한 목록으로, 오래된 순):
/// ```json
/// { "me": "alice", "open": [
///     { "direction": "outbound_pending",     "id": "m-1", "from": "alice", "to": "ghost", "age_secs": 90 },
///     { "direction": "awaiting_their_reply", "id": "m-2", "from": "alice", "to": "bob",   "age_secs": 30,
///       "reply_by": "10m", "timed_out": false },
///     { "direction": "reply_owed_by_me",     "id": "m-3", "from": "carol", "to": "alice", "age_secs": 5 } ] }
/// ```
/// - `direction` = 이 줄이 무엇인지(안정 토큰). 세 값의 **할 일이 정반대**라 이 태그가 필수다.
/// - `reply_by`·`timed_out` 은 request 줄에만 실린다.
/// - 미결이 없으면 `open: []`.
///
/// ★호출자 이름 = canonical(WYSIWYA — ADR-0101)★: 장부는 이름으로 기록되므로 신원(BoundIdentity)을 발송
///   봉투와 **같은 계산**으로 표시 이름으로 바꿔 매칭한다(`sender_display_name` 재사용 — 두 곳이 갈리면
///   "내 미결" 이 남의 것으로 보이거나 통째로 비어 버린다).
pub fn handle_messages(
    manager: &Arc<AgentManager>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    from: BoundIdentity,
    id: Option<&str>,
) -> ControlQueryResult {
    let now = std::time::Instant::now();
    match id {
        Some(raw) => {
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
                    let mut row = serde_json::json!({
                        "to": r.to,
                        "status": r.status,
                        "age_secs": r.age_secs,
                        "updated_secs_ago": r.updated_secs_ago,
                    });
                    // ★있을 때만 싣는다★: 정상 행에 `code: null` 을 붙이면 조회자가 모든 행에 사유가
                    //   있다고 오독한다.
                    if let Some(code) = r.code {
                        row["code"] = serde_json::Value::String(code.to_string());
                    }
                    if let Some(hint) = &r.hint {
                        row["hint"] = serde_json::Value::String(hint.clone());
                    }
                    row
                })
                .collect();
            let mut out = serde_json::json!({
                "id": view.id,
                "from": view.from,
                "awaiting_reply": view.awaiting_reply,
                "may_be_truncated": view.may_be_truncated,
                "rows": rows,
            });
            if view.may_be_truncated {
                out["hint"] = serde_json::Value::String(
                    "The in-memory ledger has rotated, so some delivery rows for this message may already be gone — treat the list below as partial, not as the full set of recipients.".to_string(),
                );
            }
            ControlQueryResult::Ok(out)
        }
        None => {
            let me = sender_display_name(manager, from);
            let open: Vec<serde_json::Value> = messaging
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

/// ADR-0101 (WYSIWYA): 봉투 sender 이름은 수신자가 로스터·트리·라우팅에서 보는 이름과 **byte-identical**
///   해야 한다 — 안 그러면 "A: 안녕" 봉투를 받고도 로스터엔 A 가 다른 문자열로 떠 지목이 어긋난다.
fn sender_display_name(manager: &Arc<AgentManager>, from: BoundIdentity) -> String {
    if let Some(name) = manager.canonical_name(from.agent_id) {
        return name;
    }
    // 세션 수거됨(발신자 terminal). `profile.cwd` 는 raw 라 canonical 과 다를 수 있으나, 이 경로는 산
    //   세션이 없어 라우팅 대상도 아니다(표시 전용 best-effort).
    // ★`canonical_name_when_live()` 로 바꾸지 말 것(동작 보존)★: 그쪽은 display_name 이 있으면 fs 를 안
    //   보는 단축이 있어 cwd 정규화 유무가 갈리고 결과가 달라진다.
    if let Some(p) = manager.agent_snapshot(from.agent_id) {
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
    use engram_dashboard_core::agent::types::AgentId;

    fn ident(id: AgentId) -> BoundIdentity {
        BoundIdentity {
            agent_id: id,
            epoch: 0,
        }
    }
    use engram_dashboard_messaging::envelope::wrap_message;

    /// ENGRAM_WRAP_FORMAT 은 프로세스 전역 env — set/remove·미설정 단언 테스트끼리 직렬화한다
    /// (병렬 실행 시 한 테스트의 set 이 다른 테스트의 "미설정" 단언을 짓밟지 않게).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── ControlResult wire shape ──────────────────────────────
    #[test]
    fn ok_delivered_json_shape() {
        let r = ControlResult::Ok {
            id: "mid".to_string(),
            results: vec![SendResult {
                to: "bob".to_string(),
                status: "delivered",
                code: None,
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
        let r = ControlResult::Ok {
            id: "mid".to_string(),
            results: vec![SendResult {
                to: "ghost".to_string(),
                status: "pending",
                code: None,
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

    // ── ControlCommand 정규화 ──────────────
    #[test]
    fn control_command_carries_identity_not_payload_from() {
        let id = AgentId::new_v4();
        let cmd = ControlCommand {
            from: ident(id),
            to: vec!["bob".to_string()],
            body: "hi".to_string(),
            contract: Default::default(),
        };
        assert_eq!(cmd.from.agent_id, id);
    }

    // ── C3: reply_by 기간 표기 파서 ──────────────────────────────────────────────────────────

    #[test]
    fn reply_to_requires_exactly_one_non_group_recipient() {
        let one = |v: &[&str]| {
            reply_to_has_single_recipient(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert!(one(&["bob"]), "수신자 1명 = 유효");
        assert!(!one(&["bob", "carol"]), "다중 수신자 = 반려");
        assert!(
            !one(&["@all"]),
            "@토큰 단독도 반려 — 펼침 결과가 1명이어도 표기 단계에서 막는다(비결정 금지)"
        );
        assert!(!one(&["@all", "bob"]), "혼용도 반려");
        assert!(!one(&[]), "빈 목록은 애초에 유효하지 않다");
    }

    #[test]
    fn parse_reply_by_accepts_integer_plus_unit() {
        use std::time::Duration;
        assert_eq!(
            parse_reply_by("10m").expect("10m"),
            Duration::from_secs(600)
        );
        assert_eq!(parse_reply_by("1h").expect("1h"), Duration::from_secs(3600));
        assert_eq!(parse_reply_by("60s").expect("60s"), Duration::from_secs(60));
        assert_eq!(
            parse_reply_by("120s").expect("120s"),
            Duration::from_secs(120)
        );
        assert_eq!(
            parse_reply_by("720h").expect("720h = 30d"),
            Duration::from_secs(30 * 24 * 3600)
        );
    }

    #[test]
    fn parse_reply_by_rejects_below_one_minute_floor() {
        for bad in ["1s", "30s", "59s"] {
            let err = parse_reply_by(bad).expect_err("1분 미만은 반려");
            assert!(
                err.contains("1-minute minimum"),
                "hint 가 하한을 알려야: {err}"
            );
        }
        assert!(parse_reply_by("60s").is_ok(), "정확히 1분은 수용");
        assert!(parse_reply_by("1m").is_ok(), "1m 도 같은 값이라 수용");
    }

    #[test]
    fn parse_reply_by_rejects_malformed_forms() {
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

    fn req_meta() -> engram_dashboard_messaging::service::SendMeta {
        engram_dashboard_messaging::service::SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            reply_by: Some(std::time::Duration::from_secs(600)),
            reply_to: None,
            to_attr: None,
        }
    }
    fn reply_meta() -> engram_dashboard_messaging::service::SendMeta {
        engram_dashboard_messaging::service::SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
            to_attr: None,
        }
    }

    #[test]
    fn contract_fields_are_rejected_under_colon_envelope() {
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
        assert_eq!(
            contract_unsupported_by_envelope(
                &engram_dashboard_messaging::service::SendMeta::default(),
                EnvelopeFormat::Colon
            ),
            None,
            "통보는 포맷과 무관하게 허용(기존 동작 불변)"
        );
        assert_eq!(
            contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml),
            None
        );
    }

    #[test]
    fn contract_fields_are_rejected_when_wrap_template_env_is_active() {
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
        std::env::set_var("ENGRAM_WRAP_FORMAT", "");
        let under_empty = contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml);
        std::env::remove_var("ENGRAM_WRAP_FORMAT");

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

    #[test]
    fn query_result_error_shape_matches_the_send_entrance() {
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
