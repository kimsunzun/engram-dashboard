//! engram-dashboard-daemon — 라이브러리 표면.
//!
//! `main.rs`(데몬 진입점)와 격리 하네스(`tests/ws_e2e.rs`)가 **같은 기동 흐름**을 공유하도록
//! 서버 조립·accept loop 를 여기로 모았다. main 은 `run()` 한 줄만 부르고, 테스트는
//! `start_test_server()` 로 in-process 서버를 띄워 WS 클라이언트로 검증한다.
//!
//! ★이 crate 의 범위(ADR-0130)★ — 응용 층 + 조립. **데몬 살림의 *구현*은 여기 없다**: 단일 인스턴스
//! 가드와 portfile 의 실물은 슬라이스 1 로 `engram-dashboard-net` 이 가져갔다. 여기 남은 건 그것들을
//! 어느 순서로 부르는지(`run()`)와 응용 층이다. 이름과 내용물의 이 어긋남은 알고 남긴 것 —
//! rename 하지 않는 이유와 재개 조건은 ADR-0130.

pub mod agent_conn;
pub mod connection_core;
pub mod control;
#[cfg(feature = "test-harness")]
pub mod experiment;
pub mod messaging_host;
pub mod status_fanout;
#[cfg(test)]
mod test_doubles;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::{PresetRegistry, PresetStore};
use engram_dashboard_core::agent::profile::{ProfileRegistry, ProfileStore};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::agent::types::CLI_EXE_NAME;
use engram_dashboard_core::logging;
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_protocol::PROTOCOL_VERSION;

use tokio::net::TcpListener;
use tokio::sync::watch;

use connection_core::MultiViewState;
use engram_dashboard_net::frame_port::FrameFanout;
use engram_dashboard_net::ws::ConnRegistry;
use status_fanout::DaemonStatusSink;

// ADR-0129 슬라이스 1: 네트워크 행이 소유한 타입인데 **이 crate 의 공개 시그니처에 나타나므로**
//   재수출한다(`start_test_server_with_keepalive` — `tests/ws_e2e.rs` 가 부른다). 표준 Rust API
//   재수출 패턴이고, **모듈 통째 재수출이 아니다** — 옛 `crate::ws::` 경로를 되살리는 것은 금지다.
pub use engram_dashboard_net::ws::KeepaliveConfig;
// ADR-0129 0-4: 핸드셰이크 프레임(`auth::AuthFrame`)은 **재수출하지 않는다** — 이 crate 의 공개
//   시그니처에 나타나지 않으므로 위 재수출의 사유가 성립하지 않고, 한 타입에 import 경로가 둘이 생긴다.
//   `tests/ws_e2e.rs` 는 네트워크 crate 를 직접 부른다(그 crate 는 여기 normal 의존이라 테스트 타깃에서
//   그대로 보인다) — 경계가 각 사용 지점에서 보이게 두는 슬라이스 1 의 원칙 그대로다(step-log S18.21).

const DAEMON_FILE: &str = "daemon.json";

// ── data dir / 토큰 ──────────────────────────────────────────────────────────────

fn resolve_data_dir() -> PathBuf {
    engram_dashboard_discovery::default_data_dir()
}

/// 보안: 반환값은 로그에 찍지 말 것(daemon.json 에만 기록).
pub fn generate_token() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf)?;
    let mut s = String::with_capacity(64);
    for b in buf {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    Ok(s)
}

// ── ENGRAM_EXE 주입 (설계 §5 · ADR-0014 방향, CLI 이식성) ─────────────────────────────
//
// ★왜 데몬 프로세스 env 에 넣는가★: 스폰된 에이전트(claude)가 다른 에이전트를 조종하려면 CLI exe
// (`engram-dashboard.exe`) 절대경로를 알아야 하는데, 매뉴얼에 하드코딩하면 머신마다 깨진다. 데몬은
// 자기 exe 폴더를 알고 앱 exe 는 그 **형제**다(locate_daemon_exe 와 대칭 — 배포 시 두 exe 동거). 그래서
// 데몬이 부팅 시 자기 env 에 ENGRAM_EXE 를 세팅해 두면, 이후 스폰되는 모든 PTY 자식이 이 env 를
// **상속**한다(portable-pty CommandBuilder::new 는 std::env::vars_os() 로 부모 env 를 시드한다 — 실측).
// 그러면 에이전트 매뉴얼은 `"$ENGRAM_EXE" send DEF "..."` 로 어느 머신에서든 동작한다.
//
// ★왜 core spawn 경로를 안 건드리나★: env 를 CommandSpec/manager.spawn_agent 로 threading 하면 core
// crate 가 "형제 exe 경로" 개념을 알게 돼 tauri-import-0 격리 원칙과 무관하게 관심사가 샌다. 데몬 프로세스
// env 1회 세팅은 자식 전파를 공짜로 얻고 core 를 순수하게 둔다(부수효과는 데몬 부팅 1지점에 국한).
//
// ★best-effort★: 앱 exe 를 못 찾아도(개발 중 부분 빌드 등) 데몬은 계속 뜬다 — env 미세팅이면 에이전트
// CLI 호출만 실패하고, 그건 매뉴얼이 fallback(직접 exe 지정)을 안내하면 된다(데몬 기동을 막지 않는다).

