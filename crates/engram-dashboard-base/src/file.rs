//! 데이터 파일의 공용 규칙 — 읽기 판정([`parse`] · [`classify`] · [`load`]) · 쓰기 판정([`Loaded::write_policy`] ·
//! [`judge`] · [`check_cap`]) · 상한 읽기([`read_capped`] · [`read_file_capped`]) · 원자 쓰기([`write_atomic`] ·
//! [`write_atomic_unless`]) · 원자 복사([`copy_atomic`] · [`copy_aside`]) · 남은 임시 파일 쓸기([`sweep_temps`]).
//!
//! 파일 이름 · 자리 · 스키마 · 버전 키 이름은 모른다 — 그건 각 주인(저장소)이 소유하고, 읽기 규칙에 드는 몫은
//! [`Spec`] 으로 넘긴다. 여기서 정하는 이름은 대상 옆에 붙는 둘뿐이다: 임시 `<이름>.tmp<pid>.<번호>` · 떠 둔 사본
//! `<이름>.corrupt`. ★그 꼴을 만드는 곳(`temp_path` · [`copy_aside`])과 읽는 곳(`temp_owner`)이 여기뿐이다★ — 꼴을
//! 바꾸면 셋을 함께 고친다(시험이 만든 이름을 다시 읽어 맞댄다).
//!
//! ★「덮어도 되나」는 쓰기마다 그 직전에 지금 파일로 다시 판정한다★ — 판정 규칙은 [`Loaded::write_policy`] 표
//! 하나이고, 순서(읽기 → 판정 → 떠 두기 → 쓰기)는 주인이 부른다. 새 판이 쓴 파일과 읽다 실패한 파일은 덮지도 떠
//! 두지도 않는다. 주인의 모양 검사([`Spec::shape`])는 [`parse`] 안에서 돌아, 모양이 깨진 파일은 어느 판정
//! 길에서나 손상이다.
//!
//! 폴더: 쓰기 · 복사는 없는 폴더를 대상 폴더까지 만들고 rename 뒤 그 폴더를 동기화한다. 읽기 · 판정은 아무것도
//! 만들지 않는다.
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

use serde_json::{Map, Value};

