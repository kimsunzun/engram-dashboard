//! 데스크톱 창의 OS 규칙 — 창 정체 · 자리와 z 순서 · 보임 · 자리 되돌리기 · Alt+Tab 빼기 · 커서 밑 맨 위 창 ·
//! 창 진단 사실(로그 몫) · 전경 창이 이 프로세스의 것인가.
//!
//! 창은 `raw-window-handle` 0.6 의 [`HasWindowHandle`] 로 받고 그 핸들의 변이 맞추기(Windows = `Win32` · 그 밖 =
//! [`WindowError::Unsupported`])를 이 모듈이 한다 — 부르는 쪽이 변이를 맞추면 그것도 이 crate 밖의 런타임 OS
//! 가름이다(TRD S21-slot-docking §8 · B10). 창을 바꾸는 함수는 날 Win32 호출이라 창 툴킷(tao)의 깃발을 거치지
//! 않는다 — 툴킷이 아는 보임 · 확장 스타일과 어긋날 수 있다는 것이 부르는 쪽 계약이다(TRD §2-5 「tao 깃발 불변식」).
//!
//! ★스레드★ — 창을 바꾸는 함수([`place_window`] · [`hide_window`] · [`move_window`] · [`restore_placement`] ·
//! [`set_switcher_hidden`])는 그 창을 만든 스레드(메인)에서 부른다. 다른 스레드에서 부르면 Win32 가 그 창
//! 스레드에 메시지를 보내고 답을 기다린다 — 그 스레드가 부른 쪽을 기다리고 있으면 교착이다. 읽기
//! ([`NativeWindow::of`] · [`is_window_visible`] · [`top_level_at`] · [`describe_window`] ·
//! [`foreground_is_current_process`])는 어느 스레드에서나 된다.

// ADR-0266
// ADR-0295

use std::fmt;
use std::io;

use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle};

/// 화면의 물리 px 점. 프로세스가 모니터별 DPI 인식(v2 — tao 가 켠다)이 아니면 OS 가 배율로 가상화한 값이 온다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysPoint {
    pub x: i32,
    pub y: i32,
}

/// 화면의 물리 px 사각형 — 왼쪽 위 `(x, y)` 와 크기. 점을 품는지는 반열림이다(`x ≤ 점 < x + width`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// 창 정체 — 같은 창인가 비교 · [`top_level_at`] 의 건너뛰기 · [`ZOrder::After`] 의 기준에 쓴다. 안은 부르는
/// 쪽이 모른다(Windows = HWND 값).
///
/// ★창이 사라진 뒤에도 값은 남는다★ — OS 가 같은 값을 새 창에 다시 줄 수 있어, 오래 쥔 값끼리의 같음은 「같은
/// 창」을 보증하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWindow(isize);

impl NativeWindow {
    /// `None` = 핸들을 못 얻었거나(창이 아직 · 이미 없다) 이 crate 가 다루는 창 핸들이 아니다 — Windows 밖에서는
    /// 늘 `None` 이다.
    pub fn of(w: &impl HasWindowHandle) -> Option<NativeWindow> {
        raw_of(w).ok().map(NativeWindow)
    }
}

/// 로그 몫 — 창 들여다보기 도구(Spy++ 따위)가 쓰는 16진 핸들 값.
impl fmt::LowerHex for NativeWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

/// 창을 놓을 z 자리 — [`place_window`] 의 인자.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrder {
    /// 항상 위 띠의 맨 위 — 그 창을 항상 위로 만든다(Win32 `HWND_TOPMOST`).
    Topmost,
    /// 그 창이 든 띠(항상 위 · 보통)의 맨 위.
    Top,
    /// 이 창 바로 아래.
    After(NativeWindow),
    /// z 순서를 건드리지 않는다 — 단 [`place_window`] 의 `activate` 가 참이면 활성이 된 창은 OS 가 맨 위로 올린다.
    Keep,
}

/// 창 함수의 실패.
#[derive(Debug)]
pub enum WindowError {
    /// 이 OS 에서는 이 수단이 없다 — Windows 밖이거나 Win32 창이 아닌 핸들이다.
    Unsupported,
    /// 핸들을 못 얻었거나 그 창이 이미 없다.
    NoWindow,
    /// OS 호출이 실패했다 — 문구에 그 호출 이름과 OS 오류가 든다.
    Os(io::Error),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("이 OS 에서는 창 함수가 없다"),
            Self::NoWindow => f.write_str("창이 없다"),
            Self::Os(e) => write!(f, "창 OS 호출 실패: {e}"),
        }
    }
}

impl std::error::Error for WindowError {}

/// 핸들의 변이를 맞춘다 — 창 함수가 다루는 것은 Win32 창 하나뿐이다.
fn raw_of(w: &impl HasWindowHandle) -> Result<isize, WindowError> {
    let handle = w.window_handle().map_err(|e| match e {
        HandleError::NotSupported => WindowError::Unsupported,
        _ => WindowError::NoWindow,
    })?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Ok(h.hwnd.get()),
        _ => Err(WindowError::Unsupported),
    }
}

/// 자리 · 크기 · z · 보임 · 활성을 한 번의 `SetWindowPos` 로 입힌다(TRD §8 — 깜빡임 없이 한 번에 놓는다).
///
/// - `rect: None` = 자리 · 크기를 그대로 둔다(제자리 보이기 — `show` 와 함께 쓴다).
/// - `show` = 거짓이면 보임을 건드리지 않는다(숨기지 않는다 — 숨기기는 [`hide_window`]).
/// - `activate` = 참이면 그 창을 전경으로 청한다 — 활성이 되면 `z` 가 [`ZOrder::Keep`] 이어도 맨 위로 온다.
///   ★OS 가 거절하면 그대로 두고 `Ok` 다★ — tao 의
///   `force_window_active` 처럼 Alt 를 합성해 전경을 빼앗지 않는다: 끄는 동안 웹뷰의 앱 전체 키 감시가 그 Alt 를
///   놓기로 읽는다(TRD §6-3 · F7 a). 거절됐는지는 [`foreground_is_current_process`] 따위로 따로 본다.
///
/// Windows 밖 = [`WindowError::Unsupported`].
// ADR-0295
pub fn place_window(
    w: &impl HasWindowHandle,
    rect: Option<PhysRect>,
    z: ZOrder,
    show: bool,
    activate: bool,
) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::place_window(win::hwnd_of(w)?, rect, z, show, activate)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, rect, z, show, activate);
        Err(WindowError::Unsupported)
    }
}

/// 창을 숨긴다(Win32 `SW_HIDE`). ★포인터 캡처를 쥔 웹뷰의 창에는 부르지 않는다★ — 캡처가 조용히 끊긴다(스파이크
/// S2). 툴킷의 보임 깃발은 참으로 남는다(모듈 머리).
///
/// Windows 밖 = [`WindowError::Unsupported`].
pub fn hide_window(w: &impl HasWindowHandle) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::hide_window(win::hwnd_of(w)?);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        Err(WindowError::Unsupported)
    }
}

