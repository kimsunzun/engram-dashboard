//! 토큰 핸드셰이크 프레임 — 이 crate 가 **의미를 아는 유일한 프레임**(ADR-0129 결정 1, 0-4).
//!
//! ★왜 여기 있나★: 토큰 인증은 "이 소켓을 살릴지" 판정이라 네트워크 행의 일이다. 그런데 프레임 모양을
//! 위층(에이전트 시스템)의 명령 enum 에서 빌려 쓰면 네트워크 행이 에이전트 어휘를 **타입으로** 알게
//! 된다 — ADR-0129 결정 1 이 금지하는 바로 그것. 그래서 0-4 에서 **모양만** 이 crate 로 옮겼고, 위층의
//! 명령 enum 에서는 그 variant 를 지웠다(두 정의가 공존하면 갈라진다).
//!
//! ★★wire 는 얼려 있다 — 바이트가 움직이면 안 된다★★: 이 타입은 externally-tagged serde enum 이고
//! `#[serde(tag=…)]` 류 재정의가 **없다**. 그래서 프레임은 정확히
//!   `{"Auth":{"token":"…","protocol_version":N}}`
//! 이고, 이건 옮겨오기 전 정의가 내던 바이트와 같다. 이 crate 밖에서 이 모양을 그대로 만드는 발신자 —
//! 트레이 stop 경로(`discovery`) · 데몬 클라이언트 셸(`daemon_client/connection.rs`) · 프론트
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
//!
//! ★버전 값은 여기 없다★: 이 crate 는 "기대 버전이 몇인가" 를 모른다 — `ws::handle_connection` 이
//! 조립부에서 주입받아 비교만 한다(그래서 이 파일에도 버전 상수가 없다).

/// 연결 후 **첫 프레임** 전용 인증 프레임. 데몬은 연결 1초 안에 이것 하나만 받아들이고
/// (`ws::AUTH_TIMEOUT`), 그 뒤에 오는 같은 프레임은 위층이 거절한다.
///
/// `token` 은 daemon.json 의 256-bit hex — **로그·에러 메시지에 절대 싣지 말 것**.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AuthFrame {
    Auth {
        token: String,
        protocol_version: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★wire 계약 정본★ — **문자열 리터럴**이지 다른 타입과의 비교가 아니다. 위층 명령 enum 과
    ///   round-trip 을 맞춰 보는 형태로 쓰면 이 crate 가 지우려던 그 import 가 테스트로 되살아나고
    ///   (게이트 4 가 잡는다), 무엇보다 **두 정의가 함께 틀려도 통과**한다. golden 은 그 둘을 다 막는다.
    const GOLDEN: &str = r#"{"Auth":{"token":"deadbeefdeadbeef","protocol_version":3}}"#;

    fn golden_frame() -> AuthFrame {
        AuthFrame::Auth {
            token: "deadbeefdeadbeef".to_string(),
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
            token: "deadbeef".repeat(8), // 운영과 같은 64자
            protocol_version: 7,
        };
        let json = serde_json::to_string(&frame).unwrap();
        assert_eq!(serde_json::from_str::<AuthFrame>(&json).unwrap(), frame);
    }

    #[test]
    fn other_tags_are_not_auth() {
        // 이 crate 는 위층 명령들이 무엇인지 모르므로 태그 문자열로만 확인한다.
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
        // `ws` 의 실패 분기(첫 프레임이 auth 가 아님 / 애초에 JSON 이 아님)가 이 분류에 기댄다.
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
        // 옛 정의도 `deny_unknown_fields` 가 없었다 — 필드 추가가 옛 서버를 깨지 않는 additive 관용을
        // 그대로 물려받는다(빼면 wire 호환이 조용히 좁아진다).
        let with_extra = r#"{"Auth":{"token":"t","protocol_version":3,"future_field":"ignored"}}"#;
        assert_eq!(
            serde_json::from_str::<AuthFrame>(with_extra).unwrap(),
            AuthFrame::Auth {
                token: "t".to_string(),
                protocol_version: 3,
            }
        );
    }
}