/// OS 에 따라 갈리는 둘 — 부르는 쪽이 OS 층 crate(`engram-dashboard-platform` 의 `fs`)의 것을 이어 넘긴다. 운영
/// 값은 crate 마다 상수다(예산이 다른 읽기를 가진 주인은 둘 — ADR-0291 R14).
///
/// fn 포인터인 것은 운영 값을 `const` 로 두기 위해서다 — 상태를 쥔 시험 가짜(시도 세기 · 잠김 주입 · 자지 않기 ·
/// 동기화 기록)는 이것으로 넘기지 않는다.
// ADR-0291
#[derive(Clone, Copy)]
pub struct OsHooks {
    /// 잠깐 쥐어진 파일의 다시 하기 — `attempt` 를 잠김이 풀리거나 예산이 다할 때까지 부르고 마지막 결과를
    /// 돌려준다. 성공과 잠김 아닌 오류는 바로 돌려준다(없는 파일 `NotFound` · 못 쓸 내용 `InvalidData` 는 다시
    /// 하지 않는다). 운영 = `fs::retry_busy`(예산 = [`BUSY_RETRIES`] · [`BUSY_PAUSE`] — 주인이 따로 둔 긴 적재 예산은 예외 · R14).
    pub retry: fn(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>,
    /// 폴더 안 이름 바꾸기(rename)를 디스크에 영속시키는 폴더 동기화 — 운영 = `fs::sync_dir`. 원자 쓰기 · 복사가
    /// rename 으로 갈아끼운 뒤에만 그 폴더로 한 번 부르고 결과는 보지 않는다(실패해도 그 쓰기는 성공이다).
    pub sync_dir: fn(&Path) -> io::Result<()>,
}

/// 데이터 파일의 잠김 예산 — 첫 시도 뒤 다시 하는 횟수와 그 사이 기다림. 운영 [`OsHooks::retry`] 가 이 값을 쓴다(주인이 따로 둔 긴 적재 예산은 예외 · ADR-0291 R14).
/// 최악에 약 100 ms 더 걸리고, 그보다 오래 쥐면 그대로 실패한다.
// ADR-0265 결정 4: rename 이 잠김이면 짧게 다시 한다.
// ADR-0291: 어느 실패가 잠김인지와 다시 하기 고리는 OS 층 정책 하나 — 여기는 예산만 쥔다.
pub const BUSY_RETRIES: u32 = 5;
pub const BUSY_PAUSE: Duration = Duration::from_millis(20);

/// [`OsHooks::retry`] 의 클로저 꼴 — 공개 함수는 운영 fn 포인터를 이 꼴로 풀어 넘기고, 이 모듈의 시험은 상태를
/// 쥔 가짜를 넘긴다. [`SyncDirHook`] 도 같다.
type RetryHook<'a> = &'a mut dyn FnMut(&mut dyn FnMut() -> io::Result<()>) -> io::Result<()>;
/// [`OsHooks::sync_dir`] 의 클로저 꼴.
type SyncDirHook<'a> = &'a mut dyn FnMut(&Path) -> io::Result<()>;

// ── 읽기 규칙 · 쓰기 판정 ──

/// 데이터 파일 하나의 읽기 규칙 — 주인(저장소)이 파일마다 상수로 둔다.
// ADR-0291 R1~R7
#[derive(Clone, Copy)]
pub struct Spec {
    /// 판을 싣는 최상위 키(파일마다 지금 그대로 — `$version` · `version` · `schema_version`).
    pub version_key: &'static str,
    /// 이 빌드가 쓰는 판 — 1 이상. 이보다 큰 판은 [`Parsed::Newer`].
    pub current: u64,
    /// 읽기 상한 = 쓰기 상한(바이트 — 디스크의 원문 그대로, BOM 포함). 넘는 원문은 손상이고 쓰지도 않는다
    /// ([`check_cap`]).
    pub cap: u64,
    /// 주인의 모양 검사 — 버전 문을 지난 문서(판 `1..=current`)를 그 주인이 읽을 수 있나. `Err(사유)` = 손상
    /// ([`Parsed::Unusable`] 에 사유 그대로 — 값을 실을지는 주인이 정한다).
    ///
    /// [`parse`] 가 부르므로 모양을 건너뛰는 판정 길이 없다. ★앞 판(`< current`)을 읽을 리더가 없으면 여기서
    /// `Err` 를 돌려 손상으로 만든다★ — 버전 문은 그 판을 받을지 모른다. 새 판([`Parsed::Newer`])에는 부르지 않는다.
    /// 모양을 따로 보지 않는 주인은 [`any_shape`].
    pub shape: fn(version: u64, doc: &Map<String, Value>) -> Result<(), String>,
}

/// 모양을 따로 보지 않는 주인의 [`Spec::shape`] — 늘 `Ok`.
pub fn any_shape(_version: u64, _doc: &Map<String, Value>) -> Result<(), String> {
    Ok(())
}

/// 원문 하나를 본 결과 — IO 는 모른다.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// 판 `1..=current` 이고 모양 검사를 지났다. `doc` 는 버전 키까지 그대로다.
    Usable {
        version: u64,
        doc: Map<String, Value>,
    },
    /// 손상 — 사유 문구. 이 모듈이 만드는 사유는 값을 싣지 않는다 · 모양 검사 실패면 주인의 사유 그대로다.
    Unusable(String),
    /// 이 빌드보다 새 판 — 모양은 보지 않았다(새 판은 모양부터 다를 수 있다).
    Newer { found: u64 },
}

/// 원문 하나를 판정한다 — 상한 · 앞머리 BOM 하나 떼기 · JSON 객체 · 버전 문 · [`Spec::shape`] 순.
///
/// - 버전 키가 없으면 1 판이다.
/// - 버전 값은 1 이상의 정수 값인 수만 받는다(`1` · `1.0`) — 0 · 음수 · 소수 · 문자열 · `u64` 밖은 손상.
/// - `current` 보다 크면 [`Parsed::Newer`] — 모양 검사 앞에서 끝난다.
///
/// 상한은 [`read_file_capped`] 가 이미 끊었어도 여기서 다시 잰다 — 주인의 IO 이음매가 어떤 길로 읽었든 같은
/// 손상 정의가 선다.
// ADR-0291 R1~R5 · R7: 버전 없음 = 1 · 새 판은 따로 · 정수 값인 수만 · BOM 하나 무시 · 모양 실패 = 손상.
pub fn parse(text: &str, spec: &Spec) -> Parsed {
    if text.len() as u64 > spec.cap {
        return Parsed::Unusable(format!("{} 바이트 상한을 넘었다", spec.cap));
    }
    // 메모장 · PowerShell 은 UTF-8 로 저장하면 BOM 을 붙인다 — 손으로 고친 파일이 그것만으로 못 쓰게 되지 않게.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let doc = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(doc)) => doc,
        Ok(_) => return Parsed::Unusable("JSON 객체가 아니다".to_string()),
        Err(e) => return Parsed::Unusable(format!("JSON 이 아니다: {e}")),
    };
    let version = match doc.get(spec.version_key) {
        None => 1,
        Some(raw) => match version_number(raw) {
            Some(version) if version >= 1 => version,
            _ => {
                return Parsed::Unusable(format!(
                    "`{}` 가 1 이상의 정수가 아니다",
                    spec.version_key
                ))
            }
        },
    };
    if version > spec.current {
        return Parsed::Newer { found: version };
    }
    if let Err(reason) = (spec.shape)(version, &doc) {
        return Parsed::Unusable(reason);
    }
    Parsed::Usable { version, doc }
}

