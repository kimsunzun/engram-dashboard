//! The peer state — one object the rest of the peer module reads: the identity status, this PC's ID, the trusted list and the network
//!   settings — and the operations that change them (add / edit / remove a PC, set the network, regenerate the key, accept a changed list,
//!   adopt or start fresh). ★Plain synchronous calls with no gating★: who may call which operation (human vs agent, approvals) and when
//!   the link starts are the callers' — this object only refuses what its state forbids.
//!
//! Invariants:
//! - A key is created only on first need ([`PeerStore::ensure_identity`], which [`PeerStore::add`] and [`PeerStore::set_network`] call;
//!   the other list writes need a ready state and so a key already) or by an explicit human choice ([`PeerStore::regenerate`],
//!   [`PeerStore::start_fresh`], [`PeerStore::adopt`]) — never on load, never over a key that exists without a backup, never while the
//!   trusted list names PCs without a key, never while the list file cannot be read or was written by a newer Engram (that refusal wins
//!   over the adopt-or-fresh question).
//! - The list's MAC is checked whenever the key changes (load, create, regenerate, adopt); a mismatch keeps the feature off until
//!   [`PeerStore::accept_list`] re-signs the list as it is. Regenerate does not re-sign it. With no readable key the list is not checked.
//!   A list file that is corrupt as a whole keeps the feature off the same way once a key is loaded ([`PeerStatus::ListCorrupt`]):
//!   accepting starts an empty list with default settings, signed, and the corrupt file is set aside by that write. A missing file is
//!   simply an empty list.
//! - A list change is applied to a copy, saved, and kept only when the save succeeded — except removal, which narrows trust: it is
//!   applied in memory first and stays even when its save is refused.
//! - While a key is loaded the identity lock for that ID is held — an account lock shared by this account's sessions on this machine,
//!   which no other account can create or hold. Another holder means another Engram copy with this identity on this machine, and the
//!   feature stays off. A lock that fails for an OS reason is gone without, and said so ([`PeerStore::lock_note`]; network duplicate
//!   detection still applies).
//! - Nothing here logs: the secret and the MAC key never leave as text.

// ADR-0389, ADR-0291

use std::any::Any;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_base::file::{self, Claim, Refused, WritePolicy};

use super::id::PeerId;
use super::identity::{
    create_key, load_key, read_capped_bytes, replace_key, sweep_temps, KeyLoad, PeerFiles, Secret,
    Unreadable,
};
use super::mac;
use super::os::{LockAttempt, OsPeer, PeerOs};
use super::trusted::{
    body_of, check_entries, file_text, read_list, write_list, ListFile, ListRefusal,
    NetworkSettings, TrustedList, TrustedPeer, WriteError, SPEC,
};
use crate::file_hooks::OS_HOOKS;

/// The answer when a list operation finds no usable list behind a state that should have one.
const NOT_LOADED: &str = "the trusted list is not loaded";

/// The one status the rest of the peer module reads. Only [`PeerStatus::Ready`] lets the peer feature run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PeerStatus {
    /// Key loaded, list checked.
    Ready,
    /// No key for this account and machine yet — one is made on first need, without asking.
    NoIdentityYet,
    /// A key file exists but cannot be used; a new key (human-approved regenerate) re-pairs every PC.
    KeyUnreadable(Unreadable),
    /// The key could not be read for now — retried; regenerate is not offered.
    KeyTransient(String),
    /// No key for this machine, but this account has one under other machine ids — a human chooses adopt or fresh.
    AdoptOrFresh { machines: Vec<String> },
    /// Another Engram copy with this identity runs on this machine under this account.
    IdentityInUse,
    /// The list file must not be written over (newer version, failed read); writes answer `CONFLICT`.
    ListRefused(ListRefusal),
    /// The list's MAC does not match — it was changed outside Engram; a human accepts it as it is (re-sign).
    ListChangedOutside,
    /// The list file is corrupt as a whole (the reason); a human accepts starting over with an empty list and default settings.
    ListCorrupt(String),
    /// This environment cannot hold a peer identity (account or machine id unreadable, no sealing).
    Unavailable(String),
}

