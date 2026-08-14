//! 단일 인스턴스 가드 — 데이터 폴더의 `daemon.json` 을 **공유 모드 제한**으로 붙잡는다(ADR-0135).
//!
//! 데몬은 **데이터 폴더 하나당 하나**만 떠야 한다. 판정은 그 폴더의 접속 파일을 읽기+쓰기로 열어
//! **핸들을 얻는 것 자체**다 — 얻었으면 이 폴더는 우리 것이고, 그러지 못하면 남이 쥐고 있거나 제3자가
//! 방해하는 중이다.
//!
//! ★잡는 파일 = 클라이언트가 읽는 파일(ADR-0135 결정 1)★: 잠금 파일과 접속 파일을 나누면, 데몬을
//! 켜둔 채 데이터 폴더를 지웠을 때 **지워지지 않는 잠금 파일만 남고 주소가 사라진다** — 클라이언트는
//! 붙지도(주소 없음) 끄지도(끄기 경로가 그 주소를 쓴다) 못하고, 새로 띄운 데몬은 잠금에 막혀 조용히
//! 물러난다. 작업 관리자 외에 탈출구가 없는 상태였다. ★다시 두 파일로 쪼개지 마라★ — 합쳐 두면 그
//! 상태 자체를 만들 수 없다.
//!
//! ## 공유 모드 = 읽기만 허용(`FILE_SHARE_READ` = 1)
//!
//! 실측(2026-08-14, 공유 모드 조합 프로브 · 리뷰어 독립 재실측):
//! - 두 번째 데몬의 읽기+쓰기 열기 → `ERROR_SHARING_VIOLATION`(32). **배제는 이것이 전부다.**
//!   중재가 **요청한 접근**에 걸리므로 상대가 공유를 1·3·7 중 무엇으로 잡든, 쓰기 전용·append 로 열든
//!   모두 거부된다.
//! - 이 파일의 삭제·이름 변경 → 32. 데이터 폴더 이름 변경 → `ERROR_ACCESS_DENIED`(5).
//! - 별칭 8종(대문자·`..`·`\\?\`·8.3 단축명·정션·하드링크·`subst`·매핑 드라이브 vs UNC)이 전부 같은
//!   파일 객체로 수렴한다(리뷰어 실측, 실 SMB 공유 2곳 포함). OneDrive 동기화 루트도 동일.
//!
//! ★읽는 쪽은 **공유를 좁히지 않아야** 한다★: 클라이언트의 읽기 열기가 통과하는 것은 그 열기가
//! **관대한 공유**를 쓸 때뿐이다 — 우리가 쥔 접근이 읽기+쓰기라, 읽기 전용으로 열더라도 공유에서 쓰기를
//! 빼면(예: .NET `FileStream` 기본값 `FileShare.Read` · `[System.IO.File]::ReadAllText`) 32로 거부된다
//! (실측). 오늘의 소비자는 Rust `std::fs::read` 와 Node `fs.readFileSync` 라 둘 다 기본이 관대해서
//! 문제가 없다. **새 소비자를 붙일 때 이 줄을 볼 것** — "읽기는 통과한다"로 일반화하면 거기서 깨진다.
//!
//! ★`FILE_SHARE_WRITE` 를 더하지 마라(되살리지 마라)★: 쓰기 공유는 곧 "남의 **쓰기** 열기를 허용한다"는
//! 뜻이라, 공유 3(읽기+쓰기)에서는 두 번째 데몬의 읽기+쓰기 열기가 **그대로 성공한다**(실측 — 32가
//! 아니라 OK). 그러면 한 폴더에 데몬이 둘 뜨고 클라이언트마다 다른 endpoint 를 믿는다.
//!
//! ★`FILE_SHARE_DELETE` 도 더하지 마라(되살리지 마라)★: 삭제·이름 변경이 열리면 누가 이 파일을 치우고
//! 두 번째 데몬이 같은 경로에 **새 파일**을 만들어 그것을 잡는다 — 결과는 위와 같다.
//!
//! ★구간 잠금(`try_lock`)을 되살리지 마라★: MS 문서상 배타 구간 잠금은 다른 프로세스의 **읽기까지**
//! 거부하고 std 의 API 는 파일 **전체**를 잠근다. 클라이언트가 이 파일을 못 읽으면 합친 의미가 없다.
//!
//! ★비-Windows 에서는 [`acquire`] 가 **아예 거부한다**(`Unsupported`)★: `share_mode` 는 Windows 전용이고
//! 대체 기전을 두지 않았다 — 데몬 자체가 Windows 전용이다(WMI spawn·Job Object·taskkill). ★"아무것도
//! 보장하지 않는 guard" 를 돌려주지 마라(되살리지 마라)★: 그러면 한 폴더에 데몬 둘이 다 성공하고 뒤에
//! 뜬 쪽이 앞선 쪽의 endpoint 를 조용히 덮어쓴다 — 산문으로만 적어 두면 테스트가 공허하게 통과한다.
//! 그래서 공유 의미를 단언하는 테스트 모듈 **전체**가 `#[cfg(windows)]` 다.
//!
//! ★제3자의 제한적 열기와 진짜 중복을 구분한다(ADR-0135 §영향)★: 백신·인덱서·백업이 좁은 공유로
//! 잠깐 열어도 우리 열기는 똑같이 32로 실패한다(실측). 그래서 짧게 재시도하고, 그래도 안 되면 파일을
//! **읽기 전용으로 열어** 살아 있는 데몬의 레코드가 보이는지 본다 — 보이면 중복(정상 양보), 아니면
//! [`AcquireError::FileBusy`]. ★이 읽기는 진단이지 신원 판정이 아니다★: 신원은 여전히 "핸들을 쥐었나"
//! 하나뿐이고, 파일에 적힌 값으로 소유를 주장하지 않는다 — 적힌 값을 신원으로 쓰면 폴더를 통째로
//! 복사한 두 배포판이 같은 신원을 주장해 한쪽이 못 뜬다(복사는 포터블 배포의 정상 사용이다).
//!
//! ★폴더를 이름·경로로 식별하지 마라(되살리지 마라)★: 잠금에 이름을 붙이려면 폴더 경로를 우리가
//! 정규화해야 하는데, 대소문자·`..`·8.3·정션·subst·매핑드라이브 표기 중 하나만 놓쳐도 **같은 폴더에
//! 데몬이 둘** 뜬다. 파일을 직접 잡으면 그 표기 해석은 OS 몫이다.
//!
//! ★도는 동안의 복사 — 잔여★: 일반 복사(`copy`)는 성공한다(실측). 다만 복사 도구가 원본을 **읽기
//! 공유만 허용**하며 열면 우리가 쥔 쓰기 접근과 충돌해 32로 실패할 수 있다.
//!
//! ★사정거리가 한 컴퓨터를 넘는다 — 단 파일시스템이 해 줄 때만★: Windows SMB 는 리다이렉터가
//! ShareAccess 를 전파하고 서버가 중재하므로 같은 폴더를 보는 **다른 컴퓨터**의 데몬도 배제된다(실측).
//! 그러나 공유 중재는 **파일시스템의 몫**이라 제3자 리다이렉터·유저모드 파일시스템(WebDAV · Windows NFS
//! 클라이언트 · Dokan/WinFsp 기반 rclone·sshfs-win · Google Drive 가상 드라이브)에서는 구현자가 이를
//! 무시할 수 있고, 그러면 **두 데몬이 다 획득해 각자 발행한다**. `try_lock` 이 빠진 지금 2차 방어선은
//! 없다 — 이런 곳에 데이터 폴더를 두지 말 것.
//!
//! ★수명 규칙★: 획득한 파일은 **데몬 프로세스가 사는 동안 계속 들고 있어야** 한다. guard 가 Drop 되면
//! 파일이 닫히며 단일성 보장이 깨지므로, 데몬 `run()` 은 반환된 guard 를 프로세스 종료 시점까지
//! 살려둔다. 프로세스가 비정상 종료해도 OS 가 핸들을 닫는데, **즉시가 보장되지는 않는다** — MS 는
//! 해제 시점이 시스템 자원 사정에 달렸다고 하고, SMB 공유에서 클라이언트가 전원째 끊기면 세션·durable
//! handle 타이머가 만료될 때까지 서버가 열림을 유지한다. 즉 낡은 항목 판정은 필요 없지만 "죽자마자
//! 곧바로 재기동된다"고 단정하지도 말 것.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::portfile::{self, DaemonInfo, DAEMON_FILE};