/// ★SAFETY(std::env::set_var)★: 부팅 최초(run 진입 직후, 다른 스레드 spawn 전)에 1회만 호출한다 —
/// 이 시점엔 tokio worker 외 경쟁 스레드가 env 를 동시 읽지 않으므로 data race 위험이 없다.
fn set_engram_exe_env() {
    const APP_EXE: &str = if cfg!(windows) {
        "engram-dashboard.exe"
    } else {
        "engram-dashboard"
    };
    // 이미 세팅돼 있으면(상위가 명시 주입) 존중 — 덮어쓰지 않는다.
    if std::env::var_os("ENGRAM_EXE").is_some() {
        return;
    }
    if let Ok(daemon_exe) = std::env::current_exe() {
        if let Some(dir) = daemon_exe.parent() {
            let app_exe = dir.join(APP_EXE);
            if app_exe.is_file() {
                std::env::set_var("ENGRAM_EXE", &app_exe);
                tracing::info!(path = %app_exe.display(), "ENGRAM_EXE 주입(자식 PTY 상속)");
                return;
            }
        }
    }
    tracing::warn!("ENGRAM_EXE 미주입 — 앱 exe 형제를 못 찾음(에이전트 CLI 호출은 fallback 필요)");
}

// ── 제어 평면 CLI 위치 탐색 (ADR-0086 스텝 2 · F1) ─────────────────────────────────
//
// ★왜 형제 exe 를 찾아야 하나★: **MCP 를 못 쓰는 백엔드**의 에이전트가 다른 에이전트에게 메시지를
// 보내려면 그 CLI(파일명 = `CLI_EXE_NAME` + 플랫폼 확장자)를 shell 로 불러야 하는데, 이 바이너리는
// **PATH 에 없다**(데몬과 함께 배포되는 내부 도구라 bare 이름으로는 shell 이 못 찾는다). 그래서 데몬이 자기 exe 폴더의
// **형제**에서 절대경로를 찾아(set_engram_exe_env·locate_daemon_exe 와 동일 대칭 — 배포 시 세 exe 동거),
// provision 이 그 경로를 ControlEndpoint.send_exe 로 실어 보낸다. backend 는 control endpoint 가 있는 스폰
// **전부**에 그걸 ENGRAM_CLI_EXE·PATH 로 주입한다 — 제어 동사가 전원 개방이라(ADR-0132 결정 5) 우편만 쓰는
// 경로가 아니다.
//
// ★best-effort(fail-open)★: 못 찾아도(개발 중 부분 빌드 등) None 을 돌려주고 데몬은 계속 뜬다 — MCP 가능
// 백엔드(claude)는 제어 CLI 를 잃을 뿐이고, 비-MCP 백엔드 스폰은 우편 입구가 0 이 되므로 provision 에서
// fail-closed 로 막힌다. warn 로그로 원인을 남긴다(관측성).

/// set_engram_exe_env 와 동형이나 여기선 env 를 세팅하지 않고 **경로 값**을 돌려준다(env 주입은
/// backend 소유).
///
/// ★호출자가 하나뿐이어도 지우지 말 것★: 이 값은 **비-MCP 백엔드의 유일한 우편 입구**이자(그쪽은 MCP
/// 입구가 아예 없다) **모든 스폰의 제어 입구**이고, 없으면 비-MCP provision 이 fail-closed 로 스폰을 막는다
/// — 즉 삭제는 dead-code 정리가 아니라 CLI 우편과 제어 평면의 제거다. 이 구간은 테스트가 없어(지우고
/// 호출부에 None 을 넘겨도 전 스위트가 초록) 이 앵커가 유일한 방어선이다 — 뒤 구간(provision→endpoint→
/// 스폰 env)은 daemon control 테스트 `provision_*_wires_the_*_cli_*` 두 짝이 잡는다.
// ADR-0133
fn locate_send_exe() -> Option<PathBuf> {
    // 파일명은 상수에서 파생한다 — 여기 이름을 따로 적으면 배포된 실행파일과 갈릴 수 있고, 갈리면
    //   CLI 입구가 조용히 비활성된다(경고 로그 한 줄 외엔 증상이 없다).
    let file_name = if cfg!(windows) {
        format!("{CLI_EXE_NAME}.exe")
    } else {
        CLI_EXE_NAME.to_string()
    };
    if let Ok(daemon_exe) = std::env::current_exe() {
        if let Some(dir) = daemon_exe.parent() {
            let send_exe = dir.join(&file_name);
            if send_exe.is_file() {
                tracing::info!(path = %send_exe.display(), "제어 평면 CLI 위치 확정(ADR-0086 F1)");
                return Some(send_exe);
            }
        }
    }
    tracing::warn!(
        name = %file_name,
        "제어 평면 CLI 형제 exe 를 못 찾음 — CLI 입구 비활성(MCP 입구는 정상, ADR-0086 F1)"
    );
    None
}

// ── panic hook (B-1) ──────────────────────────────────────────────────────────────

/// ★멱등(테스트 안전)★: 여러 테스트가 run()/이 함수를 반복 호출해도 hook 이 무한 중첩되지
///   않도록 Once 로 1회만 설치한다. 설치된 hook 은 프로세스 수명 동안 유지된다.
fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>");
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            let location = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "<unknown>".to_string());
            tracing::error!(thread = name, location, "스레드 panic: {msg}");
            prev(info);
        }));
    });
}

// ── AgentManager 배선 (src-tauri lib.rs setup 미러) ───────────────────────────────

