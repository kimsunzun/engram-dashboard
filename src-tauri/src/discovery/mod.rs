//! Daemon discovery — the shell finds the daemon and, if there is none, launches it via WMI and retrieves the port/token. Includes status query · stop fallback.
//!
//! ADR-0029: daemon-only (embedded removed). The app is a resident client of the daemon so it always attaches to the daemon (WS is phase4).
//!
//! ## Design — separating pure logic from the OS/WMI boundary
//! Side effects are injected as traits so unit tests do not depend on the OS · WMI · real time.
//!
//! [`ensure_daemon`] is a **pure orchestration** that works only on top of these traits, so every branch can be unit tested without a real
//! WMI spawn or a real sleep. The real spawn (WMI) integration is left as an
//! `#[ignore]` test.
//!
//! The real implementations of the traits (PID decision · WMI launch · process tree stop) merely call functions of the OS-layer crate `platform` —
//! this module's production code has no OS branches (ADR-0266).
//!
//! The data folder root · the `daemon.json` location · `logs\` are copies of the daemon's authority ([`layout`]). The shell's
//! stop path (graceful `StopDaemon`, then the identity-checked kill) is [`crate::daemon_client::shutdown`].
//!
//! ## Security
//! `DaemonInfo.token` is never printed to logs (flows only in the local IPC file).
// ADR-0271

use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_base::writable::{probe_write_in, retry_if_vanished};
use engram_dashboard_protocol::{DaemonInfo, PROTOCOL_VERSION};

pub mod layout;
#[cfg(test)]
pub(crate) mod tests;

pub use layout::{default_data_dir, DataLayout};

use layout::{data_dir_env_override, DAEMON_FILE};

/// The lock held by every shell lib test that changes `ENGRAM_DATA_DIR` or picks a path from its value — the same-path tests (`layout`) and the two real
/// WMI tests (`tests`). It is process-global environment, so with two locks they would not block each other.
// ADR-0282
#[cfg(test)]
pub(crate) static DATA_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ★Limit of the ENGRAM_DATA_DIR override — it does not reach the WMI path★ (authority for the override rule = the daemon `data_dir`):
//   this override takes effect **only on spawns that inherit the parent env**. That is, only tests that launch the daemon **directly** with `std::process::Command`
//   (the daemon's `tests/ws_e2e.rs` etc.) are isolated. This module's production spawn path (WMI Win32_Process.Create) has a child
//   that is a child of WmiPrvSE and so **does not inherit the parent env**, and this override is ignored (design settled — no channel other than daemon.json/ACL).
//   So the smoke tests that actually go through WMI (real_wmi_spawn_*) cannot be isolated by env; they poll the default
//   path (`.engram-dev`) and protect the production files with backup/restore.

fn unwritable(dir: &Path, e: &std::io::Error) -> DiscoveryError {
    DiscoveryError::DataDirUnwritable {
        path: dir.display().to_string(),
        reason: e.to_string(),
    }
}

