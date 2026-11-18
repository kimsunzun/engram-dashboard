//! tracing global init + a file log for one process run.
//!
//! ADR-0138 — release logs are written to a file **synchronously** (no async writer · the caller passes the folder).
//!
//! Two entry points: [`init_logging`] (stdout only) · [`init_logging_with_file`] (stdout + file); both idempotent, per process the first
//!   call wins.
//!
//! ★The file sink exists because in release stdout goes nowhere★ (measured): the release binary has `windows_subsystem = "windows"` (no
//!   console) and the daemon is launched by WMI `Win32_Process.Create`, which cannot pass std handles, so its stdout/stderr attach to nothing;
//! it does not inherit the parent environment either, so `RUST_LOG` does not reach it. Without a file, **no place keeps the cause of a
//!   startup failure.**
//!
//! ★The caller passes the log folder — this module does not find it itself★: the data folder layout belongs to the daemon `data_dir` and
//!   the shell `discovery` (copy)'s `DataLayout`, both depend on this crate, so calling it from here would make a cycle
//! (leaf crate invariant — lib.rs header, tenancy condition ②). This module owns only the file name convention inside the received folder
//!   (`<kind>-<UTC>-<pid>.log`) and the `%TEMP%` fallback location.
//!
//! ★If that folder cannot be used, it falls back under `%TEMP%` instead of giving up silently★: an unusable data folder is a common reason
//!   the daemon fails to start, and exactly then the reason would go only to stdout (nonexistent in release).
//! This fallback does not touch ADR-0134 decision 4 (no data folder fallback) — what that decision prevents is **state** (registry · lock)
//!   in two places, while what moves here is one state-free sheet of reasons.
//! If both fail there is truly nowhere left; the owner then is the preflight check the client runs before spawn (ADR-0135 — the reason is
//!   shown in the window, not by the daemon).

use std::cell::Cell;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use regex::Regex;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter};

type FilterHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

static RELOAD_HANDLE: OnceLock<FilterHandle> = OnceLock::new();

static BEARER_RE: OnceLock<Regex> = OnceLock::new();
static KEY_RE: OnceLock<Regex> = OnceLock::new();

/// Replaces sensitive values (API keys · Bearer tokens) with `***` before debug log output (T-1).
/// At the default log level (warn) PTY text is not printed,
/// but this prevents a key from being exposed by mistake when debug/trace is enabled.
///
/// ※ AWS Secret Access Key (40-char base64) is not included because the pattern is unidentifiable.
/// ※ Generic api_key=/token= forms are not included because of false-positive risk.
pub fn mask_secrets(s: &str) -> String {
    let bearer =
        BEARER_RE.get_or_init(|| Regex::new(r"Bearer\s+\S{10,}").expect("bearer regex compile"));
    let keys = KEY_RE.get_or_init(|| {
        Regex::new(
            r"(?:sk-(?:proj-)?[A-Za-z0-9_\-]{20,}|AKIA[A-Z0-9]{16}|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_\-]{35})",
        )
        .expect("key regex compile")
    });
    let step1 = bearer.replace_all(s, "Bearer ***");
    keys.replace_all(step1.as_ref(), "***").into_owned()
}

// ── File sink ────────────────────────────────────────────────────────────────────

/// The kind of process that writes a log file — **the file-name prefix and the unit of retention cleanup**.
///
/// ★A closed set, not a free string★: two processes picking the same prefix would share one file (interleaved, corrupted lines on
///   Windows). A new process adds a variant here so it **consciously** picks a different name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// The agent-host daemon (`engram-dashboard-daemon.exe`).
    Daemon,
    /// The desktop client shell (`engram-dashboard.exe`).
    App,
}

impl LogKind {
    fn label(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::App => "app",
        }
    }
}

/// The name of the subfolder under the fallback root where logs collect. The primary folder is passed in whole by the caller.
const LOG_SUBDIR: &str = "logs";

const LOG_EXT: &str = ".log";

