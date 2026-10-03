// ADR-0269
// ADR-0270

/// 사람이 친 경로의 **철자만** 고른다 — 감싼 따옴표 한 겹을 벗기고 `\` 를 `/` 로 옮긴다.
///
/// ★실재 확인이 아니다★ — 파일시스템을 보지 않는다. 없는 경로를 고쳐 주지 않고 상대경로도 그대로 둔다.
/// ★따옴표는 **안이 빈 짝이면 벗기지 않는다**★ — 벗기면 비어 있지 않던 값이 빈 값이 되어, 부르는 쪽이
/// 이 함수 앞에 둔 빈 값 검문을 지난 뒤에 값이 사라진다.
/// ★한 겹만 벗긴다★ — 두 겹은 호출자가 실수로 두 번 감쌌다는 뜻이고, 그것까지 조용히 삼키면 어느 쪽이
/// 진짜 경로인지 우리가 추측하게 된다.
/// ★`\` → `/` 는 OS 를 가리지 않고 돈다★ — POSIX 에서는 `\` 가 파일 이름에 쓸 수 있는 글자라 그 이름을
/// 바꿔 버린다.
/// TODO(ADR-0230): macOS 이식 때 다시 본다.
pub fn normalize_spelling(raw: &str) -> String {
    let unquoted = match raw.chars().next() {
        Some(quote @ ('"' | '\'')) => raw
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
            .filter(|inner| !inner.is_empty())
            .unwrap_or(raw),
        _ => raw,
    };
    unquoted.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::normalize_spelling;

    #[test]
    fn normalize_spelling_strips_one_quote_pair_and_turns_backslashes_around() {
        for (raw, want) in [
            (r"C:\work\thing", "C:/work/thing"),
            (r#""C:\work\thing""#, "C:/work/thing"),
            ("'C:/work/thing'", "C:/work/thing"),
            ("C:/work/thing", "C:/work/thing"),
            (r"\\server\share\thing", "//server/share/thing"),
        ] {
            assert_eq!(normalize_spelling(raw), want, "{raw}");
        }
    }

    /// ★안 벗기는 셋★ — 짝이 안 맞는 따옴표(경로의 일부일 수 있다) · 안이 빈 짝(벗기면 빈 폴더가 된다) ·
    /// 두 겹(어느 쪽이 진짜인지 우리가 추측하게 된다).
    #[test]
    fn normalize_spelling_leaves_quotes_it_cannot_safely_strip() {
        assert_eq!(normalize_spelling(r#""C:/work/thing"#), r#""C:/work/thing"#);
        assert_eq!(
            normalize_spelling(r#"'C:/work/thing""#),
            r#"'C:/work/thing""#
        );
        assert_eq!(normalize_spelling(r#""""#), r#""""#);
        assert_eq!(normalize_spelling(r#""#), r#""#);
        assert_eq!(
            normalize_spelling(r#"""C:/work/thing"""#),
            r#""C:/work/thing""#
        );
    }

    /// 따옴표 한 글자뿐인 값은 여는 쪽과 닫는 쪽이 같은 글자라 짝이 아니다.
    #[test]
    fn a_lone_quote_is_left_alone() {
        assert_eq!(normalize_spelling("\""), "\"");
        assert_eq!(normalize_spelling("'"), "'");
    }

    /// 벗기는 것은 양 끝의 짝뿐이다 — 안쪽 따옴표는 경로의 일부로 둔다.
    #[test]
    fn quotes_inside_the_value_stay() {
        assert_eq!(normalize_spelling(r#"C:\a "b"\c"#), r#"C:/a "b"/c"#);
        assert_eq!(normalize_spelling(r#""a"b""#), r#"a"b"#);
    }

    #[test]
    fn non_ascii_characters_survive_unquoting() {
        assert_eq!(normalize_spelling(r#""C:\작업\폴더""#), "C:/작업/폴더");
        assert_eq!(normalize_spelling("'D:\\\u{1F600}'"), "D:/\u{1F600}");
    }
}
