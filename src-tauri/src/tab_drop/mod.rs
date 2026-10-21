//! 탭 놓기의 셸 몫(Tauri 0) — 놓기 명령 `tab_drop` 의 장부 [`DropBook`] · 처리 [`handle`], 창 사실 `window:frame` 의
//! 셈 [`frame_insets`] · 바뀔 때만 내기 [`push_frame`]. Tauri 껍데기 · 포트 어댑터 = `commands::tab_drop`. 판정은 순수
//! 모듈 [`plan`], wire 타입은 [`wire`].
//!
//! ★끌기 명령은 끌기가 끝난 뒤에만 온다 (ADR-0296 · ADR-0297)★ — 끌기 하나에 많아야 둘이다. 창 사실은 끌기와 무관하게
//! 흐른다. 끄는 동안 끌기의 판정 · 기하 · 상태를 셸로 보내는 길을 여기 더하지 말 것 — 그 경계 동기화가 ADR-0296 이
//! 없앤 결함의 뿌리다.
//!
//! - ★놓기가 본 상태를 견주지 않는다★ — 놓기가 가리킨 것을 모델이 id 로 지금 상태에서 다시 찾아, 있으면 지금 상태에
//!   맞춰 적용하고 없으면 거절한다(규칙 = [`plan::plan`]).
//! - 모델 쓰기는 사람 · LLM 명령과 같은 `layout::apply::dock_tab` 한 번뿐이다(ADR-0081 · CLAUDE.md 「LLM-우선 제어」).
//!   이 명령은 버스 명령표에 올리지 않는다 — 같은 동작의 LLM 명령은 T-65 `tab.dock` 이 같은 함수 위에 선다.
//! - ★락 규율★ — 장부 · 창 사실 표의 락은 잎이다. 쥔 채 레이아웃 락 · emit · 창 호출을 하지 않는다. 결과 알림
//!   ([`DropShell::done`]) · 포커스는 락을 다 놓은 뒤에 부른다.
// ADR-0296 · ADR-0297
// TRD S21-slot-docking §3

pub mod plan;
pub mod wire;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use engram_dashboard_base::sync;
use uuid::Uuid;

use self::plan::{InputNow, Plan, SourceFrame, TopAt};
use self::wire::{
    DropOutcome, DropTo, EndEffect, NewWindowBy, TabDrop, TabDropDone, TabDropReply, WindowFrame,
};
use crate::layout::apply::{self, DockDest, DockOutcome};
use crate::layout::bands::SizeCss;
use crate::layout::geometry::Insets;
use crate::layout::manager::DockError;
use crate::layout::{
    LabelSource, LayoutEvents, LayoutState, SubscriptionSync, WindowAttrs, WindowHost,
};

/// 장부가 기억하는 끌기 수 — 넘치면 끝난 기록 중 가장 오래된 것부터 잊는다. 진행 중(새 창 진행 · 그 거절 알림 중)인
/// 기록은 맡은 `drop` 을 품을 수 있어 잊지 않는다. 끌기 하나의 두 명령은 곧바로 이어 닿으므로 잊힌 끌기가 다시 오는
/// 일은 없다고 본다.
const BOOK_CAP: usize = 64;

/// 명령이 어디서 왔나 — `Drop` = 우리 웹뷰의 drop(탭바 · 이음새 · `new_window{drop}`) · `DragEnd` = 출처의 dragend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    Drop,
    DragEnd,
}

impl Arrival {
    pub fn of(to: &DropTo) -> Arrival {
        match to {
            DropTo::NewWindow {
                by: NewWindowBy::DragEnd { .. },
                ..
            } => Arrival::DragEnd,
            _ => Arrival::Drop,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Arrival::Drop => "drop",
            Arrival::DragEnd => "drag_end",
        }
    }
}

// 판정까지 마친 놓기 명령 하나 — 장부가 맡아 둘 수 있게 처리에 쓰는 것을 한데 쥔다.
#[derive(Debug)]
struct Handling {
    drop: TabDrop,
    caller: String,
    input: InputNow,
    top_class: Option<String>,
    plan: Plan,
}

// 장부가 끌기마다 적는 갈래.
#[derive(Debug)]
enum Mark {
    // drop 을 처리했다(적용 · 거절 · 무시 모두 · 맡아 둔 drop 을 이어 처리하기로 한 것 포함).
    Drop,
    // drag_end 가 모델을 쓰지 않았다 — 취소 · 무시 · 거절 알림을 낸 뒤 맡은 drop 이 없던 새 창.
    Quiet,
    // drag_end 의 새 창이 진행 중이다 — 그동안 닿은 첫 drop 을 맡아 둔다.
    Placing(Option<Box<Handling>>),
    // drag_end 의 새 창이 맡은 drop 없이 거절돼 그 `done{Rejected}` 을 내는 중이다 — `Placing` 과 같이 첫 drop 을
    //   맡아 두고, 알림 뒤 `close` 가 넘긴다. 알림 전에 받아 처리하면 그 drop 의 `done` 이 거절보다 먼저 갈 수 있다.
    Rejecting(Option<Box<Handling>>),
    // drag_end 의 새 창을 만들었다.
    Placed,
}

impl Mark {
    // 맡은 drop 을 품을 수 있는 기록 — 장부가 넘쳐도 잊지 않는다.
    fn in_flight(&self) -> bool {
        matches!(self, Mark::Placing(_) | Mark::Rejecting(_))
    }
}

// [`DropBook::admit`] 의 답.
#[derive(Debug)]
enum Admit {
    Run(Handling),
    // 같은 끌기의 drag_end 새 창이 진행 중(그 거절 알림 중 포함)이라 맡아 뒀다 — 그 새 창이 거절되면 그 처리가 이어
    //   처리한다.
    Parked,
    Duplicate,
}

// [`DropBook::settle`] 의 답.
#[derive(Debug)]
enum Settle {
    // 진행 중인 새 창의 기록이 아니다 — 할 일 없음.
    Untracked,
    // 새 창이 섰다 — 맡아 둔 drop 이 있었으면 그것(버린다).
    Placed(Option<Box<Handling>>),
    // 새 창이 거절됐고 맡아 둔 drop 이 있다 — 이 처리는 자기 `done` 을 내지 않고 그것을 이어 처리한다.
    Superseded(Box<Handling>),
    // 새 창이 맡은 것 없이 거절됐다 — 기록은 `Rejecting` 이다. 이 처리가 `done{Rejected}` 을 낸 뒤 `close` 를 부른다.
    Rejecting,
}

/// 최근 끌기 id 와 그 처리 갈래 — 끌기 하나에 오는 `drag_end` · `drop` 이 어느 순서로 닿든 모델 쓰기가 많아야 한 번이고
/// `drop` 이 이긴다. 판정 · 적기는 한 락 안이고 그 락은 잎이다(`admit` · `settle` · `close` 안에서만 쥔다).
#[derive(Debug, Default)]
pub struct DropBook {
    seen: Mutex<VecDeque<(Uuid, Mark)>>,
    // 진행 중인 기록만으로 상한을 넘겨 늘렸다는 warn 을 냈나 — 장부마다 한 번만 낸다.
    overflow_warned: AtomicBool,
}

