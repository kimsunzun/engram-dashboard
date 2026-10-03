//! 셸이 디스크에 두는 파일들의 공용 읽기·쓰기 — 상한 읽기([`read_capped`] · [`read_file_capped`]) · 원자 쓰기
//! ([`write_atomic`] · [`write_atomic_unless`]) · 원자 복사([`copy_atomic`] · [`copy_aside`]) · 내용 식별값
//! ([`fnv1a_64`]).
//!
//! 저장소의 파일 이름·위치·형식은 모른다 — 그건 각 저장소(`ui_settings` · `settings::store`)가 소유한다. 여기서
//! 정하는 이름은 대상 옆에 붙는 둘뿐이다: 임시 `<이름>.tmp<pid>.<번호>` · 떠 둔 사본 `<이름>.corrupt`.
//!
//! ★잠김 재시도는 한 규칙이다★ — rename · [`read_file_capped`] 의 열기와 읽기 · [`copy_atomic`] 의 원본 열기가
//! 같은 판정([`is_lock_contention`])과 같은 한도([`RENAME_RETRIES`] · [`RENAME_PAUSE`])를 쓴다. 복사 도중의
//! 읽기 실패는 다시 하지 않는다.

use std::io;
use std::path::{Path, PathBuf};

/// ★상한까지만 읽는다 — 읽고 나서 재지 않는다★.
///
/// 먼저 통째로 읽어 길이를 재면 상한 검사가 도착하기 전에 메모리가 먼저 바닥난다(밖의 에이전트가 쓰는
/// 파일이라 크기가 우리 손에 없다). 그러면 기본값 접기도 경고도 못 돌고 프로세스가 죽는다.
///
/// 상한 초과와 UTF-8 아님은 **둘 다 `ErrorKind::InvalidData`** 다 — 둘 다 원문을 못 가져온 것이다.
/// 그 밖의 종류는 읽기 자체의 IO 실패이고, 둘을 가르는 호출자가 있다(`settings::store` — 내용이 못 쓸
/// 것이면 첫 쓰기에 옆으로 치우고, IO 실패면 손대지 않는다).
pub fn read_capped(source: impl io::Read, cap: u64) -> io::Result<String> {
    use std::io::Read;

    let mut buf = Vec::new();
    // cap + 1 = 「넘었나」를 알 수 있는 최소치. 넘었어도 읽는 양은 여기서 멈춘다.
    source.take(cap + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{cap} 바이트 상한을 넘었다"),
        ));
    }
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// 파일을 열어 [`read_capped`] 로 읽는다 — 열기나 읽기가 잠김으로 실패하면 rename 과 같은 규칙으로 다시 한다
/// ([`replace_with`]). 백신 · 색인기가 파일을 잠깐 쥐면 읽기도 그렇게 실패한다.
///
/// 그 밖의 오류는 다시 하지 않고 바로 돌려준다 — 없는 파일 = `NotFound` · 상한 초과 · UTF-8 아님 =
/// `InvalidData`. ★잠김이 한도를 넘으면 그 잠김 오류 그대로다★ — `InvalidData`(못 쓰는 내용)와 섞이지 않는다.
pub fn read_file_capped(path: &Path, cap: u64) -> io::Result<String> {
    read_capped_retrying(
        || std::fs::File::open(path),
        cap,
        || std::thread::sleep(RENAME_PAUSE),
    )
}

/// [`read_file_capped`] 의 실물 — 열기 · 기다리기를 받아 시험이 실제 잠김 없이 돈다.
fn read_capped_retrying<R: io::Read>(
    mut open: impl FnMut() -> io::Result<R>,
    cap: u64,
    pause: impl FnMut(),
) -> io::Result<String> {
    retry_denied(|| read_capped(open()?, cap), pause)
}

/// [`write_atomic_unless`] 의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Written,
    /// `skip` 이 서서 rename 하지 않았다 — 임시 파일은 치웠고 대상은 그대로다. 오류가 아니라 다시 하지 않는다.
    Skipped,
}

