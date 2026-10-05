//! 바이트 · 문자열 표기 도우미.
//!
//! ★UTF-8 경계 자르기는 여기 두지 않는다★ — std `str::floor_char_boundary`(앞을 남김) ·
//! `str::ceil_char_boundary`(뒤를 남김) 한 줄이라 감쌀 몫이 없다. 자르는 자리가 그 둘을 직접 부른다.
//! ★hex 는 `hex` crate 를 들이지 않고 직접 돈다★ — base 에 의존을 늘리지 않는다(ADR-0269 결정 6).
// ADR-0269
// ADR-0275

use std::fmt::Write as _;

/// 바이트를 소문자 16진 문자열로 — 바이트 하나당 두 글자(`0x0a` → `"0a"`), 구분자 · 접두 없음.
pub fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::hex_lower;

    #[test]
    fn an_empty_slice_is_an_empty_string() {
        assert_eq!(hex_lower(&[]), "");
    }

    #[test]
    fn each_byte_is_two_lowercase_digits_with_a_leading_zero() {
        assert_eq!(hex_lower(&[0x00, 0x0a, 0xab, 0xff, 0x10]), "000aabff10");
    }

    #[test]
    fn every_byte_value_round_trips_through_two_digits() {
        let all: Vec<u8> = (0..=u8::MAX).collect();
        let s = hex_lower(&all);
        assert_eq!(s.len(), 512);
        assert!(s
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        for (i, pair) in s.as_bytes().chunks(2).enumerate() {
            let pair = std::str::from_utf8(pair).expect("ASCII");
            assert_eq!(
                u8::from_str_radix(pair, 16).expect("hex"),
                i as u8,
                "{pair}"
            );
        }
    }
}
