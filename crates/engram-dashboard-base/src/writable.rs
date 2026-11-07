//! Can a folder really be written to — creates a file in it, loads bytes, and deletes it (write probe).
//!
//! ★Several processes · several calls may check the same folder at the same time★ — the probe name differs per process ·
//! call. A race where the folder vanishes mid-check is not retried by [`probe_write_in`] itself — the
//! caller wraps it in [`retry_if_vanished`] to try once more. Whether to create the folder and which error to fold a failure into is
//! also the caller's call.
// ADR-0269
// ADR-0282

use std::path::{Path, PathBuf};

/// The leading part of the write-probe file name. A **tail that differs per process · call** follows — a file in the folder
/// that starts with this prefix is probe residue.
///
/// ★Do not use a fixed name (do not revive it)★: several processes may probe the same folder at the same time, and if the name is
/// the same the loser fails at `create_new` and **decides a healthy folder is "not writable"**. On top of that, under an
/// ACL that blocks only deletion, one leftover file permanently blocks every later probe.
pub const WRITE_PROBE_PREFIX: &str = ".engram-write-probe-";

/// ★Do not write with 0 bytes★: even when the disk is full or the quota is exhausted, creating a zero-length file often
/// succeeds — then the probe passes and the first real write fails. The check only counts if bytes are actually loaded.
const WRITE_PROBE_PAYLOAD: &[u8] = b"engram-write-probe";

/// ★It goes all the way to sync_all★: a write that only sits in the cache passes straight through a full disk · an exhausted quota —
/// only when the bytes actually land is "can write" true.
fn write_probe_payload(mut f: std::fs::File) -> std::io::Result<()> {
    use std::io::Write;
    f.write_all(WRITE_PROBE_PAYLOAD)?;
    f.sync_all()
}

/// A probe path unique to this process · this call. The pid tells apart processes, the counter tells apart concurrent calls of the same process.
fn probe_path(dir: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{WRITE_PROBE_PREFIX}{}-{n}", std::process::id()))
}

/// Actually tries whether a file can be created **inside** `dir` and bytes written to it. `dir` must already exist.
///
/// The probe file is created with `create_new` — it never overwrites an existing file of the same name. If it already exists,
/// it is a trace of a failed cleanup last time, so it is deleted and tried once more (existence itself is not a failure reason).
///
/// ★It may be left behind (contract)★: the created file is deleted, but in a folder that allows creation and blocks deletion **the deletion fails and
/// the file remains**. Then a warning log is left and it is treated as success — that it can be written has already been proven.
pub fn probe_write_in(dir: &Path) -> std::io::Result<()> {
    probe_write_at(&probe_path(dir))
}

/// The body that takes the probe path as an argument — for a test to deterministically reproduce the failure branch "cannot create a file with that name",
/// the name must be settable. The only production entry point is [`probe_write_in`].
///
/// ★Why it returns `io::Result`★: the caller must see the `ErrorKind` — a `NotFound` where the folder vanished mid-check
/// is a race, not "cannot write", and lumping the two here would remove any way to tell them apart.
fn probe_write_at(probe: &Path) -> std::io::Result<()> {
    let create = |p: &Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
    };

    let file = match create(probe) {
        Ok(f) => f,
        // It may be a trace left by a previous run with the same pid whose cleanup failed — delete it and try once more.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(probe)?;
            create(probe)?
        }
        Err(e) => return Err(e),
    };

    let written = write_probe_payload(file);
    let cleanup = std::fs::remove_file(probe);
    written?;
    if let Err(e) = cleanup {
        tracing::warn!(
            "쓰기 프로브 파일 삭제 실패({}) — 남긴다: {e}",
            probe.display()
        );
    }
    Ok(())
}

/// If `once` fails with `NotFound`, it is called **just once** more and that result is returned. Any other result is the first result
/// as is.
///
/// ★Why it is needed (a real race)★: when two checks that revert a folder they made overlap, right after B sees the folder A made as
/// "existing", A deletes it and B's probe falls over with `NotFound`. That is timing, not a permission problem, so
/// a healthy folder must not be decided "not writable".
pub fn retry_if_vanished(mut once: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    match once() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => once(),
        first => first,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    /// A unique folder dedicated to the probe (so parallel test runs do not step on each other).
    fn fresh_probe_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "engram-writable-probe-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// ★It aims at the reason the probe exists★: the folder exists but **a file cannot be created** in it. A check that only
    /// looks at folder creation passes here. To reproduce deterministically without ACL manipulation, the probe name is injected and
    /// that name is pre-claimed as a folder — a file can neither be created nor deleted with that name (measured: both code 5).
    #[test]
    fn writable_probe_rejects_a_name_it_cannot_create() {
        let dir = fresh_probe_dir("nofile");
        std::fs::create_dir_all(&dir).expect("폴더 생성");
        let taken = dir.join("taken-by-a-directory");
        std::fs::create_dir_all(&taken).expect("프로브 이름을 폴더로 선점");
        let err = probe_write_at(&taken).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        // It must be told apart from a race (NotFound) so that the retry does not spin in vain.
        assert_ne!(
            err.kind(),
            ErrorKind::NotFound,
            "폴더가 있는데 파일을 못 만드는 것은 경합이 아니다: {err:?}"
        );
    }

    /// A probe left because a previous run with the same pid failed to clean up is not a failure reason — delete it and create it again.
    #[test]
    fn writable_probe_recovers_from_a_leftover_probe_file() {
        let dir = fresh_probe_dir("leftover");
        std::fs::create_dir_all(&dir).expect("폴더 생성");
        let leftover = dir.join(format!("{WRITE_PROBE_PREFIX}{}-0", std::process::id()));
        std::fs::write(&leftover, b"leftover").expect("잔여 프로브 생성");
        let got = probe_write_at(&leftover);
        let still_there = leftover.exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.is_ok(), "잔여 프로브는 실패 사유가 아님: {got:?}");
        assert!(!still_there, "잔여 프로브까지 정리돼야");
    }

    /// Decision table — each line = the results `once` yields in turn (`None` = success) · the expected call count · the expected error. The overlapping-check
    /// race test (consumer side) passes through this table only probabilistically, so it is measured deterministically here.
    #[test]
    fn retry_if_vanished_calls_again_only_after_not_found_and_only_once() {
        use ErrorKind::{NotFound, PermissionDenied};
        let table: [(&[Option<ErrorKind>], usize, Option<ErrorKind>); 5] = [
            (&[None], 1, None),
            (&[Some(PermissionDenied)], 1, Some(PermissionDenied)),
            (&[Some(NotFound), None], 2, None),
            (
                &[Some(NotFound), Some(PermissionDenied)],
                2,
                Some(PermissionDenied),
            ),
            (&[Some(NotFound), Some(NotFound), None], 2, Some(NotFound)),
        ];
        for (script, want_calls, want) in table {
            let mut calls = 0;
            let got = retry_if_vanished(|| {
                let step = script[calls];
                calls += 1;
                match step {
                    None => Ok(()),
                    Some(kind) => Err(std::io::Error::from(kind)),
                }
            });
            assert_eq!(calls, want_calls, "호출 수: {script:?}");
            assert_eq!(got.err().map(|e| e.kind()), want, "결과: {script:?}");
        }
    }
}