impl fmt::Display for PeerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready => f.write_str("ready"),
            Self::NoIdentityYet => f.write_str("this PC has no peer identity yet"),
            Self::KeyUnreadable(why) => write!(
                f,
                "key unreadable — it may come from another PC or account; a new key means re-pairing every PC ({why})"
            ),
            Self::KeyTransient(why) => write!(f, "the identity key cannot be read right now ({why})"),
            Self::AdoptOrFresh { machines } => write!(
                f,
                "this account has a peer identity from another machine id ({}) — adopt it or start fresh",
                machines.join(", ")
            ),
            Self::IdentityInUse => f.write_str(
                "this PC identity is already in use by another Engram copy on this PC",
            ),
            Self::ListRefused(why) => write!(f, "{why}"),
            Self::ListChangedOutside => f.write_str("trusted list was changed outside Engram"),
            Self::ListCorrupt(why) => write!(
                f,
                "the trusted list file is damaged ({why}); accepting starts an empty list with default network settings"
            ),
            Self::Unavailable(why) => write!(f, "peer links are unavailable here ({why})"),
        }
    }
}

/// Why an operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PeerError {
    /// The peer feature is off in this state — the status text.
    Blocked(String),
    /// The list file must not be written now (base `file` refusal) — the same answer until the file changes.
    Conflict(String),
    /// The argument breaks a rule (own ID, duplicate, bad name, over the cap).
    Invalid(String),
    /// No listed PC has that ID.
    NotFound(PeerId),
    /// The operation does not apply in this state (regenerate while the key is only transiently unreadable, accept with nothing to accept,
    ///   adopt without the question).
    NotOffered(String),
    /// A write or OS call failed.
    Io(String),
}

impl fmt::Display for PeerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked(why)
            | Self::Conflict(why)
            | Self::Invalid(why)
            | Self::NotOffered(why)
            | Self::Io(why) => f.write_str(why),
            Self::NotFound(id) => write!(f, "no trusted PC has the ID {id}"),
        }
    }
}

impl std::error::Error for PeerError {}

/// A change to one listed PC — `None` leaves that field as it is.
#[derive(Debug, Clone, Default)]
pub(crate) struct PeerEdit {
    pub(crate) name: Option<String>,
    /// `Some(None)` clears the direct address.
    pub(crate) address: Option<Option<SocketAddr>>,
}

/// What [`PeerStore::remove`] did. The PC is revoked in memory either way; `saved` says whether the file followed.
#[derive(Debug)]
pub(crate) struct Removal {
    pub(crate) removed: TrustedPeer,
    pub(crate) saved: Result<(), PeerError>,
}

/// What [`PeerStore::regenerate`] did — every other PC must remove `old` and pair with `id` again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Regenerated {
    pub(crate) old: Option<PeerId>,
    pub(crate) id: PeerId,
    /// The previous key file, kept; `None` when there was none.
    pub(crate) backup: Option<PathBuf>,
}

enum KeyState {
    Loaded(Secret),
    Missing,
    Unreadable(Unreadable),
    Transient(String),
    AdoptOrFresh(Vec<String>),
    Unavailable(String),
}

enum ListState {
    /// Not read — the files could not be named.
    NotLoaded,
    Usable {
        /// The list in memory — what the feature uses and what the next save writes.
        list: TrustedList,
        disk: Disk,
        claim: Claim,
        check: Check,
    },
    Refused(ListRefusal),
}

/// What the file held when last read or written — the MAC is checked against this, not against memory.
enum Disk {
    Missing,
    /// The reason base `file` gave.
    Corrupt(String),
    Signed {
        body: String,
        mac: [u8; 32],
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    Verified,
    ChangedOutside,
    /// The file was corrupt and a key is loaded — the empty list in memory waits for a human's accept.
    Corrupt,
    /// No key to check with.
    Unverified,
}

enum LockState {
    NotTaken,
    /// The guard is only held — dropping it releases the name.
    Held {
        _guard: Box<dyn Any + Send + Sync>,
    },
    InUse,
    WithoutLock(String),
}

/// The peer state object (module head).
pub(crate) struct PeerStore {
    files: Option<PeerFiles>,
    os: Arc<dyn PeerOs>,
    key: KeyState,
    list: ListState,
    lock: LockState,
}

impl PeerStore {
    /// Loads from the daemon state folder for the current OS account and machine.
    pub(crate) fn open(state_dir: PathBuf) -> Self {
        Self::load(
            state_dir,
            engram_dashboard_platform::account::current_account_id(),
            engram_dashboard_platform::account::machine_id(),
            Arc::new(OsPeer),
        )
    }

