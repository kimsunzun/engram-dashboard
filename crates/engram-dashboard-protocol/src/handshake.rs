//! 연결 첫 프레임(토큰 핸드셰이크)의 wire 모양 — 셸이 짓고 데몬이 푼다(ADR-0129 0-4 · ADR-0310 결정 9).
//!
//! ★명령 enum([`crate::AgentCommand`])의 변형이 아니다★: 토큰 인증은 「이 소켓을 살릴지」 판정이라 그
//! 판정을 하는 네트워크 행이 명령 어휘를 몰라도 이 프레임 하나는 풀 수 있어야 한다(ADR-0129 결정 1).
//! 그래서 따로 된 타입이고, 명령 enum 에 `Auth` 를 되살리면 두 정의가 공존해 갈라진다(그 enum 쪽 주석).
//!
//! ★★wire 는 얼려 있다 — 바이트가 움직이면 안 된다★★: externally-tagged serde enum 이고 `#[serde(tag=…)]` 류
//! 재정의가 **없다**. 그래서 프레임은 정확히
//!   `{"Auth":{"token":"…","protocol_version":N}}`
//! 이다. 토큰 칸의 [`Secret`] 은 `serde(transparent)` 라 바이트가 그대로다. 이 모양을 짓는 발신자 —
//! 트레이 stop 경로(셸 `daemon_client/stop.rs`) · 데몬 클라이언트 셸(`daemon_client/connection.rs`) · 프론트
//! `wsTransport` · `scripts/engram.mjs`.
//! ★뒤의 둘은 **손조립 JS 라 컴파일러가 못 잡는다**★: 둘 다 타입 없이 객체 리터럴로 프레임을 짓고,
//! `scripts/engram.mjs` 쪽은 테스트가 덮지 않는데도 실행되는 경로다(`run-dashboard-release.bat` ·
//! `docs/process/S17-llm-control-surface/spec/trd.md`). 그래서 계약을 지키는 것은 아래 golden 문자열
//! 테스트다. 모양을 바꾸려면 그 발신자를 **동시에** 바꾸고 프로토콜 버전을 올려야 한다.
//! ★제어 CLI `engram` 은 그 명단에 없다★ — 데몬의 HTTP 제어 라우트(`<base>/control/<route>` + Bearer)로
//! 붙어 이 프레임을 만들지 않으므로, wire 를 바꿔도 그쪽은 대상이 아니다.
//!
//! ★단일 variant enum 인 이유★: 구조체로 두면 `{"token":…}` 이 되어 태그가 사라진다. 태그 `"Auth"` 가
//! wire 계약의 일부라 enum 이어야 한다(그 대가로 `AuthFrame::Auth` 라는 겹말이 생기지만, 겹말 쪽이
//! wire 태그를 눈에 보이게 한다).

use crate::Secret;

/// The auth frame, valid only as the **first frame** of a connection. The daemon accepts exactly this one within
/// net `ws::AUTH_TIMEOUT`; the layer above rejects the same frame arriving later.
///
/// `token` 은 daemon.json 의 256-bit hex · `protocol_version` 은 **보내는 쪽이 컴파일된 버전**이다 —
/// daemon.json 이 준 값을 되쏘면 받는 쪽의 버전 비교가 늘 통과해 버전 게이트가 죽는다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthFrame {
    Auth {
        token: Secret,
        protocol_version: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★wire 계약 정본★ — **문자열 리터럴**이지 다른 타입과의 비교가 아니다. 다른 타입과 round-trip 을
    ///   맞춰 보는 형태로 바꾸면 **두 정의가 함께 틀려도 통과**한다.
    const GOLDEN: &str = r#"{"Auth":{"token":"deadbeefdeadbeef","protocol_version":3}}"#;

    fn golden_frame() -> AuthFrame {
        AuthFrame::Auth {
            token: Secret::new("deadbeefdeadbeef"),
            protocol_version: 3,
        }
    }

    #[test]
    fn serializes_to_the_frozen_wire_bytes() {
        assert_eq!(serde_json::to_string(&golden_frame()).unwrap(), GOLDEN);
    }

    #[test]
    fn deserializes_the_frozen_wire_bytes() {
        let back: AuthFrame = serde_json::from_str(GOLDEN).unwrap();
        assert_eq!(back, golden_frame());
    }

    #[test]
    fn roundtrips() {
        let frame = AuthFrame::Auth {
            token: Secret::new("deadbeef".repeat(8)), // 운영과 같은 64자
            protocol_version: 7,
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<AuthFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn debug_does_not_carry_the_token() {
        let token = "deadbeef".repeat(8);
        let frame = AuthFrame::Auth {
            token: Secret::new(token.clone()),
            protocol_version: 7,
        };
        for shown in [format!("{frame:?}"), format!("{frame:#?}")] {
            assert!(!shown.contains(&token), "Debug 가 토큰을 찍었다: {shown}");
            assert!(
                shown.contains("protocol_version"),
                "가린 것은 토큰뿐: {shown}"
            );
        }
    }

    #[test]
    fn other_tags_are_not_auth() {
        for text in [
            r#"{"ListAgents":{"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            r#"{"NotACommand":true}"#,
            r#"{}"#,
            r#""Auth""#,
        ] {
            assert!(
                serde_json::from_str::<AuthFrame>(text).is_err(),
                "auth 가 아닌 프레임이 통과했다: {text}"
            );
        }
    }

    #[test]
    fn syntax_error_and_shape_error_are_distinguishable() {
        // net `ws` 의 실패 분기(첫 프레임이 auth 가 아님 / 애초에 JSON 이 아님)가 이 분류에 기댄다.
        // serde_json 이 이미 계산해 둔 값이라 재파싱 없이 갈린다.
        let shape = serde_json::from_str::<AuthFrame>(r#"{"NotACommand":true}"#).unwrap_err();
        assert!(shape.is_data(), "JSON 이지만 모양이 다름 = data 에러");
        let syntax = serde_json::from_str::<AuthFrame>("not json at all").unwrap_err();
        assert!(!syntax.is_data(), "JSON 이 아님 = data 에러가 아님");
        let empty = serde_json::from_str::<AuthFrame>("").unwrap_err();
        assert!(!empty.is_data(), "빈 프레임 = data 에러가 아님(eof)");
    }

    #[test]
    fn unknown_fields_inside_the_object_are_tolerated() {
        // `deny_unknown_fields` 가 없다 — 필드 추가가 옛 서버를 깨지 않는 additive 관용이다(빼면 wire 호환이
        // 조용히 좁아진다).
        let with_extra = r#"{"Auth":{"token":"t","protocol_version":3,"future_field":"ignored"}}"#;
        assert_eq!(
            serde_json::from_str::<AuthFrame>(with_extra).unwrap(),
            AuthFrame::Auth {
                token: Secret::new("t"),
                protocol_version: 3,
            }
        );
    }
}
