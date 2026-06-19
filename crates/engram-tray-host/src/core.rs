//! tray-host core — 트레이 동작의 **순수 로직**(OS/GUI/네트워크 무의존).
//!
//! ## 설계 — seam(trait) 뒤로 부수효과를 끊는다 (ADR-0023 / CLAUDE.md 아키텍처 원칙)
//! 트레이의 실제 효과(데몬 발견·spawn·UI 열기/닫기·전체 종료)는 전부 trait 으로 주입한다:
//!   - [`DaemonProbe`] — 데몬 생존 판정. 다음 sub-step 에서 `discovery::daemon_status` 로 구현.
//!   - [`Launcher`] — 데몬 ensure/stop · UI open/close · 전체 종료. 다음 sub-step 에서
//!     실제 spawn·taskkill·graceful 종료로 구현.
//!
//! 이번 sub-step 은 **seam 정의 + 순수 매핑/디스패치 함수 + 단위테스트** 까지다. 실제 구현체는
//! main.rs 의 stub 만 둔다(로그만, 실제 프로세스 조작 없음). 디스패치 로직이 Launcher trait 위에서만
//! 동작하므로, Fake 를 주입해 "어떤 메뉴가 어떤 Launcher 메서드를 부르는지" 를 OS 없이 검증한다
//! (discovery.rs 의 Cell/RefCell 카운터 Fake 스타일과 동일).

// ── 에러 ───────────────────────────────────────────────────────────────────────

/// Launcher 실패. 실제 구현(다음 sub-step)에서 spawn/taskkill/io 실패를 이 enum 으로 승격한다.
/// 이번엔 정의만 — stub 은 Ok 만 반환하므로 일부 variant 는 아직 생성처가 없다(다음 sub-step seam).
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // 대부분 variant 는 다음 sub-step 의 real Launcher 가 생성한다.
pub enum LaunchError {
    /// 데몬 ensure(detached spawn) 실패.
    #[error("데몬 ensure 실패: {0}")]
    EnsureDaemon(String),
    /// 데몬 끄기(graceful stop) 실패.
    #[error("데몬 끄기 실패: {0}")]
    StopDaemon(String),
    /// UI 앱 열기(spawn 또는 show/focus) 실패.
    #[error("UI 열기 실패: {0}")]
    OpenUi(String),
    /// UI 앱 닫기(프로세스 종료) 실패.
    #[error("UI 닫기 실패: {0}")]
    CloseUi(String),
    /// 전체 종료(데몬+UI 정리) 실패.
    #[error("전체 종료 실패: {0}")]
    ShutdownAll(String),
}

// ── 주입 경계(trait) — 다음 sub-step 에서 real 구현 ────────────────────────────────

/// 데몬 생존 판정. real 구현은 `discovery::daemon_status(data_dir).alive`(다음 sub-step).
/// core 는 bool 만 받아 아이콘 상태를 산출한다 — 파일·PID·port 검사는 seam 뒤에 격리.
pub trait DaemonProbe {
    /// 살아있는 데몬이 발견되면 true.
    fn is_alive(&self) -> bool;
}

