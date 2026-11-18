//! Shell run lock `<shell run folder>\state.lock` (TRD S21-storage §6-5 ① · §6-6 · I1) — the boot stage takes it before reading the state files and releases it after the normal shutdown write (`Final`). The single-instance gate alone is not enough: that plugin releases the mutex at `RunEvent::Exit` before our `Final`, so a new instance passes the gate while the previous one writes its shutdown (§6-5 fact ②); this lock puts it behind that last write.
//! The app comes up even if it cannot take it: if the folder · file cannot be opened or the lock cannot be taken within [`LOCK_WAIT`], it logs and proceeds without it (D8 spirit · §12 R11). The only remaining risk is crossing with the previous instance's `Final`, and whichever wins, the file is intact.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::saver::REPLY_DEADLINE;

pub const LOCK_FILE: &str = "state.lock";

/// Deadline of the previous instance's normal shutdown write ([`REPLY_DEADLINE`]) + margin. If short, the new instance does not wait for that write.
pub const LOCK_WAIT: Duration = Duration::from_secs(3);
const _: () = assert!(LOCK_WAIT.as_millis() > REPLY_DEADLINE.as_millis());

/// Interval of retrying while it is held — when the previous instance releases it, it is taken within this.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// The lock held — the only way to release is drop (P3b3 = after `Final` · for a guard without `Final`). When the process dies the OS releases it.
#[derive(Debug)]
pub struct StateLock {
    file: File,
    path: PathBuf,
}

impl Drop for StateLock {
    // Release explicitly before closing the handle — Windows `LockFileEx` docs: the OS releases a closed handle's lock, but the timing depends on system resources, so a new instance waiting right after could be blocked that much longer.
    fn drop(&mut self) {
        if let Err(e) = self.file.unlock() {
            tracing::warn!(
                module = "state",
                path = %self.path.display(),
                error = %e,
                "could not release the shell run lock explicitly — the OS releases it when the handle closes"
            );
        }
    }
}

/// Creates `run_dir` (if missing) and takes the [`LOCK_FILE`] inside it exclusively, retrying while held until [`LOCK_WAIT`]. `None` = proceeds without the lock (the reason was already logged).
pub fn acquire(run_dir: &Path) -> Option<StateLock> {
    acquire_with(run_dir, LOCK_WAIT, Instant::now, std::thread::sleep)
}

fn acquire_with(
    run_dir: &Path,
    wait: Duration,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
) -> Option<StateLock> {
    let path = run_dir.join(LOCK_FILE);
    let proceed_without = |what: &str, error: &std::io::Error| {
        tracing::warn!(
            module = "state",
            path = %path.display(),
            error = %error,
            "shell run lock — {what} — proceeding without the lock (may cross with the previous instance's normal exit write)"
        );
    };
    if let Err(e) = std::fs::create_dir_all(run_dir) {
        proceed_without("cannot create the folder", &e);
        return None;
    }
    // The content is neither written nor read — `truncate(false)` so it is not emptied.
    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) => {
            proceed_without("cannot open the file", &e);
            return None;
        }
    };
    let start = now();
    // Writes 「took it after waiting」 only when it was blocked at least once — the difference of the two times is almost never 0.
    let mut contended = false;
    loop {
        match file.try_lock() {
            Ok(()) => {
                let waited = now().saturating_duration_since(start);
                if contended {
                    tracing::info!(
                        module = "state",
                        waited_ms = waited.as_millis() as u64,
                        "the previous instance released the shell run lock and it was taken"
                    );
                }
                return Some(StateLock { file, path });
            }
            Err(TryLockError::WouldBlock) => {
                contended = true;
                let waited = now().saturating_duration_since(start);
                if waited >= wait {
                    tracing::warn!(
                        module = "state",
                        path = %path.display(),
                        waited_ms = waited.as_millis() as u64,
                        "shell run lock — another instance holds it and does not release it — proceeding without the lock (may cross with the previous instance's normal exit write)"
                    );
                    return None;
                }
                sleep(RETRY_PAUSE.min(wait - waited));
            }
            Err(TryLockError::Error(e)) => {
                proceed_without("cannot lock", &e);
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Creates the contention with two handles of the same process instead of another process: the std lock is `LockFileEx` on Windows (per handle) and `flock` on Unix (per open file description), so even two separately opened handles block each other. So this measures "waiting · giving up · taking it when another handle holds it", not real contention between processes (a boot overlapping the previous instance's shutdown) — that is the P3b3 GUI measurement (§12 R11).

    use std::cell::Cell;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "engram-state-lock-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    /// Fake time where sleeping only pushes the clock that much — the test does not actually sleep 3 seconds.
    struct FakeTime {
        now: Cell<Instant>,
        sleeps: Cell<u32>,
        start: Instant,
    }

    impl FakeTime {
        fn new() -> Self {
            let start = Instant::now();
            Self {
                now: Cell::new(start),
                sleeps: Cell::new(0),
                start,
            }
        }

        fn sleep(&self, d: Duration) {
            self.now.set(self.now.get() + d);
            self.sleeps.set(self.sleeps.get() + 1);
        }

        fn elapsed(&self) -> Duration {
            self.now.get() - self.start
        }
    }

    #[test]
    fn a_missing_run_folder_is_made_and_the_lock_is_taken_at_once() {
        let root = temp_dir("fresh");
        let run_dir = root.join("shell").join("run");
        let time = FakeTime::new();

        let lock = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(lock.is_some());
        assert!(run_dir.join(LOCK_FILE).is_file());
        assert_eq!(time.sleeps.get(), 0);
        drop(lock);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_held_lock_is_retried_for_the_wait_and_then_given_up() {
        let run_dir = temp_dir("held");
        let holder = acquire(&run_dir).expect("the first handle takes it");
        let time = FakeTime::new();

        let second = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(second.is_none(), "if it is not released even after waiting, proceeds without the lock");
        assert_eq!(time.elapsed(), LOCK_WAIT);
        assert_eq!(
            time.sleeps.get(),
            (LOCK_WAIT.as_millis() / RETRY_PAUSE.as_millis()) as u32
        );
        drop(holder);
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn a_lock_released_while_waiting_is_taken_on_the_next_try() {
        let run_dir = temp_dir("released");
        let mut holder = Some(acquire(&run_dir).expect("the first handle takes it"));
        let time = FakeTime::new();

        let second = acquire_with(
            &run_dir,
            LOCK_WAIT,
            || time.now.get(),
            |d| {
                time.sleep(d);
                if time.sleeps.get() == 3 {
                    holder.take();
                }
            },
        );

        assert!(second.is_some(), "takes it on the first try after it is released");
        assert_eq!(time.sleeps.get(), 3);
        assert_eq!(time.elapsed(), RETRY_PAUSE * 3);
        drop(second);
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn dropping_the_held_lock_releases_it() {
        let run_dir = temp_dir("drop");
        let first = acquire(&run_dir).expect("the first handle takes it");
        drop(first);

        let time = FakeTime::new();
        let second = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(second.is_some());
        assert_eq!(time.sleeps.get(), 0, "takes it right away without waiting");
        drop(second);
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn a_run_folder_that_cannot_be_made_proceeds_without_the_lock_and_without_waiting() {
        let root = temp_dir("blocked");
        std::fs::create_dir_all(&root).unwrap();
        // A file is in the place of the folder, so the folder cannot be created.
        let run_dir = root.join("run");
        std::fs::write(&run_dir, "not a folder").unwrap();
        let time = FakeTime::new();

        let lock = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(lock.is_none());
        assert_eq!(time.sleeps.get(), 0);
        std::fs::remove_dir_all(&root).ok();
    }
}
