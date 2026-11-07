// ADR-0269
// ADR-0270

/// Normalizes **only the spelling** of a hand-typed path — strips one layer of wrapping quotes and turns `\` into `/`.
///
/// ★This is not an existence check★ — it does not look at the filesystem. It does not fix a nonexistent path and leaves relative paths as they are.
/// ★Quotes are **not stripped if the pair is empty inside**★ — stripping would turn a value that was not empty into an empty value, so the caller's
/// value vanishes after passing the empty-value check it placed before this function.
/// ★Only one layer is stripped★ — two layers mean the caller wrapped by mistake twice, and silently swallowing even that would make us guess which one is
/// the real path.
/// ★`\` → `/` runs regardless of OS★ — on POSIX `\` is a character allowed in file names, so it
/// changes that name.
/// TODO(ADR-0230): look again when porting to macOS.
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

    /// ★Three that are not stripped★ — a mismatched quote (it may be part of the path) · an empty pair (stripping makes an empty folder) ·
    /// two layers (we would have to guess which is real).
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

    /// A value that is a single quote character is not a pair, because the opening and closing sides are the same character.
    #[test]
    fn a_lone_quote_is_left_alone() {
        assert_eq!(normalize_spelling("\""), "\"");
        assert_eq!(normalize_spelling("'"), "'");
    }

    /// Only the pair at the two ends is stripped — inner quotes are left as part of the path.
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
