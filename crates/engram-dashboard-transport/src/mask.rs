//! Masks text the peer gave before carrying it in an event · a display (TRD 3 §3-0 ⑤ · §5 19).
//!
//! ★The state machine and the dialing-place type use it together★ — it is called where [`crate::machine`] carries it in an event, and
//! from `Display` by [`crate::DialFailure`]. If placed in either of those files, the seam type would end up calling the state machine, so it is kept separate.

/// Byte cap of the masked text — ★bytes, not characters★. If it overflows, it is cut at a character boundary and an ellipsis (`…`) is added to fit within this.
/// It is set with more room than the WS close-text cap of 123 bytes — the local diagnostic text and the could-not-connect text take this path too.
pub(crate) const REASON_MAX_BYTES: usize = 256;

/// Masks text the peer gave — erases characters that break lines, alter text invisibly, or carry hidden content, and bounds the length to the byte
/// cap (256 bytes). ★The same after several passes★ — masking already masked text again changes nothing.
///
/// What it erases = control characters (Cc) + U+2028/2029 + all Default_Ignorable_Code_Point + U+FFF9–FFFB. That is all.
/// - Control characters — `char::is_control` (U+0000–001F · U+007F–009F. Includes line breaks · tabs · ESC).
/// - Line · paragraph separators U+2028 · U+2029 — they break lines even though they are not control characters.
/// - All Default_Ignorable_Code_Point (a Unicode property — 「보통은 아무것도 그리지 않는 글자」 (characters that normally draw nothing)). It covers bidirectional formatting (flips the visible
///   order so it reads as different text) · zero-width characters · Hangul filler characters · tag characters U+E0000–E007F (invisible on screen but
///   an LLM reads them — a prompt-smuggling channel) · variation selectors (there are 256, so one carries a byte and hides arbitrary
///   data behind a visible character — all that erasing loses is emoji · CJK variant glyph shapes, so they have no value in diagnostic text).
///   The standard library has no property table, so `hides` writes the ranges by hand (including unassigned reserved ranges — the property definition is that way).
/// - Annotation formatting U+FFF9–FFFB — not in the property above but they open an invisible stretch.
///
/// ★What remains★ — format characters outside the above (Cf. e.g. the visible Arabic-number prefix U+0600–0605) and runs of combining characters. The
/// damage of runs of combining characters (glyphs overflowing above and below the line) is bounded by the length cap (256 bytes).
///
/// ★It does not make text 「안전하게」 (safe)★ — it handles only length and hidden characters. A plausible sentence the peer made up
/// remains as is, so the rule of not using it for branching · popup text ([`crate::ConnectFailure::Rejected`]) is still the wall.
pub fn mask_reason(text: &str) -> String {
    const ELLIPSIS: char = '…';
    let mut out: String = text.chars().filter(|c| !hides(*c)).collect();
    if out.len() > REASON_MAX_BYTES {
        let cut = out.floor_char_boundary(REASON_MAX_BYTES - ELLIPSIS.len_utf8());
        out.truncate(cut);
        out.push(ELLIPSIS);
    }
    out
}

fn hides(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_drops_control_and_bidi_characters_and_keeps_the_rest() {
        assert_eq!(mask_reason("version 3 != 4"), "version 3 != 4");
        assert_eq!(
            mask_reason("a\u{1b}[31mb\r\nc\td\u{7f}e\u{9b}f"),
            "a[31mbcdef"
        );
        assert_eq!(
            mask_reason("ok\u{202e}gnp.exe\u{2066}\u{200f}\u{61c}"),
            "okgnp.exe"
        );
        assert_eq!(mask_reason("버전 불일치"), "버전 불일치");
    }

    #[test]
    fn masking_drops_line_separators_and_invisible_format_characters() {
        assert_eq!(mask_reason("a\u{2028}b\u{2029}c"), "abc", "줄 · 문단 나눔");
        assert_eq!(
            mask_reason(
                "\u{feff}p\u{200b}w\u{200c}n\u{200d}e\u{ad}d\u{180e}!\u{2060}\u{2064}\u{206f}"
            ),
            "pwned!",
            "BOM · 폭 없는 글자 · 보이지 않는 서식"
        );
        assert_eq!(
            mask_reason("x\u{fff9}y\u{fffa}z\u{fffb}"),
            "xyz",
            "주석 서식"
        );
    }

    #[test]
    fn masking_drops_tag_characters_that_smuggle_text_to_an_llm() {
        // 「ignore」 written with tag characters — only 「ok」 shows on screen.
        let smuggled: String = "ignore"
            .chars()
            .map(|c| char::from_u32(0xE0000 + c as u32).unwrap())
            .collect();
        let text = format!("\u{e0001}ok{smuggled}\u{e007f}");
        assert_eq!(mask_reason(&text), "ok");
    }

    #[test]
    fn masking_drops_variation_selectors_that_carry_hidden_bytes() {
        assert_eq!(mask_reason("hi\u{fe00}\u{fe0f}\u{e0100}\u{e01ef}!"), "hi!");
    }

    #[test]
    fn masking_drops_hangul_filler() {
        assert_eq!(mask_reason("a\u{3164}b"), "ab");
    }

    #[test]
    fn masking_drops_mongolian_free_variation_selector() {
        assert_eq!(mask_reason("a\u{180b}b"), "ab");
    }

    #[test]
    fn masking_drops_unassigned_default_ignorable_after_tags() {
        assert_eq!(mask_reason("a\u{e0080}b"), "ab");
    }

    #[test]
    fn masking_drops_the_last_default_ignorable_in_plane_14() {
        assert_eq!(mask_reason("a\u{e0fff}b"), "ab");
    }

    #[test]
    fn masking_caps_the_length_on_a_char_boundary_and_says_it_cut() {
        let exact = "x".repeat(REASON_MAX_BYTES);
        assert_eq!(mask_reason(&exact), exact, "상한 안이면 그대로");

        let long = "가".repeat(REASON_MAX_BYTES);
        let cut = mask_reason(&long);
        assert!(cut.len() <= REASON_MAX_BYTES, "{}", cut.len());
        assert!(cut.ends_with('…'));
        assert!(cut.trim_end_matches('…').chars().all(|c| c == '가'));
        assert_eq!(mask_reason(&cut), cut, "두 번 가려도 같다");
    }

    #[test]
    fn hidden_characters_do_not_count_toward_the_cap() {
        let padded = format!("{}{}", "\u{200b}".repeat(REASON_MAX_BYTES), "ok");
        assert_eq!(mask_reason(&padded), "ok");
    }
}
