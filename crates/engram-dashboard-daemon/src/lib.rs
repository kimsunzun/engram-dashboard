//! engram-dashboard-daemon — 라이브러리 표면.
//!
//! `main.rs`(데몬 진입점)와 격리 하네스(`tests/ws_e2e.rs`)가 **같은 기동 흐름**을 공유하도록
//! 서버 조립·accept loop 를 여기로 모았다. main 은 `run()` 한 줄만 부르고, 테스트는
//! `start_test_server()` 로 in-process 서버를 띄워 WS 클라이언트로 검증한다.
//!
//! ★운영 코드 회귀 0★: 옛 main 의 동작(단일 인스턴스 가드 → data_dir → daemon.json stale 검사
//! → bind → 토큰 → manager 배선 → restore_all → accept loop → graceful 종료)을 `run()` 이
//! 그대로 수행한다. accept loop 본체는 `run_accept_loop()` 로 분리해 테스트와 공유한다.

pub mod connection_core;
pub mod control;
pub mod instance;
pub mod portfile;
pub mod ws;

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::preset::{PresetRegistry, PresetStore};
use engram_dashboard_core::agent::profile::{ProfileRegistry, ProfileStore};
use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_core::logging;
use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_protocol::PROTOCOL_VERSION;

use tokio::net::TcpListener;
use tokio::sync::watch;

use connection_core::MultiViewState;
use ws::{ConnRegistry, DaemonStatusSink, KeepaliveConfig};

const DAEMON_FILE: &str = "daemon.json";

// ── data dir / 토큰 ──────────────────────────────────────────────────────────────

/// 데이터 디렉토리 결정 — discovery 의 단일 출처(ADR-0024/0029)에 위임한다.
///
/// ★app 일치★: app(src-tauri)도 같은 `default_data_dir()` 을 쓰므로 둘이 같은 폴더의
/// `{agents.json,daemon.json}` 을 본다. ADR-0029(모드 제거): debug=repo 루트 `.engram-data`,
/// 릴리즈=`%APPDATA%\com.engram.dashboard`.
///
/// ★ENGRAM_DATA_DIR override(테스트 격리 탈출구)★: discovery 가 우선순위 1번으로 처리한다 —
/// 설정 시 그 경로로 간다. 데몬은 `std::process::Command` 로 **직접** spawn 되는 통합 테스트
/// (`tests/ws_e2e.rs`)에서 이 env 를 상속받아 임시 디렉토리로 격리된다(운영 `.engram-data` 미오염).
/// WMI-spawn 데몬엔 안 먹는다(부모 env 미상속, discovery 주석 참조).
fn resolve_data_dir() -> PathBuf {
    engram_dashboard_discovery::default_data_dir()
}

/// 256-bit(32B) 토큰을 OS CSPRNG 로 생성해 hex 64자 문자열로 반환.
/// 보안: 반환값은 로그에 찍지 말 것(daemon.json 에만 기록).
pub fn generate_token() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf)?;
    let mut s = String::with_capacity(64);
    for b in buf {
        // 소문자 hex 2자/바이트.
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

/// 데몬 프로세스 env 에 `ENGRAM_EXE`(앱 CLI exe 절대경로)를 세팅한다. 자식 PTY 가 상속한다(위 블록 주석).
/// current_exe(데몬)의 형제 `engram-dashboard.exe` 를 찾는다(locate_daemon_exe 대칭). 못 찾으면 no-op.
///
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

// ── panic hook (B-1) ──────────────────────────────────────────────────────────────

/// 데몬 전역 panic hook 설치. panic 한 스레드명·위치·메시지를 tracing::error! 로 남긴다.
///
/// ★기존 hook 보존★: set_hook 으로 교체하기 전 take_hook 으로 이전 hook 을 잡아, 새 hook
///   안에서 먼저 로깅한 뒤 이전 hook 을 이어 호출한다(default backtrace 출력 등 유지).
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
            // payload 는 보통 &str 또는 String — 둘 다 시도해 메시지를 뽑는다.
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
            // 이전 hook 이어 호출(default 동작 보존).
            prev(info);
        }));
    });
}

// ── AgentManager 배선 (src-tauri lib.rs setup 미러) ───────────────────────────────

