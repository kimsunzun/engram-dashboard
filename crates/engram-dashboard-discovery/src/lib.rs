//! 데몬 발견(discovery) — 앱(Tauri)이 데몬을 찾고, 없으면 WMI 로 띄운 뒤 port/token 회수.
//!
//! ADR-0029: daemon-only(embedded 제거). 앱은 데몬의 상주 클라이언트라 항상 데몬에 붙는다(WS 는 phase4).
//!
//! ## 설계 — 순수 로직과 OS/WMI 경계 분리
//! 단위 테스트가 OS·WMI·실시간에 의존하지 않도록 부수효과를 trait 으로 주입한다.
//!
//! [`ensure_daemon`] 은 이 trait 들 위에서만 동작하는 **순수 오케스트레이션** 이라 실제
//! WMI spawn·실제 sleep 없이 전 분기를 단위 테스트할 수 있다. 실제 spawn(WMI) 통합은
//! `#[ignore]` 테스트로 남긴다.
//!
//! ## 보안
//! `DaemonInfo.token` 은 로그에 절대 출력하지 않는다(로컬 IPC 파일에만 흐름).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engram_dashboard_net::auth::AuthFrame;
use engram_dashboard_protocol::{AgentCommand, DaemonInfo, RequestId, PROTOCOL_VERSION};

const DAEMON_FILE: &str = "daemon.json";
const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ── data_dir 단일 출처(ADR-0024) ─────────────────────────────────────────────────
//
// ★ENGRAM_DATA_DIR override (테스트 격리 탈출구 — 배포 노브 아님)★:
//   - **유일한 용도 = 통합 테스트의 데이터 격리.** 실프로세스 통합 테스트(daemon `tests/ws_e2e.rs`)가
//     데몬을 임시 디렉토리로 보내 운영 `<repo>/.engram-data` 오염을 막기 위함이다. 이 env 가 없으면
//     테스트 데몬이 운영 폴더에 daemon.json/agents.json 을 쓴다(오염).
//   - **배포용 경로 커스터마이즈 노브가 아니다.** 배포 단계의 데이터 위치는 추후 appdata 로 갈 때
//     별도 결정한다(ADR-0024). 이 override 를 "사용자가 데이터 폴더를 바꾸는 수단"으로 쓰지 말 것.
//   - ★중요 한계 — WMI 경로엔 닿지 않는다★: 이 override 는 **부모 env 를 상속하는 spawn 에만** 먹는다.
//     즉 `std::process::Command` 로 데몬을 **직접** 띄우는 ws_e2e.rs 만 격리된다. discovery 의 운영
//     spawn 경로(WMI Win32_Process.Create)는 자식이 WmiPrvSE 자식이라 **부모 env 를 상속하지 않아**
//     이 override 가 무시된다(설계 확정 — daemon.json/ACL 외 채널 없음). 그래서 WMI 를 실제로 타는
//     discovery 의 smoke 테스트(real_wmi_spawn_*)는 env 로 격리하지 못하고, default 경로(`.engram-data`)
//     를 폴링하며 운영 파일은 백업/복원으로 보호한다.

// ADR-0029: debug 분기(walk-up `.engram-data`)와 그 단위테스트에서만 쓰인다 — release default_data_dir
// 은 %APPDATA% 만 쓰므로 release 비-test 빌드에선 dead_code.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
const LOCAL_DATA_DIR: &str = ".engram-data";

const DATA_DIR_ENV: &str = "ENGRAM_DATA_DIR";

/// Tauri identifier 와 동일(ADR-0027/0029).
const APP_IDENTIFIER: &str = "com.engram.dashboard";

/// engram 프로세스의 데이터 디렉토리(ADR-0024/0029).
///
/// app(src-tauri)과 daemon 이 **같은 default_data_dir()** 를 호출해 같은 폴더의
/// daemon.json/agents.json 을 본다.
///
/// 우선순위:
/// 1. **`ENGRAM_DATA_DIR`(설정+non-empty)** → 그 경로 그대로(테스트 격리 탈출구 — 배포 노브 아님).
/// 2. **디버그(`cfg!(debug_assertions)`)**: current_exe 에서 위로 올라가 repo 루트(`.git` 또는
///    `Cargo.toml` 의 `[workspace]`)를 찾아 `<root>/.engram-data`. 루트 못 찾으면 exe 디렉토리
///    fallback, 그것도 안 되면 cwd. → 개발 한 곳에서 여러 빌드(app·daemon)가 한 폴더 공유.
/// 3. **릴리즈(`not(debug_assertions)`)**: **유저 영역 `%APPDATA%\com.engram.dashboard`**
///    (`appdata_data_dir`). 데몬은 dockerd/tailscaled 처럼 per-user 서비스라 폴더에 안 묶인다.
///
/// 어느 경로든 **절대 패닉하지 않는다**(배포·루트 미발견 상황에서도 PathBuf 를 반드시 반환).
pub fn default_data_dir() -> PathBuf {
    if let Some(val) = std::env::var_os(DATA_DIR_ENV) {
        if !val.is_empty() {
            return PathBuf::from(val);
        }
    }

    #[cfg(debug_assertions)]
    {
        // ★왜 exe-기준 walk-up 인가★: 데몬은 WMI Win32_Process.Create 로 떠 **부모의 cwd 를 상속하지
        // 않는다**(WmiPrvSE 자식) — cwd 는 신뢰할 수 없다. 반면 exe 경로는 신뢰 가능하고, 개발 빌드
        // 산출물은 같은 repo 의 target/ 아래라 어느 exe 에서 올라가도 같은 repo 루트로 수렴한다.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(root) = find_workspace_root(&exe) {
                return root.join(LOCAL_DATA_DIR);
            }
            if let Some(dir) = exe.parent() {
                return dir.join(LOCAL_DATA_DIR);
            }
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(LOCAL_DATA_DIR)
    }

    #[cfg(not(debug_assertions))]
    {
        appdata_data_dir()
    }
}

/// release 데이터 위치 = `%APPDATA%\com.engram.dashboard`(ADR-0027/0029).
///
/// ★release-only(load-bearing)★: 이 헬퍼는 **릴리즈 분기에서만** 호출된다. dirs crate 의존을
/// 추가하지 않으려고 APPDATA env 를 직접 읽는다(의존 추가 0). APPDATA 미설정(드문 환경) 시 cwd
/// join 으로 강등 — **절대 패닉 금지**. 디버그 빌드에선 호출처(release 분기)가 cfg-out 돼
/// dead_code 이므로 디버그에서만 allow.
#[cfg_attr(debug_assertions, allow(dead_code))]
fn appdata_data_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        if !appdata.is_empty() {
            return PathBuf::from(appdata).join(APP_IDENTIFIER);
        }
    }
    // 운영 Windows 에선 APPDATA 가 사실상 항상 설정돼 있다.
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(APP_IDENTIFIER)
}

