//! Tools by which a querier handles upstream strings — cleanup before putting into display fields (plan · model name) and word matching against error messages.
//!
//! ★Tools that move an upstream string into logs · [`super::ProbeError`] messages are not placed here★ — upstream messages may carry
//!   account information. What goes into a display field is only a short value used as a name. The only path that carries the original text of an error message is
//!   [`super::UpstreamText`] (a dedicated field that cannot be printed to logs).

/// External string → display field value. Strips surrounding whitespace, and `None` if empty or if it has a control character · format character · line/paragraph separator,
/// and truncates to `max_chars` characters (not bytes) if it overflows.
pub fn display_text(text: &str, max_chars: usize) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(is_unprintable) {
        return None;
    }
    Some(text.chars().take(max_chars).collect())
}

/// Characters that must not be shown on screen as is — control characters (including `\n` · `\t` · ESC) + [`is_invisible_format`]. A display field drops the
/// value if it has any, and the original-text field replaces them with a blank or U+FFFD so it stays traceable (rule = [`super::UpstreamText::new`]).
pub(super) fn is_unprintable(c: char) -> bool {
    c.is_control() || is_invisible_format(c)
}

/// Unicode general category Cf (format — bidirectional overrides · zero-width characters · tags etc.) + line/paragraph separators (U+2028·U+2029).
/// ★A table carried over by hand because std has no category API★ — the whole of Unicode 16.0's Cf (checked against Python `unicodedata` 16.0.0,
///   2026-09-27). Characters added to Cf in later versions are not filtered.
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

/// Whether `piece` is at the head of a word in `text` (the character right before is not ASCII alphanumeric) — with `whole` the end must also be a
/// word boundary. So that the same piece inside another word · id (`rate limit` in `moderate limit` · `log in` in `catalog in` · `429` in `req_14290a`)
/// is not read as a phrase. Case is compared as is — folding is the caller's share.
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
        // Just outside the range is not filtered.
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
