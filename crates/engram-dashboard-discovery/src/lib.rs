//! 데몬 발견(discovery) — Embedded Tauri 가 데몬을 찾고, 없으면 WMI 로 띄운 뒤 port/token 회수.
//!
//! 두-모드 토글의 Daemon 모드에서 Tauri 가 데몬에 붙기 위한 전제다(실제 WS 클라이언트는 phase4).
//!
//! ## 설계 — 순수 로직과 OS/WMI 경계 분리
//! 단위 테스트가 OS·WMI·실시간에 의존하지 않도록 부수효과를 trait 으로 주입한다:
//!   - [`PidLiveness`] — PID 생존 판정(real = OpenProcess). stale 단위 테스트에서 가짜 주입.
//!   - [`DaemonReader`] — daemon.json 읽기(real = 파일). 폴링 단위 테스트에서 가짜 시퀀스 주입.
//!   - [`Spawner`] — 데몬 spawn(real = WMI Win32_Process.Create). 단위에서 no-op/카운터 주입.
//!   - [`Clock`] — now/sleep(real = Instant/thread::sleep). 폴링 테스트에서 가짜 시계.
//!
//! [`ensure_daemon`] 은 이 trait 들 위에서만 동작하는 **순수 오케스트레이션** 이라 실제
//! WMI spawn·실제 sleep 없이 전 분기를 단위 테스트할 수 있다. 실제 spawn(WMI) 통합은
//! `#[ignore]` 테스트로 남긴다.
//!
//! ## 보안
//! `DaemonInfo.token` 은 로그에 절대 출력하지 않는다(로컬 IPC 파일에만 흐름).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{DaemonInfo, PROTOCOL_VERSION};

const DAEMON_FILE: &str = "daemon.json";
const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ── 에러 ───────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("daemon exe 를 찾을 수 없음: {0}")]
    ExeNotFound(String),
    #[error("daemon.json 파싱 실패: {0}")]
    Parse(String),
    #[error("daemon spawn 실패(WMI ReturnValue={rv})")]
    SpawnFailed { rv: u32 },
    #[error("daemon 시작 대기 timeout({0:?} 초과)")]
    Timeout(Duration),
    #[error("protocol 버전 불일치: 데몬={daemon}, 기대={expected}")]
    VersionMismatch { daemon: u32, expected: u32 },
    #[error("io: {0}")]
    Io(String),
}

// ── 주입 경계(trait) ─────────────────────────────────────────────────────────────

/// PID 생존 판정. real 구현은 core 의 공유 함수(pid_alive_with_start_time).
///
/// start_time 을 함께 받아 PID 재사용(M2)을 구분한다 — "PID 살아있음 AND creation time==기록값"
/// 일 때만 살아있다고 본다. start_time==0(미상, 옛 daemon.json)이면 PID 단독 생존으로 보수 판정.
pub trait PidLiveness {
    /// true=죽음(stale).
    fn is_dead(&self, pid: u32, start_time: u64) -> bool;
}

/// daemon.json 읽기. real 구현은 파일에서 bytes 를 읽어 파싱.
/// 반환: Ok(Some)=유효 파일, Ok(None)=없음(아직 안 써짐), Err=깨진 파일.
pub trait DaemonReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError>;
}

/// 데몬 spawn. real 구현은 WMI Win32_Process.Create(부모 Job 미상속).
pub trait Spawner {
    /// 절대경로 exe 를 spawn. 성공이면 Ok(()), WMI 실패면 SpawnFailed.
    fn spawn(&self, exe: &Path) -> Result<(), DiscoveryError>;
}

/// 시계 — 폴링 루프의 now/sleep 을 주입 가능하게(테스트는 가짜 시계).
pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, dur: Duration);
}

// ── 순수 오케스트레이션 ────────────────────────────────────────────────────────────

/// 발견된 DaemonInfo 가 쓸 만한지(살아있고 버전 호환) 판정한 결과.
/// info 소유권을 호출자가 유지하도록 참조 기반 판정 — Accept 는 데이터 없이 신호만 준다.
enum AcceptCheck {
    Accept,
    DeadPid,
    VersionMismatch { daemon: u32 },
}

/// 읽어온 DaemonInfo 의 수용 가능성 판정(순수). 버전 호환 + PID 생존.
/// 참조로 받아 호출자가 info 를 계속 소유한다(M1 복구용 보관 가능).
fn check_acceptable(info: &DaemonInfo, liveness: &dyn PidLiveness) -> AcceptCheck {
    if info.protocol_version != PROTOCOL_VERSION {
        return AcceptCheck::VersionMismatch {
            daemon: info.protocol_version,
        };
    }
    if liveness.is_dead(info.pid, info.start_time) {
        return AcceptCheck::DeadPid;
    }
    AcceptCheck::Accept
}

/// 데몬 발견의 핵심 흐름(주입식 — OS/WMI/실시간 무의존).
///
/// (a) reader 로 기존 daemon.json 시도 → live + 버전 호환이면 그대로 반환(spawn 안 함).
/// (b) 없거나 stale 이면 spawner 로 데몬 spawn.
/// (c) reader 를 POLL_INTERVAL 간격으로 폴링 — live + 버전 호환 DaemonInfo 가 나오면 반환.
///     timeout 초과면 Timeout.
///
/// ★stale 파일 false-live 회피★: (a) 에서 본 옛 daemon.json 이 깨졌거나 stale 이면 spawn 전에
/// **삭제**(stale_cleanup)를 호출자가 수행한다. 폴링은 "유효 파싱 + live pid + 버전 호환" 만
/// 수락하므로, 새 데몬이 파일을 덮어쓰기 전까지(=삭제돼 None) 옛 pid 를 보지 않는다.
/// 단, 삭제와 새 데몬 write 사이 경합으로 옛 파일이 잠깐 남아도 그 pid 는 이미 dead 라
/// is_dead 가 걸러낸다(이중 안전망).
#[allow(clippy::too_many_arguments)]
fn ensure_with(
    reader: &dyn DaemonReader,
    spawner: &dyn Spawner,
    liveness: &dyn PidLiveness,
    clock: &dyn Clock,
    exe: &Path,
    stale_cleanup: &mut dyn FnMut(),
    timeout: Duration,
) -> Result<DaemonInfo, DiscoveryError> {
    // M1 안전망: stale 판정으로 삭제한 옛 DaemonInfo 를 메모리에 보관한다. 폴링이 timeout 나면
    // (=새 데몬이 안 떴다 = 단일 인스턴스 mutex 충돌로 기존 데몬이 실제 살아있었을 가능성)
    // 이 옛 정보가 지금도 live 인지 재검사해 live 면 복구 반환한다. "살아있는 데몬 파일 삭제→영구교착"
    // 자동 복구. 깨진 파일은 내용을 신뢰할 수 없어 보관하지 않는다(None).
    let mut removed_live_candidate: Option<DaemonInfo> = None;

    // (a) 기존 파일 검사.
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => return Ok(info), // live + 호환 → spawn 불필요
            AcceptCheck::DeadPid => {
                // stale — spawn 전 삭제(폴링이 옛 파일을 새 것으로 오인하지 않게).
                // 삭제 전 옛 info 를 보관(M1 복구용): timeout 시 이 데몬이 사실 살아있었으면 복구한다.
                removed_live_candidate = Some(info);
                stale_cleanup();
            }
            AcceptCheck::VersionMismatch { daemon } => {
                // 버전 불일치 데몬이 살아있다 — 이번 단위는 spawn 으로 덮지 않고 명확히 실패.
                // (재기동 정책은 phase4 DaemonClient 가 결정.)
                return Err(DiscoveryError::VersionMismatch {
                    daemon,
                    expected: PROTOCOL_VERSION,
                });
            }
        },
        Ok(None) => {} // 없음 → spawn 으로 진행
        Err(DiscoveryError::Parse(_)) => {
            // 깨진 파일 → 삭제하고 spawn(새 데몬이 덮어씀). 내용 신뢰 불가라 복구 후보로 보관 안 함.
            stale_cleanup();
        }
        Err(e) => return Err(e),
    }

    // (b) spawn.
    spawner.spawn(exe)?;

    // (c) 폴링 — timeout 까지 새 daemon.json 을 기다린다.
    let deadline = clock.now() + timeout;
    loop {
        match reader.read() {
            Ok(Some(info)) => {
                if let AcceptCheck::Accept = check_acceptable(&info, liveness) {
                    return Ok(info);
                }
                // dead/버전 불일치(옛 파일 잔존 등) → 계속 폴링.
            }
            Ok(None) => {}                      // 아직 안 써짐 → 계속.
            Err(DiscoveryError::Parse(_)) => {} // 쓰는 중 부분 파일일 수 있음 → 계속.
            Err(e) => return Err(e),
        }
        if clock.now() >= deadline {
            // M1 안전망: timeout. 삭제했던 옛 데몬이 지금도 live 면 복구(삭제는 했지만 데몬은
            // 살아있으니 그 port/token 으로 붙게 한다). live 아니면 정직하게 Timeout.
            if let Some(old) = removed_live_candidate.take() {
                if !liveness.is_dead(old.pid, old.start_time)
                    && old.protocol_version == PROTOCOL_VERSION
                {
                    tracing::warn!(
                        pid = old.pid,
                        "daemon.json 을 stale 로 삭제했으나 폴링 timeout — 옛 데몬이 여전히 live: 복구"
                    );
                    return Ok(old);
                }
            }
            return Err(DiscoveryError::Timeout(timeout));
        }
        clock.sleep(POLL_INTERVAL);
    }
}