impl DropBook {
    // - 처음 본 끌기 → 처리한다.
    // - `drop` 인데 앞선 기록이 모델을 쓰지 않은 `drag_end`(취소 · 무시 · 거절된 새 창)면 → 처리한다(기록을 `drop` 으로).
    // - `drop` 인데 같은 끌기의 `drag_end` 새 창이 진행 중이거나 그 거절 알림 중이면 → 맡아 둔다(첫 것만 · 넘기는 것은
    //   `settle` · `close`).
    // - 그 밖 → `duplicate`: 같은 끌기의 `drop` 둘 · `drop` 뒤 `drag_end` · 새 창을 만든 `drag_end` 뒤 `drop`.
    // TRD S21-slot-docking §3-4
    fn admit(&self, h: Handling) -> Admit {
        let drag = h.drop.drag;
        let arrival = Arrival::of(&h.drop.to);
        let grew = {
            let mut seen = sync::lock(&self.seen);
            if let Some((_, mark)) = seen.iter_mut().find(|(d, _)| *d == drag) {
                return match mark {
                    Mark::Quiet if arrival == Arrival::Drop => {
                        *mark = Mark::Drop;
                        Admit::Run(h)
                    }
                    Mark::Placing(parked) | Mark::Rejecting(parked)
                        if arrival == Arrival::Drop && parked.is_none() =>
                    {
                        *parked = Some(Box::new(h));
                        Admit::Parked
                    }
                    _ => Admit::Duplicate,
                };
            }
            let mark = match (arrival, &h.plan) {
                (Arrival::Drop, _) => Mark::Drop,
                (Arrival::DragEnd, Plan::Dock(_)) => Mark::Placing(None),
                (Arrival::DragEnd, Plan::Cancel | Plan::Ignore(_)) => Mark::Quiet,
            };
            seen.push_back((drag, mark));
            evict(&mut seen)
        }; // ← 장부 락 드롭
        if grew && !self.overflow_warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                module = "dock",
                cap = BOOK_CAP,
                "놓기 장부 — 상한의 기록이 모두 진행 중이라 잊지 않고 늘린다(이 장부에서 한 번만 알린다)"
            );
        }
        Admit::Run(h)
    }

    // 진행 중인 `drag_end` 새 창의 결과를 적는다. 적용이면 「새 창을 만든 `drag_end`」로 적고 맡아 둔 `drop` 을 돌려준다
    //   (버림 — 뒤의 `drop` 도 `duplicate`). 거절인데 맡은 것이 있으면 그것을 이어 처리하므로 기록을 `drop` 으로, 없으면
    //   `Rejecting` 으로 두어 거절 알림을 내는 동안 닿는 `drop` 도 맡게 한다(`close` 가 넘긴다).
    // TRD S21-slot-docking §3-4
    fn settle(&self, drag: Uuid, outcome: DropOutcome) -> Settle {
        let mut seen = sync::lock(&self.seen);
        let Some((_, mark)) = seen.iter_mut().find(|(d, _)| *d == drag) else {
            return Settle::Untracked;
        };
        let Mark::Placing(parked) = mark else {
            return Settle::Untracked;
        };
        let parked = parked.take();
        match (outcome, parked) {
            (DropOutcome::Rejected, Some(parked)) => {
                *mark = Mark::Drop;
                Settle::Superseded(parked)
            }
            (DropOutcome::Rejected, None) => {
                *mark = Mark::Rejecting(None);
                Settle::Rejecting
            }
            (_, parked) => {
                *mark = Mark::Placed;
                Settle::Placed(parked)
            }
        }
    }

    // 거절 알림을 낸 뒤 부른다 — `Rejecting` 을 끝낸다. 그사이 맡아 둔 `drop` 이 있으면 기록을 `drop` 으로 적고 그것을
    //   돌려준다(이어 처리 — 그 `done` 은 거절 뒤다). 없으면 모델을 쓰지 않은 `drag_end` 로 낮춰 뒤에 닿는 `drop` 이
    //   이기게 한다. `Rejecting` 이 아니면 무동작이다(`None`).
    // TRD S21-slot-docking §3-4
    fn close(&self, drag: Uuid) -> Option<Box<Handling>> {
        let mut seen = sync::lock(&self.seen);
        let (_, mark) = seen.iter_mut().find(|(d, _)| *d == drag)?;
        let Mark::Rejecting(parked) = mark else {
            return None;
        };
        let parked = parked.take();
        *mark = match parked {
            Some(_) => Mark::Drop,
            None => Mark::Quiet,
        };
        parked
    }
}

// 상한을 넘은 만큼 끝난 기록을 오래된 것부터 잊는다 — 방금 넣은 기록(맨 뒤)과 진행 중인 기록은 잊지 않는다.
//   잊을 것이 없어 늘렸으면 `true`.
fn evict(seen: &mut VecDeque<(Uuid, Mark)>) -> bool {
    while seen.len() > BOOK_CAP {
        let older = seen.len() - 1;
        let Some(at) = seen.iter().take(older).position(|(_, m)| !m.in_flight()) else {
            return true;
        };
        seen.remove(at);
    }
    false
}

/// 처리가 레이아웃 밖에서 닿는 셸 — 둘 다 ★락을 다 놓은 뒤에만★ 불린다.
pub trait DropShell: Send + Sync {
    /// 옮긴 탭이 든 창에 포커스를 준다 — 탭바 · 이음새에 놓아 적용됐을 때만(새 창은 열 때 활성으로 연다). 보통 놓은
    /// 창이고, 이음새 대상 탭이 그사이 다른 창으로 갔으면 그 창이다.
    fn focus(&self, label: &str);
    /// 사건 `tab_drop:done` 을 출처 창(`source` = 명령의 `from`)에만 낸다 — 그 창이 없으면(비어 닫힘 따위) 건너뛴다.
    fn done(&self, source: &str, done: &TabDropDone);
}

/// [`handle`] 이 쓰는 포트 — 앞 다섯은 `layout::apply::dock_tab` 의 것 그대로다.
pub struct DropPorts<'a> {
    pub state: &'a LayoutState,
    pub subs: &'a dyn SubscriptionSync,
    pub events: &'a dyn LayoutEvents,
    pub host: &'a dyn WindowHost,
    pub labels: &'a dyn LabelSource,
    pub shell: &'a dyn DropShell,
}

/// 놓기 명령 하나를 처리한다 — 판정([`plan::plan`]) → 장부 → 적용(`dock_tab` 한 번) → 포커스 → 결과 알림 → 로그 한 줄.
/// 입력(`input`)과 출처 창 틀(`frame`)은 부르는 쪽이 이 앞에서 그 차례(입력 먼저)로 읽어 둔다. `caller` = 명령을 부른
/// 창 · `top_class` = 맨 위가 다른 앱 창일 때 그 창 클래스(로그 몫).
///
/// 같은 끌기를 다시 받으면 `Duplicate` 이고 알림이 없다. 같은 끌기의 `drag_end` 새 창이 진행 중일 때(그 거절 알림을
/// 내는 중 포함) 닿은 `drop` 도 `Duplicate` 로 답하되 장부가 맡아 둔다 — 그 새 창이 거절되면 그 처리가 맡은 `drop` 을
/// 보통 `drop` 과 같은 길로 이어 처리하고(그 결과는 `done` 으로만 간다), 서면 버린다. 거절 판정 때 이미 맡아 둔 것이
/// 있으면 그 `drag_end` 처리는 자기 `done` 을 내지 않는다(로그만 · `superseded`) — 출처가 받는 그 끌기의 `done` 은
/// 그 `drop` 의 것 하나다. 거절 알림 뒤에 맡은 것이면 그 `done` 이 거절 뒤에 따른다. 그 밖은 결과마다 `done` 을 한 번
/// 낸다 — 적용이면 `dock_tab` 의 모델 알림 뒤다. 거절은 모델 그대로이고 `Rejected{kind}` 의 `kind` = 도킹 오류 문구의 괄호 속
/// 낱말이다. 취소 · 무시는 모델을 쓰지 않는다.
///
/// ★`new_window` 를 메인 스레드 · 동기 명령에서 부르지 말 것★ — 새 창 빌드가 교착한다(`dock_tab` 문서).
// TRD S21-slot-docking §3-2 · §3-4
pub fn handle(
    book: &DropBook,
    ports: &DropPorts<'_>,
    drop: TabDrop,
    caller: &str,
    input: &InputNow,
    frame: &SourceFrame,
    top_class: Option<&str>,
) -> TabDropReply {
    let arrival = Arrival::of(&drop.to);
    let (drag, view) = (drop.drag, drop.view);
    let plan = plan::plan(&drop, caller, input, frame);
    let item = Handling {
        drop,
        caller: caller.to_owned(),
        input: input.clone(),
        top_class: top_class.map(str::to_owned),
        plan,
    };
    let item = match book.admit(item) {
        Admit::Run(item) => item,
        Admit::Parked => {
            tracing::debug!(
                module = "dock",
                %drag,
                by = arrival.name(),
                %view,
                caller,
                "놓기 — 같은 끌기의 새 창이 진행 중이라 맡겨 둔다(그 새 창이 거절되면 이어 처리)"
            );
            return TabDropReply::Duplicate;
        }
        Admit::Duplicate => {
            tracing::debug!(
                module = "dock",
                %drag,
                by = arrival.name(),
                %view,
                caller,
                "놓기 — 같은 끌기를 이미 처리해 버린다"
            );
            return TabDropReply::Duplicate;
        }
    };
    let (reply, parked) = run(book, ports, item);
    if let Some(parked) = parked {
        // 맡긴 `drop` 의 호출은 이미 `Duplicate` 로 답했다 — 이 처리의 답은 갈 곳이 없다.
        let _ = run(book, ports, *parked);
    }
    reply
}

