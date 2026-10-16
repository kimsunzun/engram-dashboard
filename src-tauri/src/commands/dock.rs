//! 도킹 끌기의 실행기 + invoke 껍데기(TRD S21-slot-docking §1-3 · §1-5) — 웹뷰 invoke · 레이아웃 계기 · 창 사건 ·
//! 시한을 끌기 장부(`dock::book`)의 입력으로 바꿔 먹이고, 장부가 낸 할 일을 실행한다. 판정은 장부가, 레이아웃
//! 변경은 적용 서비스(`layout::apply::dock_tab`)가 한다 — 여기는 락 · 스레드 · 순서만 진다.
//!
//! ## ★락 규율★
//! - 장부 락(`DockState::book`)은 잎이다 — 레이아웃 락을 쥔 채 장부 락을 잡는 것은 되고 거꾸로는 안 된다. 쥔 채
//!   emit · 창 호출 · 적용 서비스 · 게터를 부르지 않는다.
//! - 창 게터(커서 · 보임 · 최소화 · 안쪽 자리 · 크기 · 배율)는 두 락을 다 놓고 읽는다 — 메인 스레드가 아닌 곳의
//!   게터는 이벤트 루프의 답을 기다리고, 메인 스레드의 창 사건 처리기는 레이아웃 락을 잡는다(TRD §1-3 「창 게터」).
//! - 할 일은 두 락을 다 놓은 뒤 낸 순서대로 실행한다 — `ReleaseLease` 가 레이아웃 락을 잡고 std 락은 다시 들어갈
//!   수 없다.
//!
//! ## ★`Apply` 는 늘 spawn★
//! 할 일을 먹인 스레드와 무관하게 적용은 [`DockSpawn::spawn`] 으로 넘기고, 그 결과(`Applied`)와 끝 할 일도 그
//! 작업에서 먹이고 실행한다. 동기 명령(메인 스레드)인 `dock_drag_start` 도 붙든 이른 놓기를 이어 먹여 새 창 적용을
//! 낼 수 있고, 그것을 그 자리에서 돌리면 창을 만들다 교착한다(TRD §1-3 「명령의 실행 스레드」 · §11-2 #23).
//!
//! 바깥 세계에는 포트 둘([`DockPorts`] · [`DockSpawn`])로만 닿는다 — 시험이 가짜로 갈아 끼운다(ADR-0012).

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, Window};
use uuid::Uuid;

use crate::commands::layout::{RouterSubs, TauriEvents};
use crate::commands::popout::{PopupCounter, TauriWindowHost};
use crate::daemon_client::DaemonClient;
use crate::dock::book::{
    Action, Book, ClientRect, EndLog, EndReason, Input, MonoMs, PhysPoint, PullReply, Rects,
    Rejection, Reply, Report, Step, TimerKind,
};
use crate::dock::snap::StartMaterial;
use crate::dock::types::{
    DockPullReply, DockRegions, DragEffect, DragOverReport, DropReport, Picked, SessionEvent,
};
use crate::dock::DockState;
use crate::layout::apply;
use crate::layout::chrome::{DockChrome, PointCss};
use crate::layout::manager::{DockError, ViewId, WindowLabel};
use crate::layout::{DockOutcome, DockRequest, LayoutState, ViewManager};
use crate::output_router::OutputRouter;

const EVT_SESSION: &str = "dock:session";
const EVT_REGIONS: &str = "dock:regions";

/// 창 하나의 게터 값 — 실행기가 두 락 밖에서 읽는다. `origin` · `size` = 물리 안쪽 자리 · 크기(`inner_position` ·
/// `inner_size`) · `scale` = 그 창 배율.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct WindowProbe {
    pub visible: bool,
    pub minimized: bool,
    pub origin: (i32, i32),
    pub size: (u32, u32),
    pub scale: f64,
}

/// 실행기가 Tauri 세계에 닿는 포트. ★전부 레이아웃 락 · 장부 락 밖에서만 불린다★(모듈 머리 「락 규율」).
pub(crate) trait DockPorts: Send + Sync {
    /// `dock:session` 을 모든 창에.
    fn broadcast(&self, event: &SessionEvent) -> Result<(), String>;
    /// `dock:regions` 를 그 창 대상으로(`emit_to`). ★대상을 정해도 그 창에만 닿는 것이 아니다★ — JS 리스너의 기본
    /// 대상은 `Any` 이고 `Any` 리스너는 어느 대상의 사건이든 받는다(`@tauri-apps/api` `event.js` 의 `listen` ·
    /// tauri 2.11.3 `event/listener.rs:310`). 영역에는 창 label 이 없어 받은 쪽이 가를 수 없으므로, 웹뷰는 자기 창
    /// 대상으로 듣는다(`getCurrentWebviewWindow().listen` · `listen(.., { target: label })` — 7 단계 계약).
    fn send_regions(&self, label: &str, regions: &DockRegions) -> Result<(), String>;
    fn hide(&self, label: &str) -> Result<(), String>;
    /// 보이고 미룬 최대화를 입힌다(TRD §1-3 「끝」 — 보이기 경로 `apply_deferred_maximize`).
    fn show(&self, label: &str) -> Result<(), String>;
    fn focus(&self, label: &str) -> Result<(), String>;
    /// 셸 커서 — 물리 px.
    fn cursor(&self) -> Result<PhysPoint, String>;
    /// 떠 있는 우리 창 label 전부.
    fn windows(&self) -> Vec<WindowLabel>;
    fn probe(&self, label: &str) -> Result<WindowProbe, String>;
    /// 적용 서비스 `dock_tab` — [`DockSpawn::spawn`] 이 넘긴 작업 안에서만 부른다.
    fn apply(&self, layout: &LayoutState, req: DockRequest) -> Result<DockOutcome, DockError>;
    /// 단조 시계 — 원점은 구현 몫이고 장부는 차만 본다.
    fn now(&self) -> MonoMs;
}

pub(crate) type DockJob = Box<dyn FnOnce() + Send + 'static>;

/// 부른 스레드 밖에서 돌릴 작업 — 적용 · 옛 세션 끝내기(`spawn`)와 시한(`spawn_after`).
pub(crate) trait DockSpawn: Send + Sync {
    fn spawn(&self, job: DockJob);
    fn spawn_after(&self, after_ms: u64, job: DockJob);
}

// 새 창 자리에 쓸 출처 배율을 어느 창에서 읽나.
enum Source {
    // 지금 끄는 탭(모델 표식)의 창.
    Lease,
    // 그 탭의 창 — 시작 전에 온 놓기도 끌기 데이터로 탭을 안다.
    OwnerOf(ViewId),
    // 부른 창 — 출처 `dragend` 는 출처 웹뷰가 보낸다.
    Caller(WindowLabel),
}

/// 실행기 — 장부 · 레이아웃 상태 · 포트를 쥔다. 작업에 넘기려고 복제한다(전부 공유 핸들이다).
#[derive(Clone)]
pub(crate) struct DockExec {
    dock: Arc<DockState>,
    layout: LayoutState,
    ports: Arc<dyn DockPorts>,
    spawner: Arc<dyn DockSpawn>,
}

impl DockExec {
    pub(crate) fn new(
        dock: Arc<DockState>,
        layout: LayoutState,
        ports: Arc<dyn DockPorts>,
        spawner: Arc<dyn DockSpawn>,
    ) -> Self {
        Self {
            dock,
            layout,
            ports,
            spawner,
        }
    }

    fn tauri(app: &AppHandle, dock: &Arc<DockState>, layout: &LayoutState) -> Self {
        Self::new(
            Arc::clone(dock),
            layout.clone(),
            Arc::new(TauriDock { app: app.clone() }),
            Arc::new(TauriSpawn),
        )
    }

