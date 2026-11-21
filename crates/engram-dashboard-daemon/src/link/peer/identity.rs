//! The identity key on disk — `peer-key.<account>.<machine>` in the daemon state folder: the 32-byte secret sealed for the current OS
//!   account ([`PeerOs::seal`]), the blob stored as is. This file names the per-account-and-machine files ([`PeerFiles`]), reads and
//!   classifies the key ([`load_key`]), creates it without ever replacing one ([`create_key`]) and replaces it only behind a backup
//!   ([`replace_key`] — the human-approved regenerate). ★Nothing here decides when a key is made★ — that is the state's ([`super::store`]).
//!   Every write goes through base `file` (temp file → sync → rename) with platform's no-replace rename where a file must not be replaced,
//!   so the temp names are base's and [`sweep_temps`] can clear what a crash left.
//!
//! Files of other names in the folder are never touched; this account's files under another machine id are only listed
//!   ([`PeerFiles::other_machines`]), never adopted here.

// ADR-0389

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use engram_dashboard_base::file;
use engram_dashboard_platform::fs::rename_no_replace;
use engram_dashboard_platform::process::pid_alive;
use engram_dashboard_platform::secret::UnsealError;

use super::id::PeerId;
use super::os::PeerOs;
use crate::file_hooks::OS_HOOKS;

/// Read cap of the key file — a DPAPI blob of a 32-byte secret is a few hundred bytes; anything far larger is not ours.
const KEY_FILE_CAP: u64 = 64 << 10;
/// Longest account or machine id put in a file name.
const MAX_ID_IN_NAME: usize = 128;
/// Backup names tried for one regenerate before giving up (`…bak-<time>`, then `-2` …).
const MAX_BACKUP_TRIES: u32 = 100;

const KEY_PREFIX: &str = "peer-key.";
const LIST_PREFIX: &str = "peers.";
const LIST_SUFFIX: &str = ".json";
const BACKUP_MARK: &str = ".bak-";

/// The identity secret. Zeroed on drop; never printed.
pub(crate) struct Secret([u8; 32]);

impl Secret {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// A new random secret from the OS generator — any 32 bytes are a valid key.
    pub(crate) fn generate() -> io::Result<Self> {
        let mut bytes = [0u8; 32];
        // getrandom 0.2's error is not a `std::error::Error` here — carried as text.
        getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self(bytes))
    }

    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.0
    }

    /// This PC's ID — the key's public half, derived by the link adapter (the key scheme is the adapter's; the ID text is ours).
    pub(crate) fn id(&self) -> PeerId {
        PeerId::from_bytes(engram_dashboard_transport::iroh::public_key(&self.0))
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Zeroes `bytes` with volatile writes, so the wipe of secret material is not optimised away.
fn wipe(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: a valid, aligned `u8` inside the slice.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// The per-account-and-machine file names in one state folder.
#[derive(Debug, Clone)]
pub(crate) struct PeerFiles {
    dir: PathBuf,
    account: String,
    machine: String,
}

impl PeerFiles {
    /// `account` = the OS account id (Windows SID), `machine` = the machine id (Windows `MachineGuid`) — both go into file names, so
    ///   only ASCII letters, digits and `-` are taken (what a SID and a GUID are made of).
    pub(crate) fn new(dir: PathBuf, account: &str, machine: &str) -> Result<Self, String> {
        for (what, id) in [("account", account), ("machine", machine)] {
            if !is_name_safe(id) {
                return Err(format!(
                    "the {what} id cannot be part of a file name (letters, digits and '-' only, 1 to {MAX_ID_IN_NAME} characters)"
                ));
            }
        }
        Ok(Self {
            dir,
            account: account.to_string(),
            machine: machine.to_string(),
        })
    }

    pub(crate) fn key_path(&self) -> PathBuf {
        self.key_path_for(&self.machine)
    }

    pub(crate) fn list_path(&self) -> PathBuf {
        self.list_path_for(&self.machine)
    }

    pub(crate) fn key_path_for(&self, machine: &str) -> PathBuf {
        self.dir
            .join(format!("{KEY_PREFIX}{}.{machine}", self.account))
    }

    pub(crate) fn list_path_for(&self, machine: &str) -> PathBuf {
        self.dir.join(format!(
            "{LIST_PREFIX}{}.{machine}{LIST_SUFFIX}",
            self.account
        ))
    }

    /// The machine ids under which this account has a key file here, other than this machine — sorted. Only exact key names count
    ///   (backups, temp files and other accounts' files do not); a list without its key has nothing to adopt. A missing folder = none.
    pub(crate) fn other_machines(&self) -> io::Result<Vec<String>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let prefix = format!("{KEY_PREFIX}{}.", self.account);
        let mut machines = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            let Some(name) = name.to_str() else { continue };
            if let Some(machine) = name.strip_prefix(&prefix) {
                if machine != self.machine && is_name_safe(machine) {
                    machines.push(machine.to_string());
                }
            }
        }
        machines.sort();
        Ok(machines)
    }
}

