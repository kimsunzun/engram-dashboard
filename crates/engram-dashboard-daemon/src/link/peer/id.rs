//! A PC's ID — the 32-byte public half of its identity key — and its text forms. ★The daemon owns these forms, not the link adapter★:
//!   stored IDs (the trusted list) and IDs people paste must survive a replaced adapter, so the adapter only ever sees the raw bytes.
//!
//! Canonical text = 64 lowercase hex digits (what is stored and compared). [`PeerId::grouped`] = the same digits in groups of four for a
//!   human reading or comparing a full ID (approval questions show it; never a short fingerprint — a short one can be ground). Parsing
//!   also takes the grouped form and uppercase digits, so a pasted display reads back; anything else is refused.

// ADR-0389

use std::fmt;
use std::str::FromStr;

use engram_dashboard_base::text::hex_lower;

/// Digits in the canonical text.
pub(crate) const ID_TEXT_LEN: usize = 64;
/// Digits per group in [`PeerId::grouped`].
const GROUP: usize = 4;

/// A PC's ID. Ordered by its bytes (the tie-break between two PCs compares IDs).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PeerId([u8; 32]);

impl PeerId {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The canonical text — 64 lowercase hex digits.
    pub(crate) fn to_text(&self) -> String {
        hex_lower(&self.0)
    }

    /// The full ID in space-separated groups of four digits, for reading.
    pub(crate) fn grouped(&self) -> String {
        let text = self.to_text();
        let groups: Vec<&str> = (0..ID_TEXT_LEN / GROUP)
            .map(|i| &text[i * GROUP..(i + 1) * GROUP])
            .collect();
        groups.join(" ")
    }

    /// Reads the canonical text, the grouped form or either in uppercase. Spaces are dropped first; then exactly 64 hex digits must remain.
    pub(crate) fn parse(text: &str) -> Result<Self, IdTextError> {
        let digits: String = text.chars().filter(|c| *c != ' ').collect();
        decode_hex32(&digits).map(Self).ok_or(IdTextError)
    }
}

/// The text is not a PC ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdTextError;

impl fmt::Display for IdTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a PC ID — expected {ID_TEXT_LEN} hex digits")
    }
}

impl std::error::Error for IdTextError {}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({})", self.to_text())
    }
}

impl FromStr for PeerId {
    type Err = IdTextError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl serde::Serialize for PeerId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_text())
    }
}

/// A stored ID must be canonical — the grouped or uppercase forms are for people typing, not for the file.
impl<'de> serde::Deserialize<'de> for PeerId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        match PeerId::parse(&text) {
            Ok(id) if id.to_text() == text => Ok(id),
            _ => Err(serde::de::Error::custom(IdTextError)),
        }
    }
}

/// Exactly 64 hex digits (either case) → 32 bytes.
pub(super) fn decode_hex32(digits: &str) -> Option<[u8; 32]> {
    // Checked digit by digit: `from_str_radix` alone would also take a leading `+`.
    if digits.len() != ID_TEXT_LEN || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PeerId {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(37).wrapping_add(5);
        }
        PeerId::from_bytes(bytes)
    }

    #[test]
    fn the_text_round_trips_and_is_64_lowercase_hex_digits() {
        let id = sample();
        let text = id.to_text();
        assert_eq!(text.len(), ID_TEXT_LEN);
        assert!(text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(PeerId::parse(&text), Ok(id));
        assert_eq!(text.parse::<PeerId>(), Ok(id));
        assert_eq!(id.to_string(), text);
    }

    #[test]
    fn the_grouped_display_reads_back_and_shows_every_digit() {
        let id = sample();
        let grouped = id.grouped();
        assert_eq!(grouped.split(' ').count(), ID_TEXT_LEN / GROUP);
        assert!(grouped.split(' ').all(|g| g.len() == GROUP));
        assert_eq!(grouped.replace(' ', ""), id.to_text());
        assert_eq!(PeerId::parse(&grouped), Ok(id));
        assert_eq!(PeerId::parse(&grouped.to_uppercase()), Ok(id));
    }

    #[test]
    fn malformed_text_is_refused() {
        let text = sample().to_text();
        for bad in [
            String::new(),
            text[..63].to_string(),
            format!("{text}0"),
            format!("{}g", &text[..63]),
            format!("{}-{}", &text[..32], &text[32..]),
            format!("{}\t{}", &text[..32], &text[32..]),
            format!("{}é", &text[..62]),
            format!("+{}", &text[1..]),
        ] {
            assert_eq!(PeerId::parse(&bad), Err(IdTextError), "{bad:?}");
        }
    }

    #[test]
    fn a_stored_id_must_be_canonical() {
        let id = sample();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{}\"", id.to_text()));
        assert_eq!(serde_json::from_str::<PeerId>(&json).unwrap(), id);
        let upper = format!("\"{}\"", id.to_text().to_uppercase());
        assert!(serde_json::from_str::<PeerId>(&upper).is_err());
        let grouped = format!("\"{}\"", id.grouped());
        assert!(serde_json::from_str::<PeerId>(&grouped).is_err());
    }
}
