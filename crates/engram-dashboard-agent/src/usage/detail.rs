//! The grounds of an abnormal result (query failure · 「한도 정보 없음」 (no limit information)) — [`UsageDetail`] and [`UpstreamText`], which carries the upstream original text (ADR-0255).
//!
//! ★The destinations of the upstream original text are only the screen and the LLM read path — it is not put in logs (logs may be shared)★. That wall
//!   stands on the shape of [`UpstreamText`]: the constructor masks, replaces and truncates, and there is no `Debug` · `Display` that prints the original text.

use std::fmt;

use engram_dashboard_base::logging::mask_secrets;

use super::text::is_unprintable;

/// Character-count bound of one piece of [`UpstreamText`] (TRD §1-4 「실패 추적」).
const UPSTREAM_MAX_CHARS: usize = 200;

/// The grounds of an abnormal result — not mixed: our classification = `kind` · the upstream number = `code` · the upstream text = `upstream`.
///
/// - `kind` = classification word. It goes out over the wire and the screen shows it without translation. The producer decides — a neutral failure is
///   [`super::ProbeError::kind_word`], the classification of a vendor response is each vendor's query code.
/// - `code` = the error number the upstream gave (`None` = it gave none).
/// - `upstream` = the original text the upstream gave. ★Carry only the value of a named field★ — do not carry the response line · stderr whole (other
///   fields of the response carry the account identifier).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDetail {
    pub kind: &'static str,
    pub code: Option<i64>,
    pub upstream: Option<UpstreamText>,
}

/// One piece of original text the upstream gave — [`UpstreamText::new`] masks token shapes, replaces characters that must not be shown as is on screen (control characters ·
/// bidirectional / zero-width format characters — the same decision as the display field of `usage::text`) with a blank or U+FFFD, then
/// truncates to 200 characters (by character).
///
/// ★`Debug` prints only the character count and there is no `Display`★ — even if a log prints it with `{:?}` · `{}`, the original text does not go out. The only
///   way to take the original text out is [`UpstreamText::as_str`]. Do not add a `Display` · `Serialize` · `Debug` that emits the original text.
/// ★What is masked is only the token shape★ — things like account identifiers · paths may remain.
/// ★Replacing instead of dropping is intended★ — the display field drops such values whole, but the original-text field must stay traceable by the user.
#[derive(Clone, PartialEq, Eq)]
pub struct UpstreamText(String);

impl UpstreamText {
    /// Order = mask → replace → truncate. ★Masking comes first★ — the mask pattern must see the original token shape, and if truncated first, a token straddling the boundary
    /// falls short of the pattern's minimum length and survives. Replacement swaps one character for one character, so the 200-character bound is the character
    /// count after replacement.
    /// Replacement = whitespace-class control characters (`\t`·`\n`·`\r`·`\x0B`·`\x0C`·U+0085·U+2028·U+2029) become one blank, and any other
    /// character that must not be visible becomes U+FFFD — so that a multi-line phrase reads as one line.
    pub fn new(text: &str) -> Self {
        Self(
            mask_secrets(text)
                .chars()
                .map(|c| match c {
                    c if !is_unprintable(c) => c,
                    c if c.is_whitespace() => ' ',
                    _ => '\u{FFFD}',
                })
                .take(UPSTREAM_MAX_CHARS)
                .collect(),
        )
    }

    /// `None` if empty or whitespace only (whitespace decision = [`str::trim`]), otherwise [`UpstreamText::new`] — used by the place that carries
    /// a field value the upstream gave as is. If it is certain to be non-empty, like a sentence built by joining field values, use `new`.
    pub fn non_blank(text: &str) -> Option<Self> {
        (!text.trim().is_empty()).then(|| Self::new(text))
    }