/// Checks whether the data folder can be written **without creating anything** (client pre-check).
///
/// ★The point is not to create the folder★: this check asks "can the daemon write this folder", it does not settle that
/// folder. If the check created a folder, an empty folder would be left forever on the path where the daemon ends up using a different
/// folder.
///
/// Interpretation:
/// - If `dir` is already a folder, write a probe inside it.
/// - If `dir` is a **file**, it fails. Do not pass it by looking at the parent — `create_dir_all` is certain to fail.
/// - If it does not exist, **actually try creating the folder**, write the probe too, then undo it if we created it.
///
/// ★Do not substitute by trying to create a file in the parent folder (do not revive it)★: Windows
/// `FILE_ADD_FILE` and `FILE_ADD_SUBDIRECTORY` are **granted separately**. In a folder where only files can be created,
/// that substitute check passes and the daemon's `create_dir_all` fails, so the user sees a timeout instead of a message.
/// The required permission must be tested as it is.
///
/// ★Scope of the undo = **all** that we created, **and only that**★: it can create intermediate folders in a chain, and deleting only the leaf
/// leaves the rest permanently. So it creates **one layer at a time** from top to bottom and records only the layers where **we won** the
/// creation, then undoes from the leaf upward. It undoes on the failure path too.
///
/// ★Do not undo from a "snapshot before creating" (do not revive it)★: if the list of ancestors that did not exist is taken beforehand, ownership does
/// not hold — while A records `a/` as absent, B creates `a/`, and A later deletes **B's folder**.
/// The side that saw `AlreadyExists` at each layer must decide it is someone else's for that window to close.
///
/// ★Limit of the contract — what is deleted is "the **path** we created", not the directory **object** at that moment★:
/// if after we create it someone else deletes it and recreates it under the same name (ABA), our `remove_dir` deletes theirs.
/// To block that precisely we would hold the file ID at creation time and compare it just before deleting, but we did not because the harm is this
/// narrow: `remove_dir` deletes **only empty directories**, so it fails if someone has started writing, and that
/// victim also meets `NotFound` at the probe and recovers by trying once more with [`retry_if_vanished`].
///
/// A folder that already existed is left alone, and a folder the daemon has started writing to in the meantime is non-empty so the deletion fails,
/// which is harmless (it is a folder already in use).
///
/// ★Overlapping calls are real★ — the tray "turn daemon on" and the boot ensure are not serialized
/// (`src-tauri/src/commands/discovery.rs`). This check undoes the folder it created, so when they overlap another's probe
/// trips with `NotFound` in the vanished folder — that is why it is wrapped in [`retry_if_vanished`].
pub fn check_data_dir_writable(dir: &Path) -> Result<(), DiscoveryError> {
    retry_if_vanished(|| check_data_dir_writable_once(dir)).map_err(|e| unwritable(dir, &e))
}

fn check_data_dir_writable_once(dir: &Path) -> std::io::Result<()> {
    // ★One stat decides three branches (do not revive — do not follow `is_dir()` with `exists()`)★:
    //   if asked twice, when someone else creates the folder in between it reads as "not a folder + exists" = **a file is there**,
    //   and a perfectly fine race is wrongly reported as "a file with the same name already exists" (the overlapping-check test
    //   reproduced it by measurement). Two overlapping actors are real — the tray "turn daemon on" and the boot ensure.
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => return probe_write_in(dir),
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "같은 이름의 파일이 이미 있어 폴더를 만들 수 없음",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    // Collect the non-existent ancestors from the leaf upward (stop at the first existing ancestor — do not climb to the root: create_dir on a drive
    //   root is PermissionDenied, not AlreadyExists, measured 2026-08-14).
    let mut missing: Vec<&Path> = Vec::new();
    let mut cur = Some(dir);
    while let Some(p) = cur {
        // The last parent of a relative path is an empty path — it can be neither created nor deleted, so cut here.
        if p.as_os_str().is_empty() || p.exists() {
            break;
        }
        missing.push(p);
        cur = p.parent();
    }

    // One layer at a time from top to bottom. `created` holds only the layers **we created**.
    let mut created: Vec<&Path> = Vec::new();
    let mut failed: Option<std::io::Error> = None;
    for p in missing.iter().rev() {
        match std::fs::create_dir(p) {
            Ok(()) => created.push(p),
            // Someone else created it in the meantime = theirs → not something for us to undo.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }

    // ★Do not return early (do not revive)★: escaping midway with `?` leaves the layers already created as they are.
    let outcome = match failed {
        Some(e) => Err(e),
        None => probe_write_in(dir),
    };
    for p in created.iter().rev() {
        // Only empty ones are deleted — a folder someone else has started writing to is left as is.
        let _ = std::fs::remove_dir(p);
    }
    outcome
}

// ── 에러 ───────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("daemon exe 를 찾을 수 없음: {0}")]
    ExeNotFound(String),
    #[error("daemon.json 파싱 실패: {0}")]
    Parse(String),
    #[error("daemon spawn 실패(WMI ReturnValue={rv})")]
    SpawnFailed { rv: u32 },
    #[error("daemon 시작 대기 timeout({0:?} 초과)")]
    Timeout(Duration),
    #[error("protocol 버전 불일치: 데몬={daemon}, 기대={expected}")]
    VersionMismatch { daemon: u32, expected: u32 },
    /// ★The message carries the remedy (ADR-0134 decision 4)★: this string rises as is up to the frontend's failure surface —
    /// replacing a cause-less connection timeout is the reason this variant exists.
    #[error("데이터 폴더에 쓸 수 없음({path}): {reason} — 쓰기 가능한 위치에 압축을 풀어 주세요")]
    DataDirUnwritable { path: String, reason: String },
    #[error("io: {0}")]
    Io(String),
}

// ── injection boundary (trait) ─────────────────────────────────────────────────────────────

/// Takes start_time as well to tell apart PID reuse (M2) — it counts as alive only when "PID alive AND creation time==recorded value".
/// If start_time==0 (unknown, old daemon.json), conservatively decide by PID alone being alive.
pub trait PidLiveness {
    fn is_dead(&self, pid: u32, start_time: u64) -> bool;
}

/// Returns: Ok(Some)=valid file, Ok(None)=none (not written yet), Err=broken file.
pub trait DaemonReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError>;
}