/// 조립이 한 덩이로 들고 다니는 데몬 배선 — `AgentManager` 와 **그 status sink 이 팬아웃하는 바로 그**
/// 연결 레지스트리를 묶는다.
///
/// ★왜 묶는가(load-bearing — 풀면 조용한 장애가 표현 가능해진다)★: manager 는 자기 안에
///   `DaemonStatusSink` → `Arc<dyn FrameFanout>` 을 **투명하게** 지고 있어서, 둘을 따로 넘기면
///   `build_daemon_wiring(A)` 로 만든 manager 를 `run_accept_loop(.., 레지스트리 B)` 에 짝지어 넣는 것이
///   **컴파일된다**. 그러면 연결은 B 에 등록되고 상태·목록·복원 통지는 A 로 나가 아무 클라에도 닿지
///   않는다 — 빈 맵은 루프 0회라 **실패 로그조차 없고**, 프론트는 목록 이벤트로 terminal 을 판정하므로
///   (ADR-0005) 모든 에이전트가 영원히 살아 있는 것으로 보인다. lease/프로필/프리셋 브로드캐스트도
///   같은 방식으로 사라진다.
/// ★어긋난 짝이 표현 가능한 자리 — **규칙으로 적는다**★. 경로를 열거하면 매번 한 자리씩 모자란다
///   (실제로 두 번 그랬다: 처음엔 `manager` 가, 그다음엔 `handlers` 가 열거를 빠져나갔다). 규칙은:
///   **"레지스트리를 받는 파라미터" 와 "팬아웃을 — 직접이든 전이적이든 — 나르는 파라미터" 를 함께
///   받는 자리마다** 어긋난 짝이 표현 가능하다. `manager`(→ status sink)도 `handlers`(→ 공장 →
///   `AgentConnections.fanout`)도 전이적 운반자다. 그래서 **"팬아웃 타입을 파라미터로 안 받으니
///   안전" 은 판정 기준이 될 수 없다** — 무엇을 나르는지를 봐야 한다.
///   지금 이 규칙에 걸리는 자리는 **조립 밖의 공개 진입점**(`engram_dashboard_net::ws::handle_connection` ·
///   `AgentConnections::new` · `ConnectionCore::new`)과 **네트워크 crate 의** ws 테스트 하네스이고,
///   **조립 안에는 없다** —
///   그게 이 struct 의 존재 이유다. 이 struct 를 손으로 조립해 남의 레지스트리를 끼우는 것도 한 모듈
///   안이라 타입으로 못 막는다.
/// ★그 공개 진입점을 여기서 고치지 않는 이유★: `handle_connection` 은 레지스트리가 정당히 필요하고
///   **공장이 어떻게 조립됐는지는 알아선 안 된다** — 레지스트리에서 공장을 파생시키면 층이 뒤집힌다.
///   정공법은 슬라이스 3의 투영(`impl ConnRegistry { pub fn fanout(&self) -> Arc<dyn FrameFanout> }`)
///   이다 — **그 슬라이스는 ADR-0130 으로 보류됐으므로 예정 작업이 아니라 재개 시의 처방이다**:
///   팬아웃을 레지스트리에서만 얻게
///   만들면 **모든** 호출 지점에서 맞는 짝이 곧 발견 가능한 짝이 된다 — `handle_connection` 도 포함.
// ADR-0129
struct DaemonWiring {
    manager: Arc<AgentManager>,
    /// 연결이 등록되는 맵. `manager` 안의 status sink 가 팬아웃하는 그 맵과 **같은 것**이다
    /// (`ConnRegistry` 는 내부가 Arc — clone 이 같은 맵을 본다).
    registry: ConnRegistry,
}

impl DaemonWiring {
    /// 제어 라우트(`/control/agent`)가 명부 변경을 알릴 때 쓰는 팬아웃 어댑터(ADR-0132).
    ///
    /// ★필드를 밖으로 풀지 않고 **여기서** 만든다★: 팬아웃과 매니저를 따로 꺼내 조립하면 위 struct 주석이
    ///   막으려는 어긋난 짝(다른 조립의 레지스트리 + 이 매니저)이 다시 표현 가능해진다. 이 메서드는 제 필드
    ///   둘만 쓰므로 그 조합이 구조적으로 불가능하다.
    fn roster_broadcast(&self) -> Arc<dyn control::agent::RosterBroadcast> {
        Arc::new(connection_core::RosterFanout::new(
            Arc::new(self.registry.clone()),
            self.manager.clone(),
        ))
    }
}

fn build_daemon_wiring(
    data_dir: &std::path::Path,
    control: Arc<dyn engram_dashboard_core::agent::types::ControlChannel>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<messaging_host::FlushMsg>,
    idle_coalescer: Arc<messaging_host::IdleCoalescer>,
) -> DaemonWiring {
    let profile_store = Arc::new(FileProfileStore::new(data_dir.to_path_buf()));
    let preset_store = Arc::new(FilePresetStore::new(data_dir.to_path_buf()));
    build_daemon_wiring_with_store(
        profile_store,
        preset_store,
        control,
        flush_tx,
        idle_coalescer,
    )
}

