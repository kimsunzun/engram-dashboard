//! 창을 첫 화면까지 숨긴다 — main · 팝아웃은 숨긴 채 짓고([`hold_window`]), 페이지가 실제 첫 화면(탭바 + 활성 탭)을
//! 그렸다고 알리면(`webview_painted` · `src/theme/firstPaint.ts`) 창을 보인다. 사용자 결정(2026-10-19): 창은 내용과
//! 함께 뜬다 — 틀을 먼저 띄우지 않는다(네이티브 틀은 걷힐 예정이다).
//!
//! - ★웹뷰는 숨기지 않는다 — 숨긴 창 안에서 그리게 둔다★: 숨긴 웹뷰(`Webview::hide`)를 다시 보이는 순간 WebView2 가 아직
//!   그리지 않은 면을 한 번 보였다(사용자 관측 2026-10-19 「창이 뜨고 좀 있다가 깜박임」).
//! - ★창을 보이는 것은 올린 창 하나에 한 번뿐이다★ — 알림과 [`FALLBACK_REVEAL`] 중 먼저 온 쪽. 올림이 없는 창(`--hidden`
//!   부팅의 main · 맡기기 전 예열 창 · 이미 보인 창의 다시 읽기)의 알림은 아무 일도 하지 않는다.
//! - ★보일지는 메인 스레드에서, 보이기 바로 앞에 정한다★ — 알림 · 시한 · 복원은 올린 차례만 실어 메인 스레드로 넘기고,
//!   넘긴 일이 돌 때 그 차례의 올림이 남아 있으면 빼내고 보인다([`FirstPaintGate::release`]). 넘기기 전에 빼내면, 넘긴
//!   뒤 · 돌기 전에 메인 스레드의 숨기기(`--hidden` 부팅 setup 끝 · 트레이 숨기기)가 거둘 것을 못 찾고 넘긴 일이 숨긴 창을
//!   보인다. 보이는 법은 셋이다 — 지어서 올린 창 · 복원 창은 [`crate::state::placement::show_placed`](보일 때 활성화 ·
//!   미룬 최대화가 있으면 보통 크기 틀 없이 최대화로 보인다), 창 포트가 포커스 없이 새로 지은 창(I30)과 맡긴 예열 창은
//!   숨긴 채 z 만 맨 위로 올린 뒤 보인다(맡긴 창만 아래 조건에서 활성화한다 — [`new_window_show`]).
//! - 런타임 복원 수락의 팝아웃은 보일지가 모델에 넣은 뒤에 정해져 둘로 나눠 올린다 — 지을 때는 알림만 받아 두고
//!   ([`hold_window_unarmed`]) 보이기로 정하면 보인다([`reveal_held`] — 알림이 이미 왔으면 곧바로, 아니면 알림 · 시한에).
//!   시한은 보이기로 정한 때부터 잰다 — 정하기 전에 보이면 되돌릴 수도 있는 창이 드러난다. 보이는 법은 지을 때 올린
//!   창과 같다 — 활성화한다.
//! - 맡긴 예열 창(TRD S21-slot-docking §3-9)도 둘로 나눠 올린다 — 맡길 때 예열 창 풀이 알림만 받아 두는 올림을 걸고
//!   ([`SparePaintGate`]) 확정 ③ 이 그 올림이 남아 있을 때만 보이기로 정한다([`reveal_if_held`] — 시한은 여기부터).
//!   ★그 항목은 그 맡김의 채택 번호를 실은 알림만 받는다★(I26) — 번호가 없거나 다른 알림은 채택 전 틀(대기 페이지 ·
//!   `bootFailed`)이 보낸 것이라, 따르면 그 틀이 곧바로 보인다. ③ 전에 받아 둔 알림도 같은 번호일 때만 ③ 이 곧바로
//!   보인다. 번호는 이 길만 본다 — 지어서 올린 창과 복원 길은 번호가 있든 없든 알림을 받는다. ★맡긴 창이 보이면
//!   (알림 · 시한 어느 쪽이든) 예열 창 풀의 다시 짓기 완료를 부른다★(`SpareState::on_shown` — 그 밖의 완료 = 창
//!   `Destroyed` · 안전 시한).
//! - ★명부 락은 잎이다★ — 쥔 채 로그 · 창 호출 · 다른 락을 하지 않는다. 예열 창 풀이 자기 락을 쥔 채 올림을 걸고
//!   거둔다(차례 = 레이아웃 → 풀 → 첫 화면 문 — `crate::spare::SpareState`).
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
// ADR-0299

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// 차례일 때만 빼낸다. 거둔 뒤 다시 올린 창을 옛 일이 보이지 않게 한다. `since` = 올린 때(로그 `held_ms`) · `opened` =
    /// 창 포트가 그 창을 열기 시작한 때(로그 `reveal_ms` — 창 포트 밖에서 올린 창은 `None`).
    Armed {
        ticket: u64,
        reveal: Reveal,
        since: Instant,
        opened: Option<Instant>,
    },
    /// 알림만 받아 둔다 — 보일지는 [`FirstPaintGate::arm`](복원) · [`FirstPaintGate::arm_adopted`](확정 ③)가 정한다.
    Unarmed { painted: Painted },
}

