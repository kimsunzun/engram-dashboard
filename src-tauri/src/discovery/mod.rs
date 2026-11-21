//! Daemon discovery — the shell finds the daemon and, if there is none, launches it via WMI and retrieves its port from `daemon.json`. Includes status query · stop fallback.
//!
//! ADR-0029: daemon-only (embedded removed). The app is a resident client of the daemon so it always attaches to the daemon (WS is phase4).
//!
//! ## Design — pure logic separated from the OS/WMI boundary
//! Side effects are injected as traits, so unit tests do not depend on the OS, WMI or real time. [`ensure_daemon`] is pure orchestration over these traits, so every branch is unit-testable without a real WMI spawn or sleep; the real spawn is an `#[ignore]` test.
//!
//! The real implementations of the traits (PID decision · WMI launch · process tree stop) merely call functions of the OS-layer crate `platform` —
//! this module's production code has no OS branches (ADR-0266).
//!
//! The data folder root, the `daemon.json` location and `logs\` are copies of the daemon's authority ([`layout`]). The shell's stop path is [`crate::daemon_client::shutdown`].
//!
//! ## Security
//! `daemon.json` carries no token (ADR-0395): the shell token is in this account's owner-only `daemon-token.<account>` ([`read_token`]),
//! read after `daemon.json` so a new record is never paired with an old token, and never printed to logs. The record's owner account is
//! only a hint — the shell checks the listener's account on its own connection before sending the token (`daemon_client::link::account`).
//! A data folder on a drive that cannot keep owner-only files (FAT / exFAT) is refused before any spawn ([`check_data_dir_volume`],
//! ADR-0399): the WMI-launched daemon refuses it too, but its refusal reaches the shell only as a discovery timeout.
// ADR-0271

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_base::writable::{probe_write_in, retry_if_vanished};
use engram_dashboard_protocol::{DaemonInfo, Secret, PROTOCOL_VERSION};

pub mod layout;
#[cfg(test)]
pub(crate) mod tests;

pub use layout::{default_data_dir, DataLayout};

use layout::{data_dir_env_override, DAEMON_FILE};

