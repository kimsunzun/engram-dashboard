//! 예열 창 상태(TRD S21-slot-docking §3-9) — 풀([`pool`] — 순수 상태 기계)을 락 · 조건 변수 · 주입한 첫 화면 문
//! 손잡이([`PaintGate`]) · 시한 둘([`SpareSeams`])과 묶는다. 그 창 웹뷰와의 wire = [`wire`].
//!
//! Tauri 를 모른다 — 창 짓기 · 닫기 · 사건 · 시한 걸기는 부르는 쪽이 넘긴다(`commands/spare.rs` · 창 포트 ·
//! [`CommitPort`]). 레이아웃 권위(`crate::layout` · ADR-0035)에는 확정이 쥐던 재기를 옮기는 것 말고 쓰지 않는다.
// ADR-0299

pub mod pool;
pub mod wire;

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;

use pool::{
    ClaimStep, CommitDecision, Gone, Listen, Measure, NoBuild, Opening, SpareMeasures, SparePool,
    TrayShow,
};
use wire::{WindowRole, WindowRoleReply};

use crate::layout::manager::WindowLabel;
use crate::layout::{
    apply, LabelSource, LayoutError, LayoutState, SpareClaim, UiMetrics, ViewManager,
};

/// 짓는 중인 예열 창을 기다리는 상한(I2) — 넘으면 새로 짓는다. 짓는 창이 메인 스레드를 쥐고 있어 새로 지어도 그
/// 뒤에 서므로 짓기 한 번(≈ 160 ms)보다 넉넉히 둔다. 실측이 아니라 판단이다.
pub const BUILD_WAIT: Duration = Duration::from_secs(1);

/// 채택한 창의 다시 짓기 완료를 기다리는 안전 시한(I8) — 첫 화면 시한이 확정 ③ 부터 돌므로 그 위에 메인 스레드
/// 줄서기 몫을 얹는다. 먼저 울리면 보이려는 창과 짓기(≈ 160 ms)가 메인 스레드에서 겹친다(예열 F2). 여유 1 s 는 판단이다.
pub const REVEAL_SAFETY: Duration =
    crate::first_paint::FALLBACK_REVEAL.saturating_add(Duration::from_secs(1));

/// 안전 시한조차 끝내지 못한 보임 기다림을 맡기기가 끝내는 상한 — 그 시한이 걸리지 않았거나 울리지 않았을 때의 그물이다.
/// 두 배는 판단이다(시한 일이 늦게 돌 몫을 둔다).
pub const REVEAL_BACKSTOP: Duration = REVEAL_SAFETY.saturating_mul(2);

/// 첫 화면 문 손잡이(`first_paint` — I17 · I23 · I26). 풀을 만들 때 넘긴다.
///
/// ★`hold_unarmed` · `cancel` 은 풀 락을 쥔 채 불린다★ — 구현은 자기 잎 락 하나만 잡고 풀을 부르지 않는다(차례 =
/// 레이아웃 → 풀 → 첫 화면 문). 맡김 상태와 올림이 한 락 구간에서 함께 바뀌어야 그 사이의 UI 숨기기가 거둘 것을 찾고,
/// 되물림의 거둠이 곧이어 다시 맡긴 쪽의 올림을 지우지 않는다.
pub trait PaintGate: Send + Sync {
    /// 그 label 에 받아 두기만 하는 올림을 건다 — 알림 · 시한이 그 창을 보이지 않는다(보일지는 `reveal_if_held` 가 정한다).
    /// 이미 올림이 있으면 그대로 둔다(받아 둔 알림을 지우지 않는다).
    fn hold_unarmed(&self, label: &str);
    /// 그 label 의 올림을 거둔다 — 없으면 무동작.
    fn cancel(&self, label: &str);
    /// 확정 ③ — 그 label 의 올림이 남아 있으면 기다리는 상태로 올리고 `true`(그 뒤 채택 번호 `generation` 을 실은 알림만
    /// 받는다 · `activate` = 보인 뒤 활성화를 고를지 · `opened` = 그 창을 마지막으로 연(놓은) 때 — 보임 로그의 열기 → 보임
    /// 몫), 없으면(UI 숨기기가 거뒀다) 아무 일도 하지 않고 `false`. 풀 락 · 레이아웃 락 밖에서 불린다.
    fn reveal_if_held(
        &self,
        label: &str,
        generation: u32,
        activate: bool,
        opened: Option<Instant>,
    ) -> bool;
}

