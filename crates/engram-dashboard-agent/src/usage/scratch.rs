//! The scratch folder of one query — the querier writes a config file in it and uses it as the child's working folder. Deleted when done.
//!
//! ★It is an exclusive create★ — the name is random, and if the name already exists it does not use that folder and draws a new name. A file the child
//!   trusts and reads is placed inside it, so reusing a folder planted in advance by someone else would let that file be swapped.
//! ★A deletion failure is ignored (after a short retry if locked)★ — the leftover folder is swept away by the receiving side at startup with [`sweep_stale_scratch`].
//!   (Windows cannot delete a file someone has open or a working folder — so we kill the child first and wait.)

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_platform::fs::{retry_busy, Retry};
use uuid::Uuid;

use super::ProbeError;

/// Folder name = this prefix + 32 lowercase hex digits of a uuid v4. The sweep deletes only names that fit this grammar — widening it to a prefix match
/// would delete even things placed by hand.
const SCRATCH_PREFIX: &str = "probe-";
const SCRATCH_HEX_LEN: usize = 32;

/// Upper bound on drawing anew when names collide. A random name collides only when someone planted it in advance, so a few tries are enough.
const SCRATCH_CREATE_ATTEMPTS: usize = 8;

/// The scratch folder of one query. Deletes it whole on drop (failure is ignored).
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Creates a new folder under `root` exclusively. Creates `root` if it does not exist.
    pub fn create(root: &Path) -> Result<Self, ProbeError> {
        Self::create_named(root, || {
            format!("{SCRATCH_PREFIX}{}", Uuid::new_v4().simple())
        })
    }

    fn create_named(
        root: &Path,
        mut next_name: impl FnMut() -> String,
    ) -> Result<Self, ProbeError> {
        fs::create_dir_all(root)
            .map_err(|e| ProbeError::Io(format!("임시 폴더의 부모를 만들지 못했다: {e}")))?;
        for _ in 0..SCRATCH_CREATE_ATTEMPTS {
            let path = root.join(next_name());
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    tracing::debug!(path = %path.display(), "임시 폴더 이름이 이미 있다 — 새 이름을 뽑는다");
                }
                Err(e) => return Err(ProbeError::Io(format!("임시 폴더를 만들지 못했다: {e}"))),
            }
        }
        Err(ProbeError::Io(format!(
            "임시 폴더 이름이 {SCRATCH_CREATE_ATTEMPTS}번 모두 이미 있었다"
        )))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The number and interval of retries the drop makes at deleting — first try + 19 times × 10 ms (about 200 ms).
///
/// Right after the direct child ends, a grandchild may still hold the working folder — deleting immediately then fails, and a few ms
/// later it works (measured 2026-09-27, 40/40 runs · success after about 5 ms — the grounds of the budget of about 200 ms. It was carried in the same commit c7dcba33 as the querier's Job tree
/// exit wait (`wait_tree_gone`), and there is no record of whether the measurement was before or after that wait).
// ADR-0291 R14: pilot 2026-10-10 (rustc 1.95 · NTFS) — a synthetic reproduction with a grandchild holding the working folder failed only with OS error 32
//   (`ERROR_SHARING_VIOLATION` — `is_busy` bites on it) and 145 never appeared. The real query path releases the folder after the Job tree has
//   fully ended, so it was deleted on the first try 6/6 times — so we do not widen the decision. The delete-pending state (145) of FAT/exFAT
//   was not measured — a folder left that way is reaped by the sweep at startup.
const DELETE_RETRY: Retry = Retry {
    retries: 19,
    pause: Duration::from_millis(10),
};

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // ADR-0291: a missing folder is success, and an error that is not a lock is not retried.
        let outcome = retry_busy(DELETE_RETRY, || match fs::remove_dir_all(&self.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        });
        if let Err(err) = outcome {
            tracing::debug!(path = %self.path.display(), "임시 폴더 삭제 실패 — 기동 때 쓸기가 거둔다: {err}");
        }
    }
}