pub trait Spawner {
    /// Spawn an absolute-path exe.
    fn spawn(&self, exe: &Path) -> Result<(), DiscoveryError>;
}

/// Polling clock — the current read comes from the base clock plus a sleep that **puts the thread to sleep** (this module is synchronous).
// ADR-0275
pub trait Clock: engram_dashboard_base::time::Clock {
    fn sleep(&self, dur: Duration);
}

// ── pure orchestration ────────────────────────────────────────────────────────────

/// A reference-based decision so the caller keeps ownership of info — Accept gives only a signal without data.
// pub(crate) for daemon_client::link::stop and daemon_client::link::endpoint · ADR-0271
pub(crate) enum AcceptCheck {
    Accept,
    DeadPid,
    VersionMismatch { daemon: u32 },
}

// ADR-0196: the liveness check comes **before** the version comparison — do not flip it. And this decision **deliberately does not look at the version
// direction**: a dead record is stale whether it is lower or higher than ours, so do not "fix" it by adding a direction decision to make it
// 「더 새 기록이면 물러난다」 (back off if the record is newer).
// pub(crate) for daemon_client::link::stop and daemon_client::link::endpoint · ADR-0271
pub(crate) fn check_acceptable(info: &DaemonInfo, liveness: &dyn PidLiveness) -> AcceptCheck {
    if liveness.is_dead(info.pid, info.start_time) {
        return AcceptCheck::DeadPid;
    }
    if info.protocol_version != PROTOCOL_VERSION {
        return AcceptCheck::VersionMismatch {
            daemon: info.protocol_version,
        };
    }
    AcceptCheck::Accept
}