/// 정수 값인 수만 — 정수 값인 실수(`1.0`)도 받는다(JSON 도구가 수를 실수로 다시 쓰기도 한다). 음수 · 소수 ·
/// `u64` 범위 밖 · 수가 아닌 값은 `None`.
fn version_number(raw: &Value) -> Option<u64> {
    // 2^64 — `u64::MAX as f64` 와 같은 값이라 그 자체는 범위 밖이다. `as u64` 는 범위 밖을 u64::MAX 로 눌러
    //   없는 「새 판」을 지어내므로 자르기 전에 거른다.
    const U64_END: f64 = 18_446_744_073_709_551_616.0;
    raw.as_u64().or_else(|| {
        let float = raw.as_f64()?;
        (0.0..U64_END)
            .contains(&float)
            .then_some(float)
            .filter(|float| float.fract() == 0.0)
            .map(|float| float as u64)
    })
}

/// 파일 하나를 읽은 결과.
#[derive(Debug)]
pub enum Loaded {
    Missing,
    Parsed(Parsed),
    /// 내용이 아니라 읽기 자체가 실패했다(잠김 예산 뒤 · 권한 · 그 밖 IO). ★손상과 섞지 않는다★ — 잠깐 잠긴
    /// 멀쩡한 파일을 떠 두거나 덮지 않는다.
    Failed(io::Error),
}

/// 읽기 결과를 판정으로 — `NotFound` = [`Loaded::Missing`] · `InvalidData`(상한 초과 · UTF-8 아님 —
/// [`read_capped`] 의 계약) = 손상 · 그 밖 오류 = [`Loaded::Failed`] · 원문 = [`parse`].
///
/// 자기 IO 이음매(시험 가짜를 끼우는 트레이트)로 읽는 주인은 그 읽기 결과를 여기 넣는다. 경로로 바로 읽는 주인은
/// [`load`].
pub fn classify(read: io::Result<String>, spec: &Spec) -> Loaded {
    match read {
        Ok(text) => Loaded::Parsed(parse(&text, spec)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Loaded::Missing,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            Loaded::Parsed(Parsed::Unusable(e.to_string()))
        }
        Err(e) => Loaded::Failed(e),
    }
}

/// `path` 를 [`Spec::cap`] 까지 읽어([`read_file_capped`] — 잠김이면 `os.retry`) [`classify`] 한다. 아무것도
/// 만들지 않는다 — 폴더가 없으면 [`Loaded::Missing`] 이다.
pub fn load(path: &Path, spec: &Spec, os: OsHooks) -> Loaded {
    classify(read_file_capped(path, spec.cap, os), spec)
}

/// 이 실행이 그 파일을 이미 자기 것으로 잡았나 — [`Loaded::write_policy`] 의 한 축.
// ADR-0291 R8 (D14)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// 적재 판정이 쓸 수 있음이었거나 이번 실행이 그 파일에 한 번이라도 썼다.
    Adopted,
    /// 적재 판정이 없음 · 손상이었고 아직 한 번도 안 썼다 — 그 사이 놓인 쓸 만한 파일은 우리 것이 아니라 덮기
    /// 전에 떠 둔다.
    NotYet,
}

/// 그 파일에 지금 써도 되나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePolicy {
    Write,
    /// [`copy_aside`] 로 떠 둔 뒤 쓴다. 떠 두기가 실패했을 때 그 쓰기를 할지는 주인이 정한다.
    CopyAsideFirst,
    /// 쓰지 않는다 — 덮지도 떠 두지도 않는다.
    Refuse(Refused),
}

