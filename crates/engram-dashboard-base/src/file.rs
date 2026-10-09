//! 데이터 파일의 공용 읽기 · 쓰기 — 상한 읽기([`read_capped`] · [`read_file_capped`]) · 원자 쓰기
//! ([`write_atomic`] · [`write_atomic_unless`]) · 원자 복사([`copy_atomic`] · [`copy_aside`]) · 남은 임시 파일
//! 쓸기([`sweep_temps`]).
//!
//! 파일 이름 · 자리 · 형식은 모른다 — 그건 각 주인(저장소)이 소유한다. 여기서 정하는 이름은 대상 옆에 붙는
//! 둘뿐이다: 임시 `<이름>.tmp<pid>.<번호>` · 떠 둔 사본 `<이름>.corrupt`. ★그 꼴을 만드는 곳(`temp_path` ·
//! [`copy_aside`])과 읽는 곳(`temp_owner`)이 여기뿐이다★ — 꼴을 바꾸면 셋을 함께 고친다(시험이 만든 이름을 다시
//! 읽어 맞댄다).
//!
//! ★OS 에 따라 갈리는 몫은 부르는 쪽이 [`OsHooks`] 로 넘긴다★ — 이 crate 는 OS 층 crate 를 부르지 못한다(입주
//! 조건 ②). 잠김 재시도는 한 길이다: rename · [`read_file_capped`] 의 열기와 읽기 · [`copy_atomic`] 의 원본 열기가
//! 모두 [`OsHooks::retry`] 를 지난다. 복사 도중의 읽기 실패는 다시 하지 않는다.
//!
//! 로그를 내지 않는다 — 값을 돌려주고 부르는 쪽이 자기 문구로 찍는다.
// ADR-0291

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// OS 에 따라 갈리는 둘 — 부르는 쪽이 OS 층 crate(`engram-dashboard-platform` 의 `fs`)의 것을 이어 넘긴다. 운영
/// 값은 crate 마다 상수다(예산이 다른 읽기를 가진 주인은 둘 — ADR-0291 R14).
///
/// fn 포인터인 것은 운영 값을 `const` 로 두기 위해서다 — 상태를 쥔 시험 가짜(시도 세기 · 잠김 주입 · 자지 않기)는
/// 이것으로 넘기지 않는다.
// ADR-0291
#[derive(Clone, Copy)]
pub struct OsHooks {
    /// 잠깐 쥐어진 파일의 다시 하기 — `attempt` 를 잠김이 풀리거나 예산이 다할 때까지 부르고 마지막 결과를
    /// 돌려준다. 성공과 잠김 아닌 오류는 바로 돌려준다(없는 파일 `NotFound` · 못 쓸 내용 `InvalidData` 는 다시
    /// 하지 않는다). 운영 = `fs::retry_busy`(예산 = [`BUSY_RETRIES`] · [`BUSY_PAUSE`] — 주인이 따로 둔 긴 적재 예산은 예외 · R14).
    pub retry: fn(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>,
    /// 폴더 안 이름 바꾸기(rename)를 디스크에 영속시키는 폴더 동기화 — 운영 = `fs::sync_dir`.
    // 아직 부르지 않는다 — 원자 쓰기의 rename 뒤 폴더 동기화(ADR-0291 R10)를 다는 단위가 부른다.
    pub sync_dir: fn(&Path) -> io::Result<()>,
}

/// 데이터 파일의 잠김 예산 — 첫 시도 뒤 다시 하는 횟수와 그 사이 기다림. 운영 [`OsHooks::retry`] 가 이 값을 쓴다(주인이 따로 둔 긴 적재 예산은 예외 · ADR-0291 R14).
/// 최악에 약 100 ms 더 걸리고, 그보다 오래 쥐면 그대로 실패한다.
// ADR-0265 결정 4: rename 이 잠김이면 짧게 다시 한다.
// ADR-0291: 어느 실패가 잠김인지와 다시 하기 고리는 OS 층 정책 하나 — 여기는 예산만 쥔다.
pub const BUSY_RETRIES: u32 = 5;
pub const BUSY_PAUSE: Duration = Duration::from_millis(20);

/// [`OsHooks::retry`] 의 클로저 꼴 — 공개 함수는 운영 fn 포인터를 이 꼴로 풀어 넘기고, 이 모듈의 시험은 상태를
/// 쥔 가짜를 넘긴다.
type RetryHook<'a> = &'a mut dyn FnMut(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>;

/// `attempt` 를 `retry` 에 태워 그 값을 꺼낸다 — 훅은 값 타입을 모르므로 값은 여기서 따로 받는다. 훅이 `Ok` 를
/// 돌려줬는데 값이 없으면(시도를 한 번도 안 불렀다) 패닉하지 않고 `ErrorKind::Other` 다.
fn retrying<T>(retry: RetryHook, mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut value = None;
    retry(&mut || {
        value = Some(attempt()?);
        Ok(())
    })?;
    value.ok_or_else(|| io::Error::other("다시 하기가 시도를 한 번도 부르지 않았다"))
}

/// ★상한까지만 읽는다 — 읽고 나서 재지 않는다★.
///
/// 먼저 통째로 읽어 길이를 재면 상한 검사가 도착하기 전에 메모리가 먼저 바닥난다(밖의 에이전트 · 사람이 쓰는
/// 파일이라 크기가 우리 손에 없다). 그러면 기본값 접기도 경고도 못 돌고 프로세스가 죽는다.
///
/// 상한 초과와 UTF-8 아님은 **둘 다 `ErrorKind::InvalidData`** 다 — 둘 다 원문을 못 가져온 것이다. 그 밖의 종류는
/// 읽기 자체의 IO 실패이고, 둘을 가르는 호출자가 있다 — 내용이 못 쓸 것이면 옆으로 떠 두고, IO 실패면 손대지
/// 않는다. 잠김 재시도는 없다 — 파일을 여는 쪽이 필요하면 [`read_file_capped`] 를 쓴다.
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

/// 파일을 열어 [`read_capped`] 로 읽는다 — 열기나 읽기가 잠김으로 실패하면 [`OsHooks::retry`] 로 새로 열어 다시
/// 한다. 백신 · 색인기가 파일을 잠깐 쥐면 읽기도 그렇게 실패한다.
///
/// 그 밖의 오류는 다시 하지 않고 바로 돌려준다 — 없는 파일 = `NotFound` · 상한 초과 · UTF-8 아님 =
/// `InvalidData`. ★잠김이 예산을 넘으면 그 잠김 오류 그대로다★ — `InvalidData`(못 쓰는 내용)와 섞이지 않는다.
pub fn read_file_capped(path: &Path, cap: u64, os: OsHooks) -> io::Result<String> {
    read_file_capped_with(|| std::fs::File::open(path), cap, &mut |attempt| {
        (os.retry)(attempt)
    })
}

fn read_file_capped_with<R: io::Read>(
    mut open: impl FnMut() -> io::Result<R>,
    cap: u64,
    retry: RetryHook,
) -> io::Result<String> {
    retrying(retry, || read_capped(open()?, cap))
}

/// [`write_atomic_unless`] 의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Written,
    /// `skip` 이 서서 rename 하지 않았다 — 임시 파일은 치웠고 대상은 그대로다. 오류가 아니라 다시 하지 않는다.
    Skipped,
}

/// 같은 폴더의 임시 파일에 쓰고 **rename 으로 갈아끼운다** — 쓰다 죽어도 반쪽 파일이 안 남는다. 실패하면 임시
/// 파일을 치우고 대상은 그대로다. rename 이 잠김이면 [`OsHooks::retry`] 로 다시 한다.
///
/// 같은 경로를 동시에 쓰는 호출끼리는 나중에 rename 한 쪽이 남는다 — 어느 쪽이 나중인지는 정하지 않는다. 순서가
/// 중요하면 호출자가 직렬화한다.
// ADR-0265 결정 4: 설정 쓰기는 원자적이다(임시 파일 → sync_all → rename).
pub fn write_atomic(path: &Path, bytes: &[u8], os: OsHooks) -> io::Result<()> {
    write_atomic_unless(path, bytes, os, || false).map(drop)
}

/// [`write_atomic`] 에 「rename 직전에 물을 것」을 더한 것 — `skip` 을 **첫 rename 앞과 잠김 재시도의 rename
/// 앞마다** 묻고, `true` 면 [`WriteOutcome::Skipped`].
///
/// ★`skip` 이 `false` 를 돌려준 직후에 선 표지는 그 rename 을 막지 못한다★ — 묻기와 rename 사이는 원자적이지
/// 않다.
pub fn write_atomic_unless(
    path: &Path,
    bytes: &[u8],
    os: OsHooks,
    skip: impl Fn() -> bool,
) -> io::Result<WriteOutcome> {
    write_atomic_with(path, bytes, &mut |attempt| (os.retry)(attempt), &skip)
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    retry: RetryHook,
    skip: &dyn Fn() -> bool,
) -> io::Result<WriteOutcome> {
    use std::io::Write;

    replace_with(
        path,
        |file| file.write_all(bytes),
        skip,
        |from, to| std::fs::rename(from, to),
        retry,
    )
}

/// `from` 을 `to` 에 원자적으로 복사한다 — 흘려 쓰므로 통째로 메모리에 올리지 않고 크기 상한도 없다. 있던 `to`
/// 는 덮는다. 실패하면 임시 파일을 치우고 `to` 는 그대로다.
///
/// `from` 열기와 rename 이 잠김이면 [`OsHooks::retry`] 로 다시 한다. 끝내 못 열면 임시 파일을 만들기 전에 그
/// 오류다.
// ADR-0274
pub fn copy_atomic(from: &Path, to: &Path, os: OsHooks) -> io::Result<()> {
    copy_atomic_with(from, to, &mut |attempt| (os.retry)(attempt))
}

fn copy_atomic_with(from: &Path, to: &Path, retry: RetryHook) -> io::Result<()> {
    // `std::fs::copy` 를 쓰지 않는다 — 원본의 권한까지 옮겨(Windows 읽기 전용 속성 · Unix 권한 비트), 읽기 전용
    //   원본이면 사본도 읽기 전용이 되어 `sync_all` 할 쓰기 핸들을 못 연다.
    let mut source = retrying(&mut *retry, || std::fs::File::open(from))?;
    replace_with(
        to,
        |file| io::copy(&mut source, file).map(drop),
        || false,
        |from, to| std::fs::rename(from, to),
        retry,
    )
    .map(drop)
}

/// 못 쓰는 파일을 옆 이름 `<이름>.corrupt` 에 [`copy_atomic`] 으로 떠 둔다 — 원본은 그 자리에 그대로 둔다.
/// 돌려주는 값 = 사본의 자리. `Err` 면 앞서 떠 둔 사본도 그대로다.
///
/// ★이름이 하나뿐이라 앞서 떠 둔 사본을 덮는다★ — Chromium(`Preferences.bad`) · Firefox(`Invalidprefs.js`)와
/// 같은 관행이다(TRD S21-storage §10 F21). 사본이 쌓이지 않으므로 크기 상한을 두지 않는다.
// ADR-0274
pub fn copy_aside(path: &Path, os: OsHooks) -> io::Result<PathBuf> {
    let to = sibling(path, ASIDE_SUFFIX)?;
    copy_atomic(path, &to, os)?;
    Ok(to)
}

/// 같은 폴더의 임시 파일을 `stage` 로 채우고 `sync_all` 한 뒤 **rename 으로 `path` 를 갈아끼운다**. `skip` 은
/// rename 마다 그 앞에서 묻는다([`write_atomic_unless`]).
///
/// ★임시 파일은 같은 폴더에 만든다★ — rename 이 갈아끼우기로 도는 것은 같은 볼륨 안에서다. 이름은 `temp_path` —
/// 두 호출이 임시 이름을 나눠 쓰면 뒤의 생성이 앞의 임시 파일을 비우고, 앞의 rename 이 그 반쪽을 `Ok` 로
/// 갈아끼운다.
///
/// `Written` 이 아니면(실패 · 건너뜀) 임시 파일을 치우고 `path` 를 그대로 둔다 — 안 치우면 데이터 폴더에 쓰레기가
/// 쌓인다.
///
/// ★rename 이 잠김으로 실패하면 `retry` 로 다시 한다★ — 백신 · 색인기가 대상 파일을 잠깐 쥐면 Windows 의 rename 이
/// 그렇게 실패한다.
///
/// `rename` · `retry` 는 시험의 이음매다 — 운영은 `std::fs::rename` · 부르는 쪽의 [`OsHooks::retry`].
// ADR-0265 결정 4: rename 이 잠김이면 짧게 다시 한다.
fn replace_with(
    path: &Path,
    stage: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
    skip: impl Fn() -> bool,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    retry: RetryHook,
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
        retrying(retry, || {
            if skip() {
                return Ok(WriteOutcome::Skipped);
            }
            rename(&tmp, path).map(|()| WriteOutcome::Written)
        })
    });
    if !matches!(outcome, Ok(WriteOutcome::Written)) {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

const TEMP_MARK: &str = ".tmp";
const ASIDE_SUFFIX: &str = ".corrupt";

/// `path` 옆의 임시 이름 `<이름>.tmp<pid>.<번호>` — pid 는 같은 폴더를 보는 다른 프로세스와, 번호(프로세스 안에서
/// 부를 때마다 하나씩 는다)는 같은 프로세스의 다른 호출과 가른다.
fn temp_path(path: &Path) -> io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    sibling(path, &format!("{TEMP_MARK}{}.{n}", std::process::id()))
}