// 받은 처리 하나 — 적용 → 포커스 → 결과 알림 → 로그. 이 처리가 거절된 `drag_end` 새 창이고 장부가 맡아 둔 `drop` 이
//   있으면 그것을 돌려준다(이어 처리할 것). 거절 판정 때 이미 맡아 둔 것이면 이 처리는 자기 알림을 내지 않는다 — 거절
//   `done` 이 먼저 가면 출처가 바꿔 그리기를 풀어 끄는 탭이 번쩍인다(TRD §2-4 ①). 맡은 것이 없으면 거절 알림을 낸 뒤
//   `close` 가 그사이 맡아 둔 것을 돌려준다 — 그 `done` 은 거절 뒤다.
fn run(
    book: &DropBook,
    ports: &DropPorts<'_>,
    item: Handling,
) -> (TabDropReply, Option<Box<Handling>>) {
    let Handling {
        drop,
        caller,
        input,
        top_class,
        plan,
    } = item;
    let mut via = None;
    let (decision, outcome, reply, settled) = match plan {
        Plan::Dock(req) => {
            let in_place = !matches!(req.to, DockDest::NewWindow { .. });
            let applied = apply::dock_tab(
                ports.state,
                ports.subs,
                ports.events,
                ports.host,
                ports.labels,
                req,
            );
            let outcome = match applied {
                Ok(_) => DropOutcome::Applied,
                Err(_) => DropOutcome::Rejected,
            };
            if let (Ok(out), false) = (&applied, in_place) {
                via = Some(new_window_via(out));
            }
            let settled = book.settle(drop.drag, outcome);
            let reply = match applied {
                Ok(out) => {
                    if in_place {
                        ports.shell.focus(&out.window);
                    }
                    TabDropReply::Applied {
                        window: out.window,
                        reused: out.reused,
                    }
                }
                Err(e) => TabDropReply::Rejected {
                    kind: kind_of(&e),
                    reason: e.to_string(),
                },
            };
            ("dock", outcome, reply, settled)
        }
        Plan::Cancel => (
            "cancel",
            DropOutcome::Canceled,
            TabDropReply::Canceled,
            Settle::Untracked,
        ),
        Plan::Ignore(why) => (
            "ignore",
            DropOutcome::Ignored,
            TabDropReply::Ignored { why: why.into() },
            Settle::Untracked,
        ),
    };
    let superseded = matches!(settled, Settle::Superseded(_));
    if !superseded {
        ports.shell.done(
            &drop.from,
            &TabDropDone {
                drag: drop.drag,
                outcome,
            },
        );
    }
    let replay = match settled {
        Settle::Superseded(parked) => Some(parked),
        // 거절 알림을 낸 뒤에야 닫는다 — 그사이 닿은 `drop` 의 `done` 이 거절보다 먼저 가지 않게.
        Settle::Rejecting => book.close(drop.drag),
        Settle::Placed(Some(parked)) => {
            tracing::debug!(
                module = "dock",
                drag = %parked.drop.drag,
                view = %parked.drop.view,
                caller = %parked.caller,
                "놓기 — 같은 끌기의 새 창이 섰다 — 맡겨 둔 drop 을 버린다"
            );
            None
        }
        Settle::Placed(None) | Settle::Untracked => None,
    };
    log_handled(
        &drop,
        &caller,
        &input,
        top_class.as_deref(),
        decision,
        outcome,
        &reply,
        superseded,
        via,
    );
    (reply, replay)
}

// 놓은 새 창이 어디서 왔나 — 맡긴 예열 창 · 새로 지은 창 · 새 창 없이 옮긴 출처 창. 재사용에 예열 뜻을 겹쳐 싣지 않는다
//   (TRD S21-slot-docking §3-2 ⑦ · §3-9 「로그」).
// ADR-0299
fn new_window_via(out: &DockOutcome) -> &'static str {
    if out.reused {
        "reuse"
    } else if out.prewarmed {
        "spare"
    } else {
        "fresh"
    }
}

#[allow(clippy::too_many_arguments)]
fn log_handled(
    drop: &TabDrop,
    caller: &str,
    input: &InputNow,
    top_class: Option<&str>,
    decision: &str,
    outcome: DropOutcome,
    reply: &TabDropReply,
    superseded: bool,
    via: Option<&str>,
) {
    let (to, effect) = match &drop.to {
        DropTo::TabBar { .. } => ("tab_bar", None),
        DropTo::Seam { .. } => ("seam", None),
        DropTo::NewWindow { by, .. } => (
            "new_window",
            match by {
                NewWindowBy::DragEnd { effect } => Some(match effect {
                    EndEffect::None => "none",
                    EndEffect::Move => "move",
                }),
                NewWindowBy::Drop => None,
            },
        ),
    };
    let top = match &input.top {
        TopAt::Ours(label) => format!("ours={label}"),
        TopAt::Foreign => "foreign".to_owned(),
        TopAt::Unknown => "unknown".to_owned(),
    };
    let (kind, why) = match reply {
        TabDropReply::Rejected { kind, .. } => (Some(kind.as_str()), None),
        TabDropReply::Ignored { why } => (None, Some(why.as_str())),
        _ => (None, None),
    };
    tracing::info!(
        module = "dock",
        drag = %drop.drag,
        by = Arrival::of(&drop.to).name(),
        effect,
        view = %drop.view,
        from = %drop.from,
        caller,
        to,
        decision,
        cursor = ?input.cursor.map(|c| (c.x, c.y)),
        button = input.primary_down,
        esc = input.escape_down,
        top = %top,
        top_class,
        ?outcome,
        kind,
        why,
        superseded = superseded.then_some(true),
        via,
        "놓기 처리"
    );
}

// 도킹 오류 문구의 마지막 괄호 속 낱말 — 문구 = 사유 + ` (종류)`(`DockError` 문서). 사유 안의 괄호는 그 앞이다.
fn kind_of(e: &DockError) -> String {
    let text = e.to_string();
    text.rsplit_once('(')
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .unwrap_or("Unknown")
        .to_owned()
}

// ── 창 사실 `window:frame` (TRD S21-slot-docking §3-8) ──────────────────────────

/// 창 하나의 OS 기하 — 바깥 · 안쪽의 원점과 크기(물리 px — Tauri 게터 넷 그대로)와 그 창 배율.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeom {
    pub outer_pos: (i32, i32),
    pub outer_size: (u32, u32),
    pub inner_pos: (i32, i32),
    pub inner_size: (u32, u32),
    pub scale: f64,
}

/// 창 틀 = 바깥 사각형 − 안쪽 사각형(변마다 · CSS px — 그 창 배율로 나눈 값). `window:frame` 의 `insets` 와 새 창
/// 자리의 `inset_dip`([`source_frame`])이 이 하나로 센다. 배율이 유한 양수가 아니면 `None`.
// TRD S21-slot-docking §3-5 · §3-8
pub fn frame_insets(g: &FrameGeom) -> Option<Insets> {
    if !(g.scale.is_finite() && g.scale > 0.0) {
        return None;
    }
    let (ox, oy) = (f64::from(g.outer_pos.0), f64::from(g.outer_pos.1));
    let (ow, oh) = (f64::from(g.outer_size.0), f64::from(g.outer_size.1));
    let (ix, iy) = (f64::from(g.inner_pos.0), f64::from(g.inner_pos.1));
    let (iw, ih) = (f64::from(g.inner_size.0), f64::from(g.inner_size.1));
    Some(Insets {
        t: (iy - oy) / g.scale,
        r: (ox + ow - (ix + iw)) / g.scale,
        b: (oy + oh - (iy + ih)) / g.scale,
        l: (ix - ox) / g.scale,
    })
}

const NO_INSETS: Insets = Insets {
    t: 0.0,
    r: 0.0,
    b: 0.0,
    l: 0.0,
};

/// 출처 창의 틀 — 기하를 못 읽었으면 틀 0 · 배율 1 이다(새 창이 제목 표시줄 높이만큼 어긋날 뿐이다).
pub fn source_frame(geom: Option<&FrameGeom>) -> SourceFrame {
    match geom.and_then(|g| frame_insets(g).map(|insets| (insets, g.scale))) {
        Some((insets, scale)) => SourceFrame { insets, scale },
        None => SourceFrame {
            insets: NO_INSETS,
            scale: 1.0,
        },
    }
}

/// 창 사실 하나 — 복원 크기 · 최대화는 모델(`WindowAttrs`)의 값이다.
pub fn window_frame(attrs: &WindowAttrs, insets: Insets) -> WindowFrame {
    WindowFrame {
        restored: attrs.bounds.map(|b| SizeCss { w: b.w(), h: b.h() }),
        maximized: attrs.maximized,
        insets,
    }
}

/// 창 하나를 지금 읽은 것 — `minimized` · `geom` 은 게터(어느 락도 쥐지 않고 읽는다), `attrs` 는 그 창의 모델 값.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSeen {
    pub minimized: bool,
    pub attrs: WindowAttrs,
    pub geom: Option<FrameGeom>,
}

/// 창마다 마지막으로 낸 `window:frame` — 값이 바뀔 때만 내게 한다. 락은 잎이다. 닫힌 창의 항목은 그 창이 사라질 때
/// [`FrameBook::forget`] 으로 지운다.
#[derive(Debug, Default)]
pub struct FrameBook {
    last: Mutex<HashMap<String, WindowFrame>>,
}

impl FrameBook {
    fn last(&self, label: &str) -> Option<WindowFrame> {
        sync::lock(&self.last).get(label).copied()
    }

