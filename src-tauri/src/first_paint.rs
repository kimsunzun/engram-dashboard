//! 창을 첫 화면까지 숨긴다 — main · 팝아웃은 숨긴 채 짓고([`hold_window`]), 페이지가 실제 첫 화면(탭바 + 활성 탭)을
//! 그렸다고 알리면(`webview_painted` · `src/theme/firstPaint.ts`) 창을 보인다. 사용자 결정(2026-10-19): 창은 내용과
//! 함께 뜬다 — 틀을 먼저 띄우지 않는다(네이티브 틀은 걷힐 예정이다).
//!
//! - ★웹뷰는 숨기지 않는다 — 숨긴 창 안에서 그리게 둔다★: 숨긴 웹뷰(`Webview::hide`)를 다시 보이는 순간 WebView2 가 아직
//!   그리지 않은 면을 한 번 보였다(사용자 관측 2026-10-19 「창이 뜨고 좀 있다가 깜박임」).
//! - ★창을 보이는 것은 올린 창 하나에 한 번뿐이다★ — 알림과 [`FALLBACK_REVEAL`] 중 먼저 온 쪽. 올리지 않은 창(`--hidden`
//!   부팅의 main · 예비 창 · 다시 읽기)의 알림은 아무 일도 하지 않는다.
//! - ★보일지는 메인 스레드에서, 보이기 바로 앞에 정한다★ — 알림 · 시한 · 복원은 올린 차례만 실어 메인 스레드로 넘기고,
//!   넘긴 일이 돌 때 그 차례의 올림이 남아 있으면 빼내고 보인다([`FirstPaintGate::release`]). 넘기기 전에 빼내면, 넘긴
//!   뒤 · 돌기 전에 메인 스레드의 숨기기(`--hidden` 부팅 setup 끝 · 트레이 숨기기)가 거둘 것을 못 찾고 넘긴 일이 숨긴 창을
//!   보인다. 보이는 법은 [`crate::state::placement::show_placed`] 다(미룬 최대화가 있으면 보통 크기 틀 없이 최대화로
//!   보인다).
//! - 런타임 복원 수락의 팝아웃은 보일지가 모델에 넣은 뒤에 정해져 둘로 나눠 올린다 — 지을 때는 알림만 받아 두고
//!   ([`hold_window_unarmed`]) 보이기로 정하면 보인다([`reveal_held`] — 알림이 이미 왔으면 곧바로, 아니면 알림 · 시한에).
//!   시한은 보이기로 정한 때부터 잰다 — 정하기 전에 보이면 되돌릴 수도 있는 창이 드러난다. 보이는 법은 지을 때 올린
//!   창과 같다 — 활성화한다.
//! - ★숨기기는 올림을 거둔다([`cancel_hold`])★ — 트레이 숨기기 경로(`tray::actions::hide_main_ui` — ADR-0229)와 복원의
//!   숨긴 채 두기가 부른다. 거두지 않으면 숨긴 뒤 오는 알림 · 시한이 창을 다시 보인다: `--hidden` 부팅이면 main 은
//!   트레이에 둔 채 팝아웃만 뜨고(전경까지 가져가고), 사용량 관심은 그 창을 숨은 것으로 둔다. 트레이 보이기도 거둔다 —
//!   그 보이기가 창을 보이고 창 사이 차례(main 이 마지막 · 맨 앞)를 정한다. 창이 사라질 때도 거둔다(`Destroyed`) — 보이기로
//!   정하지 않은 창은 시한이 없어 그 밖에 거둘 곳이 없다.
//! - 숨긴 채 짓는 까닭이 하나 더 있다 — tao 0.35.3 은 보이는 창의 보이기를 창 만들기(`init`) 끝에서 하는데 창 바탕을
//!   칠하는 서브클래스(`WM_ERASEBKGND`)는 그 뒤에 달아, 첫 프레임 안쪽이 칠해지지 않은 채 창 뒤가 비쳤다(사용자 영상
//!   2026-10-19 · `platform_impl/windows/window.rs` — 소스 독해). 지은 뒤의 보이기는 바탕색으로 지운다.
//! - 숨은 동안 페이지가 실제로 그리는지(WebView2 가 부모 창 숨김을 가림으로 보고 그리기를 멈추는지)는 [미검 — GUI]. 멈추면
//!   보인 직후 한순간 바탕색이다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_base::sync;
use tauri::{AppHandle, Manager, WebviewWindow};

