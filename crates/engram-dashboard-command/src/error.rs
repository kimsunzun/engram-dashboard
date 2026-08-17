//! 타입드 오류 모델 — 홉을 건너도 뜻이 유지되는 **하나의** 오류 어휘(TRD §4-⑦).

use std::fmt;

/// 어휘를 **한 표에서** 찍는다 — 변형 · wire 문자열 · [`ErrorCode::ALL`] 멤버십 · 재시도 지시가 같은
/// 줄에서 함께 난다.
///
/// ★목록이 둘이면 코드를 하나 더하는 편집이 한쪽만 고친다★: 손으로 적은 `as_str` 옆에서는 컴파일러가
/// 새 변형의 `as_str` 갈래만 요구하고 `ALL` 등재는 요구하지 않아, 새 코드가 **제 이름으로 나가고
/// `INTERNAL` 로 돌아오는** 비대칭이 조용히 선다(왕복 테스트가 `ALL` 을 돌므로 그것도 못 본다).
/// 여기서는 세 칸을 다 적지 않으면 규칙에 안 맞아 **컴파일이 멈춘다.**
macro_rules! error_codes {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident {
            $( $(#[$code_meta:meta])* $variant:ident => $wire:literal retry $retry:ident, )+
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( $(#[$code_meta])* $variant, )+
        }

        impl $name {
            /// 어휘 전량 — 위 표가 그대로 난다.
            const ALL: &'static [Self] = &[ $( Self::$variant, )+ ];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $wire, )+
                }
            }

            /// 모르는 코드에 `None` — 호출자가 「낮출지」를 스스로 정하게 한다.
            pub fn from_wire(code: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|c| c.as_str() == code)
            }

            /// 코드가 정하는 재시도 지시.
            ///
            /// ★한 자리에 모아 두는 이유★: 「재시도해도 되나」는 **확실성**의 함수인데(TRD §4-④),
            /// 호출처마다 손으로 붙이면 같은 코드가 자리마다 다른 지시를 달고 나간다 — 그러면 호출자는
            /// 코드가 아니라 운에 따라 재시도 여부를 정하게 된다.
            /// ★표의 세 번째 칸인 이유★: 밖에 두면 포괄 갈래(`_ => Never`)가 서고, 그러면 새 코드가
            /// **아무 결정 없이** 「재시도 금지」를 달고 나간다.
            pub const fn default_retry(self) -> RetryMode {
                match self {
                    $( Self::$variant => RetryMode::$retry, )+
                }
            }
        }
    };
}

error_codes! {
    /// 전송·라우팅 계층 + 공통 오류 코드.
    ///
    /// ★wire 표현은 문자열이고 디코더는 **모르는 코드를 받아들인다**★ — 모르는 코드는 [`ErrorCode::Internal`]
    /// 로 낮춘다([`CommandError`] 의 역직렬화가 `retry` 도 [`RetryMode::Never`] 로 낮춘다). 닫힌 열거형으로
    /// 디코드하면 코드가 하나 느는 additive 확장이 옛 클라이언트를 깨뜨린다.
    /// ★코드 추가는 additive, 뜻 변경은 금지★(TRD §4-③).
    // ADR-0140
    pub enum ErrorCode {
        InvalidArgument => "INVALID_ARGUMENT" retry Never,
        UnknownCommand => "UNKNOWN_COMMAND" retry Never,
        OwnerUnavailable => "OWNER_UNAVAILABLE" retry AfterCondition,
        OutcomeUnknown => "OUTCOME_UNKNOWN" retry SameRequestId,
        Timeout => "TIMEOUT" retry SameRequestId,
        RequestIdConflict => "REQUEST_ID_CONFLICT" retry Never,
        AlreadyApplied => "ALREADY_APPLIED" retry Never,
        AuthFailed => "AUTH_FAILED" retry Never,
        ProtocolMismatch => "PROTOCOL_MISMATCH" retry Never,
        NotFound => "NOT_FOUND" retry Never,
        Conflict => "CONFLICT" retry Never,
        Unsupported => "UNSUPPORTED" retry Never,
        Internal => "INTERNAL" retry Never,
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 재시도 지시.
///
/// ★`SameRequestId` 는 「안전하게 재실행해도 된다」가 아니다★ — 적용 여부가 **불명**이라 **같은 id 로만**
/// 다시 물어야 한다는 뜻이다(새 id 로 재시도하면 같은 조작이 두 번 적용될 수 있다 — TRD §4-⑥).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetryMode {
    Never,
    SameRequestId,
    AfterCondition,
}

impl RetryMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::SameRequestId => "same-request-id",
            Self::AfterCondition => "after-condition",
        }
    }

    pub fn from_wire(mode: &str) -> Option<Self> {
        [Self::Never, Self::SameRequestId, Self::AfterCondition]
            .into_iter()
            .find(|m| m.as_str() == mode)
    }
}

