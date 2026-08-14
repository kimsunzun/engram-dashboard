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
//   - **배포용 경로 커스터마이즈 노브가 아니다.** 배포 단계의 데이터 위치는 실행 폴더 하위로
//     확정돼 있다(ADR-0134). 이 override 를 "사용자가 데이터 폴더를 바꾸는 수단"으로 쓰지 말 것.
//   - ★ADR-0134 이후 부수 효과★: 데이터 폴더가 곧 단일 인스턴스 스코프라, 이 env 만 갈라 주면
//     인스턴스도 함께 갈린다(따로 챙길 열쇠 변수가 없다).
//   - ★중요 한계 — WMI 경로엔 닿지 않는다★: 이 override 는 **부모 env 를 상속하는 spawn 에만** 먹는다.
//     즉 `std::process::Command` 로 데몬을 **직접** 띄우는 ws_e2e.rs 만 격리된다. discovery 의 운영
//     spawn 경로(WMI Win32_Process.Create)는 자식이 WmiPrvSE 자식이라 **부모 env 를 상속하지 않아**
//     이 override 가 무시된다(설계 확정 — daemon.json/ACL 외 채널 없음). 그래서 WMI 를 실제로 타는
//     discovery 의 smoke 테스트(real_wmi_spawn_*)는 env 로 격리하지 못하고, default 경로(`.engram-data`)
//     를 폴링하며 운영 파일은 백업/복원으로 보호한다.

// ADR-0029: debug 분기(walk-up `.engram-data`)와 그 단위테스트에서만 쓰인다 — release
// default_data_dir 은 exe 옆 `data` 만 쓰므로 release 비-test 빌드에선 dead_code.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
const LOCAL_DATA_DIR: &str = ".engram-data";

const DATA_DIR_ENV: &str = "ENGRAM_DATA_DIR";

/// ADR-0134 결정 2: 릴리스 데이터는 실행 폴더 **하위 한 폴더**에 모인다 — exe 옆에 흩어두면 배포
/// 파일과 섞여 새 버전 압축을 덮어쓸 때 사용자 데이터가 함께 날아간다.
///
/// ★`engram-` 접두사를 붙이지 마라(되살리지 마라)★: 이 폴더는 사용자가 이미 이름 붙인 배포 폴더
/// **안에** 있어 접두사가 부모 이름을 되풀이할 뿐이고, 형제인 `prompts/` 는 접두사가 없어 규칙도
/// 어긋난다. 위 [`LOCAL_DATA_DIR`] 과 이름이 다른 것은 실수가 아니다 — 디버그 쪽은 폴더가 많은 repo
/// 루트에 맨몸으로 놓이고 앞의 점이 가려 주므로 접두사가 값을 한다. 대칭을 맞추려 하지 말 것.
/// (사용자 결정 2026-08-14)
const RELEASE_DATA_DIR: &str = "data";

/// 쓰기 프로브 파일 이름의 앞부분. 뒤에 **프로세스·호출마다 다른 꼬리**가 붙는다.
///
/// ★고정 이름을 쓰지 말 것(되살리지 마라)★: 데몬과 클라이언트 관문이 같은 폴더를 동시에 프로브할 수
/// 있고(트레이 "데몬 켜기"와 부팅 ensure 는 직렬화되지 않는다 — `commands/discovery.rs` 참조), 이름이
/// 같으면 진 쪽이 `create_new` 에서 실패해 **멀쩡한 폴더를 "쓰기 불가"로 판정**한다. 더해 삭제만 막는
/// ACL 에서는 남은 파일 하나가 이후 모든 프로브를 영구히 막는다.
const WRITE_PROBE_PREFIX: &str = ".engram-write-probe-";