/// 임시 파일에 쓰고 **rename 으로 갈아끼운다** — 쓰다 죽어도 반쪽 파일이 안 남는다. 임시 이름 · 실패 때의
/// 정리 · 잠김 재시도는 [`replace_with`].
// ADR-0167
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    write_atomic_unless(path, text, || false).map(drop)
}

/// [`write_atomic`] 에 「rename 직전에 물을 것」을 더한 것 — `skip` 을 **첫 rename 앞과 잠김 재시도의 rename
/// 앞마다** 묻고, `true` 면 [`WriteOutcome::Skipped`].
///
/// ★`skip` 이 `false` 를 돌려준 직후에 선 표지는 그 rename 을 막지 못한다★ — 묻기와 rename 사이는 원자적이지
/// 않다.
pub fn write_atomic_unless(
    path: &Path,
    text: &str,
    skip: impl Fn() -> bool,
) -> io::Result<WriteOutcome> {
    use std::io::Write;

    replace_with(
        path,
        |file| file.write_all(text.as_bytes()),
        skip,
        |from, to| std::fs::rename(from, to),
        || std::thread::sleep(RENAME_PAUSE),
    )
}

/// `from` 을 `to` 에 원자적으로 복사한다 — 흘려 쓰므로 통째로 메모리에 올리지 않고 크기 상한도 없다. 있던 `to`
/// 는 덮는다. 임시 이름 · 실패 때의 정리(`to` 는 그대로) · rename 의 잠김 재시도는 [`replace_with`].
///
/// `from` 열기도 잠김이면 같은 규칙으로 다시 한다. 끝내 못 열면 임시 파일을 만들기 전에 그 오류다.
pub fn copy_atomic(from: &Path, to: &Path) -> io::Result<()> {
    // `std::fs::copy` 를 쓰지 않는다 — 원본의 권한까지 옮겨(Windows 읽기 전용 속성 · Unix 권한 비트), 읽기 전용
    //   원본이면 사본도 읽기 전용이 되어 `sync_all` 할 쓰기 핸들을 못 연다.
    let mut source = retry_denied(
        || std::fs::File::open(from),
        || std::thread::sleep(RENAME_PAUSE),
    )?;
    replace_with(
        to,
        |file| io::copy(&mut source, file).map(drop),
        || false,
        |from, to| std::fs::rename(from, to),
        || std::thread::sleep(RENAME_PAUSE),
    )
    .map(drop)
}

/// 못 쓰는 파일을 옆 이름 `<이름>.corrupt` 에 [`copy_atomic`] 으로 떠 둔다 — 원본은 그 자리에 그대로 둔다.
/// 돌려주는 값 = 사본의 자리. `Err` 면 앞서 떠 둔 사본도 그대로다.
///
/// ★이름이 하나뿐이라 앞서 떠 둔 사본을 덮는다★ — Chromium(`Preferences.bad`) · Firefox(`Invalidprefs.js`)와
/// 같은 관행이다(TRD §10 F21). 사본이 쌓이지 않으므로 크기 상한을 두지 않는다. 로그는 호출자가 낸다.
pub fn copy_aside(path: &Path) -> io::Result<PathBuf> {
    let to = sibling(path, ".corrupt")?;
    copy_atomic(path, &to)?;
    Ok(to)
}

/// 64비트 FNV-1a — 같은 바이트면 빌드 · 실행이 달라도 같은 값이다(std `DefaultHasher` 는 그것을 약속하지
/// 않는다). 같은 내용인가를 가리는 용도이지 보안용이 아니다 — 일부러 맞춘 충돌은 못 막는다.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(OFFSET_BASIS, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(PRIME)
    })
}

/// [`fnv1a_64`] 를 소문자 16진 16자리로 — 앞자리 0 을 채워 길이가 늘 같다.
pub fn fnv1a_hex(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a_64(bytes))
}