fn is_scratch_name(name: &str) -> bool {
    name.strip_prefix(SCRATCH_PREFIX).is_some_and(|hex| {
        hex.len() == SCRATCH_HEX_LEN
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Deletes the leftover scratch folders under `root` — called when no query is running at all (at startup).
///
/// What it deletes is only **folders** that fit the [`ScratchDir`] name grammar (files · links · other names are left). If `root` does not exist
/// there is nothing to do. A failure leaves only a log — a cleanup failure does not block startup.
pub fn sweep_stale_scratch(root: &Path) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!(dir = %root.display(), "임시 폴더 쓸기 미실행 — 지난 조회의 폴더가 남아 있을 수 있다: {e}");
            return;
        }
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_scratch_name) {
            continue;
        }
        // `file_type` does not follow links — a link carrying a folder name is not deleted.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        match fs::remove_dir_all(&path) {
            Ok(()) => removed += 1,
            Err(e) => {
                tracing::debug!(path = %path.display(), "임시 폴더 쓸기: 삭제 실패(계속): {e}")
            }
        }
    }
    if removed > 0 {
        tracing::info!(count = removed, "임시 폴더 쓸기: 지난 조회의 폴더를 지웠다");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::testing::TempRoot;
    use std::thread;

    #[test]
    fn create_makes_a_fresh_dir_with_the_sweepable_name_and_drop_removes_it() {
        let root = TempRoot::new("scratch-create");
        let scratch = ScratchDir::create(&root.path().join("nested")).expect("생성");
        let path = scratch.path().to_path_buf();
        assert!(path.is_dir());
        let name = path.file_name().and_then(|n| n.to_str()).expect("이름");
        assert!(is_scratch_name(name), "쓸기 문법과 어긋난 이름: {name}");
        fs::write(path.join("config.json"), "{}").expect("파일");

        drop(scratch);
        assert!(!path.exists(), "drop 뒤에도 남았다");
    }

    #[test]
    fn a_name_that_already_exists_is_never_reused() {
        let root = TempRoot::new("scratch-taken");
        let planted = root.path().join("taken");
        fs::create_dir(&planted).expect("미리 심은 폴더");
        fs::write(planted.join("config.json"), "planted").expect("심은 파일");

        let mut names = ["taken", "fresh"].into_iter();
        let scratch =
            ScratchDir::create_named(root.path(), || names.next().expect("이름").to_owned())
                .expect("생성");
        assert_eq!(scratch.path(), root.path().join("fresh"));

        drop(scratch);
        assert_eq!(
            fs::read_to_string(planted.join("config.json")).expect("심은 파일"),
            "planted",
            "남의 폴더를 건드렸다"
        );
    }

    #[test]
    fn creation_gives_up_after_bounded_attempts() {
        let root = TempRoot::new("scratch-exhausted");
        fs::create_dir(root.path().join("same")).expect("미리 심은 폴더");
        let mut calls = 0usize;
        let result = ScratchDir::create_named(root.path(), || {
            calls += 1;
            "same".to_owned()
        });
        assert!(matches!(result, Err(ProbeError::Io(_))), "{result:?}");
        assert_eq!(calls, SCRATCH_CREATE_ATTEMPTS);
    }

    #[test]
    fn drop_ignores_a_dir_that_is_already_gone() {
        let root = TempRoot::new("scratch-gone");
        let scratch = ScratchDir::create(root.path()).expect("생성");
        fs::remove_dir_all(scratch.path()).expect("먼저 지움");
        drop(scratch);
    }

    /// Windows cannot delete the folder if someone has a file inside open exclusively — drop swallows that failure.
    #[cfg(windows)]
    #[test]
    fn drop_ignores_a_delete_failure() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = TempRoot::new("scratch-locked");
        let scratch = ScratchDir::create(root.path()).expect("생성");
        let path = scratch.path().to_path_buf();
        let locked = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(path.join("held.txt"))
            .expect("배타 열기");

        drop(scratch);
        assert!(
            path.join("held.txt").exists(),
            "지우기가 실패해야 하는 조건이 안 섰다"
        );

        drop(locked);
        fs::remove_dir_all(&path).expect("정리");
    }

    /// If the lock is released soon during drop, the retried delete succeeds — the case where a grandchild lets go of the working folder a little late.
    #[cfg(windows)]
    #[test]
    fn drop_retries_a_delete_that_fails_only_briefly() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = TempRoot::new("scratch-brief-lock");
        let scratch = ScratchDir::create(root.path()).expect("생성");
        let path = scratch.path().to_path_buf();
        let locked = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(path.join("held.txt"))
            .expect("배타 열기");
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            drop(locked);
        });

        drop(scratch);
        release.join().expect("잠금 풀기");
        assert!(!path.exists(), "잠금이 풀린 뒤에도 남았다");
    }

    #[test]
    fn sweep_removes_only_scratch_dirs() {
        let root = TempRoot::new("scratch-sweep");
        let stale = ScratchDir::create(root.path()).expect("생성");
        let stale_path = stale.path().to_path_buf();
        fs::write(stale_path.join("config.json"), "{}").expect("파일");
        // Pretends to be a leftover folder — forgets the value so that drop does not delete it.
        std::mem::forget(stale);

        let other_dir = root.path().join("keep-me");
        fs::create_dir(&other_dir).expect("다른 폴더");
        let upper = root
            .path()
            .join(format!("{SCRATCH_PREFIX}{}", "A".repeat(SCRATCH_HEX_LEN)));
        fs::create_dir(&upper).expect("대문자 이름");
        let same_name_file = root
            .path()
            .join(format!("{SCRATCH_PREFIX}{}", Uuid::new_v4().simple()));
        fs::write(&same_name_file, "not a dir").expect("같은 문법의 파일");

        sweep_stale_scratch(root.path());

        assert!(!stale_path.exists(), "남은 임시 폴더를 안 지웠다");
        assert!(other_dir.is_dir());
        assert!(upper.is_dir(), "문법 밖 이름을 지웠다");
        assert!(same_name_file.is_file(), "폴더가 아닌 것을 지웠다");
    }

    #[test]
    fn sweep_of_a_missing_root_is_a_no_op() {
        let root = TempRoot::new("scratch-sweep-missing");
        sweep_stale_scratch(&root.path().join("absent"));
    }
}
