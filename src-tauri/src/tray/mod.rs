//! tray — Tauri 통합 트레이 모듈(ADR-0026: 트레이를 앱에 통합, 2프로세스).
//!
//! ## 구조 (3층 — CLAUDE.md §4/§5)
//! - [`core`] — **순수**(OS/Tauri/discovery 무의존): MenuAction 의도 enum, menu_id↔action 매핑,
//!   IconState, to_grayscale_rgba. 단위테스트 대상.
//! - [`actions`] — **불순 공유 부수효과**: 트레이 핸들러와 command 가 같은 함수를 부르게(중복 금지).
//!   show/hide 창, quit_app, refresh_tray_icon.
//! - 이 파일(`mod.rs`) — Tauri 배선(불순): TrayIconBuilder, 메뉴 생성, on_menu_event 디스패치,
//!   아이콘 두 벌 생성·보관(TrayIcons state).
//!
//! Windows 전용(트레이 GUI). setup() 에서 build_tray(app) 1회 호출.

pub mod actions;
pub mod core;

use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{App, AppHandle, Manager};

use tauri_plugin_autostart::ManagerExt;

use actions::{AutostartCheck, TRAY_ID};
use core::{IconState, MenuAction};

use crate::discovery::StopOutcome;

/// 컬러(데몬 alive)·회색(dead) 트레이 아이콘 두 벌. setup 에서 1회 생성해 manage(state).
///
/// ★1회 생성 후 보관(load-bearing)★: 매 갱신마다 .ico 디코드/grayscale 변환을 다시 하면 낭비라
/// setup 에서 두 벌을 만들어 들고, refresh 때는 set_icon 으로 둘 중 하나를 교체만 한다. Image<'static>
/// 은 내부 Cow 라 clone 이 저렴(set_icon 이 소유를 요구해 갱신 시 복제).
pub struct TrayIcons {
    pub active: Image<'static>,
    pub inactive: Image<'static>,
}

/// 내장 컬러 아이콘(.ico). 배포 시 경로 의존 제거 위해 컴파일에 박는다(tray-host 와 동일 방식).
const ICON_ICO: &[u8] = include_bytes!("../../icons/icon.ico");

/// ensure_daemon(WMI spawn + 폴링) blocking 한계. discovery 내부 폴링 timeout 과 같은 5초.
const ENSURE_TIMEOUT: Duration = Duration::from_secs(5);

/// 트레이 아이콘 두 벌을 생성한다. 컬러 = .ico 디코드, 회색 = 컬러 RGBA 를 grayscale 변환.
///
/// .ico → RGBA 디코드는 Tauri Image::from_bytes(image-ico feature)로 한다(별도 image crate 불필요).
/// 회색은 디코드한 RGBA 버퍼를 core::to_grayscale_rgba 로 변환해 Image::new_owned 로 재구성.
fn build_icons() -> tauri::Result<TrayIcons> {
    // 컬러: .ico 바이트 → Image(rgba 디코드). image-ico feature 필요.
    let active = Image::from_bytes(ICON_ICO)?;
    let (w, h) = (active.width(), active.height());
    // 회색: 디코드된 RGBA 슬라이스를 desaturate. to_grayscale_rgba 는 len==w*h*4 전제(디코드가 보장).
    let gray_rgba = core::to_grayscale_rgba(active.rgba(), w, h);
    let inactive = Image::new_owned(gray_rgba, w, h);
    // 컬러도 owned 로 승격(Image::from_bytes 는 'static — ICON_ICO 가 'static 이라 OK, 명시 clone).
    let active = active.to_owned();
    Ok(TrayIcons { active, inactive })
}

