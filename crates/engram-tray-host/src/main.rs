// 트레이 런처는 콘솔 없이 로그인 상주하는 게 정상 동작이다 — debug/release 모두 windowless(GUI
// subsystem)로 빌드해 콘솔 창을 절대 띄우지 않는다. 디버깅 로그가 필요하면 stdout/stderr 를
// 리다이렉트해 잡는다(GUI subsystem 이어도 부모가 핸들을 주면 출력은 그대로 흐른다 — QA 가 파일
// 리다이렉트로 캡처). 콘솔 창 팝업은 불필요하므로 무조건 끈다.
#![windows_subsystem = "windows"]

//! engram-tray-host — WebView 없는 순수 Rust 트레이 호스트(상주 런처). ADR-0023/0024/0025.
//!
//! 이 파일은 **얇은 GUI shell** 이다 — tao 이벤트 루프 + tray-icon 배선만 담고, 모든 판단
//! (메뉴 의도 매핑·디스패치·아이콘/라벨 상태)은 [`core`] 의 순수 함수에 위임한다(단위테스트 대상).
//!
//! Windows 전용(tray-icon/tao). 트레이는 메인 스레드 이벤트 루프와 같은 스레드여야 한다.
//!
//! ## 이번 sub-step 범위
//! - 트레이 아이콘 + 메뉴 6개(데몬 켜기/끄기, UI 열기/닫기, 구분선, 트레이 종료, 완전 종료)를 띄운다.
//! - 메뉴 클릭 → core::action_for_menu_id → core::dispatch → core::causes_tray_exit 로 exit 판정.
//! - Launcher 는 **stub**(로그만, 실제 spawn 금지 — 다음 sub-step 에서 discovery 로 구현).
//! - DaemonProbe 도 **stub**(항상 false) — 초기 아이콘 상태(회색)를 산출하는 경로만 배선.
//! - 툴팁은 앱 이름("Engram")만. 상태는 텍스트가 아니라 아이콘 색(컬러=활성/회색=비활성)으로.
//! - 초기 아이콘: probe→IconState→Active 면 컬러, Inactive 면 회색. stub probe=false → 회색(의도).
//! - "트레이 종료"/"완전 종료" 는 stub 호출 후 이벤트 루프 종료(causes_tray_exit==true).

// core 는 플랫폼 무관 순수 로직 — 항상 컴파일(테스트도 여기서 돈다).
mod core;

#[cfg(windows)]
fn main() {
    windows_main::run();
}

#[cfg(not(windows))]
fn main() {
    // 트레이 GUI 는 Windows 전용. 비-Windows 에서는 빌드는 되지만 실행은 안내만.
    eprintln!("engram-tray-host 는 Windows 전용입니다(트레이 GUI). core 로직만 단위테스트 가능.");
}

#[cfg(windows)]
mod windows_main {
    use crate::core::{self, DaemonProbe, LaunchError, Launcher, MenuAction};

    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    // 아이콘을 컴파일에 박는다(배포 시 경로 의존 제거). 워크스페이스 기준 src-tauri/icons/icon.ico.
    const ICON_ICO: &[u8] = include_bytes!("../../../src-tauri/icons/icon.ico");

    /// tao EventLoop 에 트레이 메뉴 이벤트를 끼워 넣기 위한 사용자 이벤트.
    /// tray-icon 의 MenuEvent 채널을 EventLoopProxy 로 깨워 main 스레드에서 처리한다.
    /// (TrayIconEvent=좌클릭/hover 는 이번에 동작이 없어 포워딩 자체를 등록하지 않는다 — busy
    ///  wakeup 회피. 좌클릭 동작을 붙일 때 TrayEvent variant + 핸들러를 함께 되살린다.)
    enum UserEvent {
        MenuEvent(MenuEvent),
    }

    /// stub Launcher — 실제 spawn 없이 로그만(다음 sub-step 에서 discovery 로 구현).
    struct StubLauncher;
    impl Launcher for StubLauncher {
        fn ensure_daemon(&self) -> Result<(), LaunchError> {
            tracing::info!(
                "[tray-host] ensure_daemon 호출됨(stub) — 실제 데몬 spawn 은 다음 sub-step"
            );
            Ok(())
        }
        fn stop_daemon(&self) -> Result<(), LaunchError> {
            tracing::info!(
                "[tray-host] stop_daemon 호출됨(stub) — 실제 데몬 graceful stop 은 다음 sub-step"
            );
            Ok(())
        }
        fn open_ui(&self) -> Result<(), LaunchError> {
            tracing::info!("[tray-host] open_ui 호출됨(stub) — 실제 UI spawn 은 다음 sub-step");
            Ok(())
        }
        fn close_ui(&self) -> Result<(), LaunchError> {
            tracing::info!("[tray-host] close_ui 호출됨(stub) — 실제 UI 종료는 다음 sub-step");
            Ok(())
        }
        fn shutdown_all(&self) -> Result<(), LaunchError> {
            tracing::info!(
                "[tray-host] shutdown_all 호출됨(stub) — 실제 graceful 종료는 다음 sub-step"
            );
            Ok(())
        }
    }

