//! 셸 실행 잠금 `<셸 run 폴더>\state.lock`(TRD S21-storage §6-5 ① · §6-6 · I1) — 부팅 단계가 상태 파일을 읽기
//! 전에 잡고 정상 종료 쓰기(`Final`) 뒤에 놓는다.
//!
//! 단일 인스턴스 관문만으로는 모자란다 — 그 플러그인은 `RunEvent::Exit` 에서 뮤텍스를 우리 `Final` 보다 먼저 놓아,
//! 앞 인스턴스가 정상 종료를 쓰는 동안 새 인스턴스가 관문을 지난다(§6-5 사실 ②). 이 잠금이 그 새 인스턴스를
//! 앞 인스턴스의 마지막 쓰기 뒤로 세운다.
//!
//! ★못 잡아도 앱은 뜬다★ — 폴더 · 파일을 못 열거나 [`LOCK_WAIT`] 안에 못 잡으면 log 하고 잠금 없이 진행한다
//! (D8 정신 · §12 R11). 그 길의 남은 위험은 앞 인스턴스의 `Final` 과 엇갈리는 것뿐이고, 어느 쪽이 이겨도 파일은
//! 온전하다.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::saver::REPLY_DEADLINE;

pub const LOCK_FILE: &str = "state.lock";

/// 앞 인스턴스의 정상 종료 쓰기 마감([`REPLY_DEADLINE`]) + 여유. 짧으면 새 인스턴스가 그 쓰기를 기다리지 않는다.
pub const LOCK_WAIT: Duration = Duration::from_secs(3);
const _: () = assert!(LOCK_WAIT.as_millis() > REPLY_DEADLINE.as_millis());

/// 잡혀 있는 동안 다시 해 보는 간격 — 앞 인스턴스가 놓으면 이 안에 잡는다.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// 쥔 잠금 — 놓는 길은 drop 하나다(P3b3 = `Final` 뒤 · 가드면 `Final` 없이). 프로세스가 죽으면 OS 가 푼다.
#[derive(Debug)]
pub struct StateLock {
    file: File,
    path: PathBuf,
}

impl Drop for StateLock {
    // 핸들을 닫기 전에 명시적으로 푼다 — Windows `LockFileEx` 문서: 닫힌 핸들의 잠금은 OS 가 풀되 그 시점이 시스템
    //   자원에 달려 있어, 바로 뒤에 기다리는 새 인스턴스가 그만큼 더 막힐 수 있다.
    fn drop(&mut self) {
        if let Err(e) = self.file.unlock() {
            tracing::warn!(
                module = "state",
                path = %self.path.display(),
                error = %e,
                "셸 실행 잠금을 명시적으로 풀지 못했다 — 핸들을 닫으면 OS 가 푼다"
            );
        }
    }
}

/// `run_dir` 를 만들고(없으면) 그 안의 [`LOCK_FILE`] 을 배타로 잡는다 — 잡혀 있으면 [`LOCK_WAIT`] 까지 다시 해
/// 본다. `None` = 잠금 없이 진행한다(사유는 이미 log 했다).
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
            "셸 실행 잠금 — {what} — 잠금 없이 진행한다(앞 인스턴스의 정상 종료 쓰기와 엇갈릴 수 있다)"
        );
    };
    if let Err(e) = std::fs::create_dir_all(run_dir) {
        proceed_without("폴더를 못 만든다", &e);
        return None;
    }
    // 내용은 쓰지도 읽지도 않는다 — 비우지 않게 `truncate(false)`.
    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) => {
            proceed_without("파일을 못 연다", &e);
            return None;
        }
    };
    let start = now();
    loop {
        match file.try_lock() {
            Ok(()) => {
                let waited = now().saturating_duration_since(start);
                if !waited.is_zero() {
                    tracing::info!(
                        module = "state",
                        waited_ms = waited.as_millis() as u64,
                        "앞 인스턴스가 셸 실행 잠금을 놓아 잡았다"
                    );
                }
                return Some(StateLock { file, path });
            }
            Err(TryLockError::WouldBlock) => {
                let waited = now().saturating_duration_since(start);
                if waited >= wait {
                    tracing::warn!(
                        module = "state",
                        path = %path.display(),
                        waited_ms = waited.as_millis() as u64,
                        "셸 실행 잠금 — 다른 인스턴스가 쥐고 놓지 않는다 — 잠금 없이 진행한다(앞 인스턴스의 정상 종료 쓰기와 엇갈릴 수 있다)"
                    );
                    return None;
                }
                sleep(RETRY_PAUSE.min(wait - waited));
            }
            Err(TryLockError::Error(e)) => {
                proceed_without("잠그지 못한다", &e);
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! ★다른 프로세스 대신 같은 프로세스의 두 핸들로 경합을 만든다★ — std 잠금은 Windows 에서 `LockFileEx`(핸들
    //! 마다) · Unix 에서 `flock`(열린 파일 설명마다)이라 따로 연 두 핸들도 서로 막는다. 그래서 이 시험은 「다른
    //! 핸들이 쥐었을 때의 대기 · 포기 · 잡기」를 재고, 프로세스 사이의 실제 경합(앞 인스턴스의 종료와 겹치는
    //! 부팅)은 재지 않는다 — 그쪽은 P3b3 GUI 실측(§12 R11).

    use std::cell::Cell;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "engram-state-lock-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ))
    }

    /// 잠이 시계를 그만큼 밀 뿐인 가짜 시간 — 시험이 실제로 3초를 자지 않는다.
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
        let holder = acquire(&run_dir).expect("첫 핸들은 잡는다");
        let time = FakeTime::new();

        let second = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(second.is_none(), "기다려도 안 놓이면 잠금 없이 진행한다");
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
        let mut holder = Some(acquire(&run_dir).expect("첫 핸들은 잡는다"));
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

        assert!(second.is_some(), "놓인 뒤 첫 시도에 잡는다");
        assert_eq!(time.sleeps.get(), 3);
        assert_eq!(time.elapsed(), RETRY_PAUSE * 3);
        drop(second);
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn dropping_the_held_lock_releases_it() {
        let run_dir = temp_dir("drop");
        let first = acquire(&run_dir).expect("첫 핸들은 잡는다");
        drop(first);

        let time = FakeTime::new();
        let second = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(second.is_some());
        assert_eq!(time.sleeps.get(), 0, "기다리지 않고 바로 잡는다");
        drop(second);
        std::fs::remove_dir_all(&run_dir).ok();
    }

    #[test]
    fn a_run_folder_that_cannot_be_made_proceeds_without_the_lock_and_without_waiting() {
        let root = temp_dir("blocked");
        std::fs::create_dir_all(&root).unwrap();
        // 폴더 자리에 파일이 있어 폴더를 못 만든다.
        let run_dir = root.join("run");
        std::fs::write(&run_dir, "not a folder").unwrap();
        let time = FakeTime::new();

        let lock = acquire_with(&run_dir, LOCK_WAIT, || time.now.get(), |d| time.sleep(d));

        assert!(lock.is_none());
        assert_eq!(time.sleeps.get(), 0);
        std::fs::remove_dir_all(&root).ok();
    }
}
