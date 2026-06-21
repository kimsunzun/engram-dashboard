//! tray actions — 트레이 핸들러와 Tauri command 가 **공유하는** 부수효과 함수(불순: Tauri/discovery 의존).
//!
//! ## 왜 이 모듈이 따로 있나 (CLAUDE.md §5 — load-bearing)
//! 네이티브 트레이 팝업은 WebView DOM 이 아니라 cdp 가 직접 클릭 못 한다. 그래서 트레이 메뉴 클릭과
//! LLM/cdp 의 invoke 가 **같은 내부 함수**(여기)를 부르게 해 두 조작 주체가 같은 동작을 공유한다.
//! 트레이 핸들러(mod.rs on_menu_event)도, command(commands/tray.rs)도 전부 이 함수들만 호출 —
//! 동작 로직 중복 금지. core.rs(순수 판정)와 분리: 여기는 실제 창/아이콘/데몬을 만진다(불순).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::CheckMenuItem;
use tauri::{AppHandle, Emitter, Manager, Wry};

use tauri_plugin_autostart::ManagerExt;

use super::core::{self, IconState};
use super::TrayIcons;

/// 트레이 아이콘 id(빌더에 부여, tray_by_id 로 재조회). 단일 트레이라 고정 문자열.
pub const TRAY_ID: &str = "engram-main-tray";

/// 데몬 생사 push 의 단일 소유 상태(ADR-0028 단일 채널). 모든 소스(옵저버 probe·켜기·끄기)가
/// [`publish_daemon_liveness`] 로 들어와 여기 `last` 와 비교 → 변화 시에만 트레이+emit.
///
/// ★왜 manage 상태로 일원화하나(M-1 — load-bearing)★: 기존엔 옵저버(주기 probe)와 끄기 즉시확정
/// (StopOutcome::DaemonClosed → 회색 직접 set)이 **상태를 공유하지 않아** race 가 있었다 — 끄기 클릭
/// 후 데몬이 아직 죽는 중인 death-window(연결 닫힘 ~ 프로세스 exit 사이)에 옵저버가 그 창에서
/// alive=true 를 probe 해 방금 회색 박은 아이콘을 컬러로 되돌렸다(S13 race 재발). 이제 모든 생사
/// 소스가 이 단일 진입점으로 수렴하고, 끄기는 [`force_daemon_down`] 으로 억제창을 세팅해 그 race 를 닫는다.
#[derive(Default)]
pub struct LivenessState {
    /// 마지막으로 push 한 alive 값(None=아직 한 번도 push 안 함). 변화 판정 기준.
    last: Mutex<Option<bool>>,
    /// 끄기로 회색을 강제한 직후 death-window(데몬이 연결만 닫고 아직 exit 안 함) 동안 옵저버의
    /// alive=true 거짓 probe 가 아이콘을 컬러로 되돌리는 race(M-1) 를 차단하는 억제 만료 시각.
    /// None=억제 없음. Some(t)=now<t 동안 alive=true probe 를 무시.
    suppress_alive_until: Mutex<Option<Instant>>,
}

/// 끄기 직후 death-window 억제 시간. 데몬이 연결을 닫고 실제 exit 하기까지의 여유 — discovery 의
/// stop 폴링 timeout 과 같은 5초로 잡아 그 안에 probe 가 거짓 alive 를 보고해도 무시한다.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// "부팅 시 자동 시작" CheckMenuItem 핸들 보관(ADR-0027 §55).
///
/// ★왜 핸들을 manage 로 들고 있나(load-bearing)★: 토글 후 체크 표시를 set_checked 로 즉시 갱신해야
/// 하는데, CheckMenuItem 핸들이 없으면 갱신 대상을 못 잡는다. tray.menu() → id 재조회는 MenuItem 의
/// downcast(CheckMenuItem)가 번거로워, build_tray 가 만든 핸들을 그대로 state 로 보관해 직접 set_checked 한다.
pub struct AutostartCheck(pub CheckMenuItem<Wry>);