    /// stub DaemonProbe — 실제 probe(discovery::daemon_status)는 다음 sub-step. 지금은 항상 죽음.
    struct StubProbe;
    impl DaemonProbe for StubProbe {
        fn is_alive(&self) -> bool {
            false
        }
    }

    /// icon.ico 의 첫 프레임을 RGBA 로 디코드해 (컬러 Icon, 회색 Icon) 두 벌을 만든다.
    ///
    /// 컬러 = 데몬 활성(IconState::Active), 회색 = 비활성(Inactive). 회색본은 core 의 순수 헬퍼
    /// `to_grayscale_rgba` 로 RGBA 를 desaturate 해 만든다(image 의존은 여기 main 에만, core 는
    /// 슬라이스만 받음 — 격리 유지). 동적 교체(set_icon)는 sub-step 2.
    fn load_icons() -> (Icon, Icon) {
        let img = image::load_from_memory(ICON_ICO)
            .expect("내장 icon.ico 디코드 실패")
            .into_rgba8();
        let (w, h) = img.dimensions();
        let rgba = img.into_raw();
        let color = Icon::from_rgba(rgba.clone(), w, h).expect("컬러 Icon::from_rgba 실패");
        let gray_rgba = core::to_grayscale_rgba(&rgba, w, h);
        let gray = Icon::from_rgba(gray_rgba, w, h).expect("회색 Icon::from_rgba 실패");
        (color, gray)
    }