    /// Loads from `dir` for the given account and machine ids. Creates nothing.
    pub(crate) fn load(
        dir: PathBuf,
        account: io::Result<String>,
        machine: io::Result<String>,
        os: Arc<dyn PeerOs>,
    ) -> Self {
        let files = match (account, machine) {
            (Ok(account), Ok(machine)) => PeerFiles::new(dir, &account, &machine),
            (Err(e), _) => Err(format!("the OS account id could not be read: {e}")),
            (_, Err(e)) => Err(format!("the machine id could not be read: {e}")),
        };
        let mut store = Self {
            files: None,
            os,
            key: KeyState::Unavailable(String::new()),
            list: ListState::NotLoaded,
            lock: LockState::NotTaken,
        };
        match files {
            Ok(files) => {
                store.files = Some(files);
                store.reload();
            }
            Err(why) => store.key = KeyState::Unavailable(why),
        }
        store
    }

    pub(crate) fn status(&self) -> PeerStatus {
        match &self.key {
            KeyState::Unavailable(why) => PeerStatus::Unavailable(why.clone()),
            KeyState::Transient(why) => PeerStatus::KeyTransient(why.clone()),
            KeyState::Unreadable(why) => PeerStatus::KeyUnreadable(why.clone()),
            KeyState::AdoptOrFresh(machines) => PeerStatus::AdoptOrFresh {
                machines: machines.clone(),
            },
            KeyState::Missing => match &self.list {
                ListState::Refused(why) => PeerStatus::ListRefused(why.clone()),
                _ => PeerStatus::NoIdentityYet,
            },
            KeyState::Loaded(_) => match (&self.lock, &self.list) {
                (LockState::InUse, _) => PeerStatus::IdentityInUse,
                (_, ListState::Refused(why)) => PeerStatus::ListRefused(why.clone()),
                (
                    _,
                    ListState::Usable {
                        check: Check::ChangedOutside,
                        ..
                    },
                ) => PeerStatus::ListChangedOutside,
                (
                    _,
                    ListState::Usable {
                        check: Check::Corrupt,
                        disk: Disk::Corrupt(why),
                        ..
                    },
                ) => PeerStatus::ListCorrupt(why.clone()),
                _ => PeerStatus::Ready,
            },
        }
    }

    /// Removes the temp files a crashed write of this account's key or list left (`identity::sweep_temps`) — for the daemon to call at
    ///   start, before any write. Nothing when the files cannot be named.
    // ADR-0291 R12
    pub(crate) fn sweep_temps(&self) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
        match &self.files {
            Some(files) => sweep_temps(files),
            None => Ok(Vec::new()),
        }
    }

    /// This PC's ID, once a key is loaded.
    pub(crate) fn id(&self) -> Option<PeerId> {
        match &self.key {
            KeyState::Loaded(secret) => Some(secret.id()),
            _ => None,
        }
    }

    /// The identity secret, once a key is loaded — for building the link endpoint only.
    pub(crate) fn secret(&self) -> Option<&Secret> {
        match &self.key {
            KeyState::Loaded(secret) => Some(secret),
            _ => None,
        }
    }

    /// The listed PCs in memory — also while the list waits to be accepted (the accept question shows them). Empty when the list is
    ///   refused or not read.
    pub(crate) fn peers(&self) -> &[TrustedPeer] {
        match &self.list {
            ListState::Usable { list, .. } => &list.peers,
            _ => &[],
        }
    }

    /// The network settings in memory; `None` when the list is refused or not read.
    pub(crate) fn network(&self) -> Option<&NetworkSettings> {
        match &self.list {
            ListState::Usable { list, .. } => Some(&list.network),
            _ => None,
        }
    }