    /// 그 창의 마지막 값을 잊는다 — 창이 사라진 뒤 부른다. 복원된 팝아웃은 저장된 label 로 다시 열리므로, 남겨 두면
    /// 그 창의 첫 값이 옛 창의 값과 같을 때 나가지 않는다.
    pub fn forget(&self, label: &str) {
        sync::lock(&self.last).remove(label);
    }
}

/// 자리 기록 경로(`Moved` · `Resized` · 창 포트의 `record_placement`)가 모델에 적은 뒤 부른다 — 그 창의 값이 마지막으로
/// 낸 값과 다를 때만 `emit` 을 한 번, 표 락을 놓은 뒤에 부른다. 최소화 중 · 기하를 못 읽음이면 내지도 적지도 않는다
/// (최소화 중의 게터는 화면 밖 값이다). 제목 표시줄로 창을 옮기는 동안의 `Moved` 연타는 값이 같아 내지 않는다.
// TRD S21-slot-docking §3-8
pub fn push_frame(book: &FrameBook, label: &str, seen: FrameSeen, emit: impl FnOnce(&WindowFrame)) {
    if seen.minimized {
        return;
    }
    let Some(insets) = seen.geom.as_ref().and_then(frame_insets) else {
        return;
    };
    let frame = window_frame(&seen.attrs, insets);
    let changed = {
        let mut last = sync::lock(&book.last);
        let changed = last.get(label) != Some(&frame);
        if changed {
            last.insert(label.to_owned(), frame);
        }
        changed
    }; // ← 표 락 드롭
    if changed {
        emit(&frame);
    }
}

