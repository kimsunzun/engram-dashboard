//! Single-instance guard — holds the data folder's `daemon.json` with a **restricted share mode** (ADR-0135).
//!
//! Only **one daemon per data folder** may run. The decision is **obtaining a handle**
//! to that folder's connection file with read+write access — if obtained, this folder is ours; if not, someone else holds it or a third party
//! is interfering.
//!
//! ★The file we hold = the file the client reads (ADR-0135 decision 1)★: if the lock file and the connection file are split, then when the data folder is
//! deleted while the daemon is running, **only the undeletable lock file remains and the address disappears** — the client can neither
//! connect (no address) nor shut it down (the shutdown path uses that address), and a newly started daemon is blocked by the lock and quietly
//! backs off. The only escape hatch was Task Manager. ★Do not split it into two files again★ — keeping them merged makes
//! that state impossible to create.
//!
//! ## Share mode = allow reads only (`FILE_SHARE_READ` = 1 — platform's `fs::open_deny_write`)
//!
//! Measured (2026-08-14, share-mode combination probe · independently re-measured by the reviewer):
//! - A second daemon's read+write open → `ERROR_SHARING_VIOLATION` (32). **This is all of the exclusion.**
//!   Arbitration is applied to the **requested access**, so whichever of 1·3·7 the other side holds the share with, and whether it opens write-only or append,
//!   all are refused.
//! - Deleting or renaming this file → 32. Renaming the data folder → `ERROR_ACCESS_DENIED` (5).
//! - All 8 alias kinds (uppercase · `..` · `\\?\` · 8.3 short name · junction · hard link · `subst` · mapped drive vs UNC) converge on the same
//!   file object (reviewer measurement, including 2 real SMB shares). The OneDrive sync root is the same.
//!
//! ★The reading side **must not narrow the share**★: a client's read open passes only when that open uses a
//! **permissive share** — the access we hold is read+write, so even when opening read-only, dropping write from the share
//! (e.g. the .NET `FileStream` default `FileShare.Read` · `[System.IO.File]::ReadAllText`) is refused with 32
//! (measured). Today's consumers are Rust `std::fs::read` and Node `fs.readFileSync`, and both default to permissive, so
//! there is no problem. **Look at this line when attaching a new consumer** — generalizing to "reads pass" breaks it there.
//!
//! ★Do not add `FILE_SHARE_WRITE` (do not revive it)★: write sharing means "allow others' **write** opens", so
//! with share 3 (read+write) a second daemon's read+write open **succeeds as is** (measured — OK, not 32).
//! Then two daemons run in one folder and each client trusts a different endpoint.
//!
//! ★Do not add `FILE_SHARE_DELETE` either (do not revive it)★: if delete · rename is open, someone clears this file
//! and a second daemon creates and grabs a **new file** at the same path — the result is the same as above.
//!
//! ★Do not revive range locking (`try_lock`)★: per MS docs an exclusive range lock denies even other processes'
//! **reads**, and std's API locks the **whole** file. If the client cannot read this file, merging it was pointless.
//!
//! ★On non-Windows, [`acquire`] **refuses outright** (`Unsupported`)★: the shared-restricted open (`open_deny_write`) does not
//! open outside Windows and returns `Unsupported`, and no substitute mechanism was provided — the daemon itself is Windows-only
//! (WMI spawn · Job Object · taskkill). ★Do not return a "guard that guarantees nothing" (do not revive it)★:
//! then both daemons succeed in one folder and the one that starts later quietly overwrites the earlier one's endpoint — if written only in prose,
//! the test would pass vacuously. That is why **the whole** test module that asserts share semantics is `#[cfg(windows)]`.
//!
//! ★Tell a third party's restricted open apart from a real duplicate (ADR-0135 §영향 [impact])★: even when antivirus · indexers · backup open it briefly with a narrow
//! share, our open fails with the same 32 (measured). So it retries briefly, and if that still fails, it **opens the file
//! read-only** and checks whether a live daemon's record is visible — if visible, a duplicate (normal yielding); if not,
//! [`AcquireError::FileBusy`]. ★This read is a diagnosis, not an identity decision★: identity is still the single
//! question "did we hold the handle", and ownership is not claimed by the value written in the file — using the written value as identity would make two distributions made by copying the folder
//! wholesale claim the same identity and one could not start (copying is normal use of a portable distribution).
//!
//! ★Do not identify the folder by name or path (do not revive it)★: to name the lock we would have to
//! normalize the folder path ourselves, and missing even one of the case · `..` · 8.3 · junction · subst · mapped-drive spellings puts **two daemons
//! in the same folder**. Holding the file directly leaves that spelling resolution to the OS.
//!
//! ★Copying while it runs — a residual★: a plain copy (`copy`) succeeds (measured). However, if the copy tool opens the source allowing **read
//! sharing only**, it can conflict with the write access we hold and fail with 32.
//!
//! ★The reach goes beyond one computer — but only when the filesystem does its part★: Windows SMB redirectors propagate
//! ShareAccess and the server arbitrates, so a daemon on **another computer** looking at the same folder is also excluded (measured).
//! But share arbitration is **the filesystem's job**, so on third-party redirectors · user-mode filesystems (WebDAV · Windows NFS
//! client · Dokan/WinFsp-based rclone·sshfs-win · Google Drive virtual drive) the implementer may
//! ignore it, and then **both daemons acquire and each emits**. With `try_lock` gone there is no second line of
//! defense now — do not put the data folder in such places.
//!
//! ★Lifetime rule★: the acquired file must be **held for as long as the daemon process lives**. When the guard is dropped,
//! the file closes and the singleness guarantee breaks, so the daemon's `run()` keeps the returned guard alive until the process
//! ends. Even when the process exits abnormally the OS closes the handle, but **immediacy is not guaranteed** — MS says
//! the release time depends on system resource conditions, and on an SMB share, when a client drops entirely, the server keeps the open until the session · durable
//! handle timers expire. So a stale-entry decision is unnecessary, but do not assume either that "it can restart
//! immediately after it dies".

