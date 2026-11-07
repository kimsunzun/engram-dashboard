//! 상대가 준 글을 사건 · 표시에 싣기 전에 가린다(TRD 3 §3-0 ⑤ · §5 19).
//!
//! ★상태 기계와 거는 자리의 타입이 함께 쓴다★ — [`crate::machine`] 이 사건에 싣는 자리에서,
//! [`crate::DialFailure`] 가 `Display` 에서 부른다. 한쪽 파일에 두면 seam 타입이 상태 기계를 부르게 돼 따로 둔다.

/// 가린 글의 바이트 상한 — ★글자가 아니라 바이트★다. 넘치면 문자 경계에서 자르고 말줄임표(`…`)를 붙여 이 안에
/// 맞춘다. WS 닫기 문구 상한 123 바이트보다 넉넉히 잡았다 — 로컬 진단 문구와 못 붙음의 문구도 이 길을 탄다.
pub(crate) const REASON_MAX_BYTES: usize = 256;

/// 상대가 준 글을 가린다 — 줄을 끊거나 보이지 않게 글을 바꾸거나 숨은 내용을 싣는 글자를 지우고 길이를 바이트
/// 상한(256 바이트)으로 묶는다. ★여러 번 거쳐도 같다★ — 가린 글을 다시 가려도 바뀌지 않는다.
///
/// 지우는 것 = 제어 문자(Cc) + U+2028/2029 + Default_Ignorable_Code_Point 전부 + U+FFF9–FFFB. 이것이 전부다.
/// - 제어 문자 — `char::is_control`(U+0000–001F · U+007F–009F. 줄바꿈 · 탭 · ESC 포함).
/// - 줄 · 문단 나눔 U+2028 · U+2029 — 제어 문자가 아닌데도 줄을 끊는다.
/// - Default_Ignorable_Code_Point(유니코드 속성 — 「보통은 아무것도 그리지 않는 글자」) 전부. 양방향 서식(보이는
///   순서를 뒤집어 다른 글로 읽힌다) · 폭 없는 글자 · 한글 채움 문자 · 태그 문자 U+E0000–E007F(화면에는 안 보이는데
///   LLM 은 읽는다 — 프롬프트 밀반입 통로) · 변형 선택자(256 개라 하나가 한 바이트를 실어 보이는 글자 뒤에 임의
///   데이터를 숨긴다 — 지워서 잃는 것은 이모지 · 한자 이체 모양뿐이라 진단 글에서는 값이 없다)가 여기 든다.
///   표준 라이브러리에 속성 표가 없어 `hides` 가 범위를 손으로 적는다(미할당 예약 범위 포함 — 속성 정의가 그렇다).
/// - 주석 서식 U+FFF9–FFFB — 위 속성에 들지 않지만 보이지 않는 구간을 연다.
///
/// ★남는 것★ — 위 밖의 서식 문자(Cf. 예: 보이는 아랍 숫자 앞머리 U+0600–0605)와 결합 문자 연속. 결합 문자 연속의
/// 피해(글자 위아래로 넘치는 그림)는 길이 상한(256 바이트)이 묶는다.
///
/// ★글을 「안전하게」 만들지 않는다★ — 길이와 숨은 글자만 다룬다. 상대가 지어낸 그럴듯한 문장은 그대로 남으므로
/// 분기 · 팝업 문안에 쓰지 않는 규칙([`crate::ConnectFailure::Rejected`])이 여전히 벽이다.
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
        // 「ignore」 를 태그 문자로 적은 것 — 화면에는 「ok」 만 보인다.
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