/// ★The client does not delete daemon.json (do not revive — ADR-0134)★
///
/// The old implementation **deleted** the file decided stale or broken in (a) before the spawn. That deletion
/// ① gains nothing and ② causes two kinds of damage.
///
/// ① Why it gains nothing: polling accepts only "valid parse + live pid + compatible version", so even if the old file remains
///    an already dead pid is filtered out by `check_acceptable`. And the winning daemon overwrites its own daemon.json **after getting the lock**
///    — there is no need to clear the spot in advance.
/// ② Damage: (i) if the deletion comes before the gate (`pre_spawn`), then when the gate blocks, only an irreversible deletion remains.
///    (ii) Worse, in a portable folder on a network share (a configuration that ADR-0134 decision 3 states it supports)
///    the pid of **another computer** gets decided by a local `OpenProcess`. If someone else's live daemon is read as stale
///    and its portfile is deleted, that daemon does not publish again, so **the original owner can no longer reconnect**.
///    A local pid check is not evidence of death in someone else's folder.
///
/// So this function **only reads**. The invariant is that even a wrong decision does not break someone else's file.
///
/// `pre_spawn` = a gate that runs only just before the spawn (the pre-check spot of ADR-0134 decision 4).
///
/// ★The attach point is the contract★: the path that attaches to a live daemon needs **only reading**, so it does not pass this gate
/// — failing to attach to an already well-running daemon just because the folder briefly became unwritable would be the worse failure.
#[allow(clippy::too_many_arguments)]
fn ensure_with(
    reader: &dyn DaemonReader,
    spawner: &dyn Spawner,
    liveness: &dyn PidLiveness,
    clock: &dyn Clock,
    exe: &Path,
    pre_spawn: &mut dyn FnMut() -> Result<(), DiscoveryError>,
    timeout: Duration,
) -> Result<DaemonInfo, DiscoveryError> {
    // Safety net: keep the old DaemonInfo that was decided dead in memory. If polling times out (= the new daemon did not
    // come up = possibly the existing daemon was actually alive because of a single-instance lock conflict), re-check whether this old info is still
    // live and, if live, return it as a recovery — self-repair when our decision was wrong. A broken file's content
    // cannot be trusted so it is not kept (None).
    let mut dead_candidate: Option<DaemonInfo> = None;

    // (a) Check the existing file — **read-only**.
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => return Ok(info),
            AcceptCheck::DeadPid => {
                tracing::warn!(
                    pid = info.pid,
                    daemon_version = info.protocol_version,
                    expected = PROTOCOL_VERSION,
                    "{DAEMON_FILE} 의 pid 가 살아있지 않음 — 그 기록을 stale 로 판정"
                );
                dead_candidate = Some(info);
            }
            AcceptCheck::VersionMismatch { daemon } => {
                // Do not cover a live daemon with a spawn; fail clearly (the restart policy is decided by the phase4
                // DaemonClient).
                return Err(DiscoveryError::VersionMismatch {
                    daemon,
                    expected: PROTOCOL_VERSION,
                });
            }
        },
        Ok(None) => {}
        // A broken file is not deleted either — it may be mid-write, and the winning daemon overwrites it anyway.
        Err(DiscoveryError::Parse(_)) => {}
        // ★Do not treat an io failure more harshly than a parse failure★: if a third party (antivirus · indexer) briefly opens the file with a narrow share,
        //   the read here fails with 32, while the daemon side retries the same condition 5 times/400ms. If we hard-fail here,
        //   ensure collapses wholesale **with a perfectly good daemon in place**. Report it as "not read yet" and let it flow into the
        //   spawn+polling below — if one is already up polling finds that daemon, and if not a new one comes up.
        Err(e) => {
            tracing::warn!("{DAEMON_FILE} 초기 읽기 실패 — 아직 못 읽은 것으로 보고 계속: {e}");
        }
    }

    // (b) spawn — one gate before it.
    pre_spawn()?;
    tracing::info!(exe = %exe.display(), "데몬 spawn");
    spawner.spawn(exe)?;

    // (c) polling — wait for a new daemon.json until the timeout.
    let deadline = clock.now() + timeout;
    // ★Hold "the last thing seen", not "could not read"★: if only read failures were kept here, when a timeout happens with a
    //   broken file that never gets fixed or a rejected file (dead pid · version mismatch), the diagnosis below says "the file never
    //   appeared", **which is wrong** — the only line the user can see points to the wrong place.
    let mut last_reason: Option<String> = None;
    loop {
        match reader.read() {
            Ok(Some(info)) => match check_acceptable(&info, liveness) {
                AcceptCheck::Accept => return Ok(info),
                AcceptCheck::DeadPid => {
                    last_reason = Some(format!(
                        "{DAEMON_FILE} 은 있으나 적힌 pid {} 가 살아있지 않음",
                        info.pid
                    ));
                }
                AcceptCheck::VersionMismatch { daemon } => {
                    last_reason = Some(format!(
                        "{DAEMON_FILE} 의 프로토콜 버전 {daemon} ≠ 이 클라이언트 {PROTOCOL_VERSION}"
                    ));
                }
            },
            Ok(None) => {}
            // It may be a partial file mid-write so keep looping — but a **broken file that never gets fixed** has the same
            //   shape, so carry the reason along (comment above).
            Err(e @ DiscoveryError::Parse(_)) => {
                last_reason = Some(format!("{DAEMON_FILE} 파싱 실패: {e}"));
            }
            // ★Do not abort inside the loop★: one transient open failure (a third party briefly opens it with a narrow share
            //   — the daemon side retries the same condition) would throw away **all of the remaining wait**. Reading again at the next
            //   tick is enough, and if it truly cannot be read it ends in the timeout anyway.
            // ★Do not raise to the default level every tick★: polling would flood the log with the same line. Hold only the last
            //   reason and emit it once at the timeout below — that one is the only failure the user actually sees.
            Err(e) => {
                tracing::debug!("{DAEMON_FILE} 폴링 읽기 실패 — 다음 tick 에 재시도: {e}");
                last_reason = Some(format!("{DAEMON_FILE} 읽기 실패: {e}"));
            }
        }
        if clock.now() >= deadline {
            if let Some(old) = dead_candidate.take() {
                let old_live = !liveness.is_dead(old.pid, old.start_time);
                if old_live && old.protocol_version == PROTOCOL_VERSION {
                    tracing::warn!(
                        pid = old.pid,
                        "dead 로 판정했던 daemon.json 이 폴링 timeout 시점엔 live — 그 데몬으로 복구"
                    );
                    return Ok(old);
                } else if old_live {
                    // ★Do not let it flow into a cause-less timeout★: the daemon we wrongly saw as dead is actually
                    //   alive with only a different version, so if the protocol_version in hand is thrown away and a Timeout is
                    //   emitted, the only line the user sees cannot point at the cause. In a portable
                    //   folder on a network share, the pid of **someone else's computer** is measured by a local OpenProcess, so a live daemon is
                    //   always read as dead and this path becomes the default.
                    tracing::warn!(
                        pid = old.pid,
                        daemon_version = old.protocol_version,
                        expected = PROTOCOL_VERSION,
                        "dead 로 판정했던 daemon.json 이 timeout 시점엔 live — 단 프로토콜 버전이 다름"
                    );
                    return Err(DiscoveryError::VersionMismatch {
                        daemon: old.protocol_version,
                        expected: PROTOCOL_VERSION,
                    });
                }
            }
            // ★It must remain at the default level (warn)★: this timeout is the very failure the user sees as a banner.
            //   If it is silent here, not one line of cause is left in the release log file, and the moment the banner is closed the incident becomes
            //   untraceable (the very reason the file log was made).
            tracing::warn!(
                ?timeout,
                last_reason = last_reason
                    .as_deref()
                    .unwrap_or("(없음 — daemon.json 이 끝내 생기지 않음)"),
                "데몬 기동 대기 시간 초과"
            );
            return Err(DiscoveryError::Timeout(timeout));
        }
        clock.sleep(POLL_INTERVAL);
    }
}