    /// Why the identity lock is not held although the key is loaded — an OS failure; the copy runs without it (diagnostics).
    pub(crate) fn lock_note(&self) -> Option<&str> {
        match &self.lock {
            LockState::WithoutLock(why) => Some(why),
            _ => None,
        }
    }

    /// Tries again what may clear by itself: reads the files again while the key is transiently unreadable or the list file could not be
    ///   read, and tries the identity lock again while another holder had it (the other copy may have ended). Returns whether it tried.
    ///   The caller owns the retry timing.
    pub(crate) fn retry(&mut self) -> bool {
        match self.status() {
            PeerStatus::KeyTransient(_) | PeerStatus::ListRefused(ListRefusal::ReadFailed(_)) => {
                self.reload();
                true
            }
            PeerStatus::IdentityInUse => {
                self.take_lock();
                true
            }
            _ => false,
        }
    }

    /// This PC's ID, creating the key if there is none yet (no prompt). Refused in any state other than a loaded key or no key yet.
    pub(crate) fn ensure_identity(&mut self) -> Result<PeerId, PeerError> {
        if let KeyState::Loaded(secret) = &self.key {
            return Ok(secret.id());
        }
        match self.status() {
            PeerStatus::NoIdentityYet => self.install_new_key(false).map(|done| done.id),
            other => Err(PeerError::Blocked(other.to_string())),
        }
    }

    /// Adds a PC. Creates the key on first need. Our own ID, a listed ID and a listed name (ignoring case) are refused.
    pub(crate) fn add(&mut self, peer: TrustedPeer) -> Result<(), PeerError> {
        let own = self.ensure_identity()?;
        self.require_ready()?;
        if peer.id == own {
            return Err(PeerError::Invalid("that is this PC's own ID".to_string()));
        }
        self.change(|list| {
            if let Some(listed) = list.peers.iter().find(|p| p.id == peer.id) {
                return Err(PeerError::Invalid(format!(
                    "that PC is already listed as \"{}\"",
                    listed.name
                )));
            }
            list.peers.push(peer);
            Ok(())
        })
    }

    /// Changes a listed PC's name and / or direct address in place.
    pub(crate) fn edit(&mut self, id: &PeerId, edit: PeerEdit) -> Result<(), PeerError> {
        self.require_ready()?;
        self.change(|list| {
            let peer = list
                .peers
                .iter_mut()
                .find(|p| p.id == *id)
                .ok_or(PeerError::NotFound(*id))?;
            if let Some(name) = edit.name {
                peer.name = name;
            }
            if let Some(address) = edit.address {
                peer.address = address;
            }
            Ok(())
        })
    }

    /// Removes a listed PC — revoked in memory first, then saved; a refused or failed save is reported in [`Removal::saved`] and the
    ///   revocation stays for this run.
    pub(crate) fn remove(&mut self, id: &PeerId) -> Result<Removal, PeerError> {
        self.require_ready()?;
        let ListState::Usable { list, .. } = &mut self.list else {
            return Err(PeerError::Blocked(NOT_LOADED.to_string()));
        };
        let position = list
            .peers
            .iter()
            .position(|p| p.id == *id)
            .ok_or(PeerError::NotFound(*id))?;
        let removed = list.peers.remove(position);
        let next = list.clone();
        let saved = self.save(next);
        Ok(Removal { removed, saved })
    }

    /// Replaces the network settings. Creates the key on first need.
    pub(crate) fn set_network(&mut self, network: NetworkSettings) -> Result<(), PeerError> {
        self.ensure_identity()?;
        self.require_ready()?;
        self.change(|list| {
            list.network = network;
            Ok(())
        })
    }