    fn feed(&self, input: Input) -> Step {
        sync::lock(&self.dock.book).step(input)
    }

    // TRD S21-slot-docking §1-3 「시작」
    pub(crate) fn start(
        &self,
        caller: &str,
        drag_id: String,
        view: ViewId,
        hotspot: PointCss,
    ) -> Result<(), String> {
        // 1. 장부가 받나 — 표식 · 깃발을 세우기 전에.
        let refused = {
            let mut book = sync::lock(&self.dock.book);
            book.admits(&drag_id)
                .err()
                .map(|rejection| (rejection, refuse_in_book(&mut book, &drag_id, rejection)))
        };
        if let Some((rejection, refusal)) = refused {
            return Err(self.book_refused(&drag_id, view, rejection, refusal));
        }

        // 2. 확정 — 레이아웃 락 한 번.
        let confirmed = match self.layout.0.lock() {
            Ok(mut mgr) => self.confirm(&mut mgr, caller, view),
            Err(_) => Err("LayoutPoisoned"),
        };
        // 락을 놓은 바로 뒤에 읽는다 — 시험이 이 시계 포트로 이 틈(표식 뒤 · `Start` 앞)에 레이아웃 쓰기를 끼워, 그
        //   쓰기의 계기가 깃발을 보는지(깃발이 락 안에서 섰는지) 잰다.
        let now = self.ports.now();
        let (generation, material) = match confirmed {
            Ok(confirmed) => confirmed,
            Err(word) => return Err(self.refuse(&drag_id, view, word)),
        };

        // 3. 장부.
        let StartMaterial {
            lease,
            view: source_view,
            source_label,
            expect_source,
            single_tab_popout,
            snap,
        } = material;
        let tabs = snap
            .windows
            .iter()
            .find(|w| w.label == source_label)
            .map_or(0, |w| w.tabs.len());
        let (Step { actions, .. }, refusal) = {
            let mut book = sync::lock(&self.dock.book);
            let step = book.step(Input::Start {
                drag_id: drag_id.clone(),
                view_id: source_view,
                hotspot,
                lease,
                generation,
                source_label: source_label.clone(),
                expect_source,
                single_tab_popout,
                snap,
                now,
            });
            let refusal = match step.reply {
                Reply::Rejected(rejection) => {
                    Some((rejection, refuse_in_book(&mut book, &drag_id, rejection)))
                }
                _ => None,
            };
            (step, refusal)
        };
        if let Some((rejection, refusal)) = refusal {
            // 함께 낸 표식 내리기 → 자기 세대 깃발 내리기(거절은 위 장부 락 안에서 적었다).
            self.run(actions, true);
            self.dock.end_generation(generation);
            return Err(self.book_refused(&drag_id, view, rejection, refusal));
        }

        // 4. 실행.
        self.dock.take_errors();
        tracing::info!(
            drag = %drag_id,
            view = %view,
            source = %source_label,
            tabs,
            hidden = single_tab_popout,
            "끌기 시작"
        );
        self.run(actions, true);
        Ok(())
    }

