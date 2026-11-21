//! Single-instance guard — holds the data folder's `daemon.json` with a restricted share mode (ADR-0135).
//!
//! Only one daemon per data folder may run. The decision is obtaining a read+write handle to that folder's connection file — if obtained, the folder is ours; if not, someone else holds it or a third party is interfering.
//!
//! ★The file we hold = the file the client reads (ADR-0135 decision 1)★: with a separate lock file, deleting the data folder while the daemon runs would leave only the undeletable lock file and lose the address — the client can neither connect nor shut it down, and a new daemon is blocked by the lock. ★Do not split it into two files again.★
//!
//! ## Share mode = allow reads only (`FILE_SHARE_READ` = 1 — platform's `fs::open_deny_write`)
//!
//! Measured:
//! - A second daemon's read+write open → `ERROR_SHARING_VIOLATION` (32). This is all of the exclusion: arbitration applies to the requested access, so whichever of 1·3·7 the other side holds the share with, and whether it opens write-only or append, all are refused.
//! - Deleting or renaming this file → 32. Renaming the data folder → `ERROR_ACCESS_DENIED` (5).
//! - All 8 alias kinds (uppercase · `..` · `\\?\` · 8.3 short name · junction · hard link · `subst` · mapped drive vs UNC) converge on the same file object, including 2 real SMB shares and the OneDrive sync root.
//!
//! ★The reading side must not narrow the share★: the access we hold is read+write, so a client open that drops write from the share (e.g. .NET `FileStream` default `FileShare.Read` · `[System.IO.File]::ReadAllText`) is refused with 32 even when read-only. Today's consumers (Rust `std::fs::read`, Node `fs.readFileSync`) default to permissive. Check this line when attaching a new consumer.
//!
//! ★Do not add `FILE_SHARE_WRITE` (do not revive it)★: it allows others' write opens, so with share 3 a second daemon's read+write open succeeds (measured) and two daemons run in one folder.
//!
//! ★Do not add `FILE_SHARE_DELETE` either (do not revive it)★: someone could clear this file and a second daemon would create and grab a new file at the same path.
//!
//! ★Do not revive range locking (`try_lock`)★: an exclusive range lock denies other processes' reads, and std's API locks the whole file. If the client cannot read this file, merging it was pointless.
//!
//! ★On non-Windows, [`acquire`] refuses outright (`Unsupported`)★: `open_deny_write` does not open outside Windows and no substitute exists — the daemon itself is Windows-only (WMI spawn · Job Object · taskkill). ★Do not return a "guard that guarantees nothing" (do not revive it)★: both daemons would succeed in one folder and the later one would overwrite the earlier one's endpoint. That is why the whole test module asserting share semantics is `#[cfg(windows)]`.
//!
//! ★Tell a third party's restricted open apart from a real duplicate (ADR-0135 §Impact)★: antivirus · indexers · backup opening briefly with a narrow share fail our open with the same 32. So it retries briefly, then opens the file read-only and checks for a live daemon's record — visible = duplicate (normal yielding); not visible = [`AcquireError::FileBusy`]. ★This read is a diagnosis, not an identity decision★: identity is only "did we hold the handle"; using the written value as identity would make two copies of a portable distribution folder claim the same identity.
//!
//! ★Do not identify the folder by name or path (do not revive it)★: naming a lock needs our own path normalization, and missing one of the case · `..` · 8.3 · junction · subst · mapped-drive spellings puts two daemons in one folder. Holding the file leaves spelling resolution to the OS.
//!
//! ★Copying while it runs — a residual★: a plain `copy` succeeds, but a copy tool that opens the source allowing read sharing only can conflict with our write access and fail with 32.
//!
//! ★The reach goes beyond one computer — only when the filesystem does its part★: SMB redirectors propagate ShareAccess and the server arbitrates, so a daemon on another computer is excluded too (measured). But on third-party redirectors · user-mode filesystems (WebDAV · Windows NFS client · Dokan/WinFsp-based rclone·sshfs-win · Google Drive virtual drive) the implementer may ignore it and both daemons acquire. With `try_lock` gone there is no second line of defense — do not put the data folder in such places.
//!
//! ★The shell token is not in the held file (ADR-0395)★: `daemon.json` keeps its inherited ACL so another account can read who holds the
//!   folder and take over after a crash; the token goes to this account's owner-only `daemon-token.<account>`, replaced under the held
//!   lock before the record is published ([`InstanceGuard::write_token_file`]).
//!
//! ★Lifetime rule★: the acquired file must be held as long as the daemon process lives; dropping the guard closes the file and breaks singleness, so `run()` keeps the guard until the process ends. On abnormal exit the OS closes the handle but not immediately (release depends on system resources; on SMB the server keeps the open until session · durable-handle timers expire) — do not assume "it can restart immediately after it dies".

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use engram_dashboard_platform::fs::is_access_denied;
use engram_dashboard_platform::fs::is_sharing_violation;
use engram_dashboard_platform::fs::open_deny_write;

