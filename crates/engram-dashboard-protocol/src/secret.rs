use std::fmt;

use serde::{Deserialize, Serialize};

/// 로그 · 오류 · `{:?}` 에 실리면 안 되는 문자열 — 지금은 데몬 접속 토큰(TRD 3 §5 5).
///
/// - **직렬화 = 안쪽 문자열 그대로**(`serde(transparent)`) — 감싸기 전과 wire · daemon.json 바이트가 같다.
/// - **`Debug` · `Display` = 고정 자리표시** — 값도 길이도 내지 않는다. 이 타입을 칸으로 가진 구조체의
///   `derive(Debug)` 도 그래서 값을 못 찍는다.
/// - 값이 필요한 자리(프레임 짓기 · 비교)는 [`Secret::expose`] 로 꺼낸다 — 꺼낸 `&str` 를 로그 · 오류에
///   싣지 않는 것은 부르는 쪽 몫이다.
/// - ★`==` 는 상수시간이 아니다★ — 시험 · 같은 값 대조용이다. 인증 판정은 상수시간 비교를 따로 쓴다.
/// - ★ADR-0138 이 거부한 로그 레닥션 레이어가 아니다★ — 로그 출력을 거르는 것이 아니라 타입 자체가 값을
///   가려 우리가 쓰는 문장이 값을 못 찍게 할 뿐이다(채택한 규율 「우리가 쓰는 문장에 자격증명을 넣지 않는다」의
///   구현). 그러니 그 거부는 그대로 선다.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

const REDACTED: &str = "Secret(<redacted>)";

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &str = "deadbeefcafef00d";

    #[test]
    fn serializes_as_the_bare_string() {
        let json = serde_json::to_string(&Secret::new(VALUE)).unwrap();
        assert_eq!(json, format!("\"{VALUE}\""));
        assert_eq!(json, serde_json::to_string(VALUE).unwrap());
    }

    #[test]
    fn deserializes_from_the_bare_string() {
        let back: Secret = serde_json::from_str(&format!("\"{VALUE}\"")).unwrap();
        assert_eq!(back.expose(), VALUE);
    }

    #[test]
    fn debug_and_display_hide_value_and_length() {
        let short = Secret::new("z9");
        let long = Secret::new(VALUE);
        for s in [&short, &long] {
            assert!(!format!("{s:?}").contains(s.expose()));
            assert!(!format!("{s:#?}").contains(s.expose()));
            assert!(!format!("{s}").contains(s.expose()));
        }
        assert_eq!(
            format!("{short:?}"),
            format!("{long:?}"),
            "길이도 새지 않는다"
        );
        assert_eq!(format!("{short}"), format!("{long}"));
    }
}
