//! Sealing a secret so that only the current OS account on this machine can open it again — [`seal`] / [`unseal`]. Windows = DPAPI
//!   (`CryptProtectData` / `CryptUnprotectData`) in the CurrentUser scope, never LocalMachine (which any account on the machine can open),
//!   and never with a UI prompt. The blob is the caller's to store.
//!
//! Windows only for now: outside Windows sealing is `Unsupported` without trying (no OS code). The behaviour defined for other OSes is not
//!   sealing — macOS Keychain, Linux the raw secret in an owner-only 0600 file — and is built with T-118 / T-119; the `Unsupported` answer
//!   is how a caller tells there is no sealing here.

// ADR-0266, ADR-0389

use std::fmt;
use std::io;

#[cfg(windows)]
use windows::core::HRESULT;
#[cfg(windows)]
use windows::Win32::Foundation::ERROR_INVALID_DATA;
#[cfg(windows)]
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

#[cfg(windows)]
use crate::win::{io_error, LocalMem};

/// Seals `secret` for the current account. Outside Windows = `Unsupported` (module head).
#[cfg(windows)]
pub fn seal(secret: &[u8]) -> io::Result<Vec<u8>> {
    seal_with(secret, CRYPTPROTECT_UI_FORBIDDEN)
}
#[cfg(not(windows))]
pub fn seal(_secret: &[u8]) -> io::Result<Vec<u8>> {
    Err(io::ErrorKind::Unsupported.into())
}

/// [`seal`] with the DPAPI flags passed in — the tests make a LocalMachine blob with it.
#[cfg(windows)]
fn seal_with(secret: &[u8], flags: u32) -> io::Result<Vec<u8>> {
    let input = blob_of(secret)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `secret` for the call; the output buffer is LocalAlloc'd and freed by `LocalMem`.
    unsafe { CryptProtectData(&input, None, None, None, None, flags, &mut output) }
        .map_err(io_error)?;
    // SAFETY: the output buffer DPAPI allocated with LocalAlloc, freed once.
    let _free = unsafe { LocalMem::new(output.pbData.cast()) };
    // SAFETY: on success `pbData` holds `cbData` bytes.
    Ok(unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec())
}

/// Why [`unseal`] failed — the split a caller needs to choose between "the secret is lost" and "try again later".
#[derive(Debug)]
pub enum UnsealError {
    /// The blob is not one this account can open, while DPAPI itself works for this account: bad or truncated bytes, a blob sealed under
    ///   a master key this account does not hold (another account or machine, a forced password reset, a reinstall), or a blob sealed in
    ///   the LocalMachine scope (any account on the machine could have made or opened it, so it is refused before DPAPI is asked).
    Unreadable(io::Error),
    /// Any other failure — e.g. the account's master key not available yet right after logon, or a domain controller out of reach; the
    ///   same blob may open later.
    Transient(io::Error),
    /// This OS has no sealing (module head).
    Unsupported,
}

impl fmt::Display for UnsealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(e) => {
                write!(f, "the sealed data cannot be opened by this account: {e}")
            }
            Self::Transient(e) => write!(f, "the sealed data could not be opened now: {e}"),
            Self::Unsupported => f.write_str("sealing is not supported on this OS"),
        }
    }
}

impl std::error::Error for UnsealError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable(e) | Self::Transient(e) => Some(e),
            Self::Unsupported => None,
        }
    }
}

/// Opens a blob [`seal`] made for the current account. ★Only a CurrentUser blob is accepted★ — `CryptUnprotectData` would also open a
///   LocalMachine blob, which any account can make around a secret it knows and leave where this account reads its key. The plaintext copy
///   DPAPI hands out is zeroed before it is freed; the returned `Vec` is the caller's to wipe.
#[cfg(windows)]
pub fn unseal(blob: &[u8]) -> Result<Vec<u8>, UnsealError> {
    check_current_user_blob(blob).map_err(UnsealError::Unreadable)?;
    unseal_raw(blob)
        .map_err(|e| classify_unseal_failure(e.code(), io_error(e), dpapi_works_for_this_account))
}

/// `CryptUnprotectData` alone — no header check, no classification. ★The probe in [`classify_unseal_failure`] opens through this, never
///   through [`unseal`]★: a probe that classified its own failure would probe again, without end, while the master key is unreachable.
#[cfg(windows)]
fn unseal_raw(blob: &[u8]) -> windows::core::Result<Vec<u8>> {
    let input = blob_of(blob)?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `blob` for the call; the output buffer is LocalAlloc'd and freed by `LocalMem` after zeroing.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }?;
    // SAFETY: the output buffer DPAPI allocated with LocalAlloc, freed once (after zeroing below).
    let _free = unsafe { LocalMem::new(output.pbData.cast()) };
    // SAFETY: on success `pbData` holds `cbData` bytes, owned here until `_free` drops.
    let plain = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
    let secret = plain.to_vec();
    plain.fill(0);
    Ok(secret)
}
#[cfg(not(windows))]
pub fn unseal(_blob: &[u8]) -> Result<Vec<u8>, UnsealError> {
    Err(UnsealError::Unsupported)
}