use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

// It must be an individual `use` — a grouped `use` is something net gate 2a (lib.rs header) cannot count symbols in.
use engram_dashboard_platform::fs::is_access_denied;
use engram_dashboard_platform::fs::is_sharing_violation;
use engram_dashboard_platform::fs::open_deny_write;

use crate::portfile::{self, DaemonInfo, DAEMON_FILE};

/// Number of **open** retries to get past a third party holding it with a restricted share (including the first attempt).
const OPEN_ATTEMPTS: u32 = 5;

/// Wait between open retries.
///
/// ★The worst-case delay [`acquire`] adds = `(OPEN_ATTEMPTS - 1) × OPEN_RETRY_DELAY` = **about 400ms**★
/// (no sleep after the last attempt — it earns nothing and only delays the error the user is waiting for).
/// The normal path (first attempt succeeds) is 0ms, and **a duplicate daemon also uses up this budget** — duplicates and third-party
/// interference both fail with the same 32 and cannot be split by it, so they are told apart by the diagnosis only after the retries run to the end. That delay
/// lands only on the daemon that will back off, so what the user sees is the daemon already running. Before enlarging this product, compare it with the client ensure polling
/// limit (5 seconds).
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Proof of owning this folder. While alive the file handle is open, and Drop is the release.
pub struct InstanceGuard {
    file: File,
}

impl InstanceGuard {
    /// Writes the connection record **in place through the held handle** (ADR-0135 decision 1).
    ///
    /// ★Do not change it to a temp file + rename★: we hold this file with delete sharing closed, so rename is
    /// refused (measured 32). And because it is an in-place write, **a client can see partially written content** —
    /// the reading side must treat a parse failure as "not ready yet" and keep polling.
    ///
    /// ★The type guarantees that writing comes after acquisition★: this method can be called only on a guard. If the order
    /// is reversed, two daemons' writes mix.
    // ADR-0135
    pub fn publish(&mut self, info: &DaemonInfo) -> io::Result<()> {
        portfile::write_in_place(&mut self.file, info)
    }
}

/// The two normal outcomes of [`acquire`]. Neither means "the system is broken".
pub enum Acquired {
    /// We are the only daemon for this folder.
    Held(InstanceGuard),
    /// Another daemon already holds this folder — backing off is the normal behavior.
    ///
    /// `pid` is the value that daemon left in the file (for diagnosis — not the basis of the ownership decision).
    AlreadyRunning { pid: u32 },
}