/// 제3자가 제한적 공유로 쥐고 있는 경우를 넘기기 위한 **열기** 재시도 횟수(첫 시도 포함).
const OPEN_ATTEMPTS: u32 = 5;

/// 열기 재시도 사이 대기.
///
/// ★[`acquire`] 가 더하는 최악 지연 = `(OPEN_ATTEMPTS - 1) × OPEN_RETRY_DELAY` = **약 400ms**★
/// (마지막 시도 뒤에는 자지 않는다 — 아무것도 벌지 못하고 사용자가 기다릴 오류만 늦춘다).
/// 정상 경로(첫 시도 성공)는 0ms 이고, **중복 데몬도 이 예산을 다 쓴다** — 중복과 제3자 방해가 같은
/// 32로 갈리지 않아 재시도를 끝까지 돈 뒤에야 진단으로 나뉘기 때문이다. 그 지연은 물러날 데몬 쪽에만
/// 붙으므로 사용자가 보는 것은 이미 떠 있는 데몬이다. 이 곱을 키우기 전에 클라이언트 ensure 폴링
/// 상한(5초)과 견줄 것.
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(100);

/// 이 폴더를 소유한다는 증명. 살아 있는 동안 파일 핸들이 열려 있고, Drop 이 곧 해제다.
pub struct InstanceGuard {
    file: File,
}