/// ★0바이트로 쓰지 말 것★: 디스크가 꽉 찼거나 할당량이 소진된 상태에서도 길이 0 파일 생성은 흔히
/// 성공한다 — 그러면 프로브는 통과하고 첫 실제 쓰기가 실패한다. 실제로 바이트를 실어야 검사가 된다.
const WRITE_PROBE_PAYLOAD: &[u8] = b"engram-write-probe";

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
/// 3. **릴리즈(`not(debug_assertions)`)**: **실행 파일 폴더 하위 `data/`**
///    ([`release_data_dir`]). 배포판 폴더를 지우면 흔적이 남지 않는다 — 완전 포터블(ADR-0134 결정 1).
///
/// 어느 경로든 **절대 패닉하지 않는다**(배포·루트 미발견 상황에서도 PathBuf 를 반드시 반환).
pub fn default_data_dir() -> PathBuf {
    if let Some(path) = data_dir_env_override() {
        return path;
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
        // exe 경로는 WMI spawn(부모 cwd 미상속) 아래서도 신뢰 가능한 유일한 기준점이다 — 디버그
        // 분기의 walk-up 이 exe 에서 출발하는 것과 같은 이유.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                return release_data_dir(dir);
            }
        }
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(RELEASE_DATA_DIR)
    }
}

/// `ENGRAM_DATA_DIR` override 가 **활성**이면 그 경로. 빈 값은 미설정과 같다.
///
/// ★단일 출처★: "override 가 켜져 있나"를 묻는 곳이 둘 이상이라 여기로 모은다 — 판정이 갈리면
/// 한쪽은 데몬이 안 쓸 폴더를 보게 된다(`ensure_daemon` 의 사전 점검이 그 자리다).
fn data_dir_env_override() -> Option<PathBuf> {
    let val = std::env::var_os(DATA_DIR_ENV)?;
    if val.is_empty() {
        return None;
    }
    Some(PathBuf::from(val))
}

/// 릴리스 데이터 폴더 = `<exe 폴더>/data`(ADR-0134 결정 1·2).
///
/// ★cfg 를 걸지 않는다(load-bearing)★: 호출부인 [`default_data_dir`] 의 릴리즈 분기는
/// `not(debug_assertions)` 아래에 있고 **테스트는 항상 debug 로 돈다** — 이 함수까지 cfg 로 가리면
/// 릴리스 규칙을 단언할 수단이 사라진다.
pub fn release_data_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join(RELEASE_DATA_DIR)
}

/// ★sync_all 까지 간다★: 캐시에만 얹힌 쓰기는 꽉 찬 디스크·소진된 할당량을 그대로 통과한다 —
/// 바이트가 실제로 안착해야 "쓸 수 있다"가 참이다.
fn write_probe_payload(mut f: std::fs::File) -> std::io::Result<()> {
    use std::io::Write;
    f.write_all(WRITE_PROBE_PAYLOAD)?;
    f.sync_all()
}

fn unwritable(dir: &Path, e: &std::io::Error) -> DiscoveryError {
    DiscoveryError::DataDirUnwritable {
        path: dir.display().to_string(),
        reason: e.to_string(),
    }
}

/// `dir` **안에** 파일을 만들어 바이트를 쓸 수 있는지 실제로 해 본다. `dir` 은 이미 존재해야 한다.
///
/// 프로브 파일은 `create_new` 로 만든다 — 같은 이름의 기존 파일을 절대 덮어쓰지 않는다. 이미 있으면
/// 지난 번 정리가 실패한 흔적이므로 지우고 한 번 더 시도한다(존재 자체는 실패 사유가 아니다).
///
/// ★남길 수 있다(계약)★: 만든 파일은 지우지만, 생성은 되고 삭제는 막는 폴더에서는 **삭제가 실패해
/// 파일이 남는다**. 그때는 경고 로그를 남기고 성공으로 본다 — 쓸 수 있다는 것은 이미 증명됐다.
/// 이 프로세스·이 호출만의 프로브 경로. pid 로 프로세스를, 카운터로 같은 프로세스의 동시 호출을 가른다.
fn probe_path(dir: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{WRITE_PROBE_PREFIX}{}-{n}", std::process::id()))
}

