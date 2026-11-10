//! Shared rules for data files — read decisions ([`parse`] · [`classify`] · [`load`]) · write decisions ([`Loaded::write_policy`] ·
//! [`judge`] · [`check_cap`]) · capped reads ([`read_capped`] · [`read_file_capped`]) · atomic writes ([`write_atomic`] ·
//! [`write_atomic_unless`]) · atomic copies ([`copy_atomic`] · [`copy_aside`]) · sweeping leftover temp files ([`sweep_temps`]) and the
//! temp-name test ([`is_temp_name`]).
//!
//! It knows no file names · locations · schemas · version key names — each owner (store) owns those, and the part that
//! belongs to the read rules is handed over as a [`Spec`]. The only names decided here sit next to the target: the temp file `<이름>.tmp<pid>.<번호>` · the set-aside copy
//! `<이름>.corrupt`. ★Only here are the code that makes that shape (`temp_path` · [`copy_aside`]) and the code that reads it (`split_temp_name`)★ — if the shape
//! changes, fix all three together (the tests read back the names they made and compare).
//!
//! ★「덮어도 되나」 (may we overwrite?) is decided again for every write, right before it, from the current file★ — the decision rule is one [`Loaded::write_policy`] table,
//! and the owner drives the order (read → decide → set aside → write). A file written by a newer version and a file that failed to read are neither overwritten nor set
//! aside. The owner's shape check ([`Spec::shape`]) runs inside [`parse`], so a file with a broken shape is corrupt on any decision
//! path.
//!
//! Folders: writes and copies create missing folders down to the target folder and sync that folder after the rename. Reads and decisions
//! create nothing.
//!
//! ★The OS-dependent part is passed in by the caller as [`OsHooks`]★ — this crate cannot call an OS-layer crate (tenancy
//! condition ②). Lock retry has one path: the rename, the open and read in [`read_file_capped`], and the source open in [`copy_atomic`]
//! all go through [`OsHooks::retry`]. A read failure in the middle of a copy is not retried.
//!
//! It emits no logs — it returns values and the caller prints them in its own wording.
// ADR-0291

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value};

/// Two OS-dependent hooks — the caller passes through the OS-layer crate's (`engram-dashboard-platform`'s `fs`) ones. The production
/// value is a constant per crate (two owners have reads with a different budget — ADR-0291 R14).
///
/// They are fn pointers so that the production value can be a `const` — stateful test fakes (attempt counting · lock injection · not sleeping ·
/// sync recording) are not passed through this.
// ADR-0291
#[derive(Clone, Copy)]
pub struct OsHooks {
    /// Retrying a briefly held file — calls `attempt` until the lock clears or the budget runs out and returns the last result.
    /// Success and non-lock errors are returned at once (a missing file `NotFound` · unusable content `InvalidData` are not
    /// retried). Production = `fs::retry_busy` (budget = [`BUSY_RETRIES`] · [`BUSY_PAUSE`] — the long load budget an owner sets separately is the exception · R14).
    pub retry: fn(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>,
    /// Folder sync that makes a rename inside a folder durable on disk — production = `fs::sync_dir`. Called once on that folder only after an atomic write or copy
    /// has swapped in by rename, and the result is not looked at (the write succeeds even if it fails).
    pub sync_dir: fn(&Path) -> io::Result<()>,
}

/// Lock budget for data files — the number of retries after the first attempt and the wait between them. The production [`OsHooks::retry`] uses these values (the long load budget an owner sets separately is the exception · ADR-0291 R14).
/// In the worst case it takes about 100 ms more, and a lock held longer than that fails as is.
// ADR-0265 decision 4: if rename hits a lock, retry briefly.
// ADR-0291: which failure is a lock and the retry loop are one OS-layer policy — this holds only the budget.
pub const BUSY_RETRIES: u32 = 5;
pub const BUSY_PAUSE: Duration = Duration::from_millis(20);

/// The closure shape of [`OsHooks::retry`] — public functions unpack the production fn pointer into this shape and pass it, and this module's tests pass a fake
/// that holds state. [`SyncDirHook`] is the same.
type RetryHook<'a> = &'a mut dyn FnMut(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>;
/// The closure shape of [`OsHooks::sync_dir`].
type SyncDirHook<'a> = &'a mut dyn FnMut(&Path) -> io::Result<()>;

// ── Read rules · write decisions ──

/// Read rules for one data file — the owner (store) sets them as a constant per file.
// ADR-0291 R1~R7
#[derive(Clone, Copy)]
pub struct Spec {
    /// The top-level key that carries the version (as it stands per file — `$version` · `version` · `schema_version`).
    pub version_key: &'static str,
    /// The version this build writes — 1 or more. A larger version is [`Parsed::Newer`].
    pub current: u64,
    /// Read cap = write cap (bytes — the raw text on disk as is, BOM included). Raw text over it is corrupt and is not written either
    /// ([`check_cap`]).
    pub cap: u64,
    /// The owner's shape check — can that owner read a document that passed the version gate (version `1..=current`)? `Err(사유)` = corrupt
    /// (the reason goes into [`Parsed::Unusable`] as is — the owner decides whether to carry the value).
    ///
    /// [`parse`] calls it, so no decision path skips the shape. ★If there is no reader for an earlier version (`< current`), return
    /// `Err` here to make it corrupt★ — the version gate may well let that version through. It is not called for a newer version ([`Parsed::Newer`]).
    /// An owner that does not check shape uses [`any_shape`].
    pub shape: fn(version: u64, doc: &Map<String, Value>) -> Result<(), String>,
}

/// The [`Spec::shape`] of an owner that does not check shape — always `Ok`.
pub fn any_shape(_version: u64, _doc: &Map<String, Value>) -> Result<(), String> {
    Ok(())
}

/// The result of looking at one raw text — knows no IO.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// Version `1..=current` and passed the shape check. `doc` is as is, version key included.
    Usable {
        version: u64,
        doc: Map<String, Value>,
    },
    /// Corrupt — a reason string. Reasons made by this module carry no value · if the shape check failed, it is the owner's reason as is.
    Unusable(String),
    /// A version newer than this build — shape not looked at (a newer version may already differ in shape).
    Newer { found: u64 },
}

/// Decides one raw text — in the order cap · stripping one leading BOM · JSON object · version gate · [`Spec::shape`].
///
/// - No version key means version 1.
/// - A version value is accepted only if it is an integer-valued number of 1 or more (`1` · `1.0`) — 0 · negative · fractional · string · outside `u64` is corrupt.
/// - Larger than `current` is [`Parsed::Newer`] — it ends before the shape check.
///
/// The cap is measured again here even if [`read_file_capped`] already cut it — whichever way the owner's IO seam
/// read, the same definition of corrupt holds.
// ADR-0291 R1~R5 · R7: no version = 1 · newer version handled separately · integer-valued numbers only · one BOM ignored · shape failure = corrupt.
pub fn parse(text: &str, spec: &Spec) -> Parsed {
    if text.len() as u64 > spec.cap {
        return Parsed::Unusable(format!("{} 바이트 상한을 넘었다", spec.cap));
    }
    // Notepad · PowerShell add a BOM when saving as UTF-8 — so that a hand-edited file does not become unusable for that reason alone.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let doc = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(doc)) => doc,
        Ok(_) => return Parsed::Unusable("JSON 객체가 아니다".to_string()),
        Err(e) => return Parsed::Unusable(format!("JSON 이 아니다: {e}")),
    };
    let version = match doc.get(spec.version_key) {
        None => 1,
        Some(raw) => match version_number(raw) {
            Some(version) if version >= 1 => version,
            _ => {
                return Parsed::Unusable(format!(
                    "`{}` 가 1 이상의 정수가 아니다",
                    spec.version_key
                ))
            }
        },
    };
    if version > spec.current {
        return Parsed::Newer { found: version };
    }
    if let Err(reason) = (spec.shape)(version, &doc) {
        return Parsed::Unusable(reason);
    }
    Parsed::Usable { version, doc }
}