    // 표식을 세우고 같은 락 구간에서 세대 · 시작 재료를 뜬다. `Err` = 거절 사유 낱말(부수 효과 없음).
    fn confirm(
        &self,
        mgr: &mut ViewManager,
        caller: &str,
        view: ViewId,
    ) -> Result<(u64, StartMaterial), &'static str> {
        if mgr.owner_of(view).map(String::as_str) != Some(caller) {
            return Err("TabNotFound");
        }
        let lease = mgr.lease_tab(view).map_err(|e| lease_refusal(&e))?;
        // ★깃발은 표식이 선 뒤 이 락 안에서★ — 락을 놓은 뒤 세우면 그 틈의 레이아웃 쓰기가 계기를 건너뛰어 장부가
        //   옛 배치로 남는다(TRD §1-3 「시작」 2 · N8).
        let generation = self.dock.begin_generation();
        match self.dock.start_material(mgr) {
            Some(material) => Ok((generation, material)),
            // 표식을 방금 세운 같은 구간이라 닿지 않는다 — 닿으면 세운 것을 거둔다.
            None => {
                mgr.release_lease(lease);
                self.dock.end_generation(generation);
                Err("TabNotFound")
            }
        }
    }

    // 장부가 정한 시작 거절의 락 밖 몫 — 로그 · 옛 세션 끝내기 · 웹뷰에 돌려줄 낱말.
    fn book_refused(
        &self,
        drag_id: &str,
        view: ViewId,
        rejection: Rejection,
        refusal: BookRefusal,
    ) -> String {
        match refusal {
            BookRefusal::Duplicate => log_refused(drag_id, view, "duplicate_start"),
            BookRefusal::Recorded(old) => {
                log_refused(drag_id, view, rejection.word());
                if let Some(old) = old {
                    self.supersede(old);
                }
            }
        }
        refusal_word(rejection).to_owned()
    }

    // 실행기가 정한 시작 거절 — 장부에 적고(그 id 로 붙든 끝 신호를 버린다) 사유를 돌려준다.
    fn refuse(&self, drag_id: &str, view: ViewId, word: &'static str) -> String {
        let step = self.feed(Input::Refused {
            drag_id: drag_id.to_owned(),
        });
        self.run(step.actions, true);
        log_refused(drag_id, view, word);
        word.to_owned()
    }

    // 끝 신호를 잃은 옛 세션을 「마지막 점에 놓기」로 끝낸다 — 커서 · 창 사각형을 두 락 밖에서 읽어야 해서 따로
    // 넘긴다(TRD §1-3 「시작」 1 · §7 #12).
    fn supersede(&self, old: String) {
        let exec = self.clone();
        self.spawner.spawn(Box::new(move || {
            let (cursor, rects) = exec.read_world(Source::Lease);
            let now = exec.ports.now();
            let step = exec.feed(Input::Superseded {
                drag_id: old,
                cursor,
                rects,
                now,
            });
            exec.run(step.actions, false);
        }));
    }

    // TRD S21-slot-docking §1-3 「보인 것 보고」
    pub(crate) fn over(&self, label: &str, drag_id: String, report: DragOverReport) {
        let now = self.ports.now();
        let label = label.to_owned();
        let input = match report {
            DragOverReport::Over {
                rev,
                bar_rev,
                displayed,
                shown,
                point,
            } => Input::Over {
                drag_id,
                label,
                report: Report {
                    rev,
                    bar_rev,
                    displayed,
                    shown,
                    point,
                },
                now,
            },
            DragOverReport::Left => Input::Left {
                drag_id,
                label,
                now,
            },
        };
        let step = self.feed(input);
        self.run(step.actions, true);
    }

    pub(crate) fn pointer_seen(&self, drag_id: String) {
        let now = self.ports.now();
        let step = self.feed(Input::PointerSeen { drag_id, now });
        self.run(step.actions, true);
    }

    // TRD S21-slot-docking §1-3 「끝 신호 셋」
    pub(crate) fn drop_at(&self, label: &str, drop: DropReport) {
        let (cursor, rects) = self.read_world(Source::OwnerOf(drop.view_id));
        let now = self.ports.now();
        let report = Report::from(&drop);
        let step = self.feed(Input::Drop {
            drag_id: drop.drag_id,
            view_id: drop.view_id,
            label: label.to_owned(),
            report,
            cursor,
            rects,
            now,
        });
        self.run(step.actions, false);
    }

    pub(crate) fn source_end(&self, caller: &str, drag_id: String, effect: DragEffect) {
        let (cursor, rects) = self.read_world(Source::Caller(caller.to_owned()));
        let now = self.ports.now();
        let step = self.feed(Input::SourceEnd {
            drag_id,
            effect,
            cursor,
            rects,
            now,
        });
        self.run(step.actions, false);
    }

    /// 계기(레이아웃 알림 묶음 끝 · 측정 보고) — 세션 깃발이 섰을 때만 번호를 단 스냅숏을 떠 먹인다. 끄는 탭은
    /// 스냅숏이 모델 표식에서 읽는다(`dock::snap`).
    // TRD S21-slot-docking §1-3 「영역」
    pub(crate) fn snapshot_trigger(&self) {
        if !self.dock.session_live() {
            return;
        }
        let snap = {
            let mgr = sync::lock(&self.layout.0);
            self.dock.snapshot(&mgr)
        };
        let step = self.feed(Input::Snapshot(snap));
        self.run(step.actions, false);
    }

    /// 영역 없이 끌기를 만난 웹뷰가 당긴다 — 세션이면 새 스냅숏을 먹이고, 같은 장부 락 안에서 그 창 몫을 당긴다.
    pub(crate) fn pull(&self, label: &str) -> Option<PullReply> {
        let snap = self.dock.session_live().then(|| {
            let mgr = sync::lock(&self.layout.0);
            self.dock.snapshot(&mgr)
        });
        let (actions, reply) = {
            let mut book = sync::lock(&self.dock.book);
            let mut actions = match snap {
                Some(snap) => book.step(Input::Snapshot(snap)).actions,
                None => Vec::new(),
            };
            let pulled = book.step(Input::Pull {
                label: label.to_owned(),
            });
            actions.extend(pulled.actions);
            (actions, pulled.reply)
        };
        self.run(actions, true);
        match reply {
            Reply::Pulled(pulled) => pulled,
            _ => None,
        }
    }

    /// 우리 창 하나가 사라졌다 — 그 창의 레이아웃 정리가 레이아웃 락을 놓은 뒤에 부른다.
    pub(crate) fn window_destroyed(&self, label: &str) {
        if !self.dock.session_live() {
            return;
        }
        let step = self.feed(Input::Destroyed {
            label: label.to_owned(),
        });
        self.run(step.actions, true);
    }

    // 끝 신호 순간의 커서 · 창 사각형 — ★두 락 밖에서 읽는다★. 출처 창 이름만 먼저 레이아웃 락을 잠깐 잡아 읽고
    // 놓는다.
    fn read_world(&self, source: Source) -> (Option<PhysPoint>, Rects) {
        let source = match source {
            Source::Lease => {
                let mgr = sync::lock(&self.layout.0);
                mgr.lease()
                    .and_then(|(_, view)| mgr.owner_of(view).cloned())
            }
            Source::OwnerOf(view) => sync::lock(&self.layout.0).owner_of(view).cloned(),
            Source::Caller(label) => Some(label),
        };
        let cursor = match self.ports.cursor() {
            Ok(point) => Some(point),
            Err(e) => {
                self.note_failure("cursor", None, &e);
                None
            }
        };
        let mut probes = Vec::new();
        for label in self.ports.windows() {
            match self.ports.probe(&label) {
                Ok(probe) => probes.push((label, probe)),
                Err(e) => self.note_failure("window_rect", Some(&label), &e),
            }
        }
        (cursor, client_rects(&probes, source.as_deref()))
    }

    // 장부가 낸 할 일을 낸 순서대로 — ★두 락 밖에서★. `from_sync` = 먹인 스레드가 동기 명령이었나(적용 넘김
    // 로그 몫).
    fn run(&self, actions: Vec<Action>, from_sync: bool) {
        for action in actions {
            match action {
                Action::Broadcast(event) => {
                    if let Err(e) = self.ports.broadcast(&event) {
                        self.note_failure("broadcast", None, &e);
                    }
                }
                Action::Regions { label, regions } => {
                    if let Err(e) = self.ports.send_regions(&label, &regions) {
                        self.note_failure("regions", Some(&label), &e);
                    }
                }
                Action::Hide(label) => {
                    if let Err(e) = self.ports.hide(&label) {
                        tracing::warn!(label = %label, "끌기 출처 창을 숨기지 못했다 — 보인 채 끌기를 잇는다: {e}");
                    }
                }
                Action::Show(label) => {
                    if let Err(e) = self.ports.show(&label) {
                        self.note_failure("show", Some(&label), &e);
                    }
                }
                Action::Focus(label) => {
                    if let Err(e) = self.ports.focus(&label) {
                        self.note_failure("focus", Some(&label), &e);
                    }
                }
                Action::Apply(req) => self.hand_off(req, from_sync),
                Action::ArmTimer {
                    kind,
                    number,
                    after_ms,
                } => self.arm(kind, number, after_ms),
                Action::ReleaseLease(lease) => {
                    sync::lock(&self.layout.0).release_lease(lease);
                }
                Action::EndLog(log) => self.end_log(log),
            }
        }
    }

    // ★적용은 늘 넘긴다★(모듈 머리) — 결과는 그 작업에서 정확히 한 번 먹인다. 적용 중에는 시한이 없어 빠뜨리면 세션이
    //   표식째 남는다(`Input::Applied`).
    // TRD S21-slot-docking §1-3 「명령의 실행 스레드」 · §11-2 #23
    fn hand_off(&self, req: DockRequest, from_sync: bool) {
        let drag = sync::lock(&self.dock.book)
            .live_drag_id()
            .map(str::to_owned);
        tracing::debug!(drag = ?drag, view = %req.view, from_sync, "적용을 async 로 넘긴다");
        let exec = self.clone();
        self.spawner.spawn(Box::new(move || {
            let result = exec.ports.apply(&exec.layout, req);
            let step = exec.feed(Input::Applied(result));
            exec.run(step.actions, false);
        }));
    }

    // 시한 — `kind` · `number` 를 낸 그대로 되싣는다(어긋나면 장부가 버리고 세션이 끝을 잃는다).
    fn arm(&self, kind: TimerKind, number: u64, after_ms: u64) {
        let exec = self.clone();
        self.spawner.spawn_after(
            after_ms,
            Box::new(move || {
                let (cursor, rects) = match kind {
                    // 붙든 이른 신호를 버리기만 한다 — 커서 · 사각형을 쓰지 않는다.
                    TimerKind::Early => (None, Rects::default()),
                    TimerKind::Drop | TimerKind::Grace => exec.read_world(Source::Lease),
                };
                let now = exec.ports.now();
                let step = exec.feed(Input::Timeout {
                    kind,
                    number,
                    cursor,
                    rects,
                    now,
                });
                exec.run(step.actions, false);
            }),
        );
    }

    // TRD S21-slot-docking §1-3 「끝」 · §1-7 「장부 끝」
    fn end_log(&self, log: EndLog) {
        // 자기 세대일 때만 내린다 — 그 사이 선 다음 세션의 깃발은 그대로 둔다.
        if !self.dock.end_generation(log.generation) {
            tracing::debug!(drag = %log.drag_id, generation = log.generation, "세션 깃발이 이미 다른 세대다 — 내리지 않는다");
        }
        let elapsed_ms = self.ports.now().since(log.started_at);
        // 필드 식은 그 레벨이 켜졌을 때만 돈다 — 셈 비우기를 매크로 안에 두면 기본(warn)에서 비워지지 않는다.
        let errors = self.dock.take_errors();
        tracing::info!(
            drag = %log.drag_id,
            outcome = log.outcome.word(),
            reason = log.reason.as_ref().map(EndReason::word),
            end_by = log.end_by.map(|e| e.word()),
            mode = log.mode.map(|m| m.word()),
            window = log.window.as_deref(),
            over_window = log.over_window.as_deref(),
            leave_gap_ms = log.leave_gap_ms,
            cursor_read = log.cursor_read,
            early = log.early,
            reports = log.reports,
            regions_sent = log.regions_sent,
            regions_resent = log.regions_resent,
            errors,
            elapsed_ms,
            "끌기 끝"
        );
    }

    // 세션당 첫 실패만 warn — 끄는 동안 되풀이되는 것은 끝 줄 `errors` 로 센다(TRD §1-7).
    fn note_failure(&self, op: &'static str, label: Option<&str>, e: &str) {
        if self.dock.note_error() {
            tracing::warn!(op, label, "끌기 실행기의 창 호출이 실패했다(이 세션 처음 — 나머지는 끝 줄 errors 로 센다): {e}");
        } else {
            tracing::debug!(op, label, "끌기 실행기의 창 호출이 실패했다: {e}");
        }
    }
}

