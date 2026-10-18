//! 창 이동 루프 — 떼어진 창이 커서를 따라오게 OS 의 모달 이동 루프를 세우고, 그 끝을 정확히 한 번 알린다(TRD
//! S21-slot-docking §8 · 조사 A6 — Chromium `HWNDMessageHandler::RunMoveLoop` · `MoveLoopMouseWatcher` 의 꼴).
//!
//! - **시작은 올린다** — [`begin_move_loop`] 는 캡처를 풀고 이 루프만의 시작 신호(루프 번호를 실은 등록 메시지)를
//!   그 창에 올린 뒤 곧바로 돌아온다. 메시지 펌프가 그 신호를 나르면 아래 Posted 확인을 하고, 넘길 때만 그
//!   자리에서 `WM_SYSCOMMAND(SC_MOVE | 2)` 를 보내 루프를 세운다. Chromium 처럼 동기로 보내면(IPC 처리기 안에서
//!   시작하면) 루프 안의 웹뷰 → 셸 IPC 왕복이 0/21 이었다(스파이크 S3 · TRD §1 스택 맞춤 ②). `SC_MOVE` 자체를
//!   올리지 않는 까닭은 `windows` 갈래 머리.
//! - **알림은 [`LoopEvent`] 둘** — 들어감([`LoopEvent::Entered`] — 많아야 한 번)과 끝([`LoopEvent::Ended`] — 어느
//!   길로든 정확히 한 번 · 그 뒤로는 아무 알림도 없다). 끝 까닭의 우선순위와 각 상태의 처리는 아래 「상태」 절이
//!   정본이다.
//! - **끝 까닭은 Chromium 그대로 고른다** — 그 루프에서 주 버튼 뗌(`WM_LBUTTONUP`)을 봤어야 놓기(`Released`)이고,
//!   뗌 없이 끝난 루프는 Esc 든 Enter 든 취소다(hwnd_message_handler.cc#942-961 — OS 는 루프가 왜 끝났는지 알려
//!   주지 않는다 · TRD C9).
//! - **Esc 훅** — 루프 동안(올린 때부터) Esc 가 눌림 · 뗌 · 반복 어느 꼴로든 오면 그 창의 DWM 전환을 늘 끄고,
//!   `hide_on_escape` 면 그 창을 숨긴다(hwnd_message_handler.cc#189-201 · TRD E9). 숨겼는지는 끝 알림의 `hid` 로만
//!   알린다 — 끝 까닭과 따로다(TRD F2).
//!
//! ## 상태
//!
//! | 상태 | 무엇 | 그때 온 것 → 하는 일 |
//! |---|---|---|
//! | Posted | 올렸고 아직 안 닿음 | 시작 신호 도착 → 차례대로 본다: 끝내라는 청이 있었으면 `Ended` · Esc 를 봤으면 `Canceled{esc: true}` · 주 버튼이 이미 떨어졌으면 `Released{before_entry: true}` — 셋 다 `SC_MOVE` 를 보내지 않아 루프가 아예 서지 않는다 · 아니면 `SC_MOVE` 를 보내 루프로 넘긴다(Started). 같은 창에 따로 온 `SC_MOVE` 는 이 루프의 것이 아니다 |
//! | Started | 넘겼고 아직 `WM_ENTERSIZEMOVE` 전 | 끝내라는 청 → 적어 두었다 들어갈 때 처리 · 보낸 `SC_MOVE` 가 루프 없이 돌아오면(이 프로시저 위의 다른 서브클래스가 먹었다) 아래 「끝」 줄대로 끝 |
//! | Entering | `WM_ENTERSIZEMOVE` 처리 중 | 들어오며 `Entered` · 주 버튼이 떨어졌거나 끝내라는 청이 있었으면 `WM_CANCELMODE` 를 ★올린다★(이 처리기 안에서는 루프가 아직 제 펌프 · 캡처를 세우기 전이라 동기로 보낸 취소가 닿지 않을 수 있고, 올린 것은 루프 자신의 펌프가 나른다 — TRD E10 · 미확인 → M17) · 이때의 끝내라는 청도 올린다 |
//! | Running | 루프가 돈다 | 끝내라는 청 → `WM_CANCELMODE` 를 보낸다(같은 스레드라 곧바로 불린다) |
//! | 끝 | `WM_SYSCOMMAND` 가 돌아옴 | 끝내라는 청 → `Ended` · 들어갈 때 버튼이 떨어져 있었으면 `Released{before_entry: true}` · 뗌을 봤거나 마우스 훅을 못 걸었으면 `Released{before_entry: false}` · 아니면 `Canceled{esc: Esc 를 봤나}` |
//!
//! 창이 사라지면(`WM_NCDESTROY`) 어느 상태든 `Destroyed` 다. 끝을 낸 뒤 [`end_move_loop`] 는 아무것도 하지 않는다.
//!
//! ## 스레드 · 부르는 쪽 계약
//!
//! - [`begin_move_loop`] · [`end_move_loop`] 는 그 창을 만든 스레드(메인 — UI 스레드)에서 부른다. 훅은 그 스레드의
//!   스레드 지역 훅이고 상태도 그 스레드의 것이다 — 다른 스레드의 [`end_move_loop`] 는 아무것도 하지 않는다.
//! - 알림은 그 창의 창 프로시저 안(OS 가 부르는 자리)에서 온다 — 루프가 도는 중이면 OS 의 모달 루프 안이다. 받는
//!   쪽은 막지 말고 제 줄에 넣기만 한다(TRD §2-4). 받는 쪽이 그 안에서 [`end_move_loop`] · 창 부수기를 불러도 알림은
//!   정확히 한 번이다(재진입 · 깊이 가드 — `windows` 갈래 머리).
//! - 받는 쪽 패닉은 가두지 않는다(ADR-0292 — 가두지 못한 패닉은 훅이 로그한 뒤 끝낸다 · 훅이 없어도 OS 가 부르는
//!   프로시저 경계를 넘는 되감기는 abort 다).
//! - 한 스레드에 루프는 하나다 — 앞 루프가 끝을 내기 전의 [`begin_move_loop`] 는 [`MoveLoopError::Busy`] 다.

