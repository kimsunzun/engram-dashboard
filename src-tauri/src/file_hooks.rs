//! The OS share that shell data files (settings · screen state) pass to base `file` — connects lock retry and folder sync to the OS-layer
//! crate's. Every shell `file` call that takes `OsHooks` passes [`OS_HOOKS`].
// ADR-0291

use std::io;

use engram_dashboard_base::file::{OsHooks, BUSY_PAUSE, BUSY_RETRIES};
use engram_dashboard_platform::fs::{retry_busy, sync_dir, Retry};

pub(crate) const OS_HOOKS: OsHooks = OsHooks { retry, sync_dir };

fn retry(attempt: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    retry_busy(
        Retry {
            retries: BUSY_RETRIES,
            pause: BUSY_PAUSE,
        },
        attempt,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★Even if a do-nothing retry is passed, it still compiles★ — this test catches that mistake.
    #[test]
    fn the_retry_really_retries_a_locked_attempt() {
        let mut attempts = 0;
        let outcome = (OS_HOOKS.retry)(&mut || {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                Ok(())
            }
        });
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(attempts, 2);
    }
}