/// `file_name` 이 `target` 의 임시 이름(`temp_path` 꼴)이면 그 pid. pid · 번호는 `temp_path` 가 적는 십진
/// 그대로여야 한다(`+` · 앞자리 0 · 범위 밖은 아니다).
///
/// ★번호 없는 `<target>.tmp<pid>`(번호를 더하기 전의 꼴)는 임시 이름으로 보지 않는다★ — 지금 쓸기의 대상인
/// 파일은 그 꼴로 쓰인 적이 없어 그런 이름은 우리 것이라 단정할 수 없고, 남의 것일 수 있는 파일은 지우지 않는다.
/// 옛 꼴로 쓰인 대상을 쓸게 되면 이 규칙부터 다시 본다.
fn temp_owner(file_name: &str, target: &str) -> Option<u32> {
    let rest = file_name.strip_prefix(target)?.strip_prefix(TEMP_MARK)?;
    let (pid, n) = rest.split_once('.')?;
    exact_decimal::<u64>(n)?;
    exact_decimal::<u32>(pid)
}

fn exact_decimal<T: std::str::FromStr + ToString>(digits: &str) -> Option<T> {
    digits
        .parse::<T>()
        .ok()
        .filter(|value| value.to_string() == digits)
}

/// `dir` 에서 `targets`(파일 이름) 각각의 남은 임시 파일 — 원자 쓰기의 `<대상>.tmp<pid>.<번호>` 와 떠 두기의
/// `<대상>.corrupt.tmp<pid>.<번호>` — 중 pid 가 이 프로세스거나 `is_alive` 가 죽었다고 한 것을 지운다.
///
/// - 산 남의 pid 것은 남긴다 — 같은 폴더를 쓰는 다른 프로세스가 지금 쓰는 중일 수 있다.
/// - ★자기 pid 것도 지운다★ — 같은 대상을 쓰는 중인 호출이 이 프로세스에 없을 때만 부른다.
///
/// 돌려주는 값 = 지우려 한 파일과 그 결과(그사이 이미 없어졌으면 성공). 폴더가 없으면 빈 목록이다. `Err` = 폴더를
/// 못 읽었다.
pub fn sweep_temps(
    dir: &Path,
    targets: &[&str],
    is_alive: impl Fn(u32) -> bool,
) -> io::Result<Vec<(PathBuf, io::Result<()>)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let me = std::process::id();
    let mut swept = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let owner = targets.iter().find_map(|target| {
            temp_owner(name, target)
                .or_else(|| temp_owner(name, &format!("{target}{ASIDE_SUFFIX}")))
        });
        let Some(pid) = owner else {
            continue;
        };
        if pid != me && is_alive(pid) {
            continue;
        }
        let path = entry.path();
        let removed = match std::fs::remove_file(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            removed => removed,
        };
        swept.push((path, removed));
    }
    Ok(swept)
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{self, Write};

    use super::*;

    /// 다시 하지 않는 훅 — 실제 파일 시험은 잠김을 만나지 않는다. 상태가 없어 fn 포인터로 선다.
    const ONCE: OsHooks = OsHooks {
        retry: |attempt| attempt(),
        sync_dir: |_| Ok(()),
    };

    /// 운영 다시 하기의 모양을 흉내 낸 가짜 — 잠김(여기서는 `PermissionDenied`)이면 [`BUSY_RETRIES`] 번까지 다시
    /// 하되, 자지 않고 기다림만 `pauses` 에 센다. 어느 오류가 잠김인지는 운영에선 넘긴 훅(OS 층)의 몫이라, 이
    /// 모듈의 시험이 재는 것은 그 오류가 훅까지 그대로 가는가다.
    fn counting_retry(
        pauses: &mut u32,
    ) -> impl FnMut(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()> + '_ {
        move |attempt: &mut dyn FnMut() -> io::Result<()>| {
            let mut retries = 0;
            loop {
                match attempt() {
                    Err(e)
                        if e.kind() == io::ErrorKind::PermissionDenied
                            && retries < BUSY_RETRIES =>
                    {
                        retries += 1;
                        *pauses += 1;
                    }
                    outcome => return outcome,
                }
            }
        }
    }

    fn denied() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-base-file-{tag}-{}-{}",
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

    // ── 상한 읽기 ──

    #[test]
    fn a_locked_open_is_retried_until_it_clears() {
        let mut failures = 2;
        let mut pauses = 0;
        let text = read_file_capped_with(
            || {
                if failures > 0 {
                    failures -= 1;
                    Err(denied())
                } else {
                    Ok(io::Cursor::new(b"hi".to_vec()))
                }
            },
            8,
            &mut counting_retry(&mut pauses),
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
        let text = read_file_capped_with(
            || -> io::Result<Box<dyn io::Read>> {
                opens += 1;
                Ok(if opens == 1 {
                    Box::new(LockedOnRead)
                } else {
                    Box::new(io::Cursor::new(b"hi".to_vec()))
                })
            },
            8,
            &mut counting_retry(&mut pauses),
        )
        .unwrap();
        assert_eq!((text.as_str(), opens, pauses), ("hi", 2, 1));
    }

    #[test]
    fn a_missing_file_and_unusable_content_are_not_retried() {
        let mut opens = 0;
        let mut pauses = 0;
        let missing = read_file_capped_with(
            || -> io::Result<io::Cursor<Vec<u8>>> {
                opens += 1;
                Err(io::Error::from(io::ErrorKind::NotFound))
            },
            8,
            &mut counting_retry(&mut pauses),
        );
        assert_eq!(missing.unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!((opens, pauses), (1, 0));

        let over = read_file_capped_with(
            || Ok(io::Cursor::new(b"12345".to_vec())),
            4,
            &mut counting_retry(&mut pauses),
        );
        assert_eq!(over.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(pauses, 0);
    }

    #[test]
    fn read_file_capped_reads_a_real_file_and_reports_a_missing_one() {
        let dir = temp_dir("read");
        let path = dir.join("state.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(read_file_capped(&path, 8, ONCE).unwrap(), "{}");
        assert_eq!(
            read_file_capped(&dir.join("none.json"), 8, ONCE)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exactly_the_cap_is_read_and_one_byte_more_is_refused() {
        let cap = 32u64;
        assert!(
            read_capped(io::Cursor::new(vec![b'x'; cap as usize]), cap).is_ok(),
            "상한 자체는 통과다"
        );
        let over = read_capped(io::Cursor::new(vec![b'x'; cap as usize + 1]), cap)
            .expect_err("상한 초과는 반려다");
        assert_eq!(over.kind(), io::ErrorKind::InvalidData);
    }

    /// 내보낸 바이트를 세는 원본 — 결과만 보면 「다 읽고 나서 반려」와 「끊어 읽고 반려」가 똑같이
    /// `InvalidData` 라 구분이 안 된다.
    struct Counting<'a, R> {
        inner: R,
        produced: &'a Cell<u64>,
    }

    impl<R: io::Read> io::Read for Counting<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.produced.set(self.produced.get() + n as u64);
            Ok(n)
        }
    }

    /// ★상한이 결과가 아니라 **읽는 양**을 끊는다★ — [`read_capped`] 가 존재하는 이유 그 자체다.
    #[test]
    fn the_cap_stops_the_read_rather_than_the_result() {
        let cap = 16u64;
        let produced = Cell::new(0u64);
        let reader = Counting {
            inner: io::Cursor::new(vec![b'x'; 1024 * 1024]),
            produced: &produced,
        };

        let refused = read_capped(reader, cap).expect_err("상한 초과는 반려다");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);

        let read = produced.get();
        assert!(
            read <= cap + 1,
            "상한을 넘겨 {read} 바이트를 읽었다(허용 {}) — 끊지 않으면 원문 전체가 메모리에 올라온다",
            cap + 1
        );
    }

    #[test]
    fn non_utf8_content_is_invalid_data_like_an_oversized_one() {
        let refused = read_capped(io::Cursor::new(vec![0xff, 0xfe, 0x00]), 64)
            .expect_err("UTF-8 아님은 반려다");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    }

    // ── 원자 쓰기 ──

    #[test]
    fn an_unskipped_write_replaces_the_target_and_leaves_no_temp() {
        let dir = temp_dir("write");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();

        let outcome = write_atomic_unless(&path, b"new", ONCE, || false).unwrap();

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

        let outcome = write_atomic_unless(&path, b"new", ONCE, || {
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
            &mut counting_retry(&mut pauses),
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
        let mut pauses = 0;

        let outcome = replace_with(
            &path,
            |file| file.write_all(b"new"),
            || false,
            |_, _| Err(io::Error::other("가짜 rename 실패")),
            &mut counting_retry(&mut pauses),
        );

        assert_eq!(outcome.unwrap_err().kind(), io::ErrorKind::Other);
        assert_eq!(pauses, 0, "잠김이 아니면 기다리지 않는다");
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
            &mut |_| panic!("채우기가 실패하면 다시 하기에 들지 않는다"),
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

    // ── 임시 이름 해석 · 쓸기 ──

    #[test]
    fn the_owner_of_a_made_temp_name_reads_back_as_this_process() {
        for target in ["state.json", "state.json.corrupt"] {
            let temp = temp_path(&Path::new("dir").join(target)).unwrap();
            let name = temp.file_name().unwrap().to_str().unwrap();
            assert_eq!(temp_owner(name, target), Some(std::process::id()), "{name}");
        }
    }

    #[test]
    fn only_the_exact_numbered_shape_is_a_temp_name() {
        assert_eq!(temp_owner("state.json.tmp12.3", "state.json"), Some(12));
        assert_eq!(temp_owner("state.json.tmp12.0", "state.json"), Some(12));
        for name in [
            "state.json",
            "state.json.tmpX",
            "state.json.tmp12",
            "state.json.tmp12.",
            "state.json.tmp.3",
            "state.json.tmp12.3.4",
            "state.json.tmp12.x",
            "state.json.tmp+12.3",
            "state.json.tmp012.3",
            "state.json.tmp12.03",
            "state.json.tmp4294967296.3",
            "xstate.json.tmp12.3",
            "state.json.corrupt.tmp12.3",
            "state.crash.json.tmp12.3",
        ] {
            assert_eq!(temp_owner(name, "state.json"), None, "{name}");
        }
    }

    #[test]
    fn the_sweep_removes_own_and_dead_temps_and_keeps_live_and_unrelated_names() {
        let dir = temp_dir("sweep");
        let me = std::process::id();
        let live = me.wrapping_add(1);
        let dead = me.wrapping_add(2);
        let gone = [
            format!("state.json.tmp{me}.0"),
            format!("state.json.tmp{dead}.7"),
            format!("state.json.corrupt.tmp{dead}.1"),
            format!("state.crash.json.tmp{dead}.2"),
            format!("state.crash.json.corrupt.tmp{me}.3"),
        ];
        let kept = [
            "state.json".to_string(),
            format!("state.json.tmp{live}.4"),
            format!("state.json.corrupt.tmp{live}.5"),
            "state.json.tmpX".to_string(),
            format!("state.json.tmp{dead}"),
            format!("other.json.tmp{dead}.6"),
            format!("state.json.tmp{dead}.6.bak"),
        ];
        for name in gone.iter().chain(&kept) {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let asked = std::cell::RefCell::new(Vec::new());

        let swept = sweep_temps(&dir, &["state.json", "state.crash.json"], |pid| {
            asked.borrow_mut().push(pid);
            pid == live
        })
        .unwrap();

        let mut removed: Vec<String> = swept
            .iter()
            .map(|(path, outcome)| {
                assert!(outcome.is_ok(), "{path:?}: {outcome:?}");
                path.file_name().unwrap().to_string_lossy().into_owned()
            })
            .collect();
        removed.sort();
        let mut expected_gone = gone.to_vec();
        expected_gone.sort();
        assert_eq!(removed, expected_gone);
        let mut expected_kept = kept.to_vec();
        expected_kept.sort();
        assert_eq!(names_in(&dir), expected_kept);
        assert!(
            !asked.borrow().contains(&me),
            "자기 pid 는 살았는지 묻지 않고 지운다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sweeping_a_missing_folder_is_an_empty_sweep() {
        let dir = temp_dir("sweep-missing");
        let swept = sweep_temps(&dir.join("none"), &["state.json"], |_| {
            panic!("지울 후보가 없으면 묻지 않는다")
        })
        .unwrap();
        assert!(swept.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── 원자 복사 · 떠 두기 ──

    #[test]
    fn copy_atomic_overwrites_the_target_and_leaves_no_temp() {
        let dir = temp_dir("copy");
        let from = dir.join("a.json");
        let to = dir.join("b.json");
        std::fs::write(&from, "from").unwrap();
        std::fs::write(&to, "earlier").unwrap();

        copy_atomic(&from, &to, ONCE).unwrap();

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

        let err = copy_atomic(&dir.join("none.json"), &to, ONCE).unwrap_err();

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

        assert_eq!(copy_aside(&path, ONCE).unwrap(), fixed);
        assert_eq!(std::fs::read_to_string(&fixed).unwrap(), "{broken");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{broken",
            "원본은 그 자리에"
        );

        std::fs::write(&path, "{BROKEN").unwrap();
        assert_eq!(copy_aside(&path, ONCE).unwrap(), fixed);
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

        let copied = copy_aside(&path, ONCE);

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
}