    /// Makes a new key on a human's OK: the current key file is kept as a timestamped backup, the ID changes, and the trusted list is
    ///   ★not★ re-signed — it fails its check under the new key until [`PeerStore::accept_list`]. Not offered while the key is only
    ///   transiently unreadable, while the adopt-or-fresh question is open, or where no key can exist.
    pub(crate) fn regenerate(&mut self) -> Result<Regenerated, PeerError> {
        match &self.key {
            KeyState::Loaded(_) | KeyState::Unreadable(_) => {}
            KeyState::Missing => {
                if let PeerStatus::ListRefused(why) = self.status() {
                    return Err(PeerError::Conflict(why.to_string()));
                }
            }
            KeyState::Transient(why) => {
                return Err(PeerError::NotOffered(format!(
                    "the identity key cannot be read right now ({why}); it recovers with the old ID, so a new key is not offered"
                )))
            }
            KeyState::AdoptOrFresh(_) => {
                return Err(PeerError::NotOffered(
                    "answer the adopt-or-fresh question first".to_string(),
                ))
            }
            KeyState::Unavailable(why) => return Err(PeerError::Blocked(why.clone())),
        }
        let old = self.id();
        let done = self.install_new_key(true)?;
        Ok(Regenerated { old, ..done })
    }

    /// Re-signs the list as it is now — only while it waits for that ([`PeerStatus::ListChangedOutside`], or [`PeerStatus::ListCorrupt`]
    ///   where "as it is" is the empty list with default settings; that write sets the corrupt file aside).
    pub(crate) fn accept_list(&mut self) -> Result<(), PeerError> {
        if !matches!(
            self.status(),
            PeerStatus::ListChangedOutside | PeerStatus::ListCorrupt(_)
        ) {
            return Err(PeerError::NotOffered(
                "the trusted list has no outside change to accept".to_string(),
            ));
        }
        let ListState::Usable { list, .. } = &self.list else {
            return Err(PeerError::Blocked(self.status().to_string()));
        };
        let list = list.clone();
        self.save(list)
    }

    /// Takes this account's key (and list, if any) from `machine` as this machine's — only while the adopt-or-fresh question is open.
    ///   The other machine's files stay as they are. A list file already here is written over only under base `file`'s rules: one this run
    ///   may not overwrite (newer version, failed read) refuses the adopt before anything is written (`Conflict`); any other is set aside
    ///   as `.corrupt` first. Then loads again: a key this account cannot open becomes unreadable (the X5 flow).
    pub(crate) fn adopt(&mut self, machine: &str) -> Result<(), PeerError> {
        let KeyState::AdoptOrFresh(machines) = &self.key else {
            return Err(PeerError::NotOffered(
                "there is no identity from another machine id to adopt".to_string(),
            ));
        };
        if !machines.iter().any(|m| m == machine) {
            return Err(PeerError::Invalid(format!(
                "no identity of this account under the machine id {machine}"
            )));
        }
        let files = self.files()?.clone();
        let theirs = files.list_path_for(machine);
        let ours = files.list_path();
        let take_list = !matches!(
            std::fs::symlink_metadata(&theirs),
            Err(e) if e.kind() == io::ErrorKind::NotFound
        );
        // ADR-0291 R2 · R8: decided before anything is written; `NotYet` — whatever is here was never this run's to overwrite.
        let set_aside =
            match file::load(&ours, &SPEC, OS_HOOKS).write_policy(Claim::NotYet) {
                _ if !take_list => false,
                WritePolicy::Write => false,
                WritePolicy::CopyAsideFirst => true,
                WritePolicy::Refuse(Refused::Newer { found }) => {
                    return Err(PeerError::Conflict(
                        ListRefusal::Newer { found }.to_string(),
                    ))
                }
                WritePolicy::Refuse(Refused::ReadFailed) => return Err(PeerError::Conflict(
                    "this machine's trusted list file could not be read, so it is not written over"
                        .to_string(),
                )),
            };
        let blob = read_capped_bytes(&files.key_path_for(machine)).map_err(|e| {
            PeerError::Io(format!("the other machine's key could not be read: {e}"))
        })?;
        create_key(&files.key_path(), &blob)
            .map_err(|e| PeerError::Io(format!("the key could not be written: {e}")))?;
        let copied = if take_list {
            let aside = if set_aside {
                file::copy_aside(&ours, OS_HOOKS).map(drop)
            } else {
                Ok(())
            };
            aside
                .and_then(|()| file::copy_atomic(&theirs, &ours, OS_HOOKS))
                .map_err(|e| {
                    PeerError::Io(format!(
                        "the key was adopted but its trusted list could not be copied: {e}"
                    ))
                })
        } else {
            Ok(())
        };
        self.reload();
        copied
    }