/// Integer-valued numbers only — a float with an integer value (`1.0`) is accepted too (JSON tools sometimes rewrite numbers as floats). Negative · fractional ·
/// outside the `u64` range · non-number values give `None`.
fn version_number(raw: &Value) -> Option<u64> {
    // 2^64 — the same value as `u64::MAX as f64`, so it is itself out of range. `as u64` clamps out-of-range to u64::MAX and
    //   invents a nonexistent 「새 판」 (newer version), so filter it out before casting.
    const U64_END: f64 = 18_446_744_073_709_551_616.0;
    raw.as_u64().or_else(|| {
        let float = raw.as_f64()?;
        (0.0..U64_END)
            .contains(&float)
            .then_some(float)
            .filter(|float| float.fract() == 0.0)
            .map(|float| float as u64)
    })
}

/// The result of reading one file.
#[derive(Debug)]
pub enum Loaded {
    Missing,
    Parsed(Parsed),
    /// Not the content — the read itself failed (after the lock budget · permissions · other IO). ★Not mixed with corrupt★ — a perfectly fine file that is
    /// briefly locked is neither set aside nor overwritten.
    Failed(io::Error),
}

/// Turns a read result into a decision — `NotFound` = [`Loaded::Missing`] · `InvalidData` (over the cap · not UTF-8 —
/// the contract of [`read_capped`]) = corrupt · any other error = [`Loaded::Failed`] · raw text = [`parse`].
///
/// An owner that reads through its own IO seam (the trait that takes test fakes) feeds that read result in here. An owner that reads directly by path uses
/// [`load`].
pub fn classify(read: io::Result<String>, spec: &Spec) -> Loaded {
    match read {
        Ok(text) => Loaded::Parsed(parse(&text, spec)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Loaded::Missing,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            Loaded::Parsed(Parsed::Unusable(e.to_string()))
        }
        Err(e) => Loaded::Failed(e),
    }
}

/// Reads `path` up to [`Spec::cap`] ([`read_file_capped`] — `os.retry` on a lock) and runs [`classify`] on it. Creates nothing —
/// if the folder is missing it is [`Loaded::Missing`].
pub fn load(path: &Path, spec: &Spec, os: OsHooks) -> Loaded {
    classify(read_file_capped(path, spec.cap, os), spec)
}

/// Has this run already taken that file as its own — one axis of [`Loaded::write_policy`].
// ADR-0291 R8 (D14)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// The load decision was writable, or this run has written to that file at least once.
    Adopted,
    /// The load decision was missing or corrupt and nothing has been written yet — a usable file that appeared in between is not ours, so it is set
    /// aside before overwriting.
    NotYet,
}

/// May we write to that file now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePolicy {
    Write,
    /// Write after setting it aside with [`copy_aside`]. Whether to still write when the set-aside fails is up to the owner.
    CopyAsideFirst,
    /// Do not write — neither overwrite nor set aside.
    Refuse(Refused),
}

/// The reason for [`WritePolicy::Refuse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A file written by a version newer than this build.
    Newer { found: u64 },
    /// The current file could not be read ([`Loaded::Failed`]) — what it holds is unknown, so it is not overwritten. An owner that needs that error
    /// holds the [`Loaded`] via [`load`] instead of [`judge`] and calls [`Loaded::write_policy`].
    ReadFailed,
}

impl Loaded {
    /// May we write over this read result — one common rule for data files.
    ///
    /// | current file | [`Claim::Adopted`] | [`Claim::NotYet`] |
    /// |---|---|---|
    /// | missing | write | write |
    /// | usable | write | set aside, then write |
    /// | corrupt | set aside, then write | set aside, then write |
    /// | newer version | refuse | refuse |
    /// | read failure | refuse | refuse |
    ///
    /// ★The decision is made for every write, right before it, by reading the current file again★ — holding the load-time decision would overwrite a file that
    /// turned into a newer version during the run, and holding 「손상이라 떠 두고 쓴다」 (corrupt, so set aside and write) would set aside our healthy file on every write after the first and
    /// delete the real corrupt copy.
    // ADR-0291 R2 · R8 · R16
    pub fn write_policy(&self, claim: Claim) -> WritePolicy {
        match (self, claim) {
            (Loaded::Missing, Claim::Adopted | Claim::NotYet) => WritePolicy::Write,
            (Loaded::Parsed(Parsed::Usable { .. }), Claim::Adopted) => WritePolicy::Write,
            (Loaded::Parsed(Parsed::Usable { .. }), Claim::NotYet) => WritePolicy::CopyAsideFirst,
            (Loaded::Parsed(Parsed::Unusable(_)), Claim::Adopted | Claim::NotYet) => {
                WritePolicy::CopyAsideFirst
            }
            (Loaded::Parsed(Parsed::Newer { found }), Claim::Adopted | Claim::NotYet) => {
                WritePolicy::Refuse(Refused::Newer { found: *found })
            }
            (Loaded::Failed(_), Claim::Adopted | Claim::NotYet) => {
                WritePolicy::Refuse(Refused::ReadFailed)
            }
        }
    }
}

/// The [`Loaded::write_policy`] of the current file just [`load`]ed — the pre-save decision for an owner that reads directly by path.
pub fn judge(path: &Path, spec: &Spec, os: OsHooks, claim: Claim) -> WritePolicy {
    load(path, spec, os).write_policy(claim)
}

/// Is the length of the raw text to write within [`Spec::cap`] — if over, `InvalidData` (the next load could not use that file at all).
// ADR-0291 R6: read cap = write cap.
pub fn check_cap(len: usize, spec: &Spec) -> io::Result<()> {
    if len as u64 > spec.cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "쓸 원문({len} 바이트)이 읽기 상한 {} 바이트를 넘는다",
                spec.cap
            ),
        ));
    }
    Ok(())
}

// ── Read · write machinery ──

