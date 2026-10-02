//! 셸이 디스크에 두는 파일들의 공용 읽기·쓰기 — 상한 읽기([`read_capped`])와 원자 쓰기([`write_atomic`]).
//! 파일 이름·위치·형식은 모른다 — 그건 각 저장소(`ui_settings` · `settings::store`)가 소유한다.

use std::path::Path;

/// ★상한까지만 읽는다 — 읽고 나서 재지 않는다★.
///
/// 먼저 통째로 읽어 길이를 재면 상한 검사가 도착하기 전에 메모리가 먼저 바닥난다(밖의 에이전트가 쓰는
/// 파일이라 크기가 우리 손에 없다). 그러면 기본값 접기도 경고도 못 돌고 프로세스가 죽는다.
///
/// 상한 초과와 UTF-8 아님은 **둘 다 `ErrorKind::InvalidData`** 다 — 둘 다 원문을 못 가져온 것이다.
/// 그 밖의 종류는 읽기 자체의 IO 실패이고, 둘을 가르는 호출자가 있다(`settings::store` — 내용이 못 쓸
/// 것이면 첫 쓰기에 옆으로 치우고, IO 실패면 손대지 않는다).
pub fn read_capped(source: impl std::io::Read, cap: u64) -> std::io::Result<String> {
    use std::io::Read;

    let mut buf = Vec::new();
    // cap + 1 = 「넘었나」를 알 수 있는 최소치. 넘었어도 읽는 양은 여기서 멈춘다.
    source.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{cap} 바이트 상한을 넘었다"),
        ));
    }
    String::from_utf8(buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// 임시 파일에 쓰고 **rename 으로 갈아끼운다** — 쓰다 죽어도 반쪽 파일이 안 남는다.
///
/// ★임시 파일은 같은 폴더에 만든다★ — rename 이 갈아끼우기로 도는 것은 같은 볼륨 안에서다. 이름에 pid 를
/// 붙이는 것은 같은 폴더를 보는 다른 프로세스와 임시 이름이 겹치면 서로의 반쪽을 rename 하기 때문이다.
/// ★같은 프로세스 안에서는 이름이 겹친다★ — 한 경로에 쓰는 호출을 호출자가 직렬화해야 한다.
///
/// 실패하면 임시 파일을 치우고 원본을 그대로 둔다 — 안 치우면 데이터 폴더에 쓰레기가 쌓인다.
///
/// ★rename 이 잠김으로 실패하면 잠깐 기다렸다 다시 한다(최대 [`RENAME_RETRIES`]번 · [`RENAME_PAUSE`]
/// 간격 — 어느 오류가 잠김인지는 [`retry_denied`])★ — 백신 · 색인기가 대상 파일을 잠깐 쥐면 Windows 의
/// rename 이 그렇게 실패한다. 그래서 이 함수는 최악에 그만큼(약 100 ms) 더 걸린다. 그보다 오래 쥐면 그대로
/// 실패한다.
// ADR-0167
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;

    let invalid = |what: &str| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{what} 를 못 고르는 경로"),
        )
    };
    let dir = path.parent().ok_or_else(|| invalid("부모 폴더"))?;
    let mut name = path
        .file_name()
        .ok_or_else(|| invalid("파일 이름"))?
        .to_os_string();
    name.push(format!(".tmp{}", std::process::id()));
    let tmp = dir.join(name);

    let staged = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        // ★flush 로는 부족하다★ — 그건 프로세스 버퍼만 비운다. rename 이 가리키게 될 내용이 실제로
        //   디스크에 있어야 「반쪽이 안 남는다」가 성립한다.
        file.sync_all()
    })();
    let outcome = staged.and_then(|()| {
        retry_denied(
            || std::fs::rename(&tmp, path),
            || std::thread::sleep(RENAME_PAUSE),
        )
    });
    if outcome.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

const RENAME_RETRIES: u32 = 5;
const RENAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(20);

/// `attempt` 를 한 번 하고, 잠김([`is_lock_contention`])이면 `pause` 뒤 [`RENAME_RETRIES`]번까지 다시 한다.
/// 다른 오류는 바로 돌려준다.
fn retry_denied(
    mut attempt: impl FnMut() -> std::io::Result<()>,
    mut pause: impl FnMut(),
) -> std::io::Result<()> {
    let mut retries = 0;
    loop {
        match attempt() {
            Err(e) if is_lock_contention(&e) && retries < RENAME_RETRIES => {
                retries += 1;
                pause();
            }
            outcome => return outcome,
        }
    }
}

/// 다른 프로세스가 파일을 잠깐 쥐어서 난 실패인가 — `PermissionDenied`(Windows 에선 `ERROR_ACCESS_DENIED`
/// = 5), 그리고 Windows 에서만 `ERROR_SHARING_VIOLATION`(32) · `ERROR_LOCK_VIOLATION`(33).
///
/// ★32 · 33 은 종류로 못 잡는다★ — Rust std 는 5 만 `PermissionDenied` 로 옮기고 그 둘은 이름 없는 종류로
/// 남긴다. 그래서 OS 코드로 본다. 다른 OS 에서 그 번호는 다른 오류다(Linux 32 = `EPIPE`).
fn is_lock_contention(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied
        || (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    #[test]
    fn a_brief_denial_is_retried_until_it_clears() {
        let mut failures = 2;
        let mut pauses = 0;
        let outcome = retry_denied(
            || {
                if failures > 0 {
                    failures -= 1;
                    Err(denied())
                } else {
                    Ok(())
                }
            },
            || pauses += 1,
        );
        outcome.unwrap();
        assert_eq!(pauses, 2);
    }

    #[test]
    fn a_lasting_denial_gives_up_after_the_bound() {
        let mut attempts = 0;
        let mut pauses = 0;
        let outcome = retry_denied(
            || {
                attempts += 1;
                Err(denied())
            },
            || pauses += 1,
        );
        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!((attempts, pauses), (RENAME_RETRIES + 1, RENAME_RETRIES));
    }

    #[test]
    fn windows_sharing_and_lock_violations_are_retried_too() {
        for code in [32, 33] {
            let mut failures = 2;
            let mut pauses = 0;
            let outcome = retry_denied(
                || {
                    if failures > 0 {
                        failures -= 1;
                        Err(io::Error::from_raw_os_error(code))
                    } else {
                        Ok(())
                    }
                },
                || pauses += 1,
            );
            if cfg!(windows) {
                outcome.unwrap();
                assert_eq!(pauses, 2, "{code}");
            } else {
                assert_eq!(outcome.unwrap_err().raw_os_error(), Some(code));
                assert_eq!(pauses, 0, "{code}: 다른 OS 에선 잠김이 아니다");
            }
        }
    }

    #[test]
    fn a_lasting_sharing_violation_gives_up_after_the_same_bound() {
        let mut attempts = 0;
        let outcome = retry_denied(
            || {
                attempts += 1;
                Err(io::Error::from_raw_os_error(32))
            },
            || {},
        );
        assert_eq!(outcome.unwrap_err().raw_os_error(), Some(32));
        let expected = if cfg!(windows) { RENAME_RETRIES + 1 } else { 1 };
        assert_eq!(attempts, expected);
    }

    #[test]
    fn other_errors_are_not_retried() {
        let mut attempts = 0;
        let outcome = retry_denied(
            || {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            || panic!("기다리지 않는다"),
        );
        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(attempts, 1);
    }
}
