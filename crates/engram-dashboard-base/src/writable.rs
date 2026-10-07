//! 폴더에 실제로 쓸 수 있나 — 그 안에 파일을 만들어 바이트를 싣고 지워 본다(쓰기 프로브).
//!
//! ★여러 프로세스 · 여러 호출이 같은 폴더를 동시에 검사할 수 있다★ — 프로브 이름은 프로세스 · 호출마다
//! 다르다. 검사 도중 폴더가 사라지는 경합은 [`probe_write_in`] 이 스스로 다시 해 보지 않는다 — 부르는
//! 쪽이 그것을 [`retry_if_vanished`] 로 감싸 한 번 더 해 본다. 폴더를 만들지와 실패를 어떤 오류로 접을지도
//! 부르는 쪽 몫이다.
// ADR-0269
// ADR-0282

use std::path::{Path, PathBuf};

/// 쓰기 프로브 파일 이름의 앞부분. 뒤에 **프로세스·호출마다 다른 꼬리**가 붙는다 — 폴더에 이 접두로
/// 시작하는 파일이 남아 있으면 프로브 잔여물이다.
///
/// ★고정 이름을 쓰지 말 것(되살리지 마라)★: 여러 프로세스가 같은 폴더를 동시에 프로브할 수 있고, 이름이
/// 같으면 진 쪽이 `create_new` 에서 실패해 **멀쩡한 폴더를 "쓰기 불가"로 판정**한다. 더해 삭제만 막는
/// ACL 에서는 남은 파일 하나가 이후 모든 프로브를 영구히 막는다.
pub const WRITE_PROBE_PREFIX: &str = ".engram-write-probe-";

/// ★0바이트로 쓰지 말 것★: 디스크가 꽉 찼거나 할당량이 소진된 상태에서도 길이 0 파일 생성은 흔히
/// 성공한다 — 그러면 프로브는 통과하고 첫 실제 쓰기가 실패한다. 실제로 바이트를 실어야 검사가 된다.
const WRITE_PROBE_PAYLOAD: &[u8] = b"engram-write-probe";

/// ★sync_all 까지 간다★: 캐시에만 얹힌 쓰기는 꽉 찬 디스크·소진된 할당량을 그대로 통과한다 —
/// 바이트가 실제로 안착해야 "쓸 수 있다"가 참이다.
fn write_probe_payload(mut f: std::fs::File) -> std::io::Result<()> {
    use std::io::Write;
    f.write_all(WRITE_PROBE_PAYLOAD)?;
    f.sync_all()
}

/// 이 프로세스·이 호출만의 프로브 경로. pid 로 프로세스를, 카운터로 같은 프로세스의 동시 호출을 가른다.
fn probe_path(dir: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{WRITE_PROBE_PREFIX}{}-{n}", std::process::id()))
}

/// `dir` **안에** 파일을 만들어 바이트를 쓸 수 있는지 실제로 해 본다. `dir` 은 이미 존재해야 한다.
///
/// 프로브 파일은 `create_new` 로 만든다 — 같은 이름의 기존 파일을 절대 덮어쓰지 않는다. 이미 있으면
/// 지난 번 정리가 실패한 흔적이므로 지우고 한 번 더 시도한다(존재 자체는 실패 사유가 아니다).
///
/// ★남길 수 있다(계약)★: 만든 파일은 지우지만, 생성은 되고 삭제는 막는 폴더에서는 **삭제가 실패해
/// 파일이 남는다**. 그때는 경고 로그를 남기고 성공으로 본다 — 쓸 수 있다는 것은 이미 증명됐다.
pub fn probe_write_in(dir: &Path) -> std::io::Result<()> {
    probe_write_at(&probe_path(dir))
}

/// 프로브 경로를 인자로 받는 본체 — 테스트가 "그 이름으로 파일을 만들 수 없는" 실패 분기를 결정적으로
/// 재현하려면 이름을 정할 수 있어야 한다. 운영 진입점은 [`probe_write_in`] 뿐이다.
///
/// ★`io::Result` 로 돌려주는 이유★: 호출자가 `ErrorKind` 를 봐야 한다 — 폴더가 검사 도중 사라진
/// `NotFound` 는 "쓸 수 없다"가 아니라 경합이고, 그 둘을 여기서 뭉치면 구분할 방법이 사라진다.
fn probe_write_at(probe: &Path) -> std::io::Result<()> {
    let create = |p: &Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
    };

    let file = match create(probe) {
        Ok(f) => f,
        // 같은 pid 의 지난 실행이 정리에 실패하고 남긴 흔적일 수 있다 — 지우고 한 번만 다시 시도한다.
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

/// `once` 가 `NotFound` 로 실패하면 **한 번만** 더 부르고 그 결과를 돌려준다. 그 밖의 결과는 첫 결과
/// 그대로다.
///
/// ★왜 필요한가(실재하는 경합)★: 자기가 만든 폴더를 되돌리는 검사가 둘 겹치면, A 가 만든 폴더를 B 가
/// "있다"고 본 직후 A 가 지워 B 의 프로브가 `NotFound` 로 넘어진다. 그건 권한 문제가 아니라 타이밍이라
/// 멀쩡한 폴더를 "쓰기 불가"로 판정하면 안 된다.
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

    /// 프로브 전용 유니크 폴더(테스트 병렬 실행에서 서로 밟지 않게).
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

    /// ★프로브의 존재 이유를 겨눈다★: 폴더는 있는데 그 안에 **파일을 만들 수 없는** 경우. 폴더 생성만
    /// 보는 검사는 여기서 통과해 버린다. ACL 조작 없이 결정적으로 재현하려고 프로브 이름을 주입해
    /// 그 이름을 폴더로 선점한다 — 그 이름으로는 파일을 만들 수도 지울 수도 없다(실측: 둘 다 code 5).
    #[test]
    fn writable_probe_rejects_a_name_it_cannot_create() {
        let dir = fresh_probe_dir("nofile");
        std::fs::create_dir_all(&dir).expect("폴더 생성");
        let taken = dir.join("taken-by-a-directory");
        std::fs::create_dir_all(&taken).expect("프로브 이름을 폴더로 선점");
        let err = probe_write_at(&taken).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        // 경합(NotFound)과 구분돼야 재시도가 헛돌지 않는다.
        assert_ne!(
            err.kind(),
            ErrorKind::NotFound,
            "폴더가 있는데 파일을 못 만드는 것은 경합이 아니다: {err:?}"
        );
    }

    /// 같은 pid 의 지난 실행이 정리에 실패해 남긴 프로브는 실패 사유가 아니다 — 지우고 다시 만든다.
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

    /// 판정 표 — 각 줄 = `once` 가 차례로 낼 결과(`None` = 성공) · 기대 호출 수 · 기대 오류. 겹친 점검
    /// 경합 시험(소비자 쪽)은 이 표를 확률적으로만 지나가므로 여기서 결정적으로 잰다.
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