/// 풀을 만들 때 넘기는 seam(I23) — 시험은 짧은 시한과 가짜 문을 넘긴다.
pub struct SpareSeams {
    pub gate: Arc<dyn PaintGate>,
    /// 짓는 중인 예열 창을 기다리는 상한([`BUILD_WAIT`]).
    pub build_wait: Duration,
    /// 확정 뒤 다시 짓기 완료의 안전 시한([`REVEAL_SAFETY`]) — [`CommitPort::reveal_deadline`] 에 넘긴다.
    pub reveal_safety: Duration,
    /// 맡기기가 끝나지 않은 보임 기다림을 끝내는 상한([`REVEAL_BACKSTOP`]).
    pub reveal_backstop: Duration,
}

impl SpareSeams {
    pub fn new(gate: Arc<dyn PaintGate>) -> Self {
        Self {
            gate,
            build_wait: BUILD_WAIT,
            reveal_safety: REVEAL_SAFETY,
            reveal_backstop: REVEAL_BACKSTOP,
        }
    }
}

/// 확정 ③ ④ 뒤의 창 쪽 일(셸 어댑터). ★전부 레이아웃 락 · 풀 락 밖에서 불린다★ — `destroy` 는 이벤트 루프를 기다리고 그
/// `Destroyed` 처리기가 레이아웃 락 · 풀 락을 잡는다.
pub trait CommitPort {
    /// ④ — 그 label 의 웹뷰에 `spare:adopted { generation }` 을 낸다.
    fn adopted(&self, label: &str, generation: u32);
    /// 안전 시한을 건다 — `after` 뒤 [`SpareState::on_reveal_overdue`] 를 부르고 그 답이 참이면 짓기를 띄운다.
    fn reveal_deadline(&self, label: &str, after: Duration);
    /// 그 창을 닫는다(destroy). `Err` = 못 닫았다.
    fn destroy(&self, label: &str) -> Result<(), String>;
}

/// [`SpareState::claim`] 의 답.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReply {
    /// 맡긴 예열 창 — `None` 이면 부른 쪽이 새 label 로 새로 짓는다.
    pub claim: Option<SpareClaim>,
    /// 부른 쪽이 예열 창 짓기를 띄운다(I16) — `claim = None` 일 때만 참이다.
    pub build: bool,
    /// 짓는 중인 창을 기다린 시간(로그 `wait_ms`) — 기다리지 않았으면 `None`.
    pub waited: Option<Duration>,
}

/// [`SpareState::answer_role`] 의 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleAnswer {
    pub reply: WindowRoleReply,
    /// 듣기를 못 건 풀의 창이라 풀에서 뺐다 — 부른 쪽이 그 창을 닫는다(락 밖 · 동기 처리기 밖 · I28).
    pub close: bool,
}

/// [`SpareState::build`] 의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpareBuild {
    /// 자리를 못 잡아 짓지 않았다 — 번호를 받지 않았다(I13).
    Refused(NoBuild),
    Failed {
        label: WindowLabel,
        error: String,
    },
    Built {
        label: WindowLabel,
    },
    /// 지었으나 그사이 칸을 떠났다(`Destroyed` · 듣기 실패).
    Vanished {
        label: WindowLabel,
    },
}

