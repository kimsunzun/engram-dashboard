use std::fmt;

use serde::{Deserialize, Serialize};

/// A string that must not appear in logs · errors · `{:?}` — for now the daemon connection token (TRD 3 §5 5).
///
/// - **Serialization = the inner string as is** (`serde(transparent)`) — the wire · daemon.json bytes are the same as before wrapping.
/// - **`Debug` · `Display` = a fixed placeholder** — neither the value nor the length is emitted. So the
///   `derive(Debug)` of a struct that has this type as a field cannot print the value either.
/// - Places that need the value (building frames · comparison) take it out with [`Secret::expose`] — not putting the taken-out `&str`
///   into logs · errors is the caller's job.
/// - ★`==` is not constant-time★ — it is for tests · same-value comparison. The auth decision uses a separate constant-time comparison.
/// - ★This is not the log redaction layer that ADR-0138 rejected★: it does not filter log output; the type itself
///   hides the value so that the sentences we write cannot print it (the implementation of the adopted discipline 「우리가 쓰는 문장에 자격증명을 넣지 않는다」
///   (we do not put credentials in the sentences we write)). So that rejection stands as is.
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
            "길이도 새지 않는다"
        );
        assert_eq!(format!("{short}"), format!("{long}"));
    }
}