/// OS 가 보는 그 창의 보임(Win32 `IsWindowVisible`) — 툴킷의 보임 깃발과 대조하는 데 쓴다(TRD §6-1 · F7 c). 창이
/// 없거나 Windows 밖이면 거짓이다.
pub fn is_window_visible(w: &impl HasWindowHandle) -> bool {
    #[cfg(windows)]
    {
        win::hwnd_of(w).map(win::is_visible).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        false
    }
}

/// 화면 점 `p` 를 품은 맨 위 최상위 창 — z 순서 위에서부터 걸으며 `skip` 을 건너뛰고, 보이고 · 최소화 아니고 ·
/// 가려지지(DWM cloaked — 다른 가상 데스크톱 따위) 않고 · 비쳐 보이는 창이 아닌 첫 창이다. 비쳐 보이는 창 = 클릭이
/// 지나가는 창(`WS_EX_LAYERED` + `WS_EX_TRANSPARENT` — 모든 창) · 다른 앱의 layered 창 중 창 전체 알파가 0 이거나
/// 색 키가 있는 창이다. 픽셀마다 알파를 주는 다른 앱 layered 창은 가리는 창으로 친다(판정 = `hit_accepts`). 우리
/// 창인지는 부르는 쪽이 [`NativeWindow::of`] 와 견준다 — 다른 앱 창 · 바탕화면이 오면 그 자리는 가려진 것이다(PRD
/// R15).
///
/// 「품는다」는 OS 바깥 사각형(`GetWindowRect` — 보이지 않는 크기 조절 테두리 포함)이다. OS 의 적중 판정도 그
/// 테두리를 그 창으로 친다. 단 창 모양(`SetWindowRgn`)이 있는 창은 점이 그 모양 안이어야 한다. `None` = 품은 창이
/// 없다 · Windows 밖.
// ADR-0295
pub fn top_level_at(p: PhysPoint, skip: &[NativeWindow]) -> Option<NativeWindow> {
    #[cfg(windows)]
    {
        win::top_level_at(p, skip)
    }
    #[cfg(not(windows))]
    {
        let _ = (p, skip);
        None
    }
}

/// layered 창(Win32 `WS_EX_LAYERED`)이 얼마나 비쳐 보이는가 — OS 에서 읽을 수 있는 만큼.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayeredKind {
    NotLayered,
    /// `SetLayeredWindowAttributes` 로 준 창 전체 알파 — `LWA_ALPHA` 없이 준 창은 255(불투명)다.
    Alpha(u8),
    /// `SetLayeredWindowAttributes` 의 색 키(`LWA_COLORKEY`) — 그 색 픽셀은 그려지지 않고 클릭도 지나간다. 창 전체
    /// 알파가 함께 있어도 이것이다.
    ColorKey,
    /// 속성을 못 읽었다 — 픽셀마다 알파를 주는 `UpdateLayeredWindow` 창이면 OS 가 답하지 않는다(MS
    /// `GetLayeredWindowAttributes` 문서). 속성을 아직 안 준 layered 창도 여기다(실측). 어느 픽셀이 비었는지는 모른다.
    PerPixel,
}

/// 창 하나의 진단 사실 — [`describe_window`] 의 답(로그 몫).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowFacts {
    /// 그 창을 만든 프로세스 — 0 = 그 사이 사라졌다.
    pub pid: u32,
    /// 창 클래스 이름 — 못 읽었으면 빈 문자열.
    pub class: String,
    /// 확장 스타일(Win32 `GWL_EXSTYLE` 값) — 못 읽었으면 0.
    pub ex_style: u32,
    /// 확장 스타일을 못 읽었으면 `NotLayered` 다.
    pub layered: LayeredKind,
    /// 오른쪽에서 왼쪽 배치(`WS_EX_LAYOUTRTL`) — 창 모양 좌표를 오른쪽 위에서 잰다.
    pub rtl: bool,
    /// 창 모양(Win32 `SetWindowRgn`)이 있다.
    pub has_region: bool,
}

/// 그 창의 진단 사실 — 창에 메시지를 보내지 않는 조회뿐이라 응답 없는 다른 앱 창에도 막히지 않는다. `None` =
/// 그 창이 없다 · Windows 밖.
pub fn describe_window(w: NativeWindow) -> Option<WindowFacts> {
    #[cfg(windows)]
    {
        win::describe(w)
    }
    #[cfg(not(windows))]
    {
        let _ = w;
        None
    }
}

/// 자리만 옮긴다 — 크기 · z · 활성은 그대로(TRD §5-5 두 걸음 놓기 ①).
///
/// Windows 밖 = [`WindowError::Unsupported`].
pub fn move_window(w: &impl HasWindowHandle, origin: PhysPoint) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::move_window(win::hwnd_of(w)?, origin)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, origin);
        Err(WindowError::Unsupported)
    }
}

/// 보통 자리 `normal`(★화면 좌표★)과 최대화를 되돌리고(`SetWindowPlacement`), `after` 가 있으면 그 창 바로
/// 아래로 둔다(TRD §8 · PRD R14).
///
/// - 좌표계 — `WINDOWPLACEMENT` 의 보통 자리는 `WS_EX_TOOLWINDOW` 가 없는 최상위 창이면 작업 공간 좌표(그 모니터
///   작업 영역의 왼쪽 위가 0 — 작업 표시줄이 위 · 왼쪽에 있으면 그만큼 밀린다)이고 있으면 화면 좌표다(MS
///   `WINDOWPLACEMENT` 문서 · TRD E12). 이 함수가 그 창의 지금 확장 스타일과 `normal` 에 가장 가까운 모니터로 옮겨
///   넣는다 — 그래서 Alt+Tab 빼기를 되돌린 뒤(`set_switcher_hidden(false)`)에 부른다(TRD §6-2 ②).
/// - `activate` = 거짓이면 보통 자리는 활성화 없이 보인다(`SW_SHOWNOACTIVATE`). ★`maximized` 는 `activate` 와
///   무관하게 `SW_SHOWMAXIMIZED` 다★ — Win32 에 활성화 없이 최대화로 보이는 show 명령이 없다. 그래서 OS 의 전경
///   규칙이 허락하면 활성이 된다(미확인 — TRD §12 M6 이 재고, 안 되면 §15-2 S-c).
/// - OS 는 화면 밖으로 완전히 나가는 보통 자리를 화면 안으로 고쳐 넣는다(MS `SetWindowPlacement` 문서).
///
/// Windows 밖 = [`WindowError::Unsupported`].
// ADR-0295
pub fn restore_placement(
    w: &impl HasWindowHandle,
    normal: PhysRect,
    maximized: bool,
    after: Option<NativeWindow>,
    activate: bool,
) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::restore_placement(win::hwnd_of(w)?, normal, maximized, after, activate)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, normal, maximized, after, activate);
        Err(WindowError::Unsupported)
    }
}