/// The number of files kept per kind — a cap **including this run's file** (so only
/// `KEEP_PER_KIND - 1` are kept before creating a new file).
const KEEP_PER_KIND: usize = 10;

/// Where it falls back when the primary folder is unusable = `%TEMP%/<this name>/logs/`. The product name is used as is so it **does not
///   collide in name** with the data folder (distributions falling back into one person's temp folder share
/// a folder, but the pid in file names keeps them apart; only the retention cap is shared).
const FALLBACK_DIR_NAME: &str = "engram-dashboard";

/// The log file actually opened. **Filled only after winning the subscriber install** — anything written before that is swallowed.
static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

/// A file writer used instead of the default [`MakeWriter`] implementation of [`Mutex`]. If the slot it points to is still empty it drops
///   that line — the layer is built at install time and the file is opened **after** that.
///
/// ★It ignores poison and keeps writing★: the standard implementation panics on poison, and this sink's consumers include a panic hook — a
///   poisoned lock would make that hook's log panic again (panic-in-panic → abort) and
/// **lose the reason for the death we most need to keep.** The lock only prevents interleaved lines, so overriding is safe.
///
/// ★If the same thread comes back while holding the writer, that line is not kept in the file★: the panic hook runs **before** unwinding
///   **on the panicking thread** and logs through this sink again; a thread that panicked inside `write_all`
/// would re-lock the same `Mutex` and self-deadlock (a hang — worse than abort). So a [`WRITING`] flag is raised while the writer is held,
///   and a thread with the flag set gets a writer that drops instead of locking (the stdout layer receives it as is).
///
/// ★A panicking thread waits for a lock held by another thread only up to [`PANICKING_LOCK_WAIT`], then drops the line★: a stuck holder
///   would hang the hook before it reaches the end (abort). A write not during a panic waits as is;
/// a line from a destructor during unwinding is also during a panic.
// ADR-0292
struct FileSink(&'static OnceLock<Mutex<File>>);

/// The cap on how long a panicking thread waits for the file lock. The lock is normally held only while one line is written, so in common
///   contention it is enough to keep the line, and the delay it adds to the end (abort) when the holder is stuck stays below what a person
///   can notice.
const PANICKING_LOCK_WAIT: Duration = Duration::from_millis(100);

thread_local! {
    /// Does this thread currently hold the file lock of [`FileSink`]? As a const value without `Drop` it is thought to be readable even
    ///   during thread teardown,
    /// but that is std implementation behavior, not a documented guarantee — if it cannot be read, it is read as 「not holding」.
    static WRITING: Cell<bool> = const { Cell::new(false) };
}

enum FileSinkWriter<'a> {
    Live(Held<'a>),
    /// The file is not yet (or never) opened, or this thread is already writing — quietly dropped. The stdout layer
    /// receives it as is.
    Void,
}

/// File lock + the [`WRITING`] flag raised meanwhile. drop lowers the flag — it is lowered even if dropped by unwinding.
struct Held<'a>(MutexGuard<'a, File>);

impl<'a> Held<'a> {
    fn new(guard: MutexGuard<'a, File>) -> Self {
        let _ = WRITING.try_with(|w| w.set(true));
        Self(guard)
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let _ = WRITING.try_with(|w| w.set(false));
    }
}

impl<'a> MakeWriter<'a> for FileSink {
    type Writer = FileSinkWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        let Some(m) = self.0.get() else {
            return FileSinkWriter::Void;
        };
        if WRITING.try_with(Cell::get).unwrap_or(false) {
            return FileSinkWriter::Void;
        }
        // Does not call the `sync` tenant of the same crate — no references between tenants (lib.rs gate ③ · ADR-0275).
        let guard = if std::thread::panicking() {
            match lock_within(m, PANICKING_LOCK_WAIT) {
                Some(guard) => guard,
                None => return FileSinkWriter::Void,
            }
        } else {
            m.lock().unwrap_or_else(|e| e.into_inner())
        };
        FileSinkWriter::Live(Held::new(guard))
    }
}

