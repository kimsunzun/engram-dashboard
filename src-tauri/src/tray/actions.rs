//! tray actions — 트레이 핸들러와 Tauri command 가 **공유하는** 부수효과 함수(불순: Tauri/discovery 의존).
//!
//! ## 왜 이 모듈이 따로 있나 (CLAUDE.md §5 — load-bearing)
//! 네이티브 트레이 팝업은 WebView DOM 이 아니라 cdp 가 직접 클릭 못 한다. 그래서 트레이 메뉴 클릭과
//! LLM/cdp 의 invoke 가 **같은 내부 함수**(여기)를 부르게 해 두 조작 주체가 같은 동작을 공유한다.
//! 트레이 핸들러(mod.rs on_menu_event)도, command(commands/tray.rs)도 전부 이 함수들만 호출 —
//! 동작 로직 중복 금지. core.rs(순수 판정)와 분리: 여기는 실제 창/아이콘/데몬을 만진다(불순).

use tauri::{AppHandle, Manager};

use super::core::{self, IconState};
use super::TrayIcons;

/// 트레이 아이콘 id(빌더에 부여, tray_by_id 로 재조회). 단일 트레이라 고정 문자열.
pub const TRAY_ID: &str = "engram-main-tray";

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
    // 데몬 graceful 일방 발사(결과 무시). data_dir 은 단일 출처(ADR-0024).
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

/// 데몬 alive 를 조회해 트레이 아이콘을 컬러/회색으로 갱신한다.
///
/// ★set_icon 은 메인 스레드 보장 경로로★: TrayIcon::set_icon 은 내부적으로 메인 스레드 실행을
/// 보장하지만, 워커 스레드(spawn_blocking)에서 호출될 수 있으므로 `run_on_main_thread` 로 감싸
/// 명시적으로 메인에서 돌린다(플랫폼 안전). 아이콘 두 벌은 TrayIcons state 에서 clone(저렴 — Arc).
///
/// daemon_status 는 daemon.json + PID liveness 판정(빠름, 비-blocking 수준). 외부/크래시 죽음
/// 주기감지는 비범위(액션 직후·setup 초기 갱신만, ADR-0026/TRD §3).
pub fn refresh_tray_icon(app: &AppHandle) {
    let data_dir = crate::discovery::default_data_dir();
    let alive = crate::discovery::daemon_status(&data_dir).alive;
    let state = core::icon_state_for(alive);
    set_tray_icon_state(app, state);
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
