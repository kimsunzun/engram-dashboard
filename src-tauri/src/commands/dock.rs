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
//! - 대기 창 명부 락(`DockState::parked`)도 잎이다 — 가드는 문장 하나 안에서 쓰고 놓고, 쥔 채 레이아웃 락 · 장부 락 ·
//!   emit · 창 호출 · 창 짓기를 하지 않는다. 레이아웃 락을 쥔 채 잡는 것은 된다(차례 = `DockState` 문서).
//!
//! ## ★`Apply` 는 늘 spawn★
//! 할 일을 먹인 스레드와 무관하게 적용은 [`DockSpawn::spawn`] 으로 넘기고, 그 결과(`Applied`)와 끝 할 일도 그
//! 작업에서 먹이고 실행한다. 동기 명령(메인 스레드)인 `dock_drag_start` 도 붙든 이른 놓기를 이어 먹여 새 창 적용을
//! 낼 수 있고, 그것을 그 자리에서 돌리면 창을 만들다 교착한다(TRD §1-3 「명령의 실행 스레드」 · §11-2 #23).
//!
//! ## ★숨기기 · 보이기 · 포커스는 메인 스레드 맞추기★
//! `Hide` · `Show` 는 창을 그대로 부르지 않고 메인 스레드 줄([`DockSpawn::on_main`])에 「맞추기」 한 건으로 올린다.
//! 맞추기는 돌 때 장부 락을 잠깐 잡아 [`Book::visibility`] 의 지금 값을 읽고, 락을 놓은 뒤 창이 그 값이 아닐 때만
//! 숨기거나 보인다(TRD §1-3 「숨기기 · 보이기」). 여러 스레드가 낸 창 호출은 이벤트 루프에 장부가 낸 순서로 닿지
//! 않는다 — 보이기 → 숨기기로 닿으면 끝난 세션의 팝아웃이 숨은 채 남는다. 장부 값이 바뀌는 걸음마다 맞추기 한 건이
//! 그 뒤에 올라가고 맞추기는 늘 그때의 장부 값을 입히므로, 마지막으로 돈 맞추기가 장부의 마지막 값이다 — 할 일의
//! 세대는 보지 않는다(지난 세션의 할 일도 그 창의 지금 값으로만 맞춘다). ★되보이기는 맞추기가 스스로 숨긴 창만★ —
//! 트레이 등 다른 길이 숨긴 창은 끌기 끝이 드러내지 않는다(`DockState` 의 「맞추기가 숨긴 창」). 같은 걸음의 `Focus` 는
//! 같은 줄에 그 뒤로 올리고, 돌 때 그 뒤 새 세션이 섰으면 버린다 — 보이기 전에 포커스를 주지 않고, 새 세션이 숨길 창에
//! 옛 끝의 포커스를 주지 않게. 메인 스레드에서 올린 작업은 그 자리에서 돈다(tauri-runtime-wry
//! `send_user_message`) — 할 일 실행이 두 락을 다 놓은 채여야 하는 또 하나의 이유다.
//!
//! 바깥 세계에는 포트 둘([`DockPorts`] · [`DockSpawn`])로만 닿는다 — 시험이 가짜로 갈아 끼운다(ADR-0012).

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;
use tauri::{
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window,
};
use uuid::Uuid;

use crate::commands::layout::{RouterSubs, TauriEvents};
use crate::commands::popout::{PopupCounter, TauriWindowHost};
use crate::daemon_client::DaemonClient;
use crate::dock::book::{
    Action, Book, ClientRect, EndLog, EndReason, Input, MonoMs, PhysPoint, PullReply, Rects,
    Rejection, Reply, Report, Step, TimerKind, Visibility,
};
use crate::dock::parked::{Measure, ParkedRole};
use crate::dock::snap::StartMaterial;
use crate::dock::types::{
    DockPullReply, DockRegions, DragEffect, DragOverReport, DrawnReport, DropReport, Picked,
    SessionEvent, StripStart,
};
use crate::dock::wire::WindowRoleReply;
use crate::dock::DockState;
use crate::layout::apply;
use crate::layout::chrome::{DockChrome, PointCss};
use crate::layout::manager::{DockError, ViewId, WindowLabel};
use crate::layout::{DockOutcome, DockRequest, LabelSource, LayoutState, ViewManager};
use crate::output_router::OutputRouter;
use crate::webview_env::WebviewEnv;

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
    /// 그 창이 지금 보이나 — `None` = 그 창이 없다(맞추기를 올린 뒤 사라졌다). 메인 스레드 맞추기가 부른다.
    fn visible(&self, label: &str) -> Result<Option<bool>, String>;
    /// 적용 서비스 `dock_tab` — [`DockSpawn::spawn`] 이 넘긴 작업 안에서만 부른다.
    fn apply(&self, layout: &LayoutState, req: DockRequest) -> Result<DockOutcome, DockError>;
    /// 단조 시계 — 원점은 구현 몫이고 장부는 차만 본다.
    fn now(&self) -> MonoMs;
}

pub(crate) type DockJob = Box<dyn FnOnce() + Send + 'static>;