/// 트레이 메뉴가 일으키는 실제 효과(lifecycle 핸들, ADR-0023 §lifecycle owner).
///
/// real 구현(다음 sub-step)은 detached spawn(데몬·UI)·show/focus·graceful 종료(C4 순서)로 채운다.
/// core 의 디스패치는 이 trait 만 호출하므로 Fake 로 호출 검증이 가능하다.
///
/// 데몬이 메인(detached, Job 미상속)이라 [`Launcher::stop_daemon`]/[`close_ui`](Launcher::close_ui)
/// 는 각각을 **독립적으로** 끈다. "트레이 종료"(QuitTray)는 트레이 프로세스 자기 종료라 Launcher
/// 메서드가 없다(데몬·UI 는 계속 detached 로 돌고, 다음에 트레이를 켜면 재발견 — sub-step 2).
///
// ★sub-step 2 실제 구현 계약:
//   - ensure_daemon = discovery::ensure_daemon(WMI Win32_Process.Create — WmiPrvSE 부모라 Job
//     미상속 = ADR-0024 C1 detached 자동충족). 절대 std::process::Command 직접 spawn 금지
//     (Tauri/tray-host Job 상속 위험).
//   - stop_daemon = discovery::send_stop(WS 로 StopDaemon{force} 일방 발사 → 데몬이 self-exit).
//     데몬만 끈다(UI 무관). taskkill(daemon_stop) 폴백/ack 대기는 미구현(send_stop 안에 나중에 붙음).
//   - close_ui = UI 프로세스 종료(show/focus 의 역). UI 만 끈다(데몬 무관).
//   - shutdown_all = 데몬 graceful → UI 종료(C4) — "완전 종료" 전용.
//   real 구현체는 생성자에서 data_dir: PathBuf + daemon_exe 경로를 주입받는다(TRD §데이터 위치:
//   런처가 .engram-data 절대경로 결정·주입). LaunchError 는 sub-step 2 에서 DiscoveryError 원본을
//   #[source] 로 보존하도록 확장 예정(timeout/version-mismatch 분기용).
pub trait Launcher {
    /// 데몬을 detached 로 ensure(살아있으면 no-op). real = `discovery::ensure_daemon`.
    fn ensure_daemon(&self) -> Result<(), LaunchError>;
    /// 데몬을 graceful stop(UI 는 건드리지 않음). real = `discovery::send_stop`(WS StopDaemon 발사).
    fn stop_daemon(&self) -> Result<(), LaunchError>;
    /// UI 앱 열기 — 살아있으면 show/focus, 없으면 spawn. real = OS spawn + 신호.
    fn open_ui(&self) -> Result<(), LaunchError>;
    /// UI 앱 닫기 — UI 프로세스 종료(데몬 무관). real = UI 프로세스 종료.
    fn close_ui(&self) -> Result<(), LaunchError>;
    /// 전체 종료 — 데몬 graceful → UI 종료 → (호출 측이) tray-host 종료(ADR-0024 C4).
    fn shutdown_all(&self) -> Result<(), LaunchError>;
}

// ── 메뉴 의도 ──────────────────────────────────────────────────────────────────

/// 트레이 메뉴 클릭이 표현하는 **의도**(렌더링/원천과 분리된 단일 enum).
/// 사람 클릭·LLM 호출·단축키가 모두 이 의도로 수렴한다(CLAUDE.md §5 손발/두뇌).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// "데몬 켜기" — 데몬 ensure.
    StartDaemon,
    /// "데몬 끄기" — 데몬만 graceful stop(UI 무관).
    StopDaemon,
    /// "UI 열기" — UI open(show/focus or spawn).
    OpenUi,
    /// "UI 닫기" — UI 만 종료(데몬 무관).
    CloseUi,
    /// "트레이 종료" — 트레이 프로세스만 종료. 데몬·UI 는 detached 로 계속 돈다(Launcher 호출 없음).
    QuitTray,
    /// "완전 종료" — 전체 종료(데몬+UI graceful), 이후 tray-host 자기 종료(ADR-0024 C4).
    ShutdownAll,
}