// ── 데몬 lifecycle 상태/종료(ADR-0021 §5 command 표면) ──────────────────────────────

/// 데몬 alive 판정 결과(daemon_status command 반환). 순수 — daemon.json + liveness 만으로 산출.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    /// 살아있는 데몬이 발견됐는가(파일 존재 + 호환 버전 + PID live).
    pub alive: bool,
    /// 발견된 데몬 PID(파일이 있으면, 죽었어도 보고). 없으면 None.
    pub pid: Option<u32>,
    /// 발견된 데몬 포트. 없으면 None.
    pub port: Option<u16>,
}

/// daemon.json 을 읽어 데몬 alive 여부를 판정한다(순수 — reader/liveness 주입).
///
/// alive=true 조건: 파일 존재 + 버전 호환 + PID live. 파일이 있으나 죽었으면 pid/port 는 보고하되
/// alive=false. 파일이 없으면 전부 None+false. 깨진 파일/IO 오류는 "데몬 없음"으로 본다(보수).
fn status_with(reader: &dyn DaemonReader, liveness: &dyn PidLiveness) -> DaemonStatus {
    match reader.read() {
        Ok(Some(info)) => {
            let alive = matches!(check_acceptable(&info, liveness), AcceptCheck::Accept);
            DaemonStatus {
                alive,
                pid: Some(info.pid),
                port: Some(info.port),
            }
        }
        // 없음/깨짐/IO 오류 → 데몬 없음(보수). 깨진 파일은 신뢰 불가라 pid 미보고.
        _ => DaemonStatus {
            alive: false,
            pid: None,
            port: None,
        },
    }
}

/// 데몬 상태 조회(real 진입점). data_dir/daemon.json 을 읽어 alive/pid/port 를 반환.
pub fn daemon_status(data_dir: &Path) -> DaemonStatus {
    let reader = FileReader {
        path: data_dir.join(DAEMON_FILE),
    };
    status_with(&reader, &RealLiveness)
}

/// 프로세스 종료자 — real 은 taskkill /F. 단위 테스트는 호출 인자를 캡처하는 가짜 주입.
pub trait ProcessKiller {
    /// 주어진 pid 를 강제 종료. 성공 여부는 best-effort(이미 죽었으면 Ok 취급).
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError>;
}

/// 데몬 종료 fallback(real 진입점) — daemon.json 의 pid 를 taskkill /F.
///
/// ★분담★: graceful 종료(StopDaemon AgentCommand)는 **연결을 쥔 프론트**가 보낸다(데몬이 자식
/// PTY 를 정리하고 스스로 내려감). 이 command 는 연결이 없거나 graceful 이 실패했을 때의 **fallback** —
/// daemon.json 의 pid 를 직접 kill 한다. 데몬은 KILL_ON_JOB_CLOSE Job 으로 자식을 담으므로 데몬
/// 프로세스가 죽으면 자식 PTY 도 함께 정리된다(detach 불가, connection_core StopDaemon 주석과 동일).
///
/// 반환: Ok(Some(pid))=kill 시도한 pid, Ok(None)=죽일 데몬 없음(파일 없음/이미 죽음).
pub fn daemon_stop(data_dir: &Path) -> Result<Option<u32>, DiscoveryError> {
    stop_with(
        &FileReader {
            path: data_dir.join(DAEMON_FILE),
        },
        &RealLiveness,
        &TaskKiller,
    )
}

/// 데몬 종료 로직(순수 — reader/liveness/killer 주입). 살아있는 데몬만 kill 한다.
fn stop_with(
    reader: &dyn DaemonReader,
    liveness: &dyn PidLiveness,
    killer: &dyn ProcessKiller,
) -> Result<Option<u32>, DiscoveryError> {
    match reader.read() {
        Ok(Some(info)) => {
            // 이미 죽은 데몬이면 kill 불필요(None). PID 재사용 방어는 liveness 가 start_time 으로 처리.
            if liveness.is_dead(info.pid, info.start_time) {
                return Ok(None);
            }
            killer.kill(info.pid)?;
            Ok(Some(info.pid))
        }
        // 파일 없음/깨짐 → 죽일 데몬 없음.
        _ => Ok(None),
    }
}

/// taskkill /F 로 pid 를 종료하는 real ProcessKiller(Windows). non-windows 는 SIGKILL(미지원 stub).
struct TaskKiller;

impl ProcessKiller for TaskKiller {
    #[cfg(windows)]
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError> {
        // taskkill /PID <pid> /F /T — /T 로 자식 트리도 정리(데몬 Job 안전망과 중복이나 무해).
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| DiscoveryError::Io(format!("taskkill 실행 실패: {e}")))?;
        // taskkill 은 "이미 종료됨"(exit 128)도 있으므로 성공/실패를 강제하지 않는다(best-effort).
        let _ = status;
        Ok(())
    }

    #[cfg(not(windows))]
    fn kill(&self, _pid: u32) -> Result<(), DiscoveryError> {
        Err(DiscoveryError::Io("daemon_stop 은 Windows 전용".into()))
    }
}

// ── 공개 진입점 ─────────────────────────────────────────────────────────────────