/// 올린 항목이 받는 알림과 보이는 법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reveal {
    /// 지어서 올린 창 · 복원 창 — 번호를 보지 않고 어느 알림이든 받는다. 보일 때 활성화한다.
    Placed,
    /// 창 포트가 포커스 없이 새로 지은 창(I30) — 번호를 보지 않는다. 숨긴 채 z 만 올리고 활성화 없이 보인다.
    Raised,
    /// 맡긴 예열 창(확정 ③) — 채택 번호 `generation` 을 실은 알림만 받는다(I26). `activate` = 그 맡김이 활성으로
    /// 열렸다(I5).
    Adopted { generation: u32, activate: bool },
}

/// 받아 두는 항목에 온 알림 — 복원 길([`FirstPaintGate::arm`])은 `Numberless` 와 `Generation` 을 둘 다 「알림 옴」으로
/// 보고, 확정 ③([`FirstPaintGate::arm_adopted`])은 그 채택 번호의 `Generation` 만 본다. 참 · 거짓 하나로 뭉치면 복원
/// 창이 알림을 잃거나(번호를 요구하면) 채택 전 틀이 곧바로 보인다(번호를 안 보면).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Painted {
    Not,
    Numberless,
    Generation(u32),
}

/// [`FirstPaintGate::arm`] · [`FirstPaintGate::arm_adopted`] 의 답 — 둘 다 그 창을 이 차례로 올렸다.
#[derive(Debug, PartialEq, Eq)]
enum Arm {
    /// 알림이 이미 왔다 — 지금 보이기를 넘긴다.
    Painted(u64),
    /// 알림 또는 시한을 기다린다.
    Waiting(u64),
}

/// [`FirstPaintGate::painted`] 의 답.
#[derive(Debug, PartialEq, Eq)]
enum Signal {
    /// 올린 항목이 받았다 — 이 차례로 보이기를 넘긴다.
    Release(u64),
    /// 받아 두는 항목에 적었다.
    Recorded,
    /// 맡긴 예열 창 항목이 번호가 다른(없는) 알림을 버렸다(I26).
    Mismatch { expected: u32, got: Option<u32> },
    /// 그 label 의 항목이 없다.
    Ignored,
}

/// [`FirstPaintGate::release`] 가 빼낸 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Released {
    reveal: Reveal,
    since: Instant,
    opened: Option<Instant>,
}