/// build_daemon_wiring 의 store 주입형 — 테스트가 in-memory store 를 끼워 디스크/Embedded 와 격리한다.
fn build_daemon_wiring_with_store(
    store: Arc<dyn ProfileStore>,
    preset_store: Arc<dyn PresetStore>,
    control: Arc<dyn engram_dashboard_core::agent::types::ControlChannel>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<messaging_host::FlushMsg>,
    idle_coalescer: Arc<messaging_host::IdleCoalescer>,
) -> DaemonWiring {
    // ADR-0129: 운영 조립에서 연결 레지스트리와 그 팬아웃 면(面)을 만드는 **유일한 자리**다 — 포트화(dyn)
    //   이전엔 concrete `ConnRegistry` 를 받아 짝 불일치가 타입상 불가능했고, 그 보장을 산문으로 격하시키지
    //   않으려 생성을 한 자리로 모았다(ADR-0129 결정 3 — 조립 행만 두 행을 다 안다).
    let registry = ConnRegistry::new();
    let fanout: Arc<dyn FrameFanout> = Arc::new(registry.clone());
    let status_sink = Arc::new(messaging_host::MessagingFlushSink::new(
        DaemonStatusSink::new(fanout),
        flush_tx,
        idle_coalescer,
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let presets = Arc::new(PresetRegistry::new(preset_store));

    let profiles_cb = profiles.clone();
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig::default(),
        Arc::new(move |agent_id, new_sid| {
            profiles_cb.observe_session_id(agent_id, new_sid);
        }),
    ));
    tracker.start();

    let manager = Arc::new(AgentManager::new_with_control(
        status_sink,
        profiles,
        presets,
        tracker,
        control,
    ));
    DaemonWiring { manager, registry }
}

// ── accept loop (main + 테스트 공유) ──────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_accept_loop(
    listener: TcpListener,
    wiring: DaemonWiring,
    multiview: MultiViewState,
    control_registry: Arc<control::registry::ControlRegistry>,
    messaging_slot: Arc<control::mcp_server::MessagingSlot>,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
    enable_ctrl_c: bool,
    keepalive: KeepaliveConfig,
) {
    // ★팬아웃 포트를 인자로 받지 않고 여기서 뽑는 이유(ADR-0129)★: 아래 accept 갈래가 등록하는 **그 값**
    //   에서 뽑으므로 "브로드캐스트 대상 ≠ 등록 대상" 을 이 함수 안에서는 쓸 수가 없다. 조립 밖에서 여전히
    //   표현 가능한 자리와 그 판정 규칙은 `DaemonWiring` 주석에 있다.
    let DaemonWiring { manager, registry } = wiring;
    let fanout: Arc<dyn FrameFanout> = Arc::new(registry.clone());
    let handlers: Arc<dyn engram_dashboard_net::frame_port::ConnectionHandlerFactory> =
        Arc::new(agent_conn::AgentConnections::new(
            manager,
            multiview,
            fanout,
            control_registry,
            messaging_slot,
            shutdown_tx,
        ));

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        tracing::debug!(%peer, "연결 수락 — WS 핸들러로 넘김");
                        let registry = registry.clone();
                        let handlers = handlers.clone();
                        let expected_token = expected_token.clone();
                        tokio::spawn(async move {
                            engram_dashboard_net::ws::handle_connection(
                                stream,
                                peer,
                                registry,
                                handlers,
                                expected_token,
                                PROTOCOL_VERSION,
                                keepalive,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        tracing::warn!("accept 실패: {e}");
                    }
                }
            }
            // StopDaemon 명령 수신 — watch 가 true 로 바뀌면 종료.
            res = shutdown_rx.changed() => {
                match res {
                    Ok(()) if *shutdown_rx.borrow() => {
                        tracing::info!("종료 신호(watch=true) 수신 — accept loop 탈출");
                        break;
                    }
                    Ok(()) => {} // false 로의 변경은 무시(현재 발생 안 함)
                    Err(_) => break, // 모든 sender drop — 종료
                }
            }
            _ = tokio::signal::ctrl_c(), if enable_ctrl_c => {
                tracing::info!("Ctrl-C 수신 — accept loop 탈출");
                break;
            }
        }
    }
}

// ── main 본체 (운영) ──────────────────────────────────────────────────────────────