impl fmt::Display for RetryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 실패 하나 — 코드(기계) + 문구(사람) + 재시도 지시.
///
/// ★`message` 로 기계 분기하지 말 것★: 분기 재료는 `code` 하나다. 지금 CLI 가 하는 문자열 패턴매칭이
/// 이 계약이 없어서 생긴 것이고, 그 취약함을 여기서 끝낸다(TRD §4-⑦).
/// wire 형태 = `{"code":"NOT_FOUND","message":"…","retry":"never"}`.
///
/// ★동등성은 **뜻**이 정한다 — 세 칸뿐이다★: 아래 `received` 는 중계용 사본이라 비교에 끼지 않는다.
/// 끼면 「미지 코드를 낮춰 읽은 값」과 「같은 뜻으로 직접 만든 값」이 다르다고 나와, 계약(TRD §4-⑦ 의
/// 평평한 세 칸)이 말하는 같음과 코드가 말하는 같음이 갈린다.
///
/// ★세 칸은 비공개다 — 쓰기는 setter 로만 한다★: 원문 사본을 쥔 값에 칸을 직접 갈아 끼우면 재직렬화가
/// 그 원문을 내보내 **쓴 값이 조용히 무시된다**(중계 홉이 위험한 재시도를 끊으려고 `retry` 를 내려도
/// 다음 홉은 원문 지시를 그대로 읽는다 — TRD §4-④·§4-⑦). setter 는 그 경로를 없앤다:
/// **쓴 값은 언제나 그대로 나간다.**
#[derive(Debug, Clone)]
pub struct CommandError {
    code: ErrorCode,
    message: String,
    retry: RetryMode,
    /// 받은 원문 중 **타입드 세 칸이 못 나르는 것**만. 직접 만든 오류는 `None` 이다.
    ///
    /// ★수용이 파괴여선 안 된다★: 미지 코드를 `INTERNAL` 로 낮춰 읽는 것(§4-⑦)과 그것을 **다시 내보낼 때
    ///   원문을 지우는 것**은 다른 일이다. 중계 홉이 낮춘 값으로 재직렬화하면 최종 호출자는 주인이 보낸
    ///   코드를 영영 못 본다. 같은 이유로 계약 밖 필드도 그대로 들고 간다.
    ///   ★단 `null` 과 부재는 **함께** 정규화된다★ — 세 칸 중 하나가 `null` 로 실려 오면 부재로 읽고
    ///   나갈 때도 싣지 않는다(둘 다 「값 없음」이라 다시 디코드하면 같은 값이 나온다). 계약 밖 필드는
    ///   그 정규화를 받지 않고 `null` 인 채로 실려 나간다 — 뜻을 모르는 칸이라 접을 근거가 없다.
    /// 이 칸이 비공개라 구조체 리터럴로는 못 만든다 — 생성자를 쓴다.
    received: Option<Box<ReceivedError>>,
}