/// 첫 화면 알림을 기다리는 상한 — 넘으면 알림 없이 보인다. 덮는 것 = 페이지 읽기 · 번들 실행 · 첫 마운트 · 탭 목록과 활성
/// 탭 레이아웃 조회. 값은 실측이 아니라 「넘으면 깨진 것으로 본다」는 판단이다(개발 서버로 띄운 창은 0.8~1.1 s — 사용자
/// 관측 2026-10-19). 넘으면 첫 화면 전에 보인다.
pub(crate) const FALLBACK_REVEAL: Duration = Duration::from_millis(1500);

/// 보이기를 기다리는 숨긴 창의 명부 — label → 올린 상태. 보이기는 메인 스레드에서 그 차례의 항목을 빼낸 쪽 하나만 한다
/// (모듈 머리). 셸에 하나다(빌더에서 manage — 창을 만드는 어느 자리보다 먼저 있어야 한다).
#[derive(Default)]
pub(crate) struct FirstPaintGate {
    held: Mutex<HashMap<String, Hold>>,
    next: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// 알림 또는 시한에 보인다. `ticket` = 올린 차례 — 올릴 때마다 새로 뽑아(명부 전체에서 단조) 넘긴 보이기는 자기
    /// 차례일 때만 빼낸다. 거둔 뒤 다시 올린 창을 옛 일이 보이지 않게 한다.
    Armed { ticket: u64 },
    /// 알림만 받아 둔다 — 보일지는 [`FirstPaintGate::arm`] 이 정한다. `painted` = 알림이 이미 왔다.
    Unarmed { painted: bool },
}

/// [`FirstPaintGate::arm`] 의 답 — 둘 다 그 창을 이 차례로 올렸다.
#[derive(Debug, PartialEq, Eq)]
enum Arm {
    /// 알림이 이미 왔다 — 지금 보이기를 넘긴다.
    Painted(u64),
    /// 알림 또는 시한을 기다린다.
    Waiting(u64),
}