    /// The original text — only the place that moves it to the wire calls this. Do not put it in logs · error strings.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for UpstreamText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UpstreamText(<{}자>)", self.0.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shaped_secrets_are_masked() {
        let text = UpstreamText::new(
            "denied for sk-ant-abcdefghijklmnopqrstuvwxyz0123 with Bearer abcdefghijklmnop",
        );
        assert_eq!(text.as_str(), "denied for *** with Bearer ***");
    }

    #[test]
    fn long_multibyte_text_is_cut_at_a_char_boundary() {
        for unit in ["가", "😀", "a가😀"] {
            let text = UpstreamText::new(&unit.repeat(300));
            assert_eq!(text.as_str().chars().count(), UPSTREAM_MAX_CHARS, "{unit}");
            assert!(unit.repeat(300).starts_with(text.as_str()), "{unit}");
        }
        assert_eq!(UpstreamText::new("짧다").as_str(), "짧다");
        assert_eq!(UpstreamText::new("").as_str(), "");
    }

    /// ★If truncation comes before masking, the head of a token straddling the bound survives★ — this test goes red if the order is reversed.
    #[test]
    fn a_secret_straddling_the_cap_is_masked_before_the_cut() {
        let head = "x".repeat(UPSTREAM_MAX_CHARS - 10);
        let text = UpstreamText::new(&format!("{head} sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!text.as_str().contains("sk-"), "{}", text.as_str());
        assert!(text.as_str().ends_with("***"), "{}", text.as_str());
    }

    #[test]
    fn unprintable_characters_become_replacement_chars() {
        assert_eq!(
            UpstreamText::new("a\u{202E}b\x1b[31mc").as_str(),
            "a\u{FFFD}b\u{FFFD}[31mc"
        );
        let hidden = "\u{061C}\u{200E}\u{200F}\u{202A}\u{202E}\u{2066}\u{2069}\u{FEFF}\u{1F}\u{7F}";
        assert_eq!(
            UpstreamText::new(hidden).as_str(),
            "\u{FFFD}".repeat(hidden.chars().count())
        );
    }

    /// Whitespace-class control characters become a blank — a multi-line phrase reads as one line.
    #[test]
    fn whitespace_controls_become_plain_spaces() {
        let breaks = "\t\n\r\u{0B}\u{0C}\u{85}\u{2028}\u{2029}";
        assert_eq!(
            UpstreamText::new(&format!("a{breaks}b")).as_str(),
            format!("a{}b", " ".repeat(breaks.chars().count()))
        );
        assert_eq!(
            UpstreamText::new("line one\r\nline two").as_str(),
            "line one  line two"
        );
    }

    /// ★The `\s+` of the Bearer pattern also catches a token after a tab · newline★ (base `mask_secrets`) — if replacement ran before masking
    /// and turned whitespace-class characters into U+FFFD, this token would survive.
    #[test]
    fn a_bearer_token_after_a_tab_or_newline_is_masked() {
        assert_eq!(
            UpstreamText::new("x Bearer\tabcdefghijklmnop").as_str(),
            "x Bearer ***"
        );
        assert_eq!(
            UpstreamText::new("x Bearer\nabcdefghijklmnop").as_str(),
            "x Bearer ***"
        );
    }

    #[test]
    fn the_cap_counts_characters_after_replacement() {
        let text = UpstreamText::new(&"\u{202E}".repeat(300));
        assert_eq!(text.as_str(), "\u{FFFD}".repeat(UPSTREAM_MAX_CHARS));
        let text = UpstreamText::new(&"a\u{1b}".repeat(150));
        assert_eq!(text.as_str(), "a\u{FFFD}".repeat(UPSTREAM_MAX_CHARS / 2));
    }

    #[test]
    fn a_token_next_to_a_control_character_is_still_masked() {
        let text = UpstreamText::new("\x1b[31msk-abcdefghijklmnopqrstuvwxyz\x1b[0m done");
        assert_eq!(text.as_str(), "\u{FFFD}[31m***\u{FFFD}[0m done");
    }

    #[test]
    fn non_blank_refuses_empty_and_whitespace_only_text() {
        for blank in ["", "   ", "\n\t \r\n", "\u{3000}"] {
            assert_eq!(UpstreamText::non_blank(blank), None, "{blank:?}");
        }
        assert_eq!(
            UpstreamText::non_blank(" x ").map(|t| t.as_str().to_owned()),
            Some(" x ".to_owned())
        );
        // An invisible non-whitespace character counts as non-empty — it is carried in its replaced form.
        assert_eq!(
            UpstreamText::non_blank("\u{200B}").map(|t| t.as_str().to_owned()),
            Some("\u{FFFD}".to_owned())
        );
    }

    #[test]
    fn debug_shows_only_the_length() {
        let text = UpstreamText::new("someone@example.com 계정");
        assert_eq!(format!("{text:?}"), "UpstreamText(<22자>)");
        let detail = UsageDetail {
            kind: "vendor_class",
            code: Some(7),
            upstream: Some(text),
        };
        let shown = format!("{detail:?} {detail:#?}");
        assert!(!shown.contains("example.com"), "{shown}");
        assert!(shown.contains("vendor_class"), "{shown}");
    }
}