// 장부가 거절한 시작의 뒷일 — 거절한 그 장부 락 안에서 정한다(락을 놓은 뒤 옛 세션 id 를 읽으면 그 사이 선 다른
// 세션을 끝낼 수 있다).
enum BookRefusal {
    // 살아 있는 그 끌기의 시작이 또 왔다(웹뷰의 겹친 invoke).
    Duplicate,
    // 그 id 를 끝난 id 로 적었다. `Some` = 끝 신호를 잃은 옛 세션 — 끝낸다.
    Recorded(Option<String>),
}

// ★살아 있는 id 의 시작이면 적지도 끝내지도 않는다★ — 적으면 그 id 가 끝난 id 가 되고, 옛 세션으로 끝내면 사용자가
//   끄는 중인 바로 그 끌기가 마지막 점에서 새 창으로 끝나 진짜 놓기가 늦은 신호로 버려진다. 적용 중인 옛 세션은
//   끝내지 않는다 — 그 끝은 `Applied` 가 정한다(TRD §1-3 「시작」 1 · N2).
fn refuse_in_book(book: &mut Book, drag_id: &str, rejection: Rejection) -> BookRefusal {
    let live = book.live_drag_id().map(str::to_owned);
    if live.as_deref() == Some(drag_id) {
        return BookRefusal::Duplicate;
    }
    // `Refused` 는 할 일을 내지 않는다.
    book.step(Input::Refused {
        drag_id: drag_id.to_owned(),
    });
    BookRefusal::Recorded(match rejection {
        Rejection::AlreadyDragging { applying: false } => live,
        _ => None,
    })
}

// 웹뷰에 돌려줄 거절 낱말(TRD §1-5) — 적용 중인 옛 세션도 `AlreadyDragging` 이다(로그의 `reason` 만 가른다).
fn refusal_word(rejection: Rejection) -> &'static str {
    match rejection {
        Rejection::AlreadyDragging { .. } => "AlreadyDragging",
        Rejection::LateStart => "LateStart",
    }
}

fn lease_refusal(e: &DockError) -> &'static str {
    EndReason::Refused(e.clone()).word()
}

fn log_refused(drag_id: &str, view: ViewId, reason: &str) {
    tracing::debug!(drag = drag_id, view = %view, reason, "끌기 시작 거절");
}

/// 게터 값을 장부의 창 사각형으로 — 보이고 최소화 안 된 창만 싣고, 출처 배율은 숨긴 출처에서도 읽는다.
fn client_rects(probes: &[(WindowLabel, WindowProbe)], source: Option<&str>) -> Rects {
    let mut rects = Rects::default();
    for (label, probe) in probes {
        if Some(label.as_str()) == source {
            rects.source_scale = Some(probe.scale);
        }
        if probe.visible && !probe.minimized {
            rects.windows.insert(
                label.clone(),
                ClientRect {
                    origin: probe.origin,
                    size: probe.size,
                    scale: probe.scale,
                },
            );
        }
    }
    rects
}

// 단조 시계의 원점 — 처음 읽을 때 선다.
fn mono_now() -> MonoMs {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let elapsed = ORIGIN.get_or_init(Instant::now).elapsed().as_millis();
    MonoMs(u64::try_from(elapsed).unwrap_or(u64::MAX))
}

// ── 포트의 Tauri 어댑터 ──────────────────────────────────────────────────────

struct TauriDock {
    app: AppHandle,
}

impl TauriDock {
    fn window(&self, label: &str) -> Result<WebviewWindow, String> {
        self.app
            .get_webview_window(label)
            .ok_or_else(|| format!("창이 없다: {label}"))
    }
}

impl DockPorts for TauriDock {
    fn broadcast(&self, event: &SessionEvent) -> Result<(), String> {
        self.app.emit(EVT_SESSION, event).map_err(|e| e.to_string())
    }

    fn send_regions(&self, label: &str, regions: &DockRegions) -> Result<(), String> {
        self.app
            .emit_to(label, EVT_REGIONS, regions)
            .map_err(|e| e.to_string())
    }

    fn hide(&self, label: &str) -> Result<(), String> {
        self.window(label)?.hide().map_err(|e| e.to_string())
    }

    fn show(&self, label: &str) -> Result<(), String> {
        let window = self.window(label)?;
        window.show().map_err(|e| e.to_string())?;
        crate::state::placement::apply_deferred_maximize(&window.as_ref().window());
        Ok(())
    }

    fn focus(&self, label: &str) -> Result<(), String> {
        self.window(label)?.set_focus().map_err(|e| e.to_string())
    }

    fn cursor(&self) -> Result<PhysPoint, String> {
        self.app
            .cursor_position()
            .map(|p| PhysPoint { x: p.x, y: p.y })
            .map_err(|e| e.to_string())
    }

    fn windows(&self) -> Vec<WindowLabel> {
        self.app.webview_windows().into_keys().collect()
    }

    fn probe(&self, label: &str) -> Result<WindowProbe, String> {
        let window = self.window(label)?;
        let text = |e: tauri::Error| e.to_string();
        let origin = window.inner_position().map_err(text)?;
        let size = window.inner_size().map_err(text)?;
        Ok(WindowProbe {
            visible: window.is_visible().map_err(text)?,
            minimized: window.is_minimized().map_err(text)?,
            origin: (origin.x, origin.y),
            size: (size.width, size.height),
            scale: window.scale_factor().map_err(text)?,
        })
    }

    fn apply(&self, layout: &LayoutState, req: DockRequest) -> Result<DockOutcome, DockError> {
        // 빌더 · setup 이 등록한다 — 데몬 클라이언트를 못 만든 셸은 다른 레이아웃 명령처럼 도킹도 실패한다.
        let (Some(router), Some(client), Some(labels)) = (
            self.app.try_state::<Arc<OutputRouter>>(),
            self.app.try_state::<Arc<DaemonClient>>(),
            self.app.try_state::<Arc<PopupCounter>>(),
        ) else {
            return Err(DockError::WindowOpen(
                "도킹에 쓸 셸 상태(출력 라우터 · 데몬 클라이언트 · 창 label 발급기)가 등록돼 있지 않다"
                    .to_owned(),
            ));
        };
        apply::dock_tab(
            layout,
            &RouterSubs {
                router: &router,
                client: &client,
            },
            &TauriEvents { app: &self.app },
            &TauriWindowHost { app: &self.app },
            &**labels,
            req,
        )
    }

    fn now(&self) -> MonoMs {
        mono_now()
    }
}

struct TauriSpawn;

impl DockSpawn for TauriSpawn {
    // 적용은 창 만들기를 기다리며 막힐 수 있어 막혀도 되는 풀에서 돈다.
    fn spawn(&self, job: DockJob) {
        tauri::async_runtime::spawn_blocking(job);
    }

    fn spawn_after(&self, after_ms: u64, job: DockJob) {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(after_ms)).await;
            tauri::async_runtime::spawn_blocking(job);
        });
    }
}