/// Runs `attempt` through `retry` and takes its value out — the hook does not know the value type, so the value is received separately here. If the hook
/// returns `Ok` but there is no value (the attempt was never called), it is `ErrorKind::Other`, not a panic.
fn retrying<T>(retry: RetryHook, mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut value = None;
    retry(&mut || {
        value = Some(attempt()?);
        Ok(())
    })?;
    value.ok_or_else(|| io::Error::other("다시 하기가 시도를 한 번도 부르지 않았다"))
}

/// ★Reads only up to the cap — does not measure after reading★.
///
/// If it read everything first and then measured the length, memory would run out before the cap check arrived (the file is written by outside agents · people,
/// so its size is not in our hands). Then neither the default fold nor a warning could run and the process would die.
///
/// Over the cap and not UTF-8 are **both `ErrorKind::InvalidData`** — in both cases the raw text could not be fetched. Other kinds
/// are IO failures of the read itself, and what tells them apart is [`classify`] — if the content is unusable it is corrupt, if it is an IO failure it is left
/// alone. There is no lock retry — whoever opens the file uses [`read_file_capped`] if needed.
pub fn read_capped(source: impl io::Read, cap: u64) -> io::Result<String> {
    use std::io::Read;

    let mut buf = Vec::new();
    // cap + 1 = the smallest amount that can tell 「넘었나」 (did it go over?). Even if it went over, the amount read stops here.
    source.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{cap} 바이트 상한을 넘었다"),
        ));
    }
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Opens the file and reads it with [`read_capped`] — if the open or read fails on a lock, it opens again via [`OsHooks::retry`] and
/// retries. When an antivirus · indexer briefly holds the file, reads fail that way too.
///
/// Other errors are returned at once without retry — missing file = `NotFound` · over the cap · not UTF-8 =
/// `InvalidData`. ★If a lock exceeds the budget, it is that lock error as is★ — not mixed with `InvalidData` (unusable content).
pub fn read_file_capped(path: &Path, cap: u64, os: OsHooks) -> io::Result<String> {
    read_file_capped_with(|| std::fs::File::open(path), cap, &mut |attempt| {
        (os.retry)(attempt)
    })
}

fn read_file_capped_with<R: io::Read>(
    mut open: impl FnMut() -> io::Result<R>,
    cap: u64,
    retry: RetryHook,
) -> io::Result<String> {
    retrying(retry, || read_capped(open()?, cap))
}

/// The result of [`write_atomic_unless`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Written,
    /// `skip` stood and no rename happened — the temp file was removed and the target is unchanged. Not an error, so not retried.
    Skipped,
}

/// Writes to a temp file in the same folder and **swaps it in by rename** — dying mid-write leaves no half file. On failure the temp
/// file is removed and the target is unchanged. If rename hits a lock, it retries via [`OsHooks::retry`].
///
/// If the target folder is missing, it creates the missing folders above it too — created folders stay even if the write fails. After the swap it
/// syncs that folder with [`OsHooks::sync_dir`] (the result is not looked at).
///
/// For calls writing the same path concurrently, the one that renames later remains — which is later is not decided. If order
/// matters, the caller serializes.
// ADR-0265 decision 4: config writes are atomic (temp file → sync_all → rename).
// ADR-0291 R10 · R11: folder sync after rename · the writer creates the folder when it writes.
pub fn write_atomic(path: &Path, bytes: &[u8], os: OsHooks) -> io::Result<()> {
    write_atomic_unless(path, bytes, os, || false).map(drop)
}

/// [`write_atomic`] plus 「rename 직전에 물을 것」 (what to ask right before rename) — asks `skip` **before the first rename and before every
/// rename of a lock retry**, and if `true`, [`WriteOutcome::Skipped`].
///
/// ★A marker set right after `skip` returned `false` cannot stop that rename★ — the gap between asking and the rename is not
/// atomic.
pub fn write_atomic_unless(
    path: &Path,
    bytes: &[u8],
    os: OsHooks,
    skip: impl Fn() -> bool,
) -> io::Result<WriteOutcome> {
    write_atomic_with(
        path,
        bytes,
        &mut |attempt| (os.retry)(attempt),
        &mut |dir| (os.sync_dir)(dir),
        &skip,
    )
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    retry: RetryHook,
    sync_dir: SyncDirHook,
    skip: &dyn Fn() -> bool,
) -> io::Result<WriteOutcome> {
    use std::io::Write;

    replace_with(
        path,
        |file| file.write_all(bytes),
        skip,
        |from, to| std::fs::rename(from, to),
        retry,
        sync_dir,
    )
}

/// Atomically copies `from` to `to` — it streams, so it does not load everything into memory and has no size cap. An existing `to`
/// is overwritten. On failure the temp file is removed and `to` is unchanged.
///
/// If opening `from` or rename hits a lock, it retries via [`OsHooks::retry`]. If the open never succeeds, that error comes before either the temp file or the folder of `to` is
/// created. Folder creation · sync are the same as for [`write_atomic`].
// ADR-0274
// ADR-0291 R10 · R11
pub fn copy_atomic(from: &Path, to: &Path, os: OsHooks) -> io::Result<()> {
    copy_atomic_with(from, to, &mut |attempt| (os.retry)(attempt), &mut |dir| {
        (os.sync_dir)(dir)
    })
}

fn copy_atomic_with(
    from: &Path,
    to: &Path,
    retry: RetryHook,
    sync_dir: SyncDirHook,
) -> io::Result<()> {
    // Does not use `std::fs::copy` — it carries over the source's permissions (the Windows read-only attribute · Unix permission bits), so a read-only
    //   source makes a read-only copy too, and the write handle for `sync_all` cannot be opened.
    let mut source = retrying(&mut *retry, || std::fs::File::open(from))?;
    replace_with(
        to,
        |file| io::copy(&mut source, file).map(drop),
        || false,
        |from, to| std::fs::rename(from, to),
        retry,
        sync_dir,
    )
    .map(drop)
}

/// Sets aside an unusable file with [`copy_atomic`] at the neighboring name `<이름>.corrupt` — the original stays in place.
/// Return value = the copy's location. On `Err`, a copy set aside earlier stays as it was.
///
/// ★There is only one name, so it overwrites a copy set aside earlier★ — the same practice as Chromium (`Preferences.bad`) · Firefox (`Invalidprefs.js`)
/// (TRD S21-storage §10 F21). Copies do not pile up, so no size cap is set.
// ADR-0274
pub fn copy_aside(path: &Path, os: OsHooks) -> io::Result<PathBuf> {
    let to = sibling(path, ASIDE_SUFFIX)?;
    copy_atomic(path, &to, os)?;
    Ok(to)
}