// ── daemon lifecycle status/stop (ADR-0021 §5 command surface) ──────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    /// Was a live daemon discovered (file exists + compatible version + PID live).
    pub alive: bool,
    /// Discovered daemon PID (if the file exists, reported even if dead). None if absent.
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

fn status_with(reader: &dyn DaemonReader, liveness: &dyn PidLiveness) -> DaemonStatus {
    match reader.read() {
        Ok(Some(info)) => {
            let alive = matches!(check_acceptable(&info, liveness), AcceptCheck::Accept);
            DaemonStatus {
                alive,
                pid: Some(info.pid),
                port: Some(info.port),
            }
        }
        // A broken file is untrustworthy so the pid is not reported.
        _ => DaemonStatus {
            alive: false,
            pid: None,
            port: None,
        },
    }
}

pub fn daemon_status(data_dir: &Path) -> DaemonStatus {
    let reader = FileReader {
        path: DataLayout::new(data_dir).daemon_file(),
    };
    status_with(&reader, &RealLiveness)
}

/// ★How it differs from daemon_status★: daemon_status is a lifecycle probe giving pid/port only. This returns the
/// full record (the token included) — see [`read_live_daemon`] for who receives it.
fn read_live_with(reader: &dyn DaemonReader, liveness: &dyn PidLiveness) -> Option<DaemonInfo> {
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => Some(info),
            AcceptCheck::DeadPid | AcceptCheck::VersionMismatch { .. } => None,
        },
        _ => None,
    }
}

