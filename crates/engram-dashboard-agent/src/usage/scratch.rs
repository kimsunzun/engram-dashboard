//! The scratch folder of one query — the querier writes a config file in it and uses it as the child's working folder. Deleted when done.
//!
//! ★Exclusive create★ — the name is random, and an existing name is not reused (a new name is drawn): a file the child trusts and reads
//!   sits inside, so a folder planted in advance by someone else would let that file be swapped.
//! ★A deletion failure is ignored (after a short retry if locked)★ — the leftover is swept at startup by the receiving side with
//!   [`sweep_stale_scratch`]. (Windows cannot delete an open file or a working folder — so the child is killed first and waited for.)

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_platform::fs::{retry_busy, Retry};
use uuid::Uuid;

use super::ProbeError;

/// Folder name = this prefix + 32 lowercase hex digits of a uuid v4. The sweep deletes only names that fit this grammar — widening it to a
/// prefix match would delete even things placed by hand.
const SCRATCH_PREFIX: &str = "probe-";
const SCRATCH_HEX_LEN: usize = 32;

/// Upper bound on drawing anew when names collide. A random name collides only when someone planted it in advance, so a few tries are enough.
const SCRATCH_CREATE_ATTEMPTS: usize = 8;

#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub fn create(root: &Path) -> Result<Self, ProbeError> {
        Self::create_named(root, || {
            format!("{SCRATCH_PREFIX}{}", Uuid::new_v4().simple())
        })
    }

    fn create_named(
        root: &Path,
        mut next_name: impl FnMut() -> String,
    ) -> Result<Self, ProbeError> {
        fs::create_dir_all(root).map_err(|e| {
            ProbeError::Io(format!(
                "could not create the parent of the scratch folder: {e}"
            ))
        })?;
        for _ in 0..SCRATCH_CREATE_ATTEMPTS {
            let path = root.join(next_name());
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    tracing::debug!(path = %path.display(), "scratch folder name already exists — drawing a new name");
                }
                Err(e) => {
                    return Err(ProbeError::Io(format!(
                        "could not create the scratch folder: {e}"
                    )))
                }
            }
        }
        Err(ProbeError::Io(format!(
            "the scratch folder name already existed on all {SCRATCH_CREATE_ATTEMPTS} attempts"
        )))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The number and interval of drop's deletion retries — first try + 19 times × 10 ms (about 200 ms).
///
/// Right after the direct child ends, a grandchild may still hold the working folder — immediate deletion fails and a few ms later works
/// (measured 2026-09-27, 40/40 runs, success after ~5 ms — the grounds of the ~200 ms budget).
// ADR-0291 R14: pilot 2026-10-10 (rustc 1.95 · NTFS) — a synthetic grandchild-held reproduction failed only with OS error 32
//   (`ERROR_SHARING_VIOLATION`, which `is_busy` catches); 145 never appeared. The real query path releases the folder after the Job tree
//   ended and deleted on the first try 6/6, so the decision is not widened. FAT/exFAT's delete-pending (145) was not measured — a folder
//   left that way is reaped by the startup sweep.
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
            tracing::debug!(path = %self.path.display(), "scratch folder delete failed — the sweep at startup will collect it: {err}");
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

