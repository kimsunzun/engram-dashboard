//! 데몬 발견(discovery) — 셸이 데몬을 찾고, 없으면 WMI 로 띄운 뒤 port/token 회수. 상태 조회 · 끄기 fallback 포함.
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
//! trait 의 실물 구현(PID 판정 · WMI 띄우기 · 프로세스 트리 끄기)은 OS 층 crate `platform` 의 함수를 부를 뿐이다 —
//! 이 모듈의 운영 코드에는 OS 분기가 없다(ADR-0266).
//!
//! 데이터 폴더 루트 · `daemon.json` 자리 · `logs\` 는 데몬 정본의 사본이다([`layout`]). graceful 끄기(`StopDaemon`
//! 일방 발사)는 [`crate::daemon_client::stop`] 이 갖는다.
//!
//! ## 보안
//! `DaemonInfo.token` 은 로그에 절대 출력하지 않는다(로컬 IPC 파일에만 흐름).
// ADR-0271

use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_base::writable::{probe_write_in, retry_if_vanished};
use engram_dashboard_protocol::{DaemonInfo, PROTOCOL_VERSION};

pub mod layout;
#[cfg(test)]
pub(crate) mod tests;

pub use layout::{default_data_dir, DataLayout};

use layout::{data_dir_env_override, DAEMON_FILE};

/// `ENGRAM_DATA_DIR` 를 바꾸거나 그 값으로 경로를 고르는 셸 lib 시험이 전부 쥐는 락 — 같은 경로 시험(`layout`)과 실
/// WMI 시험 둘(`tests`)이다. 프로세스 전역 환경이라 락이 둘이면 서로를 막지 못한다.
// ADR-0282
#[cfg(test)]
pub(crate) static DATA_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ★ENGRAM_DATA_DIR override 의 한계 — WMI 경로엔 닿지 않는다★(override 규칙의 정본 = 데몬 `data_dir`):
//   이 override 는 **부모 env 를 상속하는 spawn 에만** 먹는다. 즉 `std::process::Command` 로 데몬을 **직접**
//   띄우는 시험(데몬 `tests/ws_e2e.rs` 등)만 격리된다. 이 모듈의 운영 spawn 경로(WMI Win32_Process.Create)는
//   자식이 WmiPrvSE 자식이라 **부모 env 를 상속하지 않아** 이 override 가 무시된다(설계 확정 — daemon.json/ACL
//   외 채널 없음). 그래서 WMI 를 실제로 타는 smoke 테스트(real_wmi_spawn_*)는 env 로 격리하지 못하고, default
//   경로(`.engram-dev`)를 폴링하며 운영 파일은 백업/복원으로 보호한다.

fn unwritable(dir: &Path, e: &std::io::Error) -> DiscoveryError {
    DiscoveryError::DataDirUnwritable {
        path: dir.display().to_string(),
        reason: e.to_string(),
    }
}