/// If the lock is acquired within `limit`, that guard; otherwise `None`. Poison is ignored for the same reason as [`FileSink`] above.
fn lock_within(m: &Mutex<File>, limit: Duration) -> Option<MutexGuard<'_, File>> {
    let deadline = Instant::now() + limit;
    loop {
        match m.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

impl io::Write for FileSinkWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Live(held) => held.0.write(buf),
            Self::Void => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Live(held) => held.0.flush(),
            Self::Void => Ok(()),
        }
    }
}

/// A UTC second-resolution timestamp `YYYYMMDD-HHMMSS`. The digit count is fixed so **lexicographic order = time order**, and retention cleanup
/// relies on that property.
fn utc_stamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// Days elapsed since the Unix epoch → (year, month, day). Howard Hinnant's `civil_from_days` (public domain
/// chrono algorithm) narrowed to the range after 1970.
///
/// ★Hand-written to avoid pulling in a date crate★: this crate's dependencies are not to be increased for this one file name
/// (the timestamp of a log **line** is already printed by tracing-subscriber).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Is this name a run log **of this kind** — only a name that matches exactly the `<kind>-YYYYMMDD-HHMMSS-<pid>.log` grammar
/// is true.
///
/// ★Do not select by prefix match (`starts_with("<kind>-")`) (do not revive it)★: ① a label that is merely the prefix of another (`app` vs
///   `app-updater`) makes one side's cleanup **delete the other kind's files** (labels will keep growing)
/// ② the name is the sort key, so one hand-placed `daemon-note.log` settles at the end of lexicographic order and **pushes real logs out
///   of the cap and deletes them.**
///
/// It does not check calendar validity either (`daemon-20261301-…` passes); the grammar alone blocks the two above, and names we wrote
///   come from [`utc_stamp`].
fn is_run_log_name(name: &str, kind: LogKind) -> bool {
    let Some(rest) = name
        .strip_prefix(kind.label())
        .and_then(|r| r.strip_prefix('-'))
        .and_then(|r| r.strip_suffix(LOG_EXT))
    else {
        return false;
    };
    let mut parts = rest.split('-');
    let (Some(date), Some(time), Some(pid), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    date.len() == 8
        && time.len() == 6
        && !pid.is_empty()
        && [date, time, pid]
            .iter()
            .all(|s| s.bytes().all(|b| b.is_ascii_digit()))
}

/// Deletes old files of the same kind and leaves only `keep`. Returns = **the reasons of the failures it swallowed**.
///
/// A deletion failure does not stop startup — a file held by another process legitimately exists (e.g. a daemon of the same kind on two
///   data folders). But it is not swallowed **silently** (it could fail forever unnoticed):
/// the caller emits it all at once right after the subscriber is installed (it cannot emit here — no subscriber yet).
fn prune_old_logs(dir: &Path, kind: LogKind, keep: usize) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            return vec![format!(
                "could not read the {} listing, skipping retention cleanup: {e}",
                dir.display()
            )]
        }
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_run_log_name(n, kind))
        .collect();
    if names.len() <= keep {
        return Vec::new();
    }
    names.sort_unstable();
    let mut failures = Vec::new();
    for stale in &names[..names.len() - keep] {
        if let Err(e) = std::fs::remove_file(dir.join(stale)) {
            failures.push(format!("could not delete stale log {stale}: {e}"));
        }
    }
    failures
}

/// One set of files this run actually opened + **diagnostics that could not be emitted at that time**.
///
/// `deferred` is the reasons of failures that happened before the subscriber install (retention deletion · header write · fallback).
/// At that point there is nowhere to emit logs, so they are gathered, and the caller emits them all at once right after install.
struct OpenedLog {
    path: PathBuf,
    file: File,
    deferred: Vec<String>,
}