fn probe_write_in(dir: &Path) -> std::io::Result<()> {
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

/// 폴더가 검사 도중 사라지면 **한 번만** 다시 해 본다.
///
/// ★왜 필요한가(실재하는 경합)★: 사전 점검([`check_data_dir_writable`])은 자기가 만든 폴더를 되돌리므로,
/// 두 호출이 겹치면 A 가 만든 폴더를 B 가 "있다"고 본 직후 A 가 지워 B 의 프로브가 `NotFound` 로
/// 넘어진다. 그건 권한 문제가 아니라 타이밍이라 멀쩡한 폴더를 "쓰기 불가"로 판정하면 안 된다.
/// 트레이 "데몬 켜기"와 부팅 ensure 는 직렬화되지 않는다(`src-tauri/src/commands/discovery.rs`).
fn retry_if_vanished(
    dir: &Path,
    mut once: impl FnMut() -> std::io::Result<()>,
) -> Result<(), DiscoveryError> {
    match once() {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            once().map_err(|e| unwritable(dir, &e))
        }
        Err(e) => Err(unwritable(dir, &e)),
    }
}

/// 데이터 폴더를 **만들고** 실제로 쓸 수 있는지 확인한다(ADR-0134 결정 4 — 폴백 없음).
///
/// ★폴더를 소유하는 쪽(데몬) 전용★: 폴더를 생성하는 부수효과가 있다. 아직 그 폴더를 쓸지 확정하지
/// 않은 쪽(클라이언트 사전 점검)은 [`check_data_dir_writable`] 을 쓴다.
///
/// 존재 검사만으로는 부족하다: 권한 없는 위치(`C:\Program Files` 하위 등)는 폴더가 이미 있어도 첫
/// 쓰기에서 막힌다.
///
/// 실패는 [`DiscoveryError::DataDirUnwritable`] 하나로 접는다 — 원인이 무엇이든 사용자가 할 일은
/// "쓸 수 있는 곳에 풀기" 하나다.
pub fn ensure_data_dir_writable(dir: &Path) -> Result<(), DiscoveryError> {
    retry_if_vanished(dir, || {
        std::fs::create_dir_all(dir)?;
        probe_write_in(dir)
    })
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
pub fn check_data_dir_writable(dir: &Path) -> Result<(), DiscoveryError> {
    retry_if_vanished(dir, || check_data_dir_writable_once(dir))
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
            // ★루프 안에서 중단하지 마라★: 한 번의 일시적 열기 실패(제3자가 좁은 공유로 잠깐 여는
            //   경우 — 데몬 쪽은 같은 조건을 재시도한다)로 **남은 대기 전부**를 버리게 된다. 다음
            //   tick 에 다시 읽으면 되고, 진짜 못 읽는 상태면 어차피 timeout 으로 귀결된다.
            Err(e) => {
                tracing::debug!("{DAEMON_FILE} 폴링 읽기 실패 — 다음 tick 에 재시도: {e}");
            }
        }
        if clock.now() >= deadline {
            if let Some(old) = dead_candidate.take() {
                if !liveness.is_dead(old.pid, old.start_time)
                    && old.protocol_version == PROTOCOL_VERSION
                {
                    tracing::warn!(
                        pid = old.pid,
                        "dead 로 판정했던 daemon.json 이 폴링 timeout 시점엔 live — 그 데몬으로 복구"
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
        let want = std::env::temp_dir().join("engram-override-data-dir-test");
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

    /// release 분기는 `not(debug_assertions)` 라 테스트(항상 debug)가 직접 못 탄다 — 그래서 규칙을
    /// 담은 순수 헬퍼를 cfg 없이 두고 여기서 단언한다(ADR-0134).
    #[test]
    fn release_data_dir_is_exe_adjacent() {
        let exe_dir = Path::new("C:\\portable\\engram");
        assert_eq!(
            release_data_dir(exe_dir),
            exe_dir.join("data"),
            "릴리스 데이터 폴더는 exe 폴더 하위 data(`engram-` 접두사 없음 — 상수 주석 참조)"
        );
    }

    // ── 데이터 폴더 쓰기 가능 프로브(ADR-0134 결정 4) ─────────────────────────────────

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

    /// 폴더 안에 프로브 잔여물이 하나라도 있나(이름이 호출마다 달라 접두사로 센다).
    fn probe_leftovers(dir: &Path) -> usize {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(WRITE_PROBE_PREFIX)
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn writable_probe_accepts_temp_dir_and_leaves_nothing() {
        let dir = fresh_probe_dir("ok");
        ensure_data_dir_writable(&dir).expect("temp 하위는 쓸 수 있어야");
        let left = probe_leftovers(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(left, 0, "프로브 파일을 남기면 안 됨");
    }

    #[test]
    fn writable_probe_rejects_child_of_a_file() {
        // 파일의 자식 경로는 만들 수 없다 — 폴더 생성 자체가 막히는 경우의 대표.
        let file = fresh_probe_dir("blocker");
        std::fs::create_dir_all(file.parent().unwrap()).ok();
        std::fs::write(&file, b"x").expect("blocker 파일 생성");
        let err = ensure_data_dir_writable(&file.join("child")).unwrap_err();
        let _ = std::fs::remove_file(&file);
        assert!(
            matches!(err, DiscoveryError::DataDirUnwritable { .. }),
            "{err:?}"
        );
        assert!(
            err.to_string().contains("쓰기 가능한 위치"),
            "사용자가 할 일이 메시지에 있어야: {err}"
        );
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
            std::io::ErrorKind::NotFound,
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

    /// ★실재하는 경합을 겨눈다★: 사전 점검은 **자기가 만든 폴더를 되돌리므로**, 두 호출이 겹치면
    /// A 가 만든 폴더를 B 가 "있다"고 본 직후 A 가 지워 B 의 프로브가 NotFound 로 넘어진다. 그건
    /// 타이밍이지 권한이 아니라서 멀쩡한 폴더를 "쓰기 불가"로 판정하면 안 된다.
    /// 트레이 "데몬 켜기"와 부팅 ensure 는 직렬화되지 않는다(commands/discovery.rs).
    ///
    /// ★대상 폴더를 미리 만들지 말 것★: 만들어 두면 아무도 되돌리지 않아 위 인터리빙이 아예 일어나지
    /// 않는다(그러면 이 테스트는 통과해도 아무것도 증명하지 못한다).
    #[test]
    fn concurrent_checks_racing_on_folder_creation_do_not_produce_a_false_failure() {
        let parent = fresh_probe_dir("concurrent");
        std::fs::create_dir_all(&parent).expect("상위 폴더 생성");
        let target = parent.join("data");

        // 사전 점검(만들고 되돌림)과 데몬 경로(만들고 유지)를 섞는다 — 실제로 겹치는 두 주체다.
        let results: Vec<Result<(), DiscoveryError>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..12)
                .map(|i| {
                    let target = &target;
                    s.spawn(move || {
                        if i % 3 == 0 {
                            ensure_data_dir_writable(target)
                        } else {
                            check_data_dir_writable(target)
                        }
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        let left = probe_leftovers(&parent) + probe_leftovers(&target);
        let _ = std::fs::remove_dir_all(&parent);
        assert!(
            results.iter().all(|r| r.is_ok()),
            "겹친 점검이 서로를 실패시키면 안 됨: {results:?}"
        );
        assert_eq!(left, 0, "겹쳐도 프로브 잔여물은 없어야");
    }

    /// ★사전 점검은 폴더를 남기지 않는다 — 중간 폴더까지★: `create_dir_all` 은 줄줄이 만들 수 있어
    /// 잎 하나만 지우면 나머지가 영구히 남는다. 그래서 **없던 조상들을 만들게 하는** 깊은 경로로 본다
    /// (바로 위 폴더를 미리 만들어 두면 이 손상 모드가 재현되지 않는다).
    #[test]
    fn check_writable_leaves_no_folder_behind_including_intermediates() {
        let root = fresh_probe_dir("nocreate");
        std::fs::create_dir_all(&root).expect("루트 폴더 생성");
        let target = root.join("a").join("b").join("data");

        check_data_dir_writable(&target).expect("만들 수 있으면 통과");

        let leaf = target.exists();
        let mid_b = root.join("a").join("b").exists();
        let mid_a = root.join("a").exists();
        let left = probe_leftovers(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert!(!leaf, "대상 폴더를 남기면 안 됨");
        assert!(
            !mid_b && !mid_a,
            "중간 폴더도 되돌려야(a={mid_a}, a/b={mid_b})"
        );
        assert_eq!(left, 0, "프로브도 남기면 안 됨");
        assert!(root.parent().is_some(), "루트 자체는 손대지 않는다");
    }

    /// ★중간에서 실패해도 되돌린다★: 조기 반환으로 빠져나가면 이미 만든 겹이 그대로 남는다.
    /// 마지막 겹만 실패하게 만들려고 Windows 가 거부하는 이름을 쓴다(`?` — `ERROR_INVALID_NAME`).
    #[cfg(windows)]
    #[test]
    fn check_writable_unwinds_what_it_created_even_when_creation_fails_midway() {
        let root = fresh_probe_dir("partial-unwind");
        std::fs::create_dir_all(&root).expect("루트 폴더 생성");
        let mid = root.join("a");
        let target = mid.join("b?");

        let err = check_data_dir_writable(&target).unwrap_err();

        let mid_left = mid.exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            matches!(err, DiscoveryError::DataDirUnwritable { .. }),
            "만들 수 없는 이름은 실패여야: {err:?}"
        );
        assert!(
            !mid_left,
            "실패 경로에서도 우리가 만든 중간 폴더를 되돌려야"
        );
    }

    /// 이미 있던 **중간** 폴더는 되돌리지 않는다 — 우리가 이긴 겹만 기록한다는 계약의 관측 가능한 절반
    /// (남이 먼저 만든 겹을 지우지 않는다는 쪽은 경합이라 `concurrent_checks_...` 가 확률적으로 훑는다).
    #[test]
    fn check_writable_keeps_intermediate_folders_it_did_not_create() {
        let root = fresh_probe_dir("keep-mid");
        let mid = root.join("a");
        std::fs::create_dir_all(&mid).expect("중간 폴더까지 미리 생성");
        let target = mid.join("b").join("data");

        check_data_dir_writable(&target).expect("만들 수 있으면 통과");

        let mid_left = mid.exists();
        let ours_left = mid.join("b").exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(mid_left, "이미 있던 중간 폴더는 남아야");
        assert!(!ours_left, "우리가 만든 겹은 되돌려야");
    }

    /// 이미 있던 폴더는 되돌리지 않는다 — 우리가 만든 것만 되돌린다는 계약의 반대편.
    #[test]
    fn check_writable_does_not_remove_pre_existing_folders() {
        let root = fresh_probe_dir("preexisting");
        let target = root.join("data");
        std::fs::create_dir_all(&target).expect("대상까지 미리 생성");
        check_data_dir_writable(&target).expect("통과");
        let survived = target.exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(survived, "이미 있던 폴더를 검사가 지우면 안 됨");
    }

    /// ★R10★: 대상 경로가 **파일**이면 `create_dir_all` 이 반드시 실패한다 — 상위를 보고 통과시키면
    /// 사용자는 메시지 대신 시간 초과를 본다.
    #[test]
    fn check_writable_rejects_a_path_that_exists_as_a_file() {
        let parent = fresh_probe_dir("asfile");
        std::fs::create_dir_all(&parent).expect("상위 폴더 생성");
        let target = parent.join("data");
        std::fs::write(&target, b"not a folder").expect("같은 이름 파일 생성");
        let err = check_data_dir_writable(&target).unwrap_err();
        let _ = std::fs::remove_dir_all(&parent);
        assert!(
            matches!(err, DiscoveryError::DataDirUnwritable { .. }),
            "같은 이름의 파일이 있으면 실패여야: {err:?}"
        );
    }

    #[test]
    fn check_writable_fails_when_the_folder_cannot_be_created() {
        let blocker = fresh_probe_dir("blocked");
        std::fs::create_dir_all(blocker.parent().unwrap()).ok();
        std::fs::write(&blocker, b"x").expect("blocker 파일 생성");
        let err = check_data_dir_writable(&blocker.join("a").join("b")).unwrap_err();
        let _ = std::fs::remove_file(&blocker);
        assert!(
            matches!(err, DiscoveryError::DataDirUnwritable { .. }),
            "{err:?}"
        );
    }

    /// ★부분 기록 = 아직 준비 안 됨(ADR-0135)★: 데몬이 daemon.json 을 **제자리에** 쓰므로(원자적 교체
    /// 불가 — 데몬이 그 파일을 붙잡고 있다) 클라이언트가 반쯤 쓰인 내용을 실제로 볼 수 있다. 그때 하드
    /// 실패가 아니라 `Parse` 로 갈려야 폴링이 계속된다(`ensure_with` 의 (c) 갈래가 `Parse` 만 무시한다).
    #[test]
    fn real_reader_treats_a_partially_written_file_as_not_ready() {
        let dir = fresh_probe_dir("partial-json");
        std::fs::create_dir_all(&dir).expect("폴더 생성");
        let path = dir.join(DAEMON_FILE);
        let full = serde_json::to_vec_pretty(&info(1234, PROTOCOL_VERSION)).expect("직렬화");

        std::fs::write(&path, &full[..full.len() / 2]).expect("절반만 기록");
        let partial = FileReader { path: path.clone() }.read();
        // 길이를 0으로 줄인 직후의 창.
        std::fs::write(&path, b"").expect("빈 파일");
        let empty = FileReader { path: path.clone() }.read();
        // 온전히 쓰인 뒤에는 그대로 읽힌다 — 위 둘이 "영영 못 읽는다"가 아님을 함께 못 박는다.
        std::fs::write(&path, &full).expect("전체 기록");
        let whole = FileReader { path }.read();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            matches!(partial, Err(DiscoveryError::Parse(_))),
            "부분 파일은 Parse(= 아직 준비 안 됨)여야: {partial:?}"
        );
        assert!(
            matches!(empty, Err(DiscoveryError::Parse(_))),
            "빈 파일도 Parse 여야: {empty:?}"
        );
        assert!(matches!(whole, Ok(Some(_))), "완성되면 읽혀야: {whole:?}");
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

    /// 대부분의 ensure_with 테스트는 spawn 관문을 다루지 않는다 — 통과시킨다.
    fn noop_pre_spawn() -> impl FnMut() -> Result<(), DiscoveryError> {
        || Ok(())
    }

    #[test]
    fn live_existing_file_returns_without_spawn() {
        let reader = FakeReader::new(vec![Ok(Some(info(100, PROTOCOL_VERSION)))]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
            Duration::from_secs(5),
        )
        .expect("live 파일이면 성공");
        assert_eq!(got.pid, 100);
        assert_eq!(spawner.count.get(), 0, "live 면 spawn 금지");
    }

    /// ★attach 는 관문을 지나지 않는다(ADR-0134)★: 폴더가 못 쓰는 상태여도 이미 도는 데몬에는
    /// 붙어야 한다 — 붙는 데는 읽기만 필요하다. 관문이 앞에 서면 잘 도는 데몬을 못 쓰게 만든다.
    #[test]
    fn attach_to_live_daemon_skips_the_pre_spawn_gate() {
        let reader = FakeReader::new(vec![Ok(Some(info(100, PROTOCOL_VERSION)))]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let gate_calls = Cell::new(0usize);
        let mut pre_spawn = || {
            gate_calls.set(gate_calls.get() + 1);
            Err(DiscoveryError::DataDirUnwritable {
                path: "X".into(),
                reason: "테스트".into(),
            })
        };

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut pre_spawn,
            Duration::from_secs(5),
        )
        .expect("live 데몬엔 관문과 무관하게 붙어야");
        assert_eq!(got.pid, 100);
        assert_eq!(gate_calls.get(), 0, "attach 경로는 관문을 부르지 않는다");
    }

    /// 반대 방향: spawn 하러 가는 경로에서는 관문이 서고, 실패하면 spawn 자체가 없다.
    #[test]
    fn pre_spawn_gate_failure_blocks_spawn_and_propagates() {
        let reader = FakeReader::new(vec![Ok(None)]);
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![] };
        let clock = FakeClock::new();
        let mut pre_spawn = || {
            Err(DiscoveryError::DataDirUnwritable {
                path: "X".into(),
                reason: "테스트".into(),
            })
        };

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut pre_spawn,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(
            matches!(err, DiscoveryError::DataDirUnwritable { .. }),
            "{err:?}"
        );
        assert_eq!(spawner.count.get(), 0, "관문이 막으면 spawn 하지 않는다");
    }

    /// ★R3 회귀 방지★: 관문이 막았을 때 남의 portfile 이 살아남아야 한다. 옛 구현은 stale 판정 직후
    /// 파일을 지우고 그 다음에 관문을 돌려서, 관문이 막으면 되돌릴 수 없는 삭제만 남았다.
    /// (네트워크 공유에선 그 파일이 **다른 컴퓨터의 살아있는 데몬**의 것일 수 있다.)
    #[test]
    fn gate_failure_leaves_an_existing_portfile_intact() {
        let dir = fresh_probe_dir("portfile-survives");
        std::fs::create_dir_all(&dir).expect("폴더 생성");
        let path = dir.join(DAEMON_FILE);
        let existing = info(4321, PROTOCOL_VERSION);
        std::fs::write(
            &path,
            serde_json::to_vec(&existing).expect("기존 portfile 직렬화"),
        )
        .expect("기존 portfile 작성");

        let reader = FileReader { path: path.clone() };
        let spawner = CountingSpawner::ok();
        // 로컬 판정이 "죽었다"고 보는 상황 — 옛 구현이 삭제로 넘어가던 바로 그 분기.
        let liveness = FakeLiveness { dead: vec![4321] };
        let clock = FakeClock::new();
        let mut pre_spawn = || {
            Err(DiscoveryError::DataDirUnwritable {
                path: dir.display().to_string(),
                reason: "테스트".into(),
            })
        };

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut pre_spawn,
            Duration::from_millis(50),
        )
        .unwrap_err();

        let survived = path.exists();
        let same = std::fs::read(&path).ok();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            matches!(err, DiscoveryError::DataDirUnwritable { .. }),
            "{err:?}"
        );
        assert!(survived, "관문이 막아도 남의 portfile 을 지우면 안 된다");
        assert_eq!(
            same,
            Some(serde_json::to_vec(&existing).unwrap()),
            "내용까지 그대로여야"
        );
        assert_eq!(spawner.count.get(), 0);
    }

    /// dead 로 판정해도 파일은 그대로 둔다 — 폴링이 새 파일을 보면 되고, 이긴 데몬이 덮어쓴다.
    #[test]
    fn a_dead_looking_portfile_is_never_deleted() {
        let dir = fresh_probe_dir("portfile-kept");
        std::fs::create_dir_all(&dir).expect("폴더 생성");
        let path = dir.join(DAEMON_FILE);
        std::fs::write(
            &path,
            serde_json::to_vec(&info(4321, PROTOCOL_VERSION)).unwrap(),
        )
        .unwrap();

        let reader = FileReader { path: path.clone() };
        let spawner = CountingSpawner::ok();
        let liveness = FakeLiveness { dead: vec![4321] };
        let clock = FakeClock::new();

        let _ = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
            Duration::from_millis(50),
        );

        let survived = path.exists();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(survived, "dead 판정만으로 남의 portfile 을 지우지 않는다");
        assert_eq!(spawner.count.get(), 1, "대신 spawn 은 한다");
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

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
            Duration::from_secs(5),
        )
        .expect("새 데몬 발견 성공");
        assert_eq!(got.pid, 200);
        assert_eq!(spawner.count.get(), 1, "stale 면 spawn 1회");
        assert!(
            reader.calls.get() >= 2,
            "옛 파일을 지우지 않고 폴링으로 새 파일을 본다"
        );
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

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(got.pid, 500);
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

        let err = ensure_with(
            &reader,
            &FailingSpawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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

        let got = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
            Duration::from_millis(150),
        )
        .expect("옛 데몬이 사실 live 면 복구");
        assert_eq!(got.pid, 42, "dead 로 봤던 옛 데몬 정보를 복구");
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

        let err = ensure_with(
            &reader,
            &spawner,
            &liveness,
            &clock,
            Path::new("daemon.exe"),
            &mut noop_pre_spawn(),
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
    // ★기존 데몬이 살아있으면 단일 인스턴스 잠금으로 우리 spawn 이 거부돼 검증이 무의미하므로
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