/// Alt+Tab 에서 빼고(`hidden`) 넣는다(TRD §8 · B7 · E2). 뺄 때 `WS_EX_TOOLWINDOW` 를 켜고 `WS_EX_APPWINDOW` 를
/// 끈다 · 넣을 때 둘 다 되돌린다(+ `SWP_FRAMECHANGED`).
///
/// - `WS_EX_APPWINDOW` 도 끄는 까닭 — tao 는 주인 없는 창에 그 비트를 늘 켜고(tao 0.35.3 `window_state.rs` 의
///   `ON_TASKBAR`), 켜져 있으면 도구 창이어도 작업 표시줄 · Alt+Tab 에 나갈 수 있다(가능성 높음 — TRD §12 M18).
///   넣을 때 그 비트를 켜는 것도 같은 까닭이다(우리 창은 주인 없는 tao 창이다).
/// - ★뺀 창에 tao 깃발 setter 를 부르지 않는다★ — tao 는 깃발이 바뀔 때마다 `GWL_EXSTYLE` 을 제 깃발로 통째 다시
///   써 이 두 비트를 되돌린다(TRD §2-5 「tao 깃발 불변식」).
/// - 테두리 있는 창은 틀 두께가 바뀐다 — 자리는 이것을 바꾼 뒤에 읽어 셈한다(TRD §5-5 · W-13).
/// - 작업 표시줄 빼기는 이 함수의 몫이 아니다(tao `skip_taskbar` — `ITaskbarList`).
///
/// Windows 밖 = [`WindowError::Unsupported`].
// ADR-0295
pub fn set_switcher_hidden(w: &impl HasWindowHandle, hidden: bool) -> Result<(), WindowError> {
    #[cfg(windows)]
    {
        win::set_switcher_hidden(win::hwnd_of(w)?, hidden)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, hidden);
        Err(WindowError::Unsupported)
    }
}

/// 지금 전경 창이 이 프로세스의 것인가. `None` = 이 OS 에서는 답할 수단이 이 crate 에 없다 — 부르는 쪽이 자기
/// 수단(창 툴킷의 포커스 게터 따위)으로 판정한다(lib.rs 「들이는 규칙」 3 — Windows 밖 갈래는 자리채움).
///
/// Windows = `GetForegroundWindow` 의 창을 만든 스레드의 PID 가 이 프로세스의 PID 와 같은가. 전경 창이 없거나 그
/// 사이 사라졌으면 `Some(false)` 다.
#[cfg(windows)]
pub fn foreground_is_current_process() -> Option<bool> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let mut pid = 0u32;
    // SAFETY: 인자 없는 조회 하나와, 살아 있는 지역 변수에 PID 를 쓰는 조회 하나다. 전경 창이 없거나 그 사이
    //   사라졌으면 PID 가 0 으로 남는다.
    unsafe {
        let foreground = GetForegroundWindow();
        if !foreground.is_invalid() {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
    }
    Some(pid != 0 && pid == std::process::id())
}
#[cfg(not(windows))]
pub fn foreground_is_current_process() -> Option<bool> {
    None
}

/// 화면 좌표 사각형을 작업 공간 좌표로 — 모니터 왼쪽 위에서 작업 영역 왼쪽 위까지 민 만큼 뺀다. Chromium 이 읽을
/// 때 쓰는 반대 셈과 같은 꼴이다(hwnd_message_handler.cc `GetWindowPlacement` — `rcWork − rcMonitor` 를 더한다).
fn screen_to_workspace(rect: PhysRect, monitor: PhysRect, work: PhysRect) -> PhysRect {
    PhysRect {
        x: rect.x - (work.x - monitor.x),
        y: rect.y - (work.y - monitor.y),
        ..rect
    }
}

/// Alt+Tab 에서 빼고 넣을 때의 확장 스타일(인자 · 결과 = `GWL_EXSTYLE` 값).
fn switcher_ex_style(ex: u32, hidden: bool, tool_window: u32, app_window: u32) -> u32 {
    if hidden {
        (ex | tool_window) & !app_window
    } else {
        (ex & !tool_window) | app_window
    }
}

fn contains(r: PhysRect, p: PhysPoint) -> bool {
    p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height
}

/// 점과 그 창의 창 모양(Win32 `SetWindowRgn`) — [`hit_accepts`] 의 재료.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionHit {
    /// 창 모양이 없다(또는 못 읽었다) — 바깥 사각형 전체가 그 창이다.
    NoRegion,
    Inside,
    Outside,
}