/// Reads the live daemon's record (the token included) from `daemon.json` (real entry point).
///
/// ★How the direct-path reconnect follows a moved daemon (ADR-0021)★: after a hot swap (daemon_stop → start) or a
/// crash respawn the daemon comes up on a new port; the dev `wsTransport` reconnect re-reads the **current
/// `daemon.json`** through this (via the `read_daemon_info` command, its only caller, which strips the token before it
/// reaches the WebView — ADR-0310 D23 ①) instead of a cached address. The shell's own link reads `daemon.json` itself
/// (`daemon_client/link/endpoint.rs`). ★It never spawns★ — it follows a daemon that is up, it does not wake one.
pub fn read_live_daemon(data_dir: &Path) -> Option<DaemonInfo> {
    let reader = FileReader {
        path: DataLayout::new(data_dir).daemon_file(),
    };
    read_live_with(&reader, &RealLiveness)
}

pub trait ProcessKiller {
    /// Success is best-effort (treated as Ok if already dead).
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError>;
}

// ADR-0024: the pid kill behind the `daemon_stop` command. No frontend calls the command; U5b-2 removes it (TRD S21
//   trd-3-2-u5 §3-5). The shell's stop path does not use it: its kill is handle-pinned (`daemon_client::shutdown` ·
//   `platform::process::terminate_if_same`).
//
/// Daemon pid kill (real entry point).
///
/// It kills the pid in daemon.json directly, with no graceful `StopDaemon` first — the graceful stop is the shell's
/// stop path (see the note above). The daemon holds its children in a KILL_ON_JOB_CLOSE Job, so when the daemon
/// process dies the child PTYs are cleaned up with it (no detach possible, same as the connection_core StopDaemon comment).
///
/// Returns: Ok(Some(pid))=the pid a kill was attempted on, Ok(None)=no daemon to kill (no file/already dead).
pub fn daemon_stop(data_dir: &Path) -> Result<Option<u32>, DiscoveryError> {
    stop_with(
        &FileReader {
            path: DataLayout::new(data_dir).daemon_file(),
        },
        &RealLiveness,
        &TaskKiller,
    )
}

fn stop_with(
    reader: &dyn DaemonReader,
    liveness: &dyn PidLiveness,
    killer: &dyn ProcessKiller,
) -> Result<Option<u32>, DiscoveryError> {
    match reader.read() {
        Ok(Some(info)) => {
            if liveness.is_dead(info.pid, info.start_time) {
                return Ok(None);
            }
            killer.kill(info.pid)?;
            Ok(Some(info.pid))
        }
        _ => Ok(None),
    }
}

struct TaskKiller;

// ADR-0266
impl ProcessKiller for TaskKiller {
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError> {
        // Stops down to the child tree — overlaps the daemon Job safety net but is harmless.
        engram_dashboard_platform::process::kill_tree(pid).map_err(|e| {
            DiscoveryError::Io(if e.kind() == std::io::ErrorKind::Unsupported {
                "daemon_stop 은 Windows 전용".into()
            } else {
                e.to_string()
            })
        })
    }
}

// ── public entry points ─────────────────────────────────────────────────────────

/// `data_dir` = the daemon.json directory (default_data_dir() shared by app and daemon).
/// `daemon_exe` = path of the daemon executable (made absolute internally with dunce::canonicalize).
pub fn ensure_daemon(
    data_dir: &Path,
    daemon_exe: &Path,
    timeout: Duration,
    console: bool,
) -> Result<DaemonInfo, DiscoveryError> {
    let daemon_path = DataLayout::new(data_dir).daemon_file();

    let exe_abs = dunce::canonicalize(daemon_exe)
        .map_err(|e| DiscoveryError::ExeNotFound(format!("{}: {e}", daemon_exe.display())))?;

    let reader = FileReader {
        path: daemon_path.clone(),
    };
    let spawner = WmiSpawner { console };
    let liveness = RealLiveness;
    let clock = engram_dashboard_base::time::SystemClock;
    // ADR-0134 decision 4: check the data folder on our side **before** launching the daemon. If the folder is unwritable the
    // daemon dies as soon as it starts and the client is left with only a cause-less connection timeout, so the cause is attached here
    // and raised as is into the existing failure path (command Err / DaemonClient Err).
    //
    // ★If the override is on, do not assert (load-bearing)★: a daemon launched via WMI **does not inherit the parent env**
    // (the override comment at the top of this file), so it cannot see `ENGRAM_DATA_DIR` — the folder we look at and the folder
    // the daemon will write differ. Checking our folder in that state would declare pass/fail **about the wrong folder**,
    // so saying nothing is right (it stays the existing behavior of a polling timeout).
    let mut pre_spawn = || {
        if data_dir_env_override().is_some() {
            tracing::debug!(
                "ENGRAM_DATA_DIR 설정됨 — WMI 데몬은 이 값을 상속하지 않으므로 데이터 폴더 사전 점검을 건너뛴다"
            );
            return Ok(());
        }
        check_data_dir_writable(data_dir)
    };

    ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        &exe_abs,
        &mut pre_spawn,
        timeout,
    )
}