    /// Makes a new key for this machine instead of adopting — only while the adopt-or-fresh question is open. The other machine's files
    ///   stay as they are.
    pub(crate) fn start_fresh(&mut self) -> Result<PeerId, PeerError> {
        if !matches!(self.key, KeyState::AdoptOrFresh(_)) {
            return Err(PeerError::NotOffered(
                "there is no adopt-or-fresh question open".to_string(),
            ));
        }
        self.install_new_key(false).map(|done| done.id)
    }

    // ── internals ──

    fn files(&self) -> Result<&PeerFiles, PeerError> {
        self.files
            .as_ref()
            .ok_or_else(|| PeerError::Blocked(self.status().to_string()))
    }

    fn require_ready(&self) -> Result<(), PeerError> {
        match self.status() {
            PeerStatus::Ready => Ok(()),
            // ADR-0291: a list file that must not be written over answers CONFLICT.
            PeerStatus::ListRefused(why) => Err(PeerError::Conflict(why.to_string())),
            other => Err(PeerError::Blocked(other.to_string())),
        }
    }

    fn reload(&mut self) {
        let Some(files) = &self.files else { return };
        let list_file = read_list(&files.list_path());
        let key = match load_key(&files.key_path(), &*self.os) {
            KeyLoad::Loaded(secret) => KeyState::Loaded(secret),
            KeyLoad::Missing if list_file.names_peers() => {
                KeyState::Unreadable(Unreadable::MissingUnderList)
            }
            // The list file is unknown or not ours to write: the status says so, and nothing may make or adopt a key meanwhile.
            KeyLoad::Missing if matches!(list_file, ListFile::Refused(_)) => KeyState::Missing,
            KeyLoad::Missing => match files.other_machines() {
                Ok(machines) if machines.is_empty() => KeyState::Missing,
                Ok(machines) => KeyState::AdoptOrFresh(machines),
                Err(e) => KeyState::Transient(format!("the state folder could not be listed: {e}")),
            },
            KeyLoad::Unreadable(why) => KeyState::Unreadable(why),
            KeyLoad::Transient(why) => KeyState::Transient(why),
            KeyLoad::Unavailable(why) => KeyState::Unavailable(why),
        };
        self.key = key;
        self.list = match list_file {
            ListFile::Missing => usable(TrustedList::default(), Disk::Missing, Claim::NotYet),
            // ADR-0291 R8: a corrupt file starts as the default and is set aside before the first write.
            ListFile::Corrupt(why) => {
                usable(TrustedList::default(), Disk::Corrupt(why), Claim::NotYet)
            }
            ListFile::Signed { list, body, mac } => {
                usable(list, Disk::Signed { body, mac }, Claim::Adopted)
            }
            ListFile::Refused(why) => ListState::Refused(why),
        };
        self.after_key_change();
    }

    /// Checks the list against the current key and (re)takes the identity lock for the current ID.
    fn after_key_change(&mut self) {
        let secret = match &self.key {
            KeyState::Loaded(secret) => Some(secret),
            _ => None,
        };
        if let ListState::Usable { disk, check, .. } = &mut self.list {
            *check = match (disk, secret) {
                (Disk::Missing, _) => Check::Verified,
                // A corrupt file may have held pairings and settings nobody can read now — a human decides to start over.
                (Disk::Corrupt(_), Some(_)) => Check::Corrupt,
                (Disk::Corrupt(_), None) => Check::Unverified,
                (Disk::Signed { .. }, None) => Check::Unverified,
                (Disk::Signed { body, mac }, Some(secret)) => {
                    match mac::verify(secret, body, mac) {
                        Ok(true) => Check::Verified,
                        Ok(false) | Err(_) => Check::ChangedOutside,
                    }
                }
            };
        }
        self.take_lock();
    }