fn is_name_safe(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_IN_NAME
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// What reading the key file found.
#[derive(Debug)]
pub(crate) enum KeyLoad {
    Loaded(Secret),
    /// No key file for this account and machine.
    Missing,
    /// The file is there but is not a key this account can use.
    Unreadable(Unreadable),
    /// The file could not be read or opened now — the same file may work later.
    Transient(String),
    /// This OS cannot seal a key.
    Unavailable(String),
}

/// Why a key is unreadable — every cause means "a new key re-pairs every PC", and none is regenerated without a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unreadable {
    /// The OS refused to open the blob as this account's (another account or PC, a forced password reset, a reinstall).
    NotThisAccount(String),
    /// The file is not a sealed 32-byte key.
    Corrupt(String),
    /// No key file while the trusted list names PCs — never recreated silently.
    MissingUnderList,
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotThisAccount(e) => write!(f, "this account cannot open it ({e})"),
            Self::Corrupt(e) => write!(f, "the key file is damaged ({e})"),
            Self::MissingUnderList => {
                f.write_str("the key file is missing while the trusted list names PCs")
            }
        }
    }
}

/// Reads and opens the key at `path`.
pub(crate) fn load_key(path: &Path, os: &dyn PeerOs) -> KeyLoad {
    let blob = match read_capped_bytes(path) {
        Ok(blob) => blob,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return KeyLoad::Missing,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return KeyLoad::Unreadable(Unreadable::Corrupt(e.to_string()))
        }
        Err(e) => return KeyLoad::Transient(format!("the key file could not be read: {e}")),
    };
    if blob.is_empty() {
        return KeyLoad::Unreadable(Unreadable::Corrupt("the file is empty".to_string()));
    }
    match os.unseal(&blob) {
        // The plaintext copy is the caller's to wipe (platform `secret::unseal`), on every arm.
        Ok(mut plain) => {
            let loaded = match <[u8; 32]>::try_from(plain.as_slice()) {
                Ok(bytes) => KeyLoad::Loaded(Secret::from_bytes(bytes)),
                Err(_) => KeyLoad::Unreadable(Unreadable::Corrupt(format!(
                    "it opened to {} bytes, not 32",
                    plain.len()
                ))),
            };
            wipe(&mut plain);
            loaded
        }
        Err(UnsealError::Unreadable(e)) => {
            KeyLoad::Unreadable(Unreadable::NotThisAccount(e.to_string()))
        }
        Err(UnsealError::Transient(e)) => {
            KeyLoad::Transient(format!("the key could not be opened now: {e}"))
        }
        Err(UnsealError::Unsupported) => {
            KeyLoad::Unavailable("this OS cannot seal the identity key".to_string())
        }
    }
}