use crate::portfile::{self, DaemonInfo, DAEMON_FILE};

/// Open retries to get past a third party holding a restricted share (including the first attempt).
const OPEN_ATTEMPTS: u32 = 5;

/// Wait between open retries.
///
/// ★The worst-case delay [`acquire`] adds = `(OPEN_ATTEMPTS - 1) × OPEN_RETRY_DELAY` = about 400ms★ (no sleep after the last attempt). The normal path is 0ms, and a duplicate daemon also uses up this budget — duplicates and third-party interference fail with the same 32, so they are told apart only by the diagnosis after the retries. That delay lands only on the daemon that backs off. Before enlarging it, compare with the client ensure polling limit (5 seconds).
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Proof of owning this folder. The file handle stays open while alive; Drop releases it.
pub struct InstanceGuard {
    file: File,
}

impl InstanceGuard {
    /// Writes the connection record in place through the held handle (ADR-0135 decision 1).
    ///
    /// ★Do not change it to a temp file + rename★: we hold the file with delete sharing closed, so rename is refused (32). Because the write is in place, a client can see partially written content — the reading side must treat a parse failure as "not ready yet" and keep polling.
    ///
    /// ★The type guarantees that writing comes after acquisition★: only a guard can call this; reversed order would mix two daemons' writes.
    // ADR-0135
    pub fn publish(&mut self, info: &DaemonInfo) -> io::Result<()> {
        portfile::write_in_place(&mut self.file, info)
    }

    /// Replaces this account's shell-token file `path` with a new one holding `token`: the previous file is deleted, a new one created
    ///   `CREATE_NEW` with the owner-only DACL passed at create (platform `owner_only`), then the token written and synced. Both steps
    ///   retry briefly on a held file (antivirus), like base's rename retries. `Err` = start refused: the previous file could not be
    ///   deleted (e.g. another account owns it), or the new one could not be made owner-only (a volume without file ACLs — ADR-0399).
    ///
    /// ★Order is the contract★: call it before [`Self::publish`] — a reader takes `daemon.json` first and the token after, so a token
    ///   written first is never older than the record it pairs with. ★Under the held lock★ — only a guard can call it, so two daemons of one
    ///   folder never write the file at once. The previous file is deleted every start (a new token every run), whatever it is; one that was
    ///   not owner-only or not this account's is reported (tampering or a pre-S37 folder).
    // ADR-0395
    pub fn write_token_file(&self, path: &Path, token: &str) -> io::Result<()> {
        write_token_file_with(path, token, &OsTokenFiles)
    }
}

/// The OS calls [`InstanceGuard::write_token_file`] makes — a seam so the tests drive a foreign-owned or undeletable file.
trait TokenFiles {
    fn is_owner_only(&self, path: &Path) -> io::Result<bool>;
    fn remove(&self, path: &Path) -> io::Result<()>;
    fn create_owner_only(&self, path: &Path) -> io::Result<File>;
}

struct OsTokenFiles;