/// 받은 `code` 칸이 타입드 칸으로 복원되지 않는 두 얼굴 — 아는 코드는 여기 담지 않는다.
#[derive(Debug, Clone)]
enum ReceivedCode {
    /// 그 칸이 **없었거나 `null` 이었다** — 나갈 때도 싣지 않는다(없던 코드를 지어내지 않는다).
    Absent,
    /// 모르는 코드 문자열 — 낮춰 읽되 나갈 때는 받은 그대로 싣는다.
    Unknown(String),
}

/// 디코드가 붙잡아 두는 원문 조각 — [`CommandError`] 의 세 칸으로 복원할 수 없는 것만 담는다.
#[derive(Debug, Clone)]
struct ReceivedError {
    /// 아는 코드였으면 `None`(타입드 칸이 그대로 나른다). 두 예외는 [`ReceivedCode`].
    code: Option<ReceivedCode>,
    /// 받은 `retry` 원문. ★`None` = 그 필드가 **없었거나 `null` 이었다**★ — 나갈 때도 싣지 않는다.
    retry: Option<String>,
    /// `message` 칸이 실려 있었나(`null` 은 부재로 센다). 빈 문구를 지어내지 않으려고 부재를 기억한다.
    message_present: bool,
    /// 계약 밖 필드 — additive 확장(§4-③)이 중계 홉에서 증발하지 않게 그대로 나른다.
    extra: serde_json::Map<String, serde_json::Value>,
}

impl PartialEq for CommandError {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code && self.message == other.message && self.retry == other.retry
    }
}

impl Eq for CommandError {}