/// The lock held by every shell lib test that changes `ENGRAM_DATA_DIR` or picks a path from its value. The environment is process-global, so two locks would not block each other.
// ADR-0282
#[cfg(test)]
pub(crate) static DATA_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ★The ENGRAM_DATA_DIR override does not reach the WMI path★ (authority for the override rule = the daemon `data_dir`): it takes effect only on spawns that inherit the parent env (tests launching the daemon directly with `std::process::Command`, e.g. the daemon's `tests/ws_e2e.rs`). This module's production spawn (WMI Win32_Process.Create) makes a child of WmiPrvSE that does not inherit the parent env, and there is no other channel than daemon.json/ACL. So the smoke tests that really go through WMI (real_wmi_spawn_*) cannot be isolated by env; they poll the default path (`.engram-dev`) and protect the production files with backup/restore.

fn unwritable(dir: &Path, e: &std::io::Error) -> DiscoveryError {
    DiscoveryError::DataDirUnwritable {
        path: dir.display().to_string(),
        reason: e.to_string(),
    }
}

/// Checks whether the data folder can be written **without creating anything** (client pre-check).
///
/// ★Do not leave a folder behind★: the check asks "can the daemon write this folder"; an empty folder left on a path the daemon ends up not using would stay forever.
///
/// - `dir` is already a folder: write a probe inside it.
/// - `dir` is a **file**: fail (`create_dir_all` is certain to fail; do not pass by looking at the parent).
/// - `dir` does not exist: **actually try creating it**, write the probe, then undo what we created.
///
/// ★Do not substitute a file-create test in the parent folder (do not revive)★: Windows `FILE_ADD_FILE` and `FILE_ADD_SUBDIRECTORY` are granted separately; where only files can be created that check passes, the daemon's `create_dir_all` fails, and the user sees a timeout instead of a message.
///
/// ★The undo covers all that we created and only that★: create one layer at a time from top to bottom, record only the layers where **we won** the creation, undo from the leaf upward, also on the failure path. ★Do not undo from a "snapshot before creating" (do not revive)★: A records `a/` as absent, B creates `a/`, and A later deletes **B's folder**; the side that saw `AlreadyExists` must decide it is someone else's.
///
/// Known limit: what is deleted is the **path** we created, not the directory object (ABA: someone deletes and recreates it under the same name, then our `remove_dir` deletes theirs). Accepted because `remove_dir` deletes only empty directories, and that victim's probe meets `NotFound` and recovers through [`retry_if_vanished`].
///
/// ★Overlapping calls are real★: the tray "turn daemon on" and the boot ensure are not serialized (`src-tauri/src/commands/discovery.rs`), and since this check undoes its own folder, another's probe can trip with `NotFound` — hence [`retry_if_vanished`].
pub fn check_data_dir_writable(dir: &Path) -> Result<(), DiscoveryError> {
    retry_if_vanished(|| check_data_dir_writable_once(dir)).map_err(|e| unwritable(dir, &e))
}

fn check_data_dir_writable_once(dir: &Path) -> std::io::Result<()> {
    // ★One stat decides three branches (do not revive — do not follow `is_dir()` with `exists()`)★: asked twice, a folder created by someone else in between reads as "not a folder + exists" = **a file is there**, and a fine race is reported as "a file with the same name already exists" (reproduced by the overlapping-check test).
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => return probe_write_in(dir),
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "a file with the same name already exists, so the folder cannot be created",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    // Collect the non-existent ancestors from the leaf upward; stop at the first existing one (create_dir on a drive root is PermissionDenied, not AlreadyExists).
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

/// Refuses a data folder whose drive does not keep file ACLs (FAT / exFAT), and one whose drive's properties cannot be read — the
///   token file and the agent control files could not be made owner-only there (no opt-in). `dir` must exist: the volume is read
///   through it.
// ADR-0399: unreadable flags refuse too — reading them as "has ACLs" would fail open on exactly the drives this guards.
pub fn check_data_dir_volume(dir: &Path, volume: &dyn VolumeAcls) -> Result<(), DiscoveryError> {
    match volume.stores_acls(dir) {
        Ok(true) => Ok(()),
        Ok(false) => Err(DiscoveryError::UnprotectedVolume {
            path: dir.display().to_string(),
        }),
        Err(e) => Err(DiscoveryError::VolumeUnreadable {
            path: dir.display().to_string(),
            reason: e.to_string(),
        }),
    }
}

/// [`ensure_daemon`]'s gate just before a spawn: the data folder must be writable (ADR-0134 decision 4) and on a drive that keeps file
///   ACLs (ADR-0399). `env_override` = `ENGRAM_DATA_DIR` is set — then nothing is checked (why: [`ensure_daemon`]).
pub(crate) fn pre_spawn_checks(
    data_dir: &Path,
    env_override: bool,
    volume: &dyn VolumeAcls,
) -> Result<(), DiscoveryError> {
    if env_override {
        tracing::debug!(
            "ENGRAM_DATA_DIR is set — the WMI daemon does not inherit this value, so the data folder pre-checks are skipped"
        );
        return Ok(());
    }
    check_data_dir_writable(data_dir)?;
    check_data_dir_volume(data_dir, volume)
}

// ── Errors ───────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("daemon exe not found: {0}")]
    ExeNotFound(String),
    #[error("failed to parse daemon.json: {0}")]
    Parse(String),
    #[error("daemon spawn failed (WMI ReturnValue={rv})")]
    SpawnFailed { rv: u32 },
    #[error("timed out waiting for the daemon to start (over {0:?})")]
    Timeout(Duration),
    #[error("protocol version mismatch: daemon={daemon}, expected={expected}")]
    VersionMismatch { daemon: u32, expected: u32 },
    /// ★The message carries the remedy (ADR-0134 decision 4)★: this string rises as is to the frontend's failure surface; replacing a cause-less connection timeout is why this variant exists.
    #[error("cannot write to the data folder ({path}): {reason} — please extract to a writable location")]
    DataDirUnwritable { path: String, reason: String },
    /// The log text; the band shows its own translated text from [`VolumeRefusal`] (ADR-0399 · ADR-0373).
    #[error("the data folder ({path}) is on a drive that does not store file permissions (FAT or exFAT), so Engram's private files cannot be protected — exit Engram, move the Engram folder (for a portable copy, the whole folder with the exe) to an NTFS drive and start it again")]
    UnprotectedVolume { path: String },
    /// The log text; the band shows its own translated text from [`VolumeRefusal`] (ADR-0399 · ADR-0373).
    #[error("could not read the properties of the drive holding the data folder ({path}): {reason} — exit Engram, move the Engram folder (for a portable copy, the whole folder with the exe) to an NTFS drive and start it again")]
    VolumeUnreadable { path: String, reason: String },
    #[error("io: {0}")]
    Io(String),
}

impl DiscoveryError {
    /// The typed band cause this failure is, if it is one (ADR-0399).
    pub fn volume_refusal(&self) -> Option<VolumeRefusal> {
        match self {
            DiscoveryError::UnprotectedVolume { .. } => Some(VolumeRefusal::Unprotected),
            DiscoveryError::VolumeUnreadable { .. } => Some(VolumeRefusal::Unreadable),
            _ => None,
        }
    }
}

/// Why the data folder's drive refused Engram before a spawn (ADR-0399) — the typed cause the shell band shows, translated where shown
///   (ADR-0372 · ADR-0373).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VolumeRefusal {
    /// The drive does not store file ACLs (FAT, exFAT).
    Unprotected,
    /// Whether the drive stores file ACLs could not be read.
    Unreadable,
}