impl MenuAction {
    /// 메뉴 항목의 안정 id(tray-icon MenuItem 의 id 문자열로 사용).
    /// 디스플레이 라벨과 분리 — 라벨이 바뀌어도 id 는 불변(클릭 매핑 안정).
    pub const fn menu_id(self) -> &'static str {
        match self {
            MenuAction::StartDaemon => "start_daemon",
            MenuAction::StopDaemon => "stop_daemon",
            MenuAction::OpenUi => "open_ui",
            MenuAction::CloseUi => "close_ui",
            MenuAction::QuitTray => "quit_tray",
            MenuAction::ShutdownAll => "shutdown_all",
        }
    }

    /// 메뉴 항목의 고정 라벨(상태 비반영).
    pub const fn label(self) -> &'static str {
        match self {
            MenuAction::StartDaemon => "데몬 켜기",
            MenuAction::StopDaemon => "데몬 끄기",
            MenuAction::OpenUi => "UI 열기",
            MenuAction::CloseUi => "UI 닫기",
            MenuAction::QuitTray => "트레이 종료",
            MenuAction::ShutdownAll => "완전 종료",
        }
    }

    /// v1 메뉴에 노출되는 액션들(순서 = 표시 순서).
    /// 표시: 데몬 켜기, 데몬 끄기, UI 열기, UI 닫기, (구분선), 트레이 종료, 완전 종료.
    /// (구분선은 GUI shell 이 QuitTray 앞에 삽입 — core 는 액션만 안다.)
    pub const ALL: [MenuAction; 6] = [
        MenuAction::StartDaemon,
        MenuAction::StopDaemon,
        MenuAction::OpenUi,
        MenuAction::CloseUi,
        MenuAction::QuitTray,
        MenuAction::ShutdownAll,
    ];
}

/// 메뉴 클릭 id → [`MenuAction`] 매핑(순수). 알 수 없는 id 면 None.
/// tray-icon 의 MenuEvent.id 문자열을 받아 의도로 환원한다.
pub fn action_for_menu_id(id: &str) -> Option<MenuAction> {
    MenuAction::ALL.into_iter().find(|a| a.menu_id() == id)
}

/// 메뉴 의도 → Launcher 메서드 디스패치(순수, Launcher trait 위).
///
/// 사람/LLM/단축키가 어떤 경로로 의도를 만들든 이 한 함수로 수렴한다(단일 control surface).
/// Fake Launcher 를 주입하면 "어떤 의도가 어떤 메서드를 부르는지" 를 OS 없이 검증할 수 있다.
///
/// [`MenuAction::QuitTray`] 는 트레이 프로세스 자기 종료라 Launcher 호출이 없다 → no-op(Ok).
/// 실제 프로세스 exit 는 shell 이 [`causes_tray_exit`] 로 판단해 수행한다.
pub fn dispatch(action: MenuAction, launcher: &dyn Launcher) -> Result<(), LaunchError> {
    match action {
        MenuAction::StartDaemon => launcher.ensure_daemon(),
        MenuAction::StopDaemon => launcher.stop_daemon(),
        MenuAction::OpenUi => launcher.open_ui(),
        MenuAction::CloseUi => launcher.close_ui(),
        MenuAction::QuitTray => Ok(()), // 트레이 자기 종료만 — Launcher 효과 없음(아래 causes_tray_exit).
        MenuAction::ShutdownAll => launcher.shutdown_all(),
    }
}

/// 이 액션이 **트레이 프로세스 자체**를 종료시키는지(순수 판정).
///
/// QuitTray(트레이만 종료)·ShutdownAll(데몬+UI 종료 후 트레이도 종료) 둘만 true.
/// shell 이 dispatch 후 이 함수로 `ControlFlow::Exit` 여부를 정한다 — exit 판단을 GUI 분기에
/// 흩지 않고 한 곳에 모은다(LLM 도 같은 판정을 재사용 가능).
///
/// ★sub-step 2: QuitTray 는 즉시 종료(drain 없음)라 이 경로 유지. ShutdownAll 은 ADR-0024 C4
///   다단계 비동기 종료(owner=Stopping → UI full_shutdown → 데몬 graceful drain ack+타임아웃 →
///   데몬 exit 확인 → UI 종료 → 트레이 종료)로 분기 예정 → 그때 ShutdownAll 은 이 bool 경로를
///   떠난다(별도 종료 상태머신). 즉 **이 함수의 ShutdownAll=true 는 임시 계약**이지 안정 계약이
///   아니다 — sub-step 2 에서 갈라지므로 안정 계약으로 오인 금지.
pub fn causes_tray_exit(action: MenuAction) -> bool {
    match action {
        MenuAction::QuitTray | MenuAction::ShutdownAll => true,
        MenuAction::StartDaemon
        | MenuAction::StopDaemon
        | MenuAction::OpenUi
        | MenuAction::CloseUi => false,
    }
}