/// src-tauri 의 setup 블록과 동일한 방식으로 AgentManager 를 조립한다(파일 기반 store).
/// 차이: StatusSink 가 TauriStatusSink 대신 DaemonStatusSink(연결된 WS 클라이언트에 push).
/// `registry` 는 호출자가 만들어 주입한다 — DaemonStatusSink 와 accept loop 가 같은 인스턴스를
/// 공유해야 status 브로드캐스트 대상(전 연결 conn_tx)이 일치한다.
fn build_manager(
    data_dir: &std::path::Path,
    registry: ConnRegistry,
    control: Arc<dyn engram_dashboard_core::agent::types::ControlChannel>,
) -> Arc<AgentManager> {
    // 프로필 저장 = data_dir/agents.json, 프리셋 저장 = data_dir/presets.json (ADR-0061).
    // 두 store 모두 디렉토리를 받고 내부에서 파일명을 결합한다.
    let profile_store = Arc::new(FileProfileStore::new(data_dir.to_path_buf()));
    let preset_store = Arc::new(FilePresetStore::new(data_dir.to_path_buf()));
    build_manager_with_store(profile_store, preset_store, registry, control)
}

/// build_manager 의 store 주입형 — 테스트가 in-memory store 를 끼워 디스크/Embedded 와 격리할 수
/// 있게 store 를 인자로 받는다(운영 경로는 위 build_manager 가 File{Profile,Preset}Store 를 넘김).
/// 배선 로직(status_sink/profiles/presets/tracker)은 운영과 동일 — 회귀 없음.
/// `control`(ADR-0086): 제어 채널 seam — 운영은 DaemonControlChannel, 제어 채널 미사용 테스트는 Noop.
fn build_manager_with_store(
    store: Arc<dyn ProfileStore>,
    preset_store: Arc<dyn PresetStore>,
    registry: ConnRegistry,
    control: Arc<dyn engram_dashboard_core::agent::types::ControlChannel>,
) -> Arc<AgentManager> {
    let status_sink = Arc::new(DaemonStatusSink::new(registry));
    let profiles = Arc::new(ProfileRegistry::new(store));
    // ADR-0061: 프리셋 레지스트리도 데몬이 소유. 프로필과 동일하게 store 에서 로드해 초기화.
    let presets = Arc::new(PresetRegistry::new(preset_store));

    // 세션 추적: sid 변경(/clear 등) 관측 시 레지스트리에 반영(즉시 persist).
    let profiles_cb = profiles.clone();
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig::default(),
        Arc::new(move |agent_id, new_sid| {
            profiles_cb.observe_session_id(agent_id, new_sid);
        }),
    ));
    tracker.start();

    // ADR-0086: 제어 채널 seam 을 주입해 spawn=provision / terminal=revoke 인과를 코어에 잇는다.
    Arc::new(AgentManager::new_with_control(
        status_sink,
        profiles,
        presets,
        tracker,
        control,
    ))
}

// ── accept loop (main + 테스트 공유) ──────────────────────────────────────────────