/// Fills a temp file in the same folder with `stage`, `sync_all`s it, then **swaps `path` in by rename**. `skip` is asked
/// before every rename ([`write_atomic_unless`]).
///
/// ★The temp file is made in the same folder★ — rename works as a swap only within the same volume. The name is `temp_path` —
/// if two calls shared a temp name, the later create would empty the earlier temp file, and the earlier rename would swap that half file in
/// as `Ok`.
///
/// If not `Written` (failure · skipped), the temp file is removed and `path` is left as is — if not removed, garbage piles up in the data
/// folder. If `Written`, `sync_dir` that folder — rename changes an entry of the folder, so the file's `sync_all`
/// alone does not guarantee that the change lands on disk (POSIX).
///
/// ★If rename fails on a lock, retry with `retry`★ — when an antivirus · indexer briefly holds the target file, rename on Windows
/// fails that way.
///
/// `rename` · `retry` · `sync_dir` are test seams — production is `std::fs::rename` · the caller's [`OsHooks`].
// ADR-0265 decision 4: if rename hits a lock, retry briefly.
// ADR-0291 R10 · R11
fn replace_with(
    path: &Path,
    stage: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
    skip: impl Fn() -> bool,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    retry: RetryHook,
    sync_dir: SyncDirHook,
) -> io::Result<WriteOutcome> {
    let tmp = temp_path(path)?;
    let dir = folder_of(path);

    let staged = (|| {
        // Loading does not create the folder, so the first write creates it — a folder deleted during the run is set up again here too.
        std::fs::create_dir_all(dir)?;
        let mut file = std::fs::File::create(&tmp)?;
        stage(&mut file)?;
        // ★flush is not enough★ — it only empties the process buffer. The content that rename will point to must actually be
        //   on disk for 「반쪽이 안 남는다」 (no half file is left) to hold.
        file.sync_all()
    })();
    let outcome = staged.and_then(|()| {
        retrying(retry, || {
            if skip() {
                return Ok(WriteOutcome::Skipped);
            }
            rename(&tmp, path).map(|()| WriteOutcome::Written)
        })
    });
    if matches!(outcome, Ok(WriteOutcome::Written)) {
        let _ = sync_dir(dir);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

/// The folder that holds `path` — a path given as a bare name without a folder (`a.json`) is the current folder (`.`). If an empty path is passed, folder sync
/// cannot open that folder.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

const TEMP_MARK: &str = ".tmp";
const ASIDE_SUFFIX: &str = ".corrupt";

/// The temp name next to `path`, `<이름>.tmp<pid>.<번호>` — the pid tells apart other processes looking at the same folder, and the number (one more each time it is
/// called within the process) tells apart other calls of the same process.
fn temp_path(path: &Path) -> io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    sibling(path, &format!("{TEMP_MARK}{}.{n}", std::process::id()))
}

/// If `file_name` is a temp name of `target` (the `temp_path` shape), its pid. The pid · number must be the decimal exactly
/// as `temp_path` writes it (not `+` · leading zeros · out of range).
///
/// ★A `<target>.tmp<pid>` without a number (the shape before the number was added) is not treated as a temp name★ — the target of the current sweep
/// has never been written in that shape, so we cannot assert such a name is ours, and a file that may be someone else's is not deleted.
/// If a target written in the old shape is to be swept, look at this rule first.
fn temp_owner(file_name: &str, target: &str) -> Option<u32> {
    let (name, pid) = split_temp_name(file_name)?;
    (name == target).then_some(pid)
}

/// Whether `file_name` (a bare name, not a path) has the temp-name shape `temp_path` writes, `<name>.tmp<pid>.<n>`, for
/// any non-empty `<name>` — so a set-aside's `<x>.corrupt.tmp<pid>.<n>` is one too. The pid · number rule is
/// `temp_owner`'s (exact decimals). It reads the name only; it does not say whose file it is or whether it may be deleted
/// (that is [`sweep_temps`]).
pub fn is_temp_name(file_name: &str) -> bool {
    split_temp_name(file_name).is_some()
}

/// `<name>.tmp<pid>.<n>` → (`<name>`, pid). The number has no `.`, so the last `.` ends `<pid>`; the pid has no `.tmp`,
/// so the last `.tmp` before it ends `<name>`.
fn split_temp_name(file_name: &str) -> Option<(&str, u32)> {
    let (head, n) = file_name.rsplit_once('.')?;
    exact_decimal::<u64>(n)?;
    let mark = head.rfind(TEMP_MARK)?;
    let name = &head[..mark];
    let pid = exact_decimal::<u32>(&head[mark + TEMP_MARK.len()..])?;
    (!name.is_empty()).then_some((name, pid))
}

fn exact_decimal<T: std::str::FromStr + ToString>(digits: &str) -> Option<T> {
    digits
        .parse::<T>()
        .ok()
        .filter(|value| value.to_string() == digits)
}

/// In `dir`, for each of `targets` (file names), deletes the leftover temp files — the atomic write's `<대상>.tmp<pid>.<번호>` and the set-aside's
/// `<대상>.corrupt.tmp<pid>.<번호>` — whose pid is this process or that `is_alive` said is dead.
///
/// - Those of a live other pid are kept — another process using the same folder may be writing right now.
/// - ★Those of our own pid are deleted too★ — call it only when no call writing the same target is in this process.
///
/// Return value = the files it tried to delete and their results (success if already gone in the meantime). An empty list if the folder is missing. `Err` = the folder
/// could not be read.
pub fn sweep_temps(
    dir: &Path,
    targets: &[&str],
    is_alive: impl Fn(u32) -> bool,
) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let me = std::process::id();
    let mut swept = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let owner = targets.iter().find_map(|target| {
            temp_owner(name, target)
                .or_else(|| temp_owner(name, &format!("{target}{ASIDE_SUFFIX}")))
        });
        let Some(pid) = owner else {
            continue;
        };
        if pid != me && is_alive(pid) {
            continue;
        }
        let path = entry.path();
        let removed = match std::fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            removed => removed,
        };
        swept.push((path, removed));
    }
    Ok(swept)
}