impl FirstPaintGate {
    fn ticket(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn hold(&self, label: &str) -> u64 {
        let ticket = self.ticket();
        sync::lock(&self.held).insert(label.to_owned(), Hold::Armed { ticket });
        ticket
    }

    fn hold_unarmed(&self, label: &str) {
        sync::lock(&self.held).insert(label.to_owned(), Hold::Unarmed { painted: false });
    }

    // 명부에 없는 창(거둔 창 포함)도 지금부터 기다린다 — 그 전에 온 알림은 남아 있지 않아 시한에 보인다.
    fn arm(&self, label: &str) -> Arm {
        let ticket = self.ticket();
        let mut held = sync::lock(&self.held);
        let painted = held.get(label) == Some(&Hold::Unarmed { painted: true });
        held.insert(label.to_owned(), Hold::Armed { ticket });
        if painted {
            Arm::Painted(ticket)
        } else {
            Arm::Waiting(ticket)
        }
    }

    // 첫 화면 알림 — 올린 창이면 넘길 차례, 받아 두기만 하는 창이면 알림을 적고 `None`.
    fn painted(&self, label: &str) -> Option<u64> {
        let mut held = sync::lock(&self.held);
        match held.get(label).copied()? {
            Hold::Armed { ticket } => Some(ticket),
            Hold::Unarmed { .. } => {
                held.insert(label.to_owned(), Hold::Unarmed { painted: true });
                None
            }
        }
    }

    // 넘기기 전에 본다 — 빼낸 차례는 다시 오지 않아(차례는 단조) 「없다」는 넘긴 뒤에도 그대로다. 「있다」는 넘긴 일이
    //   다시 정한다([`Self::release`]).
    fn is_waiting(&self, label: &str, ticket: u64) -> bool {
        matches!(
            sync::lock(&self.held).get(label),
            Some(Hold::Armed { ticket: t }) if *t == ticket
        )
    }

    /// 넘긴 보이기가 메인 스레드에서 돌 때 정한다 — 그 창이 이 차례로 올린 채 남아 있으면 빼내고 `true`(보인다).
    /// `false` = 이미 보였거나 거뒀거나 다시 올렸다.
    fn release(&self, label: &str, ticket: u64) -> bool {
        let mut held = sync::lock(&self.held);
        let ours = held.get(label) == Some(&Hold::Armed { ticket });
        if ours {
            held.remove(label);
        }
        ours
    }

    fn cancel(&self, label: &str) -> bool {
        sync::lock(&self.held).remove(label).is_some()
    }
}

fn gate(app: &AppHandle) -> Option<Arc<FirstPaintGate>> {
    app.try_state::<Arc<FirstPaintGate>>()
        .map(|gate| gate.inner().clone())
}

/// 숨긴 채 지은 창을 첫 화면 알림(또는 [`FALLBACK_REVEAL`])에 보이도록 올린다 — 보일 때 활성화한다. ★창은 숨은 채여야
/// 하고 자리 · 미룬 최대화를 이미 입혔어야 한다★ — 보이기가 그 자리 그대로 보인다. 명부가 등록돼 있지 않으면 곧바로
/// 보인다(보일 길이 없는 숨은 창을 만들지 않는다).
pub(crate) fn hold_window(window: &WebviewWindow) {
    let Some(gate) = gate(window.app_handle()) else {
        tracing::warn!(
            module = "first_paint",
            label = window.label(),
            "첫 화면 명부가 등록돼 있지 않아 창을 곧바로 보인다"
        );
        show(window);
        return;
    };
    let ticket = gate.hold(window.label());
    reveal_when_overdue_on_main_thread(window.app_handle(), gate, window.label(), ticket);
}

/// 숨긴 채 지은 창의 첫 화면 알림을 받아 두기만 한다 — 보일지는 뒤에 [`reveal_held`](보인다) · [`cancel_hold`](숨긴 채
/// 둔다)가 정한다. 정할 때까지 알림도 시한도 창을 보이지 않는다. 명부가 등록돼 있지 않으면 아무것도 하지 않는다(그때
/// [`reveal_held`] 가 곧바로 보인다).
pub(crate) fn hold_window_unarmed(window: &WebviewWindow) {
    if let Some(gate) = gate(window.app_handle()) {
        gate.hold_unarmed(window.label());
    }
}

/// 그 창을 첫 화면에 보이기로 정한다 — 알림이 이미 왔으면 곧바로, 아니면 알림 또는 지금부터 [`FALLBACK_REVEAL`] 에
/// 보인다 — 보일 때 활성화한다. 받아 두지 않은 창(거둔 창 포함)도 지금부터 기다린다. 어느 스레드에서 불러도 된다(보이기는
/// 메인 스레드로 넘긴다).
pub(crate) fn reveal_held(app: &AppHandle, label: &str) {
    let Some(gate) = gate(app) else {
        tracing::warn!(
            module = "first_paint",
            label,
            "첫 화면 명부가 등록돼 있지 않아 창을 곧바로 보인다"
        );
        reveal_on_main_thread(app, label, |_| true);
        return;
    };
    match gate.arm(label) {
        Arm::Painted(ticket) => {
            release_on_main_thread(app, gate, label, ticket, Cause::PaintedBeforeArming)
        }
        Arm::Waiting(ticket) => reveal_when_overdue_on_main_thread(app, gate, label, ticket),
    }
}

/// 그 창의 올림을 거둔다 — 뒤에 오는 첫 화면 알림 · 시한과 이미 넘긴 보이기가 그 창을 보이지 않는다. 올린 적 없는 창이면
/// 아무 일도 하지 않는다. 숨기기 · 트레이 보이기 경로가 그 창마다 부른다(모듈 머리).
pub(crate) fn cancel_hold(app: &AppHandle, label: &str) {
    if gate(app).is_some_and(|gate| gate.cancel(label)) {
        tracing::debug!(
            module = "first_paint",
            label,
            "첫 화면을 기다리던 창의 올림을 거둔다 — 알림 · 시한이 그 창을 보이지 않는다"
        );
    }
}

/// 그 창의 첫 화면 알림 — 올린 창이면 보이고(받아 두기만 하는 창이면 받아 둔다), 아니면 아무 일도 하지 않는다.
pub(crate) fn reveal_painted(window: &WebviewWindow) {
    let app = window.app_handle();
    let Some(gate) = gate(app) else {
        return;
    };
    if let Some(ticket) = gate.painted(window.label()) {
        release_on_main_thread(app, gate, window.label(), ticket, Cause::Painted);
    }
}

/// 넘긴 보이기를 무엇이 정했나 — 로그 몫.
#[derive(Debug, Clone, Copy)]
enum Cause {
    Painted,
    PaintedBeforeArming,
    Overdue,
}

impl Cause {
    fn log(self, label: &str) {
        match self {
            Self::Painted => {
                tracing::debug!(module = "first_paint", label, "첫 화면 알림에 창을 보인다")
            }
            Self::PaintedBeforeArming => tracing::debug!(
                module = "first_paint",
                label,
                "첫 화면 알림이 먼저 와 있어 창을 곧바로 보인다"
            ),
            Self::Overdue => tracing::warn!(
                module = "first_paint",
                label,
                "첫 화면 알림이 시한 안에 오지 않아 창을 보인다"
            ),
        }
    }
}

fn reveal_when_overdue_on_main_thread(
    app: &AppHandle,
    gate: Arc<FirstPaintGate>,
    label: &str,
    ticket: u64,
) {
    let app = app.clone();
    let label = label.to_owned();
    tauri::async_runtime::spawn(async move {
        let reveal = {
            let gate = gate.clone();
            let label = label.clone();
            move || release_on_main_thread(&app, gate, &label, ticket, Cause::Overdue)
        };
        queue_when_overdue(&gate, &label, ticket, FALLBACK_REVEAL, reveal).await;
    });
}

async fn queue_when_overdue(
    gate: &FirstPaintGate,
    label: &str,
    ticket: u64,
    after: Duration,
    queue: impl FnOnce(),
) {
    tokio::time::sleep(after).await;
    if gate.is_waiting(label, ticket) {
        queue();
    }
}

fn release_on_main_thread(
    app: &AppHandle,
    gate: Arc<FirstPaintGate>,
    label: &str,
    ticket: u64,
    cause: Cause,
) {
    reveal_on_main_thread(app, label, move |label| {
        let reveal = gate.release(label, ticket);
        if reveal {
            cause.log(label);
        } else {
            tracing::debug!(
                module = "first_paint",
                label,
                ?cause,
                "넘긴 보이기가 돌기 전에 그 올림이 보였거나 거둬져 보이지 않는다"
            );
        }
        reveal
    });
}

// `decide` 는 메인 스레드에서 보이기 바로 앞에 돈다(모듈 머리).
fn reveal_on_main_thread(
    app: &AppHandle,
    label: &str,
    decide: impl FnOnce(&str) -> bool + Send + 'static,
) {
    let handle = app.clone();
    let owned = label.to_owned();
    let queued = app.run_on_main_thread(move || {
        if !decide(&owned) {
            return;
        }
        match handle.get_webview_window(&owned) {
            Some(window) => show(&window),
            None => {
                tracing::debug!(module = "first_paint", label = %owned, "보이기 전에 창이 닫혔다")
            }
        }
    });
    if let Err(e) = queued {
        tracing::warn!(
            module = "first_paint",
            label,
            "숨긴 창을 보이는 일을 메인 스레드에 넘기지 못했다: {e}"
        );
    }
}

fn show(window: &WebviewWindow) {
    crate::state::placement::show_placed(&window.as_ref().window());
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 명부 ──

    #[test]
    fn the_painted_signal_reveals_a_held_window_once() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        assert_eq!(gate.painted("main"), Some(ticket));
        assert!(gate.release("main", ticket));
        assert_eq!(gate.painted("main"), None, "두 번째 알림은 보이지 않는다");
    }