/// The file's bytes up to [`KEY_FILE_CAP`] — over it = `InvalidData`. A briefly held file is retried (`OS_HOOKS`).
pub(crate) fn read_capped_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let mut out = None;
    (OS_HOOKS.retry)(&mut || {
        let mut buf = Vec::new();
        std::fs::File::open(path)?
            .take(KEY_FILE_CAP + 1)
            .read_to_end(&mut buf)?;
        out = Some(buf);
        Ok(())
    })?;
    let buf = out.ok_or_else(|| io::Error::other("the retry never called the attempt"))?;
    if buf.len() as u64 > KEY_FILE_CAP {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("exceeded the {KEY_FILE_CAP} byte cap"),
        ));
    }
    Ok(buf)
}

/// Writes `blob` as a new file at `path` (base `file::write_atomic_new` with platform's `fs::rename_no_replace`): ★never replaces★ —
///   anything already at `path` fails it with `AlreadyExists`; dying midway leaves no half key, only a base temp file. Creates the folder.
pub(crate) fn create_key(path: &Path, blob: &[u8]) -> io::Result<()> {
    file::write_atomic_new(path, blob, OS_HOOKS, rename_no_replace)
}

/// Removes the temp files a crashed write of this account's key or list left in the state folder (base `file::sweep_temps`: this process's
///   and dead processes' only). For the daemon to call at start, before the first key or list write. Regenerate backups' temp files are
///   not covered (their names carry a time).
// ADR-0291 R12
pub(crate) fn sweep_temps(files: &PeerFiles) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
    let names: Vec<String> = [files.key_path(), files.list_path()]
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
        .collect();
    let targets: Vec<&str> = names.iter().map(String::as_str).collect();
    file::sweep_temps(&files.dir, &targets, pid_alive)
}

/// The human-approved regenerate's write: copies the current key file to `<name>.bak-<UTC time>` (copy without replace — ★an earlier
///   backup is never overwritten★: a taken name moves on to `-2`, `-3` …), then swaps `blob` in by an atomic replace, so a key file is in
///   place at every moment. Returns the backup's path. If there
///   is no current file, nothing is backed up and the key is created ([`create_key`]).
pub(crate) fn replace_key(
    path: &Path,
    blob: &[u8],
    now: SystemTime,
) -> io::Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return create_key(path, blob).map(|()| None)
        }
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let backup = back_up(path, now)?;
    file::write_atomic(path, blob, OS_HOOKS)?;
    Ok(Some(backup))
}

fn back_up(path: &Path, now: SystemTime) -> io::Result<PathBuf> {
    let stamp = utc_stamp(now);
    for n in 1..=MAX_BACKUP_TRIES {
        let suffix = if n == 1 {
            format!("{BACKUP_MARK}{stamp}")
        } else {
            format!("{BACKUP_MARK}{stamp}-{n}")
        };
        let backup = sibling(path, &suffix)?;
        match file::copy_atomic_new(path, &backup, OS_HOOKS, rename_no_replace) {
            Ok(()) => return Ok(backup),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{MAX_BACKUP_TRIES} backup names for this time are already taken"),
    ))
}