impl FirstPaintGate {
    fn ticket(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn hold(&self, label: &str) -> u64 {
        self.hold_as(label, Reveal::Placed, None)
    }

    fn hold_as(&self, label: &str, reveal: Reveal, opened: Option<Instant>) -> u64 {
        let ticket = self.ticket();
        let since = Instant::now();
        sync::lock(&self.held).insert(
            label.to_owned(),
            Hold::Armed {
                ticket,
                reveal,
                since,
                opened,
            },
        );
        ticket
    }

    fn hold_unarmed(&self, label: &str) {
        sync::lock(&self.held).insert(
            label.to_owned(),
            Hold::Unarmed {
                painted: Painted::Not,
            },
        );
    }

    // 명부에 없는 창(거둔 창 포함)도 지금부터 기다린다 — 그 전에 온 알림은 남아 있지 않아 시한에 보인다.
    fn arm(&self, label: &str) -> Arm {
        let ticket = self.ticket();
        let since = Instant::now();
        let mut held = sync::lock(&self.held);
        let painted = matches!(
            held.get(label),
            Some(Hold::Unarmed { painted }) if *painted != Painted::Not
        );
        held.insert(
            label.to_owned(),
            Hold::Armed {
                ticket,
                reveal: Reveal::Placed,
                since,
                opened: None,
            },
        );
        if painted {
            Arm::Painted(ticket)
        } else {
            Arm::Waiting(ticket)
        }
    }

    // 항목이 없으면(거뒀다) 아무것도 하지 않는다 — `arm` 처럼 지금부터 기다리면 UI 숨기기가 거둔 창을 되살린다.
    fn arm_adopted(
        &self,
        label: &str,
        generation: u32,
        activate: bool,
        opened: Option<Instant>,
    ) -> Option<Arm> {
        let ticket = self.ticket();
        let since = Instant::now();
        let mut held = sync::lock(&self.held);
        let painted = match held.get(label)? {
            Hold::Unarmed {
                painted: Painted::Generation(got),
            } => *got == generation,
            Hold::Unarmed { .. } | Hold::Armed { .. } => false,
        };
        held.insert(
            label.to_owned(),
            Hold::Armed {
                ticket,
                reveal: Reveal::Adopted {
                    generation,
                    activate,
                },
                since,
                opened,
            },
        );
        Some(if painted {
            Arm::Painted(ticket)
        } else {
            Arm::Waiting(ticket)
        })
    }

    fn painted(&self, label: &str, generation: Option<u32>) -> Signal {
        let mut held = sync::lock(&self.held);
        let Some(hold) = held.get_mut(label) else {
            return Signal::Ignored;
        };
        match hold {
            Hold::Armed {
                ticket,
                reveal: Reveal::Placed | Reveal::Raised,
                ..
            } => Signal::Release(*ticket),
            Hold::Armed {
                ticket,
                reveal:
                    Reveal::Adopted {
                        generation: expected,
                        ..
                    },
                ..
            } => {
                if generation == Some(*expected) {
                    Signal::Release(*ticket)
                } else {
                    Signal::Mismatch {
                        expected: *expected,
                        got: generation,
                    }
                }
            }
            Hold::Unarmed { painted } => {
                // 번호 없는 알림은 받아 둔 번호를 지우지 않는다 — 늦게 처리된 채택 전 틀의 알림이 같은 번호로 ③ 전에
                //   닿은 알림을 덮으면 ③ 이 곧바로 보이지 못하고 시한을 기다린다.
                match generation {
                    Some(got) => *painted = Painted::Generation(got),
                    None if *painted == Painted::Not => *painted = Painted::Numberless,
                    None => {}
                }
                Signal::Recorded
            }
        }
    }

    // 넘기기 전에 본다 — 빼낸 차례는 다시 오지 않아(차례는 단조) 「없다」는 넘긴 뒤에도 그대로다. 「있다」는 넘긴 일이
    //   다시 정한다([`Self::release`]).
    fn is_waiting(&self, label: &str, ticket: u64) -> bool {
        matches!(
            sync::lock(&self.held).get(label),
            Some(Hold::Armed { ticket: t, .. }) if *t == ticket
        )
    }

    /// 넘긴 보이기가 메인 스레드에서 돌 때 정한다 — 그 창이 이 차례로 올린 채 남아 있으면 빼내고 돌려준다(보인다).
    /// `None` = 이미 보였거나 거뒀거나 다시 올렸다.
    fn release(&self, label: &str, ticket: u64) -> Option<Released> {
        let mut held = sync::lock(&self.held);
        match held.get(label).copied() {
            Some(Hold::Armed {
                ticket: t,
                reveal,
                since,
                opened,
            }) if t == ticket => {
                held.remove(label);
                Some(Released {
                    reveal,
                    since,
                    opened,
                })
            }
            _ => None,
        }
    }

    fn cancel(&self, label: &str) -> bool {
        sync::lock(&self.held).remove(label).is_some()
    }
}

/// 예열 창 풀이 쥐는 첫 화면 문 손잡이(`crate::spare::PaintGate` — I17 · I26). 빌더에서 풀보다 먼저 만들고, 확정 ③ 의
/// 보이기를 넘길 앱 손잡이는 setup 이 [`Self::attach`] 로 꽂는다 — 그 전에는 창이 없어 맡길 것도 없다.
pub(crate) struct SparePaintGate {
    gate: Arc<FirstPaintGate>,
    app: OnceLock<AppHandle>,
}

impl SparePaintGate {
    pub(crate) fn new(gate: Arc<FirstPaintGate>) -> Self {
        Self {
            gate,
            app: OnceLock::new(),
        }
    }

    /// 앱 손잡이를 꽂는다 — 한 번만 듣는다(둘째부터는 무시).
    pub(crate) fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }
}

// `hold_unarmed` · `cancel` 은 풀 락 안에서 불린다 — 명부의 잎 락만 잡는다(모듈 머리).
impl crate::spare::PaintGate for SparePaintGate {
    fn hold_unarmed(&self, label: &str) {
        self.gate.hold_unarmed(label);
    }

    fn cancel(&self, label: &str) {
        self.gate.cancel(label);
    }

    fn reveal_if_held(
        &self,
        label: &str,
        generation: u32,
        activate: bool,
        opened: Option<Instant>,
    ) -> bool {
        if let Some(app) = self.app.get() {
            return reveal_if_held(app, &self.gate, label, generation, activate, opened);
        }
        // 「거둬졌다」와 같이 다룬다 — 숨긴 채 모델 창이 되고 다음 트레이 보이기가 보인다.
        self.gate.cancel(label);
        tracing::warn!(
            module = "first_paint",
            label,
            generation,
            "앱 손잡이가 꽂히기 전의 확정이라 맡긴 창을 첫 화면에 올리지 않는다"
        );
        false
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
        show_as(window, Reveal::Placed);
        return;
    };
    let ticket = gate.hold(window.label());
    reveal_when_overdue_on_main_thread(window.app_handle(), gate, window.label(), ticket);
}

