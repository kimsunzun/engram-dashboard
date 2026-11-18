use std::fmt;

use serde::{Deserialize, Serialize};

/// A string that must not appear in logs · errors · `{:?}` — for now the daemon connection token (TRD 3 §5 5).
///
/// - Serialization = the inner string as is (`serde(transparent)`).
/// - `Debug` · `Display` = a fixed placeholder (neither value nor length), so a struct deriving `Debug` with this field cannot print it either.
/// - Callers that need the value use [`Secret::expose`] and must keep the `&str` out of logs · errors.
/// - ★`==` is not constant-time★ — for tests · same-value comparison; the auth decision uses a separate constant-time comparison.
/// - ★Not the log redaction layer ADR-0138 rejected★: nothing is filtered; the type itself hides the value (the adopted discipline "we do not put credentials in
///   the sentences we write").
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

const REDACTED: &str = "Secret(<redacted>)";

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALUE: &str = "deadbeefcafef00d";

    #[test]
    fn serializes_as_the_bare_string() {
        let json = serde_json::to_string(&Secret::new(VALUE)).unwrap();
        assert_eq!(json, format!("\"{VALUE}\""));
        assert_eq!(json, serde_json::to_string(VALUE).unwrap());
    }

    #[test]
    fn deserializes_from_the_bare_string() {
        let back: Secret = serde_json::from_str(&format!("\"{VALUE}\"")).unwrap();
        assert_eq!(back.expose(), VALUE);
    }

    #[test]
    fn debug_and_display_hide_value_and_length() {
        let short = Secret::new("z9");
        let long = Secret::new(VALUE);
        for s in [&short, &long] {
            assert!(!format!("{s:?}").contains(s.expose()));
            assert!(!format!("{s:#?}").contains(s.expose()));
            assert!(!format!("{s}").contains(s.expose()));
        }
        assert_eq!(
            format!("{short:?}"),
            format!("{long:?}"),
            "the length does not leak either"
        );
        assert_eq!(format!("{short}"), format!("{long}"));
    }
}