/// 데이터 폴더에 쓸 수 있을지를 **아무것도 만들지 않고** 본다(클라이언트 사전 점검).
///
/// ★폴더를 만들지 않는 것이 요점이다★: 이 검사는 "데몬이 이 폴더를 쓸 수 있을까"를 묻는 것이지 그
/// 폴더를 확정하는 것이 아니다. 검사가 폴더를 만들어 두면, 데몬이 결국 다른 폴더를 쓰게 되는 경로에서
/// 영영 비는 폴더가 남는다.
///
/// 해석:
/// - `dir` 이 이미 폴더면 그 안에 프로브를 쓴다.
/// - `dir` 이 **파일**이면 실패다. 상위를 보고 통과시키면 안 된다 — `create_dir_all` 이 반드시 실패한다.
/// - 없으면 **폴더를 실제로 만들어 보고** 프로브까지 쓴 뒤, 우리가 만든 것이면 되돌린다.
///
/// ★상위 폴더에 파일을 만들어 보는 것으로 대신하지 말 것(되살리지 마라)★: Windows 의
/// `FILE_ADD_FILE` 과 `FILE_ADD_SUBDIRECTORY` 는 **따로 부여된다**. 파일만 만들 수 있는 폴더에서는
/// 그 대체 검사가 통과하고 데몬의 `create_dir_all` 이 실패해, 사용자는 메시지 대신 시간 초과를 본다.
/// 필요한 권한을 그대로 시험해야 한다.
///
/// ★되돌리기의 범위 = 우리가 만든 것 **전부, 그리고 그것만**★: 중간 폴더까지 줄줄이 만들 수 있어 잎
/// 하나만 지우면 나머지가 영구히 남는다. 그래서 위에서 아래로 **한 겹씩** 만들고 생성에 **우리가 이긴**
/// 겹만 기록해 잎부터 위로 되돌린다. 실패한 경로에서도 되돌린다.
///
/// ★"만들기 전 스냅샷"으로 되돌리지 마라(되살리지 마라)★: 없던 조상 목록을 미리 찍어 두면 소유가
/// 성립하지 않는다 — A 가 `a/` 를 없다고 기록한 사이 B 가 `a/` 를 만들면, A 는 나중에 **B 의 폴더**를
/// 지운다. 겹마다 `AlreadyExists` 를 본 쪽이 남의 것이라고 판정해야 그 창이 닫힌다.
///
/// ★계약의 한계 — 지우는 대상은 "우리가 만든 **경로**" 이지 그 순간의 디렉터리 **객체**가 아니다★:
/// 우리가 만든 뒤 남이 그것을 지우고 같은 이름으로 다시 만들면(ABA) 우리의 `remove_dir` 이 남의 것을
/// 지운다. 정밀하게 막으려면 생성 시점의 파일 ID 를 들고 삭제 직전에 대조해야 하는데, 피해가 이만큼
/// 좁아서 하지 않았다: `remove_dir` 은 **빈 디렉터리만** 지우므로 남이 쓰기 시작했으면 실패하고, 그
/// 피해자도 프로브에서 `NotFound` 를 만나 [`retry_if_vanished`] 로 한 번 더 시도해 회복한다.
///
/// 이미 있던 폴더는 손대지 않고, 그 사이 데몬이 쓰기 시작한 폴더는 비어 있지 않아 삭제가 실패하는데
/// 그건 무해하다(이미 쓰이는 폴더다).
///
/// ★겹치는 호출이 실재한다★ — 트레이 "데몬 켜기"와 부팅 ensure 는 직렬화되지 않는다
/// (`src-tauri/src/commands/discovery.rs`). 이 검사는 자기가 만든 폴더를 되돌리므로, 겹치면 남의 프로브가
/// 사라진 폴더에서 `NotFound` 로 넘어진다 — 그래서 [`retry_if_vanished`] 로 감싼다.
pub fn check_data_dir_writable(dir: &Path) -> Result<(), DiscoveryError> {
    retry_if_vanished(|| check_data_dir_writable_once(dir)).map_err(|e| unwritable(dir, &e))
}