impl InstanceGuard {
    /// 접속 레코드를 **보유 중인 핸들로 제자리에** 기록한다(ADR-0135 결정 1).
    ///
    /// ★임시 파일 + rename 으로 바꾸지 마라★: 우리가 삭제 공유를 닫은 채 이 파일을 쥐고 있어 rename 이
    /// 거부된다(실측 32). 그리고 제자리 쓰기라 **클라이언트가 부분적으로 쓰인 내용을 볼 수 있다** —
    /// 읽는 쪽은 파싱 실패를 "아직 준비 안 됨"으로 보고 폴링을 계속해야 한다.
    ///
    /// ★쓰기가 획득 뒤라는 것을 타입이 보장한다★: 이 메서드는 guard 로만 부를 수 있다. 순서가
    /// 뒤집히면 두 데몬의 쓰기가 섞인다.
    // ADR-0135
    pub fn publish(&mut self, info: &DaemonInfo) -> io::Result<()> {
        portfile::write_in_place(&mut self.file, info)
    }
}

/// [`acquire`] 의 정상 결과 두 갈래. 둘 다 "시스템이 고장 난 것은 아니다".
pub enum Acquired {
    /// 우리가 이 폴더의 유일한 데몬이다.
    Held(InstanceGuard),
    /// 다른 데몬이 이미 이 폴더를 잡고 있다 — 물러나는 것이 정상 동작이다.
    ///
    /// `pid` 는 그 데몬이 파일에 남긴 값(진단용 — 소유 판정의 근거가 아니다).
    AlreadyRunning { pid: u32 },
}