#[cfg(windows)]
fn blob_of(bytes: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let len = u32::try_from(bytes.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "more than 4 GiB cannot be sealed",
        )
    })?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: len,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

/// The DPAPI blob header (layout: version u32 = 1, provider GUID, master-key version u32, master-key GUID, flags u32 at bytes 40..44)
///   must say version 1 and not carry `CRYPTPROTECT_LOCAL_MACHINE`. DPAPI picks the key by these flags, so a LocalMachine blob relabelled
///   as CurrentUser looks for a master key this account does not have and fails.
#[cfg(windows)]
fn check_current_user_blob(blob: &[u8]) -> io::Result<()> {
    use windows::Win32::Security::Cryptography::CRYPTPROTECT_LOCAL_MACHINE;
    const HEADER: usize = 44;
    let word = |at: usize| u32::from_le_bytes([blob[at], blob[at + 1], blob[at + 2], blob[at + 3]]);
    let refused = |why: &str| Err(io::Error::new(io::ErrorKind::InvalidData, why.to_string()));
    if blob.len() < HEADER {
        return refused("shorter than a DPAPI blob header");
    }
    if u32::try_from(blob.len()).is_err() {
        return refused("larger than DPAPI accepts");
    }
    if word(0) != 1 {
        return refused("not a version-1 DPAPI blob");
    }
    if word(40) & CRYPTPROTECT_LOCAL_MACHINE != 0 {
        return refused("sealed in the LocalMachine scope, not for this account");
    }
    Ok(())
}

/// Whether a failed open of a blob whose header passed means the blob is lost or only that DPAPI cannot work right now (measured on
///   Windows 11, 2026-11-20): `ERROR_INVALID_DATA` (bad or tampered bytes, a blob cut to 52 bytes or more), `ERROR_INVALID_PARAMETER`
///   (a blob cut to 44–51 bytes — the header passed, so the blob is the only argument at fault) and `NTE_BAD_DATA` (documented, not met)
///   are the blob's fault → `Unreadable`. `NTE_BAD_KEY_STATE` comes both from a blob naming a master key this account does not hold (measured) and from a master
///   key that is merely unreachable now (profile not loaded, an S4U or key-authenticated SSH logon), so `dpapi_works` is asked — a fresh
///   seal-and-open round trip: it works → the blob is at fault → `Unreadable`; it fails → `Transient`. Everything else is `Transient`.
#[cfg(windows)]
fn classify_unseal_failure(
    code: HRESULT,
    e: io::Error,
    dpapi_works: impl FnOnce() -> bool,
) -> UnsealError {
    use windows::Win32::Foundation::{ERROR_INVALID_PARAMETER, NTE_BAD_DATA, NTE_BAD_KEY_STATE};
    let blob_at_fault = code == ERROR_INVALID_DATA.to_hresult()
        || code == ERROR_INVALID_PARAMETER.to_hresult()
        || code == NTE_BAD_DATA
        || (code == NTE_BAD_KEY_STATE && dpapi_works());
    if blob_at_fault {
        UnsealError::Unreadable(e)
    } else {
        UnsealError::Transient(e)
    }
}

/// Can this account seal and open a fresh secret right now?
#[cfg(windows)]
fn dpapi_works_for_this_account() -> bool {
    dpapi_round_trip(seal, unseal_raw)
}