/// 연결 수락 루프. 각 연결을 handle_connection(WS 업그레이드 + auth + 프레임 핸들링)으로 넘긴다.
/// 연결마다 task spawn — 한 연결의 느림/오류가 다른 연결·accept 를 막지 않는다.
///
/// 종료 경로: shutdown_rx 가 true 로 바뀌면(StopDaemon) 또는 Ctrl-C(run() 만 — 테스트는 watch 로
/// 종료) 루프를 빠져나온다. ★이 함수는 self-contained accept loop 로, main 과 테스트가 동일하게
/// 쓴다 — 운영/테스트 경로가 한 코드를 공유해 회귀를 막는다.★
#[allow(clippy::too_many_arguments)]
async fn run_accept_loop(
    listener: TcpListener,
    manager: Arc<AgentManager>,
    registry: ConnRegistry,
    multiview: MultiViewState,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
    enable_ctrl_c: bool,
    keepalive: KeepaliveConfig,
) {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        tracing::debug!(%peer, "연결 수락 — WS 핸들러로 넘김");
                        let manager = manager.clone();
                        let registry = registry.clone();
                        let multiview = multiview.clone();
                        let expected_token = expected_token.clone();
                        let shutdown_tx = shutdown_tx.clone();
                        tokio::spawn(async move {
                            ws::handle_connection(
                                stream,
                                peer,
                                manager,
                                registry,
                                multiview,
                                expected_token,
                                shutdown_tx,
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
            // Ctrl-C 는 운영(run) 경로에서만 활성. 테스트는 watch 로만 종료(시그널 미설치).
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
    // 0) 기본 warn(OFF) — RUST_LOG 로 재정의. core 의 init_logging 재사용.
    //    ★마스킹은 미포함★ — init_logging 은 키를 가리지 않는다. mask_secrets 는 헬퍼만 제공하고
    //    적용은 호출자 책임이다(민감 출력 로깅 시 명시 적용). 근거: docs/reference/logging-conventions.md.
    logging::init_logging();

    // 0.5) panic hook 설치(B-1). 데몬 내부 스레드(pump 등)가 panic 하면 silent 정지로
    //   넘어가기 쉬우므로(§5 "죽음 감지는 백엔드가 판단"), panic 위치·스레드명·메시지를
    //   tracing::error! 로 가시화한다. ★기존 default hook 동작 보존★: backtrace/표준 출력
    //   동작을 잃지 않게 이전 hook 도 이어서 호출한다(연쇄). 데몬 전체는 죽이지 않는다 —
    //   연결 task panic 은 tokio 가 이미 격리하고, pump panic 은 B-2 가 Failed 로 전이시킨다.
    install_panic_hook();

    // 0.6) ENGRAM_EXE 주입(설계 §5 · ADR-0014 방향) — 스폰될 자식 PTY 가 상속할 수 있게 부팅 최초 1회.
    //   ★반드시 에이전트 spawn 전★: 이 env 를 세팅한 뒤에야 이후 spawn_agent 의 PTY 자식이 상속한다.
    //   다른 스레드 spawn 전(run 진입 직후)이라 set_var data race 안전(set_engram_exe_env SAFETY 주석).
    set_engram_exe_env();

    // 1) 단일 인스턴스 가드. 이미 실행 중이면 로그 남기고 정상 종료(exit 0).
    //    ★_guard 는 프로세스 수명 동안 살아 있어야 한다★(Drop 시 mutex 해제 = 단일성 깨짐).
    let _guard = match instance::acquire() {
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

    // 2) data_dir 결정 + 생성(ADR-0029 — release 에서 %APPDATA%, debug repo 루트 `.engram-data`).
    let data_dir = resolve_data_dir();
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        tracing::error!("data_dir 생성 실패({:?}): {e}", data_dir);
        return Err(1);
    }
    let daemon_path = data_dir.join(DAEMON_FILE);

    // 2.5) 기존 daemon.json 검사. stale(죽은 PID)이면 무시(로그만)하고 덮어쓴다.
    //      살아있으면 방어적으로 덮어쓰지 않고 정상 종료(살아있는 데몬 보호).
    if let Some(prev) = portfile::read(&daemon_path) {
        if portfile::is_stale(&prev) {
            tracing::info!(pid = prev.pid, "기존 daemon.json 이 stale — 덮어씀");
        } else {
            tracing::warn!(
                pid = prev.pid,
                "기존 daemon.json 의 PID 가 살아있음 — 덮어쓰지 않고 종료(살아있는 데몬 보호)"
            );
            return Ok(());
        }
    }

    // 3) 127.0.0.1:0 바인드 → 실제 포트 취득(로컬 전용).
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

    // 4) 256-bit 토큰 생성. 보안: 토큰 자체는 절대 로그에 찍지 않는다.
    let token = match generate_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("토큰 생성 실패: {e}");
            return Err(1);
        }
    };

    // ADR-0086: 제어 채널 토큰 레지스트리 — MCP auth 미들웨어(검증)와 DaemonControlChannel(발급)이
    //   공유하는 단일 출처. daemon.json 의 WS 토큰과는 완전히 다른 관심사(혼용 금지 — ADR-0086 §맥락).
    let control_registry = Arc::new(control::registry::ControlRegistry::new());

    // 5) 연결 레지스트리(status 브로드캐스트용) — DaemonStatusSink 와 accept loop 가 공유한다.
    let registry = ConnRegistry::new();
    // 5b) 멀티뷰어 협상 상태(resize smallest + 입력 lease) — 전 연결이 공유한다.
    let multiview = MultiViewState::new();

    // 5b.5) ADR-0086 부팅 스윕(FIX 5): 이전 데몬 크래시/실패 스폰이 남긴 stale mcp-config 를 청소한다.
    //   ★반드시 MCP 서버·provision 시작 전★: registry 는 방금 빈 상태로 만들었으니(위 5b) 이 시점의
    //   모든 기존 파일은 dead credential(토큰이 registry 에 없음)이다. 부팅 초입에 일괄 삭제해 평문 토큰
    //   파일을 방치하지 않는다. (삭제 실패는 warn 만 — 청소 실패로 데몬 기동을 막지 않는다.)
    control::mcp_config::sweep_stale_configs(&data_dir);

    // 5c) ADR-0086: 제어 채널 MCP 서버 기동(WS 서버와 나란히). 스폰될 에이전트가 mcp-config 로 붙는
    //     입구다. 토큰 레지스트리는 auth 미들웨어(검증)와 provision(발급)이 공유한다.
    //     ★fail-closed(FIX 1)★: bind/start 실패는 **치명**이다 — 데몬을 NoopControlChannel 로 조용히
    //     계속 띄우면(옛 동작) 제어 채널 없이 도는데도 health 를 위장한다. 대신 이미 만든 자원(WS
    //     listener·control_registry)을 drop 하고 Err(1) 로 데몬 시작을 중단한다. 데몬은 자기 제어
    //     엔드포인트 없이는 뜨지 않는다(에이전트 오케스트레이션이 §5 LLM-우선 제어의 근간이라, 그게
    //     없는 반쪽 데몬은 정상 상태가 아니다). ★반드시 에이전트 spawn 전★.
    // MCP 서버 핸들 — Some 이면 프로세스 수명 동안 살아 있어야 서버가 유지된다(drop=종료). fail-closed
    //   라 실패 시 아래 match 가 early-return 하므로, 여기 도달하면 항상 살아 있는 핸들을 든다.
    let (control, mut mcp_server_handle): (
        Arc<dyn engram_dashboard_core::agent::types::ControlChannel>,
        Option<control::mcp_server::McpServerHandle>,
    ) = match control::mcp_server::start_mcp_server(control_registry.clone()).await {
        Ok(handle) => {
            let url = handle.url.clone();
            let channel = Arc::new(control::DaemonControlChannel::new(
                control_registry.clone(),
                url,
                data_dir.clone(),
            ));
            // 핸들을 살려 둔다(drop 시 서버 종료). 프로세스 수명 동안 유지.
            (channel, Some(handle))
        }
        Err(e) => {
            // fail-closed: 이미 만든 자원 정리(listener·registry 는 이 스코프 drop 로 회수) 후 중단.
            //   daemon.json 은 아직 안 썼으므로(아래 8단계) 남는 stale portfile 도 없다.
            tracing::error!(
                "MCP 서버 기동 실패 — 제어 채널 없이는 데몬을 띄우지 않는다(fail-closed): {e}"
            );
            drop(listener);
            return Err(1);
        }
    };

    // 6) AgentManager 배선(src-tauri 미러). status_sink = DaemonStatusSink(registry).
    let manager = build_manager(&data_dir, registry.clone(), control);

    // 7) auth 비교용 토큰을 Arc 로 보관(daemon.json 에 token 을 move 하므로 그 전에 공유본을 뜸).
    //    보안: 이 값은 로그/외부 노출 금지(handle_connection 내부 비교 전용).
    let expected_token = Arc::new(token.clone());

    // 8) daemon.json atomic 기록. 토큰을 포함하나 파일에만 — 로그엔 port/pid 만.
    let start_time =
        engram_dashboard_core::agent::platform::current_process_start_time().unwrap_or(0);
    let info = portfile::DaemonInfo {
        pid: std::process::id(),
        host: "127.0.0.1".to_string(),
        port,
        token,
        protocol_version: PROTOCOL_VERSION,
        start_time,
    };
    if let Err(e) = portfile::write_atomic(&daemon_path, &info) {
        tracing::error!("daemon.json 기록 실패: {e}");
        // ADR-0086 F5: 여기서 Err 로 반환하면 mcp_server_handle(Some)이 스코프 종료로 drop 되며
        //   McpServerHandle::drop 이 cancel 토큰을 발화해 detached serve 태스크를 확실히 내린다
        //   (프로세스 종료가 대개 무의미하게 만들지만 태스크 누수를 airtight 하게 막는다).
        return Err(1);
    }
    tracing::info!(
        port,
        pid = info.pid,
        protocol_version = PROTOCOL_VERSION,
        path = %daemon_path.display(),
        "데몬 시작 — daemon.json 기록 완료"
    );

    // 9) 복원은 blocking(3s 조기종료 윈도·stagger). spawn_blocking 으로 async executor 보호.
    // ★자동 부팅 resume 기본 OFF (2026-07-09, 사용자 결정)★ — 부팅 시 auto_restore=true 프로필을
    //   전부 되살리던 mgr.restore_all() 을 비활성화한다. 기본 = "부팅 자동 복원 안 함"(이벤트성으로
    //   꼭 떠야 하는 일부만 명시 복원). auto_restore 필드·reaper disposition·restore_all() 구현은
    //   그대로 유지(호출만 끔) — 특정 에이전트 이벤트성 복원은 향후 명시 command(RestoreAgents 류)에서
    //   restore_all() 을 부른다. handle 은 아래 abort/await 계약 유지용 no-op.
    //   (ADR-0016 "부팅 복원" 기본을 이 stopgap 이 뒤집음 — 정식 opt-in 설계 시 ADR 로 박을 것.)
    let restore_handle = tokio::task::spawn_blocking(|| {
        // manager.restore_all();  // ← 자동 부팅 복원 비활성 (위 주석)
    });

    // 10) 종료 신호 채널(watch). StopDaemon 명령이 이 watch 로 종료를 트리거한다.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 11) accept loop(운영: Ctrl-C 활성). main 과 테스트가 같은 run_accept_loop 를 공유한다.
    tracing::info!("accept loop 시작(WS 핸들링 활성)");
    run_accept_loop(
        listener,
        manager.clone(),
        registry,
        multiview,
        expected_token,
        shutdown_tx,
        shutdown_rx,
        true,                       // 운영: Ctrl-C graceful 종료 활성
        KeepaliveConfig::default(), // 운영 기본 keepalive(20s/50s)
    )
    .await;

    // 12) graceful 종료. 먼저 in-flight restore 를 abort 해 shutdown_all 과의 경합을 막는다.
    restore_handle.abort();
    let _ = restore_handle.await; // abort/완료 결과 무시(Cancelled 또는 Ok)

    // 모든 에이전트 정리(PTY kill + tracker 정지). blocking 이므로 spawn_blocking 으로 실행하고 대기.
    let mgr = manager.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await {
        tracing::warn!("shutdown_all join 실패: {e}");
    }

    // ADR-0086: 제어 채널 MCP 서버 graceful 종료(에이전트 정리 후 — 남은 세션도 함께 정리된다).
    if let Some(handle) = mcp_server_handle.take() {
        handle.shutdown().await;
        tracing::info!("MCP 서버 종료 완료");
    }

    // daemon.json 은 남겨둔다 — 다음 부팅이 stale 판정으로 무시한다.
    tracing::info!("데몬 종료 완료");
    // _guard 가 여기서 drop 되며 mutex 해제.
    Ok(())
}

// ── 테스트용 서버 기동 헬퍼 ───────────────────────────────────────────────────────

/// in-process 로 뜬 테스트 서버 핸들. drop 만으로도 서버를 내리지만(shutdown 신호 + abort),
/// 누수 없는 정리를 위해 테스트는 끝에서 `shutdown().await` 를 권장한다.
///
/// ★격리 설계★:
/// - bind 는 127.0.0.1:0 → 실제 포트(`port`)를 OS 가 할당(테스트 병렬 실행 시 충돌 없음).
/// - token 은 테스트가 아는 값(`token`)을 직접 주입 — daemon.json·파일 IO 없이 auth 검증.
/// - manager 는 in-memory ProfileStore 로 배선 → 디스크/Embedded 의 agents.json 과 격리.
/// - 단일 인스턴스 가드·daemon.json·restore_all 은 ★의도적으로 생략★(실프로세스 전용 관심사).
///   그 경로는 `tests/ws_e2e.rs` 의 #[ignore]/harness 가 실제 .exe 로 검증한다.
pub struct TestServerHandle {
    /// OS 가 할당한 실제 포트(클라가 ws://127.0.0.1:{port} 로 붙는다).
    pub port: u16,
    /// 이 서버가 기대하는 auth 토큰(테스트가 아는 값).
    pub token: String,
    /// 에이전트 spawn/kill 등 직접 조작용(테스트가 결정적 출력 agent 를 띄울 때).
    pub manager: Arc<AgentManager>,
    /// accept loop task 핸들 — shutdown 시 join.
    accept_handle: tokio::task::JoinHandle<()>,
    /// accept loop 종료 신호(watch). shutdown() 이 true 로 보낸다.
    shutdown_tx: watch::Sender<bool>,
}

impl TestServerHandle {
    /// 서버를 graceful 하게 내린다: 종료 신호 → accept loop join → 전 에이전트 kill.
    /// 좀비 PTY 방지를 위해 shutdown_all 까지 동기 대기한다.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.accept_handle.await;
        let mgr = self.manager.clone();
        let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;
    }
}

