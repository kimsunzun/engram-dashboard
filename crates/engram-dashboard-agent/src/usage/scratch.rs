//! 조회 한 번의 임시 폴더 — 조회기가 설정 파일을 쓰고 자식의 작업 폴더로 삼는다. 끝나면 지운다.
//!
//! ★배타 생성이다★ — 이름은 무작위이고, 이미 있는 이름이면 그 폴더를 쓰지 않고 새 이름을 뽑는다. 자식이
//!   믿고 읽는 파일이 이 안에 놓이므로, 남이 미리 심어 둔 폴더를 재사용하면 그 파일을 바꿔치기당한다.
//! ★지우기 실패는 (잠김이면 짧게 다시 해 본 뒤) 무시한다★ — 남은 폴더는 받는 쪽이 기동 때 [`sweep_stale_scratch`] 로 쓸어 낸다.
//!   (Windows 는 누가 열고 있는 파일·작업 폴더를 못 지운다 — 그래서 자식을 먼저 죽이고 기다린다.)

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_platform::fs::{retry_busy, Retry};
use uuid::Uuid;

use super::ProbeError;

/// 폴더 이름 = 이 접두 + uuid v4 의 32 자리 소문자 16진. 쓸기는 이 문법에 맞는 이름만 지운다 — 접두 일치로
/// 넓히면 손으로 둔 것까지 지운다.
const SCRATCH_PREFIX: &str = "probe-";
const SCRATCH_HEX_LEN: usize = 32;

/// 이름이 겹칠 때 새로 뽑는 상한. 무작위 이름이 겹치는 것은 누가 미리 심었을 때뿐이라 몇 번이면 충분하다.
const SCRATCH_CREATE_ATTEMPTS: usize = 8;

/// 조회 한 번의 임시 폴더. drop 하면 통째로 지운다(실패는 무시).
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// `root` 아래에 새 폴더를 배타로 만든다. `root` 가 없으면 만든다.
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

/// drop 이 지우기를 다시 해 보는 횟수와 간격 — 첫 시도 + 19 번 × 10 ms(약 200 ms).
///
/// 직접 자식이 끝난 직후에는 손자가 아직 작업 폴더를 쥐고 있을 수 있다 — 그때 곧바로 지우면 실패하고 몇 ms
/// 뒤에는 된다(실측 2026-09-27, 40/40 회 · 약 5 ms 뒤 성공 — 예산 약 200 ms 의 근거. 조회기의 Job 트리
/// 종료 기다리기(`wait_tree_gone`)와 같은 커밋 c7dcba33 에 실렸고, 그 기다리기 앞뒤 어느 쪽 측정인지는 기록이 없다).
// ADR-0291 R14: 파일럿 2026-10-10(rustc 1.95 · NTFS) — 손자가 작업 폴더를 쥔 합성 재현은 OS 오류 32
//   (`ERROR_SHARING_VIOLATION` — `is_busy` 가 문다)로만 실패했고 145 는 나오지 않았다. 실 조회 경로는 Job 트리가
//   다 끝난 뒤에 폴더를 놓으므로 6/6 회 첫 시도에 지워졌다 — 그래서 판정을 넓히지 않는다. FAT/exFAT 의 삭제
//   대기(145)는 재지 않았다 — 그렇게 남은 폴더는 기동 때 쓸기가 거둔다.
const DELETE_RETRY: Retry = Retry {
    retries: 19,
    pause: Duration::from_millis(10),
};

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // ADR-0291: 없는 폴더는 성공이고, 잠김 아닌 오류는 다시 하지 않는다.
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

/// `root` 아래 남은 임시 폴더를 지운다 — 조회가 하나도 돌지 않을 때(기동 때) 부른다.
///
/// 지우는 것은 [`ScratchDir`] 이름 문법에 맞는 **폴더**뿐이다(파일·링크·다른 이름은 둔다). `root` 가 없으면
/// 할 일이 없다. 실패는 로그만 남긴다 — 청소 실패로 기동을 막지 않는다.
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
        // `file_type` 은 링크를 따라가지 않는다 — 폴더 이름을 단 링크는 지우지 않는다.
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

    /// 누가 안의 파일을 배타로 열고 있으면 Windows 는 그 폴더를 못 지운다 — drop 은 그 실패를 삼킨다.
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

    /// 잠금이 drop 도중 곧 풀리면 다시 해 본 지우기가 성공한다 — 손자가 작업 폴더를 조금 늦게 놓는 경우.
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
        // 남은 폴더인 척 — drop 이 지우지 않게 값을 잊는다.
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