/// exe 기준으로 해석한 **설치/repo 루트 절대경로**(빌드모드 무관). 데몬이 상대 리소스(예: ADR-0092
/// 프라이밍 `prompts/agent-priming.md`)를 붙일 **신뢰 가능한 base** 로 쓴다.
///
/// 해석(default_data_dir 의 디버그/릴리즈 분기와 동형):
///   - exe 에서 위로 올라가 workspace 루트(`.git` 또는 `Cargo.toml [workspace]`)를 찾으면 그것
///     (개발: `<repo>` — target/debug 어느 exe 에서 올라가도 repo 루트로 수렴).
///   - 루트 못 찾으면 exe 디렉토리(릴리즈: 번들 exe 들이 co-located 되는 폴더 — 리소스도 동거).
///   - exe 조차 못 얻으면 None(호출자가 처리 — 프라이밍은 그때 None 을 산출).
/// ★절대경로 보장★: current_exe 는 절대경로를 주고 walk-up/parent 는 그 절대성을 보존한다.
pub fn find_install_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    if let Some(root) = find_workspace_root(&exe) {
        return Some(root);
    }
    exe.parent().map(|p| p.to_path_buf())
}

fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = if start.is_dir() {
        Some(start)
    } else {
        start.parent()
    };
    while let Some(dir) = cur {
        if is_workspace_root(dir) {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

fn is_workspace_root(dir: &Path) -> bool {
    if dir.join(".git").exists() {
        return true;
    }
    let cargo = dir.join("Cargo.toml");
    match std::fs::read_to_string(&cargo) {
        // 주석에 박힌 `[workspace]` 문자열 같은 극단 케이스는 무시 — repo 루트는 .git 으로도 잡힌다.
        Ok(s) => s.contains("[workspace]"),
        Err(_) => false,
    }
}

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

/// start_time 을 함께 받아 PID 재사용(M2)을 구분한다 — "PID 살아있음 AND creation time==기록값"
/// 일 때만 살아있다고 본다. start_time==0(미상, 옛 daemon.json)이면 PID 단독 생존으로 보수 판정.
pub trait PidLiveness {
    fn is_dead(&self, pid: u32, start_time: u64) -> bool;
}

/// 반환: Ok(Some)=유효 파일, Ok(None)=없음(아직 안 써짐), Err=깨진 파일.
pub trait DaemonReader {
    fn read(&self) -> Result<Option<DaemonInfo>, DiscoveryError>;
}

pub trait Spawner {
    /// 절대경로 exe 를 spawn.
    fn spawn(&self, exe: &Path) -> Result<(), DiscoveryError>;
}

pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, dur: Duration);
}

// ── 순수 오케스트레이션 ────────────────────────────────────────────────────────────

/// info 소유권을 호출자가 유지하도록 참조 기반 판정 — Accept 는 데이터 없이 신호만 준다.
enum AcceptCheck {
    Accept,
    DeadPid,
    VersionMismatch { daemon: u32 },
}

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
            AcceptCheck::Accept => return Ok(info),
            AcceptCheck::DeadPid => {
                removed_live_candidate = Some(info);
                stale_cleanup();
            }
            AcceptCheck::VersionMismatch { daemon } => {
                // 살아있는 데몬을 spawn 으로 덮지 않고 명확히 실패한다(재기동 정책은 phase4
                // DaemonClient 가 결정).
                return Err(DiscoveryError::VersionMismatch {
                    daemon,
                    expected: PROTOCOL_VERSION,
                });
            }
        },
        Ok(None) => {}
        Err(DiscoveryError::Parse(_)) => {
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
            }
            Ok(None) => {}
            Err(DiscoveryError::Parse(_)) => {} // 쓰는 중 부분 파일일 수 있음 → 계속.
            Err(e) => return Err(e),
        }
        if clock.now() >= deadline {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    /// 살아있는 데몬이 발견됐는가(파일 존재 + 호환 버전 + PID live).
    pub alive: bool,
    /// 발견된 데몬 PID(파일이 있으면, 죽었어도 보고). 없으면 None.
    pub pid: Option<u32>,
    pub port: Option<u16>,
}

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
        // 깨진 파일은 신뢰 불가라 pid 미보고.
        _ => DaemonStatus {
            alive: false,
            pid: None,
            port: None,
        },
    }
}

pub fn daemon_status(data_dir: &Path) -> DaemonStatus {
    let reader = FileReader {
        path: data_dir.join(DAEMON_FILE),
    };
    status_with(&reader, &RealLiveness)
}

/// ★daemon_status 와의 차이★: daemon_status 는 pid/port 만 주는 lifecycle probe 다(token 없음). 이
/// 함수는 **재연결이 옮겨간 데몬을 attach** 하는 데 쓰여 host/port/token 전부가 필요하다.
fn read_live_with(reader: &dyn DaemonReader, liveness: &dyn PidLiveness) -> Option<DaemonInfo> {
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => Some(info),
            AcceptCheck::DeadPid | AcceptCheck::VersionMismatch { .. } => None,
        },
        _ => None,
    }
}

/// 살아있는 데몬의 접속 정보(token 포함)를 daemon.json 에서 읽어 반환(real 진입점).
///
/// ★재연결의 "옮겨간 데몬 추적" 수단(ADR-0021)★: hot-swap(daemon_stop→start)·크래시-재spawn 으로
/// 데몬이 새 port/token 으로 떠도, 재연결 루프가 캐시된 옛 주소 대신 이 함수로 **현재 daemon.json** 을
/// 재조회해 그 주소로 attach 한다. ★spawn 하지 않는다★ — 단지 떠 있으면 따라갈 뿐, 깨우지 않는다.
pub fn read_live_daemon(data_dir: &Path) -> Option<DaemonInfo> {
    let reader = FileReader {
        path: data_dir.join(DAEMON_FILE),
    };
    read_live_with(&reader, &RealLiveness)
}

pub trait ProcessKiller {
    /// 성공 여부는 best-effort(이미 죽었으면 Ok 취급).
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError>;
}