/// 트레이를 생성·배선한다(setup 에서 1회). 아이콘 두 벌을 manage 하고, 메뉴·핸들러를 단다.
///
/// 메뉴(순서): 데몬 켜기 / 데몬 끄기 / UI 보이기 / UI 숨기기 / ──separator── / 완전 종료.
/// 메뉴 id 와 라벨은 core::MenuAction 에서(순수). 클릭 → action_for_menu_id → dispatch.
pub fn build_tray(app: &App) -> tauri::Result<()> {
    let icons = build_icons()?;
    // 초기 아이콘 = 회색(데몬 상태는 setup 직후 refresh 가 확정). 두 벌은 state 로 보관.
    let initial = icons.inactive.clone();
    app.manage(icons);

    // ── 메뉴 항목(core 의 id/label 사용 — 순수 분리) ──────────────────────────────────
    let handle = app.handle();
    let mi = |a: MenuAction| MenuItem::with_id(handle, a.menu_id(), a.label(), true, None::<&str>);
    let start = mi(MenuAction::StartDaemon)?;
    let stop = mi(MenuAction::StopDaemon)?;
    let show = mi(MenuAction::ShowUi)?;
    let hide = mi(MenuAction::HideUi)?;
    // ADR-0027 §55: 자동 시작은 체크 가능 항목(CheckMenuItem). 초기 체크 = 현재 레지스트리 등록 여부.
    // 등록 인자(--mode=daemon --hidden)는 init() 에서 박았고, 여기선 활성 여부만 읽어 체크에 반영.
    let autostart_action = MenuAction::ToggleAutostart;
    let autostart_checked = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        handle,
        autostart_action.menu_id(),
        autostart_action.label(),
        true,
        autostart_checked,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(handle)?;
    let quit = mi(MenuAction::QuitApp)?;
    // 순서: 데몬 켜기/끄기/UI 보이기/UI 숨기기/부팅 자동 시작/(구분선)/완전 종료 — core::ALL 과 일치.
    let menu = Menu::with_items(
        handle,
        &[&start, &stop, &show, &hide, &autostart, &sep, &quit],
    )?;

    // 토글 시 set_checked 동기화용으로 CheckMenuItem 핸들 보관(actions::toggle_autostart 가 재조회).
    app.manage(AutostartCheck(autostart));

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(initial)
        .tooltip("Engram")
        .menu(&menu)
        .on_menu_event(|app, event| dispatch_menu(app, event.id.as_ref()))
        .build(app)?;

    // setup 직후 데몬 상태로 아이콘 확정(컬러/회색).
    actions::refresh_tray_icon(&app.handle().clone());
    Ok(())
}

/// 메뉴 클릭 id → MenuAction → 동작. 모든 동작은 actions(공유 부수효과)만 호출.
///
/// ★데몬 켜기/끄기 = blocking → 워커(load-bearing)★: ensure/send_stop 은 WMI 폴링/WS 접속으로 수초
/// blocking. on_menu_event 는 메인 스레드라 직접 부르면 트레이·창이 그동안 얼어붙는다 → spawn_blocking
/// 워커로 보내고, 완료 후 refresh_tray_icon(메인 스레드 set_icon)으로 아이콘 갱신.
fn dispatch_menu(app: &AppHandle, menu_id: &str) {
    let Some(action) = core::action_for_menu_id(menu_id) else {
        tracing::warn!("[tray] 알 수 없는 메뉴 id: {menu_id}");
        return;
    };
    match action {
        MenuAction::StartDaemon => spawn_daemon_action(app, DaemonOp::Start),
        MenuAction::StopDaemon => spawn_daemon_action(app, DaemonOp::Stop),
        MenuAction::ShowUi => actions::show_main_ui(app),
        MenuAction::HideUi => actions::hide_main_ui(app),
        MenuAction::QuitApp => actions::quit_app(app),
        MenuAction::ToggleAutostart => actions::toggle_autostart(app),
    }
}

enum DaemonOp {
    Start,
    Stop,
}