/// 레이아웃 알림 묶음 끝(`TauriEvents::settled`) · 측정 보고 뒤의 계기 — 세션 깃발이 섰을 때만 스냅숏을 먹인다.
pub(crate) fn on_layout_changed(app: &AppHandle) {
    let Some(dock) = app.try_state::<Arc<DockState>>() else {
        return;
    };
    if !dock.session_live() {
        return;
    }
    if let Some(layout) = app.try_state::<LayoutState>() {
        DockExec::tauri(app, dock.inner(), layout.inner()).snapshot_trigger();
    }
}

/// 창 `Destroyed` — 그 창의 레이아웃 정리가 레이아웃 락을 놓은 뒤에 부른다(`lib.rs`).
pub(crate) fn on_window_destroyed(app: &AppHandle, label: &str) {
    let Some(dock) = app.try_state::<Arc<DockState>>() else {
        return;
    };
    if !dock.session_live() {
        return;
    }
    if let Some(layout) = app.try_state::<LayoutState>() {
        DockExec::tauri(app, dock.inner(), layout.inner()).window_destroyed(label);
    }
}

// ── invoke 껍데기(TRD §1-5) ─────────────────────────────────────────────────
//
// 창 label 은 인자로 받지 않는다 — Tauri 가 넣어 주는 [`Window`] 가 그 값의 유일한 권위다(`report_window_canvas` 와
// 같은 규칙). 인자 이름은 snake_case 그대로 받는다(`rename_all`) — 웹뷰는 `dock_drop` 에 `DropReport` 를 그대로 넘긴다.

/// 끌기 시작(동기 · `dragstart` 디스패치 뒤) — `invoke('dock_drag_start', { drag_id, view_id, hotspot })`.
/// `hotspot` = 끌기 그림 안의 커서 자리(CSS px). `Err` = 거절 낱말 `TabNotFound` · `AlreadyDragging` · `LateStart` ·
/// `Dragging` · `LayoutPoisoned` — 그 끌기의 놓기는 아무것도 바꾸지 않는다.
#[tauri::command(rename_all = "snake_case")]
pub fn dock_drag_start(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    drag_id: String,
    view_id: Uuid,
    hotspot: PointCss,
) -> Result<(), String> {
    DockExec::tauri(&app, dock.inner(), layout.inner()).start(
        window.label(),
        drag_id,
        view_id,
        hotspot,
    )
}

/// 대상 창이 보인 것(동기) — `invoke('dock_drag_over', { drag_id, report })`. 모르는 `drag_id` 는 장부가 버린다.
#[tauri::command(rename_all = "snake_case")]
pub fn dock_drag_over(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    drag_id: String,
    report: DragOverReport,
) {
    DockExec::tauri(&app, dock.inner(), layout.inner()).over(window.label(), drag_id, report);
}

/// 대상 창의 놓기(async — 새 창을 만들 수 있다) — `invoke('dock_drop', dropReport)`(칸 = [`DropReport`]). 늘 `Ok` 다.
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn dock_drop(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    drag_id: String,
    view_id: Uuid,
    rev: u64,
    bar_rev: u64,
    displayed: Option<Uuid>,
    shown: Picked,
    point: PointCss,
) -> Result<(), String> {
    let drop = DropReport {
        drag_id,
        view_id,
        rev,
        bar_rev,
        displayed,
        shown,
        point,
    };
    DockExec::tauri(&app, dock.inner(), layout.inner()).drop_at(window.label(), drop);
    Ok(())
}

/// 출처의 `dragend`(async — 새 창을 만들 수 있다) — `invoke('dock_drag_end', { drag_id, effect })`. 늘 `Ok` 다.
#[tauri::command(rename_all = "snake_case")]
pub async fn dock_drag_end(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    drag_id: String,
    effect: DragEffect,
) -> Result<(), String> {
    DockExec::tauri(&app, dock.inner(), layout.inner()).source_end(window.label(), drag_id, effect);
    Ok(())
}

/// 끝난 뒤의 포인터 이동 · 누름(동기 · 세션마다 창마다 한 번) — `invoke('dock_pointer_seen', { drag_id })`.
#[tauri::command(rename_all = "snake_case")]
pub fn dock_pointer_seen(
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    drag_id: String,
) {
    DockExec::tauri(&app, dock.inner(), layout.inner()).pointer_seen(drag_id);
}

/// 지금 세션과 부른 창의 영역(동기) — `invoke('dock_pull')`. `null` = 세션이 없다.
#[tauri::command]
pub fn dock_pull(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
) -> Option<DockPullReply> {
    DockExec::tauri(&app, dock.inner(), layout.inner())
        .pull(window.label())
        .map(|p| DockPullReply {
            started: p.started,
            regions: p.regions,
        })
}