// ADR-0024: graceful StopDaemon 무응답/타임아웃 시 taskkill 폴백 자리. send_stop(일방 발사)에
// ack 대기가 추가될 때 여기로 escalate.
//
// ★send_stop 에서 escalate 하는 호출처는 아직 없음 = 의도된 상태(사용자 결정: 강제 폴백은 나중에
//   이어붙임, 일방 발사 먼저). 지우지 말 것 — send_stop 의 미래 폴백 경로다.★
//   (현 호출처 = src-tauri 의 daemon_stop command — 프론트 stop() 의 fallback kill 경로. 배선은
//    send_stop 의 ★나중에 이어붙일 자리★ 주석 참조: send_stop 안에서 ack 타임아웃 시 호출.)
//
/// 데몬 종료 fallback(real 진입점).
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

fn stop_with(
    reader: &dyn DaemonReader,
    liveness: &dyn PidLiveness,
    killer: &dyn ProcessKiller,
) -> Result<Option<u32>, DiscoveryError> {
    match reader.read() {
        Ok(Some(info)) => {
            if liveness.is_dead(info.pid, info.start_time) {
                return Ok(None);
            }
            killer.kill(info.pid)?;
            Ok(Some(info.pid))
        }
        _ => Ok(None),
    }
}

// ── graceful stop(StopDaemon WS 일방 발사, S13 sub-step 2 "2차") ─────────────────────
//
// ★분담(daemon_stop 와의 차이)★: send_stop 은 그 위 계층의 **graceful** 경로 — 데몬에 WS 로
// StopDaemon{force} 를 보내 데몬이 스스로 shutdown_all(자식 PTY 정리) + self-exit 하게
// 한다(connection_core StopDaemon 핸들러가 처리).
//
// ★일방 발사(fire-and-forget) — 사용자 결정★: ack/응답을 읽지 않는다. 응답이 없거나 데몬이 정리
// 중이면 데몬은 그대로 살아있고(probe 가 alive 로 보고), 사용자가 다시 누르면 재발사한다. close 전
// flush 로 메시지가 소켓에 실제 나가는 것만 보장한다.

/// ★왜 enum 으로 끌어올리나(load-bearing)★: 끄기 직후 트레이가 `daemon_status`(PID probe)로 아이콘을
/// 정하면, 데몬이 죽기 직전 수 ms 동안 "아직 살아있음"으로 보여 **아이콘이 컬러로 고착**되는 race 가
/// 있었다(QA 실측). 해결 = PID 를 다시 묻지 않고, drain read 루프에서 관측한 **"데몬이 연결을 닫음"**
/// 을 "꺼짐 확정" 신호로 쓴다. send_stop 이 그 신호를 이 enum 으로 호출자(트레이)에게 올려, 트레이가
/// probe 우회로 아이콘을 회색 확정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// 데몬이 graceful 하게 **이 WS 연결을 닫았다**. 트레이는 probe 없이 회색 확정.
    /// ★의미 한정(과신 금지)★: 이것은 정확히는 "데몬이 StopDaemon 을 처리하고 **종료 경로에 진입**해
    /// 이 연결을 닫았다"는 신호다. 실제 프로세스 exit 는 그 직후(보통 ms)에 일어난다 — 연결 닫힘과
    /// 프로세스 소멸은 동일 순간이 아니다. 정상 경로에선 ms 차라 회색 확정이 맞지만, 데몬의 graceful
    /// 종료 자체가 hang/panic 하면 연결은 닫혔어도 프로세스가 잠깐 더 살 수 있다(그건 별도 버그 —
    /// 일방 발사 재발사 모델이 다음 클릭에서 회수). 이 신호를 "프로세스 죽음 확정"으로 더 신뢰해
    /// probe 폴백을 추가로 제거하지 말 것.
    DaemonClosed,
    /// STOP_WS_TIMEOUT(3s) 내 데몬이 닫지도 응답하지도 않음.
    /// = 불확실(데몬이 아직 정리 중일 수 있음) → 트레이는 기존 probe 폴백.
    Timeout,
    /// 끌 데몬이 없었음(daemon.json 없음/죽음/깨짐/버전 불일치 — send 자체를 안 함).
    /// = 트레이는 기존 probe 폴백(이미 회색일 것).
    NoTarget,
}

pub trait StopSender {
    /// 살아있는 데몬 `info` 에 graceful StopDaemon 을 보낸다(Auth → StopDaemon → flush → drain → close).
    /// 일방 발사라 ack **내용**은 해석하지 않지만, drain read 의 **종료 사유**로
    /// [`StopOutcome::DaemonClosed`](연결 닫힘=꺼짐 확정) / [`StopOutcome::Timeout`](3s 무응답)을 구분해
    /// 반환한다. 송신/연결 실패만 Err.
    fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError>;
}

/// 데몬에 graceful StopDaemon 을 WS 로 보낸다(real 진입점, S13 sub-step 2).
///
/// ★나중에 이어붙일 자리(load-bearing)★: 이 send_stop 경로엔 taskkill(daemon_stop) 강제 폴백이
/// 없다(사용자 결정: 응답 없으면 데몬 활성 유지, 다시 누르면 재발사). 나중에 graceful-with-fallback
/// 으로 키우려면 **이 함수(또는 TungsteniteStopSender::send_stop) 안에** "Timeout 시
/// daemon_stop(data_dir) 호출"을 추가하면 된다 — 호출부는 send_stop 시그니처만 보므로 폴백 자체는
/// 여기서 흡수.
pub fn send_stop(data_dir: &Path) -> Result<StopOutcome, DiscoveryError> {
    stop_with_sender(
        &FileReader {
            path: data_dir.join(DAEMON_FILE),
        },
        &RealLiveness,
        &TungsteniteStopSender,
    )
}

/// ★대상 판정 = check_acceptable(Accept)★: 버전 불일치 데몬은 어차피 데몬의 Auth 가
/// protocol_version 검사로 거부하므로 일방 발사가 무의미하고, 그런 데몬 종료는 taskkill
/// 폴백(daemon_stop)의 몫이다(미래 연결).
fn stop_with_sender(
    reader: &dyn DaemonReader,
    liveness: &dyn PidLiveness,
    sender: &dyn StopSender,
) -> Result<StopOutcome, DiscoveryError> {
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => sender.send_stop(&info),
            AcceptCheck::DeadPid | AcceptCheck::VersionMismatch { .. } => Ok(StopOutcome::NoTarget),
        },
        _ => Ok(StopOutcome::NoTarget),
    }
}

/// force=true·kill_agents=true 고정(작업 중 에이전트가 있어도 데몬이 정리하고 끔 — 사용자 결정).
/// request_id 는 새 Uuid(데몬이 에코하지만 우리는 ack 를 안 읽으므로 매칭에 안 씀 — 프로토콜 필수
/// 필드라 채울 뿐).
fn build_stop_command() -> AgentCommand {
    AgentCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: RequestId::new(),
    }
}