/// in-process 테스트 서버 기동. 127.0.0.1:0 bind → 실제 포트 + 알려진 토큰 + 실제
/// AgentManager(in-memory store) + DaemonStatusSink 를 배선하고 accept loop 를 tokio task 로 띄운다.
///
/// ★main 과의 공유★: accept loop 본체(`run_accept_loop`)와 manager 배선(`build_manager_with_store`)을
/// 운영 경로와 같은 함수로 호출한다 — 테스트가 검증하는 코드 = 실제 도는 코드.
pub async fn start_test_server() -> std::io::Result<TestServerHandle> {
    // in-memory store — 디스크/Embedded agents.json 과 격리. ProfileStore trait 구현체.
    let store: Arc<dyn ProfileStore> = Arc::new(MemProfileStore::default());
    start_test_server_with_store(store).await
}

/// keepalive 주입형 — keepalive(half-open 감지) 동작을 검증하는 테스트가 짧은 ping/idle 값을
/// 끼운다(상수 하드코딩 회피 — 테스트가 수십 초 걸리지 않게). 운영 기본은 위 start_test_server.
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
    // keepalive 미관심 테스트는 운영 기본값 사용.
    start_test_server_inner(store, KeepaliveConfig::default()).await
}

/// store + keepalive 둘 다 주입하는 내부 구현(공유). 위 공개 헬퍼들이 이걸 호출한다.
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

    let registry = ConnRegistry::new();
    let multiview = MultiViewState::new();
    // 프리셋 store 는 테스트마다 새 in-memory(디스크 비오염). 프리셋 persist 를 검증하는 테스트는
    // 별도 store 주입형이 필요하면 추후 추가한다(현재 프리셋 unit 은 core 에서 격리 검증).
    let preset_store: Arc<dyn PresetStore> = Arc::new(MemPresetStore::default());
    // WS 테스트는 제어 채널 미사용 → Noop(제어 채널 통합 테스트는 별도 control::mcp_server 테스트가 담당).
    let control: Arc<dyn engram_dashboard_core::agent::types::ControlChannel> =
        Arc::new(engram_dashboard_core::agent::types::NoopControlChannel);
    let manager = build_manager_with_store(store, preset_store, registry.clone(), control);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let accept_handle = {
        let manager = manager.clone();
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            run_accept_loop(
                listener,
                manager,
                registry,
                multiview,
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
    })
}

