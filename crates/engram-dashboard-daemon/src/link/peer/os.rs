//! What the peer state needs from the OS, behind one seam so its tests run without DPAPI or a second session: sealing for the current
//!   account, the identity lock shared by this account's sessions on this machine, and the wall clock (the regenerate backup's name).
//!   [`OsPeer`] is the real one — each call goes straight to `platform`.

// ADR-0266, ADR-0389

use std::any::Any;
use std::io;
use std::time::SystemTime;

use engram_dashboard_platform::account_lock::{self, AccountLock};
use engram_dashboard_platform::secret::{self, UnsealError};

pub(crate) trait PeerOs: Send + Sync {
    /// Seals `secret` so only the current OS account opens it again (Windows DPAPI CurrentUser).
    fn seal(&self, secret: &[u8]) -> io::Result<Vec<u8>>;
    /// Opens a blob [`PeerOs::seal`] made; the error says lost (`Unreadable`) or not now (`Transient`).
    fn unseal(&self, blob: &[u8]) -> Result<Vec<u8>, UnsealError>;
    /// Takes the identity lock `name` (a file-name-safe text) for the current account — shared by all of its sessions on this machine;
    ///   no other account can create or hold it.
    fn lock(&self, name: &str) -> LockAttempt;
    fn now(&self) -> SystemTime;
}

/// What taking the identity lock found.
pub(crate) enum LockAttempt {
    /// Held until the value is dropped.
    Held(Box<dyn Any + Send + Sync>),
    /// Another process of this account holds it — another Engram copy with this identity runs on this machine.
    HeldByThisAccount,
    /// The lock could not be taken for an OS reason (the account unreadable, the OS call failing, or no such lock on this OS) — not
    ///   because another copy holds it. The reason is for diagnostics.
    NotHeld(String),
}

/// Prefix of the identity lock names (platform `account_lock` — nothing on disk).
const LOCK_PREFIX: &str = "engram-peer-identity.";

/// The real OS.
pub(crate) struct OsPeer;

impl PeerOs for OsPeer {
    fn seal(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
        secret::seal(secret)
    }

    fn unseal(&self, blob: &[u8]) -> Result<Vec<u8>, UnsealError> {
        secret::unseal(blob)
    }

    fn lock(&self, name: &str) -> LockAttempt {
        match account_lock::acquire(&format!("{LOCK_PREFIX}{name}")) {
            Ok(AccountLock::Held(guard)) => LockAttempt::Held(Box::new(guard)),
            Ok(AccountLock::HeldElsewhere) => LockAttempt::HeldByThisAccount,
            Err(e) => LockAttempt::NotHeld(e.to_string()),
        }
    }

    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