/// 데몬은 연결 1초 내 첫 프레임으로 이걸 기대한다(네트워크 lib 의 AUTH_TIMEOUT).
///
/// ★타입 출처(ADR-0129 0-4)★: 이건 **명령이 아니라 네트워크 lib 소유 프레임**이다(`AuthFrame`).
/// 데몬의 인증 판정을 하는 그 crate 가 모양의 정본을 쥐므로, 발신자가 제 손으로 JSON 을 짜는 대신
/// 같은 타입을 쓴다 — 그래야 한쪽만 바뀌는 표류가 컴파일 에러가 된다.
fn build_auth_command(token: &str) -> AuthFrame {
    AuthFrame::Auth {
        token: token.to_string(),
        protocol_version: PROTOCOL_VERSION,
    }
}

/// ★데몬 핸드셰이크와 1:1(ws.rs)★: 데몬 read_task 는 Message::Text 만 AgentCommand 로 파싱하고
/// Binary 는 거부하므로 Text 로 보낸다.
struct TungsteniteStopSender;

/// send_stop 의 connect/handshake/read/write 상한(초). 이 값을 넘으면 깔끔히 에러로 빠진다.
///
/// ★왜 timeout 이 load-bearing 인가★: 기본 `tungstenite::connect(url)` 은 내부 `TcpStream::connect`
/// 를 **timeout 없이** 호출하고 handshake read 에도 상한이 없다. daemon.json 의 pid/port 가 stale
/// 인데 그 PID 가 재사용(M2)으로 liveness 판정을 우회한 드문 경우, 닫혔거나 방화벽이 막은 포트로의
/// connect 시도가 Windows 기본 ~21초까지 블록될 수 있다 — 트레이 stop 워커 스레드가 그동안 묶여
/// 아이콘/상태 갱신이 지연된다(워커 누수에 준함). connect_timeout + set_read/write_timeout 으로
/// 모든 블로킹 구간(TCP 연결 → WS handshake read → send/flush/close)에 상한을 박아 무한 블록을 막는다.
const STOP_WS_TIMEOUT: Duration = Duration::from_secs(3);

impl StopSender for TungsteniteStopSender {
    fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
        use std::net::{SocketAddr, TcpStream};
        use tungstenite::{Error as WsError, Message};

        // ws://host:port — 데몬은 로컬 평문 WS(TLS 없음, ws:// 고정). host 는 항상 127.0.0.1 loopback.
        let url = format!("ws://{}:{}", info.host, info.port);

        let addr: SocketAddr = format!("{}:{}", info.host, info.port)
            .parse()
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 주소 파싱 실패({url}): {e}")))?;
        let stream = TcpStream::connect_timeout(&addr, STOP_WS_TIMEOUT)
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon WS 접속 실패({url}): {e}")))?;
        stream
            .set_read_timeout(Some(STOP_WS_TIMEOUT))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon read timeout 설정 실패: {e}")))?;
        stream
            .set_write_timeout(Some(STOP_WS_TIMEOUT))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon write timeout 설정 실패: {e}")))?;
        // 요청은 URL 뿐이라 HandshakeError Display 에도 token 은 들어가지 않는다.
        let (mut ws, _resp) = tungstenite::client(&url, stream)
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon WS handshake 실패({url}): {e}")))?;

        let auth = serde_json::to_string(&build_auth_command(&info.token))
            .map_err(|e| DiscoveryError::Io(format!("Auth 직렬화 실패: {e}")))?;
        ws.send(Message::Text(auth.into()))
            .map_err(|e| DiscoveryError::Io(format!("Auth 전송 실패: {e}")))?;

        let stop = serde_json::to_string(&build_stop_command())
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 직렬화 실패: {e}")))?;
        ws.send(Message::Text(stop.into()))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 전송 실패: {e}")))?;

        // ★flush 로 두 프레임을 소켓에 실제 밀어낸다★ — tungstenite send 는 내부 버퍼링이라
        // flush 없이 곧장 close 하면 미전송 가능. 일방 발사의 "도달 보장"이 이 flush 다.
        ws.flush()
            .map_err(|e| DiscoveryError::Io(format!("WS flush 실패: {e}")))?;

        // ★drain read — 즉시 close 금지(QA 실측 회귀 수정)★:
        //    flush 직후 곧장 ws.close() 하면 데몬 write_task 가 닫힌 소켓에 outbound(Hello 등)를 write
        //    하다 os error 10053 으로 실패 → write_task 종료 → 데몬의 "한쪽 끝나면 상대 abort"(ws.rs)로
        //    read_task 가 StopDaemon 을 read 하기 전에 abort → StopDaemon dispatch 안 됨 → 데몬이
        //    graceful self-exit 못 함(생존). 즉시 close 가 데몬에게서 "StopDaemon 을 read 하고 처리할
        //    시간"을 뺏는 게 결함이었다. 그래서 데몬이 self-exit 로 연결을 닫을 때까지(또는 read_timeout
        //    3s) 소켓에서 read 를 돌려 처리 시간을 준다.
        //    "받기"가 아니라 "데몬에 시간 주기"로 read 를 도는 것이다. 3s 상한이라 데몬이 안 죽어도
        //    send_stop 은 최대 3s 후 반환(connect timeout 과 같은 워커 블록 bound).
        let outcome = loop {
            match ws.read() {
                Ok(Message::Close(_)) => break StopOutcome::DaemonClosed,
                Ok(_) => {}
                Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                    break StopOutcome::DaemonClosed
                }
                Err(WsError::Io(e)) => {
                    use std::io::ErrorKind;
                    match e.kind() {
                        ErrorKind::WouldBlock | ErrorKind::TimedOut => break StopOutcome::Timeout,
                        // EOF/연결 끊김 — 데몬 프로세스가 사라져 소켓이 끊김 → 꺼짐 확정.
                        _ => break StopOutcome::DaemonClosed,
                    }
                }
                // 그 외 WS 에러(프로토콜/Utf8 등) — 더 받을 게 없으니 종료하되, 데몬이 닫았다고 단정할 수
                // 없어 Timeout(불확실)으로 본다(probe 폴백으로 안전하게 회수).
                Err(_) => break StopOutcome::Timeout,
            }
        };

        // 데몬이 이미 닫았으면 무해하고, 안 닫았어도 drop 으로 닫힌다 — 명시적 close 는 best-effort.
        let _ = ws.close(None);
        Ok(outcome)
    }
}

struct TaskKiller;