/// 창 웹뷰가 탭바 · 탭 머리 · 캔버스 원점을 알린다(동기 · 측정) — `invoke('report_dock_chrome', { chrome })`. 계약은
/// [`DockChrome`] 문서. 탭 모델에 없는 창이면 `Err` 이고 직전 값을 유지한다. 버스 명령이 아니다(ADR-0227 — 측정은
/// 위조할 수 없게 웹뷰만 보고한다).
#[tauri::command]
pub fn report_dock_chrome(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    chrome: DockChrome,
) -> Result<(), String> {
    apply::report_dock_chrome(&layout, window.label(), chrome)?;
    DockExec::tauri(&app, dock.inner(), layout.inner()).snapshot_trigger();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard};

    use super::*;
    use crate::dock::book::{Outcome, DROP_WAIT_MS, EARLY_SIGNAL_MS, END_GRACE_MS};
    use crate::dock::types::SessionPhase;
    use crate::layout::{
        DockDest, LayoutEvents, SubscriptionSync, ViewSnapshot, WindowTabsPayload,
        MAIN_WINDOW_LABEL as MAIN,
    };

    const P1: &str = "slot-popup-1";
    const NEW_WINDOW: &str = "slot-popup-9";
    const HOTSPOT: PointCss = PointCss { x: 20.0, y: 10.0 };
    const CURSOR: PhysPoint = PhysPoint {
        x: 1000.0,
        y: 800.0,
    };

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Broadcast(SessionPhase),
        Regions(String),
        Hide(String),
        Show(String),
        Focus(String),
        Cursor,
        Probe(String),
        Apply(DockRequest),
    }

    type Hook = Box<dyn FnOnce() + Send>;

    // 포트 호출을 적고, 불릴 때마다 두 락이 다 풀려 있는지 본다(모듈 머리 「락 규율」).
    struct Fake {
        dock: Arc<DockState>,
        layout: LayoutState,
        calls: Mutex<Vec<Call>>,
        // 보낸 영역 그대로(창, 영역) — 보낸 순서.
        regions: Mutex<Vec<(String, DockRegions)>>,
        cursor: Mutex<Result<PhysPoint, String>>,
        probes: Mutex<Vec<(String, Result<WindowProbe, String>)>>,
        outcome: Mutex<Result<DockOutcome, DockError>>,
        clock: AtomicU64,
        // 다음 `now` 한 번에 먼저 돌린다 — 시작의 레이아웃 락 뒤 · `Start` 앞의 틈을 흉내 낸다.
        on_now: Mutex<Option<Hook>>,
    }

    impl Fake {
        fn check(&self, what: &str) {
            assert!(
                self.layout.0.try_lock().is_ok(),
                "{what}: 레이아웃 락을 쥔 채 불렸다"
            );
            assert!(
                self.dock.book.try_lock().is_ok(),
                "{what}: 장부 락을 쥔 채 불렸다"
            );
        }

        fn record(&self, call: Call) {
            self.check(&format!("{call:?}"));
            self.calls.lock().unwrap().push(call);
        }
    }

    impl DockPorts for Fake {
        fn broadcast(&self, event: &SessionEvent) -> Result<(), String> {
            self.record(Call::Broadcast(event.phase.clone()));
            Ok(())
        }

        fn send_regions(&self, label: &str, regions: &DockRegions) -> Result<(), String> {
            self.record(Call::Regions(label.to_owned()));
            self.regions
                .lock()
                .unwrap()
                .push((label.to_owned(), regions.clone()));
            Ok(())
        }

        fn hide(&self, label: &str) -> Result<(), String> {
            self.record(Call::Hide(label.to_owned()));
            Ok(())
        }

        fn show(&self, label: &str) -> Result<(), String> {
            self.record(Call::Show(label.to_owned()));
            Ok(())
        }

        fn focus(&self, label: &str) -> Result<(), String> {
            self.record(Call::Focus(label.to_owned()));
            Ok(())
        }

        fn cursor(&self) -> Result<PhysPoint, String> {
            self.record(Call::Cursor);
            self.cursor.lock().unwrap().clone()
        }

        fn windows(&self) -> Vec<WindowLabel> {
            self.check("windows");
            self.probes
                .lock()
                .unwrap()
                .iter()
                .map(|(label, _)| label.clone())
                .collect()
        }

        fn probe(&self, label: &str) -> Result<WindowProbe, String> {
            self.record(Call::Probe(label.to_owned()));
            self.probes
                .lock()
                .unwrap()
                .iter()
                .find(|(l, _)| l == label)
                .map_or_else(|| Err("창이 없다".to_owned()), |(_, probe)| probe.clone())
        }

        fn apply(&self, _: &LayoutState, req: DockRequest) -> Result<DockOutcome, DockError> {
            self.record(Call::Apply(req));
            self.outcome.lock().unwrap().clone()
        }

        fn now(&self) -> MonoMs {
            let hook = self.on_now.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
            MonoMs(self.clock.fetch_add(10, Ordering::SeqCst))
        }
    }

    // 넘긴 작업을 줄에 세워 두기만 한다 — 시험이 언제 돌릴지 고른다.
    #[derive(Default)]
    struct FakeSpawn {
        jobs: Mutex<VecDeque<(Option<u64>, DockJob)>>,
    }

    impl DockSpawn for FakeSpawn {
        fn spawn(&self, job: DockJob) {
            self.jobs.lock().unwrap().push_back((None, job));
        }

        fn spawn_after(&self, after_ms: u64, job: DockJob) {
            self.jobs.lock().unwrap().push_back((Some(after_ms), job));
        }
    }

    impl FakeSpawn {
        fn pending(&self) -> Vec<Option<u64>> {
            self.jobs
                .lock()
                .unwrap()
                .iter()
                .map(|(after, _)| *after)
                .collect()
        }

        fn take(&self, after: Option<u64>) -> Option<DockJob> {
            let mut jobs = self.jobs.lock().unwrap();
            let at = jobs.iter().position(|(a, _)| *a == after)?;
            jobs.remove(at).map(|(_, job)| job)
        }

        // 바로 도는 작업(적용 · 옛 세션 끝내기)을 그것이 넘긴 것까지 다 돌린다 — 시한은 남긴다. 돌린 수.
        fn run_now(&self) -> usize {
            let mut ran = 0;
            while let Some(job) = self.take(None) {
                job();
                ran += 1;
            }
            ran
        }

        fn fire(&self, after_ms: u64) {
            let job = self
                .take(Some(after_ms))
                .unwrap_or_else(|| panic!("{after_ms}ms 시한이 걸려 있지 않다"));
            job();
        }
    }

    struct Rig {
        fake: Arc<Fake>,
        spawn: Arc<FakeSpawn>,
        exec: DockExec,
    }

    impl Rig {
        fn new(mgr: ViewManager) -> Self {
            let dock = Arc::new(DockState::new());
            let layout = LayoutState(Arc::new(Mutex::new(mgr)));
            let fake = Arc::new(Fake {
                dock: Arc::clone(&dock),
                layout: layout.clone(),
                calls: Mutex::new(Vec::new()),
                regions: Mutex::new(Vec::new()),
                cursor: Mutex::new(Ok(CURSOR)),
                probes: Mutex::new(vec![(MAIN.to_owned(), Ok(main_probe()))]),
                outcome: Mutex::new(Ok(DockOutcome {
                    window: NEW_WINDOW.to_owned(),
                    slot_ids: Vec::new(),
                    closed_window: None,
                    reused: false,
                })),
                clock: AtomicU64::new(1_000),
                on_now: Mutex::new(None),
            });
            let spawn = Arc::new(FakeSpawn::default());
            let exec = DockExec::new(dock, layout, fake.clone(), spawn.clone());
            Self { fake, spawn, exec }
        }

        fn dock(&self) -> &DockState {
            &self.fake.dock
        }

        fn mgr(&self) -> MutexGuard<'_, ViewManager> {
            self.fake.layout.0.lock().unwrap()
        }

        fn lease(&self) -> Option<ViewId> {
            self.mgr().lease().map(|(_, view)| view)
        }

        fn calls(&self) -> Vec<Call> {
            self.fake.calls.lock().unwrap().clone()
        }

        fn applies(&self) -> Vec<DockRequest> {
            self.calls()
                .into_iter()
                .filter_map(|call| match call {
                    Call::Apply(req) => Some(req),
                    _ => None,
                })
                .collect()
        }

        fn last_regions(&self, label: &str) -> Option<DockRegions> {
            self.fake
                .regions
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(l, _)| l == label)
                .map(|(_, regions)| regions.clone())
        }

        fn regions_sent(&self) -> usize {
            self.calls()
                .iter()
                .filter(|call| matches!(call, Call::Regions(_)))
                .count()
        }
    }

    // main — 보이고 · 안쪽 원점 (100, 50) · 배율 1.5.
    fn main_probe() -> WindowProbe {
        WindowProbe {
            visible: true,
            minimized: false,
            origin: (100, 50),
            size: (800, 600),
            scale: 1.5,
        }
    }

    // main 에 탭 둘(a · b) — 끄는 탭을 떼어도 main 에 보일 탭이 남는다.
    fn two_tabs() -> (ViewManager, ViewId, ViewId) {
        let mut mgr = ViewManager::new();
        let a = mgr.windows[MAIN].active;
        let b = mgr.create_tab(MAIN, None).unwrap();
        (mgr, a, b)
    }

    // 탭 하나뿐인 팝아웃 P1 — 끄는 동안 숨는다. 게터는 숨은 채 배율 2 를 돌려준다.
    fn lone_popout() -> (Rig, ViewId) {
        let mut mgr = ViewManager::new();
        let lone = mgr.create_window(P1).unwrap();
        let rig = Rig::new(mgr);
        rig.fake.probes.lock().unwrap().push((
            P1.to_owned(),
            Ok(WindowProbe {
                visible: false,
                scale: 2.0,
                ..main_probe()
            }),
        ));
        (rig, lone)
    }

    // 영역을 받기 전의 놓기 — 가이드 없음 · rev 0.
    fn unguided_drop(drag: &str, view: ViewId, point: PointCss) -> DropReport {
        DropReport {
            drag_id: drag.to_owned(),
            view_id: view,
            rev: 0,
            bar_rev: 0,
            displayed: None,
            shown: Picked::None,
            point,
        }
    }

    fn new_window_pos(req: &DockRequest) -> Option<(i32, i32)> {
        match &req.to {
            DockDest::NewWindow { at } => at.pos,
            other => panic!("새 창 적용이 아니다: {other:?}"),
        }
    }

    fn position(calls: &[Call], pick: impl Fn(&Call) -> bool) -> usize {
        calls
            .iter()
            .position(pick)
            .unwrap_or_else(|| panic!("그 호출이 없다: {calls:?}"))
    }

    fn end_log(generation: u64) -> EndLog {
        EndLog {
            drag_id: "d".to_owned(),
            generation,
            outcome: Outcome::Noop,
            reason: None,
            end_by: None,
            mode: None,
            window: None,
            over_window: None,
            leave_gap_ms: None,
            cursor_read: None,
            early: false,
            reports: 0,
            regions_sent: 0,
            regions_resent: 0,
            started_at: MonoMs(0),
        }
    }

    struct NoSubs;

    impl SubscriptionSync for NoSubs {
        fn resync(&self, _: &ViewManager) {}
    }

    // `TauriEvents::settled` 의 거울 — 알림 묶음 끝에 실행기 계기를 단다.
    struct SettledTrigger<'a> {
        exec: &'a DockExec,
        notices: Mutex<usize>,
    }

    impl LayoutEvents for SettledTrigger<'_> {
        fn layout_updated(&self, _: &ViewSnapshot) {
            *self.notices.lock().unwrap() += 1;
        }

        fn window_tabs_updated(&self, _: &WindowTabsPayload) {
            *self.notices.lock().unwrap() += 1;
        }

        fn settled(&self) {
            self.exec.snapshot_trigger();
        }
    }

    #[test]
    fn rects_carry_only_shown_windows_and_the_source_scale_even_when_hidden() {
        let shown = WindowProbe {
            visible: true,
            minimized: false,
            origin: (10, 20),
            size: (300, 200),
            scale: 1.25,
        };
        let probes = vec![
            (MAIN.to_owned(), shown),
            (
                P1.to_owned(),
                WindowProbe {
                    visible: false,
                    scale: 2.0,
                    ..shown
                },
            ),
            (
                "slot-popup-2".to_owned(),
                WindowProbe {
                    minimized: true,
                    ..shown
                },
            ),
        ];

        let rects = client_rects(&probes, Some(P1));

        assert_eq!(rects.windows.len(), 1, "숨김 · 최소화 창은 사각형이 없다");
        assert_eq!(
            rects.windows[MAIN],
            ClientRect {
                origin: (10, 20),
                size: (300, 200),
                scale: 1.25
            }
        );
        assert_eq!(rects.source_scale, Some(2.0), "숨긴 출처도 배율은 읽는다");
        assert_eq!(client_rects(&probes, None).source_scale, None);
    }

    #[test]
    fn triggers_without_a_session_take_no_snapshot() {
        let (mgr, _, _) = two_tabs();
        let rig = Rig::new(mgr);

        rig.exec.snapshot_trigger();
        assert_eq!(rig.exec.pull(MAIN), None);
        rig.exec.window_destroyed(P1);

        assert_eq!(
            rig.dock().snapshot(&rig.mgr()).number,
            1,
            "스냅숏 번호를 하나도 쓰지 않았다"
        );
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn the_generation_rises_only_after_the_lease() {
        let (mut mgr, a, b) = two_tabs();
        let other = mgr.lease_tab(b).unwrap();
        let rig = Rig::new(mgr);

        assert_eq!(
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT),
            Err("Dragging".to_owned())
        );
        assert!(
            !rig.dock().session_live(),
            "표식을 못 세운 시작은 깃발을 세우지 않는다"
        );
        assert_eq!(
            rig.exec.start(P1, "d2".into(), a, HOTSPOT),
            Err("TabNotFound".to_owned()),
            "부른 창이 그 탭의 주인이 아니다"
        );
        assert!(!rig.dock().session_live());
        assert!(rig.calls().is_empty() && rig.spawn.pending().is_empty());

        rig.mgr().release_lease(other);
        assert_eq!(rig.exec.start(MAIN, "d3".into(), a, HOTSPOT), Ok(()));
        assert!(rig.dock().session_live());
        assert_eq!(rig.lease(), Some(a));
        assert!(matches!(
            rig.calls().first(),
            Some(Call::Broadcast(SessionPhase::Started(_)))
        ));
    }

    #[test]
    fn a_start_the_book_refuses_after_the_lease_lowers_its_own_generation_and_lease() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        // 표식 · 깃발을 세운 뒤 `Start` 앞에서 이 id 가 끝난 id 로 적힌다(이른 신호 시한 · 밀려남과 같은 자리).
        let dock = Arc::clone(&rig.fake.dock);
        *rig.fake.on_now.lock().unwrap() = Some(Box::new(move || {
            sync::lock(&dock.book).step(Input::Refused {
                drag_id: "d1".into(),
            });
        }));

        assert_eq!(
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT),
            Err("LateStart".to_owned())
        );

        assert_eq!(rig.lease(), None, "함께 낸 표식 내리기를 실행했다");
        assert!(!rig.dock().session_live(), "자기 세대를 내렸다");
        assert!(rig.calls().is_empty(), "거절한 시작은 알리지 않는다");
        assert_eq!(rig.exec.start(MAIN, "d2".into(), a, HOTSPOT), Ok(()));
    }

    #[test]
    fn a_write_between_the_lease_and_start_reaches_the_book() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        // 표식 뒤 · `Start` 앞의 틈에 탭 하나를 더 만든다 — 그 쓰기의 계기가 깃발을 보면 장부가 그 스냅숏을 들고 있다가
        //   `Start` 뒤에 이어 먹는다. 깃발이 락 밖에서 서면 계기가 그 쓰기를 건너뛴다(TRD §1-3 「시작」 2 · N8).
        let created = Arc::new(Mutex::new(None));
        let (exec, slot) = (rig.exec.clone(), Arc::clone(&created));
        *rig.fake.on_now.lock().unwrap() = Some(Box::new(move || {
            let events = SettledTrigger {
                exec: &exec,
                notices: Mutex::new(0),
            };
            let tab = apply::create_tab(&exec.layout, &NoSubs, &events, MAIN, None).unwrap();
            *slot.lock().unwrap() = Some(tab);
        }));

        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();

        let tab = created.lock().unwrap().expect("틈의 쓰기가 돌았다");
        assert_eq!(
            rig.last_regions(MAIN).and_then(|r| r.view_id),
            Some(tab),
            "main 영역이 그 쓰기 뒤의 배치(새 탭이 보임)로 다시 나갔다"
        );
    }

    #[test]
    fn a_repeated_start_of_the_live_drag_leaves_it_running() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();
        let before = rig.calls().len();

        assert_eq!(
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT),
            Err("AlreadyDragging".to_owned())
        );

        assert_eq!(rig.calls().len(), before, "알리지도 창을 읽지도 않는다");
        assert!(rig.spawn.pending().is_empty(), "옛 세션으로 끝내지 않는다");
        assert!(rig.dock().session_live());
        assert_eq!(rig.lease(), Some(a));

        // 그 끌기의 진짜 놓기는 늦은 신호가 아니다.
        rig.exec
            .drop_at(MAIN, unguided_drop("d1", a, PointCss { x: 40.0, y: 20.0 }));
        assert_eq!(rig.spawn.run_now(), 1);
        assert_eq!(rig.applies().len(), 1);
        assert!(!rig.dock().session_live());
    }

    #[test]
    fn an_end_lowers_only_its_own_generation() {
        let rig = Rig::new(ViewManager::new());
        let old = rig.dock().begin_generation();
        let new = rig.dock().begin_generation();

        rig.exec.run(vec![Action::EndLog(end_log(old))], false);
        assert!(
            rig.dock().session_live(),
            "옛 세션의 끝이 다음 세션의 깃발을 내리지 않는다"
        );

        rig.exec.run(vec![Action::EndLog(end_log(new))], false);
        assert!(!rig.dock().session_live());
    }

    #[test]
    fn one_settled_feeds_one_snapshot() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();
        let before = rig.dock().snapshot(&rig.mgr()).number;
        let sent = rig.regions_sent();
        let events = SettledTrigger {
            exec: &rig.exec,
            notices: Mutex::new(0),
        };

        apply::create_tab(&rig.fake.layout, &NoSubs, &events, MAIN, None).unwrap();

        assert!(*events.notices.lock().unwrap() >= 2, "알림이 여럿인 쓰기");
        assert_eq!(
            rig.dock().snapshot(&rig.mgr()).number,
            before + 2,
            "그 쓰기의 스냅숏은 하나"
        );
        assert!(
            rig.regions_sent() > sent,
            "장부가 그 스냅숏으로 main 영역을 다시 보냈다"
        );
    }

    #[test]
    fn a_start_replaying_a_parked_drop_hands_apply_to_the_spawn_port() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        // 시작보다 먼저 온 놓기 — 장부가 붙들고 이른 신호 시한을 건다.
        rig.exec
            .drop_at(MAIN, unguided_drop("d1", a, PointCss { x: 40.0, y: 20.0 }));
        assert_eq!(rig.spawn.pending(), [Some(EARLY_SIGNAL_MS)]);
        assert!(rig.calls().contains(&Call::Cursor));
        assert!(rig.calls().contains(&Call::Probe(MAIN.to_owned())));

        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();

        assert!(
            rig.applies().is_empty(),
            "시작 명령 안에서 적용 서비스를 부르지 않는다"
        );
        assert_eq!(rig.spawn.pending(), [Some(EARLY_SIGNAL_MS), None]);
        assert!(rig.dock().session_live(), "적용 결과 전에는 끝나지 않는다");

        assert_eq!(rig.spawn.run_now(), 1);

        let applies = rig.applies();
        assert_eq!(applies.len(), 1);
        // 새 창 자리 = 놓기 때 커서 − 그림 커서 자리 × 출처 배율 = (1000 − 20 × 1.5, 800 − 10 × 1.5).
        assert_eq!(new_window_pos(&applies[0]), Some((970, 785)));
        let calls = rig.calls();
        let applied = position(&calls, |c| matches!(c, Call::Apply(_)));
        let ended = position(&calls, |c| *c == Call::Broadcast(SessionPhase::Ended));
        assert!(
            applied < ended,
            "끝은 적용 결과를 먹인 그 작업에서 — 적용 뒤에"
        );
        assert!(calls[ended..].contains(&Call::Focus(NEW_WINDOW.to_owned())));
        assert_eq!(rig.lease(), None);
        assert!(!rig.dock().session_live());

        // 남은 이른 신호 시한은 지난 시한으로 버려진다.
        rig.spawn.fire(EARLY_SIGNAL_MS);
        assert_eq!(rig.applies().len(), 1);
    }

    #[test]
    fn an_unreadable_cursor_places_the_new_window_from_the_drop_point() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();
        *rig.fake.cursor.lock().unwrap() = Err("잠금 화면".to_owned());

        rig.exec
            .drop_at(MAIN, unguided_drop("d1", a, PointCss { x: 40.0, y: 20.0 }));
        rig.spawn.run_now();

        // 마지막으로 알려진 점 = 놓은 창 안쪽 원점 + 점 × 배율 = (160, 80) → 그림 커서 자리 × 1.5 를 뺀다.
        assert_eq!(new_window_pos(&rig.applies()[0]), Some((130, 65)));
    }

    #[test]
    fn with_no_cursor_and_no_window_under_the_new_window_has_no_place() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();
        *rig.fake.cursor.lock().unwrap() = Err("잠금 화면".to_owned());

        rig.exec.source_end(MAIN, "d1".into(), DragEffect::None);
        rig.spawn.run_now();

        assert_eq!(new_window_pos(&rig.applies()[0]), None);
    }

    #[test]
    fn a_start_over_a_live_session_refuses_and_ends_the_old_one_off_the_command() {
        let (mgr, a, b) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();
        let before = rig.calls().len();

        assert_eq!(
            rig.exec.start(MAIN, "d2".into(), b, HOTSPOT),
            Err("AlreadyDragging".to_owned())
        );
        assert_eq!(
            rig.calls().len(),
            before,
            "거절한 명령 안에서는 창을 읽지도 알리지도 않는다"
        );
        assert_eq!(rig.spawn.pending(), [None]);

        assert_eq!(rig.spawn.run_now(), 2, "옛 세션 끝내기 → 그것이 낸 적용");
        assert_eq!(new_window_pos(&rig.applies()[0]), Some((970, 785)));
        assert_eq!(rig.lease(), None);
        assert!(!rig.dock().session_live());
        assert_eq!(
            rig.exec.start(MAIN, "d2".into(), b, HOTSPOT),
            Err("LateStart".to_owned()),
            "거절한 id 는 끝난 id 다"
        );
        assert_eq!(rig.exec.start(MAIN, "d3".into(), b, HOTSPOT), Ok(()));
    }

    #[test]
    fn the_drop_wait_timer_comes_back_with_its_kind_and_number_and_ends_the_session() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();

        rig.exec.source_end(MAIN, "d1".into(), DragEffect::Move);
        assert_eq!(rig.spawn.pending(), [Some(DROP_WAIT_MS)]);
        *rig.fake.cursor.lock().unwrap() = Ok(PhysPoint { x: 0.0, y: 0.0 });
        rig.spawn.fire(DROP_WAIT_MS);
        assert_eq!(rig.spawn.run_now(), 1);

        assert_eq!(
            new_window_pos(&rig.applies()[0]),
            Some((970, 785)),
            "끝 신호 때의 커서"
        );
        assert!(!rig.dock().session_live());
    }

    #[test]
    fn the_grace_timer_places_the_new_window_at_the_cursor_it_reads() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT).unwrap();

        rig.exec.pointer_seen("d1".into());
        assert_eq!(rig.spawn.pending(), [Some(END_GRACE_MS)]);
        *rig.fake.cursor.lock().unwrap() = Ok(PhysPoint { x: 500.0, y: 400.0 });
        rig.spawn.fire(END_GRACE_MS);
        rig.spawn.run_now();

        assert_eq!(new_window_pos(&rig.applies()[0]), Some((470, 385)));
        assert!(!rig.dock().session_live());
    }

    #[test]
    fn a_hidden_single_tab_popout_comes_back_when_the_apply_is_refused() {
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() =
            Err(DockError::WindowOpen("이벤트 루프가 닫혔다".to_owned()));
        rig.exec.start(P1, "d1".into(), lone, HOTSPOT).unwrap();
        assert!(rig.calls().contains(&Call::Hide(P1.to_owned())));
        assert!(
            !rig.calls().contains(&Call::Regions(P1.to_owned())),
            "숨긴 출처에는 영역을 보내지 않는다"
        );

        rig.exec.source_end(P1, "d1".into(), DragEffect::None);
        rig.spawn.run_now();

        // 숨긴 출처의 배율 2 — (1000 − 20 × 2, 800 − 10 × 2).
        assert_eq!(new_window_pos(&rig.applies()[0]), Some((960, 780)));
        let calls = rig.calls();
        let applied = position(&calls, |c| matches!(c, Call::Apply(_)));
        assert!(calls[applied..].contains(&Call::Show(P1.to_owned())));
        assert!(
            !calls.iter().any(|c| matches!(c, Call::Focus(_))),
            "실패한 적용은 포커스를 옮기지 않는다"
        );
        assert_eq!(rig.lease(), None);
        assert!(!rig.dock().session_live());
    }

    #[test]
    fn a_destroyed_source_ends_the_session_without_showing_it() {
        let (rig, lone) = lone_popout();
        rig.exec.start(P1, "d1".into(), lone, HOTSPOT).unwrap();

        rig.exec.window_destroyed(P1);

        let calls = rig.calls();
        assert!(calls.contains(&Call::Broadcast(SessionPhase::Ended)));
        assert!(!calls.iter().any(|c| matches!(c, Call::Show(_))));
        assert!(rig.applies().is_empty());
        assert_eq!(rig.lease(), None);
        assert!(!rig.dock().session_live());
    }
}