/// ★It exists to be reported separately from duplicates★: if the two are lumped into one error, the user cannot
/// tell why the daemon did not start (a duplicate is normal; the two below are states that need action).
#[derive(Debug)]
pub enum AcquireError {
    /// A third party has the connection file open with a restricted share and we could not open it within the retry budget. **Not a duplicate daemon.**
    FileBusy { attempts: u32, source: io::Error },
    /// The file exists but **write access itself was denied** (ACL etc.). Waiting does not clear it.
    ///
    /// ★Why it is kept separate★: this state stays as is across restarts · reboots, so what the user should do is the
    /// opposite of "held briefly". The read-only **attribute** is cleared by [`acquire`] itself, so it does not land here.
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
                "{DAEMON_FILE} 을 다른 프로그램이 붙들고 있음({attempts}회 재시도 후 포기): {source}"
            ),
            Self::AccessDenied { path, source } => write!(
                f,
                "{path} 에 쓸 권한이 없음(읽기 전용 속성은 자동 해제를 시도했으나 실패 — 폴더 권한을 확인하거나 쓰기 가능한 위치에 압축을 풀어 주세요): {source}"
            ),
            Self::Unsupported => write!(
                f,
                "이 플랫폼에는 단일 인스턴스 배제 기전이 없음(Windows 전용)"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AcquireError {}

/// When the open stays blocked, look only at **why** it is blocked — the pid if there is a live daemon's record.
///
/// ★A diagnosis, not an identity decision★: whatever is seen here, ownership has already been decided as "could not get the handle",
/// and this value is used only to choose the message to show the user. The read opens with a permissive share (std default), so even the
/// file we failed to open can be read.
///
/// ★Residual 1 — if the earlier daemon is **not yet emitting**, a duplicate is misdiagnosed as interference★: that daemon created and
/// holds the file but has not yet written the record, so there is nothing to read. That window spans not just right after acquisition but **bind + MCP server startup +
/// the whole wiring** (steps 3~8 of the daemon's `run()`), and measured (2026-08-14) on a warm local startup it is 16~19ms,
/// so the 400ms budget has a 20x margin, but **it can be exceeded on a cold start or a network data folder**. Even when misdiagnosed,
/// the result is only the log wording and the exit code (neither has a consumer — the client polls the file). So
/// no separate wait budget was added.
///
/// ★Residual 2 — the opposite misdiagnosis cannot be prevented★: if a third party is holding it but the pid of the old record left in the file
/// happens to be alive (a record from another folder's daemon was copied over, or pid + creation time coincidentally match), it is
/// read as "already running" and quietly exits 0. A diagnosis read cannot in principle tell them apart.
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
/// The folder of `lock_file` **must already exist** — creating the folder is the caller's job (daemon startup order).
/// Path computation is also the caller's job (this crate does not know the data folder layout — the daemon's `data_dir::DataLayout` is the source of truth).
///
/// ★The basis for "already running" is only when open failure + a live record both hold★: do not
/// declare a duplicate from open failure alone. Opening also fails for reasons unrelated to the daemon, such as a third party's handle · permissions, and reading that as a duplicate
/// makes the daemon exit without leaving a cause (a connection timeout with no cause = the very symptom ADR-0134 decision 4 aims to eliminate).
// ADR-0135
pub fn acquire(lock_file: &Path) -> Result<Acquired, AcquireError> {
    acquire_with(lock_file, OPEN_ATTEMPTS, OPEN_RETRY_DELAY)
}

fn acquire_with(path: &Path, attempts: u32, delay: Duration) -> Result<Acquired, AcquireError> {
    let mut tried = 0;
    let mut cleared_readonly = false;
    loop {
        tried += 1;
        // ★It must be a non-truncating open★: truncating would erase someone else's valid record the moment we open. If we win,
        //   `InstanceGuard::publish` overwrites it, and if we lose, not touching the file is right.
        match open_deny_write(path) {
            Ok(file) => return Ok(Acquired::Held(InstanceGuard { file })),
            // ★Do not mimic a guard (module header)★: returning `Held` on an OS with no means of exclusion lets both daemons
            //   start and the later one overwrites the earlier one's endpoint. Report failure so the caller stops startup.
            //   An `Unsupported` carrying an OS error code was raised by the OS while attempting the open, so it is sent to `Io` below.
            Err(e) if e.kind() == io::ErrorKind::Unsupported && e.raw_os_error().is_none() => {
                return Err(AcquireError::Unsupported);
            }
            Err(e) if is_access_denied(&e) => {
                // ★Clear the read-only **attribute** and try once more (load-bearing — without it, a permanent deadlock)★:
                //   setting the attribute bypasses share arbitration and succeeds **even while we hold it** (measured), and after that
                //   a restart is blocked on the write open by 5, not 32 — it is not a retry target, so it goes out as
                //   Io on the first attempt and the daemon exits 1, and in release there is not even a console to see that log. The client's
                //   folder probe writes a file with a **different name**, so it passes and cannot catch the cause either. The result is
                //   a 5-second connection timeout that even a reboot does not clear. Backup restore · copying from read-only media · antivirus
                //   quarantine release all leave this attribute.
                //   ★Only once★: if it is still 5 after clearing, it is an ACL, not the attribute, and waiting · retrying gives the same result.
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
                // ★Retry only a sharing violation★: only that is a failure that "might succeed a moment later". A failure
                //   like a missing folder does not change by waiting, and reporting it as FileBusy wrongly blames
                //   "another program" as the cause.
                if !is_sharing_violation(&e) {
                    return Err(AcquireError::Io(e));
                }
                // ★No sleep after the last attempt★: that wait earns nothing and only delays the error the user is
                //   waiting for. So the wait goes only between attempts (attempts-1 times in total).
                if tried >= attempts {
                    if let Some(pid) = live_owner(path) {
                        return Ok(Acquired::AlreadyRunning { pid });
                    }
                    // ★Open once more after the diagnosis read★: the interference may have cleared in between, and in that case
                    //   emitting FileBusy would make the daemon back off from a folder **it can now take**.
                    //   ★This success branch has no test (known unverified)★ — there is no deterministic means to make the interfering handle close exactly between these two
                    //   lines. The failure branch is covered by the busy test.
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

/// Clears the read-only **attribute**. True on success (= the attribute is now gone).
///
/// ★Do not touch the ACL★: editing permissions changes the user's policy and is not the daemon's job. This function
/// reverts only **one attribute** attached to our own runtime file.
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
            "배제 기전이 없는 플랫폼에서 Held 를 돌려주면 데몬 둘이 다 뜬다"
        );
    }
}

// ★All of it is Windows-gated (intended)★: every assertion below relies on share-mode arbitration, and on other platforms
//   `acquire` yields `Unsupported` in the first place — lifting the gate makes them pass vacuously or simply break.
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

    /// Makes a unique path so folders do not overlap between tests (and with cargo parallel runs) — the lock scope is
    /// the folder, so overlapping folders refuse each other by themselves.
    fn fresh_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("engram-instance-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("테스트 폴더 생성");
        dir
    }

    /// For it to look like a live daemon, the pid must actually be alive — use our own process.
    fn record_of_this_process() -> DaemonInfo {
        DaemonInfo {
            pid: std::process::id(),
            host: "127.0.0.1".into(),
            port: 54321,
            token: engram_dashboard_protocol::Secret::new("a".repeat(64)),
            protocol_version: 1,
            start_time: 0,
        }
    }

    fn write_live_record(path: &Path) {
        let bytes = record_of_this_process().to_json_pretty().expect("직렬화");
        std::fs::write(path, bytes).expect("레코드 선기록");
    }

    #[test]
    fn second_guard_on_same_folder_is_refused() {
        let dir = fresh_dir("same");
        let mut first = match acquire(&lock_in(&dir)).expect("첫 획득은 시스템 오류가 아님")
        {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("첫 데몬은 잠금을 얻어야"),
        };
        // The duplicate decision is when "open failure + a live record" both hold — this reproduces the state after the first daemon has emitted.
        first.publish(&record_of_this_process()).expect("발행");

        let fast = || acquire_with(&lock_in(&dir), 2, Duration::from_millis(1));
        assert!(
            matches!(fast(), Ok(Acquired::AlreadyRunning { .. })),
            "같은 폴더의 두 번째 데몬은 거부돼야"
        );
        // The refusal is not a one-time state.
        assert!(
            matches!(fast(), Ok(Acquired::AlreadyRunning { .. })),
            "첫 가드가 살아 있는 동안 계속 거부돼야"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guards_on_different_folders_coexist() {
        // Normal use: unpacking a portable distribution in two places and running each.
        let a = fresh_dir("diff-a");
        let b = fresh_dir("diff-b");
        let ga = acquire(&lock_in(&a)).expect("A 획득");
        let gb = acquire(&lock_in(&b)).expect("B 획득");
        assert!(
            matches!(ga, Acquired::Held(_)) && matches!(gb, Acquired::Held(_)),
            "다른 폴더면 둘 다 떠야"
        );
        drop(ga);
        drop(gb);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn dropping_guard_releases_the_folder() {
        let dir = fresh_dir("release");
        let first = acquire(&lock_in(&dir)).expect("첫 획득");
        drop(first);
        assert!(
            held(acquire(&lock_in(&dir))),
            "가드를 놓으면 같은 폴더를 다시 잡을 수 있어야"
        );
        assert!(dir.join(DAEMON_FILE).exists(), "접속 파일은 폴더 안에 산다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★What this test guards is the entire reason for merging (ADR-0135 decision 1)★: the client **must be able to read** the file the daemon holds
    /// for a connection to be established. Narrowing the share or reviving range locking breaks here.
    #[test]
    fn a_client_can_read_the_record_while_the_daemon_holds_it() {
        let dir = fresh_dir("read-while-held");
        let mut guard = match acquire(&lock_in(&dir)).expect("획득") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("빈 폴더는 획득돼야"),
        };
        let want = record_of_this_process();
        guard.publish(&want).expect("발행");

        let path = dir.join(DAEMON_FILE);
        let bytes = std::fs::read(&path).expect("쥐고 있는 동안에도 읽혀야");
        assert_eq!(
            DaemonInfo::parse(&bytes).expect("파싱"),
            want,
            "쓴 내용 그대로 읽혀야"
        );
        // Also nails down the read-only open (permissive share) — the form the diagnosis path uses.
        assert!(
            std::fs::OpenOptions::new().read(true).open(&path).is_ok(),
            "읽기 전용 열기도 허용돼야"
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Write only after acquisition (ADR-0135 §영향 [impact])★: the acquisition-failure path does not touch the file — if it did, two daemons'
    /// writes would mix, and the side backing off would erase the address of the client already attached.
    #[test]
    fn a_refused_daemon_does_not_touch_the_record() {
        let dir = fresh_dir("no-write-on-refusal");
        let mut first = match acquire(&lock_in(&dir)).expect("첫 획득") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("빈 폴더는 획득돼야"),
        };
        let owner = record_of_this_process();
        first.publish(&owner).expect("발행");

        assert!(
            matches!(
                acquire_with(&lock_in(&dir), 2, Duration::from_millis(1)),
                Ok(Acquired::AlreadyRunning { .. })
            ),
            "두 번째는 거부돼야"
        );
        let bytes = std::fs::read(dir.join(DAEMON_FILE)).expect("읽기");
        assert_eq!(
            DaemonInfo::parse(&bytes).expect("파싱"),
            owner,
            "거부된 데몬이 레코드를 건드리면 안 됨"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★The regression this test blocks★: if the share is reverted to full denial (0), our open fails with only a third party that **opens with a permissive share**
    /// (the default behavior of antivirus · indexers), and the daemon exits quietly.
    /// ★The name is the scope★: a third party that opens with a restricted share is not covered here — that is the separate test below.
    #[test]
    fn a_permissive_third_party_handle_does_not_look_like_another_daemon() {
        let dir = fresh_dir("third-party-permissive");
        let path = dir.join(DAEMON_FILE);
        // Even with a live record present, it must not be mistaken for a duplicate — the basis of the decision is the handle, not the content.
        write_live_record(&path);
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("제3자 읽기 핸들(std 기본 = 관대한 공유)");

        assert!(
            held(acquire(&lock_in(&dir))),
            "관대한 제3자 핸들이 열려 있어도 데몬은 잠금을 얻어야"
        );

        drop(onlooker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A third party holding it with a restricted share actually blocks our open (measured 32). In that case it must emit a distinguishable error
    /// **without mistaking it for a duplicate** — with a duplicate the daemon quietly exits 0, so the user cannot see the cause.
    /// ★That holds even if a live record remains in the file★: so this test writes a stale record —
    /// if the diagnosis read cannot find a "live owner", it is classified as interference.
    #[test]
    fn a_restrictive_third_party_handle_is_reported_as_busy_not_as_another_daemon() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = fresh_dir("third-party-restrictive");
        let path = dir.join(DAEMON_FILE);
        let mut dead = record_of_this_process();
        dead.pid = 0; // A dead pid — meaning no daemon owns this folder.
        std::fs::write(&path, dead.to_json_pretty().expect("직렬화")).expect("선기록");
        const SHARE_READ_ONLY: u32 = 1;
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_ONLY)
            .open(&path)
            .expect("제한적 공유 제3자 핸들");

        // Give a small retry budget so the test does not take long (the production budget is a constant).
        let got = acquire_with(&lock_in(&dir), 2, Duration::from_millis(1));
        assert!(
            matches!(got, Err(super::AcquireError::FileBusy { .. })),
            "중복이 아니라 '붙들려 있음'으로 보고돼야"
        );

        // Once the interference is gone it is acquired as is — the reason retrying is meaningful.
        drop(onlooker);
        assert!(held(acquire(&lock_in(&dir))), "방해가 걷히면 획득돼야");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Permanent deadlock prevention (load-bearing)★: if the read-only **attribute** is set, our open is blocked by 5 rather than 32,
    /// it is not caught by the retry, and the daemon dies immediately on every startup — not cleared even by a reboot, and in release there is not even a console to see that
    /// log, so to the user it looks only like an eternal 5-second connection timeout. Backup restore · copying from read-only
    /// media · antivirus quarantine release all leave this attribute.
    #[test]
    fn a_read_only_attribute_is_cleared_instead_of_wedging_the_daemon() {
        let dir = fresh_dir("readonly-attr");
        let path = dir.join(DAEMON_FILE);
        write_live_record(&path);
        let mut perms = std::fs::metadata(&path).expect("메타").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).expect("읽기 전용 속성 부여");
        // Premise check — this attribute actually blocks the write open (if it did not, this test would prove nothing).
        assert!(
            super::open_deny_write(&path).is_err(),
            "읽기 전용 속성이 쓰기 열기를 막아야 이 테스트가 의미 있다"
        );

        let got = acquire(&lock_in(&dir));
        assert!(held(got), "속성은 걷어내고 획득해야(영구 교착 금지)");
        assert!(
            !std::fs::metadata(&path)
                .expect("메타")
                .permissions()
                .readonly(),
            "속성이 실제로 걷혀야"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★Nails down a residual (not a fixing test)★: if a third party is holding it and the old record's pid in the file
    /// happens to be alive, it is read as "already running" and quietly exits 0. A diagnosis read cannot in principle tell them
    /// apart — if the behavior changes, the design has changed, so it is caught here.
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
            .expect("제한적 공유 제3자 핸들");

        let got = acquire_with(&lock_in(&dir), 2, Duration::from_millis(1));
        assert!(
            matches!(got, Ok(Acquired::AlreadyRunning { .. })),
            "알려진 오진 — 바뀌었다면 설계가 바뀐 것이다: {:?}",
            got.err().map(|e| e.to_string())
        );

        drop(onlooker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★R1 regression prevention★: if the connection file is deleted or renamed while the guard is held, a second daemon creates and grabs a new file at the same
    /// path and wins, so **two daemons run in one folder**.
    #[test]
    fn the_record_file_cannot_be_unlinked_while_held() {
        let dir = fresh_dir("unlink");
        let guard = acquire(&lock_in(&dir)).expect("획득");
        assert!(matches!(guard, Acquired::Held(_)));
        let path = dir.join(DAEMON_FILE);

        assert!(
            std::fs::remove_file(&path).is_err(),
            "쥐고 있는 동안 삭제는 거부돼야(삭제되면 두 번째 데몬이 새 파일을 잡는다)"
        );
        assert!(
            std::fs::rename(&path, dir.join("moved.json")).is_err(),
            "이름 변경도 같은 이유로 거부돼야"
        );
        assert!(path.exists(), "파일이 그대로 있어야");

        drop(guard);
        // Once released it can be deleted — this also nails down that the lock does not hold the file permanently.
        assert!(
            std::fs::remove_file(&path).is_ok(),
            "가드를 놓으면 삭제 가능"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