impl ProcessKiller for TaskKiller {
    #[cfg(windows)]
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError> {
        // /T 로 자식 트리도 정리(데몬 Job 안전망과 중복이나 무해).
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| DiscoveryError::Io(format!("taskkill 실행 실패: {e}")))?;
        // taskkill 은 "이미 종료됨"(exit 128)도 있으므로 종료 코드를 판정하지 않는다.
        let _ = status;
        Ok(())
    }

    #[cfg(not(windows))]
    fn kill(&self, _pid: u32) -> Result<(), DiscoveryError> {
        Err(DiscoveryError::Io("daemon_stop 은 Windows 전용".into()))
    }
}

// ── 공개 진입점 ─────────────────────────────────────────────────────────────────

/// `data_dir` = daemon.json 디렉토리(앱·데몬 공유 default_data_dir()).
/// `daemon_exe` = 데몬 실행 파일 경로(절대화는 내부에서 dunce::canonicalize).
pub fn ensure_daemon(
    data_dir: &Path,
    daemon_exe: &Path,
    timeout: Duration,
    console: bool,
) -> Result<DaemonInfo, DiscoveryError> {
    let daemon_path = data_dir.join(DAEMON_FILE);

    let exe_abs = dunce::canonicalize(daemon_exe)
        .map_err(|e| DiscoveryError::ExeNotFound(format!("{}: {e}", daemon_exe.display())))?;

    let reader = FileReader {
        path: daemon_path.clone(),
    };
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
            candidates.push(dir.join(EXE));
        }
    }
    // 워크스페이스 빌드면 target/debug 가 공유라 위 후보로 충분하나, 안전하게 한 번 더.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join(EXE));
        candidates.push(cwd.join("..").join("target").join("debug").join(EXE));
    }

    locate_in(&candidates)
}

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

struct WmiSpawner {
    /// true=별도 콘솔 창(CREATE_NEW_CONSOLE, 디버그 로그 가시화), false=CreateFlags 미전달(기본).
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
    //     - windowless(console=false) → ProcessStartupInformation 자체 생략(create_flags=None). RV=0.
    //       ★주의(2026-06-19 실측 정정)★: 콘솔 창 노출 여부는 여기 플래그가 아니라 **데몬 exe 의
    //       서브시스템**에 달렸다. 데몬은 디버그=콘솔 앱(`windows_subsystem` 미설정) → WMI-spawn 시
    //       콘솔 창이 **뜬다**(로그용, 의도) / 릴리즈=windows 앱(`#![cfg_attr(not(debug_assertions),
    //       windows_subsystem="windows")]`) → 콘솔 창 **없음**. 옛 주석은 "WmiPrvSE 자식이라 콘솔이
    //       애초에 안 뜬다"고 단정했으나 콘솔 앱에선 거짓이었다 — windowless 는 WMI 플래그가 아니라
    //       데몬 서브시스템으로만 달성된다(CREATE_NO_WINDOW 는 위 RV=21 로 막혀 WMI 로는 불가).
    //     - console=true → CREATE_NEW_CONSOLE(0x10): 허용 플래그라 RV=0, 별도 콘솔 창과 함께 뜬다.
    const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
    let create_flags: Option<i32> = if console {
        Some(CREATE_NEW_CONSOLE)
    } else {
        None
    };

    let rv = wmi_create_raw(exe, create_flags)?;
    if rv != 0 {
        return Err(DiscoveryError::SpawnFailed { rv });
    }
    Ok(())
}