/// 점을 바깥 사각형에 품은(보이고 · 최소화 아니고 · 가려지지 않은) 창을 [`top_level_at`] 의 답으로 치는가 — OS 의
/// 적중 판정을 읽을 수 있는 만큼 따른다. `ours` = 이 프로세스의 창 · `transparent` = `WS_EX_TRANSPARENT`.
///
/// - layered + `WS_EX_TRANSPARENT` 창은 건너뛴다 — 클릭이 지나간다(MS 「Layered Windows」 · OS `WindowFromPoint` 도
///   건너뛴다). `WS_EX_TRANSPARENT` 만 켜진 창은 가리는 창이다 — 그 깃발 혼자는 그리기 순서일 뿐 클릭은 그 창이 받는다(실측 —
///   `WindowFromPoint` 와 견준 시험 `transparent_only_window_still_takes_the_click`).
/// - 다른 앱 layered 창은 창 전체 알파 0 · 색 키면 건너뛴다. OS 는 알파 0 · 색 키 픽셀에서만 클릭을 지나가게 하는데
///   (MS 「Layered Windows」) 픽셀은 못 읽어 색 키 창은 창 전체를 비친 것으로 친다 — 대가: 그 창의 칠해진 자리
///   밑도 가려지지 않은 자리다. 늘 위인 녹화 영역 틀(화면 녹화 도구 따위)이 그 밑 우리 창에 붙지 못하게 하던 결함
///   (2026-10-19)을 이 갈래로 푼다 — 그 틀이 픽셀마다 알파 창이면 풀리지 않는데 어느 쪽인지 재지 않았다.
/// - 그 밖의 다른 앱 layered 창(알파 1 이상 · 픽셀마다 알파)은 가리는 창이다 — 불투명하게 그리는 layered 앱이 많다
///   (WPF `AllowsTransparency` · Electron `transparent` · Qt 반투명 창). 다 건너뛰면 그 앱에 가린 우리 창에 붙고 750 ms
///   머묾이 그 창을 올린다(PRD R15 · AC43). Chromium `TopMostFinder`(Chrome 70 `window_finder_win.cc`)는 layered ·
///   `WS_EX_TRANSPARENT` 중 하나만 켜져도 건너뛰는데, 이 대가를 받지 않으려고 따르지 않는다.
/// - 우리 창은 클릭 통과 창만 건너뛴다 — tao 는 커서 무시 창에만 layered 를 켠다(tao 0.35.3 `window_state.rs`
///   `IGNORE_CURSOR_EVENT`).
/// - 창 모양이 있으면 점이 그 안이어야 한다 — 모양 밖은 그 창이 그리지도 클릭을 받지도 않는다(Chromium 같은 곳).
// ADR-0295
fn hit_accepts(ours: bool, layered: LayeredKind, transparent: bool, region: RegionHit) -> bool {
    let click_through = layered != LayeredKind::NotLayered && transparent;
    let see_through = click_through
        || (!ours && matches!(layered, LayeredKind::Alpha(0) | LayeredKind::ColorKey));
    !see_through && region != RegionHit::Outside
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::io;

    use raw_window_handle::HasWindowHandle;
    use windows::Win32::Foundation::{GetLastError, SetLastError, HWND, RECT, WIN32_ERROR};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::Graphics::Gdi::{
        CreateRectRgn, DeleteObject, GetMonitorInfoW, GetWindowRgn, MonitorFromRect, PtInRegion,
        HRGN, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_ERROR,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetLayeredWindowAttributes, GetTopWindow, GetWindow, GetWindowLongPtrW,
        GetWindowPlacement, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, SetForegroundWindow, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos,
        ShowWindow, GWL_EXSTYLE, GW_HWNDNEXT, HWND_TOP, HWND_TOPMOST,
        LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA, LWA_COLORKEY, SET_WINDOW_POS_FLAGS,
        SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
        SW_HIDE, SW_SHOWMAXIMIZED, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, WINDOWPLACEMENT,
        WINDOWPLACEMENT_FLAGS, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_LAYOUTRTL, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT,
    };

    use super::{
        contains, hit_accepts, raw_of, screen_to_workspace, switcher_ex_style, LayeredKind,
        NativeWindow, PhysPoint, PhysRect, RegionHit, WindowError, WindowFacts, ZOrder,
    };

    /// 최상위 창 z 걷기의 상한 — MS 는 `GetWindow` 로 걸으면 그동안 z 순서가 바뀌어 끝없이 돌 수 있다고 적는다
    /// (`GetWindow` 문서). 한 데스크톱의 최상위 창은 숨은 것까지 수천 개를 넘지 않는다.
    const MAX_Z_WALK: usize = 16_384;

    pub(crate) fn hwnd_of(w: &impl HasWindowHandle) -> Result<HWND, WindowError> {
        let hwnd = HWND(raw_of(w)? as *mut c_void);
        // SAFETY: 값만 보는 조회다 — 사라진 핸들이면 거짓이다.
        if unsafe { IsWindow(hwnd) }.as_bool() {
            Ok(hwnd)
        } else {
            Err(WindowError::NoWindow)
        }
    }

    pub(crate) fn native(hwnd: HWND) -> NativeWindow {
        NativeWindow(hwnd.0 as isize)
    }

    fn os(api: &str, e: windows::core::Error) -> WindowError {
        WindowError::Os(io::Error::other(format!("{api}: {e}")))
    }

    /// `place_window` 의 `SetWindowPos` 인자 — (z 기준 창, 깃발). `Keep` 의 `SWP_NOZORDER` 는 z 기준 창만 무시하게
    /// 할 뿐 활성화가 창을 올리는 것은 막지 않는다(MS `SetWindowPos` 문서 — `SWP_NOACTIVATE` 가 없으면 활성으로
    /// 그 띠의 맨 위 · 뒤따르는 `SetForegroundWindow` 도 그렇다).
    pub(super) fn place_plan(
        keep_rect: bool,
        z: ZOrder,
        show: bool,
        activate: bool,
    ) -> (HWND, SET_WINDOW_POS_FLAGS) {
        let mut flags = SET_WINDOW_POS_FLAGS(0);
        if keep_rect {
            flags |= SWP_NOMOVE | SWP_NOSIZE;
        }
        if show {
            flags |= SWP_SHOWWINDOW;
        }
        if !activate {
            flags |= SWP_NOACTIVATE;
        }
        let after = match z {
            ZOrder::Topmost => HWND_TOPMOST,
            ZOrder::Top => HWND_TOP,
            ZOrder::After(w) => HWND(w.0 as *mut c_void),
            ZOrder::Keep => {
                flags |= SWP_NOZORDER;
                HWND::default()
            }
        };
        (after, flags)
    }

    pub(super) fn place_window(
        hwnd: HWND,
        rect: Option<PhysRect>,
        z: ZOrder,
        show: bool,
        activate: bool,
    ) -> Result<(), WindowError> {
        let (after, flags) = place_plan(rect.is_none(), z, show, activate);
        let r = rect.unwrap_or_default();
        // SAFETY: 살아 있는 창 핸들(`hwnd_of` 가 걸렀다)과 값 인자뿐이다.
        unsafe { SetWindowPos(hwnd, after, r.x, r.y, r.width, r.height, flags) }
            .map_err(|e| os("SetWindowPos", e))?;
        if activate {
            // 거절(거짓)은 실패가 아니다 — `place_window` 문서.
            // SAFETY: 살아 있는 창 핸들 하나.
            let _ = unsafe { SetForegroundWindow(hwnd) };
        }
        Ok(())
    }

    pub(super) fn hide_window(hwnd: HWND) {
        // 돌려주는 값은 전의 보임이지 성공 여부가 아니다.
        // SAFETY: 살아 있는 창 핸들 하나.
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    }

    pub(super) fn is_visible(hwnd: HWND) -> bool {
        // SAFETY: 값만 보는 조회.
        unsafe { IsWindowVisible(hwnd) }.as_bool()
    }

    pub(super) fn move_window(hwnd: HWND, origin: PhysPoint) -> Result<(), WindowError> {
        // SAFETY: 살아 있는 창 핸들과 값 인자뿐이다.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND::default(),
                origin.x,
                origin.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| os("SetWindowPos", e))
    }

    fn to_rect(r: RECT) -> PhysRect {
        PhysRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        }
    }

    fn to_win(r: PhysRect) -> RECT {
        RECT {
            left: r.x,
            top: r.y,
            right: r.x + r.width,
            bottom: r.y + r.height,
        }
    }

    fn ex_style(hwnd: HWND) -> Result<u32, WindowError> {
        // SAFETY: 살아 있는 창 핸들. 0 은 정당한 값일 수 있어 마지막 오류를 지운 뒤 읽고 함께 본다(MS 문서).
        unsafe {
            SetLastError(WIN32_ERROR(0));
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if ex == 0 && GetLastError().0 != 0 {
                return Err(WindowError::Os(io::Error::last_os_error()));
            }
            Ok(ex as u32)
        }
    }

    pub(super) fn restore_placement(
        hwnd: HWND,
        normal: PhysRect,
        maximized: bool,
        after: Option<NativeWindow>,
        activate: bool,
    ) -> Result<(), WindowError> {
        let tool = ex_style(hwnd)? & WS_EX_TOOLWINDOW.0 != 0;
        let stored = if tool {
            normal
        } else {
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            // SAFETY: 살아 있는 지역 사각형 · 구조체를 가리키는 포인터뿐이다. `NEAREST` 라 모니터는 늘 있다.
            let ok = unsafe {
                let monitor = MonitorFromRect(&to_win(normal), MONITOR_DEFAULTTONEAREST);
                GetMonitorInfoW(monitor, &mut mi)
            };
            if !ok.as_bool() {
                return Err(WindowError::Os(io::Error::other("GetMonitorInfoW 실패")));
            }
            screen_to_workspace(normal, to_rect(mi.rcMonitor), to_rect(mi.rcWork))
        };
        let mut wp = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        // SAFETY: 살아 있는 창 핸들과 길이를 채운 지역 구조체.
        unsafe { GetWindowPlacement(hwnd, &mut wp) }.map_err(|e| os("GetWindowPlacement", e))?;
        wp.flags = WINDOWPLACEMENT_FLAGS(0);
        wp.rcNormalPosition = to_win(stored);
        let show = if maximized {
            SW_SHOWMAXIMIZED
        } else if activate {
            SW_SHOWNORMAL
        } else {
            SW_SHOWNOACTIVATE
        };
        wp.showCmd = show.0 as u32;
        // SAFETY: 위와 같다.
        unsafe { SetWindowPlacement(hwnd, &wp) }.map_err(|e| os("SetWindowPlacement", e))?;
        if let Some(after) = after {
            // SAFETY: 살아 있는 창 핸들과 값 인자. 기준 창이 사라졌으면 OS 가 실패를 돌려준다.
            unsafe {
                SetWindowPos(
                    hwnd,
                    HWND(after.0 as *mut c_void),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
            }
            .map_err(|e| os("SetWindowPos", e))?;
        }
        Ok(())
    }

    pub(super) fn set_switcher_hidden(hwnd: HWND, hidden: bool) -> Result<(), WindowError> {
        let ex = ex_style(hwnd)?;
        let next = switcher_ex_style(ex, hidden, WS_EX_TOOLWINDOW.0, WS_EX_APPWINDOW.0);
        if next != ex {
            // SAFETY: 살아 있는 창 핸들. 전 값 0 이 정당할 수 있어 마지막 오류를 지운 뒤 함께 본다(MS 문서).
            unsafe {
                SetLastError(WIN32_ERROR(0));
                let prev = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as i32 as isize);
                if prev == 0 && GetLastError().0 != 0 {
                    return Err(WindowError::Os(io::Error::last_os_error()));
                }
            }
        }
        // 틀 스타일을 바꾼 뒤 OS 가 틀을 다시 셈하게 한다(MS `SetWindowLongPtr` 문서).
        // SAFETY: 살아 있는 창 핸들과 값 인자.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND::default(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| os("SetWindowPos", e))
    }

    fn cloaked(hwnd: HWND) -> bool {
        let mut v = 0u32;
        // SAFETY: 살아 있는 지역 u32 에 4 바이트를 쓰는 조회. DWM 이 없거나 실패하면 가려지지 않은 것으로 친다.
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut v as *mut u32 as *mut c_void,
                std::mem::size_of::<u32>() as u32,
            )
        }
        .is_ok()
            && v != 0
    }

    /// 그 창을 만든 프로세스 — 사라진 창이면 0.
    fn pid_of(hwnd: HWND) -> u32 {
        let mut pid = 0u32;
        // SAFETY: 살아 있는 지역 변수에 PID 를 쓰는 조회 — 사라진 창이면 0 으로 남는다.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid
    }

    /// 창 모양의 사본 — `GetWindowRgn` 이 부르는 쪽이 만든 영역에 베껴 준다. 버리면 지운다.
    struct Region(HRGN);

    impl Drop for Region {
        fn drop(&mut self) {
            // SAFETY: 이 값이 만든 영역이고 창에 넘기지 않았다(베껴 받기만 했다).
            let _ = unsafe { DeleteObject(self.0) };
        }
    }

    /// 그 창의 창 모양 — 없거나 · 못 읽었거나 · 받을 영역을 못 만들었으면 `None`.
    fn window_region(hwnd: HWND) -> Option<Region> {
        // SAFETY: 값 인자뿐이다 — 실패하면 null.
        let rgn = unsafe { CreateRectRgn(0, 0, 0, 0) };
        if rgn.is_invalid() {
            return None;
        }
        let rgn = Region(rgn);
        // SAFETY: 살아 있는 영역 핸들 — 창이 사라졌거나 모양이 없으면 `RGN_ERROR`.
        (unsafe { GetWindowRgn(hwnd, rgn.0) } != RGN_ERROR).then_some(rgn)
    }

    fn region_hit(hwnd: HWND, outer: RECT, rtl: bool, p: PhysPoint) -> RegionHit {
        let Some(rgn) = window_region(hwnd) else {
            return RegionHit::NoRegion;
        };
        // 창 모양 좌표는 바깥 사각형의 왼쪽 위가 0 이다(MS `GetWindowRgn` 문서). `WS_EX_LAYOUTRTL` 창은 오른쪽 위가 0
        //   이고 x 가 왼쪽으로 는다 — 맨 오른쪽 픽셀이 x = 0(실측 — OS 적중 판정 `WindowFromPoint` 와 견준 시험
        //   `top_level_at_mirrors_the_region_of_rtl_windows`). DPI 인식이 다른 다른 앱 창의 모양이 그 창의 논리
        //   좌표로 오는지는 재지 않았다 — 그러면 그 창의 모양 판정이 배율만큼 어긋난다.
        let x = if rtl {
            outer.right - 1 - p.x
        } else {
            p.x - outer.left
        };
        // SAFETY: 살아 있는 영역 핸들과 값 인자.
        if unsafe { PtInRegion(rgn.0, x, p.y - outer.top) }.as_bool() {
            RegionHit::Inside
        } else {
            RegionHit::Outside
        }
    }

    fn layered_kind(hwnd: HWND, ex: u32) -> LayeredKind {
        if ex & WS_EX_LAYERED.0 == 0 {
            return LayeredKind::NotLayered;
        }
        let mut alpha = 0u8;
        let mut flags = LAYERED_WINDOW_ATTRIBUTES_FLAGS(0);
        // 다른 프로세스의 창에도 답하는지는 재지 않았다 — 못 받으면 `PerPixel` 이라 가리는 창(PRD R15 쪽)으로 떨어진다.
        // SAFETY: 살아 있는 지역 변수에 쓰는 조회다(색 키 값은 받지 않는다). 사라진 창이면 실패한다.
        if unsafe { GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), Some(&mut flags)) }
            .is_err()
        {
            return LayeredKind::PerPixel;
        }
        if flags.0 & LWA_COLORKEY.0 != 0 {
            LayeredKind::ColorKey
        } else if flags.0 & LWA_ALPHA.0 != 0 {
            LayeredKind::Alpha(alpha)
        } else {
            LayeredKind::Alpha(u8::MAX)
        }
    }

    fn candidate(hwnd: HWND, p: PhysPoint) -> bool {
        let mut r = RECT::default();
        // SAFETY: 걷는 동안 사라진 창이면 조회가 실패하거나 거짓을 돌려준다 — 그 창은 건너뛴다.
        unsafe {
            if GetWindowRect(hwnd, &mut r).is_err() || !contains(to_rect(r), p) {
                return false;
            }
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return false;
            }
        }
        let Ok(ex) = ex_style(hwnd) else {
            return false;
        };
        let ours = pid_of(hwnd) == std::process::id();
        let transparent = ex & WS_EX_TRANSPARENT.0 != 0;
        let rtl = ex & WS_EX_LAYOUTRTL.0 != 0;
        hit_accepts(
            ours,
            layered_kind(hwnd, ex),
            transparent,
            region_hit(hwnd, r, rtl, p),
        ) && !cloaked(hwnd)
    }

    pub(super) fn describe(w: NativeWindow) -> Option<WindowFacts> {
        let hwnd = HWND(w.0 as *mut c_void);
        // SAFETY: 값만 보는 조회 — 사라진 핸들이면 거짓이다.
        if !unsafe { IsWindow(hwnd) }.as_bool() {
            return None;
        }
        // 창 클래스 이름은 256 자를 넘지 않는다(MS `WNDCLASSEXW` 문서).
        let mut class = [0u16; 256];
        // SAFETY: 살아 있는 지역 버퍼 — 돌려준 길이만큼 썼다(실패면 0).
        let len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
        let ex = ex_style(hwnd).unwrap_or(0);
        Some(WindowFacts {
            pid: pid_of(hwnd),
            class: String::from_utf16_lossy(&class[..len.min(class.len())]),
            ex_style: ex,
            layered: layered_kind(hwnd, ex),
            rtl: ex & WS_EX_LAYOUTRTL.0 != 0,
            has_region: window_region(hwnd).is_some(),
        })
    }

    pub(super) fn top_level_at(p: PhysPoint, skip: &[NativeWindow]) -> Option<NativeWindow> {
        // SAFETY: 인자 없는(null = 바탕) 조회 — 최상위 창 z 순서의 맨 위.
        let mut hwnd = unsafe { GetTopWindow(HWND::default()) }.ok()?;
        for _ in 0..MAX_Z_WALK {
            if !skip.contains(&native(hwnd)) && candidate(hwnd, p) {
                return Some(native(hwnd));
            }
            // SAFETY: 값 조회 — 맨 아래거나 그 사이 사라진 창이면 실패한다.
            hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT) }.ok()?;
        }
        None
    }
}