    pub fn run() {
        // ★sub-step 2: data_dir(.engram-data 절대경로)를 여기서 결정해 real Launcher/Probe
        //   생성자에 주입할 자리. resolve_data_dir 훅 예정.

        // 로그 OFF 기본(RUST_LOG 로 켬) — 프로젝트 규약(기본 warn).
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .try_init();

        // stub probe 로 현재 데몬 상태(이번엔 항상 false)를 읽어 초기 아이콘 상태를 정한다.
        // core 의 순수 매핑을 거치는 경로를 실제로 배선(IconState 자체 테스트는 core 에 있음).
        // 상태는 텍스트 툴팁이 아니라 아이콘 색으로 보여준다 → 툴팁은 앱 이름만.
        let probe = StubProbe;
        let icon_state = core::icon_state_from_probe(&probe);
        let tooltip = "Engram";

        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

        // tray-icon 의 전역 MenuEvent 채널 → EventLoopProxy 로 포워딩(main 스레드에서 처리).
        // TrayIconEvent(좌클릭/hover)는 처리할 동작이 없어 핸들러를 등록하지 않는다 — 등록하면
        // 매 마우스 이벤트가 루프를 깨우고 버려진다(busy wakeup). 좌클릭 동작 추가 시 되살린다.
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(UserEvent::MenuEvent(e));
        }));

        let launcher = StubLauncher;

        // 트레이 아이콘은 이벤트 루프 진입 후에도 살아있도록 소유를 유지한다(드롭되면 아이콘 사라짐).
        // ★실제 생성은 아래 run() 클로저의 StartCause::Init arm 에서 한다 — tray-icon 0.24.1 문서:
        //   "On Windows and Linux, an event loop must be running on the thread ... the earliest you
        //   can create icons is on StartCause::Init." build() 를 run() 전에 부르면 객체는 생기지만
        //   아이콘이 taskbar 에 등록되지 않아 보이지 않는다(이 버그의 근본 원인).
        let mut tray_icon: Option<tray_icon::TrayIcon> = None;

        // tooltip/launcher 는 아래 클로저로 move 캡처(Init 시점에 메뉴/아이콘/툴팁을 조립).
        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;

            // tao 는 run() 진입 직후 NewEvents(Init) 를 정확히 1회 보낸다(0.31 event.rs: "Sent once,
            // immediately after run is called"). 이벤트 루프가 도는 이 시점에 트레이를 생성해야
            // 아이콘이 실제 taskbar 에 등록된다.
            if let Event::NewEvents(StartCause::Init) = event {
                // 메뉴 6개 — id/label 은 core::MenuAction 단일 출처(클릭 매핑이 같은 id 를 본다).
                // 표시 순서: 데몬 켜기, 데몬 끄기, UI 열기, UI 닫기, (구분선), 트레이 종료, 완전 종료.
                // 이번 sub-step 은 6개 전부 enabled — 데몬·UI 상태 기반 enable/disable 동적 제어는
                // real probe 가 필요해 sub-step 2(이번엔 순수 판정 causes_tray_exit 만 도입).
                fn item(a: MenuAction) -> MenuItem {
                    MenuItem::with_id(a.menu_id(), a.label(), true, None)
                }
                let item_start = item(MenuAction::StartDaemon);
                let item_stop = item(MenuAction::StopDaemon);
                let item_open = item(MenuAction::OpenUi);
                let item_close = item(MenuAction::CloseUi);
                let sep = PredefinedMenuItem::separator();
                let item_quit_tray = item(MenuAction::QuitTray);
                let item_shutdown = item(MenuAction::ShutdownAll);

                let menu = Menu::new();
                menu.append_items(&[
                    &item_start,
                    &item_stop,
                    &item_open,
                    &item_close,
                    &sep,
                    &item_quit_tray,
                    &item_shutdown,
                ])
                .expect("메뉴 항목 추가 실패");

                // 초기 아이콘: Active=컬러 / Inactive=회색. stub probe=false → 회색(의도 — 회색 확인용).
                // ★sub-step 2: 데몬 재발견/상태 변화 시 tray.set_icon(color|gray) 로 동적 교체.
                let (color_icon, gray_icon) = load_icons();
                let initial_icon = match icon_state {
                    core::IconState::Active => color_icon,
                    core::IconState::Inactive => gray_icon,
                };

                tray_icon = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip(tooltip)
                        .with_icon(initial_icon)
                        .build()
                        .expect("트레이 아이콘 생성 실패"),
                );

                tracing::info!("[tray-host] 트레이 시작 — {tooltip} (icon_state={icon_state:?})");
                return;
            }

            if let Event::UserEvent(UserEvent::MenuEvent(menu_event)) = event {
                // 클릭 id → 의도 → 디스패치(전부 core 순수 함수). 알 수 없는 id 는 무시.
                let Some(action) = core::action_for_menu_id(menu_event.id.as_ref()) else {
                    tracing::debug!("[tray-host] unknown menu id: {:?}", menu_event.id);
                    return;
                };
                // ★sub-step 2: real Launcher 의 ensure_daemon 은 discovery::ensure_daemon 으로
                //   내부 blocking 폴링(최대 timeout 수초)한다. 메인 스레드(tao 이벤트 루프)에서
                //   직접 동기 호출하면 트레이 UI 가 그 시간 멈춘다 → 워커 스레드 실행 + 결과를
                //   EventLoopProxy::send_event 로 회수하는 패턴 필요.
                if let Err(e) = core::dispatch(action, &launcher) {
                    tracing::error!("[tray-host] dispatch 실패: {e}");
                }
                // 트레이 프로세스를 종료시키는 액션(QuitTray·ShutdownAll)인지 core 순수 판정에 위임.
                //   - QuitTray: 트레이만 종료(데몬·UI 는 detached 로 계속 — Launcher 무호출).
                //   - ShutdownAll: 데몬+UI graceful(stub no-op) 후 트레이도 종료.
                // 둘 다 이번 stub 에선 즉시 Exit. 실제 graceful 순서는 다음 sub-step.
                //
                // ★sub-step 2(ADR-0024 C4): ShutdownAll 은 즉시 Exit 가 아니라 owner=Stopping
                //   (ensure/open 차단) → UI full_shutdown → 데몬 graceful drain(ack+타임아웃) →
                //   데몬 exit 확인 → UI 종료 → tray-host 종료의 다단계 비동기. 그때
                //   (a)전역 MenuEvent 핸들러 set_event_handler(None) 해제
                //   (b)종료 진행 플래그로 추가 클릭 무시
                //   (c)아래 ControlFlow::Exit 를 drain 완료 후로 이동.
                //   QuitTray 는 graceful 대상이 없어 sub-step 2 에서도 즉시 Exit 유지.
                //   현재 stub 은 둘 다 즉시 종료라 잠복.
                //   ★주의: core::causes_tray_exit 가 지금 ShutdownAll=true 로 묶지만 이는 임시
                //   계약 — sub-step 2 에서 ShutdownAll 은 이 bool 경로를 떠나 위 C4 상태머신으로
                //   분기한다(이 if 분기에는 QuitTray 만 남는다). core.rs 의 같은 마커 참조.
                if core::causes_tray_exit(action) {
                    tray_icon.take(); // 아이콘 즉시 제거(소유 드롭).
                    *control_flow = ControlFlow::Exit;
                }
            }
        });
    }
}