/// 데몬 본체. 반환 Err(code) 면 호출자(main)가 그 코드로 exit. 정상 종료(이미 실행 중 포함)는 Ok.
pub async fn run() -> Result<(), i32> {
    // 0) ★마스킹은 미포함★ — init_logging 은 키를 가리지 않는다. mask_secrets 는 헬퍼만 제공하고
    //    적용은 호출자 책임이다(민감 출력 로깅 시 명시 적용). 근거: docs/reference/logging-conventions.md.
    logging::init_logging();

    // 0.5) panic hook 설치(B-1). 데몬 내부 스레드(pump 등)가 panic 하면 silent 정지로
    //   넘어가기 쉬우므로(§5 "죽음 감지는 백엔드가 판단") 가시화한다. ★데몬 전체는 죽이지 않는다★ —
    //   연결 task panic 은 tokio 가 이미 격리하고, pump panic 은 B-2 가 Failed 로 전이시킨다.
    install_panic_hook();

    // 0.6) ENGRAM_EXE 주입 — ★반드시 에이전트 spawn 전★(이 env 를 세팅한 뒤에야 spawn_agent 의 PTY
    //   자식이 상속한다). 이 위치가 set_engram_exe_env SAFETY 주석이 요구하는 "부팅 최초 1회" 다.
    set_engram_exe_env();

    // 1) 단일 인스턴스 가드.
    //    ★_guard 는 프로세스 수명 동안 살아 있어야 한다★(Drop 시 mutex 해제 = 단일성 깨짐).
    let _guard = match engram_dashboard_net::instance::acquire() {
        Ok(Some(g)) => g,
        Ok(None) => {
            tracing::info!("데몬이 이미 실행 중 — 종료");
            return Ok(());
        }
        Err(e) => {
            tracing::error!("단일 인스턴스 가드 획득 실패: {e}");
            return Err(1);
        }
    };

    // 2) data_dir 결정 + 생성.
    let data_dir = resolve_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        tracing::error!("data_dir 생성 실패({:?}): {e}", data_dir);
        return Err(1);
    }
    let daemon_path = data_dir.join(DAEMON_FILE);

    // 2.5) 기존 daemon.json stale 검사.
    if let Some(prev) = engram_dashboard_net::portfile::read(&daemon_path) {
        if engram_dashboard_net::portfile::is_stale(&prev) {
            tracing::info!(pid = prev.pid, "기존 daemon.json 이 stale — 덮어씀");
        } else {
            tracing::warn!(
                pid = prev.pid,
                "기존 daemon.json 의 PID 가 살아있음 — 덮어쓰지 않고 종료(살아있는 데몬 보호)"
            );
            return Ok(());
        }
    }

    // 3) bind → 실제 포트 취득.
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("TcpListener bind 실패: {e}");
            return Err(1);
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            tracing::error!("local_addr 조회 실패: {e}");
            return Err(1);
        }
    };

    // 4) WS auth 토큰 생성.
    let token = match generate_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("토큰 생성 실패: {e}");
            return Err(1);
        }
    };

    // ADR-0086: 제어 채널 토큰 레지스트리. **위 4)의 daemon.json WS 토큰과는 완전히 다른 관심사다**
    //   (혼용 금지 — ADR-0086 §맥락).
    let control_registry = Arc::new(control::registry::ControlRegistry::new());

    // 5b) 멀티뷰어 협상 상태 — 전 연결이 공유한다.
    let multiview = MultiViewState::new();

    // 5b.5) 부팅 스윕(FIX 5). ★반드시 MCP 서버·provision 시작 전★: `control_registry` 를 방금 빈
    //   상태로 만들었으므로(위) 이 시점의 모든 기존 mcp-config 는 dead credential 이다.
    control::mcp_config::sweep_stale_configs(&data_dir);

    // 5c) 제어 채널 MCP 서버 기동.
    //     ★fail-closed(FIX 1)★: bind/start 실패는 **치명**이다 — 데몬을 NoopControlChannel 로 조용히
    //     계속 띄우면(옛 동작) 제어 채널 없이 도는데도 health 를 위장한다. 대신 이미 만든 자원(WS
    //     listener·control_registry)을 drop 하고 Err(1) 로 데몬 시작을 중단한다. 데몬은 자기 제어
    //     엔드포인트 없이는 뜨지 않는다(에이전트 오케스트레이션이 §5 LLM-우선 제어의 근간이라, 그게
    //     없는 반쪽 데몬은 정상 상태가 아니다). ★반드시 에이전트 spawn 전★.
    let manager_slot = Arc::new(control::mcp_server::ManagerSlot::new());
    let messaging_slot = Arc::new(control::mcp_server::MessagingSlot::new());
    let roster_broadcast_slot = Arc::new(control::mcp_server::RosterBroadcastSlot::new());
    let (flush_tx, flush_rx) = tokio::sync::mpsc::unbounded_channel::<messaging_host::FlushMsg>();
    let idle_coalescer = Arc::new(messaging_host::IdleCoalescer::new());

    // ★핸들은 프로세스 수명 동안 살아 있어야 한다★(drop = 서버 종료).
    let (control, mut mcp_server_handle): (
        Arc<dyn engram_dashboard_core::agent::types::ControlChannel>,
        Option<control::mcp_server::McpServerHandle>,
    ) = match control::mcp_server::start_mcp_server(
        control_registry.clone(),
        manager_slot.clone(),
        messaging_slot.clone(),
        roster_broadcast_slot.clone(),
    )
    .await
    {
        Ok(handle) => {
            let url = handle.url.clone();
            let send_exe = locate_send_exe();
            let priming: Arc<dyn control::priming::PrimingProvider> =
                Arc::new(control::priming::FilePrimingProvider::from_install_root());
            let channel = Arc::new(control::DaemonControlChannel::new(
                control_registry.clone(),
                url,
                data_dir.clone(),
                send_exe,
                priming,
            ));
            (channel, Some(handle))
        }
        Err(e) => {
            // fail-closed 정리는 이게 전부다 — ★연결 레지스트리는 아직 없고★(아래 6단계
            //   `build_daemon_wiring` 이 만든다), daemon.json 도 아직 안 썼다(아래 8단계 — 남는
            //   stale portfile 없음).
            tracing::error!(
                "MCP 서버 기동 실패 — 제어 채널 없이는 데몬을 띄우지 않는다(fail-closed): {e}"
            );
            drop(listener);
            return Err(1);
        }
    };

    // 6) AgentManager 배선.
    let wiring = build_daemon_wiring(&data_dir, control, flush_tx.clone(), idle_coalescer.clone());
    // ★레지스트리는 `wiring` 이 계속 들고 있다가 accept loop 로 함께 넘어간다★ — 여기서 풀어 두면
    //   짝을 어긋나게 넘길 여지가 생긴다(ADR-0129 `DaemonWiring`).
    let manager = wiring.manager.clone();
    manager_slot.set(manager.clone());
    // 제어 라우트가 바꾼 명부를 붙어 있는 클라이언트가 보게 하는 배선(ADR-0132). ★이 한 줄이 빠지면★
    //   `engram agent rename/new/move` 가 성공해도 대시보드 트리는 무관한 이벤트가 올 때까지 옛 명부를
    //   보여 준다(에러도 로그도 없다).
    roster_broadcast_slot.set(wiring.roster_broadcast());

    // 6.4) idle 게이트 조립 — ★턴 관측 자체는 코어가 출력 pump 에서 직접 적재하므로 여기서 배선할
    //    것이 없다★.
    let idle_notifier = Arc::new(messaging_host::ChannelIdleNotifier::new(
        flush_tx,
        idle_coalescer.clone(),
    ));
    let busy = Arc::new(messaging_host::busy_gate_for_manager(
        manager.clone(),
        idle_notifier.clone(),
    ));

    // 6.5) MessagingService 조립.
    let messaging = Arc::new(
        messaging_host::messaging_for_manager_gated(
            manager.clone(),
            control_registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier.clone()),
    );
    messaging_slot.set(messaging.clone());

    // 6.6) 유지보수 sweep task(데몬 수명 동안 도는 long-lived tokio task).
    //    ★주기 = 60s(내부 선택 — 보고)★: TTL(24h)에 비해 촘촘해 만료가 크게 지연되지 않고, 극저
    //    메시지율이라 부하가 무의미하다.
    //    ★`spawn_blocking` 없이 async task 로 두는 근거★: 두 sweep 어느 쪽도 자식 stdin blocking write 를
    //    하지 않는다(실제 주입은 도어벨을 받은 flush 레인 몫 — service.rs `deliver_notice` 주석). 그래서
    //    abort 도 즉시 먹는다.
    //    ★reply_by 하한과의 결합★: 기한 초과 판정 해상도가 곧 이 주기다 — ingress 의 `MIN_REPLY_BY_SECS`
    //    (1분)가 이 값과 짝이므로, 주기를 바꾸면 그 하한도 함께 봐야 한다.
    const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
    let sweep_messaging = messaging.clone();
    let sweep_busy = busy.clone();
    let sweep_task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        // 첫 tick 즉발을 건너뛴다(부팅 직후 파킹 없음 — 첫 주기 뒤부터 sweep).
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = std::time::Instant::now();
            // ★틱 본체 패닉 격리(리뷰 fix 9 · load-bearing)★: 이 루프가 죽으면 **두 안전장치가 동시에**
            //   멈춘다 — 파킹 TTL 만료 처리와 busy 상한 fail-open(멈춘 턴 깨우기). 즉 한 번의 패닉이
            //   "그 뒤로 아무도 배달을 못 받는" 조용한 정지로 번진다. 그래서 틱 본체를 unwind 경계로 감싸
            //   경고만 남기고 다음 틱을 계속 돈다.
            //   ★release 빌드는 panic=abort 라 여기 Err 갈래가 아예 도달 불가하다★(워크스페이스 Cargo.toml
            //   `[profile.release] panic = "abort"` — 패닉하면 프로세스가 즉시 죽는다). 이 가드가 실제로
            //   의미 있는 건 **debug/테스트 빌드**이고, 거기서 조용한 정지를 막는 게 목적이다.
            let ticked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                sweep_messaging.sweep(now);
                sweep_busy.sweep_stale_busy(now);
            }));
            if ticked.is_err() {
                tracing::warn!(
                    "sweep 틱이 패닉했다 — 이번 틱만 건너뛰고 계속(다음 주기에 재시도). TTL 만료·busy fail-open 이 함께 멈추지 않게 하는 격리(debug 빌드 한정 — release 는 panic=abort)"
                );
            }
        }
    });

    // 6.7) flush worker task(sweep task 옆 — 종료 시 abort).
    let flush_worker = messaging_host::spawn_flush_worker(
        flush_rx,
        messaging_host::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer.clone(),
        },
    );

    // 7) auth 비교용 토큰을 Arc 로 보관(daemon.json 에 token 을 move 하므로 그 전에 공유본을 뜸).
    let expected_token = Arc::new(token.clone());

    // 8) daemon.json 기록.
    let start_time =
        engram_dashboard_core::agent::platform::current_process_start_time().unwrap_or(0);
    let info = engram_dashboard_net::portfile::DaemonInfo {
        pid: std::process::id(),
        host: "127.0.0.1".to_string(),
        port,
        token,
        protocol_version: PROTOCOL_VERSION,
        start_time,
    };
    if let Err(e) = engram_dashboard_net::portfile::write_atomic(&daemon_path, &info) {
        tracing::error!("daemon.json 기록 실패: {e}");
        return Err(1);
    }
    tracing::info!(
        port,
        pid = info.pid,
        protocol_version = PROTOCOL_VERSION,
        path = %daemon_path.display(),
        "데몬 시작 — daemon.json 기록 완료"
    );

    // 9) ★자동 부팅 resume 기본 OFF (2026-07-09, 사용자 결정)★ — 부팅 시 auto_restore=true 프로필을
    //   전부 되살리던 mgr.restore_all() 을 비활성화한다. 기본 = "부팅 자동 복원 안 함"(이벤트성으로
    //   꼭 떠야 하는 일부만 명시 복원). auto_restore 필드·reaper disposition·restore_all() 구현은
    //   그대로 유지(호출만 끔) — 특정 에이전트 이벤트성 복원은 향후 명시 command(RestoreAgents 류)에서
    //   restore_all() 을 부른다. handle 은 아래 abort/await 계약 유지용 no-op.
    //   (ADR-0016 "부팅 복원" 기본을 이 stopgap 이 뒤집음 — 정식 opt-in 설계 시 ADR 로 박을 것.)
    let restore_handle = tokio::task::spawn_blocking(|| {});

    // 10) 종료 신호 채널(watch). StopDaemon 명령이 이 watch 로 종료를 트리거한다.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 11) accept loop.
    tracing::info!("accept loop 시작(WS 핸들링 활성)");
    run_accept_loop(
        listener,
        wiring,
        multiview,
        control_registry,
        messaging_slot.clone(),
        expected_token,
        shutdown_tx,
        shutdown_rx,
        true, // 운영: Ctrl-C graceful 종료 활성
        KeepaliveConfig::default(),
    )
    .await;

    // 12) graceful 종료.
    restore_handle.abort();
    let _ = restore_handle.await;

    // ★남은 만료 처리는 불요★: 파킹은 인메모리라 프로세스 종료 = 상태 소멸이다(spec §0 "영속화 없음").
    //    sweep 은 blocking write 를 하지 않으므로(6.6) abort 가 항상 즉시 먹는다 — flush worker 와 다르다.
    sweep_task.abort();
    let _ = sweep_task.await;

    // ★종료 순서(BLOCK — round-3 finding 1)★: 에이전트 정리(shutdown_all)를 **flush worker 종료보다
    //   먼저** 한다. 자식이 살아 stdin 을 안 비우면 flush 의 blocking write 는 abort 로 끊기지 않으므로,
    //   순서를 뒤집으면 데몬 종료가 영영 hang 한다(잔여 분석 = `FlushWorkerHandles::shutdown`).
    let mgr = manager.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await {
        tracing::warn!("shutdown_all join 실패: {e}");
    }

    flush_worker.shutdown().await;

    // ADR-0086: 제어 채널 MCP 서버 graceful 종료(에이전트 정리 후 — 남은 세션도 함께 정리된다).
    if let Some(handle) = mcp_server_handle.take() {
        handle.shutdown().await;
        tracing::info!("MCP 서버 종료 완료");
    }

    // daemon.json 은 남겨둔다 — 다음 부팅이 stale 판정으로 무시한다.
    tracing::info!("데몬 종료 완료");
    Ok(())
}