/// Newly opens `<dir>/<kind>-<UTC>-<pid>.log` and returns the handle (`dir` is created if missing). The pid keeps two runs started in the
///   same second from sharing one file.
///
/// Known residual (not fixed): a pid reused within the same UTC second or a backwards clock makes names collide, and append **joins two
///   runs in one file** (the header is printed each time so the boundary is readable);
/// preventing it needs exclusive create + an open retry loop, costing more than it gains.
fn open_run_log(dir: &Path, kind: LogKind) -> io::Result<OpenedLog> {
    std::fs::create_dir_all(dir)?;
    let mut deferred = prune_old_logs(dir, kind, KEEP_PER_KIND.saturating_sub(1));

    let stamp = utc_stamp(SystemTime::now());
    let path = dir.join(format!(
        "{}-{stamp}-{}{LOG_EXT}",
        kind.label(),
        std::process::id()
    ));
    let mut file = File::options().create(true).append(true).open(&path)?;
    if let Err(e) = write_header(&mut file, kind, &stamp) {
        deferred.push(format!("could not write the log header: {e}"));
    }
    Ok(OpenedLog {
        path,
        file,
        deferred,
    })
}

/// Tries [`open_run_log`] in the order primary (caller's folder) → secondary (`%TEMP%`). `Err` = both failed, the value is the two reasons.
///
/// ★The secondary exists because the primary failing is exactly where diagnosis is needed most★ (an unusable data folder makes the daemon
///   give up right after, and the reason would go only to stdout, nonexistent in release);
/// rationale and why it does not infringe ADR-0134 decision 4: this file's header.
fn open_run_log_with_fallback(logs_dir: &Path, kind: LogKind) -> Result<OpenedLog, Vec<String>> {
    let primary = match open_run_log(logs_dir, kind) {
        Ok(opened) => return Ok(opened),
        Err(e) => format!("could not open {}: {e}", logs_dir.display()),
    };

    let fallback_root = std::env::temp_dir().join(FALLBACK_DIR_NAME);
    match open_run_log(&fallback_root.join(LOG_SUBDIR), kind) {
        Ok(mut opened) => {
            opened.deferred.insert(
                0,
                format!("{primary} — logging to {} instead", opened.path.display()),
            );
            Ok(opened)
        }
        Err(e) => Err(vec![
            primary,
            format!(
                "could not open the {} fallback either: {e}",
                fallback_root.join(LOG_SUBDIR).display()
            ),
        ]),
    }
}

/// Stamps the identity of this run on the first line of the file.
///
/// ★A file header, not a log event★: the default level is `warn`, so **a normal startup leaves not a single line** — an empty file that
///   cannot even say which binary from when.
/// Raising the startup notice from `info` → `warn` would break the meaning of levels, so one line is placed outside the event plane.
fn write_header(file: &mut File, kind: LogKind, stamp: &str) -> io::Result<()> {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("(exe path unknown: {e})"));
    writeln!(
        file,
        "==== engram {kind} | {stamp} UTC | pid {pid} | {exe} ====",
        kind = kind.label(),
        pid = std::process::id()
    )
}

// ── Initialization ───────────────────────────────────────────────────────────────

/// Global tracing-subscriber init (stdout only). Call only once at app boot (idempotent — repeated calls are no-ops).
/// Default level: the RUST_LOG environment variable takes priority, otherwise "warn" (release default OFF — almost no output normally).
pub fn init_logging() {
    init_subscriber(false);
}