/// [`hold_window`] 의 창 포트 판 — 창 포트가 새로 지은 팝아웃. `focused` = 포커스를 받으며 지었다(보일 때 활성화 —
/// [`hold_window`] 와 같다) · `false` = 포커스 없이 지었다(숨긴 채 z 만 올리고 활성화 없이 보인다 — I30). `opened` = 창
/// 포트가 그 창을 열기 시작한 때(보임 로그 `reveal_ms`).
// ADR-0299
pub(crate) fn hold_built(window: &WebviewWindow, focused: bool, opened: Instant) {
    let reveal = if focused {
        Reveal::Placed
    } else {
        Reveal::Raised
    };
    let Some(gate) = gate(window.app_handle()) else {
        tracing::warn!(
            module = "first_paint",
            label = window.label(),
            "첫 화면 명부가 등록돼 있지 않아 창을 곧바로 보인다"
        );
        show_as(window, reveal);
        return;
    };
    let ticket = gate.hold_as(window.label(), reveal, Some(opened));
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
        reveal_on_main_thread(app, label, |_| Some(Reveal::Placed));
        return;
    };
    match gate.arm(label) {
        Arm::Painted(ticket) => {
            release_on_main_thread(app, gate, label, ticket, Cause::PaintedBeforeArming)
        }
        Arm::Waiting(ticket) => reveal_when_overdue_on_main_thread(app, gate, label, ticket),
    }
}