fn sibling(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut name = name.to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

/// `YYYYMMDDTHHMMSSZ` in UTC. A clock before 1970 reads as 1970.
fn utc_stamp(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 → (year, month, day) in the proleptic Gregorian calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_utc_stamp_is_the_calendar_time() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101T000000Z");
        // 2026-11-21 10:30:05 UTC.
        let t = UNIX_EPOCH + Duration::from_secs(1_795_257_005);
        assert_eq!(utc_stamp(t), "20261121T103005Z");
        // A leap day.
        let t = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(utc_stamp(t), "20000229T000000Z");
    }

    #[test]
    fn ids_that_cannot_be_file_name_parts_are_refused() {
        let dir = PathBuf::from("x");
        assert!(PeerFiles::new(dir.clone(), "S-1-5-21-1-2-3-1001", "0a1b-2c3d").is_ok());
        for bad in ["", "a.b", "a/b", "a\\b", "a b", "é"] {
            assert!(PeerFiles::new(dir.clone(), bad, "m").is_err(), "{bad:?}");
            assert!(PeerFiles::new(dir.clone(), "a", bad).is_err(), "{bad:?}");
        }
        assert!(PeerFiles::new(dir, &"a".repeat(MAX_ID_IN_NAME + 1), "m").is_err());
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("engram-peer-identity-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn creating_a_key_never_replaces_a_file_and_leaves_no_temp_behind() {
        let dir = TempDir::new();
        let path = dir.0.join("nested").join("peer-key.a.m");
        create_key(&path, b"first").unwrap();
        let again = create_key(&path, b"second").unwrap_err();
        assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["peer-key.a.m"]);
    }

    #[test]
    fn the_sweep_clears_leftover_temps_of_this_accounts_files_only() {
        let dir = TempDir::new();
        let files = PeerFiles::new(dir.0.clone(), "S-1-5-21-9", "m-1").unwrap();
        let me = std::process::id();
        let ours = [
            format!("peer-key.S-1-5-21-9.m-1.tmp{me}.7"),
            format!("peers.S-1-5-21-9.m-1.json.tmp{me}.3"),
        ];
        let others = [
            format!("peer-key.S-1-5-21-8.m-1.tmp{me}.1"),
            "agents.json".to_string(),
            "peer-key.S-1-5-21-9.m-1".to_string(),
        ];
        for name in ours.iter().chain(&others) {
            std::fs::write(dir.0.join(name), b"x").unwrap();
        }
        let swept = sweep_temps(&files).unwrap();
        assert_eq!(swept.len(), ours.len());
        for name in &ours {
            assert!(!dir.0.join(name).exists(), "{name}");
        }
        for name in &others {
            assert!(dir.0.join(name).exists(), "{name}");
        }
    }

    #[test]
    fn replacing_without_a_current_file_creates_it_without_a_backup() {
        let dir = TempDir::new();
        let path = dir.0.join("peer-key.a.m");
        assert_eq!(replace_key(&path, b"new", UNIX_EPOCH).unwrap(), None);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn a_taken_backup_name_moves_on_and_keeps_the_earlier_backup() {
        let dir = TempDir::new();
        let path = dir.0.join("peer-key.a.m");
        let taken = dir.0.join("peer-key.a.m.bak-19700101T000000Z");
        std::fs::write(&path, b"old").unwrap();
        std::fs::write(&taken, b"earlier").unwrap();
        let backup = replace_key(&path, b"new", UNIX_EPOCH).unwrap().unwrap();
        assert_eq!(backup, dir.0.join("peer-key.a.m.bak-19700101T000000Z-2"));
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
        assert_eq!(std::fs::read(&taken).unwrap(), b"earlier");
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    /// An IO failure reading the key is transient, never "unreadable" — a folder in the file's place stands in for one.
    #[test]
    fn a_key_file_that_cannot_be_read_is_transient() {
        struct NoUnseal;
        impl PeerOs for NoUnseal {
            fn seal(&self, _: &[u8]) -> io::Result<Vec<u8>> {
                unreachable!()
            }
            fn unseal(&self, _: &[u8]) -> Result<Vec<u8>, UnsealError> {
                unreachable!("an unreadable file must not reach unseal")
            }
            fn lock(&self, _: &str) -> super::super::os::LockAttempt {
                unreachable!()
            }
            fn now(&self) -> SystemTime {
                UNIX_EPOCH
            }
        }
        let dir = TempDir::new();
        let path = dir.0.join("peer-key.a.m");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(load_key(&path, &NoUnseal), KeyLoad::Transient(_)));
        assert!(matches!(
            load_key(&dir.0.join("absent"), &NoUnseal),
            KeyLoad::Missing
        ));
    }

    #[test]
    fn the_file_names_carry_account_and_machine() {
        let files = PeerFiles::new(PathBuf::from("d"), "S-1-5-21-9", "m-1").unwrap();
        assert_eq!(
            files.key_path(),
            PathBuf::from("d").join("peer-key.S-1-5-21-9.m-1")
        );
        assert_eq!(
            files.list_path(),
            PathBuf::from("d").join("peers.S-1-5-21-9.m-1.json")
        );
    }
}