// ── 테스트용 서버 기동 헬퍼 ───────────────────────────────────────────────────────

/// in-process 로 뜬 테스트 서버 핸들. 좀비 PTY 를 남기지 않으려면 테스트가 끝에서 반드시
/// `shutdown().await` 를 부른다 — drop 은 accept loop 만 끝내고 자식 정리를 하지 않는다.
///
/// 단일 인스턴스 가드·daemon.json 은 ★의도적으로 생략★(실프로세스 전용 관심사). 그 경로는
/// `tests/ws_e2e.rs` 의 #[ignore]/harness 가 실제 .exe 로 검증한다.
pub struct TestServerHandle {
    pub port: u16,
    pub token: String,
    pub manager: Arc<AgentManager>,
    accept_handle: tokio::task::JoinHandle<()>,
    shutdown_tx: watch::Sender<bool>,
    flush_worker: messaging_host::FlushWorkerHandles,
}

impl TestServerHandle {
    /// 서버를 graceful 하게 내린다. **전 에이전트 kill → flush worker 종료** 순서가 load-bearing 이며
    /// (근거 = run() 종료 주석), 좀비 PTY 방지를 위해 shutdown_all 까지 동기 대기한다.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.accept_handle.await;
        let mgr = self.manager.clone();
        let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;
        self.flush_worker.shutdown().await;
    }
}