/// main 창을 보이고 포커스(숨김/최소화 상태에서 복귀).
///
/// ★순서가 load-bearing(Windows focus-stealing)★: show()→unminimize()→set_focus() 순서가 아니면
/// hidden/minimized 에서 작업표시줄 깜빡임만 나고 실제로 안 떠오를 수 있다(TRD §4). 프로세스 내부
/// 호출이라 IPC 없음. 창이 없으면(아직 미생성 등) 조용히 no-op.
pub fn show_main_ui(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// main 창을 숨긴다(파괴 아님 — WebView 상주, 트레이 "UI 보이기"로 복귀). X=hide 와 같은 종착.
pub fn hide_main_ui(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

/// 앱 전체 종료. **best-effort 데몬 graceful stop 후 exit**(ADR-0026).
///
/// ★best-effort 한정(load-bearing)★: 여기선 데몬을 graceful 로 한 번 찔러보고 결과를 무시한 뒤
/// 즉시 exit 한다. 정밀한 C4 graceful 순서(ack 대기·taskkill 폴백·아이콘 동기화)는 다음 단계 —
/// 여기서 데몬 종료를 기다리면(blocking) 종료가 지연되고, 데몬은 detached 라 우리가 안 죽여도
/// 독립 생존한다(ADR-0024). send_stop 은 수초 blocking 가능 → exit 전에 짧게만 시도하도록 별도
/// 스레드에서 발사하지 않고 동기로 부르되, 실패/무응답이어도 exit 로 진행한다.
pub fn quit_app(app: &AppHandle) {
    // 데몬 graceful 일방 발사(결과 무시). data_dir 은 default_data_dir()(데몬과 같은 폴더 단일 출처,
    // ADR-0024/0029)로 산출. ADR-0029: 모드 제거 → 무인자.
    let data_dir = crate::discovery::default_data_dir();
    match crate::discovery::send_stop(&data_dir) {
        Ok(outcome) => tracing::info!(
            ?outcome,
            "[tray] quit_app: 데몬 graceful stop 발사(best-effort)"
        ),
        Err(e) => tracing::warn!("[tray] quit_app: 데몬 stop 실패(무시하고 종료): {e}"),
    }
    app.exit(0);
}

/// 데몬 alive 를 probe 해 단일 publish 진입점([`publish_daemon_liveness`])으로 흘린다.
///
/// ★publish 경유로 일원화(ADR-0028 단일 채널 — load-bearing)★: 예전엔 여기서 직접 set_icon 했으나,
/// 이제 probe 결과를 publish 에 넘겨 **중복차단(변화 시에만 set)·emit·억제창 판정**을 한 곳에서
/// 처리한다. 그래야 옵저버·켜기·끄기 모든 생사 소스가 같은 게이트(LivenessState)를 거쳐 M-1 race 와
/// emit 비대칭(m-2)이 사라진다.
///
/// daemon_status 는 daemon.json + PID liveness 판정(빠름, 비-blocking 수준). 외부/크래시 죽음
/// 주기감지는 옵저버가 담당(spawn_daemon_observer) — 여기는 액션 직후·setup 초기 갱신용.
pub fn refresh_tray_icon(app: &AppHandle) {
    // data_dir 은 default_data_dir()(데몬과 같은 폴더 단일 출처, ADR-0024/0029)로 산출.
    let data_dir = crate::discovery::default_data_dir();
    let alive = crate::discovery::daemon_status(&data_dir).alive;
    publish_daemon_liveness(app, alive);
}

/// 데몬 생사 단일 publish — 변화 시에만 트레이 set_icon + emit("daemon-status-changed").
///
/// ★억제창 중 alive=true 는 무시(M-1 race 차단 — load-bearing)★: 끄기 직후 death-window 동안
/// 옵저버 probe 가 "연결은 닫혔지만 프로세스가 아직 살아있는" 데몬을 alive=true 로 거짓 보고해 방금
/// 회색 박은 아이콘을 컬러로 되돌리는 race 가 있다. 억제창(suppress_alive_until) 안에서는 alive=true
/// 를 버린다. alive=false 는 항상 통과한다(끄기 확정 — 억제 무관).
///
/// ★락 보유 중 set_icon/emit 금지(ADR-0006 락 순서)★: 변화 판정·억제 판정만 락 안에서 하고, 락을
/// 드롭한 뒤에 set_icon/emit(외부 호출·메인 스레드 post)을 부른다. set_icon 자체는
/// set_tray_icon_state 가 run_on_main_thread 로 메인 스레드 보장.
pub fn publish_daemon_liveness(app: &AppHandle, observed_alive: bool) {
    let st = app.state::<LivenessState>();

    // 억제창 판정: alive=true 보고가 death-window 안이면 거짓일 수 있어 무시(락 짧게 잡고 드롭).
    if observed_alive {
        if let Some(until) = *st.suppress_alive_until.lock().unwrap() {
            if Instant::now() < until {
                return; // death-window — 거짓 alive 무시
            }
        }
    }

    // 변화 판정(락 짧게) → 드롭 → 부수효과. 변화 없으면 set_icon/emit 둘 다 생략.
    {
        let mut last = st.last.lock().unwrap();
        if *last == Some(observed_alive) {
            return;
        }
        *last = Some(observed_alive);
    } // ← 여기서 last 락 드롭. 아래 부수효과는 락 미보유 상태에서 실행.

    set_tray_icon_state(app, core::icon_state_for(observed_alive));
    if let Err(e) = app.emit("daemon-status-changed", observed_alive) {
        tracing::warn!("[tray] daemon-status-changed emit 실패: {e}");
    }
    tracing::debug!(alive = observed_alive, "[tray] 데몬 생사 push");
}

/// 끄기 확정(StopOutcome::DaemonClosed) — 회색을 강제하고 death-window 억제창을 설정한다.
///
/// ★억제창 먼저, publish(false) 나중(M-1 — load-bearing)★: suppress_alive_until 을 먼저 세팅한 뒤
/// publish(false) 로 회색을 확정한다. false 는 억제 판정과 무관하게 통과하므로(억제는 alive=true 만
/// 막음) 즉시 회색+emit 이 나가고, 이후 STOP_GRACE 동안 옵저버의 거짓 alive=true probe 는 publish 가
/// 억제창에서 버린다 → 회색이 컬러로 되돌아가지 않는다.
pub fn force_daemon_down(app: &AppHandle) {
    *app.state::<LivenessState>()
        .suppress_alive_until
        .lock()
        .unwrap() = Some(Instant::now() + STOP_GRACE);
    publish_daemon_liveness(app, false); // false 는 억제 무관 통과 → 회색+emit, last=false
}

/// 주어진 [`IconState`] 로 트레이 아이콘을 직접 교체한다(**PID probe 없음**).
///
/// ★probe 우회 경로(load-bearing — S13 race 재발 방지)★: `refresh_tray_icon` 은
/// `daemon_status().alive`(PID probe=OpenProcess)로 상태를 *조회*해 set 하지만, 이 함수는 호출자가
/// 이미 확정한 state 를 그대로 set 한다. 데몬 graceful stop 직후 "연결은 닫혔지만 프로세스가 아직
/// 수 ms 더 살아있는" 창에서는 PID probe 가 alive=true 를 돌려줘 아이콘이 컬러로 고착되는 race 가
/// 있다(StopOutcome 주석 참조). 그래서 StopOutcome::DaemonClosed(꺼짐 확정) 경로는 probe 를
/// 거치지 않고 이 함수로 회색을 직접 박는다. probe 가 필요한 일반 갱신은 `refresh_tray_icon` 을 쓴다.
///
/// set_icon 메인 스레드 보장은 `refresh_tray_icon` 과 동일(run_on_main_thread). 아이콘 두 벌은
/// TrayIcons state 에서 clone(저렴 — 내부 Cow/Arc).
pub fn set_tray_icon_state(app: &AppHandle, state: IconState) {
    let icons = app.state::<TrayIcons>();
    // Image<'static> 는 내부 Cow — clone 저렴. 메인 스레드 클로저로 move 하기 위해 미리 복제.
    let img = match state {
        IconState::Active => icons.active.clone(),
        IconState::Inactive => icons.inactive.clone(),
    };

    let app_main = app.clone();
    let set = move || {
        if let Some(tray) = app_main.tray_by_id(TRAY_ID) {
            if let Err(e) = tray.set_icon(Some(img)) {
                tracing::warn!("[tray] set_icon 실패: {e}");
            }
        }
    };
    // 이미 메인 스레드면 즉시, 아니면 메인 루프에 post.
    if let Err(e) = app.run_on_main_thread(set) {
        tracing::warn!("[tray] run_on_main_thread(set_icon) 실패: {e}");
    }
}

/// "부팅 시 자동 시작" 토글(ADR-0027 §55) — 현재 활성 여부를 읽어 반전(enable/disable)하고
/// CheckMenuItem 체크 표시를 새 상태로 동기화한다. 트레이 클릭·command(set_autostart) 가 공유할 수
/// 있으나, command 는 명시 bool 을 받으므로 토글(반전)은 트레이 전용 진입이다.
///
/// is_enabled/enable/disable·set_checked 실패는 warn 만(토글 1회 실패가 앱을 죽이면 안 됨). 갱신
/// 누락 시 다음 클릭이 다시 토글하므로 영구 고착 아님.
pub fn toggle_autostart(app: &AppHandle) {
    let mgr = app.autolaunch();
    let enabled = match mgr.is_enabled() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[tray] autostart is_enabled 실패(토글 취소): {e}");
            return;
        }
    };
    // 반전: 켜져 있으면 끄고, 꺼져 있으면 켠다.
    let result = if enabled { mgr.disable() } else { mgr.enable() };
    if let Err(e) = result {
        tracing::warn!("[tray] autostart 토글(enable/disable) 실패: {e}");
        return;
    }
    let new_state = !enabled;
    // 체크 표시 동기화 — 보관한 CheckMenuItem 핸들로 직접 set_checked.
    if let Some(check) = app.try_state::<AutostartCheck>() {
        if let Err(e) = check.0.set_checked(new_state) {
            tracing::warn!("[tray] autostart CheckMenuItem set_checked 실패: {e}");
        }
    }
    tracing::info!(enabled = new_state, "[tray] 부팅 자동 시작 토글");
}
