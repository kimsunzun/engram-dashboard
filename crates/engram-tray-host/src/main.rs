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
//! ## 이번 sub-step 범위 (S13 sub-step 2 "1차" — 켜기 + 상태→아이콘)
//! - 트레이 아이콘 + 메뉴 6개(데몬 켜기/끄기, UI 열기/닫기, 구분선, 트레이 종료, 완전 종료)를 띄운다.
//! - 메뉴 클릭 → core::action_for_menu_id → core::dispatch → core::causes_tray_exit 로 exit 판정.
//! - **데몬 켜기(ensure_daemon) = 실제 discovery 배선**([`RealLauncher`]/[`RealProbe`]). discovery
//!   는 WMI spawn + blocking 폴링(수 초)이라 메인 루프에서 직접 부르면 트레이가 멈춘다 → **워커
//!   `std::thread` 에서 호출하고 [`EventLoopProxy::send_event`] 로 결과를 메인 루프에 회수**한다.
//! - 상태→아이콘: probe→IconState→Active 면 컬러, Inactive 면 회색. ensure 완료/초기 진입 시
//!   probe 로 재확인해 [`TrayIcon::set_icon`] 으로 컬러/회색을 동적 교체한다.
//! - 툴팁은 앱 이름("Engram")만. 상태는 텍스트가 아니라 아이콘 색(컬러=활성/회색=비활성)으로.
//! ## S13 sub-step 2 "2차" — graceful 끄기 추가
//! - **데몬 끄기(stop_daemon) = 실제 discovery 배선**([`discovery::send_stop`]). send_stop 도 WS 접속
//!   (blocking)이라 ensure 와 똑같이 **워커 std::thread + [`EventLoopProxy`] 회수**로 돌린다(메인 루프
//!   블록 방지). 끄기 워커도 켜기와 같은 [`SignalOnDrop`] 가드로 [`UserEvent::DaemonStateChanged`] 를
//!   회수해 메인이 probe 로 아이콘을 재확인한다(panic 시에도 stale 고착 방지).
//! - **이번에도 의도적으로 안 한 것:** open_ui/close_ui/shutdown_all 은 **stub 동작 유지**(로그만).
//!   미구현이 아니라 *의도적으로 다음 단계로 미룬 것* — 다음 세션이 "안 채워졌다"고 오해해 지우거나
//!   임의 구현하지 말 것. 끄기의 ack 대기/타임아웃/taskkill 폴백도 미구현(send_stop 주석 참조).
//! - "트레이 종료"/"완전 종료" 는 (stop/shutdown stub 호출 후) 이벤트 루프 종료(causes_tray_exit==true).

// core 는 플랫폼 무관 순수 로직 — 항상 컴파일(테스트도 여기서 돈다).
mod core;

