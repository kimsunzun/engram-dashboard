//! The trusted list's MAC: HMAC-SHA256 over the exact bytes of the stored `body` string, keyed by HKDF-SHA256 of the identity secret
//!   with a fixed label (a derived key keeps the identity secret single-purpose), compared in constant time. ★Changing the label or the
//!   derivation makes every stored list fail its check★ — the known-answer test pins both. The derived key never leaves this file and is
//!   never printed.
//!
//! What it catches (ADR-0395): naive edits and other accounts' edits — another account cannot open this account's key. Not a process of
//!   the same account (it can unseal the key) and not a rollback to an older signed list.

// ADR-0389, ADR-0395

use ring::{hkdf, hmac};

use super::identity::Secret;

/// The HKDF `info` label — fixed; see the module head.
const LABEL: &[u8] = b"engram/peer/trusted-list-mac/1";

/// The derivation failed — ring refuses only an output longer than 255 hash lengths, which a 32-byte HMAC key is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacError;

fn mac_key(secret: &Secret) -> Result<hmac::Key, MacError> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]).extract(secret.expose());
    let okm = prk
        .expand(&[LABEL], hmac::HMAC_SHA256)
        .map_err(|_| MacError)?;
    Ok(hmac::Key::from(okm))
}

/// The MAC of `body` under `secret`.
pub(crate) fn sign(secret: &Secret, body: &str) -> Result<[u8; 32], MacError> {
    let tag = hmac::sign(&mac_key(secret)?, body.as_bytes());
    <[u8; 32]>::try_from(tag.as_ref()).map_err(|_| MacError)
}

/// Does `mac` match `body` under `secret` — constant-time.
pub(crate) fn verify(secret: &Secret, body: &str, mac: &[u8; 32]) -> Result<bool, MacError> {
    Ok(hmac::verify(&mac_key(secret)?, body.as_bytes(), mac).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::peer::id::decode_hex32;

    const BODY: &str =
        r#"{"peers":[],"network":{"relay":"default","address_lookup":"default","mdns":true}}"#;

    /// Computed independently (Python `hmac` / `hashlib`, RFC 5869 with an empty salt) — pins the label and the derivation.
    #[test]
    fn the_mac_matches_a_known_answer() {
        let secret = Secret::from_bytes([0x11; 32]);
        let expected =
            decode_hex32("0d5947c5cb70304c8d4f8cac57f2ee0f0aa96ce8b0a31da326c8a4d6b7744082")
                .unwrap();
        assert_eq!(sign(&secret, BODY).unwrap(), expected);
        assert_eq!(verify(&secret, BODY, &expected), Ok(true));
    }

    #[test]
    fn another_key_or_another_body_fails() {
        let secret = Secret::from_bytes([0x11; 32]);
        let mac = sign(&secret, BODY).unwrap();
        assert_eq!(
            verify(&Secret::from_bytes([0x12; 32]), BODY, &mac),
            Ok(false)
        );
        assert_eq!(verify(&secret, &format!("{BODY} "), &mac), Ok(false));
        let mut flipped = mac;
        flipped[31] ^= 1;
        assert_eq!(verify(&secret, BODY, &flipped), Ok(false));
    }
}