// ADR-0266
// ADR-0295

use std::fmt;
use std::io;

use raw_window_handle::HasWindowHandle;

#[cfg(windows)]
mod windows;

/// [`begin_move_loop`] 의 선택.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MoveLoopOpts {
    /// Esc 를 보면 그 창을 숨긴다 — 끄는 동안 새로 뗀 창이면 참(조사 E1). DWM 전환 끄기는 이 깃발과 무관하게 늘
    /// 한다(TRD E9).
    pub hide_on_escape: bool,
}

/// 루프 알림.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEvent {
    /// OS 루프에 들어갔다(`WM_ENTERSIZEMOVE`). 들어가기 전에 끝나면 오지 않는다.
    Entered,
    /// 끝 — 어느 길로든 정확히 한 번이고 마지막 알림이다. `hid` = 이 루프의 Esc 훅이 그 창을 숨겼다
    /// (`hide_on_escape` 이고 Esc 를 어느 꼴로든 봤다) — 끝 까닭과 무관하게 싣는다(TRD F2).
    Ended { end: MoveLoopEnd, hid: bool },
}

/// 끝 까닭 — 모듈 머리 「상태」 표의 우선순위로 고른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveLoopEnd {
    /// 놓았다. `before_entry` = 루프에 들어가기 전(또는 들어가는 순간)에 버튼이 이미 떨어져 있었다 — 루프가 아예
    /// 안 섰거나 곧바로 취소됐다.
    Released { before_entry: bool },
    /// [`end_move_loop`] 로 끝냈다(다른 끝 까닭보다 앞선다).
    Ended,
    /// 뗌 없이 끝났다 — 놓기가 아니다. `esc` = 이 루프에서 Esc 를 봤다.
    Canceled { esc: bool },
    /// 그 창이 사라졌다.
    Destroyed,
}