/// 같은 폴더의 임시 파일을 `stage` 로 채우고 `sync_all` 한 뒤 **rename 으로 `path` 를 갈아끼운다**. `skip` 은
/// rename 마다 그 앞에서 묻는다([`write_atomic_unless`]).
///
/// ★임시 파일은 같은 폴더에 만든다★ — rename 이 갈아끼우기로 도는 것은 같은 볼륨 안에서다. 이름은
/// [`temp_path`] — 두 호출이 임시 이름을 나눠 쓰면 뒤의 생성이 앞의 임시 파일을 비우고, 앞의 rename 이 그
/// 반쪽을 `Ok` 로 갈아끼운다.
///
/// 같은 경로를 동시에 쓰는 호출끼리는 나중에 rename 한 쪽이 남는다 — 어느 쪽이 나중인지는 정하지 않는다.
/// 순서가 중요하면 호출자가 직렬화한다.
///
/// `Written` 이 아니면(실패 · 건너뜀) 임시 파일을 치우고 `path` 를 그대로 둔다 — 안 치우면 데이터 폴더에
/// 쓰레기가 쌓인다.
///
/// ★rename 이 잠김으로 실패하면 잠깐 기다렸다 다시 한다(최대 [`RENAME_RETRIES`]번 · [`RENAME_PAUSE`] 간격 —
/// 어느 오류가 잠김인지는 [`is_lock_contention`])★ — 백신 · 색인기가 대상 파일을 잠깐 쥐면 Windows 의 rename
/// 이 그렇게 실패한다. 그래서 최악에 그만큼(약 100 ms) 더 걸린다. 그보다 오래 쥐면 그대로 실패한다.
///
/// `rename` · `pause` 는 시험의 이음매다 — 운영은 `std::fs::rename` · [`RENAME_PAUSE`] 잠.
// ADR-0167
fn replace_with(
    path: &Path,
    stage: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
    skip: impl Fn() -> bool,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    pause: impl FnMut(),
) -> io::Result<WriteOutcome> {
    let tmp = temp_path(path)?;

    let staged = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        stage(&mut file)?;
        // ★flush 로는 부족하다★ — 그건 프로세스 버퍼만 비운다. rename 이 가리키게 될 내용이 실제로
        //   디스크에 있어야 「반쪽이 안 남는다」가 성립한다.
        file.sync_all()
    })();
    let outcome = staged.and_then(|()| {
        retry_denied(
            || {
                if skip() {
                    return Ok(WriteOutcome::Skipped);
                }
                rename(&tmp, path).map(|()| WriteOutcome::Written)
            },
            pause,
        )
    });
    if !matches!(outcome, Ok(WriteOutcome::Written)) {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

/// `path` 옆의 임시 이름 `<이름>.tmp<pid>.<번호>` — pid 는 같은 폴더를 보는 다른 프로세스와, 번호(프로세스 안에서
/// 부를 때마다 하나씩 는다)는 같은 프로세스의 다른 호출과 가른다.
fn temp_path(path: &Path) -> io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    sibling(path, &format!(".tmp{}.{n}", std::process::id()))
}

/// `path` 와 같은 폴더의 `<path 이름><suffix>`.
fn sibling(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let invalid = |what: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} 를 못 고르는 경로"),
        )
    };
    let dir = path.parent().ok_or_else(|| invalid("부모 폴더"))?;
    let mut name = path
        .file_name()
        .ok_or_else(|| invalid("파일 이름"))?
        .to_os_string();
    name.push(suffix);
    Ok(dir.join(name))
}

const RENAME_RETRIES: u32 = 5;
const RENAME_PAUSE: std::time::Duration = std::time::Duration::from_millis(20);