impl CommandError {
    /// 재시도 지시를 코드에서 파생한다 — 대부분의 호출처가 써야 하는 형태.
    pub fn of(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retry: code.default_retry(),
            received: None,
        }
    }

    /// 파생값과 다른 지시를 실어야 할 때만 쓴다.
    pub fn with_retry(code: ErrorCode, message: impl Into<String>, retry: RetryMode) -> Self {
        Self {
            code,
            message: message.into(),
            retry,
            received: None,
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retry(&self) -> RetryMode {
        self.retry
    }

    /// 코드를 갈아 끼운다 — **원문 사본을 버린다.**
    ///
    /// ★버리는 것이 요점이다★: 사본을 든 채로 코드만 바꾸면 미지 코드를 받은 값에서 재직렬화가 원문을
    /// 내보내 쓴 값이 무시된다. 함께 버려지는 것은 계약 밖 필드와 받은 `retry` 원문인데, 둘 다 **바뀌기
    /// 전 코드의 부속**이라 새 코드 옆에 두면 서로 어긋난다.
    /// 중계만 할 값이면 부르지 말 것 — 그대로 두어야 원문이 최종 호출자까지 간다.
    pub fn set_code(&mut self, code: ErrorCode) {
        self.code = code;
        self.received = None;
    }

    /// 재시도 지시를 갈아 끼운다 — **원문 사본을 버린다**([`CommandError::set_code`] 와 같은 이유).
    pub fn set_retry(&mut self, retry: RetryMode) {
        self.retry = retry;
        self.received = None;
    }

    /// 문구를 갈아 끼운다 — 사본은 그대로 둔다.
    ///
    /// 문구는 재직렬화가 **언제나 이 칸에서** 읽으므로(원문 문구를 따로 들지 않는다) 사본을 버릴 이유가
    /// 없다. 그래서 홉이 문맥을 덧붙여도 받은 코드·계약 밖 필드가 살아서 최종 호출자까지 간다.
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    /// 나갈 때 실리는 코드 문자열 — 미지 코드를 받았으면 **받은 그대로**이고, `code` 칸이 **없이**
    /// (또는 `null` 로) 온 오류에는 `None`(나갈 때도 그 칸이 없다).
    /// [`CommandError::set_code`] 로 갈아 끼운 뒤에는 그 타입드 값이 나간다.
    pub fn wire_code(&self) -> Option<&str> {
        match self.received.as_deref().map(|r| &r.code) {
            Some(Some(ReceivedCode::Absent)) => None,
            Some(Some(ReceivedCode::Unknown(code))) => Some(code),
            _ => Some(self.code.as_str()),
        }
    }

    /// 나갈 때 실리는 재시도 문자열 — 미지 지시를 받았으면 **받은 그대로**이고, `retry` 칸이 **없이**
    /// (또는 `null` 로) 온 오류에는 `None`(나갈 때도 그 칸이 없다).
    /// [`CommandError::set_retry`] 로 갈아 끼운 뒤에는 그 타입드 값이 나간다.
    pub fn wire_retry(&self) -> Option<&str> {
        match self.received.as_deref() {
            Some(received) => received.retry.as_deref(),
            None => Some(self.retry.as_str()),
        }
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::of(ErrorCode::InvalidArgument, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::of(ErrorCode::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::of(ErrorCode::Internal, message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

#[derive(serde::Serialize)]
struct WireError<'a> {
    code: &'a str,
    message: &'a str,
    retry: &'a str,
}

#[derive(serde::Deserialize)]
struct RawError {
    /// ★필수로 두지 않는다★ — 코드 없는(또는 `null` 인) 오류 하나가 디코드를 실패시키면 그것을 실은
    /// [`crate::CommandReply`] 가 통째로 못 읽히고 **상관 키까지 사라진다**. 그러면 호출자는 결말 대신
    /// 마감시각을 보게 되는데, 상대는 이미 답을 보냈다(TRD §4-⑤·§4-⑥).
    code: Option<String>,
    message: Option<String>,
    retry: Option<String>,
    /// 계약 밖 필드를 버리지 않고 모은다 — `deny_unknown_fields` 는 additive 확장을 깨므로 달지 않는다.
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl serde::Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;

        let Some(received) = self.received.as_deref() else {
            return WireError {
                code: self.code.as_str(),
                message: &self.message,
                retry: self.retry.as_str(),
            }
            .serialize(s);
        };

        let mut map = s.serialize_map(None)?;
        if let Some(code) = self.wire_code() {
            map.serialize_entry("code", code)?;
        }
        if received.message_present || !self.message.is_empty() {
            map.serialize_entry("message", &self.message)?;
        }
        if let Some(retry) = &received.retry {
            map.serialize_entry("retry", retry)?;
        }
        for (key, value) in &received.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for CommandError {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawError::deserialize(d)?;
        let known_code = raw.code.as_deref().and_then(ErrorCode::from_wire);
        let known_retry = raw.retry.as_deref().and_then(RetryMode::from_wire);
        // 낮춘 해석: 모르는 코드는 INTERNAL + never(§4-⑦), 모르는 지시는 코드의 파생값.
        //   코드가 **없는** 것도 같은 자리로 접는다 — 아는 코드가 아니면 뜻을 모르는 것은 매한가지다.
        let code = known_code.unwrap_or(ErrorCode::Internal);
        let retry = match known_code {
            Some(_) => known_retry.unwrap_or_else(|| code.default_retry()),
            None => RetryMode::Never,
        };
        // 세 칸이 받은 것을 그대로 복원할 수 있으면 사본을 안 든다 — 들 이유가 없을 때 들면 위 doc 의
        //   「손으로 갈아 끼우지 말 것」이 흔한 값에까지 번진다.
        let expressible = known_code.is_some()
            && known_retry.is_some()
            && raw.message.is_some()
            && raw.extra.is_empty();
        let received = (!expressible).then(|| {
            Box::new(ReceivedError {
                code: match (known_code, raw.code) {
                    (Some(_), _) => None,
                    (None, Some(text)) => Some(ReceivedCode::Unknown(text)),
                    (None, None) => Some(ReceivedCode::Absent),
                },
                message_present: raw.message.is_some(),
                retry: raw.retry,
                extra: raw.extra,
            })
        });
        Ok(Self {
            code,
            message: raw.message.unwrap_or_default(),
            retry,
            received,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommandReply, RequestId};

    /// ★어휘가 늘 때 나가는 이름과 들어오는 이름이 함께 늘어야 한다★ — 한쪽만 늘면 새 코드가 제 이름으로
    /// 나가고 `INTERNAL` 로 돌아와, 코드를 더한 그 편집이 스스로를 무장 해제한다.
    ///
    /// 목록 어긋남 자체는 이제 표(`error_codes!`)가 막는다 — 여기 남는 위험은 **두 변형이 같은 wire
    /// 문자열을 쓰는 것**이고, 그러면 뒤 줄이 앞 줄의 이름으로 돌아온다(`from_wire` 는 표를 훑어 먼저
    /// 맞는 것을 낸다).
    #[test]
    fn every_code_in_the_alphabet_round_trips_through_its_wire_string() {
        for code in ErrorCode::ALL {
            assert_eq!(
                ErrorCode::from_wire(code.as_str()),
                Some(*code),
                "{} 가 돌아오지 않는다",
                code.as_str()
            );
        }
    }

    /// ★코드가 없어도 답장 전체는 읽혀야 한다★ — 여기서 디코드가 실패하면 [`CommandReply`] 가 통째로
    /// 깨져 **상관 키까지 잃고**, 호출자는 이미 도착한 결말 대신 마감시각을 본다(TRD §4-⑤).
    #[test]
    fn a_reply_whose_code_is_null_still_decodes_and_keeps_its_request_id() {
        let id = RequestId::new();
        let wire = format!(
            r#"{{"request_id":"{id}","outcome":{{"Err":{{"code":null,"message":"the owner fell over"}}}}}}"#
        );

        let decoded: CommandReply =
            serde_json::from_str(&wire).expect("code 가 null 이어도 디코드된다");

        assert_eq!(decoded.request_id, id, "상관 키가 살아남는다");
        let err = decoded.outcome.expect_err("오류 답장");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(err.retry(), RetryMode::Never);
        assert_eq!(err.wire_code(), None, "없던 코드를 지어내지 않는다");
        assert_eq!(
            serde_json::to_string(&err).expect("재직렬화"),
            r#"{"message":"the owner fell over"}"#
        );
    }

    /// 칸이 아예 없는 경우도 `null` 과 같이 접힌다 — 둘 다 「값 없음」이다(아래 정규화 규칙과 같은 축).
    #[test]
    fn an_error_with_no_code_field_decodes_as_internal_and_stays_codeless() {
        let decoded: CommandError =
            serde_json::from_str(r#"{"message":"x","retry":"never"}"#).expect("디코드");

        assert_eq!(decoded.code(), ErrorCode::Internal);
        assert_eq!(decoded.retry(), RetryMode::Never);
        assert_eq!(
            serde_json::to_string(&decoded).expect("재직렬화"),
            r#"{"message":"x","retry":"never"}"#
        );
    }

    #[test]
    fn unknown_wire_code_degrades_to_internal_never() {
        let e: CommandError =
            serde_json::from_str(r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#)
                .expect("모르는 코드도 디코드된다");
        assert_eq!(e.code(), ErrorCode::Internal);
        assert_eq!(e.retry(), RetryMode::Never);
    }

    /// ★수용이 파괴여선 안 된다★ — 중계 홉이 미지 코드를 지우면 최종 호출자는 원본을 못 본다.
    ///
    /// 바이트 비교가 성립하는 것은 입력이 이미 `code`·`message`·`retry` 순서일 때다 — 재직렬화는 그 순서로
    /// 찍고 계약 밖 필드를 뒤에 붙이므로, 다른 순서로 온 원문은 **같은 뜻의 다른 바이트**로 나간다.
    #[test]
    fn an_unknown_code_and_its_retry_text_survive_a_relay_hop() {
        let wire = r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("디코드");
        assert_eq!(serde_json::to_string(&decoded).expect("재직렬화"), wire);
        assert_eq!(decoded.wire_code(), Some("FROM_THE_FUTURE"));
    }

    #[test]
    fn an_unknown_retry_alone_keeps_the_known_code_and_the_original_text() {
        let wire = r#"{"code":"TIMEOUT","message":"x","retry":"tomorrow"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("디코드");
        assert_eq!(decoded.code(), ErrorCode::Timeout);
        assert_eq!(decoded.retry(), RetryMode::SameRequestId);
        assert_eq!(serde_json::to_string(&decoded).expect("재직렬화"), wire);
    }

    #[test]
    fn known_code_keeps_wire_retry() {
        let e: CommandError = serde_json::from_str(
            r#"{"code":"OWNER_UNAVAILABLE","message":"tab.create","retry":"after-condition"}"#,
        )
        .expect("디코드");
        assert_eq!(e.code(), ErrorCode::OwnerUnavailable);
        assert_eq!(e.retry(), RetryMode::AfterCondition);
    }

    #[test]
    fn retry_is_derived_from_code() {
        assert_eq!(
            CommandError::of(ErrorCode::UnknownCommand, "x").retry(),
            RetryMode::Never
        );
        assert_eq!(
            CommandError::of(ErrorCode::OwnerUnavailable, "x").retry(),
            RetryMode::AfterCondition
        );
        assert_eq!(
            CommandError::of(ErrorCode::Timeout, "x").retry(),
            RetryMode::SameRequestId
        );
    }

    #[test]
    fn round_trips_through_wire() {
        let e = CommandError::of(ErrorCode::NotFound, "no agent 'x'");
        let text = serde_json::to_string(&e).expect("직렬화");
        assert_eq!(
            text,
            r#"{"code":"NOT_FOUND","message":"no agent 'x'","retry":"never"}"#
        );
        assert_eq!(
            serde_json::from_str::<CommandError>(&text).expect("복호"),
            e
        );
    }

    /// ★없던 칸을 지어내지 않는다★ — `retry` 없이 온 오류를 중계하면서 `""` 를 채우면 다음 홉은
    /// 계약에 없는 값을 받고, 그 값은 아무 지시도 아니다.
    #[test]
    fn an_absent_retry_stays_absent_across_a_relay_hop() {
        let wire = r#"{"code":"TIMEOUT","message":"x"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("디코드");

        assert_eq!(
            decoded.retry(),
            RetryMode::SameRequestId,
            "코드에서 파생한다"
        );
        assert_eq!(decoded.wire_retry(), None);
        assert_eq!(serde_json::to_string(&decoded).expect("재직렬화"), wire);
    }

    #[test]
    fn an_absent_message_stays_absent_across_a_relay_hop() {
        let wire = r#"{"code":"TIMEOUT","retry":"same-request-id"}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("디코드");

        assert!(decoded.message().is_empty());
        assert_eq!(serde_json::to_string(&decoded).expect("재직렬화"), wire);
    }

    /// 홉이 문구를 채워 넣으면 그건 실려 나간다 — 부재 보존은 「안 채웠을 때」의 규칙이다.
    #[test]
    fn a_message_added_by_a_hop_is_serialized() {
        let mut decoded: CommandError =
            serde_json::from_str(r#"{"code":"TIMEOUT"}"#).expect("디코드");
        decoded.set_message("took too long");

        assert_eq!(
            serde_json::to_string(&decoded).expect("재직렬화"),
            r#"{"code":"TIMEOUT","message":"took too long"}"#
        );
    }

    /// ★문구를 덧붙여도 받은 원문은 산다★ — 사본을 버리는 것은 코드·재시도 쪽 규칙이다.
    #[test]
    fn a_message_written_by_a_hop_keeps_the_relayed_code_and_extras() {
        let mut decoded: CommandError = serde_json::from_str(
            r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever","details":{"agent":"alpha"}}"#,
        )
        .expect("디코드");
        decoded.set_message("relayed through the shell");

        assert_eq!(decoded.wire_code(), Some("FROM_THE_FUTURE"));
        assert_eq!(
            serde_json::to_string(&decoded).expect("재직렬화"),
            r#"{"code":"FROM_THE_FUTURE","message":"relayed through the shell","retry":"whenever","details":{"agent":"alpha"}}"#
        );
    }

    /// ★쓴 값은 그대로 나간다★ — 위험한 재시도 루프를 끊으려고 `retry` 를 내린 홉의 지시가 재직렬화에서
    /// 원문으로 되돌아가면, 다음 홉은 끊으라는 지시를 못 보고 계속 재시도한다(TRD §4-④).
    #[test]
    fn writing_a_typed_field_is_what_goes_out_even_on_a_relayed_error() {
        let wire =
            r#"{"code":"NOT_FOUND","message":"x","retry":"after-condition","details":{"tab":1}}"#;

        let mut stop: CommandError = serde_json::from_str(wire).expect("디코드");
        stop.set_retry(RetryMode::Never);
        assert_eq!(stop.wire_retry(), Some("never"));
        assert_eq!(
            serde_json::to_string(&stop).expect("재직렬화"),
            r#"{"code":"NOT_FOUND","message":"x","retry":"never"}"#
        );

        let mut recoded: CommandError =
            serde_json::from_str(r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#)
                .expect("디코드");
        recoded.set_code(ErrorCode::Timeout);
        assert_eq!(recoded.wire_code(), Some("TIMEOUT"));
        assert_eq!(
            serde_json::to_string(&recoded).expect("재직렬화"),
            r#"{"code":"TIMEOUT","message":"x","retry":"never"}"#
        );
    }

    /// ★`null` 과 부재는 함께 접힌다★ — 접힌 형태를 다시 디코드하면 같은 값이 나오므로 잃는 뜻이 없다.
    /// 계약 밖 필드는 그 정규화를 받지 않는다(뜻을 모르는 칸이라 접을 근거가 없다).
    #[test]
    fn an_explicit_null_in_a_typed_field_is_canonicalized_to_absence() {
        let decoded: CommandError =
            serde_json::from_str(r#"{"code":"TIMEOUT","message":null,"retry":null,"hint":null}"#)
                .expect("디코드");

        assert!(decoded.message().is_empty());
        assert_eq!(
            decoded.retry(),
            RetryMode::SameRequestId,
            "코드에서 파생한다"
        );
        assert_eq!(decoded.wire_retry(), None);
        assert_eq!(
            serde_json::to_string(&decoded).expect("재직렬화"),
            r#"{"code":"TIMEOUT","hint":null}"#
        );
    }

    /// ★계약 밖 필드는 additive 확장이다(§4-③)★ — 중계 홉이 지우면 최종 호출자가 못 본다.
    #[test]
    fn unknown_extra_fields_survive_a_relay_hop() {
        let wire =
            r#"{"code":"NOT_FOUND","message":"x","retry":"never","details":{"agent":"alpha"}}"#;
        let decoded: CommandError = serde_json::from_str(wire).expect("디코드");

        assert_eq!(decoded.code(), ErrorCode::NotFound);
        assert_eq!(serde_json::to_string(&decoded).expect("재직렬화"), wire);
    }

    /// ★같음은 세 칸이 정한다★ — 낮춰 읽은 값과 같은 뜻으로 만든 값이 다르다고 나오면 계약(§4-⑦)과
    /// 코드가 서로 다른 같음을 말하게 된다.
    #[test]
    fn equality_is_the_three_contract_fields_not_the_relay_copy() {
        let downgraded: CommandError =
            serde_json::from_str(r#"{"code":"FROM_THE_FUTURE","message":"x","retry":"whenever"}"#)
                .expect("디코드");
        let built = CommandError::of(ErrorCode::Internal, "x");

        assert_eq!(downgraded, built);
        assert_ne!(
            serde_json::to_string(&downgraded).expect("직렬화"),
            serde_json::to_string(&built).expect("직렬화"),
            "같은 뜻이어도 나가는 바이트는 다르다 — 원문을 들고 있기 때문이다"
        );

        let other_message: CommandError =
            serde_json::from_str(r#"{"code":"NOT_FOUND","message":"y","retry":"never"}"#)
                .expect("디코드");
        assert_ne!(other_message, CommandError::not_found("x"));
    }
}