/// [`begin_move_loop`] 의 실패 — 어느 것이든 루프는 서지 않았고 알림도 오지 않는다. 마우스 훅을 못 거는 것은 실패가
/// 아니다(Chromium 처럼 루프를 세우고 끝을 `Released` 로 친다 — TRD E7).
#[derive(Debug)]
pub enum MoveLoopError {
    /// 이 OS 에서는 이 수단이 없다 — Windows 밖이거나 Win32 창이 아닌 핸들이다.
    Unsupported,
    /// 핸들을 못 얻었거나 그 창이 이미 없다.
    NoWindow,
    /// 부르는 스레드가 그 창의 스레드가 아니다.
    WrongThread,
    /// 이 스레드의 앞 루프가 아직 끝을 내지 않았다.
    Busy,
    /// 창 서브클래스 · 메시지 올리기 같은 OS 호출이 실패했다.
    Os(io::Error),
}

impl fmt::Display for MoveLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("이 OS 에서는 창 이동 루프가 없다"),
            Self::NoWindow => f.write_str("창이 없다"),
            Self::WrongThread => f.write_str("그 창의 스레드가 아니다"),
            Self::Busy => f.write_str("앞 이동 루프가 아직 끝나지 않았다"),
            Self::Os(e) => write!(f, "이동 루프 OS 호출 실패: {e}"),
        }
    }
}

impl std::error::Error for MoveLoopError {}

/// 그 창의 이동 루프를 올린다 — 곧바로 돌아오고 루프는 메시지 펌프가 연다(모듈 머리). `Ok` 면 `on_event` 에 끝
/// 알림이 정확히 한 번 온다(돌아오기 전에 올 수도 있다 — 캡처를 놓는 사이 그 창이 부서지면 `Destroyed`). `Err` 면
/// 알림이 하나도 오지 않는다 — 어느 변이든 부르는 쪽은 시작 실패로 친다(TRD §8 · D1).
///
/// 부르는 스레드 = 그 창의 스레드(모듈 머리 「스레드」). 주 버튼이 눌려 있는 동안 불러야 루프가 선다 — 이미
/// 떨어졌으면 시작 신호가 닿을 때 루프 없이 `Released{before_entry: true}` 다.
// ADR-0295
pub fn begin_move_loop(
    w: &impl HasWindowHandle,
    opts: MoveLoopOpts,
    on_event: Box<dyn FnMut(LoopEvent)>,
) -> Result<(), MoveLoopError> {
    #[cfg(windows)]
    {
        let hwnd = crate::window::hwnd_of(w).map_err(|e| match e {
            crate::window::WindowError::Unsupported => MoveLoopError::Unsupported,
            crate::window::WindowError::NoWindow => MoveLoopError::NoWindow,
            crate::window::WindowError::Os(e) => MoveLoopError::Os(e),
        })?;
        windows::begin(hwnd, opts, on_event)
    }
    #[cfg(not(windows))]
    {
        let _ = (w, opts, on_event);
        Err(MoveLoopError::Unsupported)
    }
}

/// 그 창의 이동 루프를 끝낸다 — 상태에 따라 한다(모듈 머리 「상태」 표). 끝은 [`LoopEvent::Ended`] 의
/// [`MoveLoopEnd::Ended`] 로 온다. 그 창의 루프가 없거나 이미 끝을 냈거나 다른 스레드면 아무것도 하지 않는다.
// ADR-0295
pub fn end_move_loop(w: &impl HasWindowHandle) {
    #[cfg(windows)]
    {
        if let Ok(hwnd) = crate::window::hwnd_of(w) {
            windows::end(hwnd);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = w;
    }
}

// ── 순수 상태 — 메시지 · 깃발 → 할 일 · 끝 까닭(OS 호출 없음) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Posted,
    Started,
    Entering,
    Running,
    Done,
}