/// A failed daemon search as its callers carry it — `daemon_start`'s rejection and `EndpointSource::ensure`'s error. `refusal` = the
///   typed band cause when the failure is one (ADR-0399), `null` otherwise; `message` = the English text for the log, and band ①'s cause
///   when there is no `refusal`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct StartFailure {
    pub refusal: Option<VolumeRefusal>,
    pub message: String,
}

impl StartFailure {
    /// A failure that is no typed refusal.
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            refusal: None,
            message: message.into(),
        }
    }
}

impl From<DiscoveryError> for StartFailure {
    fn from(e: DiscoveryError) -> Self {
        Self {
            refusal: e.volume_refusal(),
            message: e.to_string(),
        }
    }
}

impl std::fmt::Display for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

// ── injection boundary (trait) ─────────────────────────────────────────────────────────────

/// Takes start_time too, to tell PID reuse apart (M2): alive only when "PID alive AND creation time == recorded value". start_time == 0 (unknown, old daemon.json) falls back to PID alone.
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

/// Whether a folder's drive keeps file ACLs — `Ok` only when that was read (ADR-0399). Real = [`PlatformVolume`].
pub trait VolumeAcls {
    fn stores_acls(&self, dir: &Path) -> io::Result<bool>;
}

/// Polling clock: the current time is the base clock plus a sleep that **blocks the thread** (this module is synchronous).
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