#[cfg(windows)]
pub(crate) use win::hwnd_of;

#[cfg(test)]
mod tests {
    use super::*;

    // ── 순수 셈 ──

    #[test]
    fn workspace_offset_follows_work_area_origin() {
        let monitor = PhysRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let normal = PhysRect {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
        };
        // (작업 영역, 기대) — 작업 표시줄 아래 · 위 · 왼쪽 · 둘째 모니터의 위
        let cases = [
            (
                PhysRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1040,
                },
                (100, 200),
            ),
            (
                PhysRect {
                    x: 0,
                    y: 40,
                    width: 1920,
                    height: 1040,
                },
                (100, 160),
            ),
            (
                PhysRect {
                    x: 60,
                    y: 0,
                    width: 1860,
                    height: 1080,
                },
                (40, 200),
            ),
        ];
        for (work, (x, y)) in cases {
            let got = screen_to_workspace(normal, monitor, work);
            assert_eq!(
                (got.x, got.y, got.width, got.height),
                (x, y, 800, 600),
                "work = {work:?}"
            );
        }
        let second = PhysRect {
            x: 1920,
            y: -200,
            width: 2560,
            height: 1440,
        };
        let second_work = PhysRect {
            x: 1920,
            y: -150,
            width: 2560,
            height: 1390,
        };
        let on_second = PhysRect {
            x: 2000,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(
            screen_to_workspace(on_second, second, second_work),
            PhysRect {
                x: 2000,
                y: -50,
                width: 800,
                height: 600
            }
        );
    }