// 확정 ③(TRD S21-slot-docking §3-9 「`committed`」) — `true` = 남은 올림을 올렸다. 어느 스레드에서 불러도 된다.
fn reveal_if_held(
    app: &AppHandle,
    gate: &Arc<FirstPaintGate>,
    label: &str,
    generation: u32,
    activate: bool,
    opened: Option<Instant>,
) -> bool {
    match gate.arm_adopted(label, generation, activate, opened) {
        None => false,
        Some(Arm::Painted(ticket)) => {
            release_on_main_thread(app, gate.clone(), label, ticket, Cause::PaintedBeforeArming);
            true
        }
        Some(Arm::Waiting(ticket)) => {
            reveal_when_overdue_on_main_thread(app, gate.clone(), label, ticket);
            true
        }
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

/// 그 창의 첫 화면 알림 — `generation` = 알림이 실은 채택 번호. 올린 창이면 보이고(받아 두기만 하는 창이면 받아 둔다 ·
/// 맡긴 예열 창은 그 채택 번호의 알림만 받는다), 아니면 아무 일도 하지 않는다.
pub(crate) fn reveal_painted(window: &WebviewWindow, generation: Option<u32>) {
    let app = window.app_handle();
    let Some(gate) = gate(app) else {
        return;
    };
    let label = window.label();
    match gate.painted(label, generation) {
        Signal::Release(ticket) => release_on_main_thread(app, gate, label, ticket, Cause::Painted),
        Signal::Mismatch { expected, got } => tracing::debug!(
            module = "first_paint",
            label,
            expected,
            got = ?got,
            "채택 번호가 다른 첫 화면 알림을 버린다 — 채택 전 틀이 보낸 알림이다"
        ),
        Signal::Recorded | Signal::Ignored => {}
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
    // `via` · `reveal_ms` = 예열 창과 새로 짓기의 열기 → 보임 지연을 견주는 재료(TRD S21-slot-docking §3-9 「로그」 · M14).
    // ADR-0299
    fn log(self, label: &str, released: &Released) {
        let held_ms = crate::spare::elapsed_ms(released.since);
        let reveal_ms = released.opened.map(crate::spare::elapsed_ms);
        let via = match released.reveal {
            Reveal::Adopted { .. } => "spare",
            Reveal::Placed | Reveal::Raised => "fresh",
        };
        match self {
            Self::Painted => tracing::info!(
                module = "first_paint",
                label,
                cause = "painted",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "첫 화면 알림에 창을 보인다"
            ),
            Self::PaintedBeforeArming => tracing::info!(
                module = "first_paint",
                label,
                cause = "painted_before_arming",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
                "첫 화면 알림이 먼저 와 있어 창을 곧바로 보인다"
            ),
            Self::Overdue => tracing::warn!(
                module = "first_paint",
                label,
                cause = "overdue",
                via,
                held_ms,
                reveal_ms = ?reveal_ms,
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
        let released = gate.release(label, ticket);
        match &released {
            Some(released) => cause.log(label, released),
            None => tracing::debug!(
                module = "first_paint",
                label,
                ?cause,
                "넘긴 보이기가 돌기 전에 그 올림이 보였거나 거둬져 보이지 않는다"
            ),
        }
        released.map(|released| released.reveal)
    });
}

// `decide` 는 메인 스레드에서 보이기 바로 앞에 돈다(모듈 머리) — `None` = 보이지 않는다.
fn reveal_on_main_thread(
    app: &AppHandle,
    label: &str,
    decide: impl FnOnce(&str) -> Option<Reveal> + Send + 'static,
) {
    let handle = app.clone();
    let owned = label.to_owned();
    let queued = app.run_on_main_thread(move || {
        let Some(reveal) = decide(&owned) else {
            return;
        };
        match handle.get_webview_window(&owned) {
            Some(window) => show_as(&window, reveal),
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

// 메인 스레드에서 부른다(모듈 머리).
fn show_as(window: &WebviewWindow, reveal: Reveal) {
    match reveal {
        Reveal::Placed => crate::state::placement::show_placed(&window.as_ref().window()),
        Reveal::Raised => raise_and_show(window, false),
        Reveal::Adopted { activate, .. } => show_adopted(window, activate),
    }
}

// 맡긴 창 보이기(TRD S21-slot-docking §3-9 「보이는 차례」) — 전경 판정은 보이기 바로 앞이다. 보인 뒤(못 보였어도)
//   다시 짓기 완료를 부른다 — 알림 · 시한 둘 다 여기로 온다.
// ADR-0299
fn show_adopted(window: &WebviewWindow, activate: bool) {
    let app = window.app_handle();
    let show = new_window_show(activate, crate::state::placement::foreground_is_ours(app));
    raise_and_show(window, show == NewWindowShow::RaiseAndActivate);
    crate::commands::spare::on_adopted_window_shown(app, window.label());
}

// ⓐ 숨긴 채 z 만 맨 위로 → ⓑ 보이기 → ⓒ 활성화(`activate` 일 때만). ⓐ 를 ⓑ 앞에 둔다 — 오래 숨은 창이 옛 z 자리에서 한
//   프레임도 보이지 않게(I29). ★ⓑ 는 Tauri `show()` 다★ — 날 Win32 로 보이면 tao 보임 깃발이 거짓으로 남아 트레이
//   숨기기가 아무것도 하지 않는다(`commands::spare` 의 예열 창 짓기 머리). 포커스 없이 지은 창이라 tao 는 활성화하지
//   않고 보인다. ⓐ 만으로 남의 앱 전경 창 위에 서는지 · 숨긴 채 올린 z 가 ⓑ 뒤에도 남는지는 [미측 — M22].
// ADR-0299
fn raise_and_show(window: &WebviewWindow, activate: bool) {
    let label = window.label();
    let os = window.as_ref().window();
    if let Err(e) = engram_dashboard_platform::window::place_window(
        &os,
        None,
        engram_dashboard_platform::window::ZOrder::Top,
        false,
        false,
    ) {
        // 올리기가 돌았는지가 M22 진단 재료다 — 이 OS 에 수단이 없는 것만 조용히 둔다.
        match e {
            engram_dashboard_platform::window::WindowError::Unsupported => {
                tracing::debug!(module = "first_paint", label, error = %e, "숨긴 창의 z 를 올리지 못하고 보인다");
            }
            engram_dashboard_platform::window::WindowError::NoWindow
            | engram_dashboard_platform::window::WindowError::Os(_) => {
                tracing::warn!(module = "first_paint", label, error = %e, "숨긴 창의 z 를 올리지 못하고 보인다");
            }
        }
    }
    if let Err(e) = window.show() {
        tracing::warn!(module = "first_paint", label, error = %e, "숨긴 창을 보이지 못했다");
        return;
    }
    if activate {
        if let Err(e) = window.set_focus() {
            tracing::debug!(module = "first_paint", label, error = %e, "보인 창에 포커스를 주지 못했다");
        }
    }
}

/// 새 창(맡긴 예열 창 · 창 포트가 새로 지은 창)의 보이기 — 둘 다 숨긴 채 z 를 맨 위로 올린 뒤 보인다(I29).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewWindowShow {
    /// 포커스를 그대로 둔다(사용자 결정 2026-10-21 「포커스 뺏지말고, 순서만 위로올려.」).
    Raise,
    /// 보이며 활성화한다 — 맡긴 창은 보인 뒤 포커스를 주고, 새로 짓는 창은 포커스를 받으며 짓는다.
    RaiseAndActivate,
}

/// 활성화는 그 요청이 활성으로 열렸고(`activate`) 지금 전경이 우리 프로세스일 때만이다(I5 · I30) — 전경이 남의 것일 때
/// 활성화하면 tao 가 가짜 Alt 로 포커스를 빼앗는다(`state::placement` 의 `foreground_is_ours` 머리). 맡긴 창은 보이기
/// 바로 앞에, 새로 짓는 창은 열 때 정한다.
pub(crate) fn new_window_show(activate: bool, foreground_is_ours: bool) -> NewWindowShow {
    if activate && foreground_is_ours {
        NewWindowShow::RaiseAndActivate
    } else {
        NewWindowShow::Raise
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::spare::PaintGate;

    fn released(signal: Signal) -> u64 {
        match signal {
            Signal::Release(ticket) => ticket,
            other => panic!("올린 항목이 받았어야 한다: {other:?}"),
        }
    }

    fn reveal_of(gate: &FirstPaintGate, label: &str, ticket: u64) -> Option<Reveal> {
        gate.release(label, ticket).map(|released| released.reveal)
    }

    // ── 명부 ──

    #[test]
    fn the_painted_signal_reveals_a_held_window_once() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        assert_eq!(gate.painted("main", None), Signal::Release(ticket));
        assert_eq!(reveal_of(&gate, "main", ticket), Some(Reveal::Placed));
        assert_eq!(
            gate.painted("main", None),
            Signal::Ignored,
            "두 번째 알림은 보이지 않는다"
        );
    }

    #[test]
    fn a_signal_from_a_window_that_was_not_held_does_nothing() {
        let gate = FirstPaintGate::default();
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
        assert_eq!(gate.painted("slot-popup-1", Some(3)), Signal::Ignored);
    }

    #[test]
    fn the_signal_and_the_deadline_reveal_the_window_once_between_them() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        let signal = released(gate.painted("main", None));
        assert!(gate.release("main", signal).is_some());
        assert!(!gate.is_waiting("main", ticket), "시한은 넘기지 않는다");
        assert!(gate.release("main", ticket).is_none());
    }

    #[test]
    fn the_painted_signal_does_nothing_after_the_deadline() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("main");
        assert!(gate.release("main", ticket).is_some());
        assert_eq!(gate.painted("main", None), Signal::Ignored);
    }

    #[test]
    fn an_old_ticket_does_not_reveal_a_window_held_again_under_the_same_label() {
        let gate = FirstPaintGate::default();
        let old = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", old).is_some());
        let new = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", old).is_none());
        assert!(gate.release("slot-popup-1", new).is_some());
    }

    #[test]
    fn a_built_window_does_not_look_at_the_adoption_number() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("slot-popup-1");
        assert_eq!(
            gate.painted("slot-popup-1", Some(7)),
            Signal::Release(ticket)
        );
    }

    // ── 넘긴 보이기 — 정하기는 넘긴 일이 돌 때다 ──

    #[test]
    fn a_window_hidden_after_the_deadline_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        let ticket = gate.hold("slot-popup-1");
        assert!(gate.is_waiting("slot-popup-1", ticket), "시한이 넘긴다");
        assert!(gate.cancel("slot-popup-1"));
        assert!(gate.release("slot-popup-1", ticket).is_none());
    }

    #[test]
    fn a_window_hidden_after_the_signal_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold("slot-popup-1");
        let queued = released(gate.painted("slot-popup-1", None));
        assert!(gate.cancel("slot-popup-1"));
        assert!(gate.release("slot-popup-1", queued).is_none());
    }

    #[test]
    fn a_window_hidden_after_arming_queued_its_reveal_stays_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Arm::Painted(queued) = gate.arm("slot-popup-1") else {
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert!(gate.cancel("slot-popup-1"));
        assert!(gate.release("slot-popup-1", queued).is_none());
    }

    #[test]
    fn a_window_held_again_after_hiding_is_not_revealed_by_a_reveal_queued_before() {
        let gate = FirstPaintGate::default();
        let first = gate.hold("slot-popup-1");
        let queued = released(gate.painted("slot-popup-1", None));
        assert!(gate.cancel("slot-popup-1"));
        let again = gate.hold("slot-popup-1");
        assert!(gate.release("slot-popup-1", queued).is_none());
        assert!(gate.release("slot-popup-1", first).is_none());
        assert!(gate.release("slot-popup-1", again).is_some());
    }

    #[test]
    fn rearming_after_hiding_ignores_the_reveal_queued_before() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Arm::Painted(queued) = gate.arm("slot-popup-1") else {
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert!(gate.cancel("slot-popup-1"));
        let Arm::Waiting(again) = gate.arm("slot-popup-1") else {
            panic!("거둔 뒤에는 그 전 알림이 남아 있지 않다");
        };
        assert!(gate.release("slot-popup-1", queued).is_none());
        assert!(gate.release("slot-popup-1", again).is_some());
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
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Arm::Painted(ticket) = gate.arm("slot-popup-1") else {
            panic!("알림 뒤에 정했으면 곧바로 넘긴다");
        };
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Placed)
        );
        assert_eq!(
            gate.painted("slot-popup-1", None),
            Signal::Ignored,
            "보인 뒤의 알림은 다시 보이지 않는다"
        );
    }

    #[test]
    fn restoring_counts_a_numbered_signal_as_painted_too() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(4)), Signal::Recorded);
        assert!(matches!(gate.arm("slot-popup-1"), Arm::Painted(_)));
    }

    #[test]
    fn arming_before_the_signal_waits_for_the_signal() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Arm::Waiting(ticket) = gate.arm("slot-popup-1") else {
            panic!("알림 전에 정했으면 기다린다");
        };
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Release(ticket));
        assert!(gate.release("slot-popup-1", ticket).is_some());
        assert!(!gate.is_waiting("slot-popup-1", ticket));
    }

    #[test]
    fn an_unarmed_window_kept_hidden_is_never_revealed() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
    }

    #[test]
    fn arming_a_window_that_was_not_held_waits_from_now() {
        let gate = FirstPaintGate::default();
        let Arm::Waiting(ticket) = gate.arm("slot-popup-1") else {
            panic!("받아 둔 알림이 없으면 기다린다");
        };
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Release(ticket));
        assert!(gate.release("slot-popup-1", ticket).is_some());
    }

    // ── 맡긴 예열 창(확정 ③ · I17 · I26) ──

    #[test]
    fn revealing_a_label_without_a_hold_does_nothing() {
        let gate = FirstPaintGate::default();
        assert_eq!(gate.arm_adopted("slot-popup-1", 1, true, None), None);
        assert_eq!(
            gate.painted("slot-popup-1", Some(1)),
            Signal::Ignored,
            "거둔 올림을 되살리지 않는다"
        );
    }

    #[test]
    fn a_hold_cancelled_before_the_commit_is_not_revealed() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert!(gate.cancel("slot-popup-1"), "UI 숨기기가 거둔다");
        assert_eq!(gate.arm_adopted("slot-popup-1", 1, true, None), None);
    }

    #[test]
    fn a_signal_before_the_commit_without_the_number_is_ignored_and_the_commit_waits() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 2, true, None) else {
            panic!("채택 전 틀의 알림으로 곧바로 보이지 않는다");
        };
        assert!(
            gate.is_waiting("slot-popup-1", ticket),
            "시한은 여기부터 돈다"
        );
        assert_eq!(
            gate.painted("slot-popup-1", Some(2)),
            Signal::Release(ticket)
        );
    }

    #[test]
    fn a_signal_before_the_commit_with_another_number_is_ignored() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Recorded);
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 2, true, None),
            Some(Arm::Waiting(_))
        ));
    }

    #[test]
    fn a_signal_before_the_commit_with_its_number_reveals_at_the_commit() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(2)), Signal::Recorded);
        let Some(Arm::Painted(ticket)) = gate.arm_adopted("slot-popup-1", 2, true, None) else {
            panic!("같은 번호의 알림이 먼저 왔으면 ③ 이 곧바로 보인다");
        };
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Adopted {
                generation: 2,
                activate: true
            })
        );
    }

    #[test]
    fn a_late_numberless_signal_does_not_erase_the_number_received_before_the_commit() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", Some(2)), Signal::Recorded);
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 2, true, None),
            Some(Arm::Painted(_))
        ));
    }

    #[test]
    fn after_the_commit_only_the_signal_with_its_number_reveals() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 5, false, None) else {
            panic!("알림 전에 정했으면 기다린다");
        };
        assert_eq!(
            gate.painted("slot-popup-1", None),
            Signal::Mismatch {
                expected: 5,
                got: None
            }
        );
        assert_eq!(
            gate.painted("slot-popup-1", Some(4)),
            Signal::Mismatch {
                expected: 5,
                got: Some(4)
            }
        );
        assert!(
            gate.is_waiting("slot-popup-1", ticket),
            "버린 알림은 올림을 건드리지 않는다"
        );
        assert_eq!(
            gate.painted("slot-popup-1", Some(5)),
            Signal::Release(ticket)
        );
        assert_eq!(
            reveal_of(&gate, "slot-popup-1", ticket),
            Some(Reveal::Adopted {
                generation: 5,
                activate: false
            }),
            "맡김의 activate 가 항목까지 옮겨진다"
        );
    }

    #[test]
    fn an_adopted_window_is_revealed_once_between_its_signal_and_the_deadline() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 1, true, None) else {
            panic!("알림 전에 정했으면 기다린다");
        };
        let signal = released(gate.painted("slot-popup-1", Some(1)));
        assert!(gate.release("slot-popup-1", signal).is_some());
        assert!(
            !gate.is_waiting("slot-popup-1", ticket),
            "시한은 넘기지 않는다"
        );
        assert!(gate.release("slot-popup-1", ticket).is_none());
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
    }

    #[test]
    fn hiding_after_the_commit_keeps_the_adopted_window_hidden() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 1, true, None) else {
            panic!("알림 전에 정했으면 기다린다");
        };
        assert!(gate.cancel("slot-popup-1"));
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
        assert!(!gate.is_waiting("slot-popup-1", ticket));
        assert!(gate.release("slot-popup-1", ticket).is_none());
    }

    #[test]
    fn a_hold_put_back_by_the_tray_is_revealed_by_the_commit() {
        let gate = FirstPaintGate::default();
        gate.hold_unarmed("slot-popup-1");
        assert!(gate.cancel("slot-popup-1"));
        gate.hold_unarmed("slot-popup-1");
        assert!(matches!(
            gate.arm_adopted("slot-popup-1", 1, true, None),
            Some(Arm::Waiting(_))
        ));
    }

    // ── 맡긴 창 보이기의 갈래(I5 · I30) ──

    #[test]
    fn a_new_window_is_activated_only_when_asked_active_and_the_foreground_is_ours() {
        for (activate, ours, show) in [
            (true, true, NewWindowShow::RaiseAndActivate),
            (true, false, NewWindowShow::Raise),
            (false, true, NewWindowShow::Raise),
            (false, false, NewWindowShow::Raise),
        ] {
            assert_eq!(
                new_window_show(activate, ours),
                show,
                "activate={activate} ours={ours}"
            );
        }
    }

    // 창 포트가 포커스 없이 새로 지은 창(I30) — 번호를 보지 않고 받으며, 올리기만 하는 보이기로 빠진다.
    #[test]
    fn a_window_built_without_focus_takes_any_signal_and_is_raised_without_activation() {
        let gate = FirstPaintGate::default();
        let opened = Instant::now();
        for generation in [None, Some(4)] {
            let ticket = gate.hold_as("slot-popup-2", Reveal::Raised, Some(opened));
            assert_eq!(
                gate.painted("slot-popup-2", generation),
                Signal::Release(ticket)
            );
            let released = gate.release("slot-popup-2", ticket).expect("빼낸다");
            assert_eq!(
                (released.reveal, released.opened),
                (Reveal::Raised, Some(opened))
            );
        }
    }

    // 맡긴 창의 열기 때가 ③ 의 항목을 거쳐 보임 로그까지 간다(`reveal_ms`).
    #[test]
    fn the_open_time_of_an_adopted_window_reaches_its_release() {
        let gate = FirstPaintGate::default();
        let opened = Instant::now();
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 2, false, Some(opened))
        else {
            panic!("알림 전에 정했으면 기다린다");
        };
        assert_eq!(
            gate.painted("slot-popup-1", Some(2)),
            Signal::Release(ticket)
        );
        let released = gate.release("slot-popup-1", ticket).expect("빼낸다");
        assert_eq!(released.opened, Some(opened));
        assert!(matches!(
            released.reveal,
            Reveal::Adopted {
                generation: 2,
                activate: false
            }
        ));
    }

    // ── 풀의 손잡이 ──

    #[test]
    fn the_pool_handle_holds_and_cancels_on_the_shared_gate() {
        let gate = Arc::new(FirstPaintGate::default());
        let handle = SparePaintGate::new(gate.clone());
        handle.hold_unarmed("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Recorded);
        handle.cancel("slot-popup-1");
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
    }

    #[test]
    fn the_pool_handle_without_the_app_treats_the_commit_as_cancelled() {
        let gate = Arc::new(FirstPaintGate::default());
        let handle = SparePaintGate::new(gate.clone());
        handle.hold_unarmed("slot-popup-1");
        assert!(!handle.reveal_if_held("slot-popup-1", 1, true, None));
        assert_eq!(gate.painted("slot-popup-1", Some(1)), Signal::Ignored);
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
        assert!(gate.release("main", ticket).is_some());
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

        let signal = released(gate.painted("main", None));
        assert!(gate.release("main", signal).is_some());
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
        assert_eq!(gate.painted("slot-popup-1", None), Signal::Ignored);
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_adopted_window_whose_signals_carry_another_number_is_revealed_by_the_deadline() {
        let gate = Arc::new(FirstPaintGate::default());
        gate.hold_unarmed("slot-popup-1");
        let Some(Arm::Waiting(ticket)) = gate.arm_adopted("slot-popup-1", 3, true, None) else {
            panic!("알림 전에 정했으면 기다린다");
        };
        let (queued, queue) = counter();
        let deadline = gate.clone();
        let task = tokio::spawn(async move {
            queue_when_overdue(&deadline, "slot-popup-1", ticket, FALLBACK_REVEAL, queue).await
        });

        assert!(matches!(
            gate.painted("slot-popup-1", None),
            Signal::Mismatch { .. }
        ));
        task.await.unwrap();
        assert_eq!(*queued.lock().unwrap(), 1);
        assert!(gate.release("slot-popup-1", ticket).is_some());
    }
}