/// 데몬을 발견(없으면 spawn)하고 DaemonInfo 를 반환한다.
///
/// `data_dir` = daemon.json 디렉토리(Embedded 와 동일 app_data_dir).
/// `daemon_exe` = 데몬 실행 파일 경로(절대화는 내부에서 dunce::canonicalize).
pub fn ensure_daemon(
    data_dir: &Path,
    daemon_exe: &Path,
    timeout: Duration,
    console: bool,
) -> Result<DaemonInfo, DiscoveryError> {
    let daemon_path = data_dir.join(DAEMON_FILE);

    // 절대경로 필수: WMI Win32_Process.Create 는 상대경로면 RV=9(Path not found) — spike 함정.
    let exe_abs = dunce::canonicalize(daemon_exe)
        .map_err(|e| DiscoveryError::ExeNotFound(format!("{}: {e}", daemon_exe.display())))?;

    let reader = FileReader {
        path: daemon_path.clone(),
    };
    // console=false(기본): windowless spawn(CREATE_NO_WINDOW). console=true: 콘솔 창과 함께(디버그).
    let spawner = WmiSpawner { console };
    let liveness = RealLiveness;
    let clock = RealClock;
    let mut stale_cleanup = || {
        // 삭제 실패는 무시 — 새 데몬이 어차피 atomic rename 으로 덮어쓴다.
        let _ = std::fs::remove_file(&daemon_path);
    };

    ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        &exe_abs,
        &mut stale_cleanup,
        timeout,
    )
}

/// 데몬 exe 경로 탐색. 우선 current_exe 와 같은 디렉토리(배포 시 동거),
/// 없으면 개발용 target/debug fallback. 못 찾으면 ExeNotFound.
pub fn locate_daemon_exe() -> Result<PathBuf, DiscoveryError> {
    const EXE: &str = if cfg!(windows) {
        "engram-dashboard-daemon.exe"
    } else {
        "engram-dashboard-daemon"
    };

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            candidates.push(dir.join(EXE)); // 배포: tauri exe 옆에 동거.
                                            // 개발: target/debug/<app>.exe → 같은 디렉토리에 데몬도 빌드됨(보통 위와 동일).
                                            // 워크스페이스 빌드면 target/debug 가 공유라 위 후보로 충분하나, 안전하게 한 번 더.
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join(EXE));
        candidates.push(cwd.join("..").join("target").join("debug").join(EXE));
    }

    locate_in(&candidates)
}

/// 후보 경로 중 존재하는 첫 파일을 반환(순수 분리 — 단위 테스트 가능).
/// 후보가 모두 없으면 ExeNotFound.
fn locate_in(candidates: &[PathBuf]) -> Result<PathBuf, DiscoveryError> {
    for c in candidates {
        if c.is_file() {
            return Ok(c.clone());
        }
    }
    Err(DiscoveryError::ExeNotFound(format!(
        "daemon exe 후보 {}개 모두 없음",
        candidates.len()
    )))
}

// ── real 구현 ──────────────────────────────────────────────────────────────────

struct FileReader {
    path: PathBuf,
}

impl DaemonReader for FileReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(DiscoveryError::Io(e.to_string())),
        };
        DaemonInfo::parse(&bytes)
            .map(Some)
            .map_err(|e| DiscoveryError::Parse(e.to_string()))
    }
}

struct RealLiveness;

impl PidLiveness for RealLiveness {
    fn is_dead(&self, pid: u32, start_time: u64) -> bool {
        // ★DRY★: liveness 판정은 core 의 공유 함수에 위임한다(daemon portfile::is_stale 과 동일 로직).
        // 옛 src-tauri 사본 pid_is_dead 는 제거 — 무테스트 중복이었다(리뷰어 지적).
        !engram_dashboard_core::agent::platform::pid_alive_with_start_time(pid, start_time)
    }
}

struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

// ── COM 초기화 RAII 가드(C1) ─────────────────────────────────────────────────────
//
// ★왜 가드인가★: wmi_spawn 은 `?` 조기반환이 많다. CoInitializeEx 성공 시 모든 탈출 경로에서
// CoUninitialize 를 정확히 1회 호출해야 COM 초기화/해제 짝이 맞는다. 수동으로 각 return 앞에
// 넣으면 누락 위험 — RAII(Drop)로 원천 차단한다.

/// CoInitializeEx 의 HRESULT 를 받아 "우리가 CoUninitialize 해야 하는가"를 결정한 결과.
/// 순수 함수로 분리해 실제 COM 없이 단위 테스트한다.
#[derive(Debug, PartialEq, Eq)]
enum ComInit {
    /// 우리가 초기화에 성공(S_OK/S_FALSE) → Uninitialize 책임 있음.
    Initialized,
    /// 이미 다른 apartment(STA)로 초기화돼 있음(RPC_E_CHANGED_MODE) → 우리가 init 안 함.
    /// WMI 호출은 기존 apartment 로 진행하되 Uninitialize 는 하지 않는다.
    AlreadyOtherMode,
    /// 그 외 HRESULT 실패 → 진행 불가.
    Failed(i32),
}

/// CoInitializeEx 결과 HRESULT → ComInit 매핑(순수).
///
/// S_OK(0)·S_FALSE(1, 이미 초기화됐지만 우리 호출도 성공으로 카운트 — Uninitialize 책임 있음)
/// → Initialized. RPC_E_CHANGED_MODE(0x80010106) → AlreadyOtherMode(uninit 금지).
/// 그 외 음수 HRESULT → Failed.
fn classify_com_init(hr: i32) -> ComInit {
    const S_OK: i32 = 0;
    const S_FALSE: i32 = 1;
    const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
    match hr {
        S_OK | S_FALSE => ComInit::Initialized,
        RPC_E_CHANGED_MODE => ComInit::AlreadyOtherMode,
        other => ComInit::Failed(other),
    }
}

/// COM 초기화 RAII 가드. needs_uninit=true 일 때만 Drop 에서 CoUninitialize 한다.
#[cfg(windows)]
struct ComGuard {
    needs_uninit: bool,
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.needs_uninit {
            use windows::Win32::System::Com::CoUninitialize;
            // SAFETY: 우리가 CoInitializeEx 로 성공 초기화한 스레드에서 정확히 1회 해제한다.
            // AlreadyOtherMode 경로는 needs_uninit=false 라 여기 진입하지 않는다.
            unsafe { CoUninitialize() };
        }
    }
}

// ── WMI spawn(real) ─────────────────────────────────────────────────────────────

/// WMI Win32_Process.Create 스포너. `console` 으로 데몬 콘솔 창 가시성을 정한다.
struct WmiSpawner {
    /// true=콘솔 창과 함께(CREATE_NEW_CONSOLE, 디버그 로그 가시화), false=windowless(CREATE_NO_WINDOW, 기본).
    console: bool,
}

impl Spawner for WmiSpawner {
    fn spawn(&self, exe: &Path) -> Result<(), DiscoveryError> {
        wmi_spawn(exe, self.console)
    }
}