/// [`SpareState::commit`] 의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    /// 채택했다 — `revealed` = ③ 이 남은 올림을 올렸다(`false` = UI 숨기기가 거뒀다 · 숨긴 채 모델 창이 된다).
    Adopted { revealed: bool },
    /// 모델에 그 label 이 없어 닫았다 — 그 `Destroyed` 가 칸을 비우고 다시 짓는다.
    Closed,
    /// 두 번 다 못 닫아 칸을 비웠다(I31) — 다음 맡기기가 짓는다.
    CloseFailed,
    /// 낡은 맡김표 — 아무것도 하지 않았다(I27).
    Stale,
}

/// 확정 ① ② 를 마친 단계 — 레이아웃 락을 놓은 뒤 [`Self::finish`] 로 ③ ④(또는 닫기)를 한다.
///
/// ★두 반쪽(`commit_locked` · `finish`)을 이 모듈 밖에 내주지 않는다★ — 바깥은 [`SpareState::commit`] 하나로 부른다.
/// 사이에서 단계를 버리면 `awaiting_reveal` 이 영영 서 그 실행의 예열이 꺼지거나, 칸이 닫는 중으로 멈춰 숨은 창이 남는다.
#[must_use = "레이아웃 락을 놓은 뒤 finish 를 불러야 확정이 끝난다"]
#[derive(Debug)]
struct CommitStep {
    claim: SpareClaim,
    decision: CommitDecision,
    carried: Result<(), LayoutError>,
}

/// 셸에 하나인 예열 창 상태.
///
/// ★락 차례 = 레이아웃 → 풀 → 첫 화면 문★ — 풀 락을 쥔 채 잡는 것은 첫 화면 문의 잎 락 하나뿐이다([`PaintGate`]).
/// 쥔 채 레이아웃 락 · emit · 창 호출 · 로그를 하지 않는다. 레이아웃 락을 쥔 채 풀을 잡는 것은 된다(확정 ① —
/// [`Self::commit`]).
pub struct SpareState {
    pool: Mutex<SparePool>,
    /// 짓는 중이던 칸이 바뀌었다(지어짐 · 짓기 실패 · 사라짐 · 종료) — 기다리는 맡기기를 깨운다.
    changed: Condvar,
    gate: Arc<dyn PaintGate>,
    build_wait: Duration,
    reveal_safety: Duration,
    reveal_backstop: Duration,
}

impl SpareState {
    pub fn new(seams: SpareSeams) -> Self {
        Self {
            pool: Mutex::new(SparePool::default()),
            changed: Condvar::new(),
            gate: seams.gate,
            build_wait: seams.build_wait,
            reveal_safety: seams.reveal_safety,
            reveal_backstop: seams.reveal_backstop,
        }
    }