    #[test]
    fn a_signal_from_a_window_that_was_not_held_does_nothing() {
        let gate = FirstPaintGate::default();
        assert_eq!(gate.painted("slot-popup-1"), None);
    }

    #[test]
    fn the_signal_and_the_deadline_reveal_the_window_once_between_them() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        let signal = gate.painted("main").unwrap();
        assert!(gate.release("main", signal));
        assert!(!gate.is_waiting("main", ticket), "시한은 넘기지 않는다");
        assert!(!gate.release("main", ticket));
    }

    #[test]
    fn the_painted_signal_does_nothing_after_the_deadline() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        assert!(gate.release("main", ticket));
        assert_eq!(gate.painted("main"), None);
    }

    #[test]
    fn an_old_ticket_does_not_reveal_a_window_held_again_under_the_same_label() {
        let gate = FirstPaintGate::default();
        let old = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", old));
        let new = gate.hold("slot-popup-1");
        assert!(!gate.release("slot-popup-1", old));
        assert!(gate.release("slot-popup-1", new));
    }

    // ── 넘긴 보이기 — 정하기는 넘긴 일이 돌 때다 ──

    #[test]
    fn a_window_hidden_after_the_deadline_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("slot-popup-1");
        assert!(gate.is_waiting("slot-popup-1", ticket), "시한이 넘긴다");
        assert!(gate.cancel("slot-popup-1"));
        assert!(!gate.release("slot-popup-1", ticket));
    }

    #[test]
    fn a_window_hidden_after_the_signal_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold("slot-popup-1");
        let queued = gate.painted("slot-popup-1").unwrap();
        assert!(gate.cancel("slot-popup-1"));
        assert!(!gate.release("slot-popup-1", queued));
    }

    #[test]
    fn a_window_hidden_after_arming_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1"), None);
        let Arm::Painted(queued) = gate.arm("slot-popup-1") else {
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert!(gate.cancel("slot-popup-1"));
        assert!(!gate.release("slot-popup-1", queued));
    }

    #[test]
    fn a_window_held_again_after_hiding_is_not_revealed_by_a_reveal_queued_before() {
        let gate = FirstPaintGate::default();
        let first = gate.hold("slot-popup-1");
        let queued = gate.painted("slot-popup-1").unwrap();
        assert!(gate.cancel("slot-popup-1"));
        let again = gate.hold("slot-popup-1");
        assert!(!gate.release("slot-popup-1", queued));
        assert!(!gate.release("slot-popup-1", first));
        assert!(gate.release("slot-popup-1", again));
    }

    #[test]
    fn rearming_after_hiding_ignores_the_reveal_queued_before() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1"), None);
        let Arm::Painted(queued) = gate.arm("slot-popup-1") else {
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert!(gate.cancel("slot-popup-1"));
        let Arm::Waiting(again) = gate.arm("slot-popup-1") else {
            panic!("거둔 뒤에는 그 전 알림이 남아 있지 않다");
        };
        assert!(!gate.release("slot-popup-1", queued));
        assert!(gate.release("slot-popup-1", again));
    }

    // ── 거두기(숨기기 · 트레이 보이기) ──

    #[test]
    fn cancelling_a_window_that_was_not_held_does_nothing() {
        let gate = FirstPaintGate::default();
        assert!(!gate.cancel("main"));
    }

    // ── 받아 두기(런타임 복원) ──

    #[test]
    fn an_unarmed_window_is_not_revealed_by_the_signal_and_arming_after_it_reveals_at_once() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1"), None);
        let Arm::Painted(ticket) = gate.arm("slot-popup-1") else {
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert!(gate.release("slot-popup-1", ticket));
        assert_eq!(
            gate.painted("slot-popup-1"),
            None,
            "보인 뒤의 알림은 다시 보이지 않는다"
        );
    }

    #[test]
    fn arming_before_the_signal_waits_for_the_signal() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Arm::Waiting(ticket) = gate.arm("slot-popup-1") else {
            panic!("알림 전에 정했으면 기다린다");
        };
        assert_eq!(gate.painted("slot-popup-1"), Some(ticket));
        assert!(gate.release("slot-popup-1", ticket));
        assert!(!gate.is_waiting("slot-popup-1", ticket));
    }

    #[test]
    fn an_unarmed_window_kept_hidden_is_never_revealed() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1"), None);
        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1"), None);
    }

    #[test]
    fn arming_a_window_that_was_not_held_waits_from_now() {
        let gate = FirstPaintGate::default();
        let Arm::Waiting(ticket) = gate.arm("slot-popup-1") else {
            panic!("받아 둔 알림이 없으면 기다린다");
        };
        assert_eq!(gate.painted("slot-popup-1"), Some(ticket));
        assert!(gate.release("slot-popup-1", ticket));
    }

    // ── 시한 ──

    fn counter() -> (Arc<Mutex<u32>>, impl FnOnce()) {
        let queued = Arc::new(Mutex::new(0));
        let sink = queued.clone();
        (queued, move || *sink.lock().unwrap() += 1)
    }

    #[tokio::test(start_paused = true)]
    async fn without_a_signal_the_deadline_queues_the_reveal_after_the_fallback() {
        let gate = Arc::new(FirstPaintGate::default());
        let ticket = gate.hold("main");
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "main", ticket, FALLBACK_REVEAL, queue).await
        });

        tokio::time::sleep(FALLBACK_REVEAL - Duration::from_millis(1)).await;
        assert_eq!(*queued.lock().unwrap(), 0, "시한 전에는 넘기지 않는다");

        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 1);
        assert!(gate.release("main", ticket));
    }

    #[tokio::test(start_paused = true)]
    async fn a_reveal_before_the_deadline_leaves_the_deadline_nothing_to_queue() {
        let gate = Arc::new(FirstPaintGate::default());
        let ticket = gate.hold("main");
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "main", ticket, FALLBACK_REVEAL, queue).await
        });

        let signal = gate.painted("main").unwrap();
        assert!(gate.release("main", signal));
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_window_hidden_before_the_deadline_leaves_the_deadline_nothing_to_queue() {
        let gate = Arc::new(FirstPaintGate::default());
        let ticket = gate.hold("slot-popup-1");
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "slot-popup-1", ticket, FALLBACK_REVEAL, queue).await
        });

        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1"), None);
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 0);
    }
}