    /// (Re)takes the identity lock for the current ID; none without a loaded key.
    // ADR-0401: held elsewhere → identity in use (never "runs without the lock"); only an OS failure runs without it.
    fn take_lock(&mut self) {
        // The old guard goes first: a regenerate must not keep the previous identity's name.
        self.lock = LockState::NotTaken;
        if let KeyState::Loaded(secret) = &self.key {
            // The lock is the account's own (`PeerOs::lock`), so the ID alone names it.
            self.lock = match self.os.lock(&secret.id().to_text()) {
                LockAttempt::Held(guard) => LockState::Held { _guard: guard },
                LockAttempt::HeldByThisAccount => LockState::InUse,
                LockAttempt::NotHeld(why) => LockState::WithoutLock(why),
            };
        }
    }

    /// A new random key sealed and written — created (`replace = false`, never over a file) or swapped in behind a backup.
    fn install_new_key(&mut self, replace: bool) -> Result<Regenerated, PeerError> {
        let path = self.files()?.key_path();
        let secret = Secret::generate()
            .map_err(|e| PeerError::Io(format!("no random key could be drawn: {e}")))?;
        let blob = match self.os.seal(secret.expose()) {
            Ok(blob) => blob,
            Err(e) if e.kind() == io::ErrorKind::Unsupported => {
                let why = "this OS cannot seal the identity key".to_string();
                self.key = KeyState::Unavailable(why.clone());
                return Err(PeerError::Blocked(why));
            }
            Err(e) => {
                return Err(PeerError::Io(format!(
                    "the new key could not be sealed: {e}"
                )))
            }
        };
        let backup = if replace {
            replace_key(&path, &blob, self.os.now())
        } else {
            create_key(&path, &blob).map(|()| None)
        }
        .map_err(|e| PeerError::Io(format!("the key file could not be written: {e}")))?;
        let id = secret.id();
        self.key = KeyState::Loaded(secret);
        self.after_key_change();
        Ok(Regenerated {
            old: None,
            id,
            backup,
        })
    }

    /// Applies `edit` to a copy of the list, checks the entry rules (and changed network settings against the link adapter), saves, and
    ///   keeps the copy only if the save succeeded.
    fn change(
        &mut self,
        edit: impl FnOnce(&mut TrustedList) -> Result<(), PeerError>,
    ) -> Result<(), PeerError> {
        let ListState::Usable { list, .. } = &self.list else {
            return Err(PeerError::Blocked(self.status().to_string()));
        };
        let mut next = list.clone();
        edit(&mut next)?;
        check_entries(&next).map_err(PeerError::Invalid)?;
        // The adapter's parse runs only when the network settings change, and then re-checks every helper URL: a stored setting it refuses does not block other list changes, but a network change must leave settings that pass.
        if next.network != list.network {
            next.network
                .check_for_endpoint()
                .map_err(PeerError::Invalid)?;
        }
        self.save(next)
    }

    /// Signs and writes `list`; on success it becomes the list in memory and on disk.
    fn save(&mut self, list: TrustedList) -> Result<(), PeerError> {
        let (KeyState::Loaded(secret), ListState::Usable { claim, .. }) = (&self.key, &self.list)
        else {
            return Err(PeerError::Blocked(self.status().to_string()));
        };
        let path = self.files()?.list_path();
        let body = body_of(&list).map_err(PeerError::Io)?;
        let mac = mac::sign(secret, &body)
            .map_err(|_| PeerError::Io("the trusted list could not be signed".to_string()))?;
        let text = file_text(&body, &mac).map_err(PeerError::Io)?;
        write_list(&path, &text, *claim).map_err(|e| match e {
            WriteError::Refused(why) => PeerError::Conflict(why.to_string()),
            WriteError::TooLarge(why) => PeerError::Invalid(why),
            WriteError::Io(e) => {
                PeerError::Io(format!("the trusted list file could not be written: {e}"))
            }
        })?;
        self.list = ListState::Usable {
            list,
            disk: Disk::Signed { body, mac },
            claim: Claim::Adopted,
            check: Check::Verified,
        };
        Ok(())
    }
}

fn usable(list: TrustedList, disk: Disk, claim: Claim) -> ListState {
    ListState::Usable {
        list,
        disk,
        claim,
        // Set by `after_key_change`, which every load runs.
        check: Check::Unverified,
    }
}

#[cfg(test)]
mod tests;