    fn pool(&self) -> MutexGuard<'_, SparePool> {
        sync::lock(&self.pool)
    }

    /// 풀의 창인가(짓는 중 · 지어짐 · 맡김 · 닫는 중) — 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록이 빼는 창.
    pub fn is_spare(&self, label: &str) -> bool {
        self.pool().contains(label)
    }

    /// 보임을 기다리는 채택 창 label 과 그 채택 번호(I26).
    pub fn awaiting_reveal(&self) -> Option<(WindowLabel, u32)> {
        self.pool()
            .awaiting_reveal()
            .map(|(label, generation)| (label.to_owned(), generation))
    }

    /// 그 label 이 보임을 기다리는 채택 창이면 그 채택 번호 — `window_role` 이 답에 싣는다(I26).
    pub fn reveal_generation(&self, label: &str) -> Option<u32> {
        self.pool().reveal_generation(label)
    }

    /// 앱이 끝나는 중이다 — 맡기지도 짓지도 다시 짓지도 않는다. 한 번 서면 내리지 않는다.
    pub fn begin_exit(&self) {
        self.pool().begin_exit();
        self.changed.notify_all();
    }

    pub fn exiting(&self) -> bool {
        self.pool().exiting()
    }

    /// 창 `label` 의 역할 — 풀의 창이면 `Spare`, 아니면 `in_model` 로 `Model` · `Unknown`. `in_model` 은 풀 락을
    /// 놓은 뒤 부르므로 레이아웃 락을 잡아도 된다.
    pub fn window_role(&self, label: &str, in_model: impl FnOnce() -> bool) -> WindowRole {
        if self.is_spare(label) {
            WindowRole::Spare
        } else if in_model() {
            WindowRole::Model
        } else {
            WindowRole::Unknown
        }
    }

    /// `window_role{adoptable}` 의 답(I26 · I28) — 채택 사건 듣기를 못 건 풀의 창이면 풀에서 빼고([`Self::on_listen`])
    /// `unknown` + 닫기다. 그 밖은 [`Self::window_role`] 의 역할이고, 모델 창이면 보임을 기다리는 채택 번호를 싣는다.
    /// `in_model` 은 풀 락을 놓은 뒤 부른다.
    pub fn answer_role(
        &self,
        label: &str,
        adoptable: bool,
        in_model: impl FnOnce() -> bool,
    ) -> RoleAnswer {
        if let Listen::Removed { .. } = self.on_listen(label, adoptable) {
            return RoleAnswer {
                reply: WindowRoleReply {
                    role: WindowRole::Unknown,
                    generation: None,
                },
                close: true,
            };
        }
        let role = self.window_role(label, in_model);
        // 번호는 역할을 본 뒤 읽는다 — 확정 ① 전의 `spare` 답에 ① 뒤의 번호가 붙지 않는다.
        let generation = match role {
            WindowRole::Model => self.reveal_generation(label),
            WindowRole::Spare | WindowRole::Unknown => None,
        };
        RoleAnswer {
            reply: WindowRoleReply { role, generation },
            close: false,
        }
    }

    // ── 재기 보고(I6) ──

    /// 풀의 창이 보낸 재기를 적는다 — `None` = 풀의 창이 아니다(부르는 쪽이 모델에 적는다 · 뜻 = [`SparePool::record`]).
    /// 풀 락은 돌아오기 전에 놓는다.
    pub fn record_measure(&self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        self.pool().record(label, measure)
    }

    /// 창 웹뷰의 캔버스 보고 — 풀의 창이면 풀에, 아니면 모델에 적는다. 풀을 먼저 보고 놓은 뒤 레이아웃 락을 잡으므로,
    /// 확정 ① 뒤에 닿은 보고는 그 확정의 레이아웃 락을 기다려 옮긴 값 위에 적힌다.
    pub fn report_canvas(
        &self,
        layout: &LayoutState,
        label: &str,
        w: u32,
        h: u32,
    ) -> Result<(), String> {
        match self.record_measure(label, Measure::Canvas { w, h }) {
            Some(recorded) => recorded.map_err(|e| e.to_string()),
            None => apply::report_window_canvas(layout, label, w, h),
        }
    }

    /// [`Self::report_canvas`] 의 UI 치수 짝.
    pub fn report_metrics(
        &self,
        layout: &LayoutState,
        label: &str,
        metrics: UiMetrics,
    ) -> Result<(), String> {
        match self.record_measure(label, Measure::Metrics(metrics)) {
            Some(recorded) => recorded.map_err(|e| e.to_string()),
            None => apply::report_ui_metrics(layout, label, metrics),
        }
    }

    // ── 짓기(TRD §3-9 「부팅」 · 「다시 짓기」 · I13) ──

    /// 예열 창 하나를 짓는다 — 칸 자리를 먼저 잡고(이미 있거나 · 보임을 기다리거나 · 종료 중 · 꺼졌으면 번호를 받지
    /// 않고 `Refused`) 그 뒤 `labels` 에서 번호를 받아 `build` 로 숨긴 채 짓는다. 락 밖에서 부르고, `build` 는 락 밖에서
    /// 돈다(창 짓기 — 작업 스레드).
    // ADR-0299
    pub fn build(
        &self,
        labels: &dyn LabelSource,
        build: impl FnOnce(&str) -> Result<(), String>,
    ) -> SpareBuild {
        let started = Instant::now();
        let reserved = self.pool().reserve();
        let revealed_at = match reserved {
            Ok(at) => at,
            Err(reason) => {
                tracing::debug!(module = "spare", ?reason, "예열 창을 짓지 않는다");
                return SpareBuild::Refused(reason);
            }
        };
        let label = labels.next_label();
        if !self.pool().assign(label.clone()) {
            tracing::warn!(
                module = "spare",
                label,
                "잡은 짓기 자리가 번호를 받는 사이 바뀌었다 — 짓지 않는다"
            );
            return SpareBuild::Vanished { label };
        }
        if let Err(error) = build(&label) {
            if self.pool().build_failed(&label) {
                self.changed.notify_all();
            }
            tracing::warn!(module = "spare", label, "예열 창을 짓지 못했다: {error}");
            return SpareBuild::Failed { label, error };
        }
        let built = self.pool().mark_built(&label);
        self.changed.notify_all();
        let build_ms = elapsed_ms(started);
        if built {
            tracing::info!(
                module = "spare",
                label,
                build_ms,
                after_reveal_ms = ?revealed_at.map(elapsed_ms),
                "예열 창을 지었다"
            );
            SpareBuild::Built { label }
        } else {
            tracing::debug!(
                module = "spare",
                label,
                build_ms,
                "예열 창이 짓는 동안 사라졌다"
            );
            SpareBuild::Vanished { label }
        }
    }

    // ── 맡기기 · 열기 · 되물림(I2 · I4 · I16 · I17 · I27) ──

    /// 지어진 예열 창을 맡긴다 — 같은 풀 락 안에서 그 label 에 받아 두기만 하는 첫 화면 올림을 건다. 짓는 중이면
    /// 지어질 때까지(상한 = `build_wait`) 기다린다 — 지어짐 · 짓기 실패 · 사라짐 · 종료가 깨우고, 깬 뒤 먼저 락을 잡은
    /// 하나만 맡는다. 기다리던 짓기가 실패하거나 사라졌으면 그 뒤에 자리를 잡은 다른 짓기를 따라가지 않고 곧바로
    /// `None` 이다. 보임 기다림이 `reveal_backstop` 을 넘겨 서 있으면 먼저 그 다시 짓기 완료를 한다(그 뒤 칸이 비었으니
    /// 이 맡기기가 짓기를 띄운다). ★락 밖 · 메인 스레드 밖에서 부른다★(I15 — 그 짓기가 메인 스레드에서 돈다).
    // ADR-0299
    pub fn claim(&self) -> ClaimReply {
        let started = Instant::now();
        let deadline = started + self.build_wait;
        let mut waited = false;
        let mut awaited = None;
        let mut pool = self.pool();
        let expired = pool.expire_reveal(started, self.reveal_backstop);
        let (claim, build, timed_out) = loop {
            match pool.take_built(awaited) {
                ClaimStep::Claimed(claim) => {
                    self.gate.hold_unarmed(&claim.label);
                    break (Some(claim), false, false);
                }
                ClaimStep::Fresh { build } => break (None, build, false),
                ClaimStep::Wait { build } => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        break (None, false, true);
                    }
                    waited = true;
                    awaited = Some(build);
                    pool.park();
                    pool = sync::wait_timeout(&self.changed, pool, left).0;
                    pool.unpark();
                }
            }
        };
        drop(pool);
        if let Some((label, generation)) = expired {
            tracing::warn!(
                module = "spare",
                label,
                generation,
                backstop_ms = u64::try_from(self.reveal_backstop.as_millis()).unwrap_or(u64::MAX),
                "채택한 창의 보임 기다림이 안전 시한을 넘겨도 끝나지 않아 맡기기가 다시 짓기 완료로 쳤다"
            );
        }
        let waited = waited.then(|| started.elapsed());
        if timed_out {
            tracing::warn!(
                module = "spare",
                wait_ms = elapsed_ms(started),
                "짓는 중인 예열 창이 시한 안에 지어지지 않았다 — 새로 짓는다"
            );
        }
        ClaimReply {
            claim,
            build,
            waited,
        }
    }

    /// 맡긴 label 을 열(놓을) 때 — 지금 맡긴 창이면 `activate` 를 그 맡김에 적고 `Claimed`(짓지 않고 놓는다). 맡긴 채
    /// 사라졌거나 맡기지 않은 예열 창 label 이면 `Unavailable`(새로 짓지 않고 `Err` — I20), 풀과 무관하면 `Fresh`.
    pub fn opening(&self, label: &str, activate: bool) -> Opening {
        self.pool().opening(label, activate, Instant::now())
    }

    /// 확정 전 되물림 — 맡김표가 지금의 맡김일 때만 숨긴 채 칸에 돌려놓고 같은 풀 락 안에서 올림을 거둔다(`true`).
    /// 낡은 맡김표면 무동작 + warn(`false`).
    pub fn release(&self, claim: &SpareClaim) -> bool {
        let released = {
            let mut pool = self.pool();
            let released = pool.release(claim);
            if released {
                self.gate.cancel(&claim.label);
            }
            released
        };
        if released {
            tracing::debug!(
                module = "spare",
                label = claim.label,
                generation = claim.generation,
                "맡긴 예열 창을 돌려받았다"
            );
        } else {
            tracing::warn!(
                module = "spare",
                label = claim.label,
                generation = claim.generation,
                "지금의 맡김이 아닌 맡김표의 되물림 — 무동작"
            );
        }
        released
    }

    // ── 확정(TRD §3-9 「`committed`」 · I6 · I21 · I25 · I27 · I31) ──

    /// 맡긴 창의 새 창 항목이 모델에 든 뒤 한 번 — ① ② 를 레이아웃 락 하나 안에서, 락을 놓은 뒤 ③ ④ 또는 닫기를 한다.
    /// 락 밖에서 부른다.
    pub fn commit(
        &self,
        layout: &LayoutState,
        claim: &SpareClaim,
        port: &dyn CommitPort,
    ) -> CommitOutcome {
        let step = {
            let mut mgr = sync::lock(&layout.0);
            self.commit_locked(&mut mgr, claim)
        };
        step.finish(self, port)
    }

    /// 확정 ① ② — `mgr` = 부른 쪽이 쥔 레이아웃 락. ① 맡김표가 지금의 맡김이면 칸을 비우며 같은 풀 락 안에서
    /// `awaiting_reveal` 을 세우고(I25), 그 label 이 모델에 없으면 비우지 않고 닫기로 정하며 같은 풀 락 안에서 올림을
    /// 거둔다(I27). ② 쥐던 재기를 같은 레이아웃 락 안에서 모델에 적는다 — 확정 도중 닿은 보고는 그 락을 기다려 이 값
    /// 위에 적힌다(I6).
    // ADR-0299
    fn commit_locked(&self, mgr: &mut ViewManager, claim: &SpareClaim) -> CommitStep {
        let in_model = mgr.window_attrs(&claim.label).is_ok();
        let decision = {
            let mut pool = self.pool();
            let decision = pool.commit(claim, in_model, Instant::now());
            if decision == CommitDecision::Close {
                self.gate.cancel(&claim.label);
            }
            decision
        };
        let carried = match &decision {
            CommitDecision::Adopt { measures, .. } => carry_measures(mgr, &claim.label, measures),
            CommitDecision::Close | CommitDecision::Stale => Ok(()),
        };
        CommitStep {
            claim: claim.clone(),
            decision,
            carried,
        }
    }

    fn close_failed(&self, claim: &SpareClaim) -> bool {
        self.pool().close_failed(claim)
    }

    // ── 다시 짓기 완료(I8 · I25) · `Destroyed` ──

    /// 채택한 창이 보였다(첫 화면 풀림 또는 그 시한) — `true` = 부른 쪽이 짓기를 띄운다. label 마다 한 번이다(그
    /// label 의 다른 완료 — 사라짐 · 안전 시한 — 가 먼저 왔으면 `false`). 보임을 기다리는 창이 아닌 label 이면 `false`.
    pub fn on_shown(&self, label: &str) -> bool {
        self.complete_reveal(label, "shown")
    }

    /// 안전 시한이 울렸다 — 뜻은 [`Self::on_shown`] 과 같다.
    pub fn on_reveal_overdue(&self, label: &str) -> bool {
        self.complete_reveal(label, "overdue")
    }

    fn complete_reveal(&self, label: &str, cause: &'static str) -> bool {
        let done = self.pool().complete_reveal(label, Instant::now());
        if let Some(build) = done {
            tracing::debug!(
                module = "spare",
                label,
                cause,
                build,
                "채택한 창의 보임 기다림이 끝났다"
            );
        }
        done.unwrap_or(false)
    }

    /// 창이 사라졌다(`Destroyed` — 모든 창) — 칸의 창이면 칸을 비우고(맡긴 창이면 같은 풀 락 안에서 올림을 거둔다)
    /// 종료 중 · 꺼짐이 아니면 `true`(다시 짓는다). 보임을 기다리던 채택 창이면 다시 짓기 완료다. 그 밖은 `false`.
    pub fn on_destroyed(&self, label: &str) -> bool {
        let gone = {
            let mut pool = self.pool();
            let gone = pool.destroyed(label, Instant::now());
            if let Gone::Removed { claimed: true, .. } = gone {
                self.gate.cancel(label);
            }
            gone
        };
        match gone {
            Gone::NotOurs => false,
            Gone::Removed {
                claimed,
                closing,
                rebuild,
            } => {
                self.changed.notify_all();
                if closing {
                    tracing::debug!(
                        module = "spare",
                        label,
                        rebuild,
                        "확정이 닫기로 정한 예열 창이 닫혀 풀에서 뺐다"
                    );
                } else if rebuild {
                    tracing::warn!(
                        module = "spare",
                        label,
                        claimed,
                        "예열 창이 사라져 풀에서 뺐다 — 다시 짓는다"
                    );
                } else {
                    tracing::debug!(
                        module = "spare",
                        label,
                        claimed,
                        "예열 창이 사라져 풀에서 뺐다 — 종료 중이거나 꺼져 다시 짓지 않는다"
                    );
                }
                rebuild
            }
            Gone::RevealDone { build } => {
                tracing::debug!(
                    module = "spare",
                    label,
                    cause = "destroyed",
                    build,
                    "채택한 창의 보임 기다림이 끝났다"
                );
                build
            }
        }
    }

    // ── 듣기 실패(I28) · 트레이 보이기 ──

    /// `window_role{adoptable}` 의 풀 몫 — 풀의 창이 듣기를 못 걸었으면 칸을 비운다(맡긴 창이면 같은 풀 락 안에서 올림을
    /// 거둔다 · 그 `Destroyed` 는 다시 짓지 않는다). 부른 쪽이 그 창을 락 밖에서 닫는다. 잇달아
    /// [`pool::LISTEN_FAILURE_LIMIT`] 번이면 그 실행 내내 풀을 끈다. 풀의 창이 듣기를 걸었으면 셈을 0 으로 되돌린다.
    pub fn on_listen(&self, label: &str, adoptable: bool) -> Listen {
        let listen = {
            let mut pool = self.pool();
            let listen = pool.listen(label, adoptable);
            if let Listen::Removed { claimed: true, .. } = listen {
                self.gate.cancel(label);
            }
            listen
        };
        if let Listen::Removed {
            claimed,
            disabled_now,
        } = listen
        {
            self.changed.notify_all();
            tracing::warn!(
                module = "spare",
                label,
                claimed,
                "예열 창이 채택 사건 듣기를 못 걸어 풀에서 뺐다 — 닫고 곧바로 다시 짓지 않는다"
            );
            if disabled_now {
                tracing::warn!(
                    module = "spare",
                    failures = pool::LISTEN_FAILURE_LIMIT,
                    "예열 창 듣기 실패가 잇달아 일어나 이 실행 내내 예열 창을 쓰지 않는다"
                );
            }
        }
        listen
    }

    /// 숨긴 UI 를 다시 보일 때 술어로 건너뛴 예열 창 label 마다 — 맡긴 창이면 같은 풀 락 안에서 올림을 다시 건다.
    pub fn on_tray_show(&self, label: &str) -> TrayShow {
        let pool = self.pool();
        let show = pool.tray_show(label);
        if show == TrayShow::Rehold {
            self.gate.hold_unarmed(label);
        }
        show
    }
}