/// WMI Win32_Process.Create 로 exe 를 spawn.
///
/// ★왜 WMI★: WMI 로 띄운 프로세스는 호출자가 아니라 WmiPrvSE 가 부모가 되어 **부모 Job 을
/// 상속하지 않는다**(spike #1 검증). 그래서 Tauri 가 KILL_ON_JOB_CLOSE Job 안에 있어도
/// 데몬이 살아남는다. 또한 WMI Create 는 **환경변수 주입 불가** — 토큰은 daemon.json(ACL)으로만
/// 흐른다(설계 확정). 그래서 여기선 CommandLine 만 넘긴다.
///
/// ★절대경로 필수★: 상대경로면 RV=9(Path not found). 호출자가 dunce::canonicalize 로 절대화한
/// exe 를 받는다.
#[cfg(windows)]
fn wmi_spawn(exe: &Path, console: bool) -> Result<(), DiscoveryError> {
    // ADR-0021 §C(개정): CreateFlags 로 콘솔 창 가시성 제어(Win32_ProcessStartup.CreateFlags).
    //
    // ★실측 확정(2026-06-17, real_wmi_spawn_flag_matrix)★: WMI Win32_Process.Create 는
    //   CREATE_NO_WINDOW(0x08000000) 을 받으면 ReturnValue=21(Invalid Parameter) 로 거부한다
    //   (알려진 WMI quirk — CREATE_NO_WINDOW 는 CreateProcess 직접 호출용이며 WMI Create 의
    //   허용 플래그 집합 밖이다). 그래서 windowless 기본은 **CreateFlags 를 아예 안 넘긴다**:
    //     - windowless(console=false) → ProcessStartupInformation 자체 생략(create_flags=None).
    //       WMI-spawn 프로세스는 WmiPrvSE 자식이라 **비대화형 컨텍스트**에서 떠 콘솔 창이
    //       애초에 나타나지 않는다. 플래그 불필요 → RV=0.
    //     - console=true → CREATE_NEW_CONSOLE(0x10): 허용 플래그라 RV=0, 별도 콘솔 창과 함께
    //       뜬다(디버그 로그 가시화).
    const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
    let create_flags: Option<i32> = if console {
        Some(CREATE_NEW_CONSOLE)
    } else {
        None
    };

    // RV!=0 → SpawnFailed(rv) 로 변환. RV=0 → Ok.
    let rv = wmi_create_raw(exe, create_flags)?;
    if rv != 0 {
        return Err(DiscoveryError::SpawnFailed { rv });
    }
    Ok(())
}