fn check_data_dir_writable_once(dir: &Path) -> std::io::Result<()> {
    // ★한 번의 stat 으로 세 갈래를 가른다(되살리지 마라 — `is_dir()` 뒤에 `exists()` 를 잇지 말 것)★:
    //   두 번 물으면 그 사이 남이 폴더를 만들었을 때 "폴더 아님 + 존재함" = **파일이 있다**로 읽혀,
    //   멀쩡한 경합을 "같은 이름의 파일이 이미 있음"으로 잘못 보고한다(겹친 점검 테스트가 실측으로
    //   재현했다). 겹치는 두 주체가 실재한다 — 트레이 "데몬 켜기"와 부팅 ensure.
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => return probe_write_in(dir),
        Ok(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "같은 이름의 파일이 이미 있어 폴더를 만들 수 없음",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }

    // 없는 조상들을 잎→위로 모은다(첫 존재 조상에서 멈춘다 — 루트까지 올라가지 않는다: 드라이브
    //   루트에 대한 create_dir 은 AlreadyExists 가 아니라 PermissionDenied 다, 실측 2026-08-14).
    let mut missing: Vec<&Path> = Vec::new();
    let mut cur = Some(dir);
    while let Some(p) = cur {
        // 상대경로의 마지막 parent 는 빈 경로다 — 만들 수도 지울 수도 없으니 여기서 끊는다.
        if p.as_os_str().is_empty() || p.exists() {
            break;
        }
        missing.push(p);
        cur = p.parent();
    }

    // 위→아래로 한 겹씩. `created` 에는 **우리가 만든** 겹만 담긴다.
    let mut created: Vec<&Path> = Vec::new();
    let mut failed: Option<std::io::Error> = None;
    for p in missing.iter().rev() {
        match std::fs::create_dir(p) {
            Ok(()) => created.push(p),
            // 그 사이 남이 만들었다 = 남의 것 → 우리가 되돌릴 대상이 아니다.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }

    // ★조기 반환하지 마라(되살리지 마라)★: 중간에서 `?` 로 빠져나가면 이미 만든 겹이 그대로 남는다.
    let outcome = match failed {
        Some(e) => Err(e),
        None => probe_write_in(dir),
    };
    for p in created.iter().rev() {
        // 비어 있는 것만 지워진다 — 남이 쓰기 시작한 폴더는 그대로 둔다.
        let _ = std::fs::remove_dir(p);
    }
    outcome
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
    /// ★메시지에 조치가 들어 있다(ADR-0134 결정 4)★: 이 문자열이 그대로 프론트의 실패 표면까지
    /// 올라간다 — 원인 없는 연결 시간 초과를 대신하는 것이 이 변형의 존재 이유다.
    #[error("데이터 폴더에 쓸 수 없음({path}): {reason} — 쓰기 가능한 위치에 압축을 풀어 주세요")]
    DataDirUnwritable { path: String, reason: String },
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

/// 폴링의 시계 — 지금 읽기는 base 시계에서 받고 **스레드를 재우는** 잠을 더한다(이 모듈은 동기다).
// ADR-0275
pub trait Clock: engram_dashboard_base::time::Clock {
    fn sleep(&self, dur: Duration);
}

// ── 순수 오케스트레이션 ────────────────────────────────────────────────────────────

/// info 소유권을 호출자가 유지하도록 참조 기반 판정 — Accept 는 데이터 없이 신호만 준다.
// pub(crate) = daemon_client::stop 전용(3-2 까지) · ADR-0271
pub(crate) enum AcceptCheck {
    Accept,
    DeadPid,
    VersionMismatch { daemon: u32 },
}

// ADR-0196: 생존 확인이 버전 대조보다 **먼저**다 — 뒤집지 마라. 그리고 이 판정은 **버전 방향을 일부러
// 보지 않는다**: 죽은 기록은 우리보다 낮든 높든 stale 이라, 방향 판정을 더해 「더 새 기록이면 물러난다」로
// 고치지 말 것.
// pub(crate) = daemon_client::stop 전용(3-2 까지) · ADR-0271
pub(crate) fn check_acceptable(info: &DaemonInfo, liveness: &dyn PidLiveness) -> AcceptCheck {
    if liveness.is_dead(info.pid, info.start_time) {
        return AcceptCheck::DeadPid;
    }
    if info.protocol_version != PROTOCOL_VERSION {
        return AcceptCheck::VersionMismatch {
            daemon: info.protocol_version,
        };
    }
    AcceptCheck::Accept
}

/// ★클라이언트는 daemon.json 을 지우지 않는다(되살리지 마라 — ADR-0134)★
///
/// 옛 구현은 (a) 에서 stale·깨짐으로 판정한 파일을 spawn 전에 **삭제**했다. 그 삭제는
/// ① 아무것도 벌지 못하고 ② 두 가지 손상을 만든다.
///
/// ① 벌지 못하는 이유: 폴링은 "유효 파싱 + live pid + 버전 호환"만 수락하므로 옛 파일이 남아 있어도
///    이미 죽은 pid 는 `check_acceptable` 이 거른다. 그리고 이긴 데몬은 **잠금을 얻은 뒤에** 자기
///    daemon.json 을 덮어쓴다 — 자리를 미리 비워 줄 필요가 없다.
/// ② 손상: (가) 삭제가 관문(`pre_spawn`)보다 앞서면, 관문이 막았을 때 되돌릴 수 없는 삭제만 남는다.
///    (나) 더 나쁘게, 네트워크 공유의 포터블 폴더(ADR-0134 결정 3이 지원한다고 명시한 구성)에서는
///    **다른 컴퓨터**의 pid 를 로컬 `OpenProcess` 로 판정하게 된다. 남의 살아있는 데몬을 stale 로 읽고
///    그 portfile 을 지우면, 그 데몬은 다시 발행하지 않으므로 **원 소유자가 재접속 불가**가 된다.
///    로컬 pid 검사는 남의 폴더에서 죽음의 증거가 아니다.
///
/// 그래서 이 함수는 **읽기만** 한다. 판정이 틀려도 남의 파일을 부수지 않는 것이 불변식이다.
///
/// `pre_spawn` = spawn 직전에만 도는 관문(ADR-0134 결정 4의 사전 점검 자리).
///
/// ★붙는 자리가 계약이다★: 살아있는 데몬에 attach 하는 경로는 **읽기만** 하면 되므로 이 관문을 지나지
/// 않는다 — 폴더가 잠깐 못 쓰는 상태가 됐다고 이미 잘 도는 데몬에 못 붙으면 그게 더 나쁜 실패다.
#[allow(clippy::too_many_arguments)]
fn ensure_with(
    reader: &dyn DaemonReader,
    spawner: &dyn Spawner,
    liveness: &dyn PidLiveness,
    clock: &dyn Clock,
    exe: &Path,
    pre_spawn: &mut dyn FnMut() -> Result<(), DiscoveryError>,
    timeout: Duration,
) -> Result<DaemonInfo, DiscoveryError> {
    // 안전망: dead 로 판정한 옛 DaemonInfo 를 메모리에 보관한다. 폴링이 timeout 나면(=새 데몬이 안
    // 떴다 = 단일 인스턴스 잠금 충돌로 기존 데몬이 실제 살아있었을 가능성) 이 옛 정보가 지금도 live
    // 인지 재검사해 live 면 복구 반환한다 — 우리 판정이 틀렸을 때의 자가 복구다. 깨진 파일은 내용을
    // 신뢰할 수 없어 보관하지 않는다(None).
    let mut dead_candidate: Option<DaemonInfo> = None;

    // (a) 기존 파일 검사 — **읽기 전용**.
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => return Ok(info),
            AcceptCheck::DeadPid => {
                tracing::warn!(
                    pid = info.pid,
                    daemon_version = info.protocol_version,
                    expected = PROTOCOL_VERSION,
                    "{DAEMON_FILE} 의 pid 가 살아있지 않음 — 그 기록을 stale 로 판정"
                );
                dead_candidate = Some(info);
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
        // 깨진 파일도 지우지 않는다 — 쓰는 도중일 수 있고, 이긴 데몬이 어차피 덮어쓴다.
        Err(DiscoveryError::Parse(_)) => {}
        // ★io 실패를 파싱 실패보다 엄하게 다루지 마라★: 제3자(백신·인덱서)가 파일을 잠깐 좁은 공유로
        //   열면 여기 읽기가 32로 실패하는데, 데몬 쪽은 같은 조건을 5회/400ms 재시도한다. 여기서
        //   하드 실패하면 **멀쩡한 데몬을 두고** ensure 가 통째로 무너진다. "아직 못 읽었다"로 보고
        //   아래 spawn+폴링으로 흘려보낸다 — 이미 떠 있으면 폴링이 그 데몬을 찾고, 없으면 새로 뜬다.
        Err(e) => {
            tracing::warn!("{DAEMON_FILE} 초기 읽기 실패 — 아직 못 읽은 것으로 보고 계속: {e}");
        }
    }

    // (b) spawn — 그 전에 관문 1회.
    pre_spawn()?;
    tracing::info!(exe = %exe.display(), "데몬 spawn");
    spawner.spawn(exe)?;

    // (c) 폴링 — timeout 까지 새 daemon.json 을 기다린다.
    let deadline = clock.now() + timeout;
    // ★"못 읽었다"가 아니라 "마지막에 본 것"을 들고 있는다★: 여기 읽기 실패만 담으면, 끝내 안 고쳐지는
    //   깨진 파일이나 거절당한 파일(죽은 pid·버전 불일치)로 timeout 났을 때 아래 진단이 "파일이 끝내
    //   생기지 않았다"고 **틀린 말**을 한다 — 사용자가 볼 수 있는 유일한 줄이 엉뚱한 곳을 가리킨다.
    let mut last_reason: Option<String> = None;
    loop {
        match reader.read() {
            Ok(Some(info)) => match check_acceptable(&info, liveness) {
                AcceptCheck::Accept => return Ok(info),
                AcceptCheck::DeadPid => {
                    last_reason = Some(format!(
                        "{DAEMON_FILE} 은 있으나 적힌 pid {} 가 살아있지 않음",
                        info.pid
                    ));
                }
                AcceptCheck::VersionMismatch { daemon } => {
                    last_reason = Some(format!(
                        "{DAEMON_FILE} 의 프로토콜 버전 {daemon} ≠ 이 클라이언트 {PROTOCOL_VERSION}"
                    ));
                }
            },
            Ok(None) => {}
            // 쓰는 중 부분 파일일 수 있어 계속 돈다 — 다만 **영영 안 고쳐지는 깨진 파일**도 같은
            //   모양이라, 사유는 들고 간다(위 주석).
            Err(e @ DiscoveryError::Parse(_)) => {
                last_reason = Some(format!("{DAEMON_FILE} 파싱 실패: {e}"));
            }
            // ★루프 안에서 중단하지 마라★: 한 번의 일시적 열기 실패(제3자가 좁은 공유로 잠깐 여는
            //   경우 — 데몬 쪽은 같은 조건을 재시도한다)로 **남은 대기 전부**를 버리게 된다. 다음
            //   tick 에 다시 읽으면 되고, 진짜 못 읽는 상태면 어차피 timeout 으로 귀결된다.
            // ★tick 마다 기본 레벨로 올리지 마라★: 폴링이 같은 줄로 로그를 도배한다. 사유는 마지막
            //   것만 들고 있다가 아래 timeout 에서 한 번 낸다 — 사용자가 실제로 보는 실패는 그 한 번뿐이다.
            Err(e) => {
                tracing::debug!("{DAEMON_FILE} 폴링 읽기 실패 — 다음 tick 에 재시도: {e}");
                last_reason = Some(format!("{DAEMON_FILE} 읽기 실패: {e}"));
            }
        }
        if clock.now() >= deadline {
            if let Some(old) = dead_candidate.take() {
                let old_live = !liveness.is_dead(old.pid, old.start_time);
                if old_live && old.protocol_version == PROTOCOL_VERSION {
                    tracing::warn!(
                        pid = old.pid,
                        "dead 로 판정했던 daemon.json 이 폴링 timeout 시점엔 live — 그 데몬으로 복구"
                    );
                    return Ok(old);
                } else if old_live {
                    // ★원인 없는 timeout 으로 흘려보내지 마라★: 우리가 dead 로 잘못 본 데몬이 실은
                    //   살아있고 버전만 다른 것이므로, 손에 든 protocol_version 을 버리고 Timeout 을
                    //   내면 사용자가 보는 유일한 줄이 원인을 못 가리킨다. 네트워크 공유의 포터블
                    //   폴더에서는 **남의 컴퓨터** pid 를 로컬 OpenProcess 로 재는 탓에 live 데몬이
                    //   항상 dead 로 읽혀 이 경로가 기본값이 된다.
                    tracing::warn!(
                        pid = old.pid,
                        daemon_version = old.protocol_version,
                        expected = PROTOCOL_VERSION,
                        "dead 로 판정했던 daemon.json 이 timeout 시점엔 live — 단 프로토콜 버전이 다름"
                    );
                    return Err(DiscoveryError::VersionMismatch {
                        daemon: old.protocol_version,
                        expected: PROTOCOL_VERSION,
                    });
                }
            }
            // ★기본 레벨(warn)에 반드시 남는다★: 이 timeout 이 사용자가 배너로 보는 그 실패다.
            //   여기가 조용하면 릴리즈 로그 파일에 원인이 한 줄도 안 남아, 배너를 닫는 순간 사고가
            //   추적 불가가 된다(파일 로그를 만든 이유 그 자체).
            tracing::warn!(
                ?timeout,
                last_reason = last_reason
                    .as_deref()
                    .unwrap_or("(없음 — daemon.json 이 끝내 생기지 않음)"),
                "데몬 기동 대기 시간 초과"
            );
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
        path: DataLayout::new(data_dir).daemon_file(),
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
        path: DataLayout::new(data_dir).daemon_file(),
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
            path: DataLayout::new(data_dir).daemon_file(),
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

struct TaskKiller;

// ADR-0266
impl ProcessKiller for TaskKiller {
    fn kill(&self, pid: u32) -> Result<(), DiscoveryError> {
        // 자식 트리까지 끈다 — 데몬 Job 안전망과 겹치나 무해하다.
        engram_dashboard_platform::process::kill_tree(pid).map_err(|e| {
            DiscoveryError::Io(if e.kind() == std::io::ErrorKind::Unsupported {
                "daemon_stop 은 Windows 전용".into()
            } else {
                e.to_string()
            })
        })
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
    let daemon_path = DataLayout::new(data_dir).daemon_file();

    let exe_abs = dunce::canonicalize(daemon_exe)
        .map_err(|e| DiscoveryError::ExeNotFound(format!("{}: {e}", daemon_exe.display())))?;

    let reader = FileReader {
        path: daemon_path.clone(),
    };
    let spawner = WmiSpawner { console };
    let liveness = RealLiveness;
    let clock = engram_dashboard_base::time::SystemClock;
    // ADR-0134 결정 4: 데몬을 띄우기 **전에** 우리 쪽에서 데이터 폴더를 확인한다. 못 쓰는 폴더면
    // 데몬은 뜨자마자 죽고 클라이언트에는 원인 없는 연결 시간 초과만 남으므로, 여기서 원인을 붙여
    // 기존 실패 경로(command Err / DaemonClient Err)로 그대로 올린다.
    //
    // ★override 가 켜져 있으면 단언하지 않는다(load-bearing)★: WMI 로 뜨는 데몬은 **부모 env 를
    // 상속하지 않아**(이 파일 상단 override 주석) `ENGRAM_DATA_DIR` 를 못 본다 — 우리가 보는 폴더와
    // 데몬이 쓸 폴더가 서로 다르다. 그 상태에서 우리 폴더를 검사하면 **엉뚱한 폴더에 대해** 통과/실패를
    // 선언하게 되므로, 아무 말도 하지 않는 쪽이 맞다(폴링 timeout 이라는 기존 동작으로 남는다).
    let mut pre_spawn = || {
        if data_dir_env_override().is_some() {
            tracing::debug!(
                "ENGRAM_DATA_DIR 설정됨 — WMI 데몬은 이 값을 상속하지 않으므로 데이터 폴더 사전 점검을 건너뛴다"
            );
            return Ok(());
        }
        check_data_dir_writable(data_dir)
    };

    ensure_with(
        &reader,
        &spawner,
        &liveness,
        &clock,
        &exe_abs,
        &mut pre_spawn,
        timeout,
    )
}

/// 데몬 exe 경로 탐색. 우선 current_exe 와 같은 디렉토리(배포 시 동거),
/// 없으면 개발용 target/debug fallback. 못 찾으면 ExeNotFound.
pub fn locate_daemon_exe() -> Result<PathBuf, DiscoveryError> {
    const DAEMON_EXE_STEM: &str = "engram-dashboard-daemon";
    let exe = engram_dashboard_platform::env::exe_file_name(DAEMON_EXE_STEM);

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(sibling) = engram_dashboard_platform::env::sibling_exe(DAEMON_EXE_STEM) {
        candidates.push(sibling);
    }
    // 워크스페이스 빌드면 target/debug 가 공유라 위 후보로 충분하나, 안전하게 한 번 더.
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target").join("debug").join(&exe));
        candidates.push(cwd.join("..").join("target").join("debug").join(&exe));
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

// pub(crate) = daemon_client::stop 전용(3-2 까지) · ADR-0271
pub(crate) struct FileReader {
    // pub(crate) = daemon_client::stop 전용(3-2 까지) · ADR-0271
    pub(crate) path: PathBuf,
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

// pub(crate) = daemon_client::stop 전용(3-2 까지) · ADR-0271
pub(crate) struct RealLiveness;

impl PidLiveness for RealLiveness {
    fn is_dead(&self, pid: u32, start_time: u64) -> bool {
        !engram_dashboard_platform::process::pid_alive_with_start_time(pid, start_time)
    }
}

impl Clock for engram_dashboard_base::time::SystemClock {
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

// ── 데몬 띄우기(real) ──────────────────────────────────────────────────────────────

/// 데몬을 셸이 든 Job 밖에서 띄운다(platform `spawn::spawn_outside_job` — WMI). ★왜 Job 밖인가★: 셸(Tauri)이
/// `KILL_ON_JOB_CLOSE` Job 안에 있어도 데몬은 살아남아야 한다. 그 수단은 환경변수를 넘기지 못하므로 토큰은
/// daemon.json(ACL)으로만 흐르고(설계 확정), pid · 포트도 그 파일 폴링으로 회수한다. 받는 exe 는 절대경로다
/// ([`ensure_daemon`] 이 canonicalize 한다).
// ADR-0021
// ADR-0266
struct WmiSpawner {
    /// 참 = 새 콘솔 창과 함께 띄운다(디버그 로그를 보려고) · 거짓 = 기본. ★거짓이어도 디버그 데몬은 콘솔 창이
    /// 뜬다★ — 디버그 빌드는 콘솔 앱이고, 창을 없애는 것은 릴리즈의 `windows_subsystem` 뿐이다.
    console: bool,
}

impl Spawner for WmiSpawner {
    fn spawn(&self, exe: &Path) -> Result<(), DiscoveryError> {
        use engram_dashboard_platform::spawn::{spawn_outside_job, DetachedSpawnError};
        spawn_outside_job(exe, self.console).map_err(|e| match e {
            DetachedSpawnError::Refused { rv } => DiscoveryError::SpawnFailed { rv },
            DetachedSpawnError::Io(e) if e.kind() == std::io::ErrorKind::Unsupported => {
                DiscoveryError::Io("WMI spawn 은 Windows 전용".into())
            }
            DetachedSpawnError::Io(e) => DiscoveryError::Io(e.to_string()),
        })
    }
}