pub async fn start_test_server() -> std::io::Result<TestServerHandle> {
    let store: Arc<dyn ProfileStore> = Arc::new(MemProfileStore::default());
    start_test_server_with_store(store).await
}

/// keepalive 주입형 — keepalive(half-open 감지) 동작을 검증하는 테스트가 짧은 ping/idle 값을 끼운다
/// (운영 기본값이면 그 테스트가 수십 초 걸린다).
pub async fn start_test_server_with_keepalive(
    keepalive: KeepaliveConfig,
) -> std::io::Result<TestServerHandle> {
    let store: Arc<dyn ProfileStore> = Arc::new(MemProfileStore::default());
    start_test_server_inner(store, keepalive).await
}

/// store 주입형 — 복원·persist 동작을 검증하고 싶은 테스트가 store 를 직접 끼운다.
pub async fn start_test_server_with_store(
    store: Arc<dyn ProfileStore>,
) -> std::io::Result<TestServerHandle> {
    start_test_server_inner(store, KeepaliveConfig::default()).await
}

async fn start_test_server_inner(
    store: Arc<dyn ProfileStore>,
    keepalive: KeepaliveConfig,
) -> std::io::Result<TestServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    // getrandom::Error 는 std::error::Error 미구현이라 메시지로 변환해 io::Error 로 감싼다.
    let token =
        generate_token().map_err(|e| std::io::Error::other(format!("token gen failed: {e}")))?;
    let expected_token = Arc::new(token.clone());

    let multiview = MultiViewState::new();
    // ADR-0096: WS dispatch(SetEnvelopeFormat)가 쓰는 봉투 포맷 전역 상태 거처. 테스트 서버는 MCP 서버·
    //   제어 채널을 배선하지 않으므로(아래 Noop) accept loop 전용 standalone registry 를 새로 만든다 —
    //   같은 Arc 라 dispatch 가 쓴 값을 그 서버 수명 동안 관측할 수 있다(운영은 control_registry 공유).
    let control_registry = Arc::new(control::registry::ControlRegistry::new());
    // 프리셋 persist 를 검증하는 테스트는 없다 — 필요해지면 store 주입형을 추가한다(현재 프리셋 unit 은
    //   core 에서 격리 검증).
    let preset_store: Arc<dyn PresetStore> = Arc::new(MemPresetStore::default());
    // WS 테스트는 제어 채널 미사용 → Noop(제어 채널 통합 테스트는 control::mcp_server 쪽이 담당).
    let control: Arc<dyn engram_dashboard_core::agent::types::ControlChannel> =
        Arc::new(engram_dashboard_core::agent::types::NoopControlChannel);
    // WS 테스트는 send/flush 를 검증하지 않지만 status sink wrapper 가 채널을 요구하므로 메시징 배선을
    //   함께 세운다.
    let messaging_slot = Arc::new(control::mcp_server::MessagingSlot::new());
    let (flush_tx, flush_rx) = tokio::sync::mpsc::unbounded_channel::<messaging_host::FlushMsg>();
    let idle_coalescer = Arc::new(messaging_host::IdleCoalescer::new());
    let wiring = build_daemon_wiring_with_store(
        store,
        preset_store,
        control,
        flush_tx.clone(),
        idle_coalescer.clone(),
    );
    let manager = wiring.manager.clone();
    // ★배선을 운영 run() 과 동일하게 유지한다★ — "테스트 서버에서만 게이트가 없는" 갈래를 만들지 않는다.
    let idle_notifier = Arc::new(messaging_host::ChannelIdleNotifier::new(
        flush_tx,
        idle_coalescer.clone(),
    ));
    let busy = Arc::new(messaging_host::busy_gate_for_manager(
        manager.clone(),
        idle_notifier.clone(),
    ));
    messaging_slot.set(Arc::new(
        messaging_host::messaging_for_manager_gated(
            manager.clone(),
            control_registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier),
    ));
    let flush_worker = messaging_host::spawn_flush_worker(
        flush_rx,
        messaging_host::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer,
        },
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let accept_handle = {
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            run_accept_loop(
                listener,
                wiring,
                multiview,
                control_registry,
                messaging_slot,
                expected_token,
                shutdown_tx,
                shutdown_rx,
                false, // 테스트: Ctrl-C 미설치(watch 로만 종료)
                keepalive,
            )
            .await;
        })
    };

    Ok(TestServerHandle {
        port,
        token,
        manager,
        accept_handle,
        shutdown_tx,
        flush_worker,
    })
}

