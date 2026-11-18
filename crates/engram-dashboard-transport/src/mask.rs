//! Masks peer-given text before carrying it in an event · a display (TRD 3 §3-0 ⑤ · §5 19). ★Used by both the state machine and the dialing-place type★ ([`crate::machine`] event carry; `Display` of [`crate::DialFailure`]) —
//! placed in either file, the seam type would end up calling the state machine.

/// Byte cap of the masked text — ★bytes, not characters★; overflow is cut at a character boundary with an ellipsis (`…`) inside the cap. Larger than the WS close-text cap (123 bytes) because local diagnostic and could-not-connect text take this path too.
pub(crate) const REASON_MAX_BYTES: usize = 256;

/// Masks peer-given text — erases line-breaking, invisibly altering and hidden-content characters, and bounds the length (256 bytes). ★Idempotent★ — masking masked text changes nothing.
///
/// Erased: control characters (`char::is_control`: U+0000–001F · 007F–009F, includes line breaks · tabs · ESC) · U+2028/2029 (break lines though not control) · all Default_Ignorable_Code_Point (「draws nothing」:
/// bidirectional formatting that flips visible order · zero-width · Hangul filler · tag characters U+E0000–E007F, invisible but read by an LLM — a prompt-smuggling channel · variation selectors, 256 of them, each
/// hiding a byte behind a visible character; losing emoji · CJK variant glyphs costs nothing in diagnostics; `hides` writes the ranges by hand since std has no property table, including unassigned reserved ranges)
/// · annotation formatting U+FFF9–FFFB (opens an invisible stretch). ★Remaining★: other format characters (Cf, e.g. U+0600–0605) and runs of combining characters (overflow bounded by the length cap).
///
/// ★It does not make text 「safe」★ — only length and hidden characters; a plausible sentence the peer made up stays, so not branching on it · not using it as popup text ([`crate::ConnectFailure::Rejected`]) remains the wall.
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
        assert_eq!(mask_reason("버전 불일치"), "버전 불일치"); // hangul-ok
    }

    #[test]
    fn masking_drops_line_separators_and_invisible_format_characters() {
        assert_eq!(
            mask_reason("a\u{2028}b\u{2029}c"),
            "abc",
            "line · paragraph separators"
        );
        assert_eq!(
            mask_reason(
                "\u{feff}p\u{200b}w\u{200c}n\u{200d}e\u{ad}d\u{180e}!\u{2060}\u{2064}\u{206f}"
            ),
            "pwned!",
            "BOM · zero-width characters · invisible formatting"
        );
        assert_eq!(
            mask_reason("x\u{fff9}y\u{fffa}z\u{fffb}"),
            "xyz",
            "annotation formatting"
        );
    }

    #[test]
    fn masking_drops_tag_characters_that_smuggle_text_to_an_llm() {
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
        assert_eq!(mask_reason(&exact), exact, "unchanged if within the cap");

        let long = "가".repeat(REASON_MAX_BYTES); // hangul-ok
        let cut = mask_reason(&long);
        assert!(cut.len() <= REASON_MAX_BYTES, "{}", cut.len());
        assert!(cut.ends_with('…'));
        assert!(cut.trim_end_matches('…').chars().all(|c| c == '가')); // hangul-ok
        assert_eq!(mask_reason(&cut), cut, "masking twice gives the same");
    }

    #[test]
    fn hidden_characters_do_not_count_toward_the_cap() {
        let padded = format!("{}{}", "\u{200b}".repeat(REASON_MAX_BYTES), "ok");
        assert_eq!(mask_reason(&padded), "ok");
    }
}