impl TokenFiles for OsTokenFiles {
    fn is_owner_only(&self, path: &Path) -> io::Result<bool> {
        engram_dashboard_platform::owner_only::is_owner_only(path)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
    fn create_owner_only(&self, path: &Path) -> io::Result<File> {
        engram_dashboard_platform::owner_only::create_owner_only(path)
    }
}

fn write_token_file_with(path: &Path, token: &str, os: &dyn TokenFiles) -> io::Result<()> {
    let retry = crate::file_hooks::OS_HOOKS.retry;
    match os.is_owner_only(path) {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            path = %path.display(),
            "the previous shell-token file was not owner-only or not this account's — replacing it"
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        // Only the report above depends on it — the file is replaced either way.
        Err(e) => tracing::debug!(
            path = %path.display(),
            "the previous shell-token file's security could not be read — replacing it: {e}"
        ),
    }
    match retry(&mut || os.remove(path)) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(io::Error::new(
                e.kind(),
                format!("the previous shell-token file could not be deleted: {e}"),
            ))
        }
    }
    let mut created = None;
    retry(&mut || {
        created = Some(os.create_owner_only(path)?);
        Ok(())
    })
    .map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("the shell-token file could not be created owner-only: {e}"),
        )
    })?;
    let mut file = created.ok_or_else(|| io::Error::other("the retry never called the create"))?;
    file.write_all(token.as_bytes())?;
    file.sync_all()
}

/// The two normal outcomes of [`acquire`]. Neither means "the system is broken".
pub enum Acquired {
    /// We are the only daemon for this folder.
    Held(InstanceGuard),
    /// Another daemon already holds this folder — backing off is the normal behavior.
    ///
    /// `pid` is the value that daemon left in the file (diagnosis only — not the ownership basis).
    AlreadyRunning { pid: u32 },
}