/// 부른 스레드 밖에서 돌릴 작업 — 적용 · 옛 세션 끝내기(`spawn`) · 시한(`spawn_after`) · 숨기기 · 보이기 맞추기와
/// 포커스(`on_main`).
pub(crate) trait DockSpawn: Send + Sync {
    fn spawn(&self, job: DockJob);
    fn spawn_after(&self, after_ms: u64, job: DockJob);
    /// 메인 스레드 줄 — 먼저 올린 것이 먼저 돈다. 메인 스레드에서 올리면 그 자리에서 돈다. `Err` = 올리지 못했다
    /// (이벤트 루프가 닫혔다) — 그 작업은 돌지 않는다.
    fn on_main(&self, job: DockJob) -> Result<(), String>;
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
            Arc::new(TauriSpawn { app: app.clone() }),
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
        strip: Option<StripStart>,
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
        // 두 락 밖에서 — 「시한이 앞지른 끝」의 사각형 검사 몫(TRD §1-3 「시작」 2). 보임 · 최소화를 거르지 않는다 —
        //   끌기를 시작한 바로 그 창이다. 실패는 셈을 비운 뒤(4)에 센다.
        let source_probe = self.ports.probe(&source_label);
        let source_rect = source_probe.as_ref().ok().map(|probe| ClientRect {
            origin: probe.origin,
            size: probe.size,
            scale: probe.scale,
        });
        let has_strip = strip.is_some();
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
                strip,
                source_rect,
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
        self.dock.open_errors(generation);
        if let Err(e) = &source_probe {
            self.note_failure("window_rect", Some(&source_label), e);
        }
        // 탭 하나 팝아웃의 숨기기는 출처 영역에 순서 이동이 서면 도킹 모드 증거까지 미룬다 — 시작 할 일에 `Hide` 가 없다.
        let hidden = actions.iter().any(|a| matches!(a, Action::Hide { .. }));
        tracing::info!(
            drag = %drag_id,
            view = %view,
            source = %source_label,
            tabs,
            hidden,
            strip = has_strip,
            hide_deferred = single_tab_popout && !hidden,
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
            let now = exec.ports.now();
            let (cursor, rects) = exec.read_world(Source::Lease);
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
                scroll,
            } => Input::Over {
                drag_id,
                label,
                report: Report {
                    rev,
                    bar_rev,
                    displayed,
                    shown,
                    point,
                    scroll,
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
    // ★끝 신호 · 시한 콜백의 `now` 는 세계를 읽기 전에 찍는다(도착 시각 — 까닭은 `dock::book` 머리)★.
    pub(crate) fn drop_at(&self, label: &str, drop: DropReport) {
        let now = self.ports.now();
        let (cursor, rects) = self.read_world(Source::OwnerOf(drop.view_id));
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

    pub(crate) fn source_end(
        &self,
        caller: &str,
        drag_id: String,
        effect: DragEffect,
        canceled: Option<bool>,
        last: Option<DrawnReport>,
    ) {
        let now = self.ports.now();
        let (cursor, rects) = self.read_world(Source::Caller(caller.to_owned()));
        let step = self.feed(Input::SourceEnd {
            drag_id,
            effect,
            canceled: canceled.unwrap_or(false),
            last,
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
        self.dock.forget_own_hidden(label);
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
            // 대기 창 명부의 창 · 안내 창은 모델 밖이라 이 장부의 대상이 아니다 — 사각형에 넣으면 「끝 커서를 품은 다른
            //   창」으로 세어져 오는 중 `Over` 를 기다린다(TRD S21-slot-docking §13 U4 「옛 끌기 그대로 돎」).
            if is_overlay_label(&label) || self.dock.is_parked(&label) {
                continue;
            }
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
        // 맞추기는 돌 때의 장부 값을 읽으니 한 걸음에 창마다 한 건이면 된다(그 걸음이 끝난 뒤의 값이다).
        let mut reconciling: Vec<WindowLabel> = Vec::new();
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
                Action::Hide { label, generation } | Action::Show { label, generation } => {
                    if !reconciling.contains(&label) {
                        let target = label.clone();
                        self.on_main(&label, generation, move |exec| {
                            exec.reconcile(&target, generation);
                        });
                        reconciling.push(label);
                    }
                }
                Action::Focus { label, generation } => {
                    let target = label.clone();
                    self.on_main(&label, generation, move |exec| {
                        exec.focus(&target, generation);
                    });
                }
                Action::Apply(req) => self.hand_off(req, from_sync),
                Action::ArmTimer {
                    kind,
                    generation,
                    number,
                    after_ms,
                } => self.arm(kind, generation, number, after_ms),
                Action::ReleaseLease(lease) => {
                    sync::lock(&self.layout.0).release_lease(lease);
                }
                Action::EndLog(log) => self.end_log(log),
            }
        }
    }

    // `generation` = 그 작업을 낸 세션 — 실패를 어느 세션 몫으로 칠지(`note_failure_of`).
    fn on_main(&self, label: &str, generation: u64, job: impl FnOnce(&DockExec) + Send + 'static) {
        let exec = self.clone();
        if let Err(e) = self.spawner.on_main(Box::new(move || job(&exec))) {
            self.note_failure_of(generation, "main_thread", Some(label), &e);
        }
    }

    // 메인 스레드에서 돈다(모듈 머리 「맞추기」). 장부 락은 지금 값을 읽는 동안만 쥔다. ★되보이기는 맞추기가 스스로
    //   숨긴 창만★(`DockState::owns_hidden` · 기본값 · 메인 판단) — 장부 값만 보면 트레이 등 다른 길이 숨긴 창을 끌기
    //   끝이 드러낸다. `generation` = 맞추기를 올린 할 일의 세션(로그 · 실패 몫 — 판정은 장부의 지금 값만 본다).
    // TRD S21-slot-docking §1-3 「숨기기 · 보이기」
    fn reconcile(&self, label: &str, generation: u64) {
        let want = sync::lock(&self.dock.book).visibility(label);
        let shown = match self.ports.visible(label) {
            Ok(Some(shown)) => shown,
            Ok(None) => {
                self.dock.forget_own_hidden(label);
                tracing::debug!(label, generation, ?want, "맞출 창이 이미 없다 — 건너뛴다");
                return;
            }
            Err(e) => return self.note_failure_of(generation, "visible", Some(label), &e),
        };
        match (want, shown) {
            // 창이 보인다 — 맞추기가 전에 숨겼어도 그 뒤 다른 길이 되보였으니 그 몫은 끝났다. 숨기기에 성공했을 때만
            //   다시 쥔다 — 실패한 숨기기가 낡은 몫을 남기면 다음 끝이 트레이가 숨긴 창을 드러낸다.
            (Visibility::Hidden, true) => {
                self.dock.forget_own_hidden(label);
                match self.ports.hide(label) {
                    Ok(()) => self.dock.mark_own_hidden(label),
                    Err(e) => tracing::warn!(
                        label,
                        "끌기 출처 창을 숨기지 못했다 — 보인 채 끌기를 잇는다: {e}"
                    ),
                }
            }
            (Visibility::Shown, false) if self.dock.owns_hidden(label) => {
                match self.ports.show(label) {
                    Ok(()) => self.dock.forget_own_hidden(label),
                    Err(e) => self.note_failure_of(generation, "show", Some(label), &e),
                }
            }
            (Visibility::Shown, false) => tracing::debug!(
                label,
                generation,
                "끌기가 숨기지 않은 숨은 창 — 드러내지 않는다"
            ),
            // 같은 값을 낸 다른 맞추기가 먼저 돌았거나 지난 세션의 할 일이다 — 이미 그 값인 창을 다시 부르지 않는다.
            //   macOS 의 보이기는 보이는 창도 늘 키 창으로 올려(tao 0.35 `set_visible` → `make_key_and_order_front`)
            //   끝의 포커스를 뺏는다 — Windows 는 같은 값이면 tao 가 무동작이다.
            _ => {
                if want == Visibility::Shown {
                    // 다른 길이 이미 되보였다 — 되보일 몫이 남지 않는다.
                    self.dock.forget_own_hidden(label);
                }
                tracing::debug!(
                    label,
                    generation,
                    ?want,
                    "창이 이미 장부의 지금 값이다 — 맞출 것이 없다"
                );
            }
        }
    }

    // 끝의 포커스 — 메인 스레드에서 그 끝의 맞추기 뒤에 돈다. 그 뒤 새 세션이 섰거나 그 창을 지금 숨겨 둬야 하면
    //   버린다(새 세션이 숨길 창에 포커스를 주지 않게). 장부 락은 읽는 동안만 쥔다.
    fn focus(&self, label: &str, generation: u64) {
        let (latest, want) = {
            let book = sync::lock(&self.dock.book);
            (book.latest_generation(), book.visibility(label))
        };
        if latest != Some(generation) || want != Visibility::Shown {
            tracing::debug!(
                label,
                generation,
                ?latest,
                ?want,
                "끝의 포커스보다 새 세션이 먼저 섰다 — 포커스를 주지 않는다"
            );
            return;
        }
        if let Err(e) = self.ports.focus(label) {
            self.note_failure_of(generation, "focus", Some(label), &e);
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

    // 시한 — `kind` · `generation` · `number` 를 낸 그대로 되싣는다(어긋나면 장부가 버리고 세션이 끝을 잃는다).
    fn arm(&self, kind: TimerKind, generation: u64, number: u64, after_ms: u64) {
        let exec = self.clone();
        self.spawner.spawn_after(
            after_ms,
            Box::new(move || {
                let now = exec.ports.now();
                let (cursor, rects) = match kind {
                    // 장부가 커서 · 사각형을 쓰지 않는다 — 이른 신호는 버리기만 하고, 떠남 시한은 숨기기만 하며, 붙들기는
                    //   붙든 끝의 것으로 판정한다.
                    TimerKind::Early | TimerKind::Leave | TimerKind::OverWait => {
                        (None, Rects::default())
                    }
                    TimerKind::Drop | TimerKind::Grace => exec.read_world(Source::Lease),
                };
                let step = exec.feed(Input::Timeout {
                    kind,
                    generation,
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
        let errors = self.dock.take_errors(log.generation);
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
            unseen = log.unseen,
            strip = log.strip,
            hide_deferred = log.hide_deferred,
            hidden = log.hidden,
            last = log.last.word(),
            last_gap_ms = log.last_gap_ms,
            tight = log.tight,
            lifted = log.lifted,
            last_differs = log.last_differs,
            late_hide = log.late_hide,
            held_rect = log.held_rect,
            over_wait = log.over_wait.word(),
            over_wait_ms = log.over_wait_ms,
            over_from = log.over_from.as_deref(),
            reports = log.reports,
            regions_sent = log.regions_sent,
            regions_resent = log.regions_resent,
            errors,
            elapsed_ms,
            "끌기 끝"
        );
    }

    // 메인 스레드 작업의 실패 — 낸 세션의 셈이 열려 있을 때만 그 셈에 넣는다(확인과 셈이 한 락 안 —
    //   `DockState::note_error_of`). 그 끝 줄 뒤면 세지 않는다 — 끝 줄은 이미 나갔고, 센 것은 다음 세션 셈에 섞인다
    //   (TRD §1-7). 그때는 그 세션당 처음 하나만 warn.
    fn note_failure_of(&self, generation: u64, op: &'static str, label: Option<&str>, e: &str) {
        match self.dock.note_error_of(generation) {
            Some(first) => log_failure(first, op, label, e),
            None if self.dock.first_late_failure(generation) => {
                tracing::warn!(op, label, generation, "끌기가 끝난 뒤 메인 스레드의 창 호출이 실패했다(그 세션 처음 — 끝 줄 뒤라 errors 에 세지 않는다): {e}");
            }
            None => {
                tracing::debug!(
                    op,
                    label,
                    generation,
                    "끌기가 끝난 뒤 메인 스레드의 창 호출이 실패했다: {e}"
                );
            }
        }
    }

    // 세션당 첫 실패만 warn — 끄는 동안 되풀이되는 것은 끝 줄 `errors` 로 센다(TRD §1-7).
    fn note_failure(&self, op: &'static str, label: Option<&str>, e: &str) {
        log_failure(self.dock.note_error(), op, label, e);
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

fn log_failure(first: bool, op: &'static str, label: Option<&str>, e: &str) {
    if first {
        tracing::warn!(
            op,
            label,
            "끌기 실행기의 창 호출이 실패했다(이 세션 처음 — 나머지는 끝 줄 errors 로 센다): {e}"
        );
    } else {
        tracing::debug!(op, label, "끌기 실행기의 창 호출이 실패했다: {e}");
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

    fn visible(&self, label: &str) -> Result<Option<bool>, String> {
        match self.app.get_webview_window(label) {
            Some(window) => window.is_visible().map(Some).map_err(|e| e.to_string()),
            None => Ok(None),
        }
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

struct TauriSpawn {
    app: AppHandle,
}

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

    fn on_main(&self, job: DockJob) -> Result<(), String> {
        self.app.run_on_main_thread(job).map_err(|e| e.to_string())
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
    if let Some(layout) = app.try_state::<LayoutState>() {
        DockExec::tauri(app, dock.inner(), layout.inner()).window_destroyed(label);
    }
}

// ── invoke 껍데기(TRD §1-5) ─────────────────────────────────────────────────
//
// 창 label 은 인자로 받지 않는다 — Tauri 가 넣어 주는 [`Window`] 가 그 값의 유일한 권위다(`report_window_canvas` 와
// 같은 규칙). 인자 이름은 snake_case 그대로 받는다(`rename_all`) — 웹뷰는 `dock_drop` 에 `DropReport` 를 그대로 넘긴다.

/// 끌기 시작(동기 · `dragstart` 디스패치 뒤) — `invoke('dock_drag_start', { drag_id, view_id, hotspot, strip })`.
/// `hotspot` = 끌기 그림 안의 커서 자리(CSS px) · `strip` = `dragstart` 안에서 잰 탭바([`StripStart`] · 선택 칸 — 없음 =
/// 순서 이동 없음). `Err` = 거절 낱말 `TabNotFound` · `AlreadyDragging` · `LateStart` ·
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
    strip: Option<StripStart>,
) -> Result<(), String> {
    DockExec::tauri(&app, dock.inner(), layout.inner()).start(
        window.label(),
        drag_id,
        view_id,
        hotspot,
        strip,
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

/// 대상 창의 놓기(async — 새 창을 만들 수 있다) — `invoke('dock_drop', dropReport)`(칸 = [`DropReport`] · `scroll` 은
/// 선택 칸). 늘 `Ok` 다.
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
    scroll: Option<f64>,
) -> Result<(), String> {
    let drop = DropReport {
        drag_id,
        view_id,
        rev,
        bar_rev,
        displayed,
        shown,
        point,
        scroll,
    };
    DockExec::tauri(&app, dock.inner(), layout.inner()).drop_at(window.label(), drop);
    Ok(())
}

/// 출처의 `dragend`(async — 새 창을 만들 수 있다) — `invoke('dock_drag_end', { drag_id, effect, canceled, last })`.
/// `canceled` = 엔진이 접은 끌기(선택 칸 — 없음 = 거짓) · `last` = 출처 창이 마지막으로 그린 판정([`DrawnReport`] ·
/// 선택 칸). 늘 `Ok` 다.
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn dock_drag_end(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    drag_id: String,
    effect: DragEffect,
    canceled: Option<bool>,
    last: Option<DrawnReport>,
) -> Result<(), String> {
    DockExec::tauri(&app, dock.inner(), layout.inner()).source_end(
        window.label(),
        drag_id,
        effect,
        canceled,
        last,
    );
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
/// [`DockChrome`] 문서. 대기 창 명부의 창이면 모델 대신 명부에 적는다(TRD §2-5). 탭 모델에도 명부에도 없는 창이면
/// `Err` 이고 직전 값을 유지한다. 버스 명령이 아니다(ADR-0227 — 측정은 위조할 수 없게 웹뷰만 보고한다).
#[tauri::command]
pub fn report_dock_chrome(
    window: Window,
    app: AppHandle,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
    chrome: DockChrome,
) -> Result<(), String> {
    match dock.record_parked(window.label(), Measure::Chrome(chrome.clone())) {
        Some(recorded) => recorded.map_err(|e| e.to_string())?,
        None => apply::report_dock_chrome(&layout, window.label(), chrome)?,
    }
    DockExec::tauri(&app, dock.inner(), layout.inner()).snapshot_trigger();
    Ok(())
}

/// 부른 창의 지금 역할(동기) — `invoke('window_role')`. 웹뷰가 마운트 때 묻고 `parked` 면 대기 모드로 그린다(TRD §4 ·
/// §2-5). 웹뷰 안 배선이라 버스 명령표에 올리지 않는다.
#[tauri::command]
pub fn window_role(
    window: Window,
    dock: State<'_, Arc<DockState>>,
    layout: State<'_, LayoutState>,
) -> WindowRoleReply {
    let label = window.label();
    let role = dock.window_role(label, || {
        apply::list_windows(&layout).is_ok_and(|windows| windows.iter().any(|w| w == label))
    });
    WindowRoleReply { role, drag: None }
}

/// 대기 창 명부의 창인가 — 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록이 빼는 창(TRD §2-5).
pub(crate) fn is_parked_window(app: &AppHandle, label: &str) -> bool {
    app.try_state::<Arc<DockState>>()
        .is_some_and(|dock| dock.is_parked(label))
}

/// 창 `Destroyed` 의 명부 갈래(TRD §2-5 「사라지면」) — 그 창을 명부에서 빼고, 예비 창이었으면 명부에 예비(짓는 중
/// 포함)가 없을 때 작업 스레드에서 새로 짓는다(예비는 늘 하나 · G11). 닫는 창의 명부 항목을 지우는 곳은 여기 하나다.
/// 레이아웃 락 밖에서 부른다(`lib.rs`).
// ADR-0295
pub(crate) fn on_parked_window_destroyed(app: &AppHandle, label: &str) {
    let Some(dock) = app.try_state::<Arc<DockState>>() else {
        return;
    };
    let Some(gone) = dock.parked().remove(label) else {
        return;
    };
    match gone.role {
        ParkedRole::Spare if dock.exiting() => {
            tracing::debug!(
                module = "dock",
                label,
                "앱이 끝나는 중에 예비 창이 사라졌다 — 다시 짓지 않는다"
            );
        }
        ParkedRole::Spare => {
            tracing::warn!(module = "dock", label, "예비 창을 잃어 명부에서 뺐다");
            let has_spare = dock.parked().has_spare();
            if !has_spare {
                spawn_spare_build(app);
            }
        }
        ParkedRole::Kept => {
            tracing::debug!(module = "dock", label, "남긴 창이 사라져 명부에서 뺐다");
        }
    }
}

/// 창 `Destroyed` 의 안내 창 갈래(TRD §2-5 「안내 창」 「사라지면」) — 작업 스레드에서 다시 짓는다.
// ADR-0295
pub(crate) fn on_overlay_destroyed(app: &AppHandle, label: &str) {
    if !is_overlay_label(label) {
        return;
    }
    if exiting(app) {
        tracing::debug!(
            module = "dock",
            label,
            "앱이 끝나는 중에 안내 창이 사라졌다 — 다시 짓지 않는다"
        );
        return;
    }
    tracing::warn!(module = "dock", label, "안내 창이 사라져 다시 짓는다");
    spawn_overlay_build(app);
}

/// 앱 종료 요청 — 이 뒤로 사라지는 예비 창 · 안내 창은 다시 짓지 않는다(`lib.rs` 의 `run` 사건).
pub(crate) fn on_app_exiting(app: &AppHandle) {
    if let Some(dock) = app.try_state::<Arc<DockState>>() {
        dock.begin_exit();
    }
}

// ── 예비 창 · 안내 창 짓기(TRD §2-5 · §2-4) ─────────────────────────────────────
//
// 둘 다 미리 하나씩 지어 ★숨긴 채★ 기다린다 — 화면 밖 대기 자리를 두지 않는다(사용자 결정 2026-10-19). 숨긴 최상위 창은
// 작업 표시줄 · Alt+Tab 에 나오지 않는다. 대기 창 명부의 남긴 창(`Kept` — 뒤 단위)도 붙는 순간의 날 `SW_HIDE` 그대로
// 숨긴 채 기다린다 — 화면 밖으로 옮기지 않는다.
// ★숨긴 채 기다린 창의 첫 보이기 모습은 잰 적이 없다★ — 스파이크 S8 의 미리 만든 창(깜빡임 0)은 화면 밖에 보이는
// 채였다. 새면 증상 = 뗄 때 · 안내 창을 처음 그릴 때 검정 · 흰 · 지난 그림이 몇 프레임 보인다.
// ★작업 스레드에서 짓는다★ — 동기 IPC 처리기 안에서 지으면 Windows 에서 교착이고(스파이크), 메인 스레드의 사건
// 처리기(`Destroyed`)도 부르는 쪽이다.

/// 안내 창 label — 팝아웃 label 이 아니라 트레이 · 닫기 요청 · 명부의 대상이 아니다(TRD §2-5 · G10).
pub(crate) const DOCK_OVERLAY_LABEL: &str = "dock-overlay";
// 안내 창 경로(G10) — 웹뷰의 `DockOverlayPage`.
const DOCK_OVERLAY_URL: &str = "index.html#/dock-overlay";
// 안내 창의 지을 때 크기(논리 px) — 그릴 때 대상 창 안쪽에 맞춰 다시 놓으므로 기다리는 동안의 값일 뿐이다(스파이크
//   setup 의 G 400 × 300 그대로).
const DOCK_OVERLAY_SIZE: (f64, f64) = (400.0, 300.0);

pub(crate) fn is_overlay_label(label: &str) -> bool {
    label == DOCK_OVERLAY_LABEL
}

/// 예비 창 하나를 작업 스레드에서 짓는다 — 명부에 예비(짓는 중 포함)가 이미 있거나 앱이 끝나는 중이면 짓지 않는다.
pub(crate) fn spawn_spare_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_spare(&app));
}

/// 안내 창을 작업 스레드에서 짓는다 — 앱이 끝나는 중이면 짓지 않는다.
pub(crate) fn spawn_overlay_build(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || build_overlay(&app));
}

fn exiting(app: &AppHandle) -> bool {
    app.try_state::<Arc<DockState>>()
        .is_some_and(|dock| dock.exiting())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

// 예비 창 — 명부에 `Spare` 로 적고(짓기 전 — 짓는 동안의 트레이 · `--hidden` 도 그 창을 보지 않게) · 숨긴 채 짓고 ·
//   지어짐을 세운다. 짓기에 실패하면 명부에서 뺀다 — 두면 「예비 있음」이 늘 참이라 다시 짓지 못한다. 기다리는 동안 이
//   창에는 창 호출을 하지 않는다 — tao 보임 깃발(거짓)이 OS 보임과 맞은 채다.
//
// ★들이는 차례(뒤 단위 — 실행기의 `Unpark` · `PlaceDetached`)★ — tao 보임 깃발이 늘 OS 보임과 맞게:
// 1. 숨긴 채 자리 · 크기 = platform `place_window(rect: Some(바깥 사각형), z: Keep, show: false, activate: false)`.
//    `SWP_SHOWWINDOW` 없는 `SetWindowPos` 한 번이라 창은 숨은 채고 tao 깃발도 건드리지 않으며, 값이 §5-5 셈의
//    단위(물리 px 바깥 사각형) 그대로다. Tauri `set_position` · `set_size` 는 쓰지 않는다 — 자리 · 크기가 따로 두 번
//    바뀌고 `set_size` 는 안쪽 크기다.
// 2. 보이기 = Tauri `show()` — tao 보임 깃발이 참이 돼 그 뒤 트레이 `show` · `hide` 가 그대로 돈다. 날 Win32 로 보이면
//    깃발이 거짓으로 남아 트레이 숨기기가 아무것도 하지 않는다(바뀐 깃발이 없으면 tao 0.35.3 `apply_diff` 가 돌아간다).
//    포커스 없이 지은 창이라 tao 는 활성화하지 않고 보인다(`SW_SHOWNOACTIVATE`).
// 3. 활성 = platform `place_window(rect: None, z: Keep, show: false, activate: true)` — Tauri `set_focus` 는 쓰지 않는다
//    (tao 가 Alt 를 합성해 키 감시가 놓기로 읽는다 — TRD F7 a).
// 예비로 되돌릴 때(`SpareGiveBack`)는 Tauri `hide()` 로 숨긴다 — 날 `SW_HIDE` 면 tao 보임 깃발이 참으로 남아 다음
//   들이기의 2 가 아무것도 하지 않는다.
// ADR-0295
fn build_spare(app: &AppHandle) {
    let started = Instant::now();
    let (Some(dock), Some(labels)) = (
        app.try_state::<Arc<DockState>>(),
        app.try_state::<Arc<PopupCounter>>(),
    ) else {
        tracing::warn!(
            module = "dock",
            "도킹 상태 · 팝아웃 번호가 등록돼 있지 않아 예비 창을 짓지 못했다"
        );
        return;
    };
    if dock.exiting() {
        tracing::debug!(module = "dock", "앱이 끝나는 중이라 예비 창을 짓지 않는다");
        return;
    }
    let label = LabelSource::next_label(&**labels);
    let reserved = dock.parked().reserve_spare(label.clone());
    if !reserved {
        tracing::debug!(module = "dock", label, "예비 창이 이미 있어 짓지 않는다");
        return;
    }
    if let Err(e) = crate::commands::popout::build_spare_window(app, &label) {
        dock.parked().remove(&label);
        tracing::warn!(module = "dock", label, "예비 창을 짓지 못했다: {e}");
        return;
    }
    let built = dock.parked().mark_built(&label);
    let build_ms = elapsed_ms(started);
    if built {
        tracing::debug!(
            module = "dock",
            label,
            role = "spare",
            build_ms,
            "예비 창을 지었다"
        );
    } else {
        tracing::debug!(
            module = "dock",
            label,
            role = "spare",
            build_ms,
            "예비 창이 짓는 동안 사라졌다"
        );
    }
}

// 안내 창 — 투명 · 테두리 · 그림자 없음 · 크기 고정 · 활성화 안 함 · 항상 위 · 클릭 통과(TRD §2-5 · 스파이크 G). 숨긴
//   채 짓고 보이지 않는다 — 끄는 동안의 보이기 · 숨기기 · 자리 · z 는 platform 함수(날 Win32)로만 한다. 그래서 tao 보임
//   깃발은 늘 거짓이다.
// ★지은 뒤로 이 창에는 tao · Tauri 창 깃발 setter 를 부르지 않는다★ — 깃발이 하나라도 바뀌면 tao 가 제 보임 깃발대로
//   창을 숨기고(`SW_HIDE`) `GWL_EXSTYLE` 을 제 깃발로 통째 다시 써 Alt+Tab 빼기를 되돌린다(TRD §2-5 「tao 깃발
//   불변식」). 지금 그런 호출은 아래 짓기 걸음뿐이다.
// ADR-0295
fn build_overlay(app: &AppHandle) {
    let started = Instant::now();
    if exiting(app) {
        tracing::debug!(module = "dock", "앱이 끝나는 중이라 안내 창을 짓지 않는다");
        return;
    }
    // 웹뷰 환경 없이 만들지 않는다 — 마무리를 건너뛴 창은 다른 창과 환경이 갈린다(ADR-0054).
    let Some(env) = app.try_state::<Arc<WebviewEnv>>() else {
        tracing::warn!(
            module = "dock",
            "웹뷰 환경이 등록돼 있지 않아 안내 창을 짓지 못했다"
        );
        return;
    };
    let (w, h) = DOCK_OVERLAY_SIZE;
    let built = env
        .finish(
            WebviewWindowBuilder::new(
                app,
                DOCK_OVERLAY_LABEL,
                WebviewUrl::App(DOCK_OVERLAY_URL.into()),
            )
            .title(format!("Engram — {DOCK_OVERLAY_LABEL}"))
            .inner_size(w, h)
            .visible(false)
            .focused(false)
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .always_on_top(true),
        )
        .build();
    let window = match built {
        Ok(window) => window,
        Err(e) => {
            tracing::warn!(module = "dock", "안내 창을 짓지 못했다: {e}");
            return;
        }
    };
    let posted = app.run_on_main_thread(move || {
        exclude_overlay(&window);
        tracing::debug!(
            module = "dock",
            label = window.label(),
            role = "overlay",
            build_ms = elapsed_ms(started),
            "안내 창을 지었다"
        );
    });
    if let Err(e) = posted {
        tracing::warn!(
            module = "dock",
            "안내 창을 다 짓지 못했다(메인 스레드에 못 넘김): {e}"
        );
    }
}

// 지은 안내 창의 깃발 — 메인 스레드에서 한 걸음에. tao 쪽(클릭 통과 · 작업 표시줄 — tao `skip_taskbar` 는 깃발을 바꾸지
//   않는다)을 다 한 뒤 Alt+Tab(platform `set_switcher_hidden`)에서 뺀다(E2). 숨은 동안에는 쓸모가 없지만 끄는 동안 날
//   Win32 로 보일 때 이미 서 있어야 한다. ★메인 스레드여야 하는 까닭★ — 다른 스레드에서 부른 Tauri setter 는
//   이벤트 루프 줄로 넘어가 나중에 돌아, 날 Win32 인 Alt+Tab 빼기가 먼저 돌면 그 뒤의 tao 깃발 바꿈이 `GWL_EXSTYLE` 을
//   통째 다시 써 되돌린다. 실패는 한 줄로 남기고 넘어간다 — 창은 그대로 안내 창이다.
// ADR-0295
fn exclude_overlay(window: &WebviewWindow) {
    let mut failed: Vec<String> = Vec::new();
    if let Err(e) = window.set_ignore_cursor_events(true) {
        failed.push(format!("click_through: {e}"));
    }
    if let Err(e) = window.set_skip_taskbar(true) {
        failed.push(format!("skip_taskbar: {e}"));
    }
    if let Err(e) = engram_dashboard_platform::window::set_switcher_hidden(window, true) {
        failed.push(format!("switcher_hidden: {e}"));
    }
    if !failed.is_empty() {
        tracing::warn!(
            module = "dock",
            label = window.label(),
            ?failed,
            "안내 창의 깃발을 다 세우지 못했다"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashSet, VecDeque};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;
    use crate::dock::book::{
        LastUse, Outcome, OverWaitEnd, CANCEL_LEAVE_MS, DROP_WAIT_MS, EARLY_SIGNAL_MS,
        END_GRACE_MS, LEAVE_GRACE_MS, OVER_WAIT_MS,
    };
    use crate::dock::types::SessionPhase;
    use crate::layout::chrome::{RectCss, TabHeader};
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

    // 포트 호출을 적고, 불릴 때마다 두 락이 다 풀려 있는지 본다(모듈 머리 「락 규율」). 창 보임은 `hidden` 하나가 쥔다
    // — `hide` · `show` 가 바꾸고 `visible` · `probe` 가 읽는다(`probes` 의 `visible` 은 쓰지 않는다).
    struct Fake {
        dock: Arc<DockState>,
        layout: LayoutState,
        calls: Mutex<Vec<Call>>,
        // 시계 · 커서 · 사각형을 읽은 순서.
        reads: Mutex<Vec<&'static str>>,
        hidden: Mutex<HashSet<String>>,
        // 사라진 창 — `visible` 이 `None` 을 · `probe` 가 실패를 돌려준다.
        gone: Mutex<HashSet<String>>,
        // 실패할 창 호출(`hide` · `show` · `focus` · `visible`) — 불린 것은 적고 실패를 돌려준다.
        failing: Mutex<HashSet<&'static str>>,
        // 보낸 영역 그대로(창, 영역) — 보낸 순서.
        regions: Mutex<Vec<(String, DockRegions)>>,
        cursor: Mutex<Result<PhysPoint, String>>,
        probes: Mutex<Vec<(String, Result<WindowProbe, String>)>>,
        outcome: Mutex<Result<DockOutcome, DockError>>,
        clock: AtomicU64,
        // 다음 `now` 한 번에 먼저 돌린다 — 시작의 레이아웃 락 뒤 · `Start` 앞의 틈을 흉내 낸다.
        on_now: Mutex<Option<Hook>>,
        // 다음 커서 읽기 한 번 뒤에 돌린다 — 끝 신호의 세계 읽기와 장부 걸음 사이의 틈을 흉내 낸다.
        on_cursor: Mutex<Option<Hook>>,
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

        fn fails(&self, op: &'static str) -> Result<(), String> {
            if self.failing.lock().unwrap().contains(op) {
                Err(format!("{op} 실패"))
            } else {
                Ok(())
            }
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
            self.fails("hide")?;
            self.hidden.lock().unwrap().insert(label.to_owned());
            Ok(())
        }

        fn show(&self, label: &str) -> Result<(), String> {
            self.record(Call::Show(label.to_owned()));
            self.fails("show")?;
            self.hidden.lock().unwrap().remove(label);
            Ok(())
        }

        fn focus(&self, label: &str) -> Result<(), String> {
            self.record(Call::Focus(label.to_owned()));
            self.fails("focus")
        }

        fn cursor(&self) -> Result<PhysPoint, String> {
            self.record(Call::Cursor);
            self.reads.lock().unwrap().push("cursor");
            let hook = self.on_cursor.lock().unwrap().take();
            if let Some(hook) = hook {
                hook();
            }
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
            self.reads.lock().unwrap().push("probe");
            if self.gone.lock().unwrap().contains(label) {
                return Err("창이 없다".to_owned());
            }
            let visible = !self.hidden.lock().unwrap().contains(label);
            self.probes
                .lock()
                .unwrap()
                .iter()
                .find(|(l, _)| l == label)
                .map_or_else(
                    || Err("창이 없다".to_owned()),
                    |(_, probe)| probe.clone().map(|p| WindowProbe { visible, ..p }),
                )
        }

        fn visible(&self, label: &str) -> Result<Option<bool>, String> {
            self.check("visible");
            self.fails("visible")?;
            if self.gone.lock().unwrap().contains(label) {
                return Ok(None);
            }
            Ok(Some(!self.hidden.lock().unwrap().contains(label)))
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
            self.reads.lock().unwrap().push("now");
            MonoMs(self.clock.fetch_add(10, Ordering::SeqCst))
        }
    }

    // 넘긴 작업을 줄에 세워 두기만 한다 — 시험이 언제 돌릴지 고른다. 메인 스레드 줄은 따로다(올린 순서대로 돈다).
    #[derive(Default)]
    struct FakeSpawn {
        jobs: Mutex<VecDeque<(Option<u64>, DockJob)>>,
        main: Mutex<VecDeque<DockJob>>,
        // 올리는 자리에서 두 락이 풀려 있는지 본다 — 운영의 메인 스레드는 올린 작업을 그 자리에서 돌린다.
        fake: OnceLock<Arc<Fake>>,
    }

    impl DockSpawn for FakeSpawn {
        fn spawn(&self, job: DockJob) {
            self.jobs.lock().unwrap().push_back((None, job));
        }

        fn spawn_after(&self, after_ms: u64, job: DockJob) {
            self.jobs.lock().unwrap().push_back((Some(after_ms), job));
        }

        fn on_main(&self, job: DockJob) -> Result<(), String> {
            if let Some(fake) = self.fake.get() {
                fake.check("on_main");
            }
            self.main.lock().unwrap().push_back(job);
            Ok(())
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

        // 메인 스레드 줄을 올린 순서대로 — 도는 동안 올라온 것까지. 돌린 수.
        fn run_main(&self) -> usize {
            let mut ran = 0;
            loop {
                let job = self.main.lock().unwrap().pop_front();
                let Some(job) = job else {
                    return ran;
                };
                job();
                ran += 1;
            }
        }

        fn main_pending(&self) -> usize {
            self.main.lock().unwrap().len()
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
                reads: Mutex::new(Vec::new()),
                hidden: Mutex::new(HashSet::new()),
                gone: Mutex::new(HashSet::new()),
                failing: Mutex::new(HashSet::new()),
                regions: Mutex::new(Vec::new()),
                cursor: Mutex::new(Ok(CURSOR)),
                probes: Mutex::new(vec![(MAIN.to_owned(), Ok(main_probe()))]),
                outcome: Mutex::new(Ok(DockOutcome {
                    window: NEW_WINDOW.to_owned(),
                    slot_ids: Vec::new(),
                    emptied: None,
                    closed_window: None,
                    main_refilled: None,
                    reused: false,
                })),
                clock: AtomicU64::new(1_000),
                on_now: Mutex::new(None),
                on_cursor: Mutex::new(None),
            });
            let spawn = Arc::new(FakeSpawn::default());
            assert!(spawn.fake.set(Arc::clone(&fake)).is_ok());
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

    // 탭 하나뿐인 팝아웃 P1 — 끄는 동안 숨는다. 배율 2 · 숨긴 뒤에도 게터가 배율을 읽는다.
    fn lone_popout() -> (Rig, ViewId) {
        let mut mgr = ViewManager::new();
        let lone = mgr.create_window(P1).unwrap();
        let rig = Rig::new(mgr);
        rig.fake.probes.lock().unwrap().push((
            P1.to_owned(),
            Ok(WindowProbe {
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
            scroll: None,
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
            unseen: false,
            strip: false,
            hide_deferred: false,
            hidden: false,
            last: LastUse::None,
            last_gap_ms: None,
            tight: false,
            lifted: false,
            last_differs: false,
            late_hide: false,
            held_rect: false,
            over_wait: OverWaitEnd::None,
            over_wait_ms: None,
            over_from: None,
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
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None),
            Err("Dragging".to_owned())
        );
        assert!(
            !rig.dock().session_live(),
            "표식을 못 세운 시작은 깃발을 세우지 않는다"
        );
        assert_eq!(
            rig.exec.start(P1, "d2".into(), a, HOTSPOT, None),
            Err("TabNotFound".to_owned()),
            "부른 창이 그 탭의 주인이 아니다"
        );
        assert!(!rig.dock().session_live());
        assert!(rig.calls().is_empty() && rig.spawn.pending().is_empty());

        rig.mgr().release_lease(other);
        assert_eq!(rig.exec.start(MAIN, "d3".into(), a, HOTSPOT, None), Ok(()));
        assert!(rig.dock().session_live());
        assert_eq!(rig.lease(), Some(a));
        // 출처 사각형을 읽은 뒤 첫 알림이 시작이다.
        assert_eq!(rig.calls().first(), Some(&Call::Probe(MAIN.to_owned())));
        assert!(matches!(
            rig.calls().get(1),
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
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None),
            Err("LateStart".to_owned())
        );

        assert_eq!(rig.lease(), None, "함께 낸 표식 내리기를 실행했다");
        assert!(!rig.dock().session_live(), "자기 세대를 내렸다");
        assert_eq!(
            rig.calls(),
            [Call::Probe(MAIN.to_owned())],
            "거절한 시작은 알리지 않는다 — 출처 사각형만 읽었다"
        );
        assert_eq!(rig.exec.start(MAIN, "d2".into(), a, HOTSPOT, None), Ok(()));
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

        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();

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
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
        let before = rig.calls().len();

        assert_eq!(
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None),
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
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
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

        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();

        assert!(
            rig.applies().is_empty(),
            "시작 명령 안에서 적용 서비스를 부르지 않는다"
        );
        assert_eq!(rig.spawn.pending(), [Some(EARLY_SIGNAL_MS), None]);
        assert!(rig.dock().session_live(), "적용 결과 전에는 끝나지 않는다");

        assert_eq!(rig.spawn.run_now(), 1);
        rig.spawn.run_main();

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
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
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
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
        *rig.fake.cursor.lock().unwrap() = Err("잠금 화면".to_owned());

        rig.exec
            .source_end(MAIN, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();

        assert_eq!(new_window_pos(&rig.applies()[0]), None);
    }

    #[test]
    fn a_start_over_a_live_session_refuses_and_ends_the_old_one_off_the_command() {
        let (mgr, a, b) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
        let before = rig.calls().len();

        assert_eq!(
            rig.exec.start(MAIN, "d2".into(), b, HOTSPOT, None),
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
            rig.exec.start(MAIN, "d2".into(), b, HOTSPOT, None),
            Err("LateStart".to_owned()),
            "거절한 id 는 끝난 id 다"
        );
        assert_eq!(rig.exec.start(MAIN, "d3".into(), b, HOTSPOT, None), Ok(()));
    }

    #[test]
    fn the_drop_wait_timer_comes_back_with_its_kind_and_number_and_ends_the_session() {
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();

        rig.exec
            .source_end(MAIN, "d1".into(), DragEffect::Move, None, None);
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
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();

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
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        assert!(
            !rig.calls().contains(&Call::Hide(P1.to_owned())),
            "숨기기는 메인 스레드 줄로 간다"
        );
        rig.spawn.run_main();
        assert!(rig.calls().contains(&Call::Hide(P1.to_owned())));
        assert!(
            !rig.calls().contains(&Call::Regions(P1.to_owned())),
            "숨긴 출처에는 영역을 보내지 않는다"
        );

        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        rig.spawn.run_main();

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
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();

        rig.exec.window_destroyed(P1);
        rig.spawn.run_main();

        let calls = rig.calls();
        assert!(calls.contains(&Call::Broadcast(SessionPhase::Ended)));
        assert!(!calls.iter().any(|c| matches!(c, Call::Show(_))));
        assert!(rig.applies().is_empty());
        assert_eq!(rig.lease(), None);
        assert!(!rig.dock().session_live());
    }

    // ── 시작 측정 · 끝 칸 · 스크롤 (5.7) ──

    // 탭바 안 순서 이동 구역의 점 — 머리마다 폭 100 · 잡은 자리 50 에서 탭 하나 팝아웃은 늘 자리 0.
    const LONE_P: PointCss = PointCss { x: 50.0, y: 10.0 };
    // main [a, b, c] 에서 b 를 끌 때 — 스크롤 0 이면 자리 1(제자리) · 스크롤 110 이면 자리 2.
    const IN_PLACE_P: PointCss = PointCss { x: 150.0, y: 10.0 };

    // 그 창 탭 목록 그대로의 시작 측정 — 탭바 (0, 0)–(600, 28) · 머리마다 폭 100 · 잡은 자리 50.
    fn strip_for(rig: &Rig, label: &str, scroll: f64) -> StripStart {
        let tabs = rig.mgr().windows[label].tabs.clone();
        let rect = |x0: f64, x1: f64| RectCss {
            x0,
            y0: 0.0,
            x1,
            y1: 28.0,
        };
        StripStart {
            bar: rect(0.0, 600.0),
            heads: tabs
                .iter()
                .enumerate()
                .map(|(i, &view_id)| TabHeader {
                    view_id,
                    rect: rect(100.0 * i as f64, 100.0 * (i + 1) as f64),
                })
                .collect(),
            grab_x: 50.0,
            scroll,
        }
    }

    fn over_report(
        regions: &DockRegions,
        shown: Picked,
        point: PointCss,
        scroll: Option<f64>,
    ) -> DragOverReport {
        DragOverReport::Over {
            rev: regions.rev,
            bar_rev: regions.bar_rev,
            displayed: regions.view_id,
            shown,
            point,
            scroll,
        }
    }

    fn set_probe(rig: &Rig, label: &str, probe: WindowProbe) {
        let mut probes = rig.fake.probes.lock().unwrap();
        probes.retain(|(l, _)| l != label);
        probes.push((label.to_owned(), Ok(probe)));
    }

    // 끄는 탭이 그 창 안에 남은 적용 — 모델만 바뀌고 창은 그대로다.
    fn kept_in(window: &str) -> Result<DockOutcome, DockError> {
        Ok(DockOutcome {
            window: window.to_owned(),
            slot_ids: Vec::new(),
            emptied: None,
            closed_window: None,
            main_refilled: None,
            reused: false,
        })
    }

    fn tab_bar_dest(req: &DockRequest) -> (String, Option<usize>) {
        match &req.to {
            DockDest::TabBar { window, index } => (window.clone(), *index),
            other => panic!("탭바 적용이 아니다: {other:?}"),
        }
    }

    fn is_hidden(rig: &Rig, label: &str) -> bool {
        rig.fake.hidden.lock().unwrap().contains(label)
    }

    fn count(rig: &Rig, call: &Call) -> usize {
        rig.calls().iter().filter(|c| *c == call).count()
    }

    // main [a, b, c] 에서 시작 측정(스크롤 0)을 실어 b 를 끈다.
    fn three_tab_reorder() -> (Rig, ViewId) {
        let mut mgr = ViewManager::new();
        let b = mgr.create_tab(MAIN, None).unwrap();
        mgr.create_tab(MAIN, None).unwrap();
        let rig = Rig::new(mgr);
        let strip = strip_for(&rig, MAIN, 0.0);
        rig.exec
            .start(MAIN, "d1".into(), b, HOTSPOT, Some(strip))
            .unwrap();
        (rig, b)
    }

    #[test]
    fn a_start_strip_defers_the_lone_popouts_hide_and_gives_its_regions_a_reorder_zone() {
        let (rig, lone) = lone_popout();
        let strip = strip_for(&rig, P1, 0.0);

        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, Some(strip))
            .unwrap();
        rig.spawn.run_main();

        assert!(
            !rig.calls().contains(&Call::Hide(P1.to_owned())),
            "출처 영역에 순서 이동이 서면 숨기기를 도킹 모드 증거까지 미룬다"
        );
        assert!(rig.last_regions(P1).and_then(|r| r.reorder).is_some());
    }

    #[test]
    fn a_missing_canceled_is_false_and_an_engine_canceled_end_changes_nothing() {
        for (canceled, applies) in [(None, 1), (Some(false), 1), (Some(true), 0)] {
            let (mgr, a, _) = two_tabs();
            let rig = Rig::new(mgr);
            rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();

            rig.exec
                .source_end(MAIN, "d1".into(), DragEffect::None, canceled, None);
            rig.spawn.run_now();

            assert_eq!(rig.applies().len(), applies, "{canceled:?}");
            assert_eq!(rig.lease(), None, "{canceled:?}");
            assert!(!rig.dock().session_live(), "{canceled:?}");
        }
    }

    #[test]
    fn the_end_snapshot_reaches_the_book() {
        for with_last in [false, true] {
            let (rig, lone) = lone_popout();
            set_probe(
                &rig,
                P1,
                WindowProbe {
                    origin: (3000, 100),
                    ..main_probe()
                },
            );
            let strip = strip_for(&rig, P1, 0.0);
            rig.exec
                .start(P1, "d1".into(), lone, HOTSPOT, Some(strip))
                .unwrap();
            let regions = rig.last_regions(P1).unwrap();
            let last = DrawnReport {
                rev: regions.rev,
                bar_rev: regions.bar_rev,
                displayed: regions.view_id,
                shown: Picked::Reorder { index: 0 },
                point: LONE_P,
                scroll: None,
                left_before_end_ms: None,
            };

            rig.exec.source_end(
                P1,
                "d1".into(),
                DragEffect::None,
                None,
                with_last.then_some(last),
            );
            rig.spawn.run_now();

            let req = &rig.applies()[0];
            if with_last {
                // 받은 보고가 없어도 출처가 끝에 그린 것 — 그 자리.
                assert_eq!(tab_bar_dest(req), (P1.to_owned(), Some(0)));
            } else {
                // 보고도 스냅숏도 없고 커서가 출처 밖 — 새 창.
                assert!(matches!(req.to, DockDest::NewWindow { .. }));
            }
        }
    }

    #[test]
    fn the_drop_scroll_reaches_the_reorder_repick() {
        for (scroll, applied) in [(Some(110.0), true), (None, false)] {
            let (rig, b) = three_tab_reorder();
            let regions = rig.last_regions(MAIN).unwrap();

            rig.exec.drop_at(
                MAIN,
                DropReport {
                    drag_id: "d1".into(),
                    view_id: b,
                    rev: regions.rev,
                    bar_rev: regions.bar_rev,
                    displayed: regions.view_id,
                    shown: Picked::Reorder { index: 2 },
                    point: IN_PLACE_P,
                    scroll,
                },
            );
            rig.spawn.run_now();

            if applied {
                assert_eq!(tab_bar_dest(&rig.applies()[0]), (MAIN.to_owned(), Some(2)));
            } else {
                assert!(
                    rig.applies().is_empty(),
                    "스크롤 없음 = 시작 스크롤 0 으로 자리 1 — 그린 자리 2 와 달라 무변경"
                );
            }
            assert!(!rig.dock().session_live());
        }
    }

    #[test]
    fn the_over_scroll_reaches_the_last_point_judgement() {
        let (rig, _) = three_tab_reorder();
        let regions = rig.last_regions(MAIN).unwrap();
        rig.exec.over(
            MAIN,
            "d1".into(),
            over_report(
                &regions,
                Picked::Reorder { index: 2 },
                IN_PLACE_P,
                Some(110.0),
            ),
        );
        // 끝 커서 = main 안쪽의 그 점(원점 (100, 50) · 배율 1.5).
        *rig.fake.cursor.lock().unwrap() = Ok(PhysPoint {
            x: 100.0 + 150.0 * 1.5,
            y: 50.0 + 10.0 * 1.5,
        });

        rig.exec
            .source_end(MAIN, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();

        assert_eq!(tab_bar_dest(&rig.applies()[0]), (MAIN.to_owned(), Some(2)));
    }

    // ── 시계 · 세계 읽기 순서 ──

    #[test]
    fn end_signals_and_timers_stamp_the_clock_before_reading_the_world() {
        fn arrival_first(rig: &Rig, what: &str) {
            let reads = std::mem::take(&mut *rig.fake.reads.lock().unwrap());
            let now = reads.iter().position(|r| *r == "now");
            let cursor = reads.iter().position(|r| *r == "cursor");
            assert!(
                now.is_some() && cursor.is_some() && now < cursor,
                "{what}: {reads:?}"
            );
        }

        let (mgr, a, b) = two_tabs();
        let rig = Rig::new(mgr);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
        rig.fake.reads.lock().unwrap().clear();
        rig.exec
            .drop_at(MAIN, unguided_drop("d1", a, PointCss { x: 40.0, y: 20.0 }));
        arrival_first(&rig, "drop");
        rig.spawn.run_now();

        rig.exec.start(MAIN, "d2".into(), b, HOTSPOT, None).unwrap();
        rig.fake.reads.lock().unwrap().clear();
        rig.exec
            .source_end(MAIN, "d2".into(), DragEffect::Move, None, None);
        arrival_first(&rig, "source_end");
        rig.spawn.fire(DROP_WAIT_MS);
        arrival_first(&rig, "drop_wait");
        rig.spawn.run_now();

        rig.exec.start(MAIN, "d3".into(), b, HOTSPOT, None).unwrap();
        assert!(rig.exec.start(MAIN, "d4".into(), b, HOTSPOT, None).is_err());
        rig.fake.reads.lock().unwrap().clear();
        rig.spawn.run_now();
        arrival_first(&rig, "superseded");
    }

    // ── 숨기기 · 보이기 맞추기 ──

    #[test]
    fn a_late_hide_queued_behind_the_ends_show_leaves_the_popout_shown() {
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = Err(DockError::WindowOpen("닫혔다".to_owned()));
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(is_hidden(&rig, P1), "살아 있는 세션의 숨기기는 숨긴다");

        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        assert!(is_hidden(&rig, P1), "보이기는 아직 메인 스레드 줄에 있다");
        // 그 세션이 먼저 낸 숨기기가 다른 스레드에서 보이기보다 늦게 올라왔다.
        rig.exec.run(
            vec![Action::Hide {
                label: P1.to_owned(),
                generation: 1,
            }],
            false,
        );
        assert_eq!(rig.spawn.run_main(), 2);

        assert!(!is_hidden(&rig, P1), "마지막 상태 = 보임");
        assert_eq!(count(&rig, &Call::Hide(P1.to_owned())), 1);
        assert_eq!(count(&rig, &Call::Show(P1.to_owned())), 1);
    }

    #[test]
    fn a_stale_sessions_hide_and_show_follow_the_current_session() {
        // 옛 세션(P1 숨김)의 보이기가 줄에 있는 동안 새 세션이 다른 창에서 섰다 — 옛 팝아웃이 숨은 채 남지 않고, 옛
        //   세션의 늦은 숨기기는 아무것도 숨기지 않는다.
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = Err(DockError::WindowOpen("닫혔다".to_owned()));
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        let main_tab = rig.mgr().windows[MAIN].active;
        rig.exec
            .start(MAIN, "d2".into(), main_tab, HOTSPOT, None)
            .unwrap();
        rig.exec.run(
            vec![Action::Hide {
                label: P1.to_owned(),
                generation: 1,
            }],
            false,
        );
        rig.spawn.run_main();
        assert!(!is_hidden(&rig, P1));
        assert_eq!(count(&rig, &Call::Hide(P1.to_owned())), 1);

        // 새 세션이 같은 팝아웃을 다시 숨기면 옛 보이기가 그것을 드러내지 않는다.
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = Err(DockError::WindowOpen("닫혔다".to_owned()));
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        rig.exec
            .start(P1, "d2".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(is_hidden(&rig, P1));
        assert_eq!(count(&rig, &Call::Show(P1.to_owned())), 0);
    }

    #[test]
    fn the_ends_focus_runs_on_the_main_thread_behind_its_show() {
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = Ok(DockOutcome {
            window: P1.to_owned(),
            slot_ids: Vec::new(),
            emptied: None,
            closed_window: None,
            main_refilled: None,
            reused: true,
        });
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();

        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        assert!(
            !rig.calls()
                .iter()
                .any(|c| matches!(c, Call::Show(_) | Call::Focus(_))),
            "끝의 보이기 · 포커스는 메인 스레드 줄에서 돈다"
        );
        assert_eq!(rig.spawn.main_pending(), 2);
        rig.spawn.run_main();

        let calls = rig.calls();
        let shown = position(&calls, |c| *c == Call::Show(P1.to_owned()));
        let focused = position(&calls, |c| *c == Call::Focus(P1.to_owned()));
        assert!(shown < focused);
        assert!(!is_hidden(&rig, P1));
    }

    #[test]
    fn a_window_gone_before_its_reconcile_runs_is_skipped_without_an_error() {
        let (rig, lone) = lone_popout();
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.fake.gone.lock().unwrap().insert(P1.to_owned());

        assert_eq!(rig.spawn.run_main(), 1);

        assert!(!rig.calls().iter().any(|c| matches!(c, Call::Hide(_))));
        assert_eq!(rig.dock().take_errors(1), 0);
    }

    #[test]
    fn a_leave_timer_fed_between_the_end_world_read_and_the_end_step_lands_on_the_source_and_shows_it(
    ) {
        let (rig, lone) = lone_popout();
        // 출처 사각형이 다른 창과 겹치지 않게 — 시작에 이 값을 쥔다(숨긴 뒤의 사각형에는 출처가 없다).
        set_probe(
            &rig,
            P1,
            WindowProbe {
                origin: (3000, 100),
                scale: 2.0,
                ..main_probe()
            },
        );
        *rig.fake.outcome.lock().unwrap() = kept_in(P1);
        let strip = strip_for(&rig, P1, 0.0);
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, Some(strip))
            .unwrap();
        let regions = rig
            .last_regions(P1)
            .expect("숨기기를 미룬 출처도 영역을 받는다");
        rig.exec.over(
            P1,
            "d1".into(),
            over_report(&regions, Picked::Reorder { index: 0 }, LONE_P, None),
        );
        rig.exec.over(P1, "d1".into(), DragOverReport::Left);
        assert_eq!(rig.spawn.pending(), [Some(LEAVE_GRACE_MS)]);
        // 끝 커서 = 출처 안쪽의 그 점. 커서를 읽은 직후 떠남 시한이 장부에 먹히고 메인 스레드가 그 숨기기를 돌린다.
        *rig.fake.cursor.lock().unwrap() = Ok(PhysPoint {
            x: 3000.0 + LONE_P.x * 2.0,
            y: 100.0 + LONE_P.y * 2.0,
        });
        let spawn = Arc::clone(&rig.spawn);
        *rig.fake.on_cursor.lock().unwrap() = Some(Box::new(move || {
            spawn.fire(LEAVE_GRACE_MS);
            spawn.run_main();
        }));

        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        assert!(is_hidden(&rig, P1), "그 틈의 떠남 시한이 출처를 숨겼다");
        rig.spawn.run_now();
        rig.spawn.run_main();

        assert_eq!(
            tab_bar_dest(&rig.applies()[0]),
            (P1.to_owned(), Some(0)),
            "끝이 시한보다 먼저 닿았다 — 출처 위에서 그린 자리"
        );
        assert!(!is_hidden(&rig, P1), "마지막 상태 = 보임");
        let calls = rig.calls();
        let hid = position(&calls, |c| *c == Call::Hide(P1.to_owned()));
        let shown = position(&calls, |c| *c == Call::Show(P1.to_owned()));
        let focused = position(&calls, |c| *c == Call::Focus(P1.to_owned()));
        assert!(hid < shown && shown < focused);
    }

    // ── 끝의 포커스 · 되보이기 몫 · 끝 줄 뒤 실패 (9b 리뷰) ──

    fn reused_p1() -> Result<DockOutcome, DockError> {
        Ok(DockOutcome {
            window: P1.to_owned(),
            slot_ids: Vec::new(),
            emptied: None,
            closed_window: None,
            main_refilled: None,
            reused: true,
        })
    }

    // P1 에서 끌어 끝낸다 — 끝의 할 일(보이기 · 포커스)은 메인 스레드 줄에 남긴다.
    fn ended_lone(outcome: Result<DockOutcome, DockError>) -> (Rig, ViewId) {
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = outcome;
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        (rig, lone)
    }

    fn refused() -> Result<DockOutcome, DockError> {
        Err(DockError::WindowOpen("닫혔다".to_owned()))
    }

    #[test]
    fn an_ends_focus_is_dropped_when_a_new_session_starts_before_it_runs() {
        // 같은 팝아웃에서 새 끌기 — 새 세션이 숨길 창에 옛 끝의 포커스를 주지 않는다.
        let (rig, lone) = ended_lone(reused_p1());
        rig.exec
            .start(P1, "d2".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(!rig.calls().iter().any(|c| matches!(c, Call::Focus(_))));
        assert!(is_hidden(&rig, P1));

        // 다른 창에서 새 끌기 — 옛 팝아웃은 되보이되 포커스는 새 세션 몫이다.
        let (rig, _) = ended_lone(reused_p1());
        let main_tab = rig.mgr().windows[MAIN].active;
        rig.exec
            .start(MAIN, "d2".into(), main_tab, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(!rig.calls().iter().any(|c| matches!(c, Call::Focus(_))));
        assert!(!is_hidden(&rig, P1));

        // 새 세션이 없으면 포커스를 준다.
        let (rig, _) = ended_lone(reused_p1());
        rig.spawn.run_main();
        assert_eq!(count(&rig, &Call::Focus(P1.to_owned())), 1);
    }

    #[test]
    fn the_end_reshows_only_what_the_reconcile_itself_hid() {
        // 맞추기가 숨긴 팝아웃은 끝에서 되보인다.
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = refused();
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(rig.dock().owns_hidden(P1));
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        rig.spawn.run_main();
        assert!(!is_hidden(&rig, P1));
        assert!(!rig.dock().owns_hidden(P1));

        // 트레이가 먼저 숨긴 팝아웃 — 맞추기가 숨기지 않았으니 끝도 드러내지 않는다.
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = refused();
        rig.fake.hidden.lock().unwrap().insert(P1.to_owned());
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        rig.spawn.run_main();
        assert!(is_hidden(&rig, P1));
        assert_eq!(count(&rig, &Call::Hide(P1.to_owned())), 0);
        assert_eq!(count(&rig, &Call::Show(P1.to_owned())), 0);

        // 숨기기 맞추기가 돌기 전에 끝난 세션 — 그 사이 트레이가 숨긴 창을 지난 할 일이 드러내지 않는다.
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = refused();
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        rig.fake.hidden.lock().unwrap().insert(P1.to_owned());
        assert_eq!(rig.spawn.run_main(), 2);
        assert!(is_hidden(&rig, P1));
        assert_eq!(count(&rig, &Call::Show(P1.to_owned())), 0);
    }

    #[test]
    fn a_destroyed_window_leaves_the_reconciles_own_hidden_set() {
        let (rig, lone) = lone_popout();
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(rig.dock().owns_hidden(P1));

        rig.exec.window_destroyed(P1);

        assert!(!rig.dock().owns_hidden(P1));
    }

    #[test]
    fn main_thread_failures_after_the_end_line_count_toward_no_session() {
        // 끝 줄 뒤에 돈 보이기가 실패했다 — 그 세션 끝 줄은 이미 나갔고, 그 사이 선 다음 세션 셈에도 섞이지 않는다.
        for next_first in [false, true] {
            let (rig, _) = ended_lone(refused());
            rig.fake.failing.lock().unwrap().insert("show");
            if next_first {
                let main_tab = rig.mgr().windows[MAIN].active;
                rig.exec
                    .start(MAIN, "d2".into(), main_tab, HOTSPOT, None)
                    .unwrap();
            }
            rig.spawn.run_main();
            assert_eq!(count(&rig, &Call::Show(P1.to_owned())), 1, "{next_first}");
            assert_eq!(rig.dock().booked_errors(), 0, "{next_first}");
        }

        // 살아 있는 세션의 메인 스레드 실패는 그 세션 몫으로 센다.
        let (rig, lone) = lone_popout();
        rig.fake.failing.lock().unwrap().insert("visible");
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert_eq!(rig.dock().take_errors(1), 1);
    }

    #[test]
    fn a_failed_hide_after_another_path_reshowed_the_window_drops_the_ownership() {
        let (rig, lone) = lone_popout();
        *rig.fake.outcome.lock().unwrap() = refused();
        rig.exec
            .start(P1, "d1".into(), lone, HOTSPOT, None)
            .unwrap();
        rig.spawn.run_main();
        assert!(rig.dock().owns_hidden(P1));
        // 다른 길이 되보였다 — 그 뒤 맞추기의 숨기기가 실패한다.
        rig.fake.hidden.lock().unwrap().remove(P1);
        rig.fake.failing.lock().unwrap().insert("hide");
        rig.exec.run(
            vec![Action::Hide {
                label: P1.to_owned(),
                generation: 1,
            }],
            false,
        );
        rig.spawn.run_main();
        assert!(!rig.dock().owns_hidden(P1));

        // 그 뒤 트레이가 숨긴 채로 끝 — 끌기가 숨긴 것이 아니니 드러내지 않는다.
        rig.fake.failing.lock().unwrap().clear();
        rig.fake.hidden.lock().unwrap().insert(P1.to_owned());
        rig.exec
            .source_end(P1, "d1".into(), DragEffect::None, None, None);
        rig.spawn.run_now();
        rig.spawn.run_main();
        assert!(is_hidden(&rig, P1));
        assert_eq!(count(&rig, &Call::Show(P1.to_owned())), 0);
    }

    #[test]
    fn a_left_that_releases_a_held_end_hands_its_apply_to_the_spawn_port() {
        const P2: &str = "slot-popup-2";
        let (mut mgr, a, _) = two_tabs();
        mgr.create_window(P2).unwrap();
        let rig = Rig::new(mgr);
        set_probe(
            &rig,
            P2,
            WindowProbe {
                origin: (3000, 100),
                scale: 1.0,
                ..main_probe()
            },
        );
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
        // 출처가 마지막으로 그린 것 — 그 뒤 실제로 떠났다(간격 > CANCEL_LEAVE_MS). 끝 커서는 P2 안이고 P2 의 Over 는
        //   아직 오는 중이다.
        let main_regions = rig.last_regions(MAIN).unwrap();
        let last = DrawnReport {
            rev: main_regions.rev,
            bar_rev: main_regions.bar_rev,
            displayed: main_regions.view_id,
            shown: Picked::None,
            point: PointCss { x: 40.0, y: 20.0 },
            scroll: None,
            left_before_end_ms: Some(CANCEL_LEAVE_MS + 25.0),
        };
        *rig.fake.cursor.lock().unwrap() = Ok(PhysPoint {
            x: 3100.0,
            y: 200.0,
        });
        rig.exec
            .source_end(MAIN, "d1".into(), DragEffect::None, None, Some(last));
        assert_eq!(rig.spawn.pending(), [Some(OVER_WAIT_MS)]);

        let p2_regions = rig.last_regions(P2).unwrap();
        rig.exec.over(
            P2,
            "d1".into(),
            over_report(
                &p2_regions,
                Picked::None,
                PointCss { x: 100.0, y: 100.0 },
                None,
            ),
        );
        let before = rig.calls().len();
        rig.exec.over(P2, "d1".into(), DragOverReport::Left);

        assert!(
            !rig.calls()[before..]
                .iter()
                .any(|c| matches!(c, Call::Apply(_) | Call::Show(_) | Call::Focus(_))),
            "붙들기를 푼 동기 명령 안에서 적용 서비스 · 창을 부르지 않는다"
        );
        assert_eq!(
            rig.spawn.pending().iter().filter(|p| p.is_none()).count(),
            1
        );
        assert_eq!(rig.spawn.run_now(), 1);
        assert_eq!(rig.applies().len(), 1);
        // 풀린 뒤의 붙들기 시한은 버린다.
        rig.spawn.fire(OVER_WAIT_MS);
        assert_eq!(rig.applies().len(), 1);
        assert!(!rig.dock().session_live());
    }

    // 예비 창(명부의 창) · 안내 창은 모델 밖이라 옛 장부의 대상이 아니다 — 끝 커서를 품어도 사각형을 읽지 않고 오는 중
    //   `Over` 를 기다리지 않는다(TRD S21-slot-docking §13 U4 「옛 끌기 그대로 돎」). 품은 창이 모델 창이면 기다린다 —
    //   바로 위 시험과 같은 자리 · 같은 끝이다.
    #[test]
    fn the_parked_spare_and_the_overlay_are_no_targets_of_the_old_drag() {
        const SPARE: &str = "slot-popup-2";
        let (mgr, a, _) = two_tabs();
        let rig = Rig::new(mgr);
        rig.dock().parked().reserve_spare(SPARE.to_owned());
        let holding = WindowProbe {
            origin: (3000, 100),
            scale: 1.0,
            ..main_probe()
        };
        set_probe(&rig, SPARE, holding);
        set_probe(&rig, DOCK_OVERLAY_LABEL, holding);
        rig.exec.start(MAIN, "d1".into(), a, HOTSPOT, None).unwrap();
        let main_regions = rig.last_regions(MAIN).unwrap();
        let last = DrawnReport {
            rev: main_regions.rev,
            bar_rev: main_regions.bar_rev,
            displayed: main_regions.view_id,
            shown: Picked::None,
            point: PointCss { x: 40.0, y: 20.0 },
            scroll: None,
            left_before_end_ms: Some(CANCEL_LEAVE_MS + 25.0),
        };
        *rig.fake.cursor.lock().unwrap() = Ok(PhysPoint {
            x: 3100.0,
            y: 200.0,
        });
        rig.exec
            .source_end(MAIN, "d1".into(), DragEffect::None, None, Some(last));

        assert!(
            !rig.spawn.pending().contains(&Some(OVER_WAIT_MS)),
            "오는 중 Over 를 기다리지 않는다: {:?}",
            rig.spawn.pending()
        );
        for label in [SPARE, DOCK_OVERLAY_LABEL] {
            assert_eq!(
                count(&rig, &Call::Probe(label.to_owned())),
                0,
                "{label} 의 사각형을 읽지 않는다"
            );
            assert!(rig.last_regions(label).is_none(), "{label} 에 영역이 없다");
        }
    }
}