/// The `<path 이름><suffix>` in the same folder as `path`.
fn sibling(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let invalid = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} 를 못 고르는 경로"),
        )
    };
    let dir = path.parent().ok_or_else(|| invalid("부모 폴더"))?;
    let mut name = path
        .file_name()
        .ok_or_else(|| invalid("파일 이름"))?
        .to_os_string();
    name.push(suffix);
    Ok(dir.join(name))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Write};

    use super::*;

    /// A hook that does not retry — real-file tests never meet a lock. It stands as an fn pointer because it has no state.
    const ONCE: OsHooks = OsHooks {
        retry: |attempt| attempt(),
        sync_dir: |_| Ok(()),
    };

    /// A fake that imitates the shape of the production retry — on a lock (here `PermissionDenied`) it retries up to [`BUSY_RETRIES`] times,
    /// but does not sleep and only counts the waits in `pauses`. Which error is a lock is, in production, up to the hook passed in (OS layer), so what
    /// this module's tests measure is whether that error goes to the hook unchanged.
    fn counting_retry(
        pauses: &mut u32,
    ) -> impl FnMut(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()> + '_ {
        move |attempt: &mut dyn FnMut() -> io::Result<()>| {
            let mut retries = 0;
            loop {
                match attempt() {
                    Err(e)
                        if e.kind() == io::ErrorKind::PermissionDenied
                            && retries < BUSY_RETRIES =>
                    {
                        retries += 1;
                        *pauses += 1;
                    }
                    outcome => return outcome,
                }
            }
        }
    }

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-base-file-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // ── Capped reads ──

    #[test]
    fn a_locked_open_is_retried_until_it_clears() {
        let mut failures = 2;
        let mut pauses = 0;
        let text = read_file_capped_with(
            || {
                if failures > 0 {
                    failures -= 1;
                    Err(denied())
                } else {
                    Ok(io::Cursor::new(b"hi".to_vec()))
                }
            },
            8,
            &mut counting_retry(&mut pauses),
        )
        .unwrap();
        assert_eq!((text.as_str(), pauses), ("hi", 2));
    }

    /// A source that fails on a lock at the first `read`.
    struct LockedOnRead;

    impl io::Read for LockedOnRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(denied())
        }
    }

    #[test]
    fn a_read_locked_mid_way_is_retried_from_a_fresh_open() {
        let mut opens = 0;
        let mut pauses = 0;
        let text = read_file_capped_with(
            || -> io::Result<Box<dyn io::Read>> {
                opens += 1;
                Ok(if opens == 1 {
                    Box::new(LockedOnRead)
                } else {
                    Box::new(io::Cursor::new(b"hi".to_vec()))
                })
            },
            8,
            &mut counting_retry(&mut pauses),
        )
        .unwrap();
        assert_eq!((text.as_str(), opens, pauses), ("hi", 2, 1));
    }

    #[test]
    fn a_missing_file_and_unusable_content_are_not_retried() {
        let mut opens = 0;
        let mut pauses = 0;
        let missing = read_file_capped_with(
            || -> io::Result<io::Cursor<Vec<u8>>> {
                opens += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            8,
            &mut counting_retry(&mut pauses),
        );
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!((opens, pauses), (1, 0));

        let over = read_file_capped_with(
            || Ok(io::Cursor::new(b"12345".to_vec())),
            4,
            &mut counting_retry(&mut pauses),
        );
        assert_eq!(over.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(pauses, 0);
    }

    #[test]
    fn read_file_capped_reads_a_real_file_and_reports_a_missing_one() {
        let dir = temp_dir("read");
        let path = dir.join("state.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(read_file_capped(&path, 8, ONCE).unwrap(), "{}");
        assert_eq!(
            read_file_capped(&dir.join("none.json"), 8, ONCE)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exactly_the_cap_is_read_and_one_byte_more_is_refused() {
        let cap = 32u64;
        assert!(
            read_capped(io::Cursor::new(vec![b'x'; cap as usize]), cap).is_ok(),
            "상한 자체는 통과다"
        );
        let over = read_capped(io::Cursor::new(vec![b'x'; cap as usize + 1]), cap)
            .expect_err("상한 초과는 반려다");
        assert_eq!(over.kind(), io::ErrorKind::InvalidData);
    }

    /// A source that counts the bytes it hands out — looking only at the result, 「다 읽고 나서 반려」 (read it all, then reject) and 「끊어 읽고 반려」 (cut the read, then reject) are both
    /// `InvalidData` and cannot be told apart.
    struct Counting<'a, R> {
        inner: R,
        produced: &'a Cell<u64>,
    }

    impl<R: io::Read> io::Read for Counting<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.produced.set(self.produced.get() + n as u64);
            Ok(n)
        }
    }

    /// ★The cap cuts the **amount read**, not the result★ — that is the very reason [`read_capped`] exists.
    #[test]
    fn the_cap_stops_the_read_rather_than_the_result() {
        let cap = 16u64;
        let produced = Cell::new(0u64);
        let reader = Counting {
            inner: io::Cursor::new(vec![b'x'; 1024 * 1024]),
            produced: &produced,
        };

        let refused = read_capped(reader, cap).expect_err("상한 초과는 반려다");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);

        let read = produced.get();
        assert!(
            read <= cap + 1,
            "상한을 넘겨 {read} 바이트를 읽었다(허용 {}) — 끊지 않으면 원문 전체가 메모리에 올라온다",
            cap + 1
        );
    }

    #[test]
    fn non_utf8_content_is_invalid_data_like_an_oversized_one() {
        let refused = read_capped(io::Cursor::new(vec![0xff, 0xfe, 0x00]), 64)
            .expect_err("UTF-8 아님은 반려다");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    }

    // ── Atomic writes ──

    #[test]
    fn an_unskipped_write_replaces_the_target_and_leaves_no_temp() {
        let dir = temp_dir("write");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();

        let outcome = write_atomic_unless(&path, b"new", ONCE, || false).unwrap();

        assert_eq!(outcome, WriteOutcome::Written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_skip_before_the_first_rename_leaves_the_target_and_no_temp() {
        let dir = temp_dir("skip-first");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        let asked = Cell::new(0);

        let outcome = write_atomic_unless(&path, b"new", ONCE, || {
            asked.set(asked.get() + 1);
            true
        })
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Skipped);
        assert_eq!(asked.get(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_skip_raised_during_a_lock_retry_stops_the_write() {
        let dir = temp_dir("skip-retry");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        // Answers no to the first ask, and yes to the ask after the first rename failed on a lock.
        let asked = Cell::new(0);
        let mut renames = 0;
        let mut pauses = 0;

        let outcome = replace_with(
            &path,
            |file| file.write_all(b"new"),
            || {
                asked.set(asked.get() + 1);
                asked.get() >= 2
            },
            |_, _| {
                renames += 1;
                Err(denied())
            },
            &mut counting_retry(&mut pauses),
            &mut |_| panic!("건너뛴 쓰기는 폴더를 동기화하지 않는다"),
        )
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Skipped);
        assert_eq!((asked.get(), renames, pauses), (2, 1, 1));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_rename_removes_the_temp_and_keeps_the_target() {
        let dir = temp_dir("rename-fail");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        let mut pauses = 0;

        let outcome = replace_with(
            &path,
            |file| file.write_all(b"new"),
            || false,
            |_, _| Err(io::Error::other("가짜 rename 실패")),
            &mut counting_retry(&mut pauses),
            &mut |_| panic!("실패한 쓰기는 폴더를 동기화하지 않는다"),
        );

        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(pauses, 0, "잠김이 아니면 기다리지 않는다");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_stage_removes_the_temp_and_never_renames() {
        let dir = temp_dir("stage-fail");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();

        let outcome = replace_with(
            &path,
            |_| Err(io::Error::other("가짜 쓰기 실패")),
            || panic!("채우기가 실패하면 묻지 않는다"),
            |_, _| panic!("채우기가 실패하면 rename 하지 않는다"),
            &mut |_| panic!("채우기가 실패하면 다시 하기에 들지 않는다"),
            &mut |_| panic!("채우기가 실패하면 폴더를 동기화하지 않는다"),
        );

        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_temps_for_one_target_never_share_a_name() {
        let path = Path::new("dir").join("state.json");
        let first = temp_path(&path).unwrap();
        let second = temp_path(&path).unwrap();

        assert_ne!(first, second);
        let prefix = format!("state.json.tmp{}.", std::process::id());
        for temp in [&first, &second] {
            assert_eq!(temp.parent(), path.parent(), "같은 폴더");
            let name = temp.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with(&prefix), "{name}");
        }
    }

    // ── Temp name parsing · sweeping ──

    #[test]
    fn the_owner_of_a_made_temp_name_reads_back_as_this_process() {
        for target in ["state.json", "state.json.corrupt"] {
            let temp = temp_path(&Path::new("dir").join(target)).unwrap();
            let name = temp.file_name().unwrap().to_str().unwrap();
            assert_eq!(temp_owner(name, target), Some(std::process::id()), "{name}");
        }
    }

    #[test]
    fn only_the_exact_numbered_shape_is_a_temp_name() {
        assert_eq!(temp_owner("state.json.tmp12.3", "state.json"), Some(12));
        assert_eq!(temp_owner("state.json.tmp12.0", "state.json"), Some(12));
        for name in [
            "state.json",
            "state.json.tmpX",
            "state.json.tmp12",
            "state.json.tmp12.",
            "state.json.tmp.3",
            "state.json.tmp12.3.4",
            "state.json.tmp12.x",
            "state.json.tmp+12.3",
            "state.json.tmp012.3",
            "state.json.tmp12.03",
            "state.json.tmp4294967296.3",
            "xstate.json.tmp12.3",
            "state.json.corrupt.tmp12.3",
            "state.crash.json.tmp12.3",
        ] {
            assert_eq!(temp_owner(name, "state.json"), None, "{name}");
        }
    }

    #[test]
    fn a_made_temp_name_is_a_temp_name_for_any_target() {
        for target in ["state.json", "state.json.corrupt", "a", "x.tmp1.2"] {
            let temp = temp_path(&Path::new("dir").join(target)).unwrap();
            let name = temp.file_name().unwrap().to_str().unwrap();
            assert!(is_temp_name(name), "{name}");
        }
    }

    // TRD S25 step-7 §8-1 P1 base row.
    #[test]
    fn only_the_exact_numbered_shape_is_a_temp_name_whatever_the_name() {
        for name in [
            "state.json.tmp12.3",
            "state.json.tmp12.0",
            "state.json.corrupt.tmp12.3",
            "preset.yaml.tmp4294967295.18446744073709551615",
            "x.tmp1.2.tmp3.4",
            "a.tmp0.0",
        ] {
            assert!(is_temp_name(name), "{name}");
        }
        for name in [
            "state.json",
            "state.json.tmpX",
            "state.json.tmp12",
            "state.json.tmp12.",
            "state.json.tmp.3",
            "state.json.tmp12.3.4",
            "state.json.tmp12.x",
            "state.json.tmp+12.3",
            "state.json.tmp12.+3",
            "state.json.tmp012.3",
            "state.json.tmp12.03",
            "state.json.tmp4294967296.3",
            "state.json.tmp12.18446744073709551616",
            "state.json.tmp-1.3",
            "state.json.tmp 12.3",
            ".tmp12.3",
            "",
        ] {
            assert!(!is_temp_name(name), "{name}");
        }
    }

    #[test]
    fn the_sweep_removes_own_and_dead_temps_and_keeps_live_and_unrelated_names() {
        let dir = temp_dir("sweep");
        let me = std::process::id();
        let live = me.wrapping_add(1);
        let dead = me.wrapping_add(2);
        let gone = [
            format!("state.json.tmp{me}.0"),
            format!("state.json.tmp{dead}.7"),
            format!("state.json.corrupt.tmp{dead}.1"),
            format!("state.crash.json.tmp{dead}.2"),
            format!("state.crash.json.corrupt.tmp{me}.3"),
        ];
        let kept = [
            "state.json".to_string(),
            format!("state.json.tmp{live}.4"),
            format!("state.json.corrupt.tmp{live}.5"),
            "state.json.tmpX".to_string(),
            format!("state.json.tmp{dead}"),
            format!("other.json.tmp{dead}.6"),
            format!("state.json.tmp{dead}.6.bak"),
        ];
        for name in gone.iter().chain(&kept) {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let asked = std::cell::RefCell::new(Vec::new());

        let swept = sweep_temps(&dir, &["state.json", "state.crash.json"], |pid| {
            asked.borrow_mut().push(pid);
            pid == live
        })
        .unwrap();

        let mut removed: Vec<String> = swept
            .iter()
            .map(|(path, outcome)| {
                assert!(outcome.is_ok(), "{path:?}: {outcome:?}");
                path.file_name().unwrap().to_string_lossy().into_owned()
            })
            .collect();
        removed.sort();
        let mut expected_gone = gone.to_vec();
        expected_gone.sort();
        assert_eq!(removed, expected_gone);
        let mut expected_kept = kept.to_vec();
        expected_kept.sort();
        assert_eq!(names_in(&dir), expected_kept);
        assert!(
            !asked.borrow().contains(&me),
            "자기 pid 는 살았는지 묻지 않고 지운다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sweeping_a_missing_folder_is_an_empty_sweep() {
        let dir = temp_dir("sweep-missing");
        let swept = sweep_temps(&dir.join("none"), &["state.json"], |_| {
            panic!("지울 후보가 없으면 묻지 않는다")
        })
        .unwrap();
        assert!(swept.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Atomic copy · setting aside ──

    #[test]
    fn copy_atomic_overwrites_the_target_and_leaves_no_temp() {
        let dir = temp_dir("copy");
        let from = dir.join("a.json");
        let to = dir.join("b.json");
        std::fs::write(&from, "from").unwrap();
        std::fs::write(&to, "earlier").unwrap();

        copy_atomic(&from, &to, ONCE).unwrap();

        assert_eq!(std::fs::read_to_string(&to).unwrap(), "from");
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "from");
        assert_eq!(
            names_in(&dir),
            vec!["a.json".to_string(), "b.json".to_string()]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_atomic_from_a_missing_file_touches_nothing() {
        let dir = temp_dir("copy-missing");
        let to = dir.join("b.json");
        std::fs::write(&to, "earlier").unwrap();

        let err = copy_atomic(&dir.join("none.json"), &to, ONCE).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "earlier");
        assert_eq!(names_in(&dir), vec!["b.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_aside_uses_one_fixed_name_and_overwrites_the_earlier_copy() {
        let dir = temp_dir("copy-aside");
        let path = dir.join("settings.json");
        let fixed = dir.join("settings.json.corrupt");
        std::fs::write(&path, "{broken").unwrap();
        std::fs::write(&fixed, "earlier").unwrap();

        assert_eq!(copy_aside(&path, ONCE).unwrap(), fixed);
        assert_eq!(std::fs::read_to_string(&fixed).unwrap(), "{broken");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{broken",
            "원본은 그 자리에"
        );

        std::fs::write(&path, "{BROKEN").unwrap();
        assert_eq!(copy_aside(&path, ONCE).unwrap(), fixed);
        assert_eq!(std::fs::read_to_string(&fixed).unwrap(), "{BROKEN");
        assert_eq!(
            names_in(&dir),
            vec![
                "settings.json".to_string(),
                "settings.json.corrupt".to_string()
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_read_only_source_is_copied_aside_into_a_writable_copy() {
        let dir = temp_dir("copy-read-only");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{broken").unwrap();
        let set_read_only = |path: &Path, read_only: bool| {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(read_only);
            std::fs::set_permissions(path, permissions).unwrap();
        };
        set_read_only(&path, true);

        let copied = copy_aside(&path, ONCE);

        set_read_only(&path, false);
        let to = copied.unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "{broken");
        assert!(!std::fs::metadata(&to).unwrap().permissions().readonly());
        std::fs::OpenOptions::new()
            .write(true)
            .open(&to)
            .expect("사본은 쓸 수 있다 — 다음 떠 두기가 덮는다");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Folder creation · sync ──

    #[test]
    fn a_write_creates_the_missing_folders_up_to_the_target() {
        let dir = temp_dir("mkdir");
        let folder = dir.join("shell").join("state");
        let path = folder.join("state.json");

        write_atomic(&path, b"new", ONCE).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&folder), vec!["state.json".to_string()]);

        // A folder deleted during the run is set up again by the next write.
        std::fs::remove_dir_all(dir.join("shell")).unwrap();
        write_atomic(&path, b"again", ONCE).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_written_file_syncs_its_folder_once_after_the_rename() {
        let dir = temp_dir("sync");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        let mut synced = Vec::new();

        let outcome = write_atomic_with(
            &path,
            b"new",
            &mut |attempt| attempt(),
            &mut |folder| {
                synced.push((
                    folder.to_path_buf(),
                    std::fs::read_to_string(&path).unwrap(),
                ));
                Ok(())
            },
            &|| false,
        )
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Written);
        assert_eq!(
            synced,
            vec![(dir.clone(), "new".to_string())],
            "갈아끼운 뒤 그 폴더를 한 번"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_folder_sync_does_not_fail_the_write() {
        let dir = temp_dir("sync-fail");
        let path = dir.join("state.json");

        let outcome = write_atomic_with(
            &path,
            b"new",
            &mut |attempt| attempt(),
            &mut |_| Err(io::Error::other("가짜 동기화 실패")),
            &|| false,
        )
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_copy_creates_the_missing_folder_of_the_copy_and_syncs_it() {
        let dir = temp_dir("copy-sync");
        let from = dir.join("a.json");
        std::fs::write(&from, "from").unwrap();
        let aside = dir.join("aside");
        let to = aside.join("b.json");
        let mut synced = Vec::new();

        copy_atomic_with(&from, &to, &mut |attempt| attempt(), &mut |folder| {
            synced.push(folder.to_path_buf());
            Ok(())
        })
        .unwrap();

        assert_eq!(std::fs::read_to_string(&to).unwrap(), "from");
        assert_eq!(synced, vec![aside.clone()]);
        assert_eq!(names_in(&aside), vec!["b.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_copy_from_a_missing_file_creates_no_folder() {
        let dir = temp_dir("copy-missing-folder");
        let to = dir.join("aside").join("b.json");

        let err = copy_atomic(&dir.join("none.json"), &to, ONCE).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            !dir.join("aside").exists(),
            "원본을 못 열면 아무것도 만들지 않는다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bare_file_name_lives_in_the_current_folder() {
        assert_eq!(folder_of(Path::new("a.json")), Path::new("."));
        assert_eq!(
            folder_of(&Path::new("dir").join("a.json")),
            Path::new("dir")
        );
    }

    // ── Read rules ──

    /// The rules of the test file — version key `v` · version 1 · 64 bytes · shape not checked.
    const SPEC: Spec = Spec {
        version_key: "v",
        current: 1,
        cap: 64,
        shape: any_shape,
    };

    /// An owner whose top-level `items` must be an array — it produces a shape failure.
    const ITEMS: Spec = Spec {
        shape: items_shape,
        ..SPEC
    };

    fn items_shape(_: u64, doc: &Map<String, Value>) -> Result<(), String> {
        match doc.get("items") {
            Some(Value::Array(_)) => Ok(()),
            _ => Err("`items` 가 배열이 아니다".to_string()),
        }
    }

    fn usable_doc(version: u64, json: &str) -> Parsed {
        let Value::Object(doc) = serde_json::from_str(json).unwrap() else {
            panic!("객체가 아니다: {json}")
        };
        Parsed::Usable { version, doc }
    }

    #[test]
    fn a_missing_version_key_reads_as_the_first_version() {
        assert_eq!(parse(r#"{"a":1}"#, &SPEC), usable_doc(1, r#"{"a":1}"#));
        assert_eq!(
            parse(r#"{"v":1,"a":1}"#, &SPEC),
            usable_doc(1, r#"{"v":1,"a":1}"#),
            "버전 키도 문서에 남는다"
        );
    }

    #[test]
    fn a_newer_version_is_newer_without_consulting_the_shape() {
        let spec = Spec {
            shape: |_, _| panic!("새 판은 모양을 보지 않는다"),
            ..SPEC
        };
        assert_eq!(parse(r#"{"v":2}"#, &spec), Parsed::Newer { found: 2 });
        assert_eq!(
            parse(r#"{"v":7.0,"items":"x"}"#, &spec),
            Parsed::Newer { found: 7 }
        );
        assert_eq!(
            parse(r#"{"v":18446744073709551615}"#, &spec),
            Parsed::Newer { found: u64::MAX }
        );
    }

    #[test]
    fn an_older_version_is_read_only_when_the_owners_shape_takes_it() {
        fn current_only(version: u64, _: &Map<String, Value>) -> Result<(), String> {
            if version == 2 {
                Ok(())
            } else {
                Err(format!("{version} 판을 읽을 리더가 없다"))
            }
        }
        let no_reader = Spec {
            current: 2,
            shape: current_only,
            ..SPEC
        };
        let with_reader = Spec { current: 2, ..SPEC };

        assert_eq!(
            parse(r#"{"v":1}"#, &no_reader),
            Parsed::Unusable("1 판을 읽을 리더가 없다".to_string())
        );
        assert_eq!(
            parse("{}", &no_reader),
            Parsed::Unusable("1 판을 읽을 리더가 없다".to_string()),
            "버전 키가 없으면 1 판이라 앞 판이다"
        );
        assert_eq!(parse(r#"{"v":2}"#, &no_reader), usable_doc(2, r#"{"v":2}"#));
        assert_eq!(
            parse(r#"{"v":1}"#, &with_reader),
            usable_doc(1, r#"{"v":1}"#)
        );
    }

    #[test]
    fn a_version_must_be_a_whole_number_of_at_least_one() {
        for raw in ["1", "1.0", "1e0"] {
            let text = format!(r#"{{"v":{raw}}}"#);
            assert_eq!(parse(&text, &SPEC), usable_doc(1, &text), "{raw}");
        }
        assert_eq!(parse(r#"{"v":2.0}"#, &SPEC), Parsed::Newer { found: 2 });
        for raw in [
            "0",
            "-0.0",
            "-1",
            "1.5",
            r#""1""#,
            "null",
            "true",
            "[1]",
            "18446744073709551616",
        ] {
            assert!(
                matches!(
                    parse(&format!(r#"{{"v":{raw}}}"#), &SPEC),
                    Parsed::Unusable(_)
                ),
                "{raw}"
            );
        }
    }

    #[test]
    fn one_leading_bom_is_ignored_and_a_second_is_not() {
        assert_eq!(
            parse("\u{feff}{\"v\":1}", &SPEC),
            usable_doc(1, r#"{"v":1}"#)
        );
        assert!(matches!(
            parse("\u{feff}\u{feff}{\"v\":1}", &SPEC),
            Parsed::Unusable(_)
        ));
    }

    #[test]
    fn non_json_and_non_object_text_is_unusable() {
        for text in ["", "{", "[]", "1", r#""x""#, "null"] {
            assert!(
                matches!(parse(text, &SPEC), Parsed::Unusable(_)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn text_over_the_cap_is_unusable_however_it_was_read() {
        let spec = Spec { cap: 8, ..SPEC };
        assert_eq!(parse(r#"{"a":12}"#, &spec), usable_doc(1, r#"{"a":12}"#));
        assert!(matches!(parse(r#"{"a":123}"#, &spec), Parsed::Unusable(_)));
    }

    #[test]
    fn classify_sorts_read_results_into_missing_unusable_failed_and_parsed() {
        assert!(matches!(
            classify(Err(io::ErrorKind::NotFound.into()), &SPEC),
            Loaded::Missing
        ));
        assert!(matches!(
            classify(
                Err(io::Error::new(io::ErrorKind::InvalidData, "상한")),
                &SPEC
            ),
            Loaded::Parsed(Parsed::Unusable(_))
        ));
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert!(
                matches!(classify(Err(kind.into()), &SPEC), Loaded::Failed(e) if e.kind() == kind),
                "{kind:?}"
            );
        }
        assert!(matches!(
            classify(Ok(r#"{"v":2}"#.to_string()), &SPEC),
            Loaded::Parsed(Parsed::Newer { found: 2 })
        ));
    }

    #[test]
    fn load_reads_real_files_and_creates_nothing() {
        let dir = temp_dir("load");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"v":1}"#).unwrap();
        assert!(matches!(
            load(&path, &SPEC, ONCE),
            Loaded::Parsed(Parsed::Usable { version: 1, .. })
        ));

        std::fs::write(&path, vec![b' '; 65]).unwrap();
        assert!(
            matches!(
                load(&path, &SPEC, ONCE),
                Loaded::Parsed(Parsed::Unusable(_))
            ),
            "상한 초과"
        );
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(
            matches!(
                load(&path, &SPEC, ONCE),
                Loaded::Parsed(Parsed::Unusable(_))
            ),
            "UTF-8 아님"
        );

        let absent = dir.join("none").join("settings.json");
        assert!(matches!(load(&absent, &SPEC, ONCE), Loaded::Missing));
        assert_eq!(
            judge(&absent, &SPEC, ONCE, Claim::NotYet),
            WritePolicy::Write
        );
        assert!(
            !dir.join("none").exists(),
            "적재 · 판정은 폴더를 만들지 않는다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_shape_failure_is_unusable_on_every_judging_path() {
        let dir = temp_dir("shape");
        let path = dir.join("agents.json");
        let text = r#"{"v":1,"items":"x"}"#;
        std::fs::write(&path, text).unwrap();
        let broken = Parsed::Unusable("`items` 가 배열이 아니다".to_string());

        assert_eq!(parse(text, &ITEMS), broken);
        assert!(matches!(
            classify(Ok(text.to_string()), &ITEMS),
            Loaded::Parsed(parsed) if parsed == broken
        ));
        assert!(matches!(
            load(&path, &ITEMS, ONCE),
            Loaded::Parsed(parsed) if parsed == broken
        ));
        assert_eq!(
            judge(&path, &ITEMS, ONCE, Claim::Adopted),
            WritePolicy::CopyAsideFirst
        );
        // The same file is usable for an owner that does not check shape — a contrast showing the decision above came from the shape check.
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Write
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Write decisions ──

    #[test]
    fn the_write_policy_table() {
        let usable = || Loaded::Parsed(parse("{}", &SPEC));
        let broken = || Loaded::Parsed(Parsed::Unusable("x".to_string()));
        let newer = || Loaded::Parsed(Parsed::Newer { found: 3 });
        let failed = || Loaded::Failed(denied());
        let refused_newer = WritePolicy::Refuse(Refused::Newer { found: 3 });
        let refused_read = WritePolicy::Refuse(Refused::ReadFailed);
        let cases = [
            ("없음", Claim::Adopted, Loaded::Missing, WritePolicy::Write),
            ("없음", Claim::NotYet, Loaded::Missing, WritePolicy::Write),
            ("쓸 수 있음", Claim::Adopted, usable(), WritePolicy::Write),
            (
                "쓸 수 있음",
                Claim::NotYet,
                usable(),
                WritePolicy::CopyAsideFirst,
            ),
            (
                "손상",
                Claim::Adopted,
                broken(),
                WritePolicy::CopyAsideFirst,
            ),
            ("손상", Claim::NotYet, broken(), WritePolicy::CopyAsideFirst),
            ("새 판", Claim::Adopted, newer(), refused_newer),
            ("새 판", Claim::NotYet, newer(), refused_newer),
            ("읽기 실패", Claim::Adopted, failed(), refused_read),
            ("읽기 실패", Claim::NotYet, failed(), refused_read),
        ];
        for (file, claim, loaded, expected) in cases {
            assert_eq!(loaded.write_policy(claim), expected, "{file} · {claim:?}");
        }
    }

    #[test]
    fn judge_reads_the_file_as_it_is_now_on_every_call() {
        let dir = temp_dir("judge");
        let path = dir.join("presets.json");
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Write
        );

        std::fs::write(&path, r#"{"v":1}"#).unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Write
        );
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::NotYet),
            WritePolicy::CopyAsideFirst
        );

        std::fs::write(&path, r#"{"v":2}"#).unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Refuse(Refused::Newer { found: 2 })
        );

        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::CopyAsideFirst
        );

        // It is a folder location, so open or read fails regardless of content.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Refuse(Refused::ReadFailed)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn check_cap_takes_exactly_the_cap_and_refuses_one_byte_more() {
        assert!(check_cap(64, &SPEC).is_ok());
        assert_eq!(
            check_cap(65, &SPEC).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