/// ★중복과 분리해서 보고하기 위해 존재한다★: 둘을 한 에러로 뭉치면 데몬이 왜 안 떴는지 사용자가
/// 구분할 수 없다(중복은 정상, 아래 둘은 조치가 필요한 상태).
#[derive(Debug)]
pub enum AcquireError {
    /// 제3자가 접속 파일을 제한적 공유로 열고 있어 재시도 예산 안에 열지 못했다. **중복 데몬이 아니다.**
    FileBusy { attempts: u32, source: io::Error },
    /// 파일은 있는데 **쓰기 접근 자체가 거부**됐다(ACL 등). 기다려도 풀리지 않는다.
    ///
    /// ★따로 두는 이유★: 이 상태는 재시작·재부팅을 넘겨도 그대로라 "잠깐 붙들려 있음"과 사용자가 할
    /// 일이 정반대다. 읽기 전용 **속성**은 [`acquire`] 가 스스로 걷어내므로 여기 오지 않는다.
    AccessDenied { path: String, source: io::Error },
    /// 이 플랫폼에는 단일 인스턴스 배제 기전이 없다(모듈 헤더 — Windows 전용).
    Unsupported,
    /// 그 밖의 시스템 오류(폴더 부재 등).
    Io(io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileBusy { attempts, source } => write!(
                f,
                "{DAEMON_FILE} 을 다른 프로그램이 붙들고 있음({attempts}회 재시도 후 포기): {source}"
            ),
            Self::AccessDenied { path, source } => write!(
                f,
                "{path} 에 쓸 권한이 없음(읽기 전용 속성은 자동 해제를 시도했으나 실패 — 폴더 권한을 확인하거나 쓰기 가능한 위치에 압축을 풀어 주세요): {source}"
            ),
            Self::Unsupported => write!(
                f,
                "이 플랫폼에는 단일 인스턴스 배제 기전이 없음(Windows 전용)"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AcquireError {}

/// 소유 목적으로 연다 — 읽기+쓰기, 없으면 생성, 공유는 **읽기만**(모듈 헤더 참조).
///
/// ★`truncate` 를 걸지 마라★: 여는 순간 남의 유효한 레코드를 지운다. 우리가 이기면 [`InstanceGuard::publish`]
/// 가 덮어쓰고, 지면 파일에 손대지 않는 것이 맞다.
#[cfg_attr(not(windows), allow(dead_code))]
fn open_owned(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ 단독. 여기에 WRITE 나 DELETE 를 더하면 배제가 사라진다 — 헤더의
        //   두 ★되살리지 마라★가 이 상수 하나에 걸려 있다.
        const SHARE_READ_ONLY: u32 = 1;
        opts.share_mode(SHARE_READ_ONLY);
    }
    opts.open(path)
}

/// 열기가 계속 막힐 때 **왜** 막히는지만 본다 — 살아 있는 데몬의 레코드가 있으면 그 pid.
///
/// ★진단이지 신원 판정이 아니다★: 여기서 무엇이 보이든 소유는 이미 "핸들을 못 얻었다"로 결정돼 있고,
/// 이 값은 사용자에게 낼 메시지를 고르는 데만 쓴다. 읽기는 관대한 공유(std 기본)로 열므로 우리가
/// 실패한 그 파일도 읽힌다.
///
/// ★잔여 1 — 앞선 데몬이 **아직 발행 전**이면 중복을 방해로 오진한다★: 그 데몬이 파일을 만들어
/// 쥐었지만 레코드를 쓰기 전이라 읽을 것이 없다. 그 창은 획득 직후가 아니라 **bind + MCP 서버 기동 +
/// 전체 배선**까지 걸쳐 있고(데몬 `run()` 의 3~8단계), 실측(2026-08-14) 따뜻한 로컬 기동에서 16~19ms
/// 라 400ms 예산이 20배 여유지만 **콜드 스타트나 네트워크 데이터 폴더에서는 넘길 수 있다**. 오진해도
/// 결과는 로그 문구와 종료 코드뿐이다(둘 다 소비자가 없다 — 클라이언트는 파일을 폴링한다). 그래서
/// 별도 대기 예산을 두지 않았다.
///
/// ★잔여 2 — 반대 방향의 오진은 못 막는다★: 제3자가 붙들고 있는데 파일에 남은 옛 레코드의 pid 가
/// 마침 살아 있으면(다른 폴더 데몬의 레코드를 복사해 왔거나 pid+생성시각이 우연히 맞는 경우)
/// "이미 실행 중"으로 읽고 조용히 exit 0 한다. 진단 읽기로는 원리적으로 가를 수 없다.
#[cfg_attr(not(windows), allow(dead_code))]
fn live_owner(path: &Path) -> Option<u32> {
    let info = portfile::read(path)?;
    if portfile::is_stale(&info) {
        None
    } else {
        Some(info.pid)
    }
}

/// 잠금 획득 시도.
///
/// `data_dir` 은 **이미 존재해야 한다** — 폴더를 만드는 것은 호출자(데몬 기동 순서) 몫이다.
///
/// ★"이미 실행 중"의 근거는 열기 실패 + 살아 있는 레코드 둘이 겹칠 때뿐이다★: 열기 실패 하나만으로
/// 중복을 선언하지 말 것. 열기는 제3자 핸들·권한 등 데몬과 무관한 이유로도 실패하고, 그걸 중복으로
/// 읽으면 데몬이 원인을 남기지 않고 종료한다(원인 없는 연결 시간 초과 = ADR-0134 결정 4가 없애려는 그 증상).
// ADR-0135
#[cfg(windows)]
pub fn acquire(data_dir: &Path) -> Result<Acquired, AcquireError> {
    acquire_with(data_dir, OPEN_ATTEMPTS, OPEN_RETRY_DELAY)
}

/// ★guard 를 흉내내지 않는다(모듈 헤더)★: 배제할 수단이 없는 플랫폼에서 `Held` 를 돌려주면 데몬 둘이
/// 다 뜨고 뒤엣것이 앞엣것의 endpoint 를 덮어쓴다. 호출자가 기동을 멈추게 실패로 알린다.
#[cfg(not(windows))]
pub fn acquire(_data_dir: &Path) -> Result<Acquired, AcquireError> {
    Err(AcquireError::Unsupported)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn acquire_with(data_dir: &Path, attempts: u32, delay: Duration) -> Result<Acquired, AcquireError> {
    let path = data_dir.join(DAEMON_FILE);
    let mut tried = 0;
    let mut cleared_readonly = false;
    loop {
        tried += 1;
        match open_owned(&path) {
            Ok(file) => return Ok(Acquired::Held(InstanceGuard { file })),
            Err(e) if e.raw_os_error() == Some(ACCESS_DENIED) => {
                // ★읽기 전용 **속성**은 걷어내고 한 번 더 해 본다(load-bearing — 없으면 영구 교착)★:
                //   속성 설정은 공유 중재를 우회해 **우리가 쥔 동안에도** 성공하고(실측), 그 뒤
                //   재시작하면 쓰기 열기가 32가 아니라 5로 막힌다 — 재시도 대상이 아니라 첫 시도에
                //   Io 로 나가 데몬이 exit 1 하고, 릴리스에선 그 로그를 볼 콘솔조차 없다. 클라이언트의
                //   폴더 프로브는 **다른 이름**의 파일을 쓰므로 통과해 원인도 못 잡는다. 결과는
                //   재부팅으로도 안 풀리는 5초 연결 시간 초과다. 백업 복원·읽기 전용 매체 복사·백신
                //   격리 해제가 모두 이 속성을 남긴다.
                //   ★한 번만★: 지운 뒤에도 5면 속성이 아니라 ACL 이라 기다려도·다시 해도 같다.
                if !cleared_readonly && clear_readonly_attr(&path) {
                    cleared_readonly = true;
                    continue;
                }
                return Err(AcquireError::AccessDenied {
                    path: path.display().to_string(),
                    source: e,
                });
            }
            Err(e) => {
                // ★공유 위반만 재시도한다★: 그것만이 "잠깐 뒤엔 될 수도 있는" 실패다. 폴더 부재 같은
                //   실패는 기다려도 달라지지 않고, 그걸 FileBusy 로 보고하면 원인을 "다른 프로그램 탓"
                //   으로 잘못 지목한다.
                if e.raw_os_error() != Some(SHARING_VIOLATION) {
                    return Err(AcquireError::Io(e));
                }
                // ★마지막 시도 뒤에는 자지 않는다★: 그 대기는 아무것도 벌지 못하고 사용자가
                //   기다리는 오류만 늦춘다. 그래서 대기는 시도 사이에만 들어간다(총 attempts-1 회).
                if tried >= attempts {
                    if let Some(pid) = live_owner(&path) {
                        return Ok(Acquired::AlreadyRunning { pid });
                    }
                    // ★진단 읽기 뒤 한 번 더 연다★: 방해가 그 사이에 걷혔을 수 있는데, 그때
                    //   FileBusy 를 내면 **지금은 잡을 수 있는** 폴더를 두고 데몬이 물러난다.
                    //   ★이 성공 갈래는 테스트가 없다(알려진 미검증)★ — 방해 핸들이 정확히 이 두 줄
                    //   사이에 닫히게 만들 결정적 수단이 없다. 실패 갈래는 busy 테스트가 덮는다.
                    return match open_owned(&path) {
                        Ok(file) => Ok(Acquired::Held(InstanceGuard { file })),
                        Err(_) => Err(AcquireError::FileBusy {
                            attempts: tried,
                            source: e,
                        }),
                    };
                }
                std::thread::sleep(delay);
            }
        }
    }
}

/// 읽기 전용 **속성**을 걷어낸다. 성공(=이제 속성이 없다)이면 true.
///
/// ★ACL 은 손대지 않는다★: 권한 편집은 사용자 정책을 바꾸는 일이라 데몬이 할 일이 아니다. 이 함수는
/// 우리 자신의 런타임 파일에 붙은 **속성 하나**만 되돌린다.
#[cfg_attr(not(windows), allow(dead_code))]
fn clear_readonly_attr(path: &Path) -> bool {
    let Ok(md) = std::fs::metadata(path) else {
        return false;
    };
    let mut perms = md.permissions();
    if !perms.readonly() {
        return false; // 속성 탓이 아니었다 — 다시 열어 봐야 같은 5다.
    }
    perms.set_readonly(false);
    std::fs::set_permissions(path, perms).is_ok()
}

/// `ERROR_SHARING_VIOLATION`. std 는 이 코드를 `Uncategorized` 로 분류해 `ErrorKind` 로는 못 가른다
/// (실측 2026-08-14) — 재시도 여부를 정하려면 raw 코드를 봐야 한다.
///
/// ★이 상수만으로 중복을 판정하지 말 것★: 여기서의 쓰임은 "기다리면 풀릴 수도 있나"뿐이다.
#[cfg(windows)]
const SHARING_VIOLATION: i32 = 32;
#[cfg(not(windows))]
const SHARING_VIOLATION: i32 = i32::MIN;

/// `ERROR_ACCESS_DENIED`. 읽기 전용 **속성**이 붙은 파일을 쓰기로 열 때 오는 코드다(실측 —
/// `PermissionDenied` 로 분류되긴 하지만 32와 갈라야 해서 raw 로 본다).
#[cfg(windows)]
const ACCESS_DENIED: i32 = 5;
#[cfg(not(windows))]
const ACCESS_DENIED: i32 = i32::MIN + 1;

/// ★비-Windows 에서 유일하게 단언할 것★: guard 를 흉내내지 않는다는 것.
#[cfg(all(test, not(windows)))]
mod non_windows_tests {
    #[test]
    fn acquire_refuses_instead_of_returning_a_guard_that_guarantees_nothing() {
        let got = super::acquire(std::path::Path::new("."));
        assert!(
            matches!(got, Err(super::AcquireError::Unsupported)),
            "배제 기전이 없는 플랫폼에서 Held 를 돌려주면 데몬 둘이 다 뜬다"
        );
    }
}

// ★전부 Windows 게이트다(의도)★: 아래 단언은 전부 공유 모드 중재에 기대고, 다른 플랫폼에서는
//   `acquire` 가 애초에 `Unsupported` 를 낸다 — 게이트를 풀면 공허하게 통과하거나 그냥 깨진다.
#[cfg(all(test, windows))]
mod tests {
    use super::{acquire, acquire_with, Acquired, DaemonInfo, DAEMON_FILE};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn held(r: Result<Acquired, super::AcquireError>) -> bool {
        matches!(r, Ok(Acquired::Held(_)))
    }

    /// 테스트끼리(그리고 cargo 병렬 실행과) 폴더가 겹치지 않게 유니크 경로를 만든다 — 잠금 스코프가
    /// 폴더이므로 폴더가 겹치면 그 자체로 서로를 거부한다.
    fn fresh_dir(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("engram-instance-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("테스트 폴더 생성");
        dir
    }

    /// 살아 있는 데몬으로 보이게 하려면 pid 가 실제로 살아 있어야 한다 — 자기 프로세스를 쓴다.
    fn record_of_this_process() -> DaemonInfo {
        DaemonInfo {
            pid: std::process::id(),
            host: "127.0.0.1".into(),
            port: 54321,
            token: "a".repeat(64),
            protocol_version: 1,
            start_time: 0,
        }
    }

    fn write_live_record(path: &Path) {
        let bytes = record_of_this_process().to_json_pretty().expect("직렬화");
        std::fs::write(path, bytes).expect("레코드 선기록");
    }

    #[test]
    fn second_guard_on_same_folder_is_refused() {
        let dir = fresh_dir("same");
        let mut first = match acquire(&dir).expect("첫 획득은 시스템 오류가 아님") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("첫 데몬은 잠금을 얻어야"),
        };
        // 중복 판정은 "열기 실패 + 살아 있는 레코드" 둘이 겹칠 때다 — 첫 데몬이 발행한 뒤를 재현한다.
        first.publish(&record_of_this_process()).expect("발행");

        let fast = || acquire_with(&dir, 2, Duration::from_millis(1));
        assert!(
            matches!(fast(), Ok(Acquired::AlreadyRunning { .. })),
            "같은 폴더의 두 번째 데몬은 거부돼야"
        );
        // 거부가 일회성 상태가 아니다.
        assert!(
            matches!(fast(), Ok(Acquired::AlreadyRunning { .. })),
            "첫 가드가 살아 있는 동안 계속 거부돼야"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guards_on_different_folders_coexist() {
        // 포터블 배포판을 두 곳에 풀어 각자 돌리는 정상 사용.
        let a = fresh_dir("diff-a");
        let b = fresh_dir("diff-b");
        let ga = acquire(&a).expect("A 획득");
        let gb = acquire(&b).expect("B 획득");
        assert!(
            matches!(ga, Acquired::Held(_)) && matches!(gb, Acquired::Held(_)),
            "다른 폴더면 둘 다 떠야"
        );
        drop(ga);
        drop(gb);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn dropping_guard_releases_the_folder() {
        let dir = fresh_dir("release");
        let first = acquire(&dir).expect("첫 획득");
        drop(first);
        assert!(
            held(acquire(&dir)),
            "가드를 놓으면 같은 폴더를 다시 잡을 수 있어야"
        );
        assert!(dir.join(DAEMON_FILE).exists(), "접속 파일은 폴더 안에 산다");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★이 테스트가 지키는 것이 합친 이유 전부다(ADR-0135 결정 1)★: 데몬이 쥔 그 파일을 클라이언트가
    /// **읽을 수 있어야** 접속이 성립한다. 공유를 좁히거나 구간 잠금을 되살리면 여기서 깨진다.
    #[test]
    fn a_client_can_read_the_record_while_the_daemon_holds_it() {
        let dir = fresh_dir("read-while-held");
        let mut guard = match acquire(&dir).expect("획득") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("빈 폴더는 획득돼야"),
        };
        let want = record_of_this_process();
        guard.publish(&want).expect("발행");

        let path = dir.join(DAEMON_FILE);
        let bytes = std::fs::read(&path).expect("쥐고 있는 동안에도 읽혀야");
        assert_eq!(
            DaemonInfo::parse(&bytes).expect("파싱"),
            want,
            "쓴 내용 그대로 읽혀야"
        );
        // 읽기 전용 열기(관대한 공유)도 함께 못 박는다 — 진단 경로가 쓰는 형태다.
        assert!(
            std::fs::OpenOptions::new().read(true).open(&path).is_ok(),
            "읽기 전용 열기도 허용돼야"
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★쓰기는 획득 뒤에만(ADR-0135 §영향)★: 획득 실패 경로는 파일에 손대지 않는다 — 손대면 두 데몬의
    /// 쓰기가 섞이고, 물러나는 쪽이 이미 붙어 있는 클라이언트의 주소를 지운다.
    #[test]
    fn a_refused_daemon_does_not_touch_the_record() {
        let dir = fresh_dir("no-write-on-refusal");
        let mut first = match acquire(&dir).expect("첫 획득") {
            Acquired::Held(g) => g,
            Acquired::AlreadyRunning { .. } => panic!("빈 폴더는 획득돼야"),
        };
        let owner = record_of_this_process();
        first.publish(&owner).expect("발행");

        assert!(
            matches!(
                acquire_with(&dir, 2, Duration::from_millis(1)),
                Ok(Acquired::AlreadyRunning { .. })
            ),
            "두 번째는 거부돼야"
        );
        let bytes = std::fs::read(dir.join(DAEMON_FILE)).expect("읽기");
        assert_eq!(
            DaemonInfo::parse(&bytes).expect("파싱"),
            owner,
            "거부된 데몬이 레코드를 건드리면 안 됨"
        );

        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★이 테스트가 막는 회귀★: 공유를 전면 거부(0)로 되돌리면 **관대한 공유로 여는** 제3자(백신·인덱서
    /// 기본 동작)만으로 우리 열기가 실패해 데몬이 조용히 종료한다.
    /// ★이름이 곧 범위다★: 제한적 공유로 여는 제3자는 여기서 다루지 않는다 — 그건 아래 별도 테스트.
    #[test]
    fn a_permissive_third_party_handle_does_not_look_like_another_daemon() {
        let dir = fresh_dir("third-party-permissive");
        let path = dir.join(DAEMON_FILE);
        // 살아 있는 레코드까지 있어도 중복으로 오인하면 안 된다 — 판정 근거는 핸들이지 내용이 아니다.
        write_live_record(&path);
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("제3자 읽기 핸들(std 기본 = 관대한 공유)");

        assert!(
            held(acquire(&dir)),
            "관대한 제3자 핸들이 열려 있어도 데몬은 잠금을 얻어야"
        );

        drop(onlooker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 제한적 공유로 쥔 제3자는 우리 열기를 실제로 막는다(실측 32). 그때 **중복으로 오인하지 않고**
    /// 구분되는 에러를 내야 한다 — 중복이면 데몬이 조용히 exit 0 하므로 사용자가 원인을 못 본다.
    /// ★파일에 살아 있는 레코드가 남아 있어도 그렇다★: 그래서 이 테스트는 stale 레코드를 쓴다 —
    /// 진단 읽기가 "살아 있는 소유자"를 못 찾으면 방해로 갈린다.
    #[test]
    fn a_restrictive_third_party_handle_is_reported_as_busy_not_as_another_daemon() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = fresh_dir("third-party-restrictive");
        let path = dir.join(DAEMON_FILE);
        let mut dead = record_of_this_process();
        dead.pid = 0; // 죽은 pid — 이 폴더를 소유한 데몬이 없다는 뜻.
        std::fs::write(&path, dead.to_json_pretty().expect("직렬화")).expect("선기록");
        const SHARE_READ_ONLY: u32 = 1;
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_ONLY)
            .open(&path)
            .expect("제한적 공유 제3자 핸들");

        // 재시도 예산을 작게 줘 테스트가 오래 걸리지 않게 한다(운영 예산은 상수).
        let got = acquire_with(&dir, 2, Duration::from_millis(1));
        assert!(
            matches!(got, Err(super::AcquireError::FileBusy { .. })),
            "중복이 아니라 '붙들려 있음'으로 보고돼야"
        );

        // 방해가 사라지면 그대로 획득된다 — 재시도가 의미 있는 이유.
        drop(onlooker);
        assert!(held(acquire(&dir)), "방해가 걷히면 획득돼야");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★영구 교착 방지(load-bearing)★: 읽기 전용 **속성**이 붙으면 우리 열기가 32가 아니라 5로 막혀
    /// 재시도에도 안 걸리고 데몬이 매 기동마다 즉시 죽는다 — 재부팅으로도 안 풀리고, 릴리스에선 그
    /// 로그를 볼 콘솔조차 없어 사용자에겐 영원한 5초 연결 시간 초과로만 보인다. 백업 복원·읽기 전용
    /// 매체 복사·백신 격리 해제가 모두 이 속성을 남긴다.
    #[test]
    fn a_read_only_attribute_is_cleared_instead_of_wedging_the_daemon() {
        let dir = fresh_dir("readonly-attr");
        let path = dir.join(DAEMON_FILE);
        write_live_record(&path);
        let mut perms = std::fs::metadata(&path).expect("메타").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).expect("읽기 전용 속성 부여");
        // 전제 확인 — 이 속성이 실제로 쓰기 열기를 막는다(막지 못하면 이 테스트는 아무것도 증명 못 한다).
        assert!(
            super::open_owned(&path).is_err(),
            "읽기 전용 속성이 쓰기 열기를 막아야 이 테스트가 의미 있다"
        );

        let got = acquire(&dir);
        assert!(held(got), "속성은 걷어내고 획득해야(영구 교착 금지)");
        assert!(
            !std::fs::metadata(&path)
                .expect("메타")
                .permissions()
                .readonly(),
            "속성이 실제로 걷혀야"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★잔여를 못 박는다(고치는 테스트가 아니다)★: 제3자가 붙들고 있는데 파일의 옛 레코드 pid 가
    /// 마침 살아 있으면 "이미 실행 중"으로 읽고 조용히 exit 0 한다. 진단 읽기로는 원리적으로 가를 수
    /// 없다 — 동작이 바뀌면 그건 설계가 바뀐 것이므로 여기서 걸린다.
    #[test]
    fn a_live_looking_record_makes_a_third_party_hold_look_like_another_daemon() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = fresh_dir("false-live");
        let path = dir.join(DAEMON_FILE);
        write_live_record(&path); // pid = 우리 자신 = 살아 있음
        const SHARE_READ_ONLY: u32 = 1;
        let onlooker = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ_ONLY)
            .open(&path)
            .expect("제한적 공유 제3자 핸들");

        let got = acquire_with(&dir, 2, Duration::from_millis(1));
        assert!(
            matches!(got, Ok(Acquired::AlreadyRunning { .. })),
            "알려진 오진 — 바뀌었다면 설계가 바뀐 것이다: {:?}",
            got.err().map(|e| e.to_string())
        );

        drop(onlooker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ★R1 회귀 방지★: 가드를 쥔 동안 접속 파일이 지워지거나 이름이 바뀌면, 두 번째 데몬이 같은
    /// 경로에 새 파일을 만들어 잡고 이겨서 **한 폴더에 데몬이 둘** 뜬다.
    #[test]
    fn the_record_file_cannot_be_unlinked_while_held() {
        let dir = fresh_dir("unlink");
        let guard = acquire(&dir).expect("획득");
        assert!(matches!(guard, Acquired::Held(_)));
        let path = dir.join(DAEMON_FILE);

        assert!(
            std::fs::remove_file(&path).is_err(),
            "쥐고 있는 동안 삭제는 거부돼야(삭제되면 두 번째 데몬이 새 파일을 잡는다)"
        );
        assert!(
            std::fs::rename(&path, dir.join("moved.json")).is_err(),
            "이름 변경도 같은 이유로 거부돼야"
        );
        assert!(path.exists(), "파일이 그대로 있어야");

        drop(guard);
        // 놓으면 지울 수 있다 — 잠금이 파일을 영구히 붙드는 것이 아님을 함께 못 박는다.
        assert!(
            std::fs::remove_file(&path).is_ok(),
            "가드를 놓으면 삭제 가능"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