// ADR-0196: the liveness check comes **before** the version comparison — do not flip it. The decision **deliberately ignores the version direction**: a dead record is stale whether lower or higher than ours, so do not add "back off if the record is newer".
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
/// Deleting a file decided stale or broken before the spawn gains nothing and does damage:
/// - It gains nothing: polling accepts only "valid parse + live pid + compatible version", so an old file with a dead pid is filtered by `check_acceptable`, and the winning daemon overwrites its own daemon.json **after getting the lock**.
/// - Damage (i): if the deletion comes before the gate (`pre_spawn`) and the gate blocks, only an irreversible deletion remains.
/// - Damage (ii): in a portable folder on a network share (supported by ADR-0134 decision 3) the pid of **another computer** is decided by a local `OpenProcess`; deleting the portfile of someone else's live daemon read as stale means it does not publish again and **the original owner can no longer reconnect**.
///
/// So this function **only reads**; even a wrong decision does not break someone else's file.
///
/// `pre_spawn` = a gate that runs only just before the spawn (the pre-check spot of ADR-0134 decision 4). ★The attach path does not pass it★: attaching to a running daemon needs only reading, and failing because the folder briefly became unwritable would be the worse failure.
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
    // Safety net: keep the old DaemonInfo decided dead. If polling times out (possibly the existing daemon was alive, e.g. a single-instance lock conflict), re-check it and return it if live. A broken file's content cannot be trusted, so it is not kept (None).
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
                    "the pid in {DAEMON_FILE} is not alive — that record is judged stale"
                );
                dead_candidate = Some(info);
            }
            AcceptCheck::VersionMismatch { daemon } => {
                // Do not cover a live daemon with a spawn; fail clearly (restart policy is the DaemonClient's).
                return Err(DiscoveryError::VersionMismatch {
                    daemon,
                    expected: PROTOCOL_VERSION,
                });
            }
        },
        Ok(None) => {}
        // A broken file is not deleted either — it may be mid-write, and the winning daemon overwrites it anyway.
        Err(DiscoveryError::Parse(_)) => {}
        // ★Do not treat an io failure more harshly than a parse failure★: a third party (antivirus, indexer) briefly opening the file with a narrow share makes the read fail with 32, while the daemon retries the same condition 5 times/400ms. Hard-failing here would collapse ensure **with a perfectly good daemon in place**. Report "not read yet" and flow into spawn + polling: an already-up daemon is found by polling, otherwise a new one comes up.
        Err(e) => {
            tracing::warn!("initial read of {DAEMON_FILE} failed — treating it as not read yet and continuing: {e}");
        }
    }

    // (b) spawn — one gate before it.
    pre_spawn()?;
    tracing::info!(exe = %exe.display(), "spawning the daemon");
    spawner.spawn(exe)?;

    // (c) polling — wait for a new daemon.json until the timeout.
    let deadline = clock.now() + timeout;
    // ★Hold "the last thing seen", not "could not read"★: keeping only read failures would make a timeout with a never-fixed broken file or a rejected file (dead pid, version mismatch) say "the file never appeared", **which is wrong** — the only line the user sees would point to the wrong place.
    let mut last_reason: Option<String> = None;
    loop {
        match reader.read() {
            Ok(Some(info)) => match check_acceptable(&info, liveness) {
                AcceptCheck::Accept => return Ok(info),
                AcceptCheck::DeadPid => {
                    last_reason = Some(format!(
                        "{DAEMON_FILE} exists but its recorded pid {} is not alive",
                        info.pid
                    ));
                }
                AcceptCheck::VersionMismatch { daemon } => {
                    last_reason = Some(format!(
                        "protocol version {daemon} in {DAEMON_FILE} ≠ this client's {PROTOCOL_VERSION}"
                    ));
                }
            },
            Ok(None) => {}
            // May be a partial file mid-write, so keep looping; a never-fixed broken file looks the same, so carry the reason along.
            Err(e @ DiscoveryError::Parse(_)) => {
                last_reason = Some(format!("failed to parse {DAEMON_FILE}: {e}"));
            }
            // ★Do not abort inside the loop★: one transient open failure (third party, narrow share) would throw away **all of the remaining wait**; reading again at the next tick is enough, and a truly unreadable file ends in the timeout.
            // ★Do not log at the default level every tick★: polling would flood the log. Hold only the last reason and emit it once at the timeout.
            Err(e) => {
                tracing::debug!(
                    "polling read of {DAEMON_FILE} failed — retrying on the next tick: {e}"
                );
                last_reason = Some(format!("failed to read {DAEMON_FILE}: {e}"));
            }
        }
        if clock.now() >= deadline {
            if let Some(old) = dead_candidate.take() {
                let old_live = !liveness.is_dead(old.pid, old.start_time);
                if old_live && old.protocol_version == PROTOCOL_VERSION {
                    tracing::warn!(
                        pid = old.pid,
                        "the daemon.json judged dead is live at the polling timeout — recovering with that daemon"
                    );
                    return Ok(old);
                } else if old_live {
                    // ★Do not flow into a cause-less timeout★: the daemon wrongly seen as dead is alive with a different version, so discarding the protocol_version in hand leaves the user's only line without the cause. In a portable folder on a network share, **another computer's** pid is measured by a local OpenProcess, so a live daemon is always read as dead and this path becomes the default.
                    tracing::warn!(
                        pid = old.pid,
                        daemon_version = old.protocol_version,
                        expected = PROTOCOL_VERSION,
                        "the daemon.json judged dead is live at the timeout — but its protocol version differs"
                    );
                    return Err(DiscoveryError::VersionMismatch {
                        daemon: old.protocol_version,
                        expected: PROTOCOL_VERSION,
                    });
                }
            }
            // ★Must stay at the default level (warn)★: this timeout is the failure the user sees as a banner; if silent, the release log file has no cause line and the incident is untraceable once the banner is closed.
            tracing::warn!(
                ?timeout,
                last_reason = last_reason
                    .as_deref()
                    .unwrap_or("(none — daemon.json never appeared)"),
                "timed out waiting for the daemon to start"
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

/// ★Differs from daemon_status★: daemon_status is a lifecycle probe giving pid/port only; this returns the full record — see [`read_live_daemon`] for who receives it.
fn read_live_with(reader: &dyn DaemonReader, liveness: &dyn PidLiveness) -> Option<DaemonInfo> {
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => Some(info),
            AcceptCheck::DeadPid | AcceptCheck::VersionMismatch { .. } => None,
        },
        _ => None,
    }
}

/// Reads the live daemon's record from `daemon.json` (real entry point). The record carries no token (ADR-0395).
///
/// ★How the direct-path reconnect follows a moved daemon (ADR-0021)★: after a hot swap (stop → start) or a
/// crash respawn the daemon comes up on a new port; the dev `wsTransport` reconnect re-reads the **current
/// `daemon.json`** through this (via the `read_daemon_info` command, its only caller — the WebView never gets a token,
/// ADR-0310 D23 ①) instead of a cached address. The shell's own link reads `daemon.json` and the token file itself
/// (`daemon_client/link/endpoint.rs`). ★It never spawns★ — it follows a daemon that is up, it does not wake one.
pub fn read_live_daemon(data_dir: &Path) -> Option<DaemonInfo> {
    let reader = FileReader {
        path: DataLayout::new(data_dir).daemon_file(),
    };
    read_live_with(&reader, &RealLiveness)
}

/// This account's shell token from `data_dir` (`daemon-token.<account>` next to `daemon.json`; `account` = platform
///   `account::current_account_id`). ★Read it after `daemon.json`, never before★ — the daemon writes the token file before it publishes
///   the record, so a token read after the record is never older than it (ADR-0395). The content is sent as it is; the file is the
///   daemon's, and a wrong one is the daemon's refusal to make. A file held briefly (antivirus) is read again within the shell's lock
///   budget (`file_hooks::OS_HOOKS`), as other data files are — otherwise a briefly held token turns a graceful stop into a kill.
// ADR-0395
pub(crate) fn read_token(data_dir: &Path, account: &str) -> std::io::Result<Secret> {
    let path = DataLayout::new(data_dir).token_file(account);
    let mut text = None;
    (crate::file_hooks::OS_HOOKS.retry)(&mut || {
        text = Some(std::fs::read_to_string(&path)?);
        Ok(())
    })?;
    text.map(Secret::new)
        .ok_or_else(|| std::io::Error::other("the retry never called the read"))
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
    // ADR-0134 decision 4 · ADR-0399: check the data folder on our side **before** launching the daemon — writable, and on a drive
    // that keeps file ACLs. Otherwise the daemon dies as soon as it starts and the client is left with only a cause-less connection
    // timeout, so the cause is attached here and raised as is into the existing failure path (command Err / DaemonClient Err).
    //
    // ★If the override is on, do not assert (load-bearing)★: a daemon launched via WMI **does not inherit the parent env**
    // (the override comment at the top of this file), so it cannot see `ENGRAM_DATA_DIR` — the folder we look at and the folder
    // the daemon will write differ. Checking our folder in that state would declare pass/fail **about the wrong folder**,
    // so saying nothing is right (it stays the existing behavior of a polling timeout).
    let mut pre_spawn =
        || pre_spawn_checks(data_dir, data_dir_env_override().is_some(), &PlatformVolume);

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
        "none of the {} daemon exe candidates exist",
        candidates.len()
    )))
}

// ── real implementation ──────────────────────────────────────────────────────────────

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

/// The OS layer's volume query (platform `owner_only::volume_stores_acls`).
struct PlatformVolume;

impl VolumeAcls for PlatformVolume {
    fn stores_acls(&self, dir: &Path) -> io::Result<bool> {
        engram_dashboard_platform::owner_only::volume_stores_acls(dir)
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
/// the files the daemon writes — the owner-only token file ([`read_token`], ADR-0395) — and the pid · port are retrieved by polling
/// `daemon.json`. The exe it receives is an absolute path ([`ensure_daemon`] canonicalizes it).
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
                DiscoveryError::Io("WMI spawn is Windows-only".into())
            }
            DetachedSpawnError::Io(e) => DiscoveryError::Io(e.to_string()),
        })
    }
}