/// `attempt` 를 한 번 하고, 잠김([`is_lock_contention`])이면 `pause` 뒤 [`RENAME_RETRIES`]번까지 다시 한다.
/// 다른 오류는 바로 돌려준다.
fn retry_denied<T>(
    mut attempt: impl FnMut() -> io::Result<T>,
    mut pause: impl FnMut(),
) -> io::Result<T> {
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
fn is_lock_contention(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
        || (cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Write};

    use super::*;

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-fsutil-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // ── 잠김 재시도 ──

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
            || -> io::Result<()> {
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
            || -> io::Result<()> {
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
            || -> io::Result<()> {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            || panic!("기다리지 않는다"),
        );
        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(attempts, 1);
    }

    // ── 상한 읽기 ──

    #[test]
    fn a_locked_open_is_retried_until_it_clears() {
        let mut failures = 2;
        let mut pauses = 0;
        let text = read_capped_retrying(
            || {
                if failures > 0 {
                    failures -= 1;
                    Err(denied())
                } else {
                    Ok(io::Cursor::new(b"hi".to_vec()))
                }
            },
            8,
            || pauses += 1,
        )
        .unwrap();
        assert_eq!((text.as_str(), pauses), ("hi", 2));
    }

    /// 첫 `read` 에서 잠김으로 실패하는 원본.
    struct LockedOnRead;

    impl io::Read for LockedOnRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(denied())
        }
    }

    #[test]
    fn a_read_locked_mid_way_is_retried_from_a_fresh_open() {
        let mut opens = 0;
        let mut pauses = 0;
        let text = read_capped_retrying(
            || -> io::Result<Box<dyn io::Read>> {
                opens += 1;
                Ok(if opens == 1 {
                    Box::new(LockedOnRead)
                } else {
                    Box::new(io::Cursor::new(b"hi".to_vec()))
                })
            },
            8,
            || pauses += 1,
        )
        .unwrap();
        assert_eq!((text.as_str(), opens, pauses), ("hi", 2, 1));
    }

    #[test]
    fn a_missing_file_and_unusable_content_are_not_retried() {
        let mut opens = 0;
        let missing = read_capped_retrying(
            || -> io::Result<io::Cursor<Vec<u8>>> {
                opens += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            8,
            || panic!("기다리지 않는다"),
        );
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(opens, 1);

        let over = read_capped_retrying(
            || Ok(io::Cursor::new(b"12345".to_vec())),
            4,
            || panic!("기다리지 않는다"),
        );
        assert_eq!(over.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_file_capped_reads_a_real_file_and_reports_a_missing_one() {
        let dir = temp_dir("read");
        let path = dir.join("state.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(read_file_capped(&path, 8).unwrap(), "{}");
        assert_eq!(
            read_file_capped(&dir.join("none.json"), 8)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── 원자 쓰기 ──

    #[test]
    fn an_unskipped_write_replaces_the_target_and_leaves_no_temp() {
        let dir = temp_dir("write");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();

        let outcome = write_atomic_unless(&path, "new", || false).unwrap();

        assert_eq!(outcome, WriteOutcome::Written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_skip_before_the_first_rename_leaves_the_target_and_no_temp() {
        let dir = temp_dir("skip-first");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        let asked = Cell::new(0);

        let outcome = write_atomic_unless(&path, "new", || {
            asked.set(asked.get() + 1);
            true
        })
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Skipped);
        assert_eq!(asked.get(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_skip_raised_during_a_lock_retry_stops_the_write() {
        let dir = temp_dir("skip-retry");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        // 첫 물음엔 아니라고 하고, 첫 rename 이 잠김으로 실패한 뒤의 물음엔 그렇다고 한다.
        let asked = Cell::new(0);
        let mut renames = 0;
        let mut pauses = 0;

        let outcome = replace_with(
            &path,
            |file| file.write_all(b"new"),
            || {
                asked.set(asked.get() + 1);
                asked.get() >= 2
            },
            |_, _| {
                renames += 1;
                Err(denied())
            },
            || pauses += 1,
        )
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Skipped);
        assert_eq!((asked.get(), renames, pauses), (2, 1, 1));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_rename_removes_the_temp_and_keeps_the_target() {
        let dir = temp_dir("rename-fail");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();

        let outcome = replace_with(
            &path,
            |file| file.write_all(b"new"),
            || false,
            |_, _| Err(io::Error::other("가짜 rename 실패")),
            || panic!("잠김이 아니면 기다리지 않는다"),
        );

        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_stage_removes_the_temp_and_never_renames() {
        let dir = temp_dir("stage-fail");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();

        let outcome = replace_with(
            &path,
            |_| Err(io::Error::other("가짜 쓰기 실패")),
            || panic!("채우기가 실패하면 묻지 않는다"),
            |_, _| panic!("채우기가 실패하면 rename 하지 않는다"),
            || {},
        );

        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn two_temps_for_one_target_never_share_a_name() {
        let path = Path::new("dir").join("state.json");
        let first = temp_path(&path).unwrap();
        let second = temp_path(&path).unwrap();

        assert_ne!(first, second);
        let prefix = format!("state.json.tmp{}.", std::process::id());
        for temp in [&first, &second] {
            assert_eq!(temp.parent(), path.parent(), "같은 폴더");
            let name = temp.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with(&prefix), "{name}");
        }
    }

    // ── 원자 복사 · 떠 두기 ──

    #[test]
    fn copy_atomic_overwrites_the_target_and_leaves_no_temp() {
        let dir = temp_dir("copy");
        let from = dir.join("a.json");
        let to = dir.join("b.json");
        std::fs::write(&from, "from").unwrap();
        std::fs::write(&to, "earlier").unwrap();

        copy_atomic(&from, &to).unwrap();

        assert_eq!(std::fs::read_to_string(&to).unwrap(), "from");
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "from");
        assert_eq!(
            names_in(&dir),
            vec!["a.json".to_string(), "b.json".to_string()]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_atomic_from_a_missing_file_touches_nothing() {
        let dir = temp_dir("copy-missing");
        let to = dir.join("b.json");
        std::fs::write(&to, "earlier").unwrap();

        let err = copy_atomic(&dir.join("none.json"), &to).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "earlier");
        assert_eq!(names_in(&dir), vec!["b.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_aside_uses_one_fixed_name_and_overwrites_the_earlier_copy() {
        let dir = temp_dir("copy-aside");
        let path = dir.join("settings.json");
        let fixed = dir.join("settings.json.corrupt");
        std::fs::write(&path, "{broken").unwrap();
        std::fs::write(&fixed, "earlier").unwrap();

        assert_eq!(copy_aside(&path).unwrap(), fixed);
        assert_eq!(std::fs::read_to_string(&fixed).unwrap(), "{broken");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{broken",
            "원본은 그 자리에"
        );

        std::fs::write(&path, "{BROKEN").unwrap();
        assert_eq!(copy_aside(&path).unwrap(), fixed);
        assert_eq!(std::fs::read_to_string(&fixed).unwrap(), "{BROKEN");
        assert_eq!(
            names_in(&dir),
            vec![
                "settings.json".to_string(),
                "settings.json.corrupt".to_string()
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_read_only_source_is_copied_aside_into_a_writable_copy() {
        let dir = temp_dir("copy-read-only");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{broken").unwrap();
        let set_read_only = |path: &Path, read_only: bool| {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(read_only);
            std::fs::set_permissions(path, permissions).unwrap();
        };
        set_read_only(&path, true);

        let copied = copy_aside(&path);

        set_read_only(&path, false);
        let to = copied.unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "{broken");
        assert!(!std::fs::metadata(&to).unwrap().permissions().readonly());
        std::fs::OpenOptions::new()
            .write(true)
            .open(&to)
            .expect("사본은 쓸 수 있다 — 다음 떠 두기가 덮는다");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── FNV-1a ──

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"a"), "af63dc4c8601ec8c");
        assert_eq!(fnv1a_hex(b"foobar"), "85944171f73967e8");
    }
}