/// [`WritePolicy::Refuse`] 의 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// 이 빌드보다 새 판이 쓴 파일이다.
    Newer { found: u64 },
    /// 지금 파일을 못 읽었다([`Loaded::Failed`]) — 무엇이 들었는지 모르므로 덮지 않는다. 그 오류가 필요한 주인은
    /// [`judge`] 대신 [`load`] 로 [`Loaded`] 를 쥐고 [`Loaded::write_policy`] 를 부른다.
    ReadFailed,
}

impl Loaded {
    /// 이 읽기 결과 위에 써도 되나 — 데이터 파일 공통 규칙 하나다.
    ///
    /// | 지금 파일 | [`Claim::Adopted`] | [`Claim::NotYet`] |
    /// |---|---|---|
    /// | 없음 | 쓴다 | 쓴다 |
    /// | 쓸 수 있음 | 쓴다 | 떠 두고 쓴다 |
    /// | 손상 | 떠 두고 쓴다 | 떠 두고 쓴다 |
    /// | 새 판 | 거절 | 거절 |
    /// | 읽기 실패 | 거절 | 거절 |
    ///
    /// ★판정은 쓰기마다 그 직전에 지금 파일을 다시 읽어 한다★ — 적재 때 판정을 들고 있으면 실행 중에 새 판으로
    /// 바뀐 파일을 덮고, 「손상이라 떠 두고 쓴다」를 들고 있으면 첫 쓰기 뒤 우리 정상 파일을 쓰기마다 떠 두어 진짜
    /// 손상 사본을 지운다.
    // ADR-0291 R2 · R8 · R16
    pub fn write_policy(&self, claim: Claim) -> WritePolicy {
        match (self, claim) {
            (Loaded::Missing, Claim::Adopted | Claim::NotYet) => WritePolicy::Write,
            (Loaded::Parsed(Parsed::Usable { .. }), Claim::Adopted) => WritePolicy::Write,
            (Loaded::Parsed(Parsed::Usable { .. }), Claim::NotYet) => WritePolicy::CopyAsideFirst,
            (Loaded::Parsed(Parsed::Unusable(_)), Claim::Adopted | Claim::NotYet) => {
                WritePolicy::CopyAsideFirst
            }
            (Loaded::Parsed(Parsed::Newer { found }), Claim::Adopted | Claim::NotYet) => {
                WritePolicy::Refuse(Refused::Newer { found: *found })
            }
            (Loaded::Failed(_), Claim::Adopted | Claim::NotYet) => {
                WritePolicy::Refuse(Refused::ReadFailed)
            }
        }
    }
}

/// [`load`] 한 지금 파일의 [`Loaded::write_policy`] — 경로로 바로 읽는 주인의 저장 직전 판정.
pub fn judge(path: &Path, spec: &Spec, os: OsHooks, claim: Claim) -> WritePolicy {
    load(path, spec, os).write_policy(claim)
}

/// 쓸 원문의 길이가 [`Spec::cap`] 안인가 — 넘으면 `InvalidData`(다음 적재가 그 파일을 통째로 못 쓴다).
// ADR-0291 R6: 읽기 상한 = 쓰기 상한.
pub fn check_cap(len: usize, spec: &Spec) -> io::Result<()> {
    if len as u64 > spec.cap {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "쓸 원문({len} 바이트)이 읽기 상한 {} 바이트를 넘는다",
                spec.cap
            ),
        ));
    }
    Ok(())
}

// ── 읽기 · 쓰기 장치 ──

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
/// 읽기 자체의 IO 실패이고, 둘을 가르는 것이 [`classify`] 다 — 내용이 못 쓸 것이면 손상, IO 실패면 손대지
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
/// 대상 폴더가 없으면 그 위로 없는 폴더까지 만든다 — 만든 폴더는 쓰기가 실패해도 남는다. 갈아끼운 뒤에는
/// [`OsHooks::sync_dir`] 로 그 폴더를 동기화한다(결과는 보지 않는다).
///
/// 같은 경로를 동시에 쓰는 호출끼리는 나중에 rename 한 쪽이 남는다 — 어느 쪽이 나중인지는 정하지 않는다. 순서가
/// 중요하면 호출자가 직렬화한다.
// ADR-0265 결정 4: 설정 쓰기는 원자적이다(임시 파일 → sync_all → rename).
// ADR-0291 R10 · R11: rename 뒤 폴더 동기화 · 폴더는 쓰는 쪽이 쓸 때 만든다.
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
    write_atomic_with(
        path,
        bytes,
        &mut |attempt| (os.retry)(attempt),
        &mut |dir| (os.sync_dir)(dir),
        &skip,
    )
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    retry: RetryHook,
    sync_dir: SyncDirHook,
    skip: &dyn Fn() -> bool,
) -> io::Result<WriteOutcome> {
    use std::io::Write;

    replace_with(
        path,
        |file| file.write_all(bytes),
        skip,
        |from, to| std::fs::rename(from, to),
        retry,
        sync_dir,
    )
}

