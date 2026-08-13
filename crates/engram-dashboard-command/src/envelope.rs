//! 대칭 봉투 — 모든 홉에서 같은 형태다(TRD §3-2).

use std::fmt;

use crate::CommandError;

/// 왕복 상관 키 — **전 구간 동일**하다(홉마다 새로 만들지 않는다, ADR-0081 결정 4).
///
/// 표현은 `engram-dashboard-protocol` 의 같은 이름 타입과 **같다**(uuid) — 홉을 건널 때 변환이 끼면
/// 상관이 그 자리에서 깨질 수 있다.
/// ★`Default` 를 달지 않는다 — 요청 id 에 기본값이란 없다★: derive 하면 nil UUID 가 나오는데 그건
/// **모든 요청이 같은 상관 키를 갖는다**는 뜻이라 답장이 남의 왕복에 붙는다. 이름이 같은 protocol 쪽
/// 타입은 `default()` 를 새 v4 로 구현해 정반대라, 형태만 보고 옮겨 쓰면 그 차이가 조용히 넘어온다
/// (봉투가 `AgentCommand` variant 로 실리는 Step 2 에서 `#[serde(default)]`·`..Default::default()` 가
/// 닿는 자리다). 만들 때는 [`RequestId::new`] 하나뿐이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RequestId(pub uuid::Uuid);

impl RequestId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 등록으로 얻은 **런타임** 주인 식별자.
///
/// ★선언된 등급이 아니다★ — 「주인」은 선언이 사는 crate 이고, 이 토큰은 그 주인이 지금 어느 연결에
/// 붙어 있는지를 가리킨다. 둘을 같은 것으로 읽으면 명부가 빌드 목록으로 되돌아간다(TRD §3-7 금지 조항).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct OwnerToken(pub String);

impl OwnerToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OwnerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 요청 봉투.
///
/// ★방향 필드가 없는 것은 의도다★ — **어느 연결에 썼는가가 방향**이다. 필드로 두면 같은 봉투가 두 가지
/// 진실(필드 값 / 실제 연결)을 갖고, 둘이 어긋나는 순간 라우팅이 갈린다(TRD §3-2).
/// ★`name` 은 겉봉, `args` 는 속★ — 데몬은 겉봉만 읽어 명령 단위 인가·관측을 하고 속은 파싱하지 않는다
/// (ADR-0081 「데몬 opaque」의 본체는 `args` 다).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandEnvelope {
    pub name: String,
    pub request_id: RequestId,
    /// ★**목적지** 주인 토큰이다 — 보낸 이가 아니다★. 앞 홉이 자기 명부의 답으로 덮어 실으므로
    /// (`route`), 받는 홉은 이 값으로 「누구 앞으로 온 것인가」를 자기 명부와 대조해 갈라 준다
    /// (TRD §3-8 의 2단 배달). ★신원으로 읽지 말 것★ — 호출자를 가리키지 않으므로 인가(TRD §4-⑧)의
    /// 재료는 `name` 과 **그 봉투가 온 연결**이다.
    pub owner: OwnerToken,
    /// 보낸 쪽 crate 의 `CATALOG_VERSION`. ★진단용이고 분기 재료가 아니다★ — 세대 번호가 crate 마다라
    /// 받는 쪽이 자기 번호와 비교해 뜻을 부여하면 틀린다(TRD §4-①).
    pub proto_ver: u32,
    pub args: serde_json::Value,
}

/// 답장 — 하나의 `request_id` 에 **정확히 하나**다(TRD §4-⑤).
///
/// wire 형태 = `{"request_id":"…","outcome":{"Ok":…}}` 또는 `{"…","outcome":{"Err":{…}}}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandReply {
    pub request_id: RequestId,
    pub outcome: Result<serde_json::Value, CommandError>,
}

impl CommandReply {
    pub fn ok(request_id: RequestId, value: serde_json::Value) -> Self {
        Self {
            request_id,
            outcome: Ok(value),
        }
    }

    pub fn err(request_id: RequestId, error: CommandError) -> Self {
        Self {
            request_id,
            outcome: Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    /// 오류는 **답장 안에 실려** 홉을 건넌다 — [`CommandError`] 의 wire 표현이 바뀌면 여기서 걸린다.
    #[test]
    fn a_reply_round_trips_both_outcomes() {
        let id = RequestId::new();

        let ok = CommandReply::ok(id, serde_json::json!({ "agent_id": "a" }));
        let text = serde_json::to_string(&ok).expect("직렬화");
        assert_eq!(
            serde_json::from_str::<CommandReply>(&text).expect("복호"),
            ok
        );

        let err = CommandReply::err(id, CommandError::of(ErrorCode::NotFound, "no agent 'x'"));
        let text = serde_json::to_string(&err).expect("직렬화");
        let decoded: CommandReply = serde_json::from_str(&text).expect("복호");
        assert_eq!(decoded, err);
        assert_eq!(
            decoded.outcome.expect_err("오류 답장").code(),
            ErrorCode::NotFound
        );
    }
}
