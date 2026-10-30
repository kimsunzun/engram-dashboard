//! 데몬이 base `file` 에 넘기는 OS 몫 — 잠김 다시 하기와 폴더 동기화를 OS 층 crate 의 것으로 잇는다.
//! `OsHooks` 를 받는 데몬의 `file` 호출은 전부 [`OS_HOOKS`] 를 넘긴다(셸 `file_hooks` 와 같은 꼴).
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

    /// ★아무것도 안 하는 다시 하기를 넘겨도 컴파일은 선다★ — 그 실수를 이 시험이 잡는다.
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