/// A fresh secret through `seal` and back through `open` — `open` is [`unseal_raw`] (not [`unseal`]: no recursion), a fake in the tests.
#[cfg(windows)]
fn dpapi_round_trip(
    seal: impl FnOnce(&[u8]) -> io::Result<Vec<u8>>,
    open: impl FnOnce(&[u8]) -> windows::core::Result<Vec<u8>>,
) -> bool {
    const PROBE: &[u8] = b"engram dpapi probe";
    seal(PROBE)
        .ok()
        .and_then(|blob| open(&blob).ok())
        .is_some_and(|back| back == PROBE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn sealing_is_unsupported_off_windows() {
        assert_eq!(seal(b"x").unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert!(matches!(unseal(b"x"), Err(UnsealError::Unsupported)));
    }

    #[cfg(windows)]
    #[test]
    fn a_sealed_secret_opens_again_and_is_not_stored_in_the_clear() {
        let secret: Vec<u8> = (0u8..32).collect();
        let blob = seal(&secret).expect("seal");
        assert!(
            !blob.windows(secret.len()).any(|w| w == secret.as_slice()),
            "the blob must not carry the secret in the clear"
        );
        assert_eq!(unseal(&blob).expect("unseal"), secret);
    }

    #[cfg(windows)]
    #[test]
    fn garbage_an_empty_a_truncated_and_a_tampered_blob_are_unreadable() {
        let garbage = [0x5Au8; 64];
        assert!(
            matches!(unseal(&garbage), Err(UnsealError::Unreadable(_))),
            "{:?}",
            unseal(&garbage)
        );

        assert!(
            matches!(unseal(&[]), Err(UnsealError::Unreadable(_))),
            "{:?}",
            unseal(&[])
        );

        let blob = seal(b"secret").expect("seal");
        let cut = &blob[..blob.len() / 2];
        assert!(
            matches!(unseal(cut), Err(UnsealError::Unreadable(_))),
            "{:?}",
            unseal(cut)
        );

        let just_past_the_header = &blob[..48];
        assert!(
            matches!(
                unseal(just_past_the_header),
                Err(UnsealError::Unreadable(_))
            ),
            "{:?}",
            unseal(just_past_the_header)
        );

        let mut tampered = blob.clone();
        let last = tampered.len() - 5;
        tampered[last] ^= 0xFF;
        assert!(
            matches!(unseal(&tampered), Err(UnsealError::Unreadable(_))),
            "{:?}",
            unseal(&tampered)
        );
    }

    /// A LocalMachine blob opens for any account on the machine, so `unseal` must refuse it — this also pins `seal` to CurrentUser,
    ///   whose own blob opens (the round-trip test).
    #[cfg(windows)]
    #[test]
    fn a_local_machine_blob_is_unreadable() {
        use windows::Win32::Security::Cryptography::CRYPTPROTECT_LOCAL_MACHINE;
        let blob = seal_with(
            b"secret",
            CRYPTPROTECT_UI_FORBIDDEN | CRYPTPROTECT_LOCAL_MACHINE,
        )
        .expect("seal in the LocalMachine scope");
        let outcome = unseal(&blob);
        assert!(
            matches!(outcome, Err(UnsealError::Unreadable(_))),
            "{outcome:?}"
        );
    }

    /// Another account's blob cannot be made here without a second account, so this rewrites the master-key GUID inside a real blob
    ///   (DPAPI blob layout: version u32, provider GUID, master-key version u32, master-key GUID at bytes 24..40) — DPAPI then looks for a
    ///   master key this account does not hold, which is what another account's or machine's blob looks like to it.
    #[cfg(windows)]
    #[test]
    fn a_blob_under_a_master_key_this_account_lacks_is_unreadable() {
        let mut blob = seal(b"secret").expect("seal");
        for byte in &mut blob[24..40] {
            *byte ^= 0xA5;
        }
        let outcome = unseal(&blob);
        assert!(
            matches!(outcome, Err(UnsealError::Unreadable(_))),
            "{outcome:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn bad_data_is_unreadable_without_asking_and_other_codes_are_transient() {
        use windows::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, NTE_BAD_DATA,
        };
        use windows::Win32::System::Rpc::RPC_S_SERVER_UNAVAILABLE;
        let e = || io::Error::other("x");
        let unasked = || -> bool { panic!("only NTE_BAD_KEY_STATE asks DPAPI") };
        for code in [
            ERROR_INVALID_DATA.to_hresult(),
            ERROR_INVALID_PARAMETER.to_hresult(),
            NTE_BAD_DATA,
        ] {
            assert!(
                matches!(
                    classify_unseal_failure(code, e(), unasked),
                    UnsealError::Unreadable(_)
                ),
                "{code:?}"
            );
        }
        for code in [
            ERROR_ACCESS_DENIED.to_hresult(),
            HRESULT::from_win32(RPC_S_SERVER_UNAVAILABLE.0 as u32),
        ] {
            assert!(
                matches!(
                    classify_unseal_failure(code, e(), unasked),
                    UnsealError::Transient(_)
                ),
                "{code:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_bad_key_state_is_unreadable_only_while_dpapi_works() {
        use windows::Win32::Foundation::NTE_BAD_KEY_STATE;
        let e = || io::Error::other("x");
        assert!(matches!(
            classify_unseal_failure(NTE_BAD_KEY_STATE, e(), || true),
            UnsealError::Unreadable(_)
        ));
        assert!(matches!(
            classify_unseal_failure(NTE_BAD_KEY_STATE, e(), || false),
            UnsealError::Transient(_)
        ));
    }

    /// The probe opens through the raw call, so a probe whose own open fails with `NTE_BAD_KEY_STATE` answers "not working" once —
    ///   `Transient` — instead of probing again.
    #[cfg(windows)]
    #[test]
    fn a_failing_probe_makes_a_bad_key_state_transient_without_probing_again() {
        // The fake cannot recurse whatever the code does; the real guard against recursion is that `dpapi_works_for_this_account`
        // opens through `unseal_raw`, whose return type cannot be `unseal`'s.
        use windows::Win32::Foundation::NTE_BAD_KEY_STATE;
        let mut opens = 0;
        let works = dpapi_round_trip(
            |secret| Ok(secret.to_vec()),
            |_| {
                opens += 1;
                Err(windows::core::Error::from_hresult(NTE_BAD_KEY_STATE))
            },
        );
        assert!(!works);
        assert_eq!(opens, 1, "the probe opens once and does not recurse");
        assert!(matches!(
            classify_unseal_failure(NTE_BAD_KEY_STATE, io::Error::other("x"), || works),
            UnsealError::Transient(_)
        ));
        assert!(
            dpapi_round_trip(seal, unseal_raw),
            "the real round trip works for this account"
        );
    }
}
