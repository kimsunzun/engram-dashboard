//! Helpers for byte · string notation.
//!
//! ★UTF-8 boundary truncation is not put here★ — std `str::floor_char_boundary` (keeps the front) ·
//! `str::ceil_char_boundary` (keeps the back) are one line, so there is nothing to wrap. The truncating site calls those two directly.
//! ★hex is done directly without pulling in the `hex` crate★ — dependencies are not added to base (ADR-0269 decision 6).
// ADR-0269
// ADR-0275

use std::fmt::Write as _;

/// Bytes to a lowercase hex string — two characters per byte (`0x0a` → `"0a"`), no separator · prefix.
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