// ── 상태 → 표시 매핑(순수) ─────────────────────────────────────────────────────────

/// 트레이 아이콘 상태. 데몬 생존을 시각화한다(활성=컬러/비활성=회색).
/// 실제 아이콘 두 벌은 main.rs(GUI)가 들고, core 는 어떤 상태인지만 결정한다(렌더링 분리).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// 데몬 alive — 활성(컬러) 아이콘.
    Active,
    /// 데몬 없음/죽음 — 비활성(회색) 아이콘.
    Inactive,
}

/// 데몬 alive(bool) → [`IconState`] 매핑(순수).
pub fn icon_state_for(alive: bool) -> IconState {
    if alive {
        IconState::Active
    } else {
        IconState::Inactive
    }
}

/// probe 로 데몬 상태를 읽어 아이콘 상태를 산출하는 편의 함수(순수 조합).
/// main.rs 가 stub probe 를 주입해 "probe → IconState" 배선을 검증/사용한다.
pub fn icon_state_from_probe(probe: &dyn DaemonProbe) -> IconState {
    icon_state_for(probe.is_alive())
}

// ── 아이콘 픽셀 변환(순수) ──────────────────────────────────────────────────────────

/// RGBA8 픽셀 버퍼를 desaturate(회색조)한 새 버퍼를 만든다(순수 — image 타입 무의존).
///
/// 비활성(데몬 죽음) 상태의 회색 아이콘을 컬러 원본에서 파생한다. luma = 0.299R+0.587G+0.114B
/// (Rec.601)로 각 RGB 채널을 동일 값으로 대체하고 alpha 는 보존한다 → R==G==B 인 무채색.
/// image 의존은 main.rs 에만 두고 core 는 `&[u8]` 슬라이스만 받아 격리를 유지한다(CLAUDE.md §4).
///
/// `rgba.len()` 은 `w*h*4` 여야 한다(RGBA 4채널). 이 전제는 유일 호출자 main.rs 의
/// `image::into_rgba8()` 가 보장한다. 어긋나면 디버그 빌드에서 panic(개발 계약 위반 조기 검출).
/// 릴리스에서 4의 배수가 아닌 잔여를 보존하지 **않는다** — 어차피 산출물의 유일 소비처
/// `Icon::from_rgba(_, w, h)` 가 `len==w*h*4` 를 요구해 잔여가 있으면 그쪽에서 Err→expect panic
/// 이라, 잔여를 살려도 "안전망"이 못 된다(전제 위반은 호출자 버그). 그래서 chunks_exact 의
/// 잔여는 버린다 — 전제가 지켜지면 잔여 자체가 없다.
pub fn to_grayscale_rgba(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    debug_assert_eq!(
        rgba.len(),
        (w as usize) * (h as usize) * 4,
        "to_grayscale_rgba: 버퍼 길이 ≠ w*h*4 (RGBA)"
    );
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        // Rec.601 luma. f32 누적 후 반올림 — 정수 근사 누적오차 회피.
        let luma = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
        let g = luma.round().clamp(0.0, 255.0) as u8;
        out.push(g); // R
        out.push(g); // G
        out.push(g); // B
        out.push(px[3]); // A 보존
    }
    out
}