/// Search for the daemon exe path. First the same directory as current_exe (side by side when deployed),
/// else the development target/debug fallback. ExeNotFound if not found.
pub fn locate_daemon_exe() -> Result<PathBuf, DiscoveryError> {
    const DAEMON_EXE_STEM: &str = "engram-dashboard-daemon";
    let exe = engram_dashboard_platform::env::exe_file_name(DAEMON_EXE_STEM);

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(sibling) = engram_dashboard_platform::env::sibling_exe(DAEMON_EXE_STEM) {
        candidates.push(sibling);
    }
    // In a workspace build target/debug is shared so the candidates above are enough, but one more to be safe.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join(&exe));
        candidates.push(cwd.join("..").join("target").join("debug").join(&exe));
    }

    locate_in(&candidates)
}

fn locate_in(candidates: &[PathBuf]) -> Result<PathBuf, DiscoveryError> {
    for c in candidates {
        if c.is_file() {
            return Ok(c.clone());
        }
    }
    Err(DiscoveryError::ExeNotFound(format!(
        "daemon exe 후보 {}개 모두 없음",
        candidates.len()
    )))
}

// ── real 구현 ──────────────────────────────────────────────────────────────────

struct FileReader {
    path: PathBuf,
}

impl DaemonReader for FileReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(DiscoveryError::Io(e.to_string())),
        };
        DaemonInfo::parse(&bytes)
            .map(Some)
            .map_err(|e| DiscoveryError::Parse(e.to_string()))
    }
}

// pub(crate) for daemon_client::link::stop and daemon_client::link::endpoint · ADR-0271
pub(crate) struct RealLiveness;

impl PidLiveness for RealLiveness {
    fn is_dead(&self, pid: u32, start_time: u64) -> bool {
        !engram_dashboard_platform::process::pid_alive_with_start_time(pid, start_time)
    }
}

impl Clock for engram_dashboard_base::time::SystemClock {
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

// ── launching the daemon (real) ──────────────────────────────────────────────────────────────

/// Launches the daemon outside the Job the shell holds (platform `spawn::spawn_outside_job` — WMI). ★Why outside the Job★: even if the shell (Tauri) is
/// inside a `KILL_ON_JOB_CLOSE` Job the daemon must survive. That mechanism cannot pass environment variables, so the token flows only through
/// daemon.json (ACL) (design settled), and the pid · port are also retrieved by polling that file. The exe it receives is an absolute path
/// ([`ensure_daemon`] canonicalizes it).
// ADR-0021
// ADR-0266
struct WmiSpawner {
    /// true = launch with a new console window (to see debug logs) · false = default. ★Even with false the debug daemon gets a console window★ — a debug build is a console app,
    /// and only the release `windows_subsystem` removes the window.
    console: bool,
}

impl Spawner for WmiSpawner {
    fn spawn(&self, exe: &Path) -> Result<(), DiscoveryError> {
        use engram_dashboard_platform::spawn::{spawn_outside_job, DetachedSpawnError};
        spawn_outside_job(exe, self.console).map_err(|e| match e {
            DetachedSpawnError::Refused { rv } => DiscoveryError::SpawnFailed { rv },
            DetachedSpawnError::Io(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                DiscoveryError::Io("WMI spawn 은 Windows 전용".into())
            }
            DetachedSpawnError::Io(e) => DiscoveryError::Io(e.to_string()),
        })
    }
}