// 트레이 싱글 인스턴스 가드(named mutex). windows/OS 의존이라 core.rs 와 분리(core 는 순수 유지).
// non-windows 는 stub(트레이 GUI 자체가 Windows 전용). run() 진입 직후 체크한다.
mod instance;

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

    // discovery 호출은 GUI shell(이 모듈)에서만 — core.rs 는 이 의존을 import 하지 않는다(순수 분리).
    use engram_dashboard_discovery as discovery;

    use std::path::PathBuf;
    use std::time::Duration;

    use tao::event::{Event, StartCause};
    use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    // 아이콘을 컴파일에 박는다(배포 시 경로 의존 제거). 워크스페이스 기준 src-tauri/icons/icon.ico.
    const ICON_ICO: &[u8] = include_bytes!("../../../src-tauri/icons/icon.ico");

    /// ensure_daemon(WMI spawn + 폴링) 의 blocking 한계. discovery 내부 폴링 timeout 과 같은 5초.
    const ENSURE_TIMEOUT: Duration = Duration::from_secs(5);

    /// tao EventLoop 에 끼워 넣는 사용자 이벤트(워커 스레드 → 메인 루프 회수 채널).
    ///
    /// tray-icon 의 MenuEvent 채널을 EventLoopProxy 로 깨워 main 스레드에서 처리한다.
    /// (TrayIconEvent=좌클릭/hover 는 이번에 동작이 없어 포워딩 자체를 등록하지 않는다 — busy
    ///  wakeup 회피. 좌클릭 동작을 붙일 때 TrayEvent variant + 핸들러를 함께 되살린다.)
    enum UserEvent {
        /// tray-icon 전역 MenuEvent(메뉴 클릭) 포워딩.
        MenuEvent(MenuEvent),
        /// 데몬 상태가 바뀌었을 수 있으니 아이콘을 **probe 로 재확인**하라는 신호(워커 스레드가 보냄).
        ///
        /// ★왜 이 variant 가 필요한가(load-bearing)★: 데몬 켜기(ensure_daemon)는 discovery 가
        /// WMI spawn 후 daemon.json 을 **blocking 폴링**(최대 ENSURE_TIMEOUT 수초)한다. 이걸 tao
        /// 메인 루프(=트레이 UI 스레드)에서 직접 부르면 그 수초간 트레이가 얼어붙는다(메뉴 무응답).
        /// 그래서 ensure 는 워커 std::thread 에서 돌리고, **완료되면** 이 이벤트를 proxy 로 보내
        /// 메인 루프를 깨운다. 메인은 그때 probe 로 alive 를 재확인해 아이콘을 컬러/회색으로 교체한다
        /// (set_icon 은 메인 스레드 전용이라 워커가 직접 못 한다 → 회수 패턴 필수).
        ///
        /// 켜기(StartDaemon)·panic 폴백·끄기의 Timeout/NoTarget 이 이 경로를 쓴다(probe 가 진실원천).
        DaemonStateChanged,
        /// 데몬 **끄기 결과**를 메인에 직접 전달(probe 우회 — S13 sub-step 2 race 수정).
        ///
        /// ★왜 probe 우회가 필요한가(load-bearing)★: 끄기(send_stop)가 끝난 직후 `DaemonStateChanged`
        /// 로 probe(`daemon_status`=PID 가 OS 에 살아있나)를 다시 물으면, 데몬이 죽기 직전 수 ms 동안
        /// "아직 살아있음"으로 보여 **아이콘이 컬러로 고착**되는 race 가 있었다(QA 실측). 그래서 끄기
        /// 정상 종료 시엔 PID 를 다시 묻지 않고, send_stop 의 drain read 가 관측한
        /// [`StopOutcome`](연결 닫힘=꺼짐 확정)을 이 이벤트로 올려 메인이 **probe 없이** 아이콘을 결정한다:
        ///   - DaemonClosed → 회색 확정(연결 닫힘 = 꺼짐).
        ///   - Timeout/NoTarget → probe 폴백(refresh_icon — 불확실하니 진실원천 재확인).
        DaemonStopOutcome(discovery::StopOutcome),
    }

    /// 워커 스레드의 회수 신호를 **Drop 으로** 정확히 1회 보낸다(RAII 가드).
    ///
    /// ★왜 수동 send 가 아니라 Drop 인가(load-bearing)★: 워커 클로저 본문에서 마지막 줄에
    /// `send_event` 를 부르면, 작업이 그 전에 **panic** 하면 회수 신호가 영영 안 간다 → 데몬이
    /// 실제로 떴어도/죽었어도 메인이 아이콘 갱신 신호를 못 받아 **아이콘이 stale 로 고착**된다. 가드를
    /// 워커 진입 시 만들어 두면 정상 종료든 panic unwind 든 Drop 이 반드시 신호를 보낸다(정확히 1회).
    ///
    /// ★단일 경로 + 결과 교체(load-bearing — 이중 전송/race 방지)★: 회수는 이 가드의 Drop **한 곳**
    /// 으로만 한다(본문에서 따로 send 하지 않음). 기본 신호는 [`UserEvent::DaemonStateChanged`]
    /// (=probe 재확인 폴백). 워커가 정상 종료하며 더 정확한 신호(예: 끄기의 [`UserEvent::DaemonStopOutcome`])
    /// 를 보내고 싶으면, Drop 전에 [`SignalOnDrop::set_signal`] 로 보낼 이벤트를 **바꿔치기**한다.
    /// 그러면 정상 경로는 그 결과를, panic 경로는 기본 probe 폴백을 보낸다 — 둘이 동시에 가지 않아
    /// "끄기 결과로 회색 set 한 직후 폴백 probe 가 컬러로 다시 덮는" race 가 원천 차단된다.
    ///   - 켜기 워커: set_signal 안 함 → 항상 DaemonStateChanged(probe 가 진실원천).
    ///   - 끄기 워커: 정상 종료 시 set_signal(DaemonStopOutcome(결과)) → probe 우회. panic 시엔 기본
    ///     DaemonStateChanged 폴백(probe 로 안전 회수 — 끄기가 도중 죽어도 아이콘이 stale 안 됨).
    struct SignalOnDrop {
        proxy: EventLoopProxy<UserEvent>,
        // Drop 에서 보낼 이벤트. 기본 = DaemonStateChanged(probe 폴백). 정상 종료 시 set_signal 로 교체.
        // Option 인 이유: send_event 가 UserEvent 소유를 요구해 Drop 에서 take 로 꺼내 보낸다.
        signal: Option<UserEvent>,
    }
    impl SignalOnDrop {
        fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
            Self {
                proxy,
                signal: Some(UserEvent::DaemonStateChanged),
            }
        }
        /// 정상 종료 경로에서 Drop 이 보낼 신호를 교체(panic 시엔 호출 안 돼 기본 폴백 유지).
        fn set_signal(&mut self, ev: UserEvent) {
            self.signal = Some(ev);
        }
    }
    impl Drop for SignalOnDrop {
        fn drop(&mut self) {
            if let Some(ev) = self.signal.take() {
                let _ = self.proxy.send_event(ev);
            }
        }
    }

    /// 실제 데몬 발견/제어 Launcher — discovery(WMI) 로 배선.
    ///
    /// `data_dir` 은 트레이 시작 시 [`discovery::default_data_dir`] 로 1회 결정해 보관한다(매 호출
    /// 재계산 X — daemon·embedded·tray-host 세 프로세스가 같은 폴더를 보게 하는 단일 출처, ADR-0024).
    ///
    /// ★이번 1차 범위 = ensure_daemon(켜기)만 실제 구현★. stop/open/close/shutdown 은 **의도적으로**
    /// stub(로그만) 유지 — 2차에서 graceful 끄기로 구현 예정. 미구현이 아니라 *미룬 것*이므로 다음
    /// 세션이 빈 stub 을 보고 지우거나 임의 구현하지 말 것.
    struct RealLauncher {
        data_dir: PathBuf,
    }
    impl Launcher for RealLauncher {
        fn ensure_daemon(&self) -> Result<(), LaunchError> {
            // ★detached 강제(ADR-0024 C1)★: 데몬은 반드시 discovery(WMI Win32_Process.Create)로만
            // spawn 한다. std::process::Command 로 직접 띄우면 자식이 tray-host/Tauri 의
            // KILL_ON_JOB_CLOSE Job 을 상속해 부모가 죽을 때 함께 죽는다(데몬 영속 위반). WMI 는
            // WmiPrvSE 가 부모라 Job 미상속 = detached 자동충족 — 그래서 여기서 Command 직접 spawn 금지.
            let daemon_exe = discovery::locate_daemon_exe()
                .map_err(|e| LaunchError::EnsureDaemon(format!("daemon exe 탐색 실패: {e}")))?;
            // console=false: windowless(데몬도 콘솔 창 없이). 내부에서 daemon.json 폴링(blocking).
            discovery::ensure_daemon(&self.data_dir, &daemon_exe, ENSURE_TIMEOUT, false)
                .map(|info| {
                    // token 은 절대 로그하지 않는다(daemon.json ACL 채널로만 흐름 — discovery 보안 주석).
                    tracing::info!(
                        pid = info.pid,
                        port = info.port,
                        "[tray-host] 데몬 ensure 완료"
                    );
                })
                .map_err(|e| LaunchError::EnsureDaemon(e.to_string()))
        }
        // ── 데몬 끄기 = graceful StopDaemon WS 일방 발사(S13 sub-step 2 "2차") ──────────────
        fn stop_daemon(&self) -> Result<(), LaunchError> {
            // discovery::send_stop 이 daemon.json 을 읽어 살아있으면 ws://host:port 로 Auth →
            // StopDaemon{force:true} 를 보내고 닫는다(일방 발사 — ack 안 기다림). 끌 데몬이 없으면
            // (파일 없음/죽음) no-op Ok. token 은 send_stop 내부에서만 다뤄지고 로그/에러에 안 실린다.
            //
            // ★일방 발사의 아이콘 거동(사용자 결정 — load-bearing)★: 보낸 직후 데몬이 아직 정리
            //   중이면 probe 가 alive=true 라 아이콘이 잠깐 컬러로 남는다. 자동 재시도/타임아웃은
            //   없다 — 응답이 없으면 데몬은 그대로 활성으로 보이고, 사용자가 "데몬 끄기"를 다시
            //   누르면 재발사한다(StopDaemon 은 멱등에 가깝다 — 데몬은 받으면 shutdown_all+exit).
            //   강제 종료(taskkill=daemon_stop) 폴백·ack 대기는 send_stop 안에 나중에 붙는다.
            //
            // ★StopOutcome 은 여기서 버린다(load-bearing)★: 이 trait 메서드는 core::Launcher 계약상
            //   Result<(), LaunchError> 라 결과를 못 올린다(core.rs 는 discovery 의존 0 — StopOutcome
            //   을 알 수 없다). 그래서 아이콘 결정에 쓰는 **트레이 끄기 워커는 이 메서드를 거치지 않고
            //   discovery::send_stop 을 직접 호출**해 StopOutcome 을 받는다(main.rs 워커 본문). 이
            //   메서드는 결과가 불필요한 호출자(향후 ShutdownAll 단계·LLM 제어 표면)를 위해 남는다.
            discovery::send_stop(&self.data_dir)
                .map(|outcome| {
                    tracing::info!(?outcome, "[tray-host] 데몬 graceful stop(StopDaemon) 발사");
                })
                .map_err(|e| LaunchError::StopDaemon(e.to_string()))
        }
        // ── 아래 3개는 이번 범위 아님 — **의도적으로** stub(로그만) 유지. ──
        // (미구현 아님 — 다음 세션이 빈 stub 으로 오해해 지우거나 임의 구현하지 말 것.)
        fn open_ui(&self) -> Result<(), LaunchError> {
            tracing::info!("[tray-host] open_ui 호출됨(stub — 2차에서 구현 예정)");
            Ok(())
        }
        fn close_ui(&self) -> Result<(), LaunchError> {
            tracing::info!("[tray-host] close_ui 호출됨(stub — 2차에서 구현 예정)");
            Ok(())
        }
        fn shutdown_all(&self) -> Result<(), LaunchError> {
            tracing::info!(
                "[tray-host] shutdown_all 호출됨(stub — 2차 graceful 종료에서 구현 예정)"
            );
            Ok(())
        }
    }

    /// 실제 데몬 생존 판정 Probe — [`discovery::daemon_status`] 의 `alive` 만 본다.
    /// `data_dir` 은 RealLauncher 와 같은 단일 출처를 공유한다(시작 시 1회 결정).
    struct RealProbe {
        data_dir: PathBuf,
    }
    impl DaemonProbe for RealProbe {
        fn is_alive(&self) -> bool {
            discovery::daemon_status(&self.data_dir).alive
        }
    }

    /// 컬러/회색 두 Icon 을 들고, [`core::IconState`] 로 현재 표시할 아이콘을 고르는 한 벌.
    ///
    /// 동적 교체(set_icon)에서 매번 디코드하지 않도록 두 벌을 미리 만들어 보관한다. tray-icon 의
    /// `set_icon` 은 `Icon` 을 소유로 받으므로(Clone 불가) 교체 시마다 `.clone()` 한 사본을 넘긴다.
    struct Icons {
        color: Icon,
        gray: Icon,
    }
    impl Icons {
        /// icon.ico 의 첫 프레임을 RGBA 로 디코드해 (컬러, 회색) 두 벌을 만든다.
        ///
        /// 회색본은 core 의 순수 헬퍼 `to_grayscale_rgba` 로 RGBA 를 desaturate 해 만든다(image
        /// 의존은 여기 main 에만, core 는 슬라이스만 받음 — 격리 유지).
        fn load() -> Self {
            let img = image::load_from_memory(ICON_ICO)
                .expect("내장 icon.ico 디코드 실패")
                .into_rgba8();
            let (w, h) = img.dimensions();
            let rgba = img.into_raw();
            let color = Icon::from_rgba(rgba.clone(), w, h).expect("컬러 Icon::from_rgba 실패");
            let gray_rgba = core::to_grayscale_rgba(&rgba, w, h);
            let gray = Icon::from_rgba(gray_rgba, w, h).expect("회색 Icon::from_rgba 실패");
            Self { color, gray }
        }
        /// IconState 에 맞는 아이콘 사본을 돌려준다(Active=컬러 / Inactive=회색).
        /// set_icon 에 넘길 소유 Icon 이 필요하므로 clone(분기 자체는 순수 — icon_uses_color 로 테스트).
        fn for_state(&self, state: core::IconState) -> Icon {
            if icon_uses_color(state) {
                self.color.clone()
            } else {
                self.gray.clone()
            }
        }
    }

    /// 이 액션을 **워커 스레드**에서 돌려야 하는가(순수 분기 — 메인 루프 블록 방지 판정).
    ///
    /// 데몬 lifecycle 의 두 blocking 액션만 true: StartDaemon(WMI spawn + 폴링), StopDaemon
    /// (WS 접속/flush). 둘 다 수초 블록 가능해 워커로 보내고 DaemonStateChanged 로 회수한다.
    /// 나머지(open/close stub·QuitTray·ShutdownAll)는 즉시 끝나 메인에서 동기 dispatch 한다.
    ///
    /// Icon/스레드 같은 부수효과 없이 분기만 떼어내 단위테스트한다(워커 라우팅 회귀 방지) —
    /// 실제 spawn/WS 는 QA 실측 영역.
    fn daemon_action_runs_in_worker(action: MenuAction) -> bool {
        matches!(action, MenuAction::StartDaemon | MenuAction::StopDaemon)
    }

    /// IconState → "컬러 아이콘을 쓰는가"(순수 분기). Active=컬러(true), Inactive=회색(false).
    ///
    /// Icon 타입(OS 자원)을 거치지 않고 분기만 떼어내 단위테스트한다 — alive→IconState 매핑은 core
    /// 가, IconState→컬러/회색 선택은 이 함수가 검증한다(set_icon 교체 로직의 분기 회귀 방지).
    fn icon_uses_color(state: core::IconState) -> bool {
        matches!(state, core::IconState::Active)
    }

    /// 현재 probe 상태를 읽어 트레이 아이콘을 컬러/회색으로 교체한다(메인 스레드 전용).
    ///
    /// ★set_icon 은 메인 스레드에서만★: tray-icon 의 아이콘 갱신은 트레이를 생성한 메인 이벤트
    /// 루프 스레드에서 해야 한다 → 워커는 직접 못 부르고 DaemonStateChanged 로 메인을 깨워 여기로 온다.
    ///
    /// ★단발 갱신의 실제 거동(과신 금지 — load-bearing)★: 이 함수는 **그 순간의** probe 한 번으로
    /// 아이콘을 확정한다(주기 감지 없음). 실제 거동:
    ///   - ensure 가 Ok 면 — discovery::ensure_daemon 이 daemon.json 이 live 로 쓰일 때까지 폴링한 뒤
    ///     반환하므로, Ok 직후 probe 는 거의 항상 alive=true → 컬러로 정확히 갱신된다(안전).
    ///   - ensure 가 실패(Timeout: ENSURE_TIMEOUT 내 데몬이 daemon.json 못 씀) + **지연 부팅**이면 —
    ///     데몬이 6초째 늦게 살아나도 이 함수는 회수 신호 시점의 단발 probe 로 이미 **회색으로 확정**해
    ///     버린다. 이후 자가복구 경로가 없어 **아이콘이 회색에 고착**된다.
    /// 이는 버그가 아니라 **의도적 설계(사용자 결정)**: 주기 probe 는 추후, 지금은 단발 갱신 — 데몬이
    /// 외부 요인으로 죽거나 늦게 떠도 다음 사용자 액션(메뉴 클릭→재 refresh) 때 갱신된다. 다음 세션이
    /// 이 한계를 "버그"로 보고 임의로 주기 probe/타이머를 넣지 말 것(2차 owner 상태머신이 흡수, ADR-0024).
    fn refresh_icon(tray: &Option<tray_icon::TrayIcon>, icons: &Icons, probe: &dyn DaemonProbe) {
        if let Some(tray) = tray {
            let state = core::icon_state_from_probe(probe);
            set_icon_state(tray, icons, state);
        }
    }

    /// 트레이 아이콘을 **명시한 state** 로 교체한다(메인 스레드 전용, probe 우회).
    ///
    /// refresh_icon 은 probe(`daemon_status`=PID 생존)로 state 를 산출하지만, 끄기 직후엔 그 probe 가
    /// race 로 false-live(컬러 고착)를 낸다 → 끄기의 [`StopOutcome::DaemonClosed`](연결 닫힘=꺼짐
    /// 확정)일 땐 probe 를 건너뛰고 이 함수로 직접 [`IconState::Inactive`](회색)를 박는다.
    fn set_icon_state(tray: &tray_icon::TrayIcon, icons: &Icons, state: core::IconState) {
        if let Err(e) = tray.set_icon(Some(icons.for_state(state))) {
            tracing::error!("[tray-host] set_icon 실패: {e}");
        } else {
            tracing::debug!("[tray-host] 아이콘 갱신 — {state:?}");
        }
    }

    /// 끄기 결과(StopOutcome) → 아이콘을 어떻게 정할지(순수 분기 — probe 우회 여부 판정).
    ///
    /// DaemonClosed(연결 닫힘=꺼짐 확정)면 `Some(Inactive)` 로 **probe 없이 회색 확정**(race 우회).
    /// Timeout/NoTarget 은 불확실 → `None`(호출자가 probe 폴백 refresh_icon). Icon/스레드 부수효과
    /// 없이 분기만 떼어 단위테스트한다(race 우회 라우팅 회귀 방지).
    fn icon_state_for_stop_outcome(outcome: discovery::StopOutcome) -> Option<core::IconState> {
        match outcome {
            // 연결 닫힘 = 꺼짐 확정 → probe 없이 회색.
            discovery::StopOutcome::DaemonClosed => Some(core::IconState::Inactive),
            // 불확실(데몬이 아직 정리 중일 수 있음) / 끌 대상 없었음 → probe 폴백.
            discovery::StopOutcome::Timeout | discovery::StopOutcome::NoTarget => None,
        }
    }

    pub fn run() {
        // 로그 OFF 기본(RUST_LOG 로 켬) — 프로젝트 규약(기본 warn).
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .try_init();

        // ★트레이 싱글 인스턴스 가드(load-bearing) — 트레이 아이콘/이벤트 루프를 만들기 전에 체크★:
        //   두 번째 실행은 기존 트레이에 양보하고 **즉시 조용히 종료**한다(아이콘 중복 방지 —
        //   실측: 3번 실행 → 3개 프로세스·3개 아이콘이 쌓이던 문제). 데몬 instance.rs 와 같은
        //   named mutex 패턴이고 이름만 트레이 전용(`Global\EngramTrayHost-<user>`, 데몬과 충돌 X).
        //   data_dir 결정·EventLoop 생성보다 앞에서 판정해 두 번째 인스턴스가 자원을 만들기 전에 빠진다.
        //   ★가드 핸들 수명★: 반환된 `_guard` 는 이 run() 스코프 변수로 잡혀 함수가 사는 동안(=프로세스
        //   수명 내내) 살아 있어야 한다 — drop 되면 mutex 가 풀려 다른 인스턴스가 진입 가능. run() 은
        //   event_loop.run(...) 으로 끝에서 diverge(!)하므로 _guard 가 이 스코프에 묶여 끝까지 산다.
        //   프로세스 종료 시 OS 가 mutex 를 자동 해제하므로 별도 정리 코드는 불필요.
        //   ★Err 시 강행(load-bearing) — 데몬 instance 정책을 트레이에 복사하지 말 것★: 트레이는
        //   데몬과 달리 단일성이 데이터 정합성이 아니라 UX(아이콘 중복) 문제다. 그래서 가드 생성
        //   실패(CreateMutexW 시스템 오류, 매우 드묾) 시 미기동(아이콘 부재 = 데몬/UI 제어 진입점을
        //   통째로 상실)보다 강행(중복 위험 감수)이 사용자에게 낫다 — 아이콘 부재 > 아이콘 중복.
        //   타입은 Option<InstanceGuard>: None 이어도 run() 동안 그대로 살아 무해하다(아무것도 안 함).
        //   ★`_guard` 이름 바인딩 유지(언더스코어 `_` 단독 금지)★ — `let _ = ...` 이면 가드가 즉시
        //   drop 돼 mutex 가 곧장 풀린다. 이름 있는 변수여야 run() 끝까지 산다.
        let _guard: Option<crate::instance::InstanceGuard> = match crate::instance::acquire() {
            Ok(Some(guard)) => Some(guard), // 우리가 첫 트레이 인스턴스 — 가드를 끝까지 보유.
            Ok(None) => {
                // 이미 다른 트레이가 떠 있음 — 기존에 양보하고 즉시 종료(표준 싱글 인스턴스 동작).
                tracing::info!("[tray-host] 이미 트레이 인스턴스가 떠 있어 종료");
                std::process::exit(0);
            }
            Err(e) => {
                // mutex 생성 자체가 실패(시스템 오류) — 위 주석대로 미기동보다 강행이 낫다.
                // 경고만 남기고 가드 없이(None) 계속 진행한다(중복 방지는 못 하지만 트레이는 뜬다).
                tracing::warn!(
                    "[tray-host] 싱글 인스턴스 가드 생성 실패 — 중복 방지 없이 강행: {e}"
                );
                None
            }
        };

        // data_dir(.engram-data 절대경로)을 **시작 시 1회** 결정해 Launcher/Probe 가 공유한다(매
        // 호출 재계산 X — daemon·embedded·tray-host 세 프로세스가 같은 폴더를 보는 단일 출처, ADR-0024).
        let data_dir = discovery::default_data_dir();
        tracing::info!(data_dir = %data_dir.display(), "[tray-host] data_dir 결정");

        // real probe 로 현재 데몬 상태를 읽어 초기 아이콘 상태를 정한다(이미 떠 있으면 컬러).
        // 상태는 텍스트 툴팁이 아니라 아이콘 색으로 보여준다 → 툴팁은 앱 이름만.
        let probe = RealProbe {
            data_dir: data_dir.clone(),
        };
        let launcher = RealLauncher {
            data_dir: data_dir.clone(),
        };
        let tooltip = "Engram";

        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

        // tray-icon 의 전역 MenuEvent 채널 → EventLoopProxy 로 포워딩(main 스레드에서 처리).
        // TrayIconEvent(좌클릭/hover)는 처리할 동작이 없어 핸들러를 등록하지 않는다 — 등록하면
        // 매 마우스 이벤트가 루프를 깨우고 버려진다(busy wakeup). 좌클릭 동작 추가 시 되살린다.
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.send_event(UserEvent::MenuEvent(e));
        }));

        // 워커 스레드(ensure_daemon)가 완료 결과를 메인 루프로 회수할 때 쓸 별도 proxy 핸들.
        // EventLoopProxy 는 Clone 이라 워커마다 복제해 넘긴다.
        let worker_proxy: EventLoopProxy<UserEvent> = event_loop.create_proxy();

        // 두 아이콘(컬러/회색)을 미리 만들어 set_icon 교체에 재사용(디코드 1회).
        let icons = Icons::load();

        // 트레이 아이콘은 이벤트 루프 진입 후에도 살아있도록 소유를 유지한다(드롭되면 아이콘 사라짐).
        // ★실제 생성은 아래 run() 클로저의 StartCause::Init arm 에서 한다 — tray-icon 0.24.1 문서:
        //   "On Windows and Linux, an event loop must be running on the thread ... the earliest you
        //   can create icons is on StartCause::Init." build() 를 run() 전에 부르면 객체는 생기지만
        //   아이콘이 taskbar 에 등록되지 않아 보이지 않는다(이 버그의 근본 원인).
        let mut tray_icon: Option<tray_icon::TrayIcon> = None;

        // probe/launcher/icons/data_dir/worker_proxy 는 아래 클로저로 move 캡처.
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

                // 초기 아이콘: real probe 로 현재 데몬 상태를 읽어 Active=컬러 / Inactive=회색.
                // (데몬이 이미 떠 있으면 컬러로 시작. 이후 켜기/상태변화 시 DaemonStateChanged → refresh_icon.)
                let icon_state = core::icon_state_from_probe(&probe);
                let initial_icon = icons.for_state(icon_state);

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

            // 워커(켜기/폴백)가 완료를 알리면 probe 로 재확인해 아이콘을 컬러/회색으로 교체.
            // set_icon 은 메인 스레드 전용이라 이 회수 지점(메인)에서만 부른다.
            if let Event::UserEvent(UserEvent::DaemonStateChanged) = event {
                refresh_icon(&tray_icon, &icons, &probe);
                return;
            }

            // 끄기 워커가 send_stop 결과(StopOutcome)를 올리면, probe race 를 우회해 아이콘을 결정한다.
            //   - DaemonClosed → probe 없이 회색 확정(연결 닫힘 = 꺼짐). PID probe 의 false-live race 회피.
            //   - Timeout/NoTarget → probe 폴백(refresh_icon — 불확실하니 진실원천 재확인).
            if let Event::UserEvent(UserEvent::DaemonStopOutcome(outcome)) = event {
                match icon_state_for_stop_outcome(outcome) {
                    Some(state) => {
                        if let Some(tray) = &tray_icon {
                            set_icon_state(tray, &icons, state);
                        }
                    }
                    None => refresh_icon(&tray_icon, &icons, &probe),
                }
                return;
            }

            if let Event::UserEvent(UserEvent::MenuEvent(menu_event)) = event {
                // 클릭 id → 의도 → 디스패치(전부 core 순수 함수). 알 수 없는 id 는 무시.
                let Some(action) = core::action_for_menu_id(menu_event.id.as_ref()) else {
                    tracing::debug!("[tray-host] unknown menu id: {:?}", menu_event.id);
                    return;
                };
                // ★비동기 워커 패턴(메인 루프 블록 방지)★: 데몬 lifecycle 두 액션이 blocking 이다 —
                //   StartDaemon(ensure_daemon)은 WMI spawn + daemon.json 폴링(최대 ENSURE_TIMEOUT 수초),
                //   StopDaemon(send_stop)은 ws://host:port 접속(blocking connect/flush). 메인 스레드(tao
                //   이벤트 루프 = 트레이 UI)에서 직접 동기 호출하면 그동안 트레이가 얼어붙는다 → **둘 다
                //   워커 std::thread 에서 호출**하고 완료 시 DaemonStateChanged 를 proxy 로 보내 메인이
                //   아이콘을 갱신한다. 나머지 액션(open/close 의 stub, QuitTray/ShutdownAll)은 즉시 끝나
                //   (로그/no-op) 메인에서 동기 dispatch 해도 안 막힌다.
                if daemon_action_runs_in_worker(action) {
                    // data_dir 을 워커로 move(PathBuf=Send). 워커에서 RealLauncher 를 새로 구성해
                    // 해당 액션을 dispatch 한다(core::dispatch 와 동일 경로지만 워커 스레드라 비동기).
                    //
                    // ★연타 안전(load-bearing)★:
                    //   - 켜기 연타 → 클릭마다 워커가 각자 discovery::ensure_daemon → 각자 WMI spawn 하나,
                    //     데몬측 `Global\EngramDashboardDaemon-<user>` named mutex 가 **첫 데몬만 살리고
                    //     나머지는 self-exit** 시킨다(daemon.json 보호는 이중 안전망). data_dir/Probe 는
                    //     PathBuf 값 복제라 워커 간 공유 상태 race 도 없다 — in-flight 가드 불필요.
                    //   - 끄기 연타/없는 데몬에 끄기 → send_stop 이 daemon.json 없음/죽음을 no-op Ok 로
                    //     흡수하고, StopDaemon 은 데몬이 받으면 shutdown_all+exit(사실상 멱등)이라 재발사가
                    //     안전하다. 그래서 두 액션 모두 in-flight 가드 없이 정합성이 깨지지 않는다.
                    //   다음 세션이 이 mutex/no-op 의존성을 모르고 임의 in-flight AtomicBool 을 넣거나
                    //   데몬측 mutex 를 지우지 말 것. in-flight 가드/주기 동기화는 2차 owner 상태머신
                    //   (ADR-0024 C4)이 흡수한다 — 지금은 위 두 장치가 정합성을 보장한다.
                    let worker_data_dir = data_dir.clone();
                    let proxy = worker_proxy.clone();
                    std::thread::spawn(move || {
                        // ★회수 신호 RAII 가드(켜기·끄기 공용)★: 클로저 진입 즉시 가드를 만들어 두면
                        //   아래 작업이 정상 끝나든 panic 으로 unwind 하든 Drop 이 신호를 정확히 1회 보낸다
                        //   (워커 panic 시에도 아이콘 stale 고착 방지). 본문에서 직접 send 하지 않는다 —
                        //   회수는 가드 Drop 단일 경로(이중 전송/race 방지, SignalOnDrop 주석).
                        //   기본 신호 = DaemonStateChanged(probe 폴백). 끄기 정상 종료 시 set_signal 로
                        //   DaemonStopOutcome(결과)로 바꿔 probe 우회한다.
                        let mut signal = SignalOnDrop::new(proxy);
                        match action {
                            // ── 끄기: send_stop 결과(StopOutcome)로 아이콘 결정(probe race 우회) ──
                            // core::dispatch(stop_daemon)는 결과를 버려 race 를 못 막는다 → discovery
                            // 를 직접 호출해 StopOutcome 을 받는다(트레이 shell 은 discovery 의존 가능,
                            // core.rs 순수성과 무관). 정상 종료면 set_signal 로 그 결과를 메인에 올린다.
                            MenuAction::StopDaemon => {
                                match discovery::send_stop(&worker_data_dir) {
                                    Ok(outcome) => {
                                        // token 미노출(send_stop 내부에서만 다룸 — 여기엔 outcome enum 만).
                                        tracing::info!(
                                            ?outcome,
                                            "[tray-host] 데몬 graceful stop 결과"
                                        );
                                        // 정상 종료 → probe 우회 신호로 교체. DaemonClosed 면 메인이 회색
                                        // 확정, Timeout/NoTarget 이면 메인이 probe 폴백(아래 메인 arm).
                                        signal.set_signal(UserEvent::DaemonStopOutcome(outcome));
                                    }
                                    Err(e) => {
                                        // 송신/접속 실패 — set_signal 안 함 → 기본 DaemonStateChanged(probe
                                        // 폴백)로 안전 회수. token 은 DiscoveryError 에 안 실린다(discovery 보안).
                                        tracing::error!("[tray-host] 데몬 끄기(worker) 실패: {e}");
                                    }
                                }
                            }
                            // ── 켜기(및 그 외 워커 액션): 기존 dispatch + probe 폴백 경로 유지 ──
                            // ensure 는 성공해도 실패해도 probe 가 진실원천이라 가드 기본 신호
                            // (DaemonStateChanged)로 메인이 재확인한다(set_signal 안 함).
                            _ => {
                                let l = RealLauncher {
                                    data_dir: worker_data_dir,
                                };
                                if let Err(e) = core::dispatch(action, &l) {
                                    tracing::error!(
                                        "[tray-host] 데몬 액션(worker) 실패 [{action:?}]: {e}"
                                    );
                                }
                            }
                        }
                    });
                } else if let Err(e) = core::dispatch(action, &launcher) {
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

    // ── 테스트 (순수 분기만 — discovery 실호출/set_icon 은 통합 영역, QA 실측) ─────────────
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn icon_uses_color_active_gray_inactive() {
            // IconState→컬러/회색 선택 분기(set_icon 교체 로직의 핵심). Active=컬러, Inactive=회색.
            // alive→IconState 매핑은 core 가, 이 선택 분기는 여기가 검증한다(이중으로 박아 회귀 차단).
            assert!(
                icon_uses_color(core::IconState::Active),
                "Active 는 컬러 아이콘"
            );
            assert!(
                !icon_uses_color(core::IconState::Inactive),
                "Inactive 는 회색 아이콘"
            );
        }

        #[test]
        fn icon_uses_color_tracks_alive_through_core_mapping() {
            // 결합 검증: alive bool → core::icon_state_for → icon_uses_color 의 전 경로가
            // "alive면 컬러, dead면 회색"으로 일관(중간 매핑이 뒤집히면 여기서 잡힌다).
            assert!(icon_uses_color(core::icon_state_for(true)));
            assert!(!icon_uses_color(core::icon_state_for(false)));
        }

        #[test]
        fn stop_outcome_daemon_closed_forces_gray_others_fall_back() {
            // S13 sub-step 2 race 수정의 핵심 분기: DaemonClosed(연결 닫힘=꺼짐 확정)는 probe 우회로
            // 회색(Inactive) 확정, Timeout/NoTarget 은 probe 폴백(None). 이 라우팅이 뒤집히면 끄기 후
            // 아이콘이 컬러로 고착(원래 버그)되거나, 불확실 상태를 멋대로 회색 확정해 버린다.
            assert_eq!(
                icon_state_for_stop_outcome(discovery::StopOutcome::DaemonClosed),
                Some(core::IconState::Inactive),
                "DaemonClosed 는 probe 없이 회색 확정"
            );
            assert_eq!(
                icon_state_for_stop_outcome(discovery::StopOutcome::Timeout),
                None,
                "Timeout 은 probe 폴백(None)"
            );
            assert_eq!(
                icon_state_for_stop_outcome(discovery::StopOutcome::NoTarget),
                None,
                "NoTarget 은 probe 폴백(None)"
            );
        }

        #[test]
        fn daemon_lifecycle_actions_run_in_worker() {
            // blocking 한 데몬 lifecycle 두 액션(켜기=WMI+폴링, 끄기=WS 접속)만 워커로 — 나머지는
            // 메인 동기. 이 분기가 깨지면 메인 루프가 수초 블록(트레이 멈춤)되거나 반대로 즉시 끝나는
            // 액션을 불필요하게 워커로 보낸다.
            assert!(daemon_action_runs_in_worker(MenuAction::StartDaemon));
            assert!(daemon_action_runs_in_worker(MenuAction::StopDaemon));
            assert!(!daemon_action_runs_in_worker(MenuAction::OpenUi));
            assert!(!daemon_action_runs_in_worker(MenuAction::CloseUi));
            assert!(!daemon_action_runs_in_worker(MenuAction::QuitTray));
            assert!(!daemon_action_runs_in_worker(MenuAction::ShutdownAll));
        }
    }
}