/// `from` 을 `to` 에 원자적으로 복사한다 — 흘려 쓰므로 통째로 메모리에 올리지 않고 크기 상한도 없다. 있던 `to`
/// 는 덮는다. 실패하면 임시 파일을 치우고 `to` 는 그대로다.
///
/// `from` 열기와 rename 이 잠김이면 [`OsHooks::retry`] 로 다시 한다. 끝내 못 열면 임시 파일도 `to` 의 폴더도
/// 만들기 전에 그 오류다. 폴더 만들기 · 동기화는 [`write_atomic`] 과 같다.
// ADR-0274
// ADR-0291 R10 · R11
pub fn copy_atomic(from: &Path, to: &Path, os: OsHooks) -> io::Result<()> {
    copy_atomic_with(from, to, &mut |attempt| (os.retry)(attempt), &mut |dir| {
        (os.sync_dir)(dir)
    })
}

fn copy_atomic_with(
    from: &Path,
    to: &Path,
    retry: RetryHook,
    sync_dir: SyncDirHook,
) -> io::Result<()> {
    // `std::fs::copy` 를 쓰지 않는다 — 원본의 권한까지 옮겨(Windows 읽기 전용 속성 · Unix 권한 비트), 읽기 전용
    //   원본이면 사본도 읽기 전용이 되어 `sync_all` 할 쓰기 핸들을 못 연다.
    let mut source = retrying(&mut *retry, || std::fs::File::open(from))?;
    replace_with(
        to,
        |file| io::copy(&mut source, file).map(drop),
        || false,
        |from, to| std::fs::rename(from, to),
        retry,
        sync_dir,
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
/// 쌓인다. `Written` 이면 그 폴더를 `sync_dir` 한다 — rename 은 폴더의 항목을 바꾸는 일이라, 파일의 `sync_all`
/// 만으로는 그 바뀜이 디스크에 남는다는 보장이 없다(POSIX).
///
/// ★rename 이 잠김으로 실패하면 `retry` 로 다시 한다★ — 백신 · 색인기가 대상 파일을 잠깐 쥐면 Windows 의 rename 이
/// 그렇게 실패한다.
///
/// `rename` · `retry` · `sync_dir` 는 시험의 이음매다 — 운영은 `std::fs::rename` · 부르는 쪽의 [`OsHooks`].
// ADR-0265 결정 4: rename 이 잠김이면 짧게 다시 한다.
// ADR-0291 R10 · R11
fn replace_with(
    path: &Path,
    stage: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
    skip: impl Fn() -> bool,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
    retry: RetryHook,
    sync_dir: SyncDirHook,
) -> io::Result<WriteOutcome> {
    let tmp = temp_path(path)?;
    let dir = folder_of(path);

    let staged = (|| {
        // 적재는 폴더를 만들지 않으므로 첫 쓰기가 만든다 — 실행 중에 지워진 폴더도 여기서 다시 선다.
        std::fs::create_dir_all(dir)?;
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
    if matches!(outcome, Ok(WriteOutcome::Written)) {
        let _ = sync_dir(dir);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome
}

/// `path` 가 든 폴더 — 폴더 없이 이름만 준 경로(`a.json`)는 지금 폴더(`.`)다. 빈 경로를 넘기면 폴더 동기화가
/// 그 폴더를 못 연다.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
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
            &mut |_| panic!("건너뛴 쓰기는 폴더를 동기화하지 않는다"),
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
            &mut |_| panic!("실패한 쓰기는 폴더를 동기화하지 않는다"),
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
            &mut |_| panic!("채우기가 실패하면 폴더를 동기화하지 않는다"),
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

    // ── 폴더 만들기 · 동기화 ──

    #[test]
    fn a_write_creates_the_missing_folders_up_to_the_target() {
        let dir = temp_dir("mkdir");
        let folder = dir.join("shell").join("state");
        let path = folder.join("state.json");

        write_atomic(&path, b"new", ONCE).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&folder), vec!["state.json".to_string()]);

        // 실행 중에 지워진 폴더도 다음 쓰기가 다시 세운다.
        std::fs::remove_dir_all(dir.join("shell")).unwrap();
        write_atomic(&path, b"again", ONCE).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_written_file_syncs_its_folder_once_after_the_rename() {
        let dir = temp_dir("sync");
        let path = dir.join("state.json");
        std::fs::write(&path, "old").unwrap();
        let mut synced = Vec::new();

        let outcome = write_atomic_with(
            &path,
            b"new",
            &mut |attempt| attempt(),
            &mut |folder| {
                synced.push((
                    folder.to_path_buf(),
                    std::fs::read_to_string(&path).unwrap(),
                ));
                Ok(())
            },
            &|| false,
        )
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Written);
        assert_eq!(
            synced,
            vec![(dir.clone(), "new".to_string())],
            "갈아끼운 뒤 그 폴더를 한 번"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_folder_sync_does_not_fail_the_write() {
        let dir = temp_dir("sync-fail");
        let path = dir.join("state.json");

        let outcome = write_atomic_with(
            &path,
            b"new",
            &mut |attempt| attempt(),
            &mut |_| Err(io::Error::other("가짜 동기화 실패")),
            &|| false,
        )
        .unwrap();

        assert_eq!(outcome, WriteOutcome::Written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(names_in(&dir), vec!["state.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_copy_creates_the_missing_folder_of_the_copy_and_syncs_it() {
        let dir = temp_dir("copy-sync");
        let from = dir.join("a.json");
        std::fs::write(&from, "from").unwrap();
        let aside = dir.join("aside");
        let to = aside.join("b.json");
        let mut synced = Vec::new();

        copy_atomic_with(&from, &to, &mut |attempt| attempt(), &mut |folder| {
            synced.push(folder.to_path_buf());
            Ok(())
        })
        .unwrap();

        assert_eq!(std::fs::read_to_string(&to).unwrap(), "from");
        assert_eq!(synced, vec![aside.clone()]);
        assert_eq!(names_in(&aside), vec!["b.json".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_copy_from_a_missing_file_creates_no_folder() {
        let dir = temp_dir("copy-missing-folder");
        let to = dir.join("aside").join("b.json");

        let err = copy_atomic(&dir.join("none.json"), &to, ONCE).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            !dir.join("aside").exists(),
            "원본을 못 열면 아무것도 만들지 않는다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bare_file_name_lives_in_the_current_folder() {
        assert_eq!(folder_of(Path::new("a.json")), Path::new("."));
        assert_eq!(
            folder_of(&Path::new("dir").join("a.json")),
            Path::new("dir")
        );
    }

    // ── 읽기 규칙 ──

    /// 시험 파일의 규칙 — 버전 키 `v` · 1 판 · 64 바이트 · 모양은 안 본다.
    const SPEC: Spec = Spec {
        version_key: "v",
        current: 1,
        cap: 64,
        shape: any_shape,
    };

    /// 최상위 `items` 가 배열이어야 하는 주인 — 모양 실패를 만든다.
    const ITEMS: Spec = Spec {
        shape: items_shape,
        ..SPEC
    };

    fn items_shape(_: u64, doc: &Map<String, Value>) -> Result<(), String> {
        match doc.get("items") {
            Some(Value::Array(_)) => Ok(()),
            _ => Err("`items` 가 배열이 아니다".to_string()),
        }
    }

    fn usable_doc(version: u64, json: &str) -> Parsed {
        let Value::Object(doc) = serde_json::from_str(json).unwrap() else {
            panic!("객체가 아니다: {json}")
        };
        Parsed::Usable { version, doc }
    }

    #[test]
    fn a_missing_version_key_reads_as_the_first_version() {
        assert_eq!(parse(r#"{"a":1}"#, &SPEC), usable_doc(1, r#"{"a":1}"#));
        assert_eq!(
            parse(r#"{"v":1,"a":1}"#, &SPEC),
            usable_doc(1, r#"{"v":1,"a":1}"#),
            "버전 키도 문서에 남는다"
        );
    }

    #[test]
    fn a_newer_version_is_newer_without_consulting_the_shape() {
        let spec = Spec {
            shape: |_, _| panic!("새 판은 모양을 보지 않는다"),
            ..SPEC
        };
        assert_eq!(parse(r#"{"v":2}"#, &spec), Parsed::Newer { found: 2 });
        assert_eq!(
            parse(r#"{"v":7.0,"items":"x"}"#, &spec),
            Parsed::Newer { found: 7 }
        );
        assert_eq!(
            parse(r#"{"v":18446744073709551615}"#, &spec),
            Parsed::Newer { found: u64::MAX }
        );
    }

    #[test]
    fn an_older_version_is_read_only_when_the_owners_shape_takes_it() {
        fn current_only(version: u64, _: &Map<String, Value>) -> Result<(), String> {
            if version == 2 {
                Ok(())
            } else {
                Err(format!("{version} 판을 읽을 리더가 없다"))
            }
        }
        let no_reader = Spec {
            current: 2,
            shape: current_only,
            ..SPEC
        };
        let with_reader = Spec { current: 2, ..SPEC };

        assert_eq!(
            parse(r#"{"v":1}"#, &no_reader),
            Parsed::Unusable("1 판을 읽을 리더가 없다".to_string())
        );
        assert_eq!(
            parse("{}", &no_reader),
            Parsed::Unusable("1 판을 읽을 리더가 없다".to_string()),
            "버전 키가 없으면 1 판이라 앞 판이다"
        );
        assert_eq!(parse(r#"{"v":2}"#, &no_reader), usable_doc(2, r#"{"v":2}"#));
        assert_eq!(
            parse(r#"{"v":1}"#, &with_reader),
            usable_doc(1, r#"{"v":1}"#)
        );
    }

    #[test]
    fn a_version_must_be_a_whole_number_of_at_least_one() {
        for raw in ["1", "1.0", "1e0"] {
            let text = format!(r#"{{"v":{raw}}}"#);
            assert_eq!(parse(&text, &SPEC), usable_doc(1, &text), "{raw}");
        }
        assert_eq!(parse(r#"{"v":2.0}"#, &SPEC), Parsed::Newer { found: 2 });
        for raw in [
            "0",
            "-0.0",
            "-1",
            "1.5",
            r#""1""#,
            "null",
            "true",
            "[1]",
            "18446744073709551616",
        ] {
            assert!(
                matches!(
                    parse(&format!(r#"{{"v":{raw}}}"#), &SPEC),
                    Parsed::Unusable(_)
                ),
                "{raw}"
            );
        }
    }

    #[test]
    fn one_leading_bom_is_ignored_and_a_second_is_not() {
        assert_eq!(
            parse("\u{feff}{\"v\":1}", &SPEC),
            usable_doc(1, r#"{"v":1}"#)
        );
        assert!(matches!(
            parse("\u{feff}\u{feff}{\"v\":1}", &SPEC),
            Parsed::Unusable(_)
        ));
    }

    #[test]
    fn non_json_and_non_object_text_is_unusable() {
        for text in ["", "{", "[]", "1", r#""x""#, "null"] {
            assert!(
                matches!(parse(text, &SPEC), Parsed::Unusable(_)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn text_over_the_cap_is_unusable_however_it_was_read() {
        let spec = Spec { cap: 8, ..SPEC };
        assert_eq!(parse(r#"{"a":12}"#, &spec), usable_doc(1, r#"{"a":12}"#));
        assert!(matches!(parse(r#"{"a":123}"#, &spec), Parsed::Unusable(_)));
    }

    #[test]
    fn classify_sorts_read_results_into_missing_unusable_failed_and_parsed() {
        assert!(matches!(
            classify(Err(io::ErrorKind::NotFound.into()), &SPEC),
            Loaded::Missing
        ));
        assert!(matches!(
            classify(
                Err(io::Error::new(io::ErrorKind::InvalidData, "상한")),
                &SPEC
            ),
            Loaded::Parsed(Parsed::Unusable(_))
        ));
        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert!(
                matches!(classify(Err(kind.into()), &SPEC), Loaded::Failed(e) if e.kind() == kind),
                "{kind:?}"
            );
        }
        assert!(matches!(
            classify(Ok(r#"{"v":2}"#.to_string()), &SPEC),
            Loaded::Parsed(Parsed::Newer { found: 2 })
        ));
    }

    #[test]
    fn load_reads_real_files_and_creates_nothing() {
        let dir = temp_dir("load");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"v":1}"#).unwrap();
        assert!(matches!(
            load(&path, &SPEC, ONCE),
            Loaded::Parsed(Parsed::Usable { version: 1, .. })
        ));

        std::fs::write(&path, vec![b' '; 65]).unwrap();
        assert!(
            matches!(
                load(&path, &SPEC, ONCE),
                Loaded::Parsed(Parsed::Unusable(_))
            ),
            "상한 초과"
        );
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(
            matches!(
                load(&path, &SPEC, ONCE),
                Loaded::Parsed(Parsed::Unusable(_))
            ),
            "UTF-8 아님"
        );

        let absent = dir.join("none").join("settings.json");
        assert!(matches!(load(&absent, &SPEC, ONCE), Loaded::Missing));
        assert_eq!(
            judge(&absent, &SPEC, ONCE, Claim::NotYet),
            WritePolicy::Write
        );
        assert!(
            !dir.join("none").exists(),
            "적재 · 판정은 폴더를 만들지 않는다"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_shape_failure_is_unusable_on_every_judging_path() {
        let dir = temp_dir("shape");
        let path = dir.join("agents.json");
        let text = r#"{"v":1,"items":"x"}"#;
        std::fs::write(&path, text).unwrap();
        let broken = Parsed::Unusable("`items` 가 배열이 아니다".to_string());

        assert_eq!(parse(text, &ITEMS), broken);
        assert!(matches!(
            classify(Ok(text.to_string()), &ITEMS),
            Loaded::Parsed(parsed) if parsed == broken
        ));
        assert!(matches!(
            load(&path, &ITEMS, ONCE),
            Loaded::Parsed(parsed) if parsed == broken
        ));
        assert_eq!(
            judge(&path, &ITEMS, ONCE, Claim::Adopted),
            WritePolicy::CopyAsideFirst
        );
        // 같은 파일이 모양을 안 보는 주인에겐 쓸 수 있음이다 — 위 판정이 모양 검사에서 왔다는 대조.
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Write
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── 쓰기 판정 ──

    #[test]
    fn the_write_policy_table() {
        let usable = || Loaded::Parsed(parse("{}", &SPEC));
        let broken = || Loaded::Parsed(Parsed::Unusable("x".to_string()));
        let newer = || Loaded::Parsed(Parsed::Newer { found: 3 });
        let failed = || Loaded::Failed(denied());
        let refused_newer = WritePolicy::Refuse(Refused::Newer { found: 3 });
        let refused_read = WritePolicy::Refuse(Refused::ReadFailed);
        let cases = [
            ("없음", Claim::Adopted, Loaded::Missing, WritePolicy::Write),
            ("없음", Claim::NotYet, Loaded::Missing, WritePolicy::Write),
            ("쓸 수 있음", Claim::Adopted, usable(), WritePolicy::Write),
            (
                "쓸 수 있음",
                Claim::NotYet,
                usable(),
                WritePolicy::CopyAsideFirst,
            ),
            (
                "손상",
                Claim::Adopted,
                broken(),
                WritePolicy::CopyAsideFirst,
            ),
            ("손상", Claim::NotYet, broken(), WritePolicy::CopyAsideFirst),
            ("새 판", Claim::Adopted, newer(), refused_newer),
            ("새 판", Claim::NotYet, newer(), refused_newer),
            ("읽기 실패", Claim::Adopted, failed(), refused_read),
            ("읽기 실패", Claim::NotYet, failed(), refused_read),
        ];
        for (file, claim, loaded, expected) in cases {
            assert_eq!(loaded.write_policy(claim), expected, "{file} · {claim:?}");
        }
    }

    #[test]
    fn judge_reads_the_file_as_it_is_now_on_every_call() {
        let dir = temp_dir("judge");
        let path = dir.join("presets.json");
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Write
        );

        std::fs::write(&path, r#"{"v":1}"#).unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Write
        );
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::NotYet),
            WritePolicy::CopyAsideFirst
        );

        std::fs::write(&path, r#"{"v":2}"#).unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Refuse(Refused::Newer { found: 2 })
        );

        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::CopyAsideFirst
        );

        // 폴더 자리라 열기나 읽기가 내용과 무관하게 실패한다.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            judge(&path, &SPEC, ONCE, Claim::Adopted),
            WritePolicy::Refuse(Refused::ReadFailed)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn check_cap_takes_exactly_the_cap_and_refuses_one_byte_more() {
        assert!(check_cap(64, &SPEC).is_ok());
        assert_eq!(
            check_cap(65, &SPEC).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
