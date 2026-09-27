//! 조회기가 상류 문자열을 다루는 도구 — 표시 칸(plan·모델 이름)에 싣기 전의 정리와 오류 문구의 낱말 대조.
//!
//! ★상류 문자열을 로그·[`super::ProbeError`] 문구로 옮기는 도구는 여기 두지 않는다★ — 상류 문구에는 계정 정보가
//!   실릴 수 있다. 표시 칸에 싣는 것은 이름으로 쓰이는 짧은 값뿐이고, 오류 문구는 분류에만 쓴다.

/// 외부 문자열 → 표시 칸 값. 앞뒤 공백을 떼고, 비었거나 제어 문자·서식 문자·줄/문단 구분자가 있으면 `None`,
/// 넘치면 `max_chars` 글자(바이트가 아니다)로 자른다.
pub fn display_text(text: &str, max_chars: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty()
        || text
            .chars()
            .any(|c| c.is_control() || is_invisible_format(c))
    {
        return None;
    }
    Some(text.chars().take(max_chars).collect())
}

/// Unicode 일반 범주 Cf(서식 — 양방향 덮어쓰기·너비 없는 문자·태그 등) + 줄/문단 구분자(U+2028·U+2029).
/// ★std 에 범주 API 가 없어 손으로 옮긴 표다★ — Unicode 16.0 의 Cf 전량(Python `unicodedata` 16.0.0 으로 대조,
///   2026-09-27). 그 뒤 판에서 Cf 에 더해진 문자는 못 거른다.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// `piece` 가 `text` 안에서 낱말 머리에 있나(바로 앞이 ASCII 영숫자가 아니다) — `whole` 이면 끝도 낱말 경계여야
/// 한다. 다른 낱말·id 속의 같은 조각(`moderate limit` 의 `rate limit` · `catalog in` 의 `log in` · `req_14290a` 의
/// `429`)을 문구로 읽지 않게. 대소문자는 그대로 대조한다 — 접는 것은 호출자 몫이다.
pub fn has_word(text: &str, piece: &str, whole: bool) -> bool {
    text.match_indices(piece).any(|(at, _)| {
        let before = text.get(..at).and_then(|head| head.chars().next_back());
        let after = text
            .get(at + piece.len()..)
            .and_then(|tail| tail.chars().next());
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !(whole && after.is_some_and(|c| c.is_ascii_alphanumeric()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_text_trims_rejects_and_caps_by_chars() {
        assert_eq!(display_text("  plus \n", 32).as_deref(), Some("plus"));
        assert_eq!(display_text("", 32), None);
        assert_eq!(display_text("   ", 32), None);
        assert_eq!(display_text("pl\u{7}us", 32), None);
        assert_eq!(display_text("a\nb", 32), None);
        assert_eq!(display_text(&"가".repeat(10), 3).as_deref(), Some("가가가"));
    }

    #[test]
    fn display_text_rejects_invisible_format_characters() {
        for hidden in [
            '\u{202E}',
            '\u{202A}',
            '\u{2066}',
            '\u{2069}',
            '\u{061C}',
            '\u{200B}',
            '\u{200D}',
            '\u{200F}',
            '\u{2060}',
            '\u{2064}',
            '\u{FEFF}',
            '\u{00AD}',
            '\u{E0041}',
            '\u{2028}',
            '\u{2029}',
        ] {
            assert_eq!(
                display_text(&format!("pl{hidden}us"), 32),
                None,
                "{hidden:?}"
            );
        }
        // 범위 바로 바깥은 거르지 않는다.
        for plain in ['\u{2027}', '\u{2030}', '\u{2065}', '\u{E0080}', '·'] {
            assert_eq!(
                display_text(&format!("pl{plain}us"), 32),
                Some(format!("pl{plain}us")),
                "{plain:?}"
            );
        }
    }

    #[test]
    fn has_word_matches_only_at_word_heads() {
        assert!(has_word("please log in again", "log in", false));
        assert!(has_word("run /login", "login", false));
        assert!(!has_word("catalog index", "log in", false));
        assert!(!has_word("bloglogin", "login", false));
        assert!(has_word("status=429", "429", true));
        assert!(!has_word("req_14290a", "429", true));
        assert!(!has_word("4290", "429", true));
        assert!(has_word("4290", "429", false));
    }
}