impl CommitStep {
    /// 확정 ③ ④ 또는 닫기 — ★레이아웃 락 · 풀 락을 놓은 뒤 부른다★. 채택이면 ③ 남은 올림을 올리기로 정하고 ④ 채택
    /// 사건을 내고 안전 시한을 건다. 닫기면 닫고, 못 닫으면 한 번 더, 그래도 못 닫으면 칸을 비운다(I31).
    fn finish(self, spare: &SpareState, port: &dyn CommitPort) -> CommitOutcome {
        let Self {
            claim,
            decision,
            carried,
        } = self;
        let label = claim.label.as_str();
        let generation = claim.generation;
        match decision {
            CommitDecision::Stale => {
                tracing::warn!(
                    module = "spare",
                    label,
                    generation,
                    "지금의 맡김이 아닌 맡김표의 확정 — 무동작"
                );
                CommitOutcome::Stale
            }
            CommitDecision::Adopt {
                activate,
                opened_at,
                ..
            } => {
                if let Err(e) = carried {
                    tracing::warn!(
                        module = "spare",
                        label,
                        "예열 창이 쥐던 재기를 모델에 옮기지 못했다: {e}"
                    );
                }
                let revealed = spare
                    .gate
                    .reveal_if_held(label, generation, activate, opened_at);
                port.adopted(label, generation);
                port.reveal_deadline(label, spare.reveal_safety);
                tracing::info!(
                    module = "spare",
                    label,
                    generation,
                    since_open_ms = ?opened_at.map(elapsed_ms),
                    revealed,
                    "맡긴 예열 창을 채택했다(committed)"
                );
                CommitOutcome::Adopted { revealed }
            }
            CommitDecision::Close => {
                tracing::warn!(
                    module = "spare",
                    label,
                    generation,
                    "확정 때 모델에 그 창이 없다 — 돌려받지 않고 닫는다"
                );
                let Err(first) = port.destroy(label) else {
                    return CommitOutcome::Closed;
                };
                let Err(second) = port.destroy(label) else {
                    tracing::warn!(
                        module = "spare",
                        label,
                        "예열 창을 한 번 못 닫아 다시 닫았다: {first}"
                    );
                    return CommitOutcome::Closed;
                };
                let freed = spare.close_failed(&claim);
                tracing::warn!(
                    module = "spare",
                    label,
                    freed,
                    "예열 창을 두 번 다 못 닫아 칸을 비운다 — 다음 맡기기가 짓는다: {first} / {second}"
                );
                CommitOutcome::CloseFailed
            }
        }
    }
}

// 모델 값이 없는 칸은 건드리지 않는다 — 새 창 항목은 재기 없이 들고, 풀의 값이 없으면 남길 것도 없다.
fn carry_measures(
    mgr: &mut ViewManager,
    label: &str,
    measures: &SpareMeasures,
) -> Result<(), LayoutError> {
    if let Some(canvas) = measures.canvas {
        mgr.set_window_canvas(label, canvas.w, canvas.h)?;
    }
    if let Some(metrics) = measures.metrics {
        mgr.set_ui_metrics(label, metrics)?;
    }
    Ok(())
}

pub fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