    #[test]
    fn switcher_bits_flip_both_ways() {
        const TOOL: u32 = 0x80;
        const APP: u32 = 0x40000;
        const OTHER: u32 = 0x100;
        assert_eq!(
            switcher_ex_style(APP | OTHER, true, TOOL, APP),
            TOOL | OTHER
        );
        assert_eq!(
            switcher_ex_style(TOOL | OTHER, false, TOOL, APP),
            APP | OTHER
        );
        assert_eq!(switcher_ex_style(TOOL, true, TOOL, APP), TOOL);
        assert_eq!(switcher_ex_style(0, false, TOOL, APP), APP);
    }

    #[test]
    fn contains_is_half_open() {
        let r = PhysRect {
            x: 10,
            y: 20,
            width: 5,
            height: 4,
        };
        assert!(contains(r, PhysPoint { x: 10, y: 20 }));
        assert!(contains(r, PhysPoint { x: 14, y: 23 }));
        assert!(!contains(r, PhysPoint { x: 15, y: 20 }));
        assert!(!contains(r, PhysPoint { x: 10, y: 24 }));
        assert!(!contains(r, PhysPoint { x: 9, y: 20 }));
    }

    #[test]
    fn hit_accepts_skips_see_through_foreign_windows_and_points_outside_the_region() {
        use LayeredKind::{Alpha, ColorKey, NotLayered, PerPixel};
        use RegionHit::{Inside, NoRegion, Outside};
        // (ours, layered, transparent, region, 기대)
        let cases = [
            // 다른 앱 창
            (false, NotLayered, false, NoRegion, true),
            (false, NotLayered, true, NoRegion, true),
            (false, Alpha(255), false, NoRegion, true),
            (false, Alpha(1), false, NoRegion, true),
            (false, Alpha(0), false, NoRegion, false),
            (false, ColorKey, false, NoRegion, false),
            (false, PerPixel, false, NoRegion, true),
            (false, Alpha(255), true, NoRegion, false),
            (false, PerPixel, true, NoRegion, false),
            // 우리 창 — 클릭 통과만 건너뛴다
            (true, NotLayered, false, NoRegion, true),
            (true, NotLayered, true, NoRegion, true),
            (true, Alpha(0), false, NoRegion, true),
            (true, ColorKey, false, NoRegion, true),
            (true, PerPixel, false, NoRegion, true),
            (true, Alpha(255), true, NoRegion, false),
            (true, PerPixel, true, NoRegion, false),
            // 창 모양
            (false, NotLayered, false, Inside, true),
            (false, NotLayered, false, Outside, false),
            (false, Alpha(255), false, Inside, true),
            (false, ColorKey, false, Inside, false),
            (true, NotLayered, false, Inside, true),
            (true, NotLayered, false, Outside, false),
        ];
        for (ours, layered, transparent, region, want) in cases {
            assert_eq!(
                hit_accepts(ours, layered, transparent, region),
                want,
                "ours={ours} layered={layered:?} transparent={transparent} region={region:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn place_plan_maps_each_argument_to_one_flag() {
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_TOP, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
            SWP_SHOWWINDOW,
        };
        let (after, flags) = win::place_plan(true, ZOrder::Keep, true, false);
        assert!(after.is_invalid());
        assert_eq!(
            flags,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOZORDER
        );
        let (after, flags) = win::place_plan(false, ZOrder::Topmost, false, true);
        assert_eq!(after, HWND_TOPMOST);
        assert_eq!(flags.0, 0);
        let (after, _) = win::place_plan(false, ZOrder::Top, false, false);
        assert_eq!(after, HWND_TOP);
        let (after, flags) =
            win::place_plan(false, ZOrder::After(NativeWindow(0x1234)), false, false);
        assert_eq!(after.0 as isize, 0x1234);
        assert_eq!(flags, SWP_NOACTIVATE);
    }

    // ── 실 창 — 화면 밖 자리 · 활성 없음(보는 사람 화면에 안 나온다) ──

    #[cfg(windows)]
    mod real {
        use super::super::*;
        use crate::test_window::TestWindow;
        use windows::core::w;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetWindowLongPtrW, GetWindowRect, SetWindowPos,
            ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
            SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_EX_APPWINDOW,
            WS_EX_TOOLWINDOW, WS_POPUP,
        };

        /// 확장 스타일을 고르는 테두리 없는 도구 창 — [`TestWindow`] 는 확장 스타일을 못 고른다. `rect` 에 늘 위로
        /// 활성 없이 보인다. 버리면 부순다.
        struct StyledWindow(HWND);

        impl StyledWindow {
            fn new(ex: WINDOW_EX_STYLE, rect: PhysRect) -> Self {
                // `SS_NOTIFY`(이 crate 가 켜지 않은 `Win32_UI_Controls` 의 상수) — 없으면 STATIC 이 OS 적중 판정에
                //   답하지 않아 `WindowFromPoint` 가 그 창을 지나간다(실측).
                const SS_NOTIFY: WINDOW_STYLE = WINDOW_STYLE(0x100);
                // SAFETY: 시스템 클래스 이름 · 값 인자뿐이다. 인스턴스 · 부모 · 메뉴 없음.
                let hwnd = unsafe {
                    CreateWindowExW(
                        WS_EX_TOOLWINDOW | ex,
                        w!("STATIC"),
                        w!("engram-platform-test"),
                        WS_POPUP | SS_NOTIFY,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        HWND::default(),
                        None,
                        None,
                        None,
                    )
                }
                .expect("시험 창 짓기");
                let w = Self(hwnd);
                // SAFETY: 이 값이 지은 살아 있는 창과 값 인자.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        SWP_SHOWWINDOW | SWP_NOACTIVATE,
                    )
                }
                .expect("시험 창 보이기");
                w
            }