/// 올린 시작 신호가 닿았을 때 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    /// `SC_MOVE` 를 보내지 않고 이 까닭으로 끝낸다.
    Swallow(MoveLoopEnd),
    /// `SC_MOVE` 를 보낸다 — 루프가 선다.
    Pass,
    /// Posted 가 아니다 — 아무것도 안 한다.
    NotOurs,
}

/// 끝내라는 청에 할 OS 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cancel {
    None,
    Post,
    Send,
}

/// 루프 하나의 상태. 끝 까닭을 돌려주는 메서드는 끝을 낸 뒤로 `None` 이다 — 끝 알림 정확히 한 번의 정본.
#[derive(Debug)]
struct Core {
    stage: Stage,
    hide_on_escape: bool,
    hook_failed: bool,
    end_requested: bool,
    esc_seen: bool,
    mouse_up: bool,
    hid: bool,
    released_at_entry: bool,
    cancel_posted: bool,
}

impl Core {
    fn new(hide_on_escape: bool) -> Self {
        Self {
            stage: Stage::Posted,
            hide_on_escape,
            hook_failed: false,
            end_requested: false,
            esc_seen: false,
            mouse_up: false,
            hid: false,
            released_at_entry: false,
            cancel_posted: false,
        }
    }

    fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    fn hid(&self) -> bool {
        self.hid
    }

    fn finish(&mut self, end: MoveLoopEnd) -> Option<MoveLoopEnd> {
        if self.is_done() {
            return None;
        }
        self.stage = Stage::Done;
        Some(end)
    }

    fn on_arrival(&mut self, button_down: bool) -> Arrival {
        if self.stage != Stage::Posted {
            return Arrival::NotOurs;
        }
        let swallow = if self.end_requested {
            Some(MoveLoopEnd::Ended)
        } else if self.esc_seen {
            Some(MoveLoopEnd::Canceled { esc: true })
        } else if !button_down {
            Some(MoveLoopEnd::Released { before_entry: true })
        } else {
            None
        };
        match swallow.and_then(|end| self.finish(end)) {
            Some(end) => Arrival::Swallow(end),
            None => {
                self.stage = Stage::Started;
                Arrival::Pass
            }
        }
    }

    /// `WM_ENTERSIZEMOVE` — `Some(취소를 올리나)` = 이 루프가 들어갔다(`Entered` 를 낸다) · `None` = 이 루프의 것이
    /// 아니다.
    fn on_enter(&mut self, button_down: bool) -> Option<bool> {
        if self.stage != Stage::Started {
            return None;
        }
        self.stage = Stage::Entering;
        self.released_at_entry = !button_down;
        let post = self.end_requested || self.released_at_entry;
        self.cancel_posted = post;
        Some(post)
    }

    /// `WM_ENTERSIZEMOVE` 처리를 마쳤다.
    fn entry_done(&mut self) {
        if self.stage == Stage::Entering {
            self.stage = Stage::Running;
        }
    }

    fn on_end_request(&mut self) -> Cancel {
        if self.is_done() {
            return Cancel::None;
        }
        self.end_requested = true;
        match self.stage {
            Stage::Posted | Stage::Started | Stage::Done => Cancel::None,
            Stage::Entering if self.cancel_posted => Cancel::None,
            Stage::Entering => {
                self.cancel_posted = true;
                Cancel::Post
            }
            Stage::Running => Cancel::Send,
        }
    }

    /// Esc 훅 — `Some(숨기나)` = 아직 끝 전이라 DWM 전환을 끈다 · `None` = 끝난 뒤.
    fn on_escape(&mut self) -> Option<bool> {
        if self.is_done() {
            return None;
        }
        self.esc_seen = true;
        if self.hide_on_escape {
            self.hid = true;
        }
        Some(self.hide_on_escape)
    }

    fn on_mouse_up(&mut self) {
        if !self.is_done() {
            self.mouse_up = true;
        }
    }