/// ★Reported separately from duplicates★: lumped into one error, the user cannot tell why the daemon did not start (a duplicate is normal; the two below need action).
#[derive(Debug)]
pub enum AcquireError {
    /// A third party has the connection file open with a restricted share and we could not open it within the retry budget. **Not a duplicate daemon.**
    FileBusy { attempts: u32, source: io::Error },
    /// The file exists but write access itself was denied (ACL etc.). Waiting does not clear it.
    ///
    /// ★Kept separate★: this state persists across restarts · reboots, so the user's action is the opposite of "held briefly". The read-only attribute is cleared by [`acquire`] itself, so it does not land here.
    AccessDenied { path: String, source: io::Error },
    /// This platform has no single-instance exclusion mechanism (module header — Windows only).
    Unsupported,
    /// Any other system error (missing folder, etc.).
    Io(io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileBusy { attempts, source } => write!(
                f,
                "{DAEMON_FILE} is held by another program (gave up after {attempts} retries): {source}"
            ),
            Self::AccessDenied { path, source } => write!(
                f,
                "no permission to write {path} (tried to clear the read-only attribute automatically but failed — check the folder permissions or extract to a writable location): {source}"
            ),
            Self::Unsupported => write!(
                f,
                "this platform has no single-instance exclusion mechanism (Windows only)"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AcquireError {}

/// When the open stays blocked, look only at why — the pid if there is a live daemon's record.
///
/// ★A diagnosis, not an identity decision★: ownership is already decided as "could not get the handle"; this value only chooses the message. The read opens with a permissive share (std default), so even the file we failed to open can be read.
///
/// ★Residual 1 — if the earlier daemon is not yet emitting, a duplicate is misdiagnosed as interference★: it holds the file but has not written the record. That window spans bind + MCP server startup + the whole wiring (steps 3~8 of `run()`); measured 16~19ms warm, so the 400ms budget has a 20x margin, but it can be exceeded on a cold start or a network data folder. The misdiagnosis only changes log wording and exit code (no consumer — the client polls the file), so no separate wait budget was added.
///
/// ★Residual 2 — the opposite misdiagnosis cannot be prevented★: if a third party holds it but the old record's pid happens to be alive (a copied record, or pid + creation time coincidence), it reads as "already running" and exits 0. A diagnosis read cannot tell them apart.
fn live_owner(path: &Path) -> Option<u32> {
    let info = portfile::read(path)?;
    if portfile::is_stale(&info) {
        None
    } else {
        Some(info.pid)
    }
}

/// Attempt to acquire the lock. `lock_file` = the file to hold (created if absent).
///
/// The folder of `lock_file` must already exist — creating it and computing the path (source of truth: `data_dir::DataLayout`) are the caller's job.
///
/// ★"Already running" only when open failure + a live record both hold★: open failure alone can come from a third party's handle · permissions, and reading that as a duplicate makes the daemon exit without leaving a cause (the symptom ADR-0134 decision 4 aims to eliminate).
// ADR-0135
pub fn acquire(lock_file: &Path) -> Result<Acquired, AcquireError> {
    acquire_with(lock_file, OPEN_ATTEMPTS, OPEN_RETRY_DELAY)
}

fn acquire_with(path: &Path, attempts: u32, delay: Duration) -> Result<Acquired, AcquireError> {
    let mut tried = 0;
    let mut cleared_readonly = false;
    loop {
        tried += 1;
        // ★Non-truncating open★: truncating would erase someone else's valid record on open. If we win, `InstanceGuard::publish` overwrites it; if we lose, not touching it is right.
        match open_deny_write(path) {
            Ok(file) => return Ok(Acquired::Held(InstanceGuard { file })),
            // ★Do not mimic a guard (module header)★: returning `Held` where no exclusion exists lets both daemons start. Report failure so the caller stops startup. An `Unsupported` carrying an OS error code came from the open attempt itself, so it goes to `Io` below.
            Err(e) if e.kind() == io::ErrorKind::Unsupported && e.raw_os_error().is_none() => {
                return Err(AcquireError::Unsupported);
            }
            Err(e) if is_access_denied(&e) => {
                // ★Clear the read-only attribute and try once more (load-bearing — without it, a permanent deadlock)★: setting the attribute bypasses share arbitration even while we hold the file (measured), and afterwards the write open is blocked by 5, not 32 — not a retry target, so it goes out as Io, the daemon exits 1 (release has no console), and the client sees a 5-second connection timeout that even a reboot does not clear. The client's folder probe writes a differently named file, so it cannot catch this. Backup restore · read-only media copies · antivirus quarantine release all leave the attribute.
                //   ★Only once★: if it is still 5 after clearing, it is an ACL, not the attribute.
                if !cleared_readonly && clear_readonly_attr(path) {
                    cleared_readonly = true;
                    continue;
                }
                return Err(AcquireError::AccessDenied {
                    path: path.display().to_string(),
                    source: e,
                });
            }
            Err(e) => {
                // ★Retry only a sharing violation★: only that "might succeed a moment later"; a missing folder does not change by waiting, and reporting it as FileBusy would wrongly blame another program.
                if !is_sharing_violation(&e) {
                    return Err(AcquireError::Io(e));
                }
                // ★No sleep after the last attempt★: it earns nothing and only delays the error the user waits for.
                if tried >= attempts {
                    if let Some(pid) = live_owner(path) {
                        return Ok(Acquired::AlreadyRunning { pid });
                    }
                    // ★Open once more after the diagnosis read★: the interference may have cleared in between; emitting FileBusy would make the daemon back off from a folder it can now take.
                    //   ★This success branch has no test (known unverified)★ — no deterministic way to close the interfering handle exactly between these two lines. The failure branch is covered by the busy test.
                    return match open_deny_write(path) {
                        Ok(file) => Ok(Acquired::Held(InstanceGuard { file })),
                        Err(_) => Err(AcquireError::FileBusy {
                            attempts: tried,
                            source: e,
                        }),
                    };
                }
                std::thread::sleep(delay);
            }
        }
    }
}

/// Clears the read-only attribute. True on success.
///
/// ★Do not touch the ACL★: editing permissions changes the user's policy; this reverts only one attribute on our own runtime file.
fn clear_readonly_attr(path: &Path) -> bool {
    let Ok(md) = std::fs::metadata(path) else {
        return false;
    };
    let mut perms = md.permissions();
    if !perms.readonly() {
        return false; // It was not the attribute — opening again gives the same 5.
    }
    perms.set_readonly(false);
    std::fs::set_permissions(path, perms).is_ok()
}

/// ★The only thing to assert on non-Windows★: that it does not mimic a guard.
#[cfg(all(test, not(windows)))]
mod non_windows_tests {
    #[test]
    fn acquire_refuses_instead_of_returning_a_guard_that_guarantees_nothing() {
        let got = super::acquire(std::path::Path::new("."));
        assert!(
            matches!(got, Err(super::AcquireError::Unsupported)),
            "returning Held on a platform without an exclusion mechanism would start both daemons"
        );
    }
}

// ★All Windows-gated (intended)★: every assertion relies on share-mode arbitration; elsewhere `acquire` yields `Unsupported`, so lifting the gate makes them pass vacuously or break.
#[cfg(all(test, windows))]
mod tests {
    use super::{acquire, acquire_with, Acquired, DaemonInfo, DAEMON_FILE};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn held(r: Result<Acquired, super::AcquireError>) -> bool {
        matches!(r, Ok(Acquired::Held(_)))
    }

    fn lock_in(dir: &Path) -> PathBuf {
        dir.join(DAEMON_FILE)
    }

    /// Unique path per test so folders do not overlap (the lock scope is the folder).
    fn fresh_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("engram-instance-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test folder");
        dir
    }

    /// To look like a live daemon the pid must be alive — use our own process.
    fn record_of_this_process() -> DaemonInfo {
        DaemonInfo {
            pid: std::process::id(),
            host: "127.0.0.1".into(),
            port: 54321,
            protocol_version: 1,
            start_time: 0,
            owner_account: String::new(),
        }
    }

    fn write_live_record(path: &Path) {
        let bytes = record_of_this_process()
            .to_json_pretty()
            .expect("serialize");
        std::fs::write(path, bytes).expect("write the record beforehand");
    }

    #[test]
    fn second_guard_on_same_folder_is_refused() {
        let dir = fresh_dir("same");
        let mut first =
            match acquire(&lock_in(&dir)).expect("the first acquisition is not a system error") {
                Acquired::Held(g) => g,
                Acquired::AlreadyRunning { .. } => panic!("the first daemon must get the lock"),
            };
        // Duplicate = "open failure + a live record" both hold — reproduces the state after the first daemon has emitted.
        first.publish(&record_of_this_process()).expect("publish");

        let fast = || acquire_with(&lock_in(&dir), 2, Duration::from_millis(1));
        assert!(
            matches!(fast(), Ok(Acquired::AlreadyRunning { .. })),
            "a second daemon in the same folder must be refused"
        );
        // The refusal is not a one-time state.
        assert!(
            matches!(fast(), Ok(Acquired::AlreadyRunning { .. })),
            "it must keep being refused while the first guard is alive"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guards_on_different_folders_coexist() {
        // Normal use: unpacking a portable distribution in two places and running each.
        let a = fresh_dir("diff-a");
        let b = fresh_dir("diff-b");
        let ga = acquire(&lock_in(&a)).expect("acquire A");
        let gb = acquire(&lock_in(&b)).expect("acquire B");
        assert!(
            matches!(ga, Acquired::Held(_)) && matches!(gb, Acquired::Held(_)),
            "with different folders both must start"
        );
        drop(ga);
        drop(gb);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn dropping_guard_releases_the_folder() {
        let dir = fresh_dir("release");
        let first = acquire(&lock_in(&dir)).expect("first acquisition");
        drop(first);
        assert!(
            held(acquire(&lock_in(&dir))),
            "after releasing the guard, the same folder must be acquirable again"
        );
        assert!(
            dir.join(DAEMON_FILE).exists(),
            "the connection file lives inside the folder"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Guards the entire reason for merging (ADR-0135 decision 1)★: the client must be able to read the file the daemon holds. Narrowing the share or reviving range locking breaks here.
    #[test]
    fn a_client_can_read_the_record_while_the_daemon_holds_it() {
        let dir = fresh_dir("read-while-held");
        let mut guard = match acquire(&lock_in(&dir)).expect("acquire") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("an empty folder must be acquired"),
        };
        let want = record_of_this_process();
        guard.publish(&want).expect("publish");

        let path = dir.join(DAEMON_FILE);
        let bytes = std::fs::read(&path).expect("it must be readable even while held");
        assert_eq!(
            DaemonInfo::parse(&bytes).expect("parse"),
            want,
            "it must read back exactly as written"
        );
        // Also nails down the read-only open (permissive share) — the form the diagnosis path uses.
        assert!(
            std::fs::OpenOptions::new().read(true).open(&path).is_ok(),
            "a read-only open must be allowed too"
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Write only after acquisition (ADR-0135 §Impact)★: the failure path does not touch the file — otherwise two daemons' writes mix and the backing-off side erases the attached client's address.
    #[test]
    fn a_refused_daemon_does_not_touch_the_record() {
        let dir = fresh_dir("no-write-on-refusal");
        let mut first = match acquire(&lock_in(&dir)).expect("first acquisition") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("an empty folder must be acquired"),
        };
        let owner = record_of_this_process();
        first.publish(&owner).expect("publish");

        assert!(
            matches!(
                acquire_with(&lock_in(&dir), 2, Duration::from_millis(1)),
                Ok(Acquired::AlreadyRunning { .. })
            ),
            "the second must be refused"
        );
        let bytes = std::fs::read(dir.join(DAEMON_FILE)).expect("read");
        assert_eq!(
            DaemonInfo::parse(&bytes).expect("parse"),
            owner,
            "a refused daemon must not touch the record"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Blocks the regression★: with the share reverted to full denial (0), our open fails against a third party that opens with a permissive share (antivirus · indexers) and the daemon exits quietly.
    /// ★The name is the scope★: restricted-share third parties are the separate test below.
    #[test]
    fn a_permissive_third_party_handle_does_not_look_like_another_daemon() {
        let dir = fresh_dir("third-party-permissive");
        let path = dir.join(DAEMON_FILE);
        // A live record must not make it look like a duplicate — the basis is the handle, not the content.
        write_live_record(&path);
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("third-party read handle (std default = permissive sharing)");

        assert!(
            held(acquire(&lock_in(&dir))),
            "the daemon must get the lock even while a permissive third-party handle is open"
        );

        drop(onlooker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A third party holding a restricted share blocks our open (32). It must emit a distinguishable error without mistaking it for a duplicate (a duplicate exits 0 quietly and hides the cause).
    /// ★Holds even if a live record remains★: this test writes a stale record — if the diagnosis read finds no "live owner", it is interference.
    #[test]
    fn a_restrictive_third_party_handle_is_reported_as_busy_not_as_another_daemon() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = fresh_dir("third-party-restrictive");
        let path = dir.join(DAEMON_FILE);
        let mut dead = record_of_this_process();
        dead.pid = 0; // A dead pid — meaning no daemon owns this folder.
        std::fs::write(&path, dead.to_json_pretty().expect("serialize")).expect("write beforehand");
        const SHARE_READ_ONLY: u32 = 1;
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_ONLY)
            .open(&path)
            .expect("restrictive-sharing third-party handle");

        // Small retry budget so the test stays fast (production budget is a constant).
        let got = acquire_with(&lock_in(&dir), 2, Duration::from_millis(1));
        assert!(
            matches!(got, Err(super::AcquireError::FileBusy { .. })),
            "it must be reported as 'held', not as a duplicate"
        );

        // Once the interference is gone it is acquired as is — the reason retrying is meaningful.
        drop(onlooker);
        assert!(
            held(acquire(&lock_in(&dir))),
            "it must be acquired once the obstruction clears"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Permanent deadlock prevention (load-bearing)★: with the read-only attribute set, our open is blocked by 5 rather than 32, is not caught by the retry, and the daemon dies on every startup — see `acquire`.
    #[test]
    fn a_read_only_attribute_is_cleared_instead_of_wedging_the_daemon() {
        let dir = fresh_dir("readonly-attr");
        let path = dir.join(DAEMON_FILE);
        write_live_record(&path);
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).expect("set the read-only attribute");
        // Premise check — the attribute actually blocks the write open (else this test proves nothing).
        assert!(
            super::open_deny_write(&path).is_err(),
            "this test is meaningful only if the read-only attribute blocks a write open"
        );

        let got = acquire(&lock_in(&dir));
        assert!(
            held(got),
            "it must clear the attribute and acquire (no permanent deadlock)"
        );
        assert!(
            !std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .readonly(),
            "the attribute must actually be cleared"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Nails down a residual (not a fixing test)★: a third party holding it plus an old record whose pid is alive reads as "already running" and exits 0. If the behavior changes, the design changed.
    #[test]
    fn a_live_looking_record_makes_a_third_party_hold_look_like_another_daemon() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = fresh_dir("false-live");
        let path = dir.join(DAEMON_FILE);
        write_live_record(&path); // pid = ourselves = alive
        const SHARE_READ_ONLY: u32 = 1;
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_ONLY)
            .open(&path)
            .expect("restrictive-sharing third-party handle");

        let got = acquire_with(&lock_in(&dir), 2, Duration::from_millis(1));
        assert!(
            matches!(got, Ok(Acquired::AlreadyRunning { .. })),
            "known misdiagnosis — if this changed, the design changed: {:?}",
            got.err().map(|e| e.to_string())
        );

        drop(onlooker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★R1 regression prevention★: deleting or renaming the connection file while the guard is held would let a second daemon create and grab a new file at the same path, so two daemons run in one folder.
    #[test]
    fn the_record_file_cannot_be_unlinked_while_held() {
        let dir = fresh_dir("unlink");
        let guard = acquire(&lock_in(&dir)).expect("acquire");
        assert!(matches!(guard, Acquired::Held(_)));
        let path = dir.join(DAEMON_FILE);

        assert!(
            std::fs::remove_file(&path).is_err(),
            "deletion must be refused while held (if deleted, a second daemon grabs a new file)"
        );
        assert!(
            std::fs::rename(&path, dir.join("moved.json")).is_err(),
            "rename must be refused for the same reason"
        );
        assert!(path.exists(), "the file must stay as is");

        drop(guard);
        // Once released it can be deleted — this also nails down that the lock does not hold the file permanently.
        assert!(
            std::fs::remove_file(&path).is_ok(),
            "deletable after releasing the guard"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── the shell-token file (ADR-0395) — real DACLs ──

    fn held_guard(dir: &Path) -> super::InstanceGuard {
        match acquire(&lock_in(dir)).expect("acquire") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("an empty folder must be acquired"),
        }
    }

    #[test]
    fn the_token_file_is_created_owner_only_and_reads_back() {
        use engram_dashboard_platform::owner_only::is_owner_only;
        let dir = fresh_dir("token-create");
        let guard = held_guard(&dir);
        let path = dir.join("daemon-token.S-1-5-21-test");
        guard.write_token_file(&path, "first").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        assert!(is_owner_only(&path).unwrap(), "created owner-only");
        // Every start replaces it: a new token, still owner-only.
        guard.write_token_file(&path, "second").expect("rewrite");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(is_owner_only(&path).unwrap());
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file with the folder's inherited ACL (a pre-S37 folder, or one someone placed) is deleted and recreated owner-only.
    #[test]
    fn a_wider_token_file_is_deleted_and_recreated_owner_only() {
        use engram_dashboard_platform::owner_only::is_owner_only;
        let dir = fresh_dir("token-wider");
        let guard = held_guard(&dir);
        let path = dir.join("daemon-token.S-1-5-21-test");
        std::fs::write(&path, "planted").unwrap();
        assert!(!is_owner_only(&path).unwrap(), "the setup file is wider");
        guard.write_token_file(&path, "fresh").expect("write");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh");
        assert!(is_owner_only(&path).unwrap());
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A previous file that cannot be deleted (held without delete sharing here) refuses the start and is left as it was.
    #[test]
    fn a_token_file_that_cannot_be_deleted_refuses_the_start() {
        let dir = fresh_dir("token-undeletable");
        let guard = held_guard(&dir);
        let path = dir.join("daemon-token.S-1-5-21-test");
        std::fs::write(&path, "old").unwrap();
        let holder = engram_dashboard_platform::fs::open_deny_write(&path).expect("hold");
        let err = guard
            .write_token_file(&path, "new")
            .expect_err("an undeletable file must refuse the start");
        assert!(err.to_string().contains("could not be deleted"), "{err}");
        drop(holder);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `daemon.json` stays a plain file with the folder's ACL (another account must still read it and take over after a crash) and
    ///   carries the owner account but no token.
    #[test]
    fn the_record_keeps_its_inherited_acl_and_carries_no_token() {
        use engram_dashboard_platform::owner_only::is_owner_only;
        let dir = fresh_dir("record-acl");
        let mut guard = held_guard(&dir);
        let mut record = record_of_this_process();
        record.owner_account = "S-1-5-21-owner".into();
        guard.publish(&record).expect("publish");
        let path = dir.join(DAEMON_FILE);
        assert!(!is_owner_only(&path).unwrap(), "not owner-only");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("token"), "{text}");
        assert!(text.contains("S-1-5-21-owner"), "{text}");
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ── the shell-token file's steps against fake OS calls (any OS) ──
#[cfg(test)]
mod token_file_tests {
    use super::{write_token_file_with, TokenFiles};
    use std::cell::RefCell;
    use std::fs::File;
    use std::io;
    use std::path::{Path, PathBuf};

    /// Scripted OS: `owner_only` = the previous file's verdict (`None` = no file); `remove` = what deleting answers; `create_ok` = whether
    ///   creating works. Records the calls in order.
    struct Script {
        owner_only: Option<bool>,
        remove: fn() -> io::Result<()>,
        create_ok: bool,
        dir: PathBuf,
        calls: RefCell<Vec<&'static str>>,
    }

    impl TokenFiles for Script {
        fn is_owner_only(&self, _path: &Path) -> io::Result<bool> {
            self.calls.borrow_mut().push("check");
            self.owner_only
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
        fn remove(&self, _path: &Path) -> io::Result<()> {
            self.calls.borrow_mut().push("remove");
            (self.remove)()
        }
        fn create_owner_only(&self, _path: &Path) -> io::Result<File> {
            self.calls.borrow_mut().push("create");
            if self.create_ok {
                File::create(self.dir.join("created"))
            } else {
                Err(io::Error::new(io::ErrorKind::Unsupported, "no file ACLs"))
            }
        }
    }

    /// A fresh folder per call — the created stand-in file goes there.
    fn script(owner_only: Option<bool>, remove: fn() -> io::Result<()>, create_ok: bool) -> Script {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("engram-token-steps-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Script {
            owner_only,
            remove,
            create_ok,
            dir,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn removed() -> io::Result<()> {
        Ok(())
    }

    /// Not a lock (`is_busy` would retry it), so the attempt count stays one.
    fn undeletable() -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a foreign owner denies delete",
        ))
    }

    fn absent() -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    #[test]
    fn a_foreign_or_wider_previous_file_is_deleted_then_recreated() {
        let os = script(Some(false), removed, true);
        write_token_file_with(Path::new("t"), "tok", &os).expect("replaced");
        assert_eq!(*os.calls.borrow(), ["check", "remove", "create"]);
        assert_eq!(
            std::fs::read_to_string(os.dir.join("created")).unwrap(),
            "tok"
        );
    }

    #[test]
    fn an_owner_only_previous_file_is_still_replaced_with_a_new_token() {
        let os = script(Some(true), removed, true);
        write_token_file_with(Path::new("t"), "tok", &os).expect("replaced");
        assert_eq!(*os.calls.borrow(), ["check", "remove", "create"]);
    }

    #[test]
    fn a_previous_file_that_cannot_be_deleted_refuses_without_creating() {
        let os = script(Some(false), undeletable, true);
        let err = write_token_file_with(Path::new("t"), "tok", &os).expect_err("refused");
        assert!(err.to_string().contains("could not be deleted"), "{err}");
        assert!(
            !os.calls.borrow().contains(&"create"),
            "{:?}",
            os.calls.borrow()
        );
    }

    #[test]
    fn a_token_file_that_cannot_be_made_owner_only_refuses_the_start() {
        let os = script(None, absent, false);
        let err = write_token_file_with(Path::new("t"), "tok", &os).expect_err("refused");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert!(err.to_string().contains("owner-only"), "{err}");
    }
}