/// 마운트 때 당기는 `window_frame` 의 답 — 복원 크기 · 최대화는 모델 값, 틀은 지금 게터로 센 값이다. 최소화 중 ·
/// 기하를 못 읽음이면 그 창에 마지막으로 낸 틀(없으면 0)을 싣는다. 표에 적지 않는다.
pub fn pull_frame(book: &FrameBook, label: &str, seen: FrameSeen) -> WindowFrame {
    let fresh = if seen.minimized {
        None
    } else {
        seen.geom.as_ref().and_then(frame_insets)
    };
    let insets = fresh
        .or_else(|| book.last(label).map(|f| f.insets))
        .unwrap_or(NO_INSETS);
    window_frame(&seen.attrs, insets)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use super::plan::{InputNow, PhysPoint, Plan, SourceFrame, TopAt};
    use super::wire::{DropAnchor, DropOutcome, DropTo, EndEffect, NewWindowBy, TabDrop};
    use super::*;
    use crate::commands::popout::PopupCounter;
    use crate::layout::apply::{
        DockDest, DockRequest, MoveFailed, OnEmptied, SpareClaim, WindowPlace, WindowTabsPayload,
    };
    use crate::layout::bands::PointCss;
    use crate::layout::dock::DockSide;
    use crate::layout::manager::{DockGuard, DockMove, DockTo, FocusPolicy};
    use crate::layout::{ViewManager, ViewSnapshot, WindowBounds, MAIN_WINDOW_LABEL};

    const MAIN: &str = MAIN_WINDOW_LABEL;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    // ── 가짜 포트 — 부른 차례를 한 줄에 적고, 락 밖이어야 할 것은 그 자리에서 `try_lock` 으로 본다 ──

    struct Rig {
        state: LayoutState,
        log: Mutex<Vec<String>>,
        // (받는 창, 몸)
        dones: Mutex<Vec<(String, TabDropDone)>>,
        open_fails: bool,
        // 같은 끌기의 drop 을 들이밀 자리 · 장부 · 그 drop — 새 창 drag_end 처리 도중의 경주를 일으킨다.
        racing: Mutex<Option<(RaceAt, Arc<DropBook>, TabDrop)>>,
        // 그렇게 닿은 drop 의 답.
        racing_replies: Mutex<Vec<TabDropReply>>,
        // 창 포트가 한 번 맡길 예열 창.
        spare: Mutex<Option<SpareClaim>>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                state: LayoutState::new(),
                log: Mutex::new(Vec::new()),
                dones: Mutex::new(Vec::new()),
                open_fails: false,
                racing: Mutex::new(None),
                racing_replies: Mutex::new(Vec::new()),
                spare: Mutex::new(None),
            }
        }

        fn outcomes(&self) -> Vec<DropOutcome> {
            self.dones
                .lock()
                .unwrap()
                .iter()
                .map(|(_, d)| d.outcome)
                .collect()
        }

        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }

        fn outside(&self, who: &str) {
            assert!(
                self.state.0.try_lock().is_ok(),
                "{who} 는 레이아웃 락을 놓은 뒤에 불려야 한다"
            );
        }

        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }

        fn mgr(&self) -> std::sync::MutexGuard<'_, ViewManager> {
            self.state.0.lock().unwrap()
        }

        // 들이밀 자리가 `at` 이면 맡겨 둔 drop 을 지금 처리한다(한 번만).
        fn race(&self, at: RaceAt) {
            let racing = {
                let mut slot = self.racing.lock().unwrap();
                match &*slot {
                    Some((when, ..)) if *when == at => slot.take(),
                    _ => None,
                }
            };
            if let Some((_, book, drop)) = racing {
                let reply = self.handle(&book, drop, MAIN, &released(TopAt::Unknown));
                self.racing_replies.lock().unwrap().push(reply);
            }
        }

        fn handle(
            &self,
            book: &DropBook,
            drop: TabDrop,
            caller: &str,
            input: &InputNow,
        ) -> TabDropReply {
            let labels = PopupCounter::default();
            let ports = DropPorts {
                state: &self.state,
                subs: self,
                events: self,
                host: self,
                labels: &labels,
                shell: self,
            };
            handle(book, &ports, drop, caller, input, &FLAT, None)
        }
    }

    impl SubscriptionSync for Rig {
        fn resync(&self, _mgr: &ViewManager) {
            self.note("resync".into());
        }
    }

    impl LayoutEvents for Rig {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {
            self.outside("layout_updated");
            self.note("layout".into());
        }

        fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
            self.outside("window_tabs_updated");
            self.note(format!("tabs:{}", tabs.label));
        }

        fn settled(&self) {
            self.outside("settled");
            self.note("settled".into());
        }
    }

    impl WindowHost for Rig {
        fn open(&self, _label: &str) -> Result<(), String> {
            Ok(())
        }

        fn open_at(&self, label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
            self.note(format!("open:{label}"));
            self.race(RaceAt::Open);
            if self.open_fails {
                Err("창을 못 만듦".into())
            } else {
                Ok(())
            }
        }

        fn close(&self, label: &str) {
            self.note(format!("close:{label}"));
        }

        fn is_open(&self, _label: &str) -> bool {
            true
        }

        fn move_window(&self, label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
            self.note(format!("move:{label}"));
            Ok(())
        }

        fn claim_spare(&self) -> Option<SpareClaim> {
            let claim = self.spare.lock().unwrap().take()?;
            self.note(format!("claim:{}", claim.label));
            Some(claim)
        }

        fn committed(&self, claim: &SpareClaim) {
            self.outside("committed");
            self.note(format!("committed:{}", claim.label));
        }

        fn release_spare(&self, claim: &SpareClaim) {
            self.outside("release_spare");
            self.note(format!("release:{}", claim.label));
        }
    }

    impl DropShell for Rig {
        fn focus(&self, label: &str) {
            self.outside("focus");
            self.note(format!("focus:{label}"));
        }

        fn done(&self, source: &str, done: &TabDropDone) {
            self.outside("done");
            self.note(format!("done:{:?}", done.outcome));
            self.dones.lock().unwrap().push((source.to_owned(), *done));
            self.race(RaceAt::Done);
        }
    }

    // 새 창 drag_end 처리 도중 같은 끌기의 drop 을 들이미는 자리.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RaceAt {
        // 새 창을 여는 사이 — 그 결과를 장부에 적기 전.
        Open,
        // 출처에 첫 `done` 을 내는 사이 — 거절이면 장부가 `Rejecting` 이고 `close` 전이다.
        Done,
    }

    // 이 스레드에서 낸 사건마다 (필드 이름, 값) — 문구는 `message` 칸이다.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<Vec<(String, String)>>>>);

    impl Logs {
        // 「놓기 처리」 줄마다 `name` 칸 — 없으면 `None`.
        fn handled(&self, name: &str) -> Vec<Option<String>> {
            let field = |line: &[(String, String)], name: &str| {
                line.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
            };
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|line| field(line, "message").as_deref() == Some("놓기 처리"))
                .map(|line| field(line, name))
                .collect()
        }
    }

    struct Fields<'a>(&'a mut Vec<(String, String)>);

    impl tracing::field::Visit for Fields<'_> {
        // 문자열 칸은 따옴표 없이 — Debug 로 적으면 `"fresh"` 처럼 따옴표가 붙는다.
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push((field.name().to_owned(), value.to_owned()));
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push((field.name().to_owned(), format!("{value:?}")));
        }
    }

    impl tracing::Subscriber for Logs {
        fn register_callsite(
            &self,
            _: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            tracing::subscriber::Interest::sometimes()
        }
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut line = Vec::new();
            event.record(&mut Fields(&mut line));
            self.0.lock().unwrap().push(line);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    const FLAT: SourceFrame = SourceFrame {
        insets: NO_INSETS,
        scale: 1.0,
    };

    fn input(top: TopAt, button: bool, esc: bool) -> InputNow {
        InputNow {
            cursor: Some(PhysPoint { x: 400, y: 300 }),
            primary_down: button,
            escape_down: esc,
            top,
            scale: 1.0,
        }
    }

    fn released(top: TopAt) -> InputNow {
        input(top, false, false)
    }

    fn drag_end(effect: EndEffect) -> DropTo {
        DropTo::NewWindow {
            by: NewWindowBy::DragEnd { effect },
            size: None,
            grab: PointCss { x: 0.0, y: 0.0 },
        }
    }

    fn drop(drag: Uuid, view: Uuid, from: &str, initial: Option<Uuid>, to: DropTo) -> TabDrop {
        TabDrop {
            drag,
            view,
            from: from.into(),
            initial,
            to,
        }
    }

    // main = [a, b] · 활성 a.
    fn two_tabs(rig: &Rig) -> (Uuid, Uuid) {
        let mut mgr = rig.mgr();
        let a = mgr.list_tabs(MAIN).unwrap().active;
        let b = mgr.create_tab(MAIN, None).unwrap();
        mgr.switch_tab(MAIN, a).unwrap();
        (a, b)
    }

    fn plan_dock() -> Plan {
        Plan::Dock(DockRequest {
            view: id(1),
            to: DockDest::TabBar {
                window: MAIN.into(),
                index: None,
            },
            guard: DockGuard::NONE,
            focus: FocusPolicy::Reveal,
            mv: DockMove::default(),
            on_emptied: OnEmptied::Close,
        })
    }

    // ── ④ 장부 (§3-4) ──────────────────────────────────────────────────────────

    // 장부에 그 끌기의 명령 하나를 들인다 — 답 = "run" · "parked" · "duplicate".
    fn admit(book: &DropBook, drag: Uuid, arrival: Arrival, plan: Plan) -> &'static str {
        let to = match arrival {
            Arrival::Drop => DropTo::TabBar { index: 0 },
            Arrival::DragEnd => drag_end(EndEffect::None),
        };
        let item = Handling {
            drop: drop(drag, id(1), MAIN, None, to),
            caller: MAIN.into(),
            input: released(TopAt::Unknown),
            top_class: None,
            plan,
        };
        match book.admit(item) {
            Admit::Run(_) => "run",
            Admit::Parked => "parked",
            Admit::Duplicate => "duplicate",
        }
    }

    // 장부에 그 끌기의 새 창 결과를 적는다 — 답 = "untracked" · "placed" · "placed+parked" · "superseded" · "rejecting".
    fn settle(book: &DropBook, drag: Uuid, outcome: DropOutcome) -> &'static str {
        match book.settle(drag, outcome) {
            Settle::Untracked => "untracked",
            Settle::Placed(None) => "placed",
            Settle::Placed(Some(_)) => "placed+parked",
            Settle::Superseded(_) => "superseded",
            Settle::Rejecting => "rejecting",
        }
    }

    #[test]
    fn the_book_takes_only_the_first_handling_of_a_drag() {
        let book = DropBook::default();
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "run");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "같은 끌기의 drop 둘"
        );
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "drop 뒤 drag_end"
        );
        assert_eq!(admit(&book, id(2), Arrival::DragEnd, Plan::Cancel), "run");
        assert_eq!(
            admit(&book, id(2), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "같은 drag_end 두 번 배달"
        );
    }

    #[test]
    fn a_drop_wins_over_an_earlier_drag_end_that_wrote_nothing() {
        for quiet in [Plan::Cancel, Plan::Ignore(plan::LOST_DROP)] {
            let book = DropBook::default();
            assert_eq!(admit(&book, id(1), Arrival::DragEnd, quiet.clone()), "run");
            assert_eq!(
                admit(&book, id(1), Arrival::Drop, plan_dock()),
                "run",
                "{quiet:?} 뒤 drop"
            );
            assert_eq!(
                admit(&book, id(1), Arrival::Drop, plan_dock()),
                "duplicate",
                "기록이 drop 으로 바뀌었다"
            );
        }
    }

    #[test]
    fn a_drop_during_a_drag_end_new_window_is_parked_and_settled_by_its_outcome() {
        let book = DropBook::default();
        // 새 창이 섰다 — 맡은 drop 을 돌려받아 버리고, 뒤의 drop 은 duplicate.
        assert_eq!(admit(&book, id(1), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "맡는 것은 첫 drop 하나"
        );
        assert_eq!(settle(&book, id(1), DropOutcome::Applied), "placed+parked");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "duplicate");
        assert_eq!(
            settle(&book, id(1), DropOutcome::Rejected),
            "untracked",
            "끝난 새 창은 다시 settle 되지 않는다"
        );

        // 맡은 drop 이 있는데 새 창이 거절됐다 — 그것이 거절을 대신하므로 뒤의 drop 은 duplicate.
        assert_eq!(admit(&book, id(3), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(3), Arrival::Drop, plan_dock()), "parked");
        let Settle::Superseded(parked) = book.settle(id(3), DropOutcome::Rejected) else {
            panic!("맡은 drop 이 거절을 대신한다");
        };
        assert_eq!(parked.drop.drag, id(3));
        assert_eq!(admit(&book, id(3), Arrival::Drop, plan_dock()), "duplicate");
        assert!(book.close(id(3)).is_none(), "대신한 거절은 닫을 것이 없다");
    }

    #[test]
    fn a_rejected_new_window_without_a_parked_drop_keeps_parking_until_closed() {
        let book = DropBook::default();
        // 거절 알림 중(`settle` 뒤 · `close` 전)에 닿은 drop 은 맡고, `close` 가 넘긴다.
        assert_eq!(admit(&book, id(1), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(settle(&book, id(1), DropOutcome::Rejected), "rejecting");
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "같은 drag_end 두 번 배달"
        );
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "맡는 것은 첫 drop 하나"
        );
        let parked = book.close(id(1)).expect("맡은 drop");
        assert_eq!(parked.drop.drag, id(1));
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "넘긴 drop 을 이어 처리한다"
        );
        assert!(
            book.close(id(1)).is_none(),
            "닫힌 기록은 다시 닫히지 않는다"
        );

        // F6 — 맡은 것 없이 닫히면 뒤의 drop 이 이긴다.
        assert_eq!(admit(&book, id(2), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(settle(&book, id(2), DropOutcome::Rejected), "rejecting");
        assert!(book.close(id(2)).is_none());
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "run");
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "duplicate");
    }

    #[test]
    fn a_rejected_drop_stays_recorded() {
        let book = DropBook::default();
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "run");
        assert_eq!(settle(&book, id(1), DropOutcome::Rejected), "untracked");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "duplicate");
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate"
        );
    }

    #[test]
    fn the_book_forgets_the_oldest_drag_after_sixty_four() {
        let book = DropBook::default();
        for n in 0..=(BOOK_CAP as u128) {
            assert_eq!(admit(&book, id(n), Arrival::Drop, plan_dock()), "run");
        }
        assert_eq!(
            admit(&book, id(0), Arrival::Drop, plan_dock()),
            "run",
            "65 번째가 가장 오래된 것을 밀어냈다"
        );
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "duplicate");
    }

    #[test]
    fn the_book_never_forgets_a_drag_whose_new_window_is_in_flight() {
        let book = DropBook::default();
        // 가장 오래된 둘 = 새 창 진행 중 · 그 거절 알림 중.
        assert_eq!(admit(&book, id(1000), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(1001), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(settle(&book, id(1001), DropOutcome::Rejected), "rejecting");
        // 둘 + 63 = 상한 + 1 — 끝난 것 중 가장 오래된 id(0) 하나가 밀려난다.
        for n in 0..(BOOK_CAP as u128 - 1) {
            assert_eq!(admit(&book, id(n), Arrival::Drop, plan_dock()), "run");
        }
        assert_eq!(
            admit(&book, id(1000), Arrival::Drop, plan_dock()),
            "parked",
            "진행 중인 새 창은 잊지 않았다"
        );
        assert_eq!(
            admit(&book, id(1001), Arrival::Drop, plan_dock()),
            "parked",
            "거절 알림 중인 새 창도"
        );
        assert_eq!(
            admit(&book, id(0), Arrival::Drop, plan_dock()),
            "run",
            "끝난 기록 중 가장 오래된 것이 밀려났다"
        );
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "duplicate");
        assert!(!book.overflow_warned.load(Ordering::Relaxed));
    }

    #[test]
    fn a_book_full_of_in_flight_drags_grows_then_shrinks_back() {
        let book = DropBook::default();
        for n in 0..=(BOOK_CAP as u128) {
            assert_eq!(admit(&book, id(n), Arrival::DragEnd, plan_dock()), "run");
        }
        assert_eq!(
            book.seen.lock().unwrap().len(),
            BOOK_CAP + 1,
            "모두 진행 중이면 늘린다"
        );
        assert!(book.overflow_warned.load(Ordering::Relaxed));
        assert_eq!(admit(&book, id(0), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(settle(&book, id(0), DropOutcome::Applied), "placed+parked");
        for n in 1..=(BOOK_CAP as u128) {
            assert_eq!(settle(&book, id(n), DropOutcome::Applied), "placed");
        }
        assert_eq!(admit(&book, id(999), Arrival::Drop, plan_dock()), "run");
        assert_eq!(
            book.seen.lock().unwrap().len(),
            BOOK_CAP,
            "끝난 뒤 다음 기록이 상한으로 되돌린다"
        );
    }

    #[test]
    fn only_a_new_window_drag_end_arrives_as_a_drag_end() {
        assert_eq!(Arrival::of(&drag_end(EndEffect::Move)), Arrival::DragEnd);
        let by_drop = DropTo::NewWindow {
            by: NewWindowBy::Drop,
            size: None,
            grab: PointCss { x: 0.0, y: 0.0 },
        };
        assert_eq!(Arrival::of(&by_drop), Arrival::Drop);
        assert_eq!(Arrival::of(&DropTo::TabBar { index: 0 }), Arrival::Drop);
    }

    // ── ⑤ 거절 옮기기 · 취소 · 무시 ────────────────────────────────────────────

    #[test]
    fn each_dock_error_kind_is_the_word_in_its_closing_parentheses() {
        let cases = [
            (DockError::TabNotFound(id(1)), "TabNotFound"),
            (DockError::WindowNotFound("w".into()), "WindowNotFound"),
            (DockError::SlotNotFound(id(1)), "SlotNotFound"),
            (
                DockError::SlotNotInTab {
                    slot: id(1),
                    tab: id(2),
                },
                "SlotNotInTab",
            ),
            (DockError::SelfTarget, "SelfTarget"),
            (DockError::ZeroArea, "ZeroArea"),
            (DockError::InvalidShare("0.9".into()), "InvalidShare"),
            (DockError::Dragging, "Dragging"),
            (DockError::WindowClosed("w".into()), "WindowClosed"),
            (DockError::StaleLease, "StaleLease"),
            (DockError::SourceChanged, "SourceChanged"),
            (DockError::TargetChanged, "TargetChanged"),
            (DockError::WindowExists("w".into()), "WindowExists"),
            (DockError::SlotIdOverlap, "SlotIdOverlap"),
            (
                DockError::WindowOpen("창 옮기기 Position 실패 — 접근 거부 (os error 5)".into()),
                "WindowOpen",
            ),
        ];
        for (e, want) in cases {
            assert_eq!(kind_of(&e), want, "{e}");
        }
    }

    #[test]
    fn a_rejected_drop_reports_the_kind_and_leaves_the_model() {
        let rig = Rig::new();
        two_tabs(&rig);
        let before = rig.mgr().version;
        let gone = id(42);
        let reply = rig.handle(
            &DropBook::default(),
            drop(id(9), gone, MAIN, None, DropTo::TabBar { index: 1 }),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert_eq!(
            reply,
            TabDropReply::Rejected {
                kind: "TabNotFound".into(),
                reason: DockError::TabNotFound(gone).to_string(),
            },
            "끄는 탭이 그사이 사라졌다"
        );
        assert_eq!(rig.mgr().version, before);
        assert_eq!(rig.mgr().list_tabs(MAIN).unwrap().tabs.len(), 2);
        assert_eq!(rig.take_log(), ["done:Rejected"], "알림 · 포커스 없음");
    }

    #[test]
    fn a_cancel_or_ignore_writes_nothing() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let before = rig.mgr().version;
        let cases = [
            (
                drag_end(EndEffect::None),
                input(TopAt::Foreign, true, false),
                TabDropReply::Canceled,
            ),
            (
                drag_end(EndEffect::None),
                released(TopAt::Ours(MAIN.into())),
                TabDropReply::Ignored {
                    why: plan::LOST_DROP.into(),
                },
            ),
            (
                drag_end(EndEffect::Move),
                released(TopAt::Unknown),
                TabDropReply::Ignored {
                    why: plan::ACCEPTED_UNKNOWN.into(),
                },
            ),
        ];
        for (n, (to, now, want)) in cases.into_iter().enumerate() {
            let reply = rig.handle(&book, drop(id(n as u128), a, MAIN, None, to), MAIN, &now);
            assert_eq!(reply, want);
        }
        assert_eq!(rig.mgr().version, before);
        assert_eq!(
            rig.take_log(),
            ["done:Canceled", "done:Ignored", "done:Ignored"],
            "모델 · 창 · 구독 어느 포트도 불리지 않는다"
        );
    }

    // ── ⑨ `tab_drop:done` ──────────────────────────────────────────────────────

    #[test]
    fn done_comes_once_per_handling_after_the_model_news_and_never_for_a_duplicate() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let reorder = drop(id(7), a, MAIN, Some(a), DropTo::TabBar { index: 1 });
        let reply = rig.handle(&book, reorder.clone(), MAIN, &released(TopAt::Unknown));
        assert_eq!(
            reply,
            TabDropReply::Applied {
                window: MAIN.into(),
                reused: false
            }
        );
        assert_eq!(rig.mgr().list_tabs(MAIN).unwrap().tabs[1].id, a);
        let log = rig.take_log();
        assert_eq!(
            log.last().map(String::as_str),
            Some("done:Applied"),
            "{log:?}"
        );
        let settled = log.iter().position(|s| s == "settled").expect("묶음 끝");
        let focus = log.iter().position(|s| s == "focus:main").expect("포커스");
        assert!(settled < focus, "모델 알림 → 포커스 → done: {log:?}");
        assert_eq!(log.iter().filter(|s| s.starts_with("done:")).count(), 1);

        assert_eq!(
            rig.handle(&book, reorder, MAIN, &released(TopAt::Unknown)),
            TabDropReply::Duplicate
        );
        assert!(rig.take_log().is_empty(), "중복은 알림도 없다");
    }

    #[test]
    fn an_ignored_drag_end_then_the_winning_drop_each_announce_once() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let ignored = rig.handle(
            &book,
            drop(id(5), a, MAIN, Some(a), drag_end(EndEffect::Move)),
            MAIN,
            &released(TopAt::Ours(MAIN.into())),
        );
        assert_eq!(
            ignored,
            TabDropReply::Ignored {
                why: plan::ACCEPTED_OURS.into()
            }
        );
        let won = rig.handle(
            &book,
            drop(id(5), a, MAIN, Some(a), DropTo::TabBar { index: 1 }),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert!(matches!(won, TabDropReply::Applied { .. }), "{won:?}");
        assert_eq!(rig.outcomes(), [DropOutcome::Ignored, DropOutcome::Applied]);
    }

    #[test]
    fn a_drop_after_a_rejected_drag_end_new_window_is_applied() {
        let rig = Rig {
            open_fails: true,
            ..Rig::new()
        };
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let rejected = rig.handle(
            &book,
            drop(id(6), a, MAIN, Some(a), drag_end(EndEffect::None)),
            MAIN,
            &released(TopAt::Foreign),
        );
        assert!(
            matches!(&rejected, TabDropReply::Rejected { kind, .. } if kind == "WindowOpen"),
            "{rejected:?}"
        );
        let won = rig.handle(
            &book,
            drop(id(6), a, MAIN, Some(a), DropTo::TabBar { index: 1 }),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert!(matches!(won, TabDropReply::Applied { .. }), "{won:?}");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Rejected, DropOutcome::Applied],
            "거절 알림 · close 뒤에 닿은 drop 은 보통 drop 처럼 이긴다"
        );
    }

    #[test]
    fn a_seam_drop_into_another_window_applies_with_focus_on_that_window() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let target = rig.mgr().create_window("slot-popup-1").unwrap();
        let seam = DropTo::Seam {
            target,
            anchor: DropAnchor::Outer,
            side: DockSide::Right,
            share: 0.5,
        };
        let reply = rig.handle(
            &DropBook::default(),
            drop(id(8), a, MAIN, None, seam),
            "slot-popup-1",
            &released(TopAt::Unknown),
        );
        assert_eq!(
            reply,
            TabDropReply::Applied {
                window: "slot-popup-1".into(),
                reused: false
            }
        );
        let log = rig.take_log();
        assert!(log.contains(&"focus:slot-popup-1".to_owned()), "{log:?}");
        assert_eq!(log.last().map(String::as_str), Some("done:Applied"));
        let dones = rig.dones.lock().unwrap();
        assert_eq!(
            dones.iter().map(|(to, _)| to.as_str()).collect::<Vec<_>>(),
            [MAIN],
            "done 은 부른 창이 아니라 출처 창(from)에만 간다"
        );
    }

    #[test]
    fn a_seam_drop_whose_target_tab_moved_to_another_window_docks_there_and_focuses_that_window() {
        let rig = Rig::new();
        let (a, b) = two_tabs(&rig);
        // main 에 보이던 b 의 이음새에 놓기 전에 b 가 팝아웃으로 갔다(LLM 따위).
        let popup = {
            let mut mgr = rig.mgr();
            mgr.create_window("slot-popup-1").unwrap();
            mgr.dock(
                b,
                DockTo::TabBar {
                    window: "slot-popup-1".into(),
                    index: None,
                },
                &DockGuard::NONE,
                FocusPolicy::Keep,
                DockMove::default(),
            )
            .unwrap();
            "slot-popup-1"
        };
        let seam = DropTo::Seam {
            target: b,
            anchor: DropAnchor::Outer,
            side: DockSide::Right,
            share: 0.5,
        };
        let reply = rig.handle(
            &DropBook::default(),
            drop(id(8), a, MAIN, None, seam),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert_eq!(
            reply,
            TabDropReply::Applied {
                window: popup.into(),
                reused: false
            }
        );
        let log = rig.take_log();
        assert!(log.contains(&format!("focus:{popup}")), "{log:?}");
        assert!(!log.contains(&"focus:main".to_owned()), "{log:?}");
        assert_eq!(rig.mgr().owner_of(a), None, "a 는 b 에 붙어 사라졌다");
    }

    // ── ⑩ 진행 중인 drag_end 새 창과 같은 끌기의 drop ──────────────────────────────

    // main = [a, b] 에서 a 를 끈 끌기 id(6) — drag_end(새 창)를 처리하는 도중 `at` 자리에서 탭바 drop 이 닿는다.
    fn race_new_window_with_a_drop(
        rig: &Rig,
        at: RaceAt,
    ) -> (Uuid, Uuid, Arc<DropBook>, TabDropReply) {
        let (a, b) = two_tabs(rig);
        let book = Arc::new(DropBook::default());
        let reorder = drop(id(6), a, MAIN, Some(a), DropTo::TabBar { index: 1 });
        *rig.racing.lock().unwrap() = Some((at, book.clone(), reorder));
        let ended = rig.handle(
            &book,
            drop(id(6), a, MAIN, Some(a), drag_end(EndEffect::None)),
            MAIN,
            &released(TopAt::Foreign),
        );
        assert_eq!(
            *rig.racing_replies.lock().unwrap(),
            [TabDropReply::Duplicate],
            "새 창이 진행 중인 동안 닿은 drop 은 맡기고 곧바로 답한다"
        );
        (a, b, book, ended)
    }

    fn main_tabs(rig: &Rig) -> Vec<Uuid> {
        rig.mgr()
            .list_tabs(MAIN)
            .unwrap()
            .tabs
            .iter()
            .map(|t| t.id)
            .collect()
    }

    // 같은 끌기의 drop 이 하나 더 닿아도 duplicate 이고 알림이 없다.
    fn assert_drag_is_over(rig: &Rig, book: &DropBook, a: Uuid) {
        let again = drop(id(6), a, MAIN, Some(a), DropTo::TabBar { index: 0 });
        assert_eq!(
            rig.handle(book, again, MAIN, &released(TopAt::Unknown)),
            TabDropReply::Duplicate,
            "이어 처리한 drop 뒤의 같은 끌기 drop"
        );
        assert!(rig.take_log().is_empty());
    }

    #[test]
    fn a_drop_parked_before_the_new_window_is_rejected_replaces_its_done() {
        let rig = Rig {
            open_fails: true,
            ..Rig::new()
        };
        let logs = Logs::default();
        let (a, b, book, ended) = tracing::subscriber::with_default(logs.clone(), || {
            race_new_window_with_a_drop(&rig, RaceAt::Open)
        });
        assert!(
            matches!(&ended, TabDropReply::Rejected { kind, .. } if kind == "WindowOpen"),
            "{ended:?}"
        );
        assert_eq!(main_tabs(&rig), [b, a], "맡겨 둔 drop 이 적용됐다");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Applied],
            "거절된 drag_end 는 done 을 내지 않는다 — 출처가 받는 done 은 drop 의 것 하나"
        );
        assert_eq!(
            logs.handled("superseded"),
            [Some("true".to_owned()), None],
            "drag_end 처리 줄은 남기되 자리를 내줬다고 적는다"
        );
        let log = rig.take_log();
        let focus = log.iter().position(|s| s == "focus:main").expect("포커스");
        let applied = log.iter().position(|s| s == "done:Applied").unwrap();
        assert!(focus < applied, "{log:?}");
        assert_drag_is_over(&rig, &book, a);
    }

    #[test]
    fn a_drop_during_the_rejected_announcement_is_parked_and_applied_after_it() {
        let rig = Rig {
            open_fails: true,
            ..Rig::new()
        };
        let logs = Logs::default();
        let (a, b, book, ended) = tracing::subscriber::with_default(logs.clone(), || {
            race_new_window_with_a_drop(&rig, RaceAt::Done)
        });
        assert!(
            matches!(&ended, TabDropReply::Rejected { kind, .. } if kind == "WindowOpen"),
            "{ended:?}"
        );
        assert_eq!(
            main_tabs(&rig),
            [b, a],
            "거절 알림 뒤 맡겨 둔 drop 이 적용됐다"
        );
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Rejected, DropOutcome::Applied],
            "거절 done 이 먼저 · 그 끌기의 마지막 done 은 drop 의 것"
        );
        assert_eq!(logs.handled("superseded"), [None, None]);
        let log = rig.take_log();
        let rejected = log.iter().position(|s| s == "done:Rejected").unwrap();
        let focus = log.iter().position(|s| s == "focus:main").expect("포커스");
        let applied = log.iter().position(|s| s == "done:Applied").unwrap();
        assert!(rejected < focus && focus < applied, "{log:?}");
        assert_drag_is_over(&rig, &book, a);
    }

    #[test]
    fn a_drop_parked_during_a_new_window_that_opened_is_a_duplicate_without_done() {
        let rig = Rig::new();
        let (a, b, _book, ended) = race_new_window_with_a_drop(&rig, RaceAt::Open);
        let TabDropReply::Applied {
            window,
            reused: false,
        } = ended
        else {
            panic!("{ended:?}");
        };
        let ids = |label: &str| -> Vec<Uuid> {
            rig.mgr()
                .list_tabs(label)
                .unwrap()
                .tabs
                .iter()
                .map(|t| t.id)
                .collect()
        };
        assert_eq!(ids(&window), [a]);
        assert_eq!(ids(MAIN), [b], "맡겨 둔 drop 은 모델을 쓰지 않았다");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Applied],
            "맡겨 둔 drop 은 알림도 없다"
        );
    }

    // ── ⑦ 놓기 처리 줄의 `via` (TRD §3-2 ⑦ · §3-9 「로그」 · ADR-0299) ─────────────────────────

    // 처리 하나의 `via` 칸과 그 답 — `view` 를 `from` 창에서 바탕화면으로 끌어 놓는다(새 창).
    fn new_window_drop_via(rig: &Rig, view: Uuid, from: &str) -> (Option<String>, TabDropReply) {
        let logs = Logs::default();
        let reply = tracing::subscriber::with_default(logs.clone(), || {
            rig.handle(
                &DropBook::default(),
                drop(id(8), view, from, Some(view), drag_end(EndEffect::None)),
                from,
                &released(TopAt::Foreign),
            )
        });
        let [via] = logs.handled("via").try_into().expect("놓기 처리 줄 하나");
        (via, reply)
    }

    #[test]
    fn a_new_window_drop_logs_whether_the_window_was_prewarmed_built_or_reused() {
        let fresh = Rig::new();
        let (a, _) = two_tabs(&fresh);
        assert_eq!(
            new_window_drop_via(&fresh, a, MAIN).0.as_deref(),
            Some("fresh")
        );

        let prewarmed = Rig::new();
        let (a, _) = two_tabs(&prewarmed);
        let spare = "slot-popup-9";
        *prewarmed.spare.lock().unwrap() = Some(SpareClaim {
            label: spare.into(),
            generation: 4,
        });
        let (via, reply) = new_window_drop_via(&prewarmed, a, MAIN);
        assert_eq!(via.as_deref(), Some("spare"));
        assert_eq!(
            reply,
            TabDropReply::Applied {
                window: spare.into(),
                reused: false
            },
            "맡긴 창의 label 로 연다"
        );
        let log = prewarmed.take_log();
        let at = |entry: &str| log.iter().position(|s| s == entry).expect(entry);
        assert!(
            at("claim:slot-popup-9") < at("open:slot-popup-9")
                && at("open:slot-popup-9") < at("settled")
                && at("settled") < at("committed:slot-popup-9"),
            "맡기기 → 열기 → 알림 → 확정: {log:?}"
        );

        let reused = Rig::new();
        let lone = reused.mgr().create_window("slot-popup-5").unwrap();
        let (via, reply) = new_window_drop_via(&reused, lone, "slot-popup-5");
        assert!(
            matches!(reply, TabDropReply::Applied { reused: true, .. }),
            "{reply:?}"
        );
        assert_eq!(via.as_deref(), Some("reuse"));

        let in_place = Rig::new();
        let (a, _) = two_tabs(&in_place);
        let logs = Logs::default();
        tracing::subscriber::with_default(logs.clone(), || {
            in_place.handle(
                &DropBook::default(),
                drop(id(9), a, MAIN, Some(a), DropTo::TabBar { index: 1 }),
                MAIN,
                &released(TopAt::Unknown),
            )
        });
        assert_eq!(logs.handled("via"), [None], "새 창이 아닌 놓기에는 없다");
    }

    // ── ⑥ `window:frame` ───────────────────────────────────────────────────────

    // 바깥 (100, 50) 816 × 639 · 안쪽 (108, 81) 800 × 600 — 틀 = 위 31 · 나머지 8.
    const GEOM: FrameGeom = FrameGeom {
        outer_pos: (100, 50),
        outer_size: (816, 639),
        inner_pos: (108, 81),
        inner_size: (800, 600),
        scale: 1.0,
    };

    fn attrs(maximized: bool) -> WindowAttrs {
        WindowAttrs {
            theme: None,
            bounds: WindowBounds::new(100.0, 50.0, 800.0, 600.0),
            maximized,
        }
    }

    fn seen(maximized: bool, geom: Option<FrameGeom>) -> FrameSeen {
        FrameSeen {
            minimized: false,
            attrs: attrs(maximized),
            geom,
        }
    }

    #[test]
    fn frame_insets_are_outer_minus_inner_divided_by_the_scale() {
        assert_eq!(
            frame_insets(&GEOM),
            Some(Insets {
                t: 31.0,
                r: 8.0,
                b: 8.0,
                l: 8.0
            })
        );
        let scaled = FrameGeom {
            outer_pos: (0, 0),
            outer_size: (1020, 799),
            inner_pos: (10, 39),
            inner_size: (1000, 750),
            scale: 1.25,
        };
        assert_eq!(
            frame_insets(&scaled),
            Some(Insets {
                t: 31.2,
                r: 8.0,
                b: 8.0,
                l: 8.0
            })
        );
        for bad in [0.0, -1.0, f64::NAN] {
            assert_eq!(frame_insets(&FrameGeom { scale: bad, ..GEOM }), None);
        }
    }

    #[test]
    fn the_frame_goes_out_only_when_it_changes() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        let mut push = |s: FrameSeen| push_frame(&book, MAIN, s, |f| sent.push(*f));
        push(seen(false, Some(GEOM)));
        // 제목 표시줄로 옮기는 동안의 `Moved` 연타 — 크기 · 틀 · 최대화가 같다.
        for x in 0..5 {
            push(seen(
                false,
                Some(FrameGeom {
                    outer_pos: (100 + x, 50),
                    inner_pos: (108 + x, 81),
                    ..GEOM
                }),
            ));
        }
        push(seen(true, Some(GEOM)));
        push(seen(true, Some(GEOM)));
        let thicker = FrameGeom {
            inner_pos: (110, 81),
            outer_size: (820, 639),
            ..GEOM
        };
        push(seen(true, Some(thicker)));
        assert_eq!(
            sent,
            [
                window_frame(&attrs(false), frame_insets(&GEOM).unwrap()),
                window_frame(&attrs(true), frame_insets(&GEOM).unwrap()),
                window_frame(&attrs(true), frame_insets(&thicker).unwrap()),
            ]
        );
        assert_eq!(sent[0].restored, Some(SizeCss { w: 800.0, h: 600.0 }));
    }

    #[test]
    fn a_forgotten_window_sends_its_first_frame_again_and_others_are_kept() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        for label in ["slot-popup-1", MAIN] {
            push_frame(&book, label, seen(false, Some(GEOM)), |_| sent.push(label));
        }
        book.forget("slot-popup-1");
        // 복원된 팝아웃이 같은 label 로 다시 열림 — 첫 값이 옛 창의 값과 같아도 나간다.
        for label in ["slot-popup-1", MAIN] {
            push_frame(&book, label, seen(false, Some(GEOM)), |_| sent.push(label));
        }
        assert_eq!(sent, ["slot-popup-1", MAIN, "slot-popup-1"]);
        assert_eq!(
            pull_frame(&book, "slot-popup-1", seen(false, None)).insets,
            frame_insets(&GEOM).unwrap()
        );
        book.forget("slot-popup-1");
        assert_eq!(
            pull_frame(&book, "slot-popup-1", seen(false, None)).insets,
            NO_INSETS,
            "잊은 창의 당기기는 틀 0 이다"
        );
    }

    #[test]
    fn no_frame_goes_out_while_minimized_or_unreadable() {
        let book = FrameBook::default();
        let mut sent = 0;
        let minimized = FrameSeen {
            minimized: true,
            ..seen(false, Some(GEOM))
        };
        push_frame(&book, MAIN, minimized, |_| sent += 1);
        push_frame(&book, MAIN, seen(false, None), |_| sent += 1);
        assert_eq!(sent, 0);
        // 적지도 않았다 — 풀린 뒤 첫 값은 나간다.
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(sent, 1);
        // 최소화 동안 못 읽은 값이 표를 바꾸지 않아, 풀린 뒤 같은 값은 다시 나가지 않는다.
        push_frame(&book, MAIN, minimized, |_| sent += 1);
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(sent, 1);
    }

    #[test]
    fn the_frame_is_emitted_outside_the_table_lock_and_per_window() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        for label in [MAIN, "slot-popup-1"] {
            push_frame(&book, label, seen(false, Some(GEOM)), |_| {
                assert!(book.last.try_lock().is_ok(), "표 락을 놓은 뒤에 낸다");
                sent.push(label);
            });
        }
        assert_eq!(sent, [MAIN, "slot-popup-1"], "창마다 따로 센다");
    }

    #[test]
    fn the_pull_answers_the_model_values_with_fresh_insets_and_does_not_touch_the_table() {
        let book = FrameBook::default();
        assert_eq!(
            pull_frame(&book, MAIN, seen(true, Some(GEOM))),
            WindowFrame {
                restored: Some(SizeCss { w: 800.0, h: 600.0 }),
                maximized: true,
                insets: frame_insets(&GEOM).unwrap(),
            }
        );
        let unknown = FrameSeen {
            attrs: WindowAttrs::default(),
            ..seen(false, Some(GEOM))
        };
        assert_eq!(pull_frame(&book, MAIN, unknown).restored, None);
        // 최소화 중 · 못 읽음 = 마지막으로 낸 틀(없으면 0).
        let minimized = FrameSeen {
            minimized: true,
            ..seen(false, Some(GEOM))
        };
        assert_eq!(pull_frame(&book, MAIN, minimized).insets, NO_INSETS);
        let mut sent = 0;
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(
            pull_frame(&book, MAIN, minimized).insets,
            frame_insets(&GEOM).unwrap()
        );
        assert_eq!(
            pull_frame(&book, MAIN, seen(false, None)).insets,
            frame_insets(&GEOM).unwrap()
        );
        // 당기기는 표를 바꾸지 않는다 — 다른 값을 당긴 뒤에도 같은 값의 밀기는 나가지 않는다.
        let _ = pull_frame(&book, MAIN, seen(true, Some(GEOM)));
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(sent, 1);
    }

    #[test]
    fn the_new_window_inset_and_the_pushed_frame_insets_come_from_one_function() {
        let book = FrameBook::default();
        let mut pushed = None;
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |f| {
            pushed = Some(f.insets)
        });
        let source = source_frame(Some(&GEOM));
        assert_eq!(Some(source.insets), pushed);
        assert_eq!(source.scale, GEOM.scale);
        assert_eq!(
            source_frame(None),
            SourceFrame {
                insets: NO_INSETS,
                scale: 1.0
            }
        );
    }
}