/// [`init_logging`] + a file sink dedicated to this run inside `logs_dir`. `logs_dir` = the log folder itself (not the data folder — the
///   layout is the caller's `DataLayout::logs_dir`).
///
/// Return value = the log file path this call actually attached. **If the primary folder is unusable it may be a path under `%TEMP%`**
///   (also left as a `warn` just before returning — this file's header). `None` = no file sink: already initialized (quiet) ·
/// someone else installed the subscriber first so the layer was discarded (quiet) · both primary and fallback failed to open (a `warn`
///   right after — in release even that has nowhere to go).
///
/// The level filter and stdout output are the same as [`init_logging`] — the file is **only added**.
///
/// Known residual (not fixed): two threads calling the same kind at once may both pass the first check; the loser is filtered at
///   `try_init`, returns `None` and creates no file, so nothing is corrupted —
/// exact exclusion would need an init-only lock, not worth it for a once-at-boot call.
pub fn init_logging_with_file(logs_dir: &Path, kind: LogKind) -> Option<PathBuf> {
    if RELOAD_HANDLE.get().is_some() {
        return None;
    }

    // ★The file is opened **only after winning** the subscriber install★: in the reverse order a losing call also creates the file and
    //   runs retention cleanup, and an empty file nobody writes **deletes one real log** and permanently takes its place.
    //   The layer is installed pointing at a not-yet-open slot (lines in between are dropped by [`FileSink`]); the slot is filled right below.
    if !init_subscriber(true) {
        return None;
    }

    match open_run_log_with_fallback(logs_dir, kind) {
        Ok(opened) => {
            let path = opened.path;
            // A set failure means another call filled this static slot, but only the one that won try_init can do that, so
            //   it never gets here. Even so, it does not advertise a path without a file.
            if LOG_FILE.set(Mutex::new(opened.file)).is_err() {
                return None;
            }
            // The things that can be emitted only now — the gathered reasons must be emitted after the sink is attached to remain in the file.
            for line in opened.deferred {
                tracing::warn!("{line}");
            }
            Some(path)
        }
        Err(reasons) => {
            // ★The last hole of this feature★: the only place to go is stdout, which release lacks. The owner for this case (neither the
            //   primary nor the temp folder usable) is the data folder preflight the client runs before starting the daemon (ADR-0135);
            //   the user sees the reason in that window.
            for r in reasons {
                tracing::warn!("could not open the file log, logging to stdout only: {r}");
            }
            None
        }
    }
}

/// `true` if the subscriber was actually installed. `with_file` says whether to attach the file layer as **a placeholder only** —
/// the real file comes alive when [`LOG_FILE`] is filled.
fn init_subscriber(with_file: bool) -> bool {
    if RELOAD_HANDLE.get().is_some() {
        return false;
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let (filter_layer, handle) = reload::Layer::new(filter);

    // ★Do not use an async writer (`tracing_appender::non_blocking`) (do not revive it)★: the daemon ends with `std::process::exit(code)`
    //   and skips destructors, so lines the worker thread has not flushed vanish — exactly
    //   **the last line just before a startup failure** this feature exists to catch. `File` has no buffer, so one event line is one write
    //   and is on disk before the process dies.
    // ANSI OFF: a person or LLM reads the file as is (color escapes are noise).
    let file_layer = with_file.then(|| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(FileSink(&LOG_FILE))
    });

    // try_init: ignored if another subscriber is already set (tests etc.)
    let result = tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .try_init();

    if result.is_ok() {
        let _ = RELOAD_HANDLE.set(handle);
    }
    result.is_ok()
}