/// WMI Win32_Process.Create 를 실제 호출하고 **raw ReturnValue(u32)** 를 그대로 돌려준다.
/// (RV!=0 을 에러로 승격하지 않음 — flag-matrix 실측 테스트가 RV 자체를 비교하기 위함.)
///
/// `create_flags`:
///   - `None`         → ProcessStartupInformation 자체를 안 넘김(windowless 기본).
///   - `Some(flags)`  → Win32_ProcessStartup{ CreateFlags=flags } 임베디드 오브젝트로 전달.
#[cfg(windows)]
fn wmi_create_raw(exe: &Path, create_flags: Option<i32>) -> Result<u32, DiscoveryError> {
    // Interface trait — startup_inst.cast::<IUnknown>() 에 필요(임베디드 오브젝트를 VARIANT 로 박기).
    use windows::core::{Interface, BSTR, VARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
    use windows::Win32::System::Wmi::{
        IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_CONNECT_USE_MAX_WAIT,
    };

    // CommandLine: "<절대exe>" (인자 없음 — 데몬은 인자 불필요).
    let exe_str = exe.to_string_lossy();
    let command_line = format!("\"{exe_str}\"");

    // SAFETY 블록: COM/WMI 호출 시퀀스. spike #1 의 PowerShell Invoke-CimMethod 와 동일한
    // Win32_Process.Create 를 COM 직접 호출로 수행한다. 각 단계 실패는 HRESULT→DiscoveryError.
    unsafe {
        // 1) COM 초기화(MTA). C1: HRESULT 를 받아 짝맞춤한다.
        //    - S_OK/S_FALSE → 우리가 초기화 성공 → ComGuard 가 함수 탈출 시 CoUninitialize 1회.
        //    - RPC_E_CHANGED_MODE → 이 스레드가 이미 STA 로 초기화됨(우리가 init 안 함) → uninit 금지.
        //      기존 apartment 로 WMI 호출은 그대로 진행한다.
        //    - 그 외 실패 → 즉시 DiscoveryError.
        // SAFETY: CoInitializeEx 는 스레드 단위 COM 초기화. 반환 HRESULT 로 짝맞춤(아래 가드).
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _com_guard = match classify_com_init(hr.0) {
            ComInit::Initialized => ComGuard { needs_uninit: true },
            ComInit::AlreadyOtherMode => ComGuard {
                needs_uninit: false, // 우리가 init 안 했으니 uninit 도 안 함.
            },
            ComInit::Failed(code) => {
                return Err(DiscoveryError::Io(format!(
                    "CoInitializeEx 실패 HRESULT {:#010x}",
                    code as u32
                )));
            }
        };

        // 2) WbemLocator 생성 → root\cimv2 connect.
        let locator: IWbemLocator =
            CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(wmi_err)?;
        let services: IWbemServices = locator
            .ConnectServer(
                &BSTR::from("ROOT\\CIMV2"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
                &BSTR::new(),
                None,
            )
            .map_err(wmi_err)?;

        // 3) 보안 blanket — 로컬 WMI 호출에 필요한 impersonation 레벨.
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
        .map_err(wmi_err)?;

        // 4) Win32_Process 클래스 → Create 메서드 in-params 인스턴스 준비.
        let class_name = BSTR::from("Win32_Process");
        let mut class_obj: Option<IWbemClassObject> = None;
        services
            .GetObject(
                &class_name,
                Default::default(),
                None,
                Some(&mut class_obj),
                None,
            )
            .map_err(wmi_err)?;
        let class_obj = class_obj.ok_or(DiscoveryError::SpawnFailed { rv: u32::MAX })?;

        let method_name = BSTR::from("Create");
        let mut in_sig: Option<IWbemClassObject> = None;
        class_obj
            .GetMethod(&method_name, 0, &mut in_sig, std::ptr::null_mut())
            .map_err(wmi_err)?;
        let in_sig = in_sig.ok_or(DiscoveryError::SpawnFailed { rv: u32::MAX })?;
        let in_inst = in_sig.SpawnInstance(0).map_err(wmi_err)?;

        // 5) CommandLine 파라미터 설정(VARIANT BSTR).
        let cl_value = VARIANT::from(BSTR::from(command_line.as_str()));
        in_inst
            .Put(&BSTR::from("CommandLine"), 0, &cl_value, 0)
            .map_err(wmi_err)?;

        // 5b) ProcessStartupInformation = Win32_ProcessStartup{ CreateFlags } — **플래그가 있을 때만**.
        //     console=true(CREATE_NEW_CONSOLE)일 때만 임베디드 오브젝트를 만들어 박는다.
        //     windowless(create_flags=None)는 이 블록 전체를 건너뛴다 — WMI 는 비대화형 spawn 이라
        //     플래그 없이도 콘솔 창이 안 뜨고, CREATE_NO_WINDOW 를 넣으면 RV=21 로 거부되기 때문.
        if let Some(create_flags) = create_flags {
            let startup_class_name = BSTR::from("Win32_ProcessStartup");
            let mut startup_class: Option<IWbemClassObject> = None;
            services
                .GetObject(
                    &startup_class_name,
                    Default::default(),
                    None,
                    Some(&mut startup_class),
                    None,
                )
                .map_err(wmi_err)?;
            let startup_class =
                startup_class.ok_or(DiscoveryError::SpawnFailed { rv: u32::MAX })?;
            let startup_inst = startup_class.SpawnInstance(0).map_err(wmi_err)?;
            // CreateFlags 는 VT_I4(부호 있는 32-bit). windows-core VARIANT::from(i32) 로 VT_I4 생성.
            let flags_value = VARIANT::from(create_flags);
            startup_inst
                .Put(&BSTR::from("CreateFlags"), 0, &flags_value, 0)
                .map_err(wmi_err)?;
            // 임베디드 오브젝트를 IUnknown VARIANT 로 ProcessStartupInformation 에 Put.
            let startup_unknown: windows::core::IUnknown = startup_inst.cast().map_err(wmi_err)?;
            let startup_value = VARIANT::from(startup_unknown);
            in_inst
                .Put(
                    &BSTR::from("ProcessStartupInformation"),
                    0,
                    &startup_value,
                    0,
                )
                .map_err(wmi_err)?;
        }

        // 6) ExecMethod 호출.
        let mut out: Option<IWbemClassObject> = None;
        services
            .ExecMethod(
                &class_name,
                &method_name,
                Default::default(),
                None,
                &in_inst,
                Some(&mut out),
                None,
            )
            .map_err(wmi_err)?;

        // 7) ReturnValue 회수(0=성공, 21=Invalid Parameter 등). 토큰/pid 는 daemon.json 폴링으로
        //    회수하므로 여기선 RV 만 본다. 승격은 호출자(wmi_spawn)가 한다.
        let rv = match out {
            Some(out) => read_u32_prop(&out, "ReturnValue").unwrap_or(u32::MAX),
            None => u32::MAX,
        };
        Ok(rv)
    }
}

/// WMI out-params 에서 u32 속성 읽기(ReturnValue/ProcessId). 실패 시 None.
#[cfg(windows)]
unsafe fn read_u32_prop(
    obj: &windows::Win32::System::Wmi::IWbemClassObject,
    name: &str,
) -> Option<u32> {
    use windows::core::{BSTR, VARIANT};
    let mut value = VARIANT::default();
    obj.Get(&BSTR::from(name), 0, &mut value, None, None).ok()?;
    // ReturnValue 는 VT_I4 — windows-core 의 TryFrom<&VARIANT> for u32 가 변환 처리.
    u32::try_from(&value).ok()
}

#[cfg(windows)]
fn wmi_err(e: windows::core::Error) -> DiscoveryError {
    // HRESULT 만 노출(토큰 등 민감정보 없음).
    DiscoveryError::Io(format!("WMI HRESULT {:#010x}", e.code().0 as u32))
}

#[cfg(not(windows))]
fn wmi_spawn(_exe: &Path, _console: bool) -> Result<(), DiscoveryError> {
    Err(DiscoveryError::Io("WMI spawn 은 Windows 전용".into()))
}

// ── 테스트 ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn info(pid: u32, version: u32) -> DaemonInfo {
        DaemonInfo {
            pid,
            host: "127.0.0.1".into(),
            port: 12345,
            token: "t".repeat(64),
            protocol_version: version,
            start_time: 0,
        }
    }

    // 가짜 PID 생존 판정 — 죽었다고 볼 pid 집합을 주입(start_time 은 무시).
    struct FakeLiveness {
        dead: Vec<u32>,
    }
    impl PidLiveness for FakeLiveness {
        fn is_dead(&self, pid: u32, _start_time: u64) -> bool {
            self.dead.contains(&pid)
        }
    }

    // 가짜 reader — read 호출마다 미리 넣은 시퀀스를 차례로 반환.
    struct FakeReader {
        seq: RefCell<std::collections::VecDeque<Result<Option<DaemonInfo>, DiscoveryError>>>,
        calls: Cell<usize>,
    }
    impl FakeReader {
        fn new(seq: Vec<Result<Option<DaemonInfo>, DiscoveryError>>) -> Self {
            Self {
                seq: RefCell::new(seq.into()),
                calls: Cell::new(0),
            }
        }
    }
    impl DaemonReader for FakeReader {
        fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError> {
            self.calls.set(self.calls.get() + 1);
            // 시퀀스 소진 후엔 마지막 동작을 반복(None) — timeout 경로 모사.
            self.seq.borrow_mut().pop_front().unwrap_or(Ok(None))
        }
    }

    // spawn 호출 횟수만 세는 가짜 — 항상 성공(spawn 자체 실패 경로는 SpawnFailed enum 으로 별도 검증).
    struct CountingSpawner {
        count: Cell<usize>,
    }
    impl CountingSpawner {
        fn ok() -> Self {
            Self {
                count: Cell::new(0),
            }
        }
    }
    impl Spawner for CountingSpawner {
        fn spawn(&self, _exe: &Path) -> Result<(), DiscoveryError> {
            self.count.set(self.count.get() + 1);
            Ok(())
        }
    }

    // 가짜 시계 — now 가 sleep 마다 전진(실시간 대기 없음).
    struct FakeClock {
        now: RefCell<Instant>,
        slept: Cell<usize>,
    }
    impl FakeClock {
        fn new() -> Self {
            Self {
                now: RefCell::new(Instant::now()),
                slept: Cell::new(0),
            }
        }
    }
    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.now.borrow()
        }
        fn sleep(&self, dur: Duration) {
            // 실제로 자지 않고 가짜 시계만 전진 — 폴링 timeout 을 즉시 도달시킨다.
            self.slept.set(self.slept.get() + 1);
            *self.now.borrow_mut() += dur;
        }
    }

    fn noop_cleanup() -> impl FnMut() {
        || {}
    }

    #[test]
    fn live_existing_file_returns_without_spawn() {
        // (a) 기존 daemon.json 이 live + 버전 호환 → spawn 안 하고 즉시 반환.
        let reader = FakeReader::new(vec![Ok(Some(info(100, PROTOCOL_VERSION)))]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_secs(5),
        )
        .expect("live 파일이면 성공");
        assert_eq!(got.pid, 100);
        assert_eq!(spawner.count.get(), 0, "live 면 spawn 금지");
    }

    #[test]
    fn stale_file_triggers_cleanup_and_spawn_then_polls_new() {
        // (a) stale(죽은 pid) → cleanup + spawn. (c) 폴링: None→None→새 live 파일.
        let reader = FakeReader::new(vec![
            Ok(Some(info(7, PROTOCOL_VERSION))), // (a) 옛 파일, pid 7 = dead
            Ok(None),                            // (c) 아직 안 써짐
            Ok(None),
            Ok(Some(info(200, PROTOCOL_VERSION))), // (c) 새 데몬 live
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![7] };
        let clock = FakeClock::new();
        let cleanup_calls = Cell::new(0);
        let mut cleanup = || cleanup_calls.set(cleanup_calls.get() + 1);

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_secs(5),
        )
        .expect("새 데몬 발견 성공");
        assert_eq!(got.pid, 200);
        assert_eq!(spawner.count.get(), 1, "stale 면 spawn 1회");
        assert_eq!(cleanup_calls.get(), 1, "stale 파일 1회 삭제");
    }

    #[test]
    fn missing_file_spawns_and_polls() {
        // (a) 없음 → spawn. (c) 곧 새 파일.
        let reader = FakeReader::new(vec![
            Ok(None),                              // (a) 없음
            Ok(Some(info(300, PROTOCOL_VERSION))), // (c)
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(got.pid, 300);
        assert_eq!(spawner.count.get(), 1);
    }

    #[test]
    fn timeout_when_daemon_never_writes() {
        // spawn 했지만 daemon.json 이 끝까지 안 나타나면 Timeout.
        let reader = FakeReader::new(vec![Ok(None)]); // 이후 계속 None(소진 후 기본 None)
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_millis(200), // 50ms 간격 → 몇 번 폴링 후 timeout
        )
        .unwrap_err();
        assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
    }

    #[test]
    fn version_mismatch_live_daemon_errors_without_spawn() {
        // 살아있는데 버전이 다른 데몬 → spawn 안 하고 VersionMismatch.
        let reader = FakeReader::new(vec![Ok(Some(info(400, PROTOCOL_VERSION + 1)))]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(
            matches!(err, DiscoveryError::VersionMismatch { .. }),
            "{err:?}"
        );
        assert_eq!(spawner.count.get(), 0);
    }

    #[test]
    fn corrupt_existing_file_cleans_and_spawns() {
        // (a) 깨진 파일 → cleanup + spawn → 폴링.
        let reader = FakeReader::new(vec![
            Err(DiscoveryError::Parse("bad".into())),
            Ok(Some(info(500, PROTOCOL_VERSION))),
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let cleanup_calls = Cell::new(0);
        let mut cleanup = || cleanup_calls.set(cleanup_calls.get() + 1);

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(got.pid, 500);
        assert_eq!(cleanup_calls.get(), 1);
        assert_eq!(spawner.count.get(), 1);
    }

    #[test]
    fn spawn_failure_propagates() {
        // spawner 가 SpawnFailed 면 ensure_with 도 그대로 전파(폴링 진입 안 함).
        struct FailingSpawner;
        impl Spawner for FailingSpawner {
            fn spawn(&self, _exe: &Path) -> Result<(), DiscoveryError> {
                Err(DiscoveryError::SpawnFailed { rv: 9 })
            }
        }
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let err = ensure_with(
            &reader,
            &FailingSpawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(
            matches!(err, DiscoveryError::SpawnFailed { rv: 9 }),
            "{err:?}"
        );
    }

    // ── 폴링 분기(리뷰어 지적): 깨진 json 연속·버전 불일치 연속 ─────────────────────

    #[test]
    fn polling_keeps_going_on_repeated_corrupt_then_timeout() {
        // (a) 없음 → spawn. (c) 폴링이 깨진 json 만 반복 → 새 파일 없이 timeout.
        // 부분 파일(쓰는 중)을 계속 무시하고 폴링을 이어가는지 검증.
        let reader = FakeReader::new(vec![
            Ok(None),                                     // (a) 없음
            Err(DiscoveryError::Parse("partial".into())), // (c) 쓰는 중
            Err(DiscoveryError::Parse("partial".into())),
            Err(DiscoveryError::Parse("partial".into())),
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
        assert_eq!(spawner.count.get(), 1);
    }

    #[test]
    fn polling_keeps_going_on_repeated_version_mismatch_then_timeout() {
        // (a) 없음 → spawn. (c) 폴링이 버전 불일치 파일만 반복(옛 파일 잔존) → 수락 안 하고 timeout.
        let reader = FakeReader::new(vec![
            Ok(None),                                  // (a)
            Ok(Some(info(900, PROTOCOL_VERSION + 1))), // (c) 버전 불일치
            Ok(Some(info(901, PROTOCOL_VERSION + 1))),
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_millis(200),
        )
        .unwrap_err();
        // 폴링 단계의 버전 불일치는 (a) 와 달리 즉시 에러로 끝내지 않고 계속 폴링 → 최종 Timeout.
        assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
    }

    // ── M1 복구 안전망: stale 삭제했으나 옛 데몬이 사실 live → 복구 ───────────────────

    // start_time 까지 보는 가짜 liveness — (pid, start_time) 쌍이 live 집합에 있으면 살아있음.
    struct StartTimeLiveness {
        live: Vec<(u32, u64)>,
    }
    impl PidLiveness for StartTimeLiveness {
        fn is_dead(&self, pid: u32, start_time: u64) -> bool {
            !self.live.contains(&(pid, start_time))
        }
    }

    fn info_with_start(pid: u32, version: u32, start: u64) -> DaemonInfo {
        let mut i = info(pid, version);
        i.start_time = start;
        i
    }

    #[test]
    fn timeout_recovers_old_daemon_if_still_live() {
        // 시나리오: (a) 옛 파일이 stale 로 판정돼 삭제됐는데, 폴링이 끝까지 새 파일을 못 봐 timeout.
        // 그러나 옛 pid+start_time 이 (timeout 재검사 시점에) 사실 live 면 그 정보를 복구 반환한다.
        //
        // 이를 모사하려면 같은 (pid,start) 가 (a) 에서는 dead, timeout 재검사에서는 live 여야 한다 —
        // is_dead 호출 시점에 따라 답이 바뀌는 가짜를 쓴다(첫 호출=dead, 이후=live).
        struct FlipLiveness {
            calls: Cell<usize>,
        }
        impl PidLiveness for FlipLiveness {
            fn is_dead(&self, _pid: u32, _start: u64) -> bool {
                let n = self.calls.get();
                self.calls.set(n + 1);
                n == 0 // 첫 판정만 dead(삭제 유발), 그 뒤(복구 재검사)는 live
            }
        }
        let reader = FakeReader::new(vec![
            Ok(Some(info_with_start(42, PROTOCOL_VERSION, 777))), // (a) 처음엔 dead 판정 → 삭제+보관
            Ok(None),                                             // (c) 새 파일 안 나옴
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = FlipLiveness {
            calls: Cell::new(0),
        };
        let clock = FakeClock::new();
        let cleanup_calls = Cell::new(0);
        let mut cleanup = || cleanup_calls.set(cleanup_calls.get() + 1);

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_millis(150),
        )
        .expect("옛 데몬이 사실 live 면 복구");
        assert_eq!(got.pid, 42, "삭제했던 옛 데몬 정보를 복구");
        assert_eq!(cleanup_calls.get(), 1, "stale 로 봐 한 번 삭제는 했음");
    }

    #[test]
    fn timeout_does_not_recover_if_old_daemon_really_dead() {
        // 옛 데몬이 끝까지 dead 면 복구하지 않고 정직하게 Timeout.
        let reader = FakeReader::new(vec![
            Ok(Some(info_with_start(43, PROTOCOL_VERSION, 888))), // dead → 삭제+보관
            Ok(None),
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = StartTimeLiveness { live: vec![] }; // 아무것도 live 아님
        let clock = FakeClock::new();
        let mut cleanup = noop_cleanup();

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut cleanup,
            Duration::from_millis(150),
        )
        .unwrap_err();
        assert!(matches!(err, DiscoveryError::Timeout(_)), "{err:?}");
    }

    // ── C1: classify_com_init 매핑(실제 CoInitialize 없이 순수 검증) ────────────────

    #[test]
    fn classify_com_init_maps_hresults() {
        const S_OK: i32 = 0;
        const S_FALSE: i32 = 1;
        const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;
        // S_OK/S_FALSE = 우리가 초기화 성공 → Initialized(uninit 책임 있음).
        assert_eq!(classify_com_init(S_OK), ComInit::Initialized);
        assert_eq!(classify_com_init(S_FALSE), ComInit::Initialized);
        // 이미 STA → AlreadyOtherMode(uninit 금지).
        assert_eq!(
            classify_com_init(RPC_E_CHANGED_MODE),
            ComInit::AlreadyOtherMode
        );
        // 그 외 실패 HRESULT → Failed.
        let e_fail = 0x8000_4005u32 as i32; // E_FAIL 류 임의 실패
        assert_eq!(classify_com_init(e_fail), ComInit::Failed(e_fail));
    }

    #[test]
    fn com_init_needs_uninit_only_when_we_initialized() {
        // ComGuard.needs_uninit 결정 로직: Initialized 만 true, AlreadyOtherMode 는 false.
        let needs = |hr: i32| match classify_com_init(hr) {
            ComInit::Initialized => true,
            ComInit::AlreadyOtherMode => false,
            ComInit::Failed(_) => false, // 실패면 가드 자체를 안 만듦
        };
        assert!(needs(0), "S_OK → uninit");
        assert!(needs(1), "S_FALSE → uninit");
        assert!(!needs(0x8001_0106u32 as i32), "CHANGED_MODE → no uninit");
    }

    // ── ADR-0021: daemon_status / daemon_stop (attach-only, spawn 0) ───────────────────

    // 호출 인자를 캡처하는 가짜 killer — kill 한 pid 목록 보관.
    struct CountingKiller {
        killed: RefCell<Vec<u32>>,
    }
    impl CountingKiller {
        fn new() -> Self {
            Self {
                killed: RefCell::new(Vec::new()),
            }
        }
    }
    impl ProcessKiller for CountingKiller {
        fn kill(&self, pid: u32) -> Result<(), DiscoveryError> {
            self.killed.borrow_mut().push(pid);
            Ok(())
        }
    }

    #[test]
    fn status_live_file_reports_alive_with_pid_port() {
        // 살아있는 데몬 파일 → alive=true + pid/port 보고.
        let reader = FakeReader::new(vec![Ok(Some(info(111, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let s = status_with(&reader, &liveness);
        assert!(s.alive);
        assert_eq!(s.pid, Some(111));
        assert_eq!(s.port, Some(12345));
    }

    #[test]
    fn status_dead_file_reports_not_alive_but_keeps_pid() {
        // 죽은 데몬 파일 → alive=false 지만 pid/port 는 보고(진단용).
        let reader = FakeReader::new(vec![Ok(Some(info(222, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![222] };
        let s = status_with(&reader, &liveness);
        assert!(!s.alive);
        assert_eq!(s.pid, Some(222));
    }

    #[test]
    fn status_version_mismatch_is_not_alive() {
        // 버전 불일치 데몬(붙을 수 없음) → alive=false.
        let reader = FakeReader::new(vec![Ok(Some(info(333, PROTOCOL_VERSION + 1)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let s = status_with(&reader, &liveness);
        assert!(!s.alive, "버전 불일치는 붙을 수 없으므로 alive=false");
        assert_eq!(s.pid, Some(333));
    }

    #[test]
    fn status_missing_file_is_not_alive_no_pid() {
        // 파일 없음 → alive=false, pid/port None.
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let s = status_with(&reader, &liveness);
        assert!(!s.alive);
        assert_eq!(s.pid, None);
        assert_eq!(s.port, None);
    }

    #[test]
    fn stop_live_daemon_kills_pid() {
        // 살아있는 데몬 → 그 pid 를 kill.
        let reader = FakeReader::new(vec![Ok(Some(info(444, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let killer = CountingKiller::new();
        let got = stop_with(&reader, &liveness, &killer).unwrap();
        assert_eq!(got, Some(444));
        assert_eq!(killer.killed.borrow().as_slice(), &[444]);
    }

    #[test]
    fn stop_dead_daemon_does_not_kill() {
        // 이미 죽은 데몬 → kill 안 함(None). PID 재사용 방어.
        let reader = FakeReader::new(vec![Ok(Some(info(555, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![555] };
        let killer = CountingKiller::new();
        let got = stop_with(&reader, &liveness, &killer).unwrap();
        assert_eq!(got, None);
        assert!(killer.killed.borrow().is_empty(), "죽은 데몬은 kill 금지");
    }

    #[test]
    fn stop_missing_file_is_noop() {
        // 파일 없음 → 죽일 데몬 없음(None), kill 0회.
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let killer = CountingKiller::new();
        let got = stop_with(&reader, &liveness, &killer).unwrap();
        assert_eq!(got, None);
        assert!(killer.killed.borrow().is_empty());
    }

    // ── locate_daemon_exe (tempfile 주입 가능 분기) ─────────────────────────────────

    #[test]
    fn locate_daemon_exe_no_candidates_returns_exe_not_found() {
        // 존재하지 않을 디렉토리만 후보로 줄 때 ExeNotFound.
        let bogus = std::env::temp_dir().join("engram-no-such-daemon-dir-xyz");
        let _ = std::fs::remove_dir_all(&bogus);
        let candidates = vec![bogus.join("engram-dashboard-daemon.exe")];
        let err = locate_in(&candidates).unwrap_err();
        assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
    }

    #[test]
    fn locate_daemon_exe_picks_first_existing() {
        // 첫 후보가 실제 파일이면 그걸 우선 반환(current_exe 우선 분기 모사).
        let dir = std::env::temp_dir().join("engram-locate-test");
        let _ = std::fs::create_dir_all(&dir);
        let first = dir.join("first-daemon.exe");
        std::fs::write(&first, b"x").unwrap();
        let second = dir.join("second-daemon.exe");
        std::fs::write(&second, b"x").unwrap();
        let got = locate_in(&[first.clone(), second]).unwrap();
        assert_eq!(got, first, "첫 존재 후보 우선");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── ensure_daemon (real 진입점) canonicalize 실패 → ExeNotFound ──────────────────

    #[test]
    fn ensure_daemon_missing_exe_is_exe_not_found() {
        // 존재하지 않는 exe 경로 → dunce::canonicalize 실패 → ExeNotFound.
        let data_dir = std::env::temp_dir();
        let missing = std::env::temp_dir().join("engram-definitely-missing-daemon.exe");
        let _ = std::fs::remove_file(&missing);
        let err = ensure_daemon(&data_dir, &missing, Duration::from_millis(50), false).unwrap_err();
        assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
    }

    // ── 실제 WMI spawn smoke(실프로세스) — `-- --ignored` 로 실행(Windows 전용) ───────────
    //
    // 검증: locate_daemon_exe 로 빌드된 데몬 .exe 를 찾아 WMI Win32_Process.Create 로 실제 spawn →
    //   RV==0(성공) → 데몬이 daemon.json 을 발행하는지 폴링으로 회수 → 그 데몬을 정리(kill).
    //
    // ★운영 daemon.json 오염 방지★: WMI Create 는 자식에 env 주입이 불가(설계 확정)하므로 spawn 된
    //   데몬은 운영 기본 data_dir(%APPDATA%\com.engram.dashboard)를 본다. ENGRAM_DATA_DIR 격리가
    //   WMI 경로엔 닿지 않는다(한계). 그래서 테스트는 (1) 기존 운영 daemon.json 을 백업하고,
    //   (2) ★기존 데몬이 살아있으면 단일-인스턴스 mutex 로 우리 spawn 이 거부돼 검증이 무의미하므로
    //   그 경우 테스트를 skip(return)★ 하며, (3) 끝에서 우리가 띄운 데몬을 kill 하고 백업을 복원한다.
    //
    // 한계(은폐 금지): 이 smoke 는 운영 data_dir 을 건드리므로(백업/복원으로 최소화하나 완전 격리는
    //   아님) CI 보다는 로컬 수동 검증용이다. 기존 살아있는 데몬이 있으면 skip 된다.
    #[cfg(windows)]
    #[test]
    #[ignore = "실제 WMI Win32_Process.Create — 데몬 exe 필요(수동 통합, Windows 전용)"]
    fn real_wmi_spawn_smoke() {
        let exe = locate_daemon_exe().expect("daemon exe — 먼저 `cargo build` 필요");
        let exe_abs = dunce::canonicalize(&exe).expect("exe canonicalize");

        // 운영 data_dir/daemon.json 경로.
        let data_dir = dirs::data_dir()
            .expect("data_dir")
            .join("com.engram.dashboard");
        std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
        let daemon_path = data_dir.join(DAEMON_FILE);

        // (1) 기존 daemon.json 백업(있으면) + 살아있는 데몬이면 skip.
        let backup = std::fs::read(&daemon_path).ok();
        if let Some(bytes) = &backup {
            if let Ok(prev) = DaemonInfo::parse(bytes) {
                if !RealLiveness.is_dead(prev.pid, prev.start_time) {
                    eprintln!(
                        "real_wmi_spawn_smoke: 기존 데몬(pid={})이 살아있어 단일-인스턴스로 spawn 이 \
                         거부됨 — 검증 무의미하므로 skip",
                        prev.pid
                    );
                    return;
                }
            }
        }
        // stale 또는 부재 → 우리 데몬이 발행할 수 있게 비운다(데몬도 stale 이면 덮어쓰지만 명확히).
        let _ = std::fs::remove_file(&daemon_path);

        // (2) 실제 WMI spawn — RV==0 이어야 성공.
        wmi_spawn(&exe_abs, false).expect("WMI Win32_Process.Create 성공(RV=0, windowless)");

        // (3) 데몬이 daemon.json 을 발행하는지 폴링 회수.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut spawned: Option<DaemonInfo> = None;
        while Instant::now() < deadline {
            if let Ok(bytes) = std::fs::read(&daemon_path) {
                if let Ok(info) = DaemonInfo::parse(&bytes) {
                    // 우리가 띄운 살아있는 데몬인지 확인(stale 잔존 아님).
                    if !RealLiveness.is_dead(info.pid, info.start_time) {
                        spawned = Some(info);
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        // (4) 정리 — 우리가 띄운 데몬 kill(taskkill /F) + daemon.json 백업 복원.
        let result = spawned.clone();
        if let Some(info) = &spawned {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &info.pid.to_string(), "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        // 백업 복원(있었으면) 또는 우리 임시 파일 제거.
        match backup {
            Some(bytes) => {
                let _ = std::fs::write(&daemon_path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&daemon_path);
            }
        }

        // 단언: 데몬이 실제로 떠 daemon.json 을 발행했고, 그 PID 가 (kill 전엔) 살아있었다.
        let info = result.expect("WMI spawn 한 데몬이 daemon.json 을 발행해야");
        assert!(info.port != 0, "spawn 한 데몬은 유효 포트 발행");
        assert_eq!(
            info.protocol_version, PROTOCOL_VERSION,
            "spawn 한 데몬의 protocol_version 일치"
        );
    }

    // ── CreateFlags 진단 매트릭스(실측) — 어느 플래그가 RV=21 을 유발하는지 확정 ─────────────
    //
    // ADR-0021 #1 버그(windowless spawn RV=21) 의 근본 원인을 실증한다. wmi_create_raw 로 동일
    // 데몬 exe 를 네 가지 CreateFlags 조합으로 spawn 하고 raw ReturnValue 를 출력/단언한다:
    //   - None                       (ProcessStartup 생략)          → RV=0 기대(채택안 a)
    //   - CREATE_NEW_CONSOLE(0x10)    (console=true)                 → RV=0 기대(기존 동작)
    //   - DETACHED_PROCESS(0x08)      (대안 b)                       → 관찰만(기대 RV=0)
    //   - CREATE_NO_WINDOW(0x08000000)(기존 windowless, 버그 플래그) → RV=21 기대(거부 재현)
    //
    // 각 spawn 직후 daemon.json 폴링으로 PID 회수 → 즉시 kill(데몬 누적 방지). 기존 살아있는
    // 데몬이 있으면 단일-인스턴스로 spawn 이 무의미하므로 skip.
    #[cfg(windows)]
    #[test]
    #[ignore = "실제 WMI Win32_Process.Create 플래그 매트릭스 — 데몬 exe 필요(수동 진단)"]
    fn real_wmi_spawn_flag_matrix() {
        const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
        const DETACHED_PROCESS: i32 = 0x0000_0008;
        const CREATE_NO_WINDOW: i32 = 0x0800_0000;

        let exe = locate_daemon_exe().expect("daemon exe — 먼저 `cargo build` 필요");
        let exe_abs = dunce::canonicalize(&exe).expect("exe canonicalize");

        let data_dir = dirs::data_dir()
            .expect("data_dir")
            .join("com.engram.dashboard");
        std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
        let daemon_path = data_dir.join(DAEMON_FILE);

        // 기존 daemon.json 백업 + 살아있는 데몬이면 skip(단일-인스턴스로 spawn 거부됨).
        let backup = std::fs::read(&daemon_path).ok();
        if let Some(bytes) = &backup {
            if let Ok(prev) = DaemonInfo::parse(bytes) {
                if !RealLiveness.is_dead(prev.pid, prev.start_time) {
                    eprintln!(
                        "real_wmi_spawn_flag_matrix: 기존 데몬(pid={})이 살아있어 skip",
                        prev.pid
                    );
                    return;
                }
            }
        }

        // 한 케이스 spawn → RV 회수 → (RV=0 이면 떠난 데몬 daemon.json 폴링으로 PID 회수 후 kill).
        let run_case = |label: &str, flags: Option<i32>| -> u32 {
            let _ = std::fs::remove_file(&daemon_path);
            let rv = wmi_create_raw(&exe_abs, flags).expect("WMI create 호출 자체는 성공해야");
            eprintln!("[flag-matrix] {label}: ReturnValue={rv}");
            if rv == 0 {
                // 떠난 데몬 회수 후 kill(누적 방지). 폴링 최대 8s.
                let deadline = Instant::now() + Duration::from_secs(8);
                while Instant::now() < deadline {
                    if let Ok(bytes) = std::fs::read(&daemon_path) {
                        if let Ok(info) = DaemonInfo::parse(&bytes) {
                            if !RealLiveness.is_dead(info.pid, info.start_time) {
                                let _ = std::process::Command::new("taskkill")
                                    .args(["/PID", &info.pid.to_string(), "/F"])
                                    .stdout(std::process::Stdio::null())
                                    .stderr(std::process::Stdio::null())
                                    .status();
                                eprintln!("[flag-matrix] {label}: 데몬 pid={} kill", info.pid);
                                break;
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            rv
        };

        let rv_none = run_case("None(ProcessStartup 생략)", None);
        let rv_new_console = run_case("CREATE_NEW_CONSOLE(0x10)", Some(CREATE_NEW_CONSOLE));
        let rv_detached = run_case("DETACHED_PROCESS(0x08)", Some(DETACHED_PROCESS));
        let rv_no_window = run_case("CREATE_NO_WINDOW(0x08000000)", Some(CREATE_NO_WINDOW));

        // 백업 복원.
        match backup {
            Some(bytes) => {
                let _ = std::fs::write(&daemon_path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&daemon_path);
            }
        }

        // 단언: 채택안(None) 과 기존 console(CREATE_NEW_CONSOLE)은 RV=0, 버그 플래그(CREATE_NO_WINDOW)는
        // RV!=0(거부). DETACHED 는 관찰만(단언 안 함 — 대안 b 참고용).
        eprintln!(
            "[flag-matrix] 요약: None={rv_none} NEW_CONSOLE={rv_new_console} \
             DETACHED={rv_detached} NO_WINDOW={rv_no_window}"
        );
        assert_eq!(
            rv_none, 0,
            "windowless 채택안(ProcessStartup 생략)은 RV=0 이어야"
        );
        assert_eq!(
            rv_new_console, 0,
            "console=true(CREATE_NEW_CONSOLE)는 RV=0 이어야"
        );
        assert_ne!(
            rv_no_window, 0,
            "기존 버그 플래그 CREATE_NO_WINDOW 는 거부(RV!=0)되어야 — 버그 재현"
        );
    }
}