/// 운영의 `FileProfileStore` 를 대신해 테스트 격리(디스크/Embedded 비오염)를 만든다.
#[derive(Default)]
struct MemProfileStore {
    saved: std::sync::Mutex<Vec<engram_dashboard_core::agent::profile::AgentProfile>>,
}

impl ProfileStore for MemProfileStore {
    fn save(&self, profiles: &[engram_dashboard_core::agent::profile::AgentProfile]) {
        *self.saved.lock().expect("mem store poisoned") = profiles.to_vec();
    }
    fn load(&self) -> Vec<engram_dashboard_core::agent::profile::AgentProfile> {
        self.saved.lock().expect("mem store poisoned").clone()
    }
}

/// `MemProfileStore` 의 프리셋판 — 운영의 `FilePresetStore` 를 대신한다.
#[derive(Default)]
struct MemPresetStore {
    saved: std::sync::Mutex<Vec<engram_dashboard_core::agent::preset::Preset>>,
}

impl PresetStore for MemPresetStore {
    fn save(&self, presets: &[engram_dashboard_core::agent::preset::Preset]) {
        *self.saved.lock().expect("mem preset store poisoned") = presets.to_vec();
    }
    fn load(&self) -> Vec<engram_dashboard_core::agent::preset::Preset> {
        self.saved
            .lock()
            .expect("mem preset store poisoned")
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_chars() {
        let t = generate_token().expect("token gen");
        assert_eq!(t.len(), 64, "256-bit = 32B → hex 64자");
        assert!(
            t.chars().all(|c| c.is_ascii_hexdigit()),
            "hex 문자만 포함해야 함"
        );
    }

    #[test]
    fn tokens_are_unique() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_data_dir_delegates_to_discovery_local_dir() {
        let _g = ENV_LOCK.lock().unwrap();
        // override 가 새어 들어오면(다른 테스트 leak) 기본 경로 단언이 깨진다 — 명시 제거.
        let prev = std::env::var_os("ENGRAM_DATA_DIR");
        std::env::remove_var("ENGRAM_DATA_DIR");
        let dir = resolve_data_dir();
        let delegated = engram_dashboard_discovery::default_data_dir();
        if let Some(v) = &prev {
            std::env::set_var("ENGRAM_DATA_DIR", v);
        }
        assert!(
            dir.ends_with(".engram-data"),
            "디버그(override 없음)에서 `.engram-data` 로 끝나야(app 과 동일 폴더): {dir:?}"
        );
        assert_eq!(
            dir, delegated,
            "resolve_data_dir 은 discovery::default_data_dir 와 동일해야"
        );
    }

    #[test]
    fn resolve_data_dir_honors_env_override() {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os("ENGRAM_DATA_DIR");
        let want = std::env::temp_dir().join("engram-daemon-resolve-override-test");
        std::env::set_var("ENGRAM_DATA_DIR", &want);
        let got = resolve_data_dir();
        match &prev {
            Some(v) => std::env::set_var("ENGRAM_DATA_DIR", v),
            None => std::env::remove_var("ENGRAM_DATA_DIR"),
        }
        assert_eq!(got, want, "ENGRAM_DATA_DIR set 시 그 경로로 격리돼야");
    }

    /// ENGRAM_DATA_DIR 은 프로세스 전역 env — set/remove 하는 테스트끼리 직렬화한다(병렬 짓밟음 방지).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