/// Runtime log level change. Valid values: "trace"|"debug"|"info"|"warn"|"error"|"off".
pub fn set_log_level(level: &str) -> Result<(), String> {
    let handle = RELOAD_HANDLE
        .get()
        .ok_or_else(|| "logging not initialized".to_string())?;

    let filter =
        EnvFilter::try_new(level).map_err(|e| format!("invalid log level \"{level}\": {e}"))?;

    handle
        .reload(filter)
        .map_err(|e| format!("reload failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-logtest-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    // ── Timestamps ──
    #[test]
    fn utc_stamp_is_lexicographically_sortable_utc() {
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101-000000");
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(1_755_000_000)),
            "20250812-120000"
        );
        // Leap day — the 400-year cycle branch of civil_from_days.
        assert_eq!(
            utc_stamp(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "20000229-000000"
        );
        let early = utc_stamp(UNIX_EPOCH + Duration::from_secs(1_755_000_000));
        let late = utc_stamp(UNIX_EPOCH + Duration::from_secs(1_755_000_001));
        assert!(
            early < late,
            "lexical order must be time order: {early} < {late}"
        );
    }

    // ── File creation ──
    #[test]
    fn open_run_log_creates_the_given_dir_and_kind_prefixed_file() {
        let logs_dir = temp_dir("open").join("logs");
        let opened = open_run_log(&logs_dir, LogKind::Daemon).expect("must open");
        let path = opened.path;

        assert!(
            opened.deferred.is_empty(),
            "the normal path must have no diagnostics"
        );
        assert_eq!(
            path.parent().unwrap(),
            logs_dir,
            "directly inside the received folder — no subfolder appended"
        );
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("daemon-"), "kind prefix: {name}");
        assert!(name.ends_with(LOG_EXT), "extension: {name}");
        assert!(
            name.contains(&std::process::id().to_string()),
            "includes pid: {name}"
        );

        // A normal startup prints nothing at warn level, so without the header the file would be entirely empty.
        let head = std::fs::read_to_string(&path).expect("read");
        assert!(head.starts_with("==== engram daemon | "), "header: {head}");
        assert!(
            head.ends_with("====\n"),
            "the header must end as one line: {head}"
        );
    }

    #[test]
    fn two_kinds_never_share_a_file() {
        let data_dir = temp_dir("kinds");
        let daemon = open_run_log(&data_dir, LogKind::Daemon).expect("must open");
        let app = open_run_log(&data_dir, LogKind::App).expect("must open");
        assert_ne!(daemon.path, app.path);
    }

    // The fallback (primary folder unusable → `%TEMP%`) needs a separate process that swaps `%TEMP%`, so it is in an integration test
    // (`tests/logging_fallback.rs`) — doing it here would touch the temp folder that other tests see too.

    // ── Retention ──
    #[test]
    fn prune_keeps_newest_per_kind_and_ignores_other_kinds() {
        let data_dir = temp_dir("prune");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");

        for i in 0..15 {
            std::fs::write(dir.join(format!("daemon-20260101-{i:06}-1{LOG_EXT}")), "x")
                .expect("write");
        }
        std::fs::write(dir.join(format!("app-20260101-000000-1{LOG_EXT}")), "x").expect("write");
        std::fs::write(dir.join("daemon-20260101-000000-1.txt"), "x").expect("write");

        assert!(
            prune_old_logs(&dir, LogKind::Daemon, 4).is_empty(),
            "no deletion failures"
        );

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "app-20260101-000000-1.log".to_string(),
                "daemon-20260101-000000-1.txt".to_string(),
                "daemon-20260101-000011-1.log".to_string(),
                "daemon-20260101-000012-1.log".to_string(),
                "daemon-20260101-000013-1.log".to_string(),
                "daemon-20260101-000014-1.log".to_string(),
            ],
            "must delete only old ones of the same kind (other kinds · other extensions kept)"
        );
    }

    /// The case where a label is the prefix of another label, and the case where a name not matching the grammar settles at the end of the
    ///   sort — if reverting to prefix
    /// match, both **delete someone else's file**.
    #[test]
    fn prune_selects_by_filename_grammar_not_prefix() {
        let data_dir = temp_dir("grammar");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");

        // A future kind that does not exist yet (the label starts with `app-`) and a hand-placed name.
        for name in [
            "app-updater-20260101-000000-1.log",
            "app-메모.log", // hangul-ok
            "app-zzzz-99999999-999999-1.log",
        ] {
            std::fs::write(dir.join(name), "x").expect("write");
        }
        for i in 0..5 {
            std::fs::write(dir.join(format!("app-20260101-{i:06}-1{LOG_EXT}")), "x")
                .expect("write");
        }

        prune_old_logs(&dir, LogKind::App, 2);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "app-20260101-000003-1.log".to_string(),
                "app-20260101-000004-1.log".to_string(),
                "app-updater-20260101-000000-1.log".to_string(),
                "app-zzzz-99999999-999999-1.log".to_string(),
                "app-메모.log".to_string(), // hangul-ok
            ],
            "only files of this kind matching the grammar may be candidates"
        );
    }

    #[test]
    fn open_run_log_enforces_ceiling_including_this_run() {
        let data_dir = temp_dir("ceiling");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");
        for i in 0..(KEEP_PER_KIND + 5) {
            std::fs::write(dir.join(format!("daemon-20260101-{i:06}-1{LOG_EXT}")), "x")
                .expect("write");
        }

        let _opened = open_run_log(&dir, LogKind::Daemon).expect("must open");

        let count = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("daemon-"))
            .count();
        assert_eq!(count, KEEP_PER_KIND);
    }

    /// A truly exclusive open that also blocks deletion can be made only with Windows share modes (std `File::open` opens
    /// including `FILE_SHARE_DELETE`, so deletion simply succeeds — that cannot catch this regression).
    #[cfg(windows)]
    #[test]
    fn prune_tolerates_locked_files() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let data_dir = temp_dir("locked");
        let dir = data_dir.join(LOG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("dir");
        for i in 0..5 {
            std::fs::write(
                dir.join(format!("daemon-2026010{i}-000000-1{LOG_EXT}")),
                "x",
            )
            .expect("write");
        }
        let held = dir.join(format!("daemon-20260100-000000-1{LOG_EXT}"));
        let _lock = File::options()
            .read(true)
            .share_mode(0)
            .open(&held)
            .expect("exclusive open");

        let failures = prune_old_logs(&dir, LogKind::Daemon, 2);

        assert!(
            held.exists(),
            "the locked file must remain (deletion failure is swallowed)"
        );
        assert_eq!(
            failures.len(),
            1,
            "swallowed, but the reason must be returned: {failures:?}"
        );
        assert!(
            failures[0].contains("daemon-20260100-000000-1.log"),
            "the failure must name the file: {failures:?}"
        );
        assert!(
            !dir.join(format!("daemon-20260102-000000-1{LOG_EXT}"))
                .exists(),
            "old files that are not locked must be deleted as usual"
        );
        assert!(dir
            .join(format!("daemon-20260104-000000-1{LOG_EXT}"))
            .exists());
    }

    // ── writer ──
    /// In production there is only [`LOG_FILE`], but it is filled once per process so tests cannot share it.
    /// So the slot the sink points to is made anew for each test.
    fn leaked_slot() -> &'static OnceLock<Mutex<File>> {
        Box::leak(Box::new(OnceLock::new()))
    }

    #[test]
    fn file_sink_writer_survives_poisoned_lock() {
        let data_dir = temp_dir("poison");
        let opened = open_run_log(&data_dir, LogKind::App).expect("must open");
        let slot = leaked_slot();
        assert!(slot.set(Mutex::new(opened.file)).is_ok(), "fill the slot");
        let sink = FileSink(slot);

        std::thread::scope(|s| {
            let _ = s
                .spawn(|| {
                    let _guard = slot.get().expect("slot").lock().expect("lock");
                    panic!("poison");
                })
                .join();
        });

        sink.make_writer()
            .write_all(b"after-poison\n")
            .expect("must write");
        assert!(std::fs::read_to_string(&opened.path)
            .expect("read")
            .ends_with("after-poison\n"));
    }

    /// Lines arriving in the window where the file is not yet open (= between layer install and slot fill) are dropped — if not blocked here,
    /// `unwrap`-family calls would kill the process with one log line early in boot.
    #[test]
    fn file_sink_writer_discards_lines_before_the_file_exists() {
        let sink = FileSink(leaked_slot());
        sink.make_writer()
            .write_all(b"nowhere\n")
            .expect("must swallow");
    }

    /// If the thread that holds the writer comes back (a hook log of a thread that panicked while holding it), that line is dropped and it
    ///   does not wait for itself.
    /// Running it on another thread with a deadline is because the regression would show up as a permanent hang.
    #[test]
    fn file_sink_drops_the_line_of_a_thread_already_writing() {
        let data_dir = temp_dir("reentry");
        let opened = open_run_log(&data_dir, LogKind::App).expect("must open");
        let slot = leaked_slot();
        assert!(slot.set(Mutex::new(opened.file)).is_ok(), "fill the slot");
        let sink = FileSink(slot);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut outer = sink.make_writer();
            outer.write_all(b"outer-1\n").expect("must write");
            let mut inner = sink.make_writer();
            let inner_void = matches!(inner, FileSinkWriter::Void);
            inner.write_all(b"inner\n").expect("must swallow");
            drop(inner);
            outer.write_all(b"outer-2\n").expect("must write");
            drop(outer);
            sink.make_writer()
                .write_all(b"after\n")
                .expect("must write");
            let _ = done_tx.send(inner_void);
        });

        let inner_void = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the second writer on the same thread waited for its own lock");
        assert!(inner_void, "the second writer must be the discarding one");
        assert!(std::fs::read_to_string(&opened.path)
            .expect("read")
            .ends_with("outer-1\nouter-2\nafter\n"));
    }

    /// A panicking thread (hook · destructor during unwinding) facing a lock held and stuck by another thread waits only up to the cap and
    ///   drops the line.
    /// The regression is a permanent hang, so a deadline is applied.
    #[test]
    fn a_panicking_thread_gives_up_on_a_lock_held_by_another_thread() {
        let data_dir = temp_dir("held-elsewhere");
        let opened = open_run_log(&data_dir, LogKind::App).expect("must open");
        let slot = leaked_slot();
        assert!(slot.set(Mutex::new(opened.file)).is_ok(), "fill the slot");

        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = std::thread::spawn(move || {
            let _guard = slot.get().expect("slot").lock().expect("lock");
            held_tx.send(()).expect("notify");
            let _ = release_rx.recv();
        });
        held_rx.recv().expect("the holding thread locked it");

        struct LogWhileUnwinding(FileSink, std::sync::mpsc::Sender<(bool, Duration)>);
        impl Drop for LogWhileUnwinding {
            fn drop(&mut self) {
                let started = Instant::now();
                let mut writer = self.0.make_writer();
                let void = matches!(writer, FileSinkWriter::Void);
                let _ = writer.write_all(b"while-panicking\n");
                let _ = self.1.send((void, started.elapsed()));
            }
        }
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _log = LogWhileUnwinding(FileSink(slot), done_tx);
                panic!("panic while someone else holds it");
            }));
        });

        let outcome = done_rx.recv_timeout(Duration::from_secs(10));
        let _ = release_tx.send(());
        holder.join().expect("holding thread");
        let (void, waited) =
            outcome.expect("a panicking thread waited forever for a lock held by another");
        assert!(void, "after the cap it must be the discarding writer");
        assert!(
            waited >= PANICKING_LOCK_WAIT,
            "must wait up to the cap: {waited:?}"
        );
        assert!(!std::fs::read_to_string(&opened.path)
            .expect("read")
            .contains("while-panicking"));
    }

    #[test]
    fn file_sink_lowers_the_writing_mark_when_a_writer_unwinds() {
        let data_dir = temp_dir("unwind");
        let opened = open_run_log(&data_dir, LogKind::App).expect("must open");
        let slot = leaked_slot();
        assert!(slot.set(Mutex::new(opened.file)).is_ok(), "fill the slot");
        let sink = FileSink(slot);

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _writer = sink.make_writer();
            panic!("panic while holding");
        }));

        assert!(unwound.is_err());
        sink.make_writer()
            .write_all(b"after-unwind\n")
            .expect("must write");
        assert!(std::fs::read_to_string(&opened.path)
            .expect("read")
            .ends_with("after-unwind\n"));
    }
}