/// 데몬 켜기/끄기를 워커에서 실행하고 완료 후 아이콘 갱신.
///
/// ★std::process::Command 로 데몬 직접 spawn 금지(ADR-0024 C1)★: ensure_daemon 은 내부에서 WMI
/// (Win32_Process.Create)로만 spawn 한다 — WmiPrvSE 자식이라 앱 Job(KILL_ON_JOB_CLOSE) 미상속 =
/// detached/breakaway 자동충족. 여기서 Command 직접 spawn 하면 앱 종료 시 데몬 동반 사살.
fn spawn_daemon_action(app: &AppHandle, op: DaemonOp) {
    let app = app.clone();
    // ★워커 panic 시 아이콘 갱신 누락 한계(reviewer Minor, 의도적 미가드)★: 아래 클로저가
    // set/refresh 도달 전에 panic 하면 이번 클릭의 아이콘 갱신은 일어나지 않는다. RAII drop 가드
    // (SignalOnDrop) 복원까지는 과하다 판단 — (1) 클로저 본문은 send_stop/ensure 호출 후 단순
    // match 뿐이라 panic 면이 거의 없고, (2) 일방 발사 재발사 모델이라 다음 켜기/끄기 클릭이 probe
    // 로 상태를 회수한다(아이콘 영구 고착 아님, 한 클릭 누락에 그침). 심각도 낮음 — 한계만 명시.
    tauri::async_runtime::spawn_blocking(move || {
        // data_dir 은 부팅 모드(AppState.mode 단일 출처, ADR-0027)로 산출. 트레이는 daemon 전용이라
        // 항상 Daemon 이지만 하드코딩 대신 AppState.mode 로 일관 조회.
        let mode = app.state::<crate::AppState>().mode;
        let data_dir = crate::discovery::default_data_dir(mode);
        match op {
            DaemonOp::Start => match crate::discovery::locate_daemon_exe() {
                Ok(exe) => {
                    // console=false: windowless. token 은 로그 금지(discovery 가 보장).
                    match crate::discovery::ensure_daemon(&data_dir, &exe, ENSURE_TIMEOUT, false) {
                        Ok(info) => tracing::info!(
                            pid = info.pid,
                            port = info.port,
                            "[tray] 데몬 ensure 완료"
                        ),
                        Err(e) => tracing::warn!("[tray] 데몬 ensure 실패: {e}"),
                    }
                }
                Err(e) => tracing::warn!("[tray] daemon exe 탐색 실패: {e}"),
            },
            DaemonOp::Stop => match crate::discovery::send_stop(&data_dir) {
                Ok(outcome) => {
                    tracing::info!(?outcome, "[tray] 데몬 graceful stop 발사");
                    // ★끄기 후 아이콘 확정은 StopOutcome 으로 분기(load-bearing — S13 race 재발 방지)★:
                    // DaemonClosed=연결닫힘=꺼짐확정 → probe 우회 회색 직접 set(probe 는 죽기 직전
                    // 수 ms 창에서 alive=true 를 줘 컬러 고착). Timeout/NoTarget=불확실 → probe 폴백.
                    // 이 분기는 impure 층(StopOutcome=discovery 타입)이라 core 가 아니라 여기 둔다
                    // (core 순수성 — core 는 IconState 만 안다).
                    match icon_state_for_stop_outcome(outcome) {
                        Some(state) => actions::set_tray_icon_state(&app, state),
                        None => actions::refresh_tray_icon(&app),
                    }
                    return; // Stop 은 위에서 아이콘을 확정했으니 아래 공통 refresh 를 타지 않는다.
                }
                Err(e) => tracing::warn!("[tray] 데몬 stop 실패: {e}"),
            },
        }
        // Start 및 Stop 실패(Err) 폴백: probe 기반 갱신(메인 스레드 set_icon 으로 post).
        // ★Stop 성공 경로는 위에서 return 으로 빠진다★ — StopOutcome 분기가 probe race 를 우회하므로
        // 여기 공통 refresh(probe)를 타면 안 된다. Start 는 alive 확정에 probe 가 맞다.
        actions::refresh_tray_icon(&app);
    });
}

/// [`StopOutcome`] → 끄기 후 아이콘 상태(impure 층 — discovery 타입 의존).
///
/// ★core 가 아니라 여기 있는 이유(load-bearing)★: StopOutcome 은 discovery 의 타입이라 core.rs
/// (tauri/discovery import 0, IconState 만 안다)에 넣으면 순수성이 깨진다. 그래서 이 매핑만 impure
/// 층에 둔다.
/// - `DaemonClosed`(연결 닫힘=꺼짐 확정) → `Some(Inactive)`: PID probe 우회하고 회색 직접 확정.
///   probe 는 데몬 exit 직전 수 ms 창에서 alive=true 를 돌려줘 아이콘이 컬러로 고착되는 race 가 있다.
/// - `Timeout | NoTarget`(불확실/끌 데몬 없음) → `None`: 호출자가 기존 probe 폴백(refresh)을 탄다.
fn icon_state_for_stop_outcome(outcome: StopOutcome) -> Option<IconState> {
    match outcome {
        StopOutcome::DaemonClosed => Some(IconState::Inactive),
        StopOutcome::Timeout | StopOutcome::NoTarget => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_outcome_daemon_closed_forces_gray_others_fall_back() {
        // DaemonClosed = 꺼짐 확정 → 회색(Inactive) 직접 set(probe 우회).
        assert_eq!(
            icon_state_for_stop_outcome(StopOutcome::DaemonClosed),
            Some(IconState::Inactive),
        );
        // Timeout/NoTarget = 불확실 → None(호출자가 probe 폴백 refresh).
        assert_eq!(icon_state_for_stop_outcome(StopOutcome::Timeout), None);
        assert_eq!(icon_state_for_stop_outcome(StopOutcome::NoTarget), None);
    }
}