// ── 테스트 (Fake 주입 — OS/GUI 무의존) ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // 데몬 생존을 고정값으로 주입하는 Fake(discovery.rs FakeLiveness 스타일).
    struct FakeProbe {
        alive: bool,
    }
    impl DaemonProbe for FakeProbe {
        fn is_alive(&self) -> bool {
            self.alive
        }
    }

    // 각 Launcher 메서드 호출 횟수를 세는 Fake(discovery.rs CountingSpawner 스타일).
    // 디스패치가 올바른 메서드만 부르는지 카운터로 검증한다.
    #[derive(Default)]
    struct CountingLauncher {
        ensure: Cell<usize>,
        stop: Cell<usize>,
        open: Cell<usize>,
        close: Cell<usize>,
        shutdown: Cell<usize>,
    }
    impl Launcher for CountingLauncher {
        fn ensure_daemon(&self) -> Result<(), LaunchError> {
            self.ensure.set(self.ensure.get() + 1);
            Ok(())
        }
        fn stop_daemon(&self) -> Result<(), LaunchError> {
            self.stop.set(self.stop.get() + 1);
            Ok(())
        }
        fn open_ui(&self) -> Result<(), LaunchError> {
            self.open.set(self.open.get() + 1);
            Ok(())
        }
        fn close_ui(&self) -> Result<(), LaunchError> {
            self.close.set(self.close.get() + 1);
            Ok(())
        }
        fn shutdown_all(&self) -> Result<(), LaunchError> {
            self.shutdown.set(self.shutdown.get() + 1);
            Ok(())
        }
    }
    impl CountingLauncher {
        // (ensure, stop, open, close, shutdown) 호출 횟수 스냅샷.
        fn counts(&self) -> (usize, usize, usize, usize, usize) {
            (
                self.ensure.get(),
                self.stop.get(),
                self.open.get(),
                self.close.get(),
                self.shutdown.get(),
            )
        }
    }

    #[test]
    fn menu_id_roundtrips_to_action() {
        // id ↔ action 매핑이 일관(각 액션의 id 로 다시 그 액션이 나온다) — 6개 전부.
        for action in MenuAction::ALL {
            assert_eq!(action_for_menu_id(action.menu_id()), Some(action));
        }
    }

    #[test]
    fn unknown_menu_id_is_none() {
        assert_eq!(action_for_menu_id("nope"), None);
        assert_eq!(action_for_menu_id(""), None);
    }

    #[test]
    fn dispatch_routes_each_action_to_exactly_one_method() {
        // 각 의도가 정확히 자기 Launcher 메서드 1회만 부르는지(QuitTray 는 0건 — no-op).
        let cases: [(MenuAction, (usize, usize, usize, usize, usize)); 6] = [
            (MenuAction::StartDaemon, (1, 0, 0, 0, 0)),
            (MenuAction::StopDaemon, (0, 1, 0, 0, 0)),
            (MenuAction::OpenUi, (0, 0, 1, 0, 0)),
            (MenuAction::CloseUi, (0, 0, 0, 1, 0)),
            (MenuAction::QuitTray, (0, 0, 0, 0, 0)), // Launcher 호출 없음.
            (MenuAction::ShutdownAll, (0, 0, 0, 0, 1)),
        ];
        for (action, expected) in cases {
            let l = CountingLauncher::default();
            dispatch(action, &l).unwrap();
            assert_eq!(l.counts(), expected, "{action:?} 라우팅");
        }
    }

    #[test]
    fn dispatch_quit_tray_is_noop_ok() {
        // QuitTray 는 어떤 Launcher 메서드도 부르지 않고 Ok — 트레이 자기 종료는 shell 책임.
        let l = CountingLauncher::default();
        assert!(dispatch(MenuAction::QuitTray, &l).is_ok());
        assert_eq!(l.counts(), (0, 0, 0, 0, 0), "QuitTray 는 Launcher 무호출");
    }

    #[test]
    fn quit_tray_zero_launcher_calls_and_exits_tray_together() {
        // QuitTray 의미의 핵심 결합: "Launcher 0호출 ∧ 트레이 종료시킴" — 두 성질이 분리되면
        // dispatch 만 no-op 으로 바꾸고 causes_tray_exit 를 false 로 떨구는(또는 반대) 회귀가
        // 각각의 단일 테스트는 통과시키며 빠져나간다. 한 곳에서 동시에 박는다.
        let l = CountingLauncher::default();
        dispatch(MenuAction::QuitTray, &l).unwrap();
        assert_eq!(
            l.counts(),
            (0, 0, 0, 0, 0),
            "QuitTray 는 어떤 Launcher 메서드도 부르지 않아야 함"
        );
        assert!(
            causes_tray_exit(MenuAction::QuitTray),
            "QuitTray 는 트레이를 종료시켜야 함"
        );
    }

    #[test]
    fn dispatch_propagates_launcher_error() {
        // Launcher 가 에러를 내면 dispatch 도 그대로 전파(삼킴 금지). 각 메서드가 자기 variant 에러를
        // 내는 Fake → dispatch 가 해당 의도에서 그 variant 를 전파하는지(QuitTray 는 무호출이라 제외).
        struct FailingLauncher;
        impl Launcher for FailingLauncher {
            fn ensure_daemon(&self) -> Result<(), LaunchError> {
                Err(LaunchError::EnsureDaemon("boom".into()))
            }
            fn stop_daemon(&self) -> Result<(), LaunchError> {
                Err(LaunchError::StopDaemon("boom".into()))
            }
            fn open_ui(&self) -> Result<(), LaunchError> {
                Err(LaunchError::OpenUi("boom".into()))
            }
            fn close_ui(&self) -> Result<(), LaunchError> {
                Err(LaunchError::CloseUi("boom".into()))
            }
            fn shutdown_all(&self) -> Result<(), LaunchError> {
                Err(LaunchError::ShutdownAll("boom".into()))
            }
        }
        let l = FailingLauncher;
        assert!(matches!(
            dispatch(MenuAction::StartDaemon, &l).unwrap_err(),
            LaunchError::EnsureDaemon(_)
        ));
        assert!(matches!(
            dispatch(MenuAction::StopDaemon, &l).unwrap_err(),
            LaunchError::StopDaemon(_)
        ));
        assert!(matches!(
            dispatch(MenuAction::OpenUi, &l).unwrap_err(),
            LaunchError::OpenUi(_)
        ));
        assert!(matches!(
            dispatch(MenuAction::CloseUi, &l).unwrap_err(),
            LaunchError::CloseUi(_)
        ));
        assert!(matches!(
            dispatch(MenuAction::ShutdownAll, &l).unwrap_err(),
            LaunchError::ShutdownAll(_)
        ));
        // QuitTray 는 Launcher 무호출이라 항상 Ok(에러 경로 없음).
        assert!(dispatch(MenuAction::QuitTray, &FailingLauncher).is_ok());
    }

    #[test]
    fn causes_tray_exit_only_for_quit_and_shutdown() {
        // 트레이 프로세스를 죽이는 액션은 QuitTray·ShutdownAll 둘뿐.
        assert!(causes_tray_exit(MenuAction::QuitTray));
        assert!(causes_tray_exit(MenuAction::ShutdownAll));
        assert!(!causes_tray_exit(MenuAction::StartDaemon));
        assert!(!causes_tray_exit(MenuAction::StopDaemon));
        assert!(!causes_tray_exit(MenuAction::OpenUi));
        assert!(!causes_tray_exit(MenuAction::CloseUi));
    }

    #[test]
    fn icon_state_maps_alive() {
        assert_eq!(icon_state_for(true), IconState::Active);
        assert_eq!(icon_state_for(false), IconState::Inactive);
    }

    #[test]
    fn icon_state_from_probe_reflects_probe() {
        assert_eq!(
            icon_state_from_probe(&FakeProbe { alive: true }),
            IconState::Active
        );
        assert_eq!(
            icon_state_from_probe(&FakeProbe { alive: false }),
            IconState::Inactive
        );
    }

    #[test]
    fn all_variants_present_in_all_array() {
        // ALL 누락 방지: 새 variant 를 추가하면 아래 exhaustive match 가 컴파일 에러를 내
        // (non-exhaustive) "이 variant 를 ALL 에 넣었는지" 를 강제 인지하게 한다.
        fn assert_in_all(a: MenuAction) {
            assert!(
                MenuAction::ALL.contains(&a),
                "{a:?} 가 MenuAction::ALL 에 없음 — 라우팅에서 silent 누락"
            );
        }
        // ※ 새 variant 추가 시 여기 arm 을 추가해야 컴파일된다(강제 인지 지점).
        match MenuAction::StartDaemon {
            MenuAction::StartDaemon => assert_in_all(MenuAction::StartDaemon),
            MenuAction::StopDaemon => assert_in_all(MenuAction::StopDaemon),
            MenuAction::OpenUi => assert_in_all(MenuAction::OpenUi),
            MenuAction::CloseUi => assert_in_all(MenuAction::CloseUi),
            MenuAction::QuitTray => assert_in_all(MenuAction::QuitTray),
            MenuAction::ShutdownAll => assert_in_all(MenuAction::ShutdownAll),
        }
        // 위 match 로 강제 인지된 variant 수와 ALL 길이가 일치하는지(중복/누락 동시 차단).
        assert_eq!(MenuAction::ALL.len(), 6, "variant 수 ↔ ALL 길이 불일치");
    }

    #[test]
    fn menu_ids_are_unique() {
        // id 충돌이면 클릭 라우팅이 깨진다 — 6개 모두 distinct 보장.
        let ids: Vec<&str> = MenuAction::ALL.iter().map(|a| a.menu_id()).collect();
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len(), "menu_id 중복: {ids:?}");
    }

    #[test]
    fn labels_are_unique_and_nonempty() {
        // 라벨 6개가 모두 비지 않고 distinct(메뉴 표시 혼동 방지).
        let labels: Vec<&str> = MenuAction::ALL.iter().map(|a| a.label()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()), "빈 라벨: {labels:?}");
        let mut dedup = labels.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(labels.len(), dedup.len(), "label 중복: {labels:?}");
    }

    #[test]
    fn grayscale_converts_color_to_gray_preserving_alpha() {
        // 컬러 픽셀 2개(빨강 반투명, 초록 불투명) → R==G==B(무채색) + alpha 보존.
        let rgba = [
            200u8, 10, 30, 128, // 빨강 계열, alpha=128
            10, 200, 30, 255, // 초록 계열, alpha=255
        ];
        let out = to_grayscale_rgba(&rgba, 2, 1);
        assert_eq!(out.len(), rgba.len(), "길이 보존");
        // px0: 무채색 + alpha 보존.
        assert_eq!(out[0], out[1]);
        assert_eq!(out[1], out[2]);
        assert_eq!(out[3], 128, "alpha 보존(px0)");
        // px1: 무채색 + alpha 보존.
        assert_eq!(out[4], out[5]);
        assert_eq!(out[5], out[6]);
        assert_eq!(out[7], 255, "alpha 보존(px1)");
        // luma 값 검증(Rec.601): px0 = 0.299*200+0.587*10+0.114*30 ≈ 68.99 → 69.
        let expected0 = (0.299 * 200.0 + 0.587 * 10.0 + 0.114 * 30.0f32).round() as u8;
        assert_eq!(out[0], expected0, "px0 luma");
    }

    #[test]
    fn grayscale_pure_gray_input_is_idempotent_ish() {
        // 이미 무채색인 입력은 거의 그대로(반올림 오차 0): R==G==B 인 회색은 luma=그 값.
        let rgba = [128u8, 128, 128, 255];
        let out = to_grayscale_rgba(&rgba, 1, 1);
        assert_eq!(out, vec![128, 128, 128, 255]);
    }

    #[test]
    fn grayscale_black_and_white_extremes() {
        // 검정→검정, 흰색→흰색(clamp 경계 안전).
        let rgba = [0u8, 0, 0, 255, 255, 255, 255, 255];
        let out = to_grayscale_rgba(&rgba, 2, 1);
        assert_eq!(&out[0..4], &[0, 0, 0, 255]);
        assert_eq!(&out[4..8], &[255, 255, 255, 255]);
    }
}