/// Deletes the leftover scratch folders under `root` — called when no query is running (at startup).
///
/// Deletes only **folders** fitting the [`ScratchDir`] name grammar (files · links · other names are left). A missing `root` is a no-op. A
/// failure leaves only a log — cleanup failure does not block startup.
pub fn sweep_stale_scratch(root: &Path) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            tracing::warn!(dir = %root.display(), "scratch folder sweep not run — folders from past queries may remain: {e}");
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
                tracing::debug!(path = %path.display(), "scratch folder sweep: delete failed (continuing): {e}")
            }
        }
    }
    if removed > 0 {
        tracing::info!(
            count = removed,
            "scratch folder sweep: removed folders from past queries"
        );
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
        let scratch = ScratchDir::create(&root.path().join("nested")).expect("create");
        let path = scratch.path().to_path_buf();
        assert!(path.is_dir());
        let name = path.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(
            is_scratch_name(name),
            "name does not match the sweep syntax: {name}"
        );
        fs::write(path.join("config.json"), "{}").expect("file");

        drop(scratch);
        assert!(!path.exists(), "still there after drop");
    }

    #[test]
    fn a_name_that_already_exists_is_never_reused() {
        let root = TempRoot::new("scratch-taken");
        let planted = root.path().join("taken");
        fs::create_dir(&planted).expect("pre-planted folder");
        fs::write(planted.join("config.json"), "planted").expect("planted file");

        let mut names = ["taken", "fresh"].into_iter();
        let scratch =
            ScratchDir::create_named(root.path(), || names.next().expect("name").to_owned())
                .expect("create");
        assert_eq!(scratch.path(), root.path().join("fresh"));

        drop(scratch);
        assert_eq!(
            fs::read_to_string(planted.join("config.json")).expect("planted file"),
            "planted",
            "touched someone else's folder"
        );
    }

    #[test]
    fn creation_gives_up_after_bounded_attempts() {
        let root = TempRoot::new("scratch-exhausted");
        fs::create_dir(root.path().join("same")).expect("pre-planted folder");
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
        let scratch = ScratchDir::create(root.path()).expect("create");
        fs::remove_dir_all(scratch.path()).expect("remove first");
        drop(scratch);
    }

    #[cfg(windows)]
    #[test]
    fn drop_ignores_a_delete_failure() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = TempRoot::new("scratch-locked");
        let scratch = ScratchDir::create(root.path()).expect("create");
        let path = scratch.path().to_path_buf();
        let locked = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(path.join("held.txt"))
            .expect("exclusive open");

        drop(scratch);
        assert!(
            path.join("held.txt").exists(),
            "the condition under which the delete must fail was not set up"
        );

        drop(locked);
        fs::remove_dir_all(&path).expect("cleanup");
    }

    /// If the lock is released soon during drop, the retried delete succeeds — the case where a grandchild lets go of the working folder a
    ///   little late.
    #[cfg(windows)]
    #[test]
    fn drop_retries_a_delete_that_fails_only_briefly() {
        use std::os::windows::fs::OpenOptionsExt;

        let root = TempRoot::new("scratch-brief-lock");
        let scratch = ScratchDir::create(root.path()).expect("create");
        let path = scratch.path().to_path_buf();
        let locked = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(path.join("held.txt"))
            .expect("exclusive open");
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            drop(locked);
        });

        drop(scratch);
        release.join().expect("release the lock");
        assert!(!path.exists(), "still there after the lock was released");
    }

    #[test]
    fn sweep_removes_only_scratch_dirs() {
        let root = TempRoot::new("scratch-sweep");
        let stale = ScratchDir::create(root.path()).expect("create");
        let stale_path = stale.path().to_path_buf();
        fs::write(stale_path.join("config.json"), "{}").expect("file");
        // Pretends to be a leftover folder — forgets the value so that drop does not delete it.
        std::mem::forget(stale);

        let other_dir = root.path().join("keep-me");
        fs::create_dir(&other_dir).expect("other folder");
        let upper = root
            .path()
            .join(format!("{SCRATCH_PREFIX}{}", "A".repeat(SCRATCH_HEX_LEN)));
        fs::create_dir(&upper).expect("uppercase name");
        let same_name_file = root
            .path()
            .join(format!("{SCRATCH_PREFIX}{}", Uuid::new_v4().simple()));
        fs::write(&same_name_file, "not a dir").expect("file with the same syntax");

        sweep_stale_scratch(root.path());

        assert!(
            !stale_path.exists(),
            "did not remove the leftover scratch folder"
        );
        assert!(other_dir.is_dir());
        assert!(upper.is_dir(), "removed a name outside the syntax");
        assert!(
            same_name_file.is_file(),
            "removed something that is not a folder"
        );
    }

    #[test]
    fn sweep_of_a_missing_root_is_a_no_op() {
        let root = TempRoot::new("scratch-sweep-missing");
        sweep_stale_scratch(&root.path().join("absent"));
    }
}