    /// 넘긴 `WM_SYSCOMMAND` 가 돌아왔다.
    fn on_returned(&mut self) -> Option<MoveLoopEnd> {
        if !matches!(
            self.stage,
            Stage::Started | Stage::Entering | Stage::Running
        ) {
            return None;
        }
        let end = if self.end_requested {
            MoveLoopEnd::Ended
        } else if self.released_at_entry {
            MoveLoopEnd::Released { before_entry: true }
        } else if self.mouse_up || self.hook_failed {
            MoveLoopEnd::Released {
                before_entry: false,
            }
        } else {
            MoveLoopEnd::Canceled { esc: self.esc_seen }
        };
        self.finish(end)
    }

    fn on_destroyed(&mut self) -> Option<MoveLoopEnd> {
        self.finish(MoveLoopEnd::Destroyed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: bool = false;
    const DOWN: bool = true;

    fn running(hide: bool) -> Core {
        let mut c = Core::new(hide);
        assert_eq!(c.on_arrival(DOWN), Arrival::Pass);
        assert_eq!(c.on_enter(DOWN), Some(false));
        c.entry_done();
        c
    }

    // ── Posted — 도착할 때 삼킴(차례 = 끝내라는 청 · Esc · 뗌) ──

    #[test]
    fn posted_end_request_swallows_as_ended() {
        let mut c = Core::new(false);
        assert_eq!(c.on_end_request(), Cancel::None);
        assert_eq!(c.on_arrival(DOWN), Arrival::Swallow(MoveLoopEnd::Ended));
        assert!(!c.hid());
    }

    #[test]
    fn posted_escape_swallows_as_canceled_and_hides_only_with_the_flag() {
        for hide in [true, false] {
            let mut c = Core::new(hide);
            assert_eq!(c.on_escape(), Some(hide));
            assert_eq!(
                c.on_arrival(DOWN),
                Arrival::Swallow(MoveLoopEnd::Canceled { esc: true })
            );
            assert_eq!(c.hid(), hide);
        }
    }

    #[test]
    fn posted_release_before_arrival_swallows_as_released_before_entry() {
        let mut c = Core::new(false);
        assert_eq!(
            c.on_arrival(UP),
            Arrival::Swallow(MoveLoopEnd::Released { before_entry: true })
        );
    }

    #[test]
    fn posted_end_request_wins_over_escape_but_keeps_hid() {
        // F2 — 끝냄 청과 Esc 가 겹치면 끝은 Ended 지만 훅은 이미 숨겼다.
        let mut c = Core::new(true);
        c.on_end_request();
        assert_eq!(c.on_escape(), Some(true));
        assert_eq!(c.on_arrival(UP), Arrival::Swallow(MoveLoopEnd::Ended));
        assert!(c.hid());
    }

    #[test]
    fn posted_escape_wins_over_release() {
        let mut c = Core::new(false);
        c.on_escape();
        assert_eq!(
            c.on_arrival(UP),
            Arrival::Swallow(MoveLoopEnd::Canceled { esc: true })
        );
    }

    // ── 들어감 ──

    #[test]
    fn entry_with_button_up_posts_cancel_and_ends_released_before_entry() {
        let mut c = Core::new(false);
        assert_eq!(c.on_arrival(DOWN), Arrival::Pass);
        assert_eq!(c.on_enter(UP), Some(true));
        c.entry_done();
        assert_eq!(
            c.on_returned(),
            Some(MoveLoopEnd::Released { before_entry: true })
        );
    }

    #[test]
    fn end_request_before_entry_is_posted_at_entry() {
        let mut c = Core::new(false);
        c.on_arrival(DOWN);
        assert_eq!(c.on_end_request(), Cancel::None, "Started — 적어 두기만");
        assert_eq!(c.on_enter(DOWN), Some(true));
        assert_eq!(c.on_returned(), Some(MoveLoopEnd::Ended));
    }

    #[test]
    fn end_request_inside_entry_posts_once() {
        let mut c = Core::new(false);
        c.on_arrival(DOWN);
        assert_eq!(c.on_enter(DOWN), Some(false));
        assert_eq!(c.on_end_request(), Cancel::Post);
        assert_eq!(c.on_end_request(), Cancel::None, "이미 올렸다");
        c.entry_done();
        assert_eq!(c.on_end_request(), Cancel::Send, "들어간 뒤는 보낸다");
    }

    #[test]
    fn entry_only_counts_once_and_only_after_pass() {
        let mut c = Core::new(false);
        assert_eq!(
            c.on_enter(DOWN),
            None,
            "Posted 의 들어감은 이 루프의 것이 아니다"
        );
        c.on_arrival(DOWN);
        assert_eq!(c.on_enter(DOWN), Some(false));
        assert_eq!(c.on_enter(DOWN), None);
    }

    // ── Running 과 끝 ──

    #[test]
    fn running_end_request_sends_cancel_and_ends_as_ended() {
        let mut c = running(false);
        c.on_mouse_up();
        assert_eq!(c.on_end_request(), Cancel::Send);
        assert_eq!(
            c.on_returned(),
            Some(MoveLoopEnd::Ended),
            "끝내라는 청이 뗌보다 앞선다"
        );
    }

    #[test]
    fn mouse_up_is_release() {
        let mut c = running(false);
        c.on_mouse_up();
        assert_eq!(
            c.on_returned(),
            Some(MoveLoopEnd::Released {
                before_entry: false
            })
        );
    }

    #[test]
    fn escape_then_release_keeps_hid_on_released() {
        // F2 — Esc 뗌 · 반복만 본 뒤 놓으면 Released{false} 인데 숨어 있다.
        let mut c = running(true);
        assert_eq!(c.on_escape(), Some(true));
        c.on_mouse_up();
        assert_eq!(
            c.on_returned(),
            Some(MoveLoopEnd::Released {
                before_entry: false
            })
        );
        assert!(c.hid());
    }

    #[test]
    fn end_without_release_is_cancel_with_escape_flag() {
        let mut c = running(false);
        assert_eq!(c.on_escape(), Some(false));
        assert_eq!(c.on_returned(), Some(MoveLoopEnd::Canceled { esc: true }));
        assert!(
            !c.hid(),
            "hide_on_escape 거짓이면 Esc 를 봐도 숨기지 않는다"
        );

        let mut c = running(false);
        assert_eq!(c.on_returned(), Some(MoveLoopEnd::Canceled { esc: false }));
    }

    #[test]
    fn hook_failure_counts_as_release() {
        let mut c = running(false);
        c.hook_failed = true;
        assert_eq!(
            c.on_returned(),
            Some(MoveLoopEnd::Released {
                before_entry: false
            })
        );
    }

    // ── 소멸 · 정확히 한 번 ──

    #[test]
    fn destroy_ends_in_any_stage_and_carries_hid() {
        let mut posted = Core::new(true);
        posted.on_escape();
        assert_eq!(posted.on_destroyed(), Some(MoveLoopEnd::Destroyed));
        assert!(posted.hid());

        let mut started = Core::new(false);
        started.on_arrival(DOWN);
        assert_eq!(started.on_destroyed(), Some(MoveLoopEnd::Destroyed));

        let mut run = running(false);
        assert_eq!(run.on_destroyed(), Some(MoveLoopEnd::Destroyed));
        assert_eq!(
            run.on_returned(),
            None,
            "펼쳐지며 돌아오는 틀은 또 끝내지 않는다"
        );
    }

    #[test]
    fn nothing_ends_twice() {
        let mut c = Core::new(true);
        c.on_end_request();
        assert_eq!(c.on_arrival(DOWN), Arrival::Swallow(MoveLoopEnd::Ended));
        assert_eq!(c.on_arrival(DOWN), Arrival::NotOurs);
        assert_eq!(c.on_returned(), None);
        assert_eq!(c.on_destroyed(), None);
        assert_eq!(c.on_end_request(), Cancel::None);
        assert_eq!(c.on_escape(), None, "끝난 뒤의 Esc 는 숨기지 않는다");
        assert!(!c.hid());
        c.on_mouse_up();
        assert!(!c.mouse_up);
    }
}
