//! 비정상 결과(조회 실패·「한도 정보 없음」)의 근거 — [`UsageDetail`] 과 상류 원문을 나르는 [`UpstreamText`].
//!
//! ★상류 원문의 행선지는 화면과 LLM 읽기 경로뿐이다 — 로그에는 안 싣는다(로그는 공유될 수 있다)★. 그 벽은
//!   [`UpstreamText`] 의 모양이 선다: 생성자가 가리고 바꾸고 자르며, 원문을 찍는 `Debug`·`Display` 가 없다.

use std::fmt;

use engram_dashboard_base::logging::mask_secrets;

use super::text::is_unprintable;

/// [`UpstreamText`] 한 조각의 글자 수 상한(TRD §1-4 「실패 추적」).
const UPSTREAM_MAX_CHARS: usize = 200;

/// 비정상 결과의 근거 — 섞지 않는다: 우리 분류 = `kind` · 상류 수 = `code` · 상류 글 = `upstream`.
///
/// - `kind` = 분류 낱말. wire 로 나가고 화면이 번역 없이 보인다. 만드는 쪽이 정한다 — 중립 실패는
///   [`super::ProbeError::kind_word`], 벤더 응답의 분류는 각 벤더 조회기.
/// - `code` = 상류가 준 오류 번호(`None` = 안 줬다).
/// - `upstream` = 상류가 준 원문. ★이름 붙은 칸의 값만 싣는다★ — 응답 줄·stderr 를 통째로 싣지 않는다(응답의
///   다른 칸에 계정 식별자가 실린다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageDetail {
    pub kind: &'static str,
    pub code: Option<i64>,
    pub upstream: Option<UpstreamText>,
}

/// 상류가 준 원문 한 조각 — [`UpstreamText::new`] 가 토큰 모양을 가리고, 화면에 그대로 내면 안 되는 문자(제어 문자 ·
/// 양방향·너비 없는 서식 문자 — `usage::text` 의 표시 칸과 같은 판정)를 빈칸이나 U+FFFD 로 바꾼 뒤 200자(글자 단위)로
/// 자른다.
///
/// ★`Debug` 는 글자 수만 찍고 `Display` 는 없다★ — 로그가 `{:?}`·`{}` 로 찍어도 원문이 안 나간다. 원문을 꺼내는
///   길은 [`UpstreamText::as_str`] 하나다. 원문을 내보내는 `Display`·`Serialize`·`Debug` 를 더하지 말 것.
/// ★가리는 것은 토큰 모양뿐이다★ — 계정 식별자·경로 같은 것은 남을 수 있다.
/// ★버리지 않고 바꾸는 것은 의도다★ — 표시 칸은 그런 값을 통째로 버리지만, 원문 칸은 사용자가 추적할 수 있어야 한다.
#[derive(Clone, PartialEq, Eq)]
pub struct UpstreamText(String);

impl UpstreamText {
    /// 순서 = 가림 → 바꿈 → 자름. ★가림이 먼저다★ — 가림 패턴이 원래 토큰 모양을 봐야 하고, 먼저 자르면 경계에 걸친
    /// 토큰이 패턴의 최소 길이에 못 미쳐 살아남는다. 바꿈은 한 글자를 한 글자로 바꾸므로 200자 상한은 바꾼 뒤의
    /// 글자 수다.
    /// 바꿈 = 공백 성질의 제어 문자(`\t`·`\n`·`\r`·`\x0B`·`\x0C`·U+0085·U+2028·U+2029)는 빈칸 하나로, 그 밖의
    /// 보이면 안 되는 문자는 U+FFFD 로 — 여러 줄 문구가 한 줄로 읽히게.
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

    /// 비었거나 공백뿐이면 `None`(공백 판정 = [`str::trim`]), 그 밖은 [`UpstreamText::new`] — 상류가 준 칸 값을 그대로 싣는
    /// 자리가 쓴다. 칸 값을 붙여 만든 문장처럼 비지 않은 것이 확실하면 `new` 를 쓴다.
    pub fn non_blank(text: &str) -> Option<Self> {
        (!text.trim().is_empty()).then(|| Self::new(text))
    }

    /// 원문 — wire 로 옮기는 자리만 부른다. 로그·오류 문자열에 싣지 않는다.
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

    /// ★자르고 가리면 상한에 걸친 토큰의 머리가 살아남는다★ — 이 시험은 순서를 뒤집으면 빨개진다.
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

    /// 공백 성질의 제어 문자는 빈칸이 된다 — 여러 줄 문구가 한 줄로 읽힌다.
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

    /// ★Bearer 패턴의 `\s+` 가 탭·줄바꿈 뒤의 토큰도 잡는다★(base `mask_secrets`) — 바꿈이 가림보다 먼저 돌면서 공백 성질
    /// 문자를 U+FFFD 로 바꾸면 이 토큰이 산다.
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
        // 공백이 아닌 보이지 않는 문자는 비지 않은 것이다 — 바꾼 채로 싣는다.
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