/// RV!=0 을 에러로 승격하지 않는다 — flag-matrix 실측 테스트가 RV 자체를 비교하기 위함.
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

    // 인자 없음 — 데몬은 인자 불필요.
    let exe_str = exe.to_string_lossy();
    let command_line = format!("\"{exe_str}\"");

    // SAFETY 블록: COM/WMI 호출 시퀀스. spike #1 의 PowerShell Invoke-CimMethod 와 동일한
    // Win32_Process.Create 를 COM 직접 호출로 수행한다.
    unsafe {
        // SAFETY: CoInitializeEx 는 스레드 단위 COM 초기화. 반환 HRESULT 로 짝맞춤(아래 가드).
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _com_guard = match classify_com_init(hr.0) {
            ComInit::Initialized => ComGuard { needs_uninit: true },
            ComInit::AlreadyOtherMode => ComGuard {
                needs_uninit: false,
            },
            ComInit::Failed(code) => {
                return Err(DiscoveryError::Io(format!(
                    "CoInitializeEx 실패 HRESULT {:#010x}",
                    code as u32
                )));
            }
        };

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

        // 로컬 WMI 호출에 필요한 impersonation 레벨.
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

        let cl_value = VARIANT::from(BSTR::from(command_line.as_str()));
        in_inst
            .Put(&BSTR::from("CommandLine"), 0, &cl_value, 0)
            .map_err(wmi_err)?;

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
            // CreateFlags 는 VT_I4(부호 있는 32-bit).
            let flags_value = VARIANT::from(create_flags);
            startup_inst
                .Put(&BSTR::from("CreateFlags"), 0, &flags_value, 0)
                .map_err(wmi_err)?;
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

        // 토큰/pid 는 daemon.json 폴링으로 회수하므로 여기선 RV 만 본다.
        let rv = match out {
            Some(out) => read_u32_prop(&out, "ReturnValue").unwrap_or(u32::MAX),
            None => u32::MAX,
        };
        Ok(rv)
    }
}

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

    // ── data_dir resolver ─────────────────────────────────────────────────────────────

    /// ENGRAM_DATA_DIR 은 프로세스 전역 env 라, 이걸 만지는 테스트끼리 병렬로 돌면 서로 set/remove 를
    /// 짓밟는다.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn data_dir_env_override_returns_path_verbatim() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os(DATA_DIR_ENV);
        let want = std::env::temp_dir().join("engram-data-dir-override-test");
        std::env::set_var(DATA_DIR_ENV, &want);
        let got = default_data_dir();
        // 단언 전에 복원해 단언 실패에도 env 가 leak 되지 않게 한다.
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert_eq!(got, want, "ENGRAM_DATA_DIR set 시 그 경로 그대로 반환");
    }

    #[test]
    fn data_dir_empty_env_falls_through_to_default() {
        // 테스트는 항상 debug 빌드라 walk-up `.engram-data` 분기를 탄다.
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os(DATA_DIR_ENV);
        std::env::set_var(DATA_DIR_ENV, "");
        let got = default_data_dir();
        match &prev {
            Some(v) => std::env::set_var(DATA_DIR_ENV, v),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        assert!(
            got.ends_with(LOCAL_DATA_DIR),
            "빈 env 는 기본 분기로 통과 → `.engram-data` 로 끝나야: {got:?}"
        );
    }

    #[test]
    fn default_data_dir_debug_is_local_data_dir() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os(DATA_DIR_ENV);
        std::env::remove_var(DATA_DIR_ENV);
        let got = default_data_dir();
        if let Some(v) = &prev {
            std::env::set_var(DATA_DIR_ENV, v);
        }
        assert!(
            got.ends_with(LOCAL_DATA_DIR),
            "debug 는 폴더-로컬 `.engram-data` 로 끝나야: {got:?}"
        );
    }

    #[test]
    fn appdata_data_dir_uses_appdata_env() {
        // release 전용 헬퍼지만 빌드모드 무관 순수 함수라 debug 테스트로 규칙을 검증한다.
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os("APPDATA");
        let want_root = std::env::temp_dir().join("engram-appdata-test");
        std::env::set_var("APPDATA", &want_root);
        let got = appdata_data_dir();
        match &prev {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        assert_eq!(
            got,
            want_root.join(APP_IDENTIFIER),
            "APPDATA/com.engram.dashboard 이어야"
        );
        assert!(
            got.ends_with(APP_IDENTIFIER),
            "release-daemon 경로는 com.engram.dashboard 로 끝나야: {got:?}"
        );
    }

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

    struct FakeLiveness {
        dead: Vec<u32>,
    }
    impl PidLiveness for FakeLiveness {
        fn is_dead(&self, pid: u32, _start_time: u64) -> bool {
            self.dead.contains(&pid)
        }
    }

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
            // 시퀀스 소진 후 Ok(None) 은 timeout 경로 모사용이다.
            self.seq.borrow_mut().pop_front().unwrap_or(Ok(None))
        }
    }

    // 항상 성공한다 — spawn 실패 경로는 spawn_failure_propagates 가 따로 본다.
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
            // 실제로 자지 않아 폴링 timeout 이 즉시 도달한다.
            self.slept.set(self.slept.get() + 1);
            *self.now.borrow_mut() += dur;
        }
    }

    fn noop_cleanup() -> impl FnMut() {
        || {}
    }

    #[test]
    fn live_existing_file_returns_without_spawn() {
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
        let reader = FakeReader::new(vec![Ok(None)]);
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
        // 같은 (pid,start) 가 (a) 에서는 dead, timeout 재검사에서는 live 여야 시나리오가 성립한다 —
        // is_dead 호출 시점에 따라 답이 바뀌는 가짜를 쓴다.
        struct FlipLiveness {
            calls: Cell<usize>,
        }
        impl PidLiveness for FlipLiveness {
            fn is_dead(&self, _pid: u32, _start: u64) -> bool {
                let n = self.calls.get();
                self.calls.set(n + 1);
                n == 0
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
        let reader = FakeReader::new(vec![
            Ok(Some(info_with_start(43, PROTOCOL_VERSION, 888))),
            Ok(None),
        ]);
        let spawner = CountingSpawner::ok();
        let liveness = StartTimeLiveness { live: vec![] };
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
        assert_eq!(classify_com_init(S_OK), ComInit::Initialized);
        assert_eq!(classify_com_init(S_FALSE), ComInit::Initialized);
        assert_eq!(
            classify_com_init(RPC_E_CHANGED_MODE),
            ComInit::AlreadyOtherMode
        );
        let e_fail = 0x8000_4005u32 as i32; // E_FAIL 류 임의 실패
        assert_eq!(classify_com_init(e_fail), ComInit::Failed(e_fail));
    }

    #[test]
    fn com_init_needs_uninit_only_when_we_initialized() {
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
        let reader = FakeReader::new(vec![Ok(Some(info(111, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let s = status_with(&reader, &liveness);
        assert!(s.alive);
        assert_eq!(s.pid, Some(111));
        assert_eq!(s.port, Some(12345));
    }

    #[test]
    fn status_dead_file_reports_not_alive_but_keeps_pid() {
        let reader = FakeReader::new(vec![Ok(Some(info(222, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![222] };
        let s = status_with(&reader, &liveness);
        assert!(!s.alive);
        assert_eq!(s.pid, Some(222));
    }

    #[test]
    fn status_version_mismatch_is_not_alive() {
        let reader = FakeReader::new(vec![Ok(Some(info(333, PROTOCOL_VERSION + 1)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let s = status_with(&reader, &liveness);
        assert!(!s.alive, "버전 불일치는 붙을 수 없으므로 alive=false");
        assert_eq!(s.pid, Some(333));
    }

    #[test]
    fn status_missing_file_is_not_alive_no_pid() {
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let s = status_with(&reader, &liveness);
        assert!(!s.alive);
        assert_eq!(s.pid, None);
        assert_eq!(s.port, None);
    }

    // ── read_live_daemon (token 포함 attach 정보, no-spawn) — ADR-0021 hot-swap 추적 ──────

    #[test]
    fn read_live_returns_full_info_with_token() {
        let reader = FakeReader::new(vec![Ok(Some(info(666, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let got = read_live_with(&reader, &liveness).expect("live 데몬이면 Some");
        assert_eq!(got.pid, 666);
        assert_eq!(got.host, "127.0.0.1");
        assert_eq!(got.port, 12345);
        assert_eq!(
            got.token,
            "t".repeat(64),
            "재연결 attach 에 token 이 실려야 함"
        );
    }

    #[test]
    fn read_live_dead_daemon_is_none() {
        let reader = FakeReader::new(vec![Ok(Some(info(777, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![777] };
        assert!(read_live_with(&reader, &liveness).is_none());
    }

    #[test]
    fn read_live_version_mismatch_is_none() {
        let reader = FakeReader::new(vec![Ok(Some(info(888, PROTOCOL_VERSION + 1)))]);
        let liveness = FakeLiveness { dead: vec![] };
        assert!(read_live_with(&reader, &liveness).is_none());
    }

    #[test]
    fn read_live_missing_or_broken_is_none() {
        let liveness = FakeLiveness { dead: vec![] };
        let none_reader = FakeReader::new(vec![Ok(None)]);
        assert!(read_live_with(&none_reader, &liveness).is_none());
        let broken_reader = FakeReader::new(vec![Err(DiscoveryError::Parse("bad".into()))]);
        assert!(read_live_with(&broken_reader, &liveness).is_none());
    }

    #[test]
    fn stop_live_daemon_kills_pid() {
        let reader = FakeReader::new(vec![Ok(Some(info(444, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let killer = CountingKiller::new();
        let got = stop_with(&reader, &liveness, &killer).unwrap();
        assert_eq!(got, Some(444));
        assert_eq!(killer.killed.borrow().as_slice(), &[444]);
    }

    #[test]
    fn stop_dead_daemon_does_not_kill() {
        let reader = FakeReader::new(vec![Ok(Some(info(555, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![555] };
        let killer = CountingKiller::new();
        let got = stop_with(&reader, &liveness, &killer).unwrap();
        assert_eq!(got, None);
        assert!(killer.killed.borrow().is_empty(), "죽은 데몬은 kill 금지");
    }

    #[test]
    fn stop_missing_file_is_noop() {
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let killer = CountingKiller::new();
        let got = stop_with(&reader, &liveness, &killer).unwrap();
        assert_eq!(got, None);
        assert!(killer.killed.borrow().is_empty());
    }

    // ── send_stop (graceful StopDaemon WS 일방 발사) — 순수 판정/조립 ──────────────────
    //
    // 실 WS 왕복은 QA(실 데몬) 영역. 여기선 (1) 대상 판정(어떤 데몬에 보내고 안 보내는지), (2) 보낼
    // 메시지 조립(Auth/StopDaemon 직렬화 형태)을 StopSender fake 로 검증한다.

    struct CountingStopSender {
        sent: RefCell<Vec<u32>>,
    }
    impl CountingStopSender {
        fn new() -> Self {
            Self {
                sent: RefCell::new(Vec::new()),
            }
        }
    }
    impl StopSender for CountingStopSender {
        fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
            self.sent.borrow_mut().push(info.pid);
            Ok(StopOutcome::DaemonClosed)
        }
    }

    #[test]
    fn send_stop_live_daemon_sends() {
        let reader = FakeReader::new(vec![Ok(Some(info(1001, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(sender.sent.borrow().as_slice(), &[1001], "live 면 1회 발사");
        assert_eq!(
            outcome,
            StopOutcome::DaemonClosed,
            "sender 결과를 그대로 전파"
        );
    }

    #[test]
    fn send_stop_dead_daemon_is_noop() {
        let reader = FakeReader::new(vec![Ok(Some(info(1002, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![1002] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget, "죽은 데몬 → NoTarget");
        assert!(
            sender.sent.borrow().is_empty(),
            "죽은 데몬엔 graceful stop 안 보냄"
        );
    }

    #[test]
    fn send_stop_missing_file_is_noop() {
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget);
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn send_stop_corrupt_file_is_noop() {
        let reader = FakeReader::new(vec![Err(DiscoveryError::Parse("bad".into()))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).expect("깨진 파일은 no-op Ok");
        assert_eq!(outcome, StopOutcome::NoTarget);
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn send_stop_version_mismatch_is_noop() {
        let reader = FakeReader::new(vec![Ok(Some(info(1003, PROTOCOL_VERSION + 1)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget, "버전 불일치 → NoTarget");
        assert!(
            sender.sent.borrow().is_empty(),
            "버전 불일치는 graceful 대상 아님"
        );
    }

    #[test]
    fn send_stop_propagates_sender_outcome_timeout() {
        struct TimeoutSender;
        impl StopSender for TimeoutSender {
            fn send_stop(&self, _info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
                Ok(StopOutcome::Timeout)
            }
        }
        let reader = FakeReader::new(vec![Ok(Some(info(1005, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let outcome = stop_with_sender(&reader, &liveness, &TimeoutSender).unwrap();
        assert_eq!(
            outcome,
            StopOutcome::Timeout,
            "live 데몬 + sender Timeout → Timeout 전파"
        );
    }

    #[test]
    fn send_stop_propagates_sender_error() {
        struct FailingSender;
        impl StopSender for FailingSender {
            fn send_stop(&self, _info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
                Err(DiscoveryError::Io("send boom".into()))
            }
        }
        let reader = FakeReader::new(vec![Ok(Some(info(1004, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let err = stop_with_sender(&reader, &liveness, &FailingSender).unwrap_err();
        assert!(matches!(err, DiscoveryError::Io(_)), "{err:?}");
    }

    #[test]
    fn build_stop_command_is_force_kill_stopdaemon() {
        // 데몬 read_task 의 serde_json::from_str 이 파싱할 형태를 못 박는다.
        match build_stop_command() {
            AgentCommand::StopDaemon {
                force, kill_agents, ..
            } => {
                assert!(force, "force=true(작업 중 에이전트 있어도 끔)");
                assert!(kill_agents, "kill_agents=true");
            }
            other => panic!("StopDaemon 이 아님: {other:?}"),
        }
        let json = serde_json::to_string(&build_stop_command()).unwrap();
        assert!(
            json.contains("StopDaemon"),
            "externally-tagged 태그: {json}"
        );
        assert!(json.contains("\"force\":true"));
        assert!(json.contains("\"kill_agents\":true"));
    }

    #[test]
    fn build_auth_command_carries_token_and_version() {
        // ★단일 variant 라 반증 불가 패턴이다(ADR-0129 0-4)★ — 옛 `other => panic!` 갈래는 이제 존재할
        //   수 없는 상태라 지웠다(단언이 약해진 게 아니라 컴파일러가 대신 보증한다).
        let token = "f".repeat(64);
        let AuthFrame::Auth {
            token: t,
            protocol_version,
        } = build_auth_command(&token);
        assert_eq!(t, token);
        assert_eq!(protocol_version, PROTOCOL_VERSION);

        // wire 형태 — 태그 존재만이 아니라 **바이트 전체**를 못 박는다. 이 프레임을 받는 쪽(네트워크 lib)이
        // 같은 문자열을 golden 으로 들고 있고, 데몬은 이 crate 의 타입을 쓰지 않으므로 둘을 잇는 것은
        // 이 형태뿐이다(트레이 stop 이 조용히 인증에 실패하면 데몬이 안 꺼진다).
        let json = serde_json::to_string(&build_auth_command(&token)).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"Auth":{{"token":"{token}","protocol_version":{PROTOCOL_VERSION}}}}}"#),
            "핸드셰이크 wire 형태(externally-tagged)"
        );
    }

    // ── find_workspace_root / is_workspace_root (임시 디렉토리 트리, 빌드모드 무관) ──────

    fn unique_tmp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "engram-ws-root-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn find_workspace_root_detects_git_marker_walking_up() {
        let root = unique_tmp("git");
        let deep = root.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let got = find_workspace_root(&deep).expect(".git 마커를 위로 올라가며 찾아야");
        // 임시 디렉토리는 심볼릭(예: macOS /var→/private) 일 수 있어 canonicalize 후 비교.
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_workspace_root_detects_cargo_workspace_marker() {
        let root = unique_tmp("cargo");
        let sub = root.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"[workspace]\nmembers = [\"x\"]\n").unwrap();

        let got = find_workspace_root(&sub).expect("[workspace] Cargo.toml 을 찾아야");
        assert_eq!(
            std::fs::canonicalize(&got).unwrap(),
            std::fs::canonicalize(&root).unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_workspace_root_none_when_no_marker() {
        let root = unique_tmp("none");
        let deep = root.join("x").join("y");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"z\"\n").unwrap();

        assert!(
            find_workspace_root(&deep).is_none(),
            "마커 없으면 None — [package] 단독은 workspace 루트가 아님"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_install_root_yields_absolute_path() {
        // 구체 경로는 실행 환경 의존이라 값은 단언하지 않고 절대성만 본다.
        let root = find_install_root().expect("테스트 실행 중이면 current_exe 존재 → Some");
        assert!(
            root.is_absolute(),
            "find_install_root 는 절대경로여야(cwd 불신 계약): {root:?}"
        );
    }

    #[test]
    fn is_workspace_root_distinguishes_markers() {
        let base = unique_tmp("is");
        let git_dir = base.join("g");
        std::fs::create_dir_all(git_dir.join(".git")).unwrap();
        assert!(is_workspace_root(&git_dir), ".git 존재 → true");

        let ws_dir = base.join("w");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::fs::write(ws_dir.join("Cargo.toml"), b"[workspace]\n").unwrap();
        assert!(is_workspace_root(&ws_dir), "[workspace] → true");

        let pkg_dir = base.join("p");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("Cargo.toml"), b"[package]\nname=\"q\"\n").unwrap();
        assert!(!is_workspace_root(&pkg_dir), "[package] 단독 → false");

        let empty_dir = base.join("e");
        std::fs::create_dir_all(&empty_dir).unwrap();
        assert!(!is_workspace_root(&empty_dir), "마커 없음 → false");

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── locate_daemon_exe (tempfile 주입 가능 분기) ─────────────────────────────────

    #[test]
    fn locate_daemon_exe_no_candidates_returns_exe_not_found() {
        let bogus = std::env::temp_dir().join("engram-no-such-daemon-dir-xyz");
        let _ = std::fs::remove_dir_all(&bogus);
        let candidates = vec![bogus.join("engram-dashboard-daemon.exe")];
        let err = locate_in(&candidates).unwrap_err();
        assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
    }

    #[test]
    fn locate_daemon_exe_picks_first_existing() {
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
        let data_dir = std::env::temp_dir();
        let missing = std::env::temp_dir().join("engram-definitely-missing-daemon.exe");
        let _ = std::fs::remove_file(&missing);
        let err = ensure_daemon(&data_dir, &missing, Duration::from_millis(50), false).unwrap_err();
        assert!(matches!(err, DiscoveryError::ExeNotFound(_)), "{err:?}");
    }

    // ── 실제 WMI spawn smoke(실프로세스) — `-- --ignored` 로 실행(Windows 전용) ───────────
    //
    // ★기존 데몬이 살아있으면 단일-인스턴스 mutex 로 우리 spawn 이 거부돼 검증이 무의미하므로
    //   그 경우 skip(return) 한다.★
    //
    // 한계(은폐 금지): 이 smoke 는 운영 data_dir(`.engram-data`)을 건드리므로(백업/복원으로 최소화하나
    //   완전 격리는 아님) CI 보다는 로컬 수동 검증용이다.
    #[cfg(windows)]
    #[test]
    #[ignore = "실제 WMI Win32_Process.Create — 데몬 exe 필요(수동 통합, Windows 전용)"]
    fn real_wmi_spawn_smoke() {
        let exe = locate_daemon_exe().expect("daemon exe — 먼저 `cargo build` 필요");
        let exe_abs = dunce::canonicalize(&exe).expect("exe canonicalize");

        // WMI-spawn 데몬이 실제로 쓰는 default 경로(env 미상속).
        let data_dir = default_data_dir();
        std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
        let daemon_path = data_dir.join(DAEMON_FILE);

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
        // 데몬도 stale 이면 덮어쓰지만 명확히 비우고 간다.
        let _ = std::fs::remove_file(&daemon_path);

        wmi_spawn(&exe_abs, false).expect("WMI Win32_Process.Create 성공(RV=0, windowless)");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut spawned: Option<DaemonInfo> = None;
        while Instant::now() < deadline {
            if let Ok(bytes) = std::fs::read(&daemon_path) {
                if let Ok(info) = DaemonInfo::parse(&bytes) {
                    if !RealLiveness.is_dead(info.pid, info.start_time) {
                        spawned = Some(info);
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let result = spawned.clone();
        if let Some(info) = &spawned {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &info.pid.to_string(), "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        match backup {
            Some(bytes) => {
                let _ = std::fs::write(&daemon_path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&daemon_path);
            }
        }

        let info = result.expect("WMI spawn 한 데몬이 daemon.json 을 발행해야");
        assert!(info.port != 0, "spawn 한 데몬은 유효 포트 발행");
        assert_eq!(
            info.protocol_version, PROTOCOL_VERSION,
            "spawn 한 데몬의 protocol_version 일치"
        );
    }

    // ── CreateFlags 진단 매트릭스(실측) — 어느 플래그가 RV=21 을 유발하는지 확정 ─────────────
    //
    // ADR-0021 #1 버그(windowless spawn RV=21) 의 근본 원인을 실증한다.
    //
    // 각 spawn 직후 daemon.json 폴링으로 PID 회수 → 즉시 kill(데몬 누적 방지).
    #[cfg(windows)]
    #[test]
    #[ignore = "실제 WMI Win32_Process.Create 플래그 매트릭스 — 데몬 exe 필요(수동 진단)"]
    fn real_wmi_spawn_flag_matrix() {
        const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
        const DETACHED_PROCESS: i32 = 0x0000_0008;
        const CREATE_NO_WINDOW: i32 = 0x0800_0000;

        let exe = locate_daemon_exe().expect("daemon exe — 먼저 `cargo build` 필요");
        let exe_abs = dunce::canonicalize(&exe).expect("exe canonicalize");

        let data_dir = default_data_dir();
        std::fs::create_dir_all(&data_dir).expect("data_dir 생성");
        let daemon_path = data_dir.join(DAEMON_FILE);

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

        let run_case = |label: &str, flags: Option<i32>| -> u32 {
            let _ = std::fs::remove_file(&daemon_path);
            let rv = wmi_create_raw(&exe_abs, flags).expect("WMI create 호출 자체는 성공해야");
            eprintln!("[flag-matrix] {label}: ReturnValue={rv}");
            if rv == 0 {
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

        match backup {
            Some(bytes) => {
                let _ = std::fs::write(&daemon_path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&daemon_path);
            }
        }

        // DETACHED 는 관찰만 한다(단언 안 함 — 대안 b 참고용).
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