            fn native(&self) -> NativeWindow {
                NativeWindow(self.0 .0 as isize)
            }
        }

        impl Drop for StyledWindow {
            fn drop(&mut self) {
                // SAFETY: 이 값이 지은 창.
                let _ = unsafe { DestroyWindow(self.0) };
            }
        }

        #[test]
        fn native_window_of_reads_the_win32_handle() {
            let w = TestWindow::new(false);
            let n = NativeWindow::of(&w).expect("Win32 핸들");
            assert_eq!(n, NativeWindow(w.hwnd().0 as isize));
        }

        #[test]
        fn place_window_moves_sizes_and_shows_without_activation() {
            let w = TestWindow::new(true);
            assert!(!is_window_visible(&w));
            let rect = PhysRect {
                x: -21000,
                y: -21000,
                width: 300,
                height: 200,
            };
            place_window(&w, Some(rect), ZOrder::Top, true, false).unwrap();
            assert!(is_window_visible(&w));
            assert_eq!(w.outer(), rect);
        }

        #[test]
        fn place_window_without_rect_keeps_place_and_size() {
            let w = TestWindow::new(true);
            let rect = PhysRect {
                x: -21500,
                y: -21500,
                width: 320,
                height: 180,
            };
            place_window(&w, Some(rect), ZOrder::Keep, false, false).unwrap();
            assert!(!is_window_visible(&w), "show 거짓이면 보이지 않는다");
            place_window(&w, None, ZOrder::Keep, true, false).unwrap();
            assert!(is_window_visible(&w));
            assert_eq!(w.outer(), rect);
        }

        #[test]
        fn hide_and_move_keep_the_rest() {
            let w = TestWindow::new(true);
            let rect = PhysRect {
                x: -22000,
                y: -22000,
                width: 260,
                height: 140,
            };
            place_window(&w, Some(rect), ZOrder::Keep, true, false).unwrap();
            move_window(
                &w,
                PhysPoint {
                    x: -22100,
                    y: -22050,
                },
            )
            .unwrap();
            assert_eq!(
                w.outer(),
                PhysRect {
                    x: -22100,
                    y: -22050,
                    width: 260,
                    height: 140
                }
            );
            assert!(is_window_visible(&w));
            hide_window(&w).unwrap();
            assert!(!is_window_visible(&w));
        }

        #[test]
        fn closed_window_is_no_window() {
            let w = TestWindow::new(true);
            let gone = w.handle_only();
            drop(w);
            assert!(matches!(hide_window(&gone), Err(WindowError::NoWindow)));
            assert!(matches!(
                move_window(&gone, PhysPoint::default()),
                Err(WindowError::NoWindow)
            ));
            assert!(!is_window_visible(&gone));
        }

        #[test]
        fn switcher_hidden_flips_tool_and_app_bits() {
            let w = TestWindow::new(false);
            set_switcher_hidden(&w, true).unwrap();
            // SAFETY: 이 시험이 쥔 살아 있는 창.
            let ex = unsafe { GetWindowLongPtrW(w.hwnd(), GWL_EXSTYLE) } as u32;
            assert_ne!(ex & WS_EX_TOOLWINDOW.0, 0);
            assert_eq!(ex & WS_EX_APPWINDOW.0, 0);
            set_switcher_hidden(&w, false).unwrap();
            // SAFETY: 위와 같다.
            let ex = unsafe { GetWindowLongPtrW(w.hwnd(), GWL_EXSTYLE) } as u32;
            assert_eq!(ex & WS_EX_TOOLWINDOW.0, 0);
            assert_ne!(ex & WS_EX_APPWINDOW.0, 0);
        }

        #[test]
        fn top_level_at_skips_listed_hidden_and_minimized_windows() {
            let below = TestWindow::new(true);
            let above = TestWindow::new(true);
            let rect = PhysRect {
                x: -23000,
                y: -23000,
                width: 200,
                height: 200,
            };
            place_window(&below, Some(rect), ZOrder::Topmost, true, false).unwrap();
            place_window(&above, Some(rect), ZOrder::Topmost, true, false).unwrap();
            let p = PhysPoint {
                x: -22900,
                y: -22900,
            };
            let (a, b) = (
                NativeWindow::of(&above).unwrap(),
                NativeWindow::of(&below).unwrap(),
            );
            assert_eq!(top_level_at(p, &[]), Some(a));
            assert_eq!(top_level_at(p, &[a]), Some(b));
            assert_eq!(top_level_at(p, &[a, b]), None);
            let right_edge = PhysPoint {
                x: rect.x + rect.width,
                y: p.y,
            };
            assert_eq!(top_level_at(right_edge, &[]), None, "오른쪽 변은 밖이다");
            hide_window(&above).unwrap();
            assert_eq!(top_level_at(p, &[]), Some(b), "숨은 창은 대상이 아니다");
            // SAFETY: 이 시험이 쥔 살아 있는 창 — 활성 없이 최소화한다.
            unsafe {
                let _ = ShowWindow(below.hwnd(), SW_SHOWMINNOACTIVE);
            }
            let mut r = windows::Win32::Foundation::RECT::default();
            // SAFETY: 위와 같다.
            unsafe { GetWindowRect(below.hwnd(), &mut r) }.unwrap();
            let in_icon = PhysPoint {
                x: r.left,
                y: r.top,
            };
            assert_ne!(
                top_level_at(in_icon, &[]),
                Some(b),
                "최소화된 창은 대상이 아니다"
            );
            // SAFETY: 위와 같다.
            unsafe {
                let _ = ShowWindow(above.hwnd(), SW_SHOWNOACTIVATE);
            }
            assert_eq!(top_level_at(p, &[]), Some(a));
        }

        #[test]
        fn top_level_at_skips_points_outside_the_window_region() {
            use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
            let below = TestWindow::new(true);
            let above = TestWindow::new(true);
            let rect = PhysRect {
                x: -25000,
                y: -25000,
                width: 200,
                height: 200,
            };
            place_window(&below, Some(rect), ZOrder::Topmost, true, false).unwrap();
            place_window(&above, Some(rect), ZOrder::Topmost, true, false).unwrap();
            let (a, b) = (
                NativeWindow::of(&above).unwrap(),
                NativeWindow::of(&below).unwrap(),
            );
            assert!(!describe_window(a).unwrap().has_region);
            // SAFETY: 이 시험이 쥔 살아 있는 창 — 성공하면 영역은 OS 것이 된다(지우지 않는다).
            unsafe {
                let rgn = CreateRectRgn(0, 0, 100, 100);
                assert_ne!(SetWindowRgn(above.hwnd(), rgn, false), 0);
            }
            let inside = PhysPoint {
                x: rect.x + 50,
                y: rect.y + 50,
            };
            let outside = PhysPoint {
                x: rect.x + 150,
                y: rect.y + 150,
            };
            assert_eq!(top_level_at(inside, &[]), Some(a), "모양 안");
            assert_eq!(top_level_at(outside, &[]), Some(b), "모양 밖이면 밑 창");
            let facts = describe_window(a).unwrap();
            assert!(facts.has_region);
            assert_eq!(facts.pid, std::process::id());
            assert!(
                facts.class.eq_ignore_ascii_case("static"),
                "{}",
                facts.class
            );
        }

        #[test]
        fn top_level_at_mirrors_the_region_of_rtl_windows() {
            use windows::Win32::Foundation::POINT;
            use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
            use windows::Win32::UI::WindowsAndMessaging::{WindowFromPoint, WS_EX_LAYOUTRTL};
            let rect = PhysRect {
                x: -26000,
                y: -26000,
                width: 200,
                height: 200,
            };
            for (ex, rtl) in [(WINDOW_EX_STYLE(0), false), (WS_EX_LAYOUTRTL, true)] {
                let w = StyledWindow::new(ex, rect);
                // SAFETY: 이 시험이 쥔 살아 있는 창 — 성공하면 영역은 OS 것이 된다(지우지 않는다).
                unsafe {
                    assert_ne!(SetWindowRgn(w.0, CreateRectRgn(0, 0, 100, 200), false), 0);
                }
                assert_eq!(describe_window(w.native()).unwrap().rtl, rtl);
                // 모양 x [0, 100) — 왼쪽 배치면 왼쪽 절반 · 오른쪽 배치면 오른쪽 절반이 그 창이다.
                for dx in [0, 99, 100, 199] {
                    let p = PhysPoint {
                        x: rect.x + dx,
                        y: rect.y + 50,
                    };
                    let want = (dx < 100) != rtl;
                    // SAFETY: 값 인자 조회.
                    let os_hit = unsafe { WindowFromPoint(POINT { x: p.x, y: p.y }) } == w.0;
                    assert_eq!(os_hit, want, "OS 적중 판정 rtl={rtl} dx={dx}");
                    assert_eq!(
                        top_level_at(p, &[]) == Some(w.native()),
                        want,
                        "rtl={rtl} dx={dx}"
                    );
                }
            }
        }

        #[test]
        fn transparent_only_window_still_takes_the_click() {
            use windows::Win32::Foundation::POINT;
            use windows::Win32::UI::WindowsAndMessaging::{WindowFromPoint, WS_EX_TRANSPARENT};
            let rect = PhysRect {
                x: -28000,
                y: -28000,
                width: 120,
                height: 90,
            };
            let w = StyledWindow::new(WS_EX_TRANSPARENT, rect);
            let p = PhysPoint {
                x: rect.x + 10,
                y: rect.y + 10,
            };
            // SAFETY: 값 인자 조회.
            let os_hit = unsafe { WindowFromPoint(POINT { x: p.x, y: p.y }) } == w.0;
            assert!(os_hit, "OS 적중 판정");
            assert_eq!(top_level_at(p, &[]), Some(w.native()));
        }

        #[test]
        fn describe_window_reads_layered_attributes() {
            use windows::Win32::Foundation::COLORREF;
            use windows::Win32::UI::WindowsAndMessaging::{
                SetLayeredWindowAttributes, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
                LWA_COLORKEY, WS_EX_LAYERED,
            };
            let rect = PhysRect {
                x: -27000,
                y: -27000,
                width: 120,
                height: 90,
            };
            let plain = StyledWindow::new(WINDOW_EX_STYLE(0), rect);
            assert_eq!(
                describe_window(plain.native()).unwrap().layered,
                LayeredKind::NotLayered
            );
            drop(plain);
            let w = StyledWindow::new(WS_EX_LAYERED, rect);
            assert_eq!(
                describe_window(w.native()).unwrap().layered,
                LayeredKind::PerPixel,
                "속성을 아직 안 준 layered 창"
            );
            let cases = [
                (LWA_ALPHA, 0, LayeredKind::Alpha(0)),
                (LWA_ALPHA, 128, LayeredKind::Alpha(128)),
                (
                    LAYERED_WINDOW_ATTRIBUTES_FLAGS(0),
                    0,
                    LayeredKind::Alpha(255),
                ),
                (LWA_COLORKEY, 255, LayeredKind::ColorKey),
                (LWA_ALPHA | LWA_COLORKEY, 0, LayeredKind::ColorKey),
            ];
            for (flags, alpha, want) in cases {
                // SAFETY: 이 시험이 쥔 살아 있는 layered 창과 값 인자.
                unsafe { SetLayeredWindowAttributes(w.0, COLORREF(0), alpha, flags) }
                    .expect("layered 속성");
                assert_eq!(
                    describe_window(w.native()).unwrap().layered,
                    want,
                    "flags={flags:?} alpha={alpha}"
                );
            }
            // 우리 창은 클릭 통과만 건너뛴다 — 색 키 창도 답이다(`hit_accepts`).
            let p = PhysPoint {
                x: rect.x + 10,
                y: rect.y + 10,
            };
            assert_eq!(top_level_at(p, &[]), Some(w.native()));
        }

        /// 보통(도구 아닌) 창을 화면 안에 보이므로 보는 사람 화면에 잠깐 나온다 — 데스크톱 세션에서 손으로 돌린다.
        #[test]
        #[ignore = "화면 안에 창을 잠깐 보인다 — 데스크톱 세션에서 손으로"]
        fn restore_placement_round_trips_screen_coordinates() {
            let w = TestWindow::new(false);
            let normal = PhysRect {
                x: 200,
                y: 200,
                width: 400,
                height: 300,
            };
            restore_placement(&w, normal, false, None, false).unwrap();
            assert!(is_window_visible(&w));
            assert_eq!(w.outer(), normal);
            hide_window(&w).unwrap();
        }
    }
}