/// 테스트 전용 in-memory ProfileStore. save 를 받아 보관하고 load 로 돌려준다(디스크 IO 없음).
/// 운영의 FileProfileStore 를 대신해 테스트 격리(디스크/Embedded 비오염)를 만든다.
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

/// 테스트 전용 in-memory PresetStore(ADR-0061). MemProfileStore 의 프리셋판 — 디스크 IO 없이
/// 프리셋 배선(PresetRegistry) 을 격리한다. 운영의 FilePresetStore 를 대신한다.
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
        // CSPRNG 라 연속 호출이 충돌하지 않아야 한다(난수성 기본 확인).
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a, b);
    }

    // resolve_data_dir 는 discovery 의 단일 출처(default_data_dir)에 위임한다(ADR-0024/0029).
    // 디버그 빌드(테스트는 항상 디버그)에서 env override 가 없으면 repo 루트의 `.engram-data` 로
    // 끝나야 한다 — app(src-tauri)과 같은 폴더를 가리키는 불변식. (릴리즈는 %APPDATA% 이며 분기
    // 자체는 discovery 단위테스트가 헬퍼로 검증한다.)
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
        // discovery 의 단일 출처와 바이트 단위로 일치해야(app·daemon 일치 불변식).
        assert_eq!(
            dir, delegated,
            "resolve_data_dir 은 discovery::default_data_dir 와 동일해야"
        );
    }

    // ENGRAM_DATA_DIR override(테스트 격리 탈출구) 가 resolve_data_dir 까지 흘러야 한다 — 통합 테스트
    // (ws_e2e.rs)가 직접-spawn 데몬을 임시 디렉토리로 보내 운영 `.engram-data` 오염을 막는 메커니즘.
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
