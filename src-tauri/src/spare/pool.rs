//! 예열 창 풀(TRD S21-slot-docking §3-9 「풀」) — 칸 하나의 순수 상태 기계. 락 · 조건 변수 · 첫 화면 문 · 창은 모른다
//! (그 몫 = [`super::SpareState`]). 자료와 판정뿐이다 — 창을 만들거나 만지지 않는다.
//!
//! 「예열 창인가」는 label 의 성질이 아니라 지금 상태라 접두가 아니라 이 풀로 가른다 — 채택한 창은 보통 팝아웃이 되는데
//! label 은 바꿀 수 없다.
//!
//! - 칸이 쥔 label 의 창(짓는 중 · 지어짐 · 맡김 · 닫는 중)은 트레이 보이기 · 숨기기 · `--hidden` 과 자리 기록에서
//!   빠진다([`SparePool::contains`]). 맡긴 뒤 확정 전에는 그 label 이 이미 모델에 있어도 풀의 술어가 이긴다.
//! - 그 창 웹뷰의 재기 보고는 모델 대신 여기 적힌다([`SparePool::record`]) — 확정이 모델로 옮긴다.
//! - 짓기는 칸 하나에 하나다 — 자리를 먼저 잡고([`SparePool::reserve`]) 그 뒤 팝아웃 번호를 받는다(I13 — 거절된
//!   짓기가 번호를 태우지 않는다).
//! - 보임을 기다리는 채택 창(`awaiting_reveal`)이 있는 동안은 짓지 않는다 — 다시 짓기는 그 창의 보임 완료 하나가
//!   띄운다(I25).
// ADR-0295 · ADR-0299

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crate::layout::manager::{check_metrics, reported_canvas, WindowLabel};
use crate::layout::{CanvasPx, LayoutError, SpareClaim, UiMetrics};

/// 듣기 실패가 잇달아 이만큼이면 그 실행 내내 풀을 끈다(I28).
pub const LISTEN_FAILURE_LIMIT: u8 = 3;

/// 예열 창 웹뷰가 보고한 재기 둘 — 모델 창 항목의 `canvas` · `metrics` 와 같은 꼴 · 같은 받는 규칙이다. `None` = 아직
/// 보고가 없다.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpareMeasures {
    pub canvas: Option<CanvasPx>,
    pub metrics: Option<UiMetrics>,
}

/// 웹뷰가 보낸 재기 하나 — 받는 규칙은 모델 항목과 같다(`ViewManager::set_window_canvas` · `set_ui_metrics`).
#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Canvas { w: u32, h: u32 },
    Metrics(UiMetrics),
}

/// 칸 하나의 상태. label 은 팝아웃 번호에서 받아 다시 쓰이지 않는다.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    Empty,
    /// 짓는 중 — `label = None` 은 자리만 잡고 번호를 아직 받지 않았다.
    Building {
        label: Option<WindowLabel>,
        measures: SpareMeasures,
    },
    Built {
        label: WindowLabel,
        measures: SpareMeasures,
    },
    /// 한 호출자에게 맡겼다(확정 · 되물림 전). `activate` = 그 label 의 `open_at` 인자 — 맡긴 직후 · `open` 은 참.
    /// `opened_at` = 그 label 을 마지막으로 연(놓은) 때 — 로그 몫.
    Claimed {
        label: WindowLabel,
        measures: SpareMeasures,
        generation: u32,
        activate: bool,
        opened_at: Option<Instant>,
    },
    /// 확정 때 모델에 없어 닫기로 정한 맡긴 창 — 그 `Destroyed` 가 빼고 다시 짓는다. 그동안 아무도 맡지 못하고 어느
    /// 맡김표도 맞지 않는다(I27 — 풀로 돌아가는 길은 되물림 하나뿐이다).
    Closing {
        label: WindowLabel,
        generation: u32,
    },
}

impl Slot {
    fn label(&self) -> Option<&str> {
        match self {
            Self::Empty | Self::Building { label: None, .. } => None,
            Self::Building {
                label: Some(label), ..
            }
            | Self::Built { label, .. }
            | Self::Claimed { label, .. }
            | Self::Closing { label, .. } => Some(label),
        }
    }

    fn measures_mut(&mut self) -> Option<&mut SpareMeasures> {
        match self {
            Self::Building { measures, .. }
            | Self::Built { measures, .. }
            | Self::Claimed { measures, .. } => Some(measures),
            Self::Empty | Self::Closing { .. } => None,
        }
    }
}

/// 짓기 자리를 잡지 못한 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoBuild {
    /// 칸에 이미 예열 창(짓는 중 포함)이 있다.
    Occupied,
    /// 채택한 창의 보임을 기다린다 — 그 완료가 짓기를 띄운다(I25).
    AwaitingReveal,
    Exiting,
    /// 듣기 실패로 꺼졌다(I28).
    Disabled,
}

/// 맡기기 한 걸음의 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStep {
    Claimed(SpareClaim),
    /// 짓는 중 — 지어질 때까지 기다린다. `build` = 그 짓기의 차례 — 깬 뒤 다음 걸음에 `awaited` 로 넘긴다.
    Wait {
        build: u64,
    },
    /// 맡길 창이 없다 — 부른 쪽은 새로 짓는다. `build` = 예열 창 짓기도 띄운다(I16).
    Fresh {
        build: bool,
    },
}

/// 확정 ① 의 판정.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitDecision {
    /// 칸을 비우고 `awaiting_reveal` 을 세웠다 — 쥐던 재기를 모델로 옮긴다.
    Adopt {
        measures: SpareMeasures,
        activate: bool,
        opened_at: Option<Instant>,
    },
    /// 모델에 그 label 이 없다 — 칸은 [`Slot::Closing`] 이 됐고 그 창을 닫는다.
    Close,
    /// 맡김표가 지금의 맡김이 아니다 — 아무것도 바꾸지 않았다.
    Stale,
}

/// 창 `Destroyed` 의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    NotOurs,
    /// 칸의 창이었다 — 칸을 비웠다. `claimed` = 맡긴 창이었다 · `closing` = 확정이 닫기로 정한 창이었다(기다리던
    /// 사라짐) · `rebuild` = 다시 짓는다.
    Removed {
        claimed: bool,
        closing: bool,
        rebuild: bool,
    },
    /// 보임을 기다리던 채택 창이었다 — 다시 짓기 완료다(`build` = 짓는다).
    RevealDone {
        build: bool,
    },
}

/// `window_role` 의 듣기 결과 판정(I28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listen {
    /// 칸의 창이 아니다 — 셈하지 않는다.
    NotPool,
    /// 칸의 창이 듣기를 걸었다 — 셈을 0 으로 되돌렸다.
    Kept,
    /// 칸의 창이 듣기를 못 걸었다 — 칸을 비웠다(부른 쪽이 그 창을 닫는다 · 그 `Destroyed` 는 다시 짓지 않는다).
    /// `disabled_now` = 이 실패로 풀이 꺼졌다.
    Removed { claimed: bool, disabled_now: bool },
}

/// 맡긴 label 을 열(놓을) 때의 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// 지금 맡긴 창이다 — 짓지 않고 그 창을 놓는다(`activate` 를 그 맡김에 적었다).
    Claimed,
    /// 맡겼던 창이 사라졌거나 맡긴 창이 아니다 — 그 label 로 새로 짓지 않고 `Err` 다(I20).
    Unavailable,
    /// 풀과 무관한 label — 새로 짓는다.
    Fresh,
}

/// 숨긴 UI 를 다시 보일 때 술어로 건너뛴 예열 창 label 의 판정(TRD §3-9 「숨어 있기」).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayShow {
    /// 맡긴 창 — 올림을 다시 걸었다. 확정 ③ 이 그것을 올린다.
    Rehold,
    /// 그사이 확정으로 칸을 떠났다 — 보통 창처럼 곧바로 보인다.
    Adopted,
    /// 맡기지 않은 예열 창 — 그대로 둔다.
    Untouched,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SparePool {
    slot: Slot,
    /// 확정 ① 이 세우고 다시 짓기 완료가 비운다 — 그동안 `window_role` 이 그 번호를 싣는다(I26).
    awaiting_reveal: Option<(WindowLabel, u32)>,
    /// `awaiting_reveal` 을 세운 때 — [`Self::expire_reveal`] 몫. 확정 ① 밖에서 세운 기다림(시험)은 `None` 이다.
    reveal_since: Option<Instant>,
    exiting: bool,
    disabled: bool,
    listen_failures: u8,
    last_generation: u32,
    /// 자리를 잡은 짓기의 차례(잡을 때마다 하나 는다) — 칸의 창은 늘 마지막 차례의 짓기가 지은 것이다.
    last_build: u64,
    /// 맡긴 채 사라진 label — 맡은 쪽의 늦은 열기가 그 label 로 새로 짓지 않게(I20).
    lost: BTreeSet<WindowLabel>,
    /// 다시 짓기 완료가 짓기를 띄운 때 — 다음 짓기의 `after_reveal_ms` 로그 몫.
    revealed_at: Option<Instant>,
    /// 짓는 중인 창을 기다리며 선 맡기기 수.
    waiting: u32,
}

impl Default for SparePool {
    fn default() -> Self {
        Self {
            slot: Slot::Empty,
            awaiting_reveal: None,
            reveal_since: None,
            exiting: false,
            disabled: false,
            listen_failures: 0,
            last_generation: 0,
            last_build: 0,
            lost: BTreeSet::new(),
            revealed_at: None,
            waiting: 0,
        }
    }
}

impl SparePool {
    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub fn awaiting_reveal(&self) -> Option<(&str, u32)> {
        self.awaiting_reveal
            .as_ref()
            .map(|(label, generation)| (label.as_str(), *generation))
    }

    pub fn exiting(&self) -> bool {
        self.exiting
    }

    pub fn disabled(&self) -> bool {
        self.disabled
    }

    pub fn waiting(&self) -> u32 {
        self.waiting
    }

    /// 칸이 쥔 창인가 — 트레이 · 자리 기록이 빼는 창.
    pub fn contains(&self, label: &str) -> bool {
        self.slot.label() == Some(label)
    }

    /// 보임을 기다리는 채택 창이면 그 채택 번호(I26).
    pub fn reveal_generation(&self, label: &str) -> Option<u32> {
        self.awaiting_reveal()
            .and_then(|(l, generation)| (l == label).then_some(generation))
    }

    pub fn begin_exit(&mut self) {
        self.exiting = true;
    }

    // ── 짓기(I13 · I16 · I25) ──

    /// 짓기 자리를 잡는다(label 없는 [`Slot::Building`]) — 번호는 그 뒤에 받는다. 성공이면 직전 다시 짓기 완료의 때를
    /// 넘긴다(로그 몫).
    // ADR-0299
    pub fn reserve(&mut self) -> Result<Option<Instant>, NoBuild> {
        if self.exiting {
            return Err(NoBuild::Exiting);
        }
        if self.disabled {
            return Err(NoBuild::Disabled);
        }
        if self.awaiting_reveal.is_some() {
            return Err(NoBuild::AwaitingReveal);
        }
        if self.slot != Slot::Empty {
            return Err(NoBuild::Occupied);
        }
        self.slot = Slot::Building {
            label: None,
            measures: SpareMeasures::default(),
        };
        self.last_build += 1;
        Ok(self.revealed_at.take())
    }

    /// 잡은 자리에 받은 label 을 적는다 — `false` = 자리가 그사이 바뀌었다.
    pub fn assign(&mut self, label: WindowLabel) -> bool {
        match &mut self.slot {
            Slot::Building { label: slot, .. } if slot.is_none() => {
                *slot = Some(label);
                true
            }
            _ => false,
        }
    }

    /// 그 창이 지어졌다 — `false` = 짓는 동안 사라져 칸을 떠났다.
    pub fn mark_built(&mut self, label: &str) -> bool {
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Building {
                label: Some(l),
                measures,
            } if l == label => {
                self.slot = Slot::Built { label: l, measures };
                true
            }
            other => {
                self.slot = other;
                false
            }
        }
    }

    /// 짓기 실패 — 칸을 비운다(두면 「예열 창 있음」이 늘 참이라 다시 짓지 못한다). `false` = 칸이 이미 그 창이 아니다.
    pub fn build_failed(&mut self, label: &str) -> bool {
        let ours = matches!(&self.slot, Slot::Building { label: Some(l), .. } if l == label);
        if ours {
            self.slot = Slot::Empty;
        }
        ours
    }

    // ── 맡기기 · 열기 · 되물림(I2 · I4 · I27) ──

    fn next_generation(&mut self) -> u32 {
        self.last_generation = self.last_generation.wrapping_add(1);
        self.last_generation
    }

    /// 맡기기 한 걸음. `awaited` = 이 맡기기가 앞서 기다리기 시작한 짓기(그때의 [`ClaimStep::Wait`] 의 `build`) — 그
    /// 짓기가 실패하거나 사라진 뒤 다른 짓기가 칸을 잡았으면 그것을 따라 기다리지도 맡지도 않고 `Fresh` 다(I2 —
    /// 기다리던 짓기의 실패 · 사라짐은 곧바로 `None`).
    // ADR-0299
    pub fn take_built(&mut self, awaited: Option<u64>) -> ClaimStep {
        if self.exiting || self.disabled {
            return ClaimStep::Fresh { build: false };
        }
        let replaced = awaited.is_some_and(|build| build != self.last_build);
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Built { label, measures } if !replaced => {
                let generation = self.next_generation();
                self.slot = Slot::Claimed {
                    label: label.clone(),
                    measures,
                    generation,
                    activate: true,
                    opened_at: None,
                };
                ClaimStep::Claimed(SpareClaim { label, generation })
            }
            other => {
                let step = match &other {
                    Slot::Building { .. } if !replaced => ClaimStep::Wait {
                        build: self.last_build,
                    },
                    Slot::Empty => ClaimStep::Fresh {
                        build: self.awaiting_reveal.is_none(),
                    },
                    _ => ClaimStep::Fresh { build: false },
                };
                self.slot = other;
                step
            }
        }
    }

    pub fn park(&mut self) {
        self.waiting += 1;
    }

    pub fn unpark(&mut self) {
        self.waiting -= 1;
    }

    pub fn opening(&mut self, label: &str, activate: bool, now: Instant) -> Opening {
        if let Slot::Claimed {
            label: l,
            activate: a,
            opened_at,
            ..
        } = &mut self.slot
        {
            if l == label {
                *a = activate;
                *opened_at = Some(now);
                return Opening::Claimed;
            }
        }
        if self.lost.contains(label) || self.contains(label) {
            Opening::Unavailable
        } else {
            Opening::Fresh
        }
    }

    /// 확정 전 되물림 — 맡김표가 지금의 맡김(label · 채택 번호 둘 다)일 때만 숨긴 채 칸에 돌려놓는다(다시 맡길 수
    /// 있다). 쥐던 재기는 그대로다.
    pub fn release(&mut self, claim: &SpareClaim) -> bool {
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Claimed {
                label,
                measures,
                generation,
                ..
            } if label == claim.label && generation == claim.generation => {
                self.slot = Slot::Built { label, measures };
                true
            }
            other => {
                self.slot = other;
                false
            }
        }
    }

    // ── 확정(I21 · I25 · I27 · I31) ──

    /// 확정 ① — 맡김표가 지금의 맡김이면 칸을 비우며 같은 자리에서 `awaiting_reveal` 을 세운다(그 뒤의 둘째 맡기기는
    /// 짓기를 띄우지 않는다 · I25). `in_model` = 부른 쪽이 레이아웃 락 안에서 본 그 label 의 모델 항목 여부.
    // ADR-0299
    pub fn commit(&mut self, claim: &SpareClaim, in_model: bool, now: Instant) -> CommitDecision {
        match std::mem::replace(&mut self.slot, Slot::Empty) {
            Slot::Claimed {
                label,
                measures,
                generation,
                activate,
                opened_at,
            } if label == claim.label && generation == claim.generation => {
                if !in_model {
                    self.slot = Slot::Closing { label, generation };
                    return CommitDecision::Close;
                }
                self.awaiting_reveal = Some((label, generation));
                self.reveal_since = Some(now);
                CommitDecision::Adopt {
                    measures,
                    activate,
                    opened_at,
                }
            }
            other => {
                self.slot = other;
                CommitDecision::Stale
            }
        }
    }

    /// 확정이 닫기로 정한 창을 두 번 다 못 닫았다 — 칸을 비운다(다음 맡기기가 짓는다 · I31). `false` = 칸이 이미 그 창이
    /// 아니다(그사이 `Destroyed` 가 뺐다).
    pub fn close_failed(&mut self, claim: &SpareClaim) -> bool {
        let ours = matches!(&self.slot, Slot::Closing { label, generation }
            if *label == claim.label && *generation == claim.generation);
        if ours {
            self.slot = Slot::Empty;
        }
        ours
    }

    // ── 다시 짓기 완료 · `Destroyed`(I8 · I25) ──

    /// 다시 짓기 완료(보임 · 기다리던 label 의 `Destroyed` · 안전 시한) — 보임을 기다리는 label 이 그것이면 비우고
    /// `Some(짓나)`, 아니면 `None`(먼저 온 쪽이 이미 끝냈다). label 마다 한 번이다.
    // ADR-0299
    pub fn complete_reveal(&mut self, label: &str, now: Instant) -> Option<bool> {
        self.reveal_generation(label)?;
        self.awaiting_reveal = None;
        self.reveal_since = None;
        let build = !self.exiting && !self.disabled;
        if build {
            self.revealed_at = Some(now);
        }
        Some(build)
    }

    /// 보임 기다림이 `older_than` 넘게 섰으면 그 다시 짓기 완료를 지금 한다 — 안전 시한(확정 뒤 창 쪽이 건다)이 끝내지
    /// 못한 기다림의 그물이다. 남기면 그 실행 내내 짓기 자리를 잡지 못해 예열이 멈춘다. `Some` = 끝낸 기다림의 label ·
    /// 채택 번호(짓기 여부는 [`Self::complete_reveal`] 과 같다 — 맡기기의 `Empty` 갈래가 이어서 띄운다).
    // ADR-0299
    pub fn expire_reveal(
        &mut self,
        now: Instant,
        older_than: Duration,
    ) -> Option<(WindowLabel, u32)> {
        let since = self.reveal_since?;
        if now.saturating_duration_since(since) <= older_than {
            return None;
        }
        let (label, generation) = self.awaiting_reveal.clone()?;
        self.complete_reveal(&label, now);
        Some((label, generation))
    }

    pub fn destroyed(&mut self, label: &str, now: Instant) -> Gone {
        if self.contains(label) {
            let claimed = matches!(self.slot, Slot::Claimed { .. });
            let closing = matches!(self.slot, Slot::Closing { .. });
            if claimed {
                self.lost.insert(label.to_owned());
            }
            self.slot = Slot::Empty;
            return Gone::Removed {
                claimed,
                closing,
                rebuild: !self.exiting && !self.disabled,
            };
        }
        match self.complete_reveal(label, now) {
            Some(build) => Gone::RevealDone { build },
            None => Gone::NotOurs,
        }
    }

    // ── 듣기 실패(I28) ──

    pub fn listen(&mut self, label: &str, adoptable: bool) -> Listen {
        if !self.contains(label) {
            return Listen::NotPool;
        }
        if adoptable {
            self.listen_failures = 0;
            return Listen::Kept;
        }
        let claimed = matches!(self.slot, Slot::Claimed { .. });
        if claimed {
            self.lost.insert(label.to_owned());
        }
        self.slot = Slot::Empty;
        self.listen_failures = self.listen_failures.saturating_add(1);
        let disabled_now = !self.disabled && self.listen_failures >= LISTEN_FAILURE_LIMIT;
        if disabled_now {
            self.disabled = true;
        }
        Listen::Removed {
            claimed,
            disabled_now,
        }
    }

    pub fn tray_show(&self, label: &str) -> TrayShow {
        match &self.slot {
            Slot::Claimed { label: l, .. } if l == label => TrayShow::Rehold,
            _ if self.contains(label) => TrayShow::Untouched,
            _ => TrayShow::Adopted,
        }
    }

    // ── 재기 ──

    /// 칸의 창이 보낸 재기를 적는다 — `None` = 칸의 창이 아니다(부르는 쪽이 모델에 적는다 · 닫는 중인 창 포함). `Some(Err)`
    /// = 범위 밖 지표라 거절하고 직전 값을 유지한다. 0 크기 캔버스는 무시하고 `Some(Ok)` 다.
    pub fn record(&mut self, label: &str, measure: Measure) -> Option<Result<(), LayoutError>> {
        if !self.contains(label) {
            return None;
        }
        let m = self.slot.measures_mut()?;
        Some(match measure {
            Measure::Canvas { w, h } => {
                if let Some(canvas) = reported_canvas(w, h) {
                    m.canvas = Some(canvas);
                }
                Ok(())
            }
            Measure::Metrics(metrics) => check_metrics(&metrics).map(|()| {
                m.metrics = Some(metrics);
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::geometry::Insets;

    const P1: &str = "slot-popup-1";
    const P2: &str = "slot-popup-2";

    fn metrics() -> UiMetrics {
        UiMetrics {
            frame_insets: Insets {
                t: 1.0,
                r: 1.0,
                b: 1.0,
                l: 1.0,
            },
            min_pane_px: 50,
        }
    }

    fn full() -> SpareMeasures {
        SpareMeasures {
            canvas: Some(CanvasPx { w: 800, h: 600 }),
            metrics: Some(metrics()),
        }
    }

    fn report_all(pool: &mut SparePool, label: &str) {
        for measure in [
            Measure::Canvas { w: 800, h: 600 },
            Measure::Metrics(metrics()),
        ] {
            assert_eq!(pool.record(label, measure), Some(Ok(())));
        }
    }

    fn building(label: &str) -> SparePool {
        let mut pool = SparePool::default();
        pool.reserve().unwrap();
        assert!(pool.assign(label.to_owned()));
        pool
    }

    fn built(label: &str) -> SparePool {
        let mut pool = building(label);
        assert!(pool.mark_built(label));
        pool
    }

    fn claimed(label: &str) -> (SparePool, SpareClaim) {
        let mut pool = built(label);
        let ClaimStep::Claimed(claim) = pool.take_built(None) else {
            panic!("지어진 창을 맡긴다");
        };
        (pool, claim)
    }

    fn now() -> Instant {
        Instant::now()
    }

    // ── 짓기 자리 ──

    #[test]
    fn the_build_place_is_taken_before_the_label_and_only_once() {
        let mut pool = SparePool::default();
        assert_eq!(pool.reserve(), Ok(None));
        assert_eq!(
            pool.slot(),
            &Slot::Building {
                label: None,
                measures: SpareMeasures::default()
            }
        );
        assert!(!pool.contains(P1), "label 은 아직 없다");
        assert_eq!(pool.reserve(), Err(NoBuild::Occupied));
        assert!(pool.assign(P1.to_owned()));
        assert!(!pool.assign(P2.to_owned()), "자리 하나에 label 하나");
        assert!(pool.contains(P1));
        assert_eq!(pool.reserve(), Err(NoBuild::Occupied));
    }

    #[test]
    fn no_build_place_while_exiting_disabled_or_awaiting_a_reveal() {
        let mut pool = SparePool::default();
        pool.awaiting_reveal = Some((P1.to_owned(), 1));
        assert_eq!(pool.reserve(), Err(NoBuild::AwaitingReveal));
        pool.disabled = true;
        assert_eq!(pool.reserve(), Err(NoBuild::Disabled));
        pool.begin_exit();
        assert_eq!(pool.reserve(), Err(NoBuild::Exiting));
        assert_eq!(pool.slot(), &Slot::Empty);
    }

    #[test]
    fn a_build_failure_frees_only_its_own_place() {
        let mut pool = building(P1);
        assert!(!pool.build_failed(P2));
        assert!(pool.build_failed(P1));
        assert_eq!(pool.slot(), &Slot::Empty);
        assert!(!pool.mark_built(P1), "실패한 짓기는 지어짐을 세우지 못한다");
    }

    // ── 맡기기 ──

    #[test]
    fn only_a_built_spare_is_handed_out_and_each_claim_gets_a_new_generation() {
        assert_eq!(building(P1).take_built(None), ClaimStep::Wait { build: 1 });
        assert_eq!(
            SparePool::default().take_built(None),
            ClaimStep::Fresh { build: true }
        );
        let (mut pool, first) = claimed(P1);
        assert_eq!(first.label, P1);
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
        assert!(pool.release(&first));
        let ClaimStep::Claimed(second) = pool.take_built(None) else {
            panic!("돌려받은 창을 다시 맡긴다");
        };
        assert_eq!(second.label, P1);
        assert!(second.generation > first.generation);
    }

    #[test]
    fn an_empty_slot_asks_for_a_build_only_without_a_pending_reveal() {
        let mut pool = SparePool::default();
        pool.awaiting_reveal = Some((P1.to_owned(), 1));
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
    }

    #[test]
    fn nothing_is_handed_out_while_exiting_or_disabled() {
        let disable = |p: &mut SparePool| p.disabled = true;
        for stop in [SparePool::begin_exit as fn(&mut SparePool), disable] {
            let mut pool = built(P1);
            stop(&mut pool);
            assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
            assert!(matches!(pool.slot(), Slot::Built { .. }));
        }
    }

    #[test]
    fn opening_tells_a_claimed_label_from_a_lost_or_fresh_one() {
        let (mut pool, claim) = claimed(P1);
        assert_eq!(pool.opening(P2, true, now()), Opening::Fresh);
        assert_eq!(pool.opening(P1, false, now()), Opening::Claimed);
        assert!(matches!(
            pool.slot(),
            Slot::Claimed {
                activate: false,
                opened_at: Some(_),
                ..
            }
        ));
        assert!(matches!(
            pool.destroyed(&claim.label, now()),
            Gone::Removed { claimed: true, .. }
        ));
        assert_eq!(pool.opening(P1, true, now()), Opening::Unavailable);
        let mut pool = built(P1);
        assert_eq!(
            pool.opening(P1, true, now()),
            Opening::Unavailable,
            "맡기지 않은 예열 창 label 로는 열지 않는다"
        );
    }

    // ── 확정 ──

    #[test]
    fn commit_empties_the_slot_and_sets_the_pending_reveal_in_one_step() {
        let (mut pool, claim) = claimed(P1);
        report_all(&mut pool, P1);
        assert!(matches!(
            pool.commit(&claim, true, now()),
            CommitDecision::Adopt { measures, activate: true, .. } if measures == full()
        ));
        assert_eq!(pool.slot(), &Slot::Empty);
        assert_eq!(pool.awaiting_reveal(), Some((P1, claim.generation)));
        assert_eq!(pool.reveal_generation(P1), Some(claim.generation));
        assert!(!pool.contains(P1), "확정 뒤는 모델 창이다");
        assert_eq!(pool.commit(&claim, true, now()), CommitDecision::Stale);
    }

    #[test]
    fn commit_for_a_label_gone_from_the_model_keeps_it_closing() {
        let (mut pool, claim) = claimed(P1);
        assert_eq!(pool.commit(&claim, false, now()), CommitDecision::Close);
        assert_eq!(
            pool.slot(),
            &Slot::Closing {
                label: P1.to_owned(),
                generation: claim.generation
            }
        );
        assert_eq!(pool.awaiting_reveal(), None);
        assert!(pool.contains(P1));
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: false });
        assert!(!pool.release(&claim), "닫기로 정한 창은 돌아가지 않는다");
        assert_eq!(pool.commit(&claim, true, now()), CommitDecision::Stale);
        assert!(pool.close_failed(&claim));
        assert_eq!(pool.slot(), &Slot::Empty);
        assert!(!pool.close_failed(&claim));
    }

    // ── 다시 짓기 완료 ──

    #[test]
    fn a_reveal_completion_while_disabled_clears_without_building() {
        let (mut pool, claim) = claimed(P1);
        pool.commit(&claim, true, now());
        pool.disabled = true;
        assert_eq!(pool.complete_reveal(P1, now()), Some(false));
        assert_eq!(pool.awaiting_reveal(), None);
        assert_eq!(pool.complete_reveal(P1, now()), None);
    }

    #[test]
    fn a_pending_reveal_older_than_the_backstop_is_completed_once_and_frees_the_build_place() {
        let (mut pool, claim) = claimed(P1);
        let at = now();
        pool.commit(&claim, true, at);
        let limit = Duration::from_secs(5);
        assert_eq!(
            pool.expire_reveal(at + limit, limit),
            None,
            "상한까지는 둔다"
        );
        assert_eq!(pool.reserve(), Err(NoBuild::AwaitingReveal));
        let late = at + limit + Duration::from_millis(1);
        assert_eq!(
            pool.expire_reveal(late, limit),
            Some((P1.to_owned(), claim.generation))
        );
        assert_eq!(pool.awaiting_reveal(), None);
        assert_eq!(
            pool.complete_reveal(P1, late),
            None,
            "끝난 기다림의 늦은 완료는 무동작"
        );
        assert_eq!(pool.expire_reveal(late, limit), None);
        assert_eq!(pool.take_built(None), ClaimStep::Fresh { build: true });
        assert_eq!(pool.reserve(), Ok(Some(late)));
    }

    #[test]
    fn a_completed_reveal_leaves_nothing_to_expire() {
        let (mut pool, claim) = claimed(P1);
        let at = now();
        pool.commit(&claim, true, at);
        assert_eq!(pool.complete_reveal(P1, at), Some(true));
        assert_eq!(
            pool.expire_reveal(at + Duration::from_secs(60), Duration::ZERO),
            None
        );
    }

    #[test]
    fn the_reveal_time_is_handed_to_the_next_build_once() {
        let (mut pool, claim) = claimed(P1);
        pool.commit(&claim, true, now());
        let at = now();
        assert_eq!(pool.complete_reveal(P1, at), Some(true));
        assert_eq!(pool.reserve(), Ok(Some(at)));
        pool.slot = Slot::Empty;
        assert_eq!(pool.reserve(), Ok(None));
    }

    // ── 트레이 보이기 ──

    #[test]
    fn a_tray_show_tells_claimed_adopted_and_untouched_apart() {
        assert_eq!(built(P1).tray_show(P1), TrayShow::Untouched);
        assert_eq!(building(P1).tray_show(P1), TrayShow::Untouched);
        let (mut pool, claim) = claimed(P1);
        assert_eq!(pool.tray_show(P1), TrayShow::Rehold);
        pool.commit(&claim, true, now());
        assert_eq!(pool.tray_show(P1), TrayShow::Adopted);
    }

    // ── 재기 받기 ──

    #[test]
    fn a_label_outside_the_slot_is_left_to_the_model() {
        let mut pool = SparePool::default();
        assert_eq!(pool.record(P1, Measure::Canvas { w: 800, h: 600 }), None);
        assert_eq!(pool, SparePool::default());
        let (mut pool, claim) = claimed(P1);
        pool.commit(&claim, false, now());
        assert_eq!(
            pool.record(P1, Measure::Canvas { w: 800, h: 600 }),
            None,
            "닫는 중인 창은 쥘 재기가 없다"
        );
    }

    #[test]
    fn measures_follow_the_window_from_building_to_claimed_and_back() {
        let mut pool = building(P1);
        report_all(&mut pool, P1);
        pool.mark_built(P1);
        assert_eq!(
            pool.slot(),
            &Slot::Built {
                label: P1.to_owned(),
                measures: full()
            }
        );
        let ClaimStep::Claimed(claim) = pool.take_built(None) else {
            panic!("맡긴다");
        };
        assert_eq!(
            pool.record(P1, Measure::Canvas { w: 1024, h: 768 }),
            Some(Ok(()))
        );
        pool.release(&claim);
        assert!(matches!(
            pool.slot(),
            Slot::Built { measures, .. } if measures.canvas == Some(CanvasPx { w: 1024, h: 768 })
        ));
    }

    #[test]
    fn a_zero_canvas_is_ignored_and_keeps_the_previous_value() {
        let mut pool = built(P1);
        report_all(&mut pool, P1);
        for (w, h) in [(0, 600), (800, 0), (0, 0)] {
            assert_eq!(pool.record(P1, Measure::Canvas { w, h }), Some(Ok(())));
        }
        assert!(matches!(
            pool.slot(),
            Slot::Built { measures, .. } if measures.canvas == Some(CanvasPx { w: 800, h: 600 })
        ));
    }

    #[test]
    fn out_of_range_metrics_are_refused_and_keep_the_previous_value() {
        let mut pool = built(P1);
        report_all(&mut pool, P1);
        let bad = UiMetrics {
            min_pane_px: 0,
            ..metrics()
        };
        assert!(matches!(
            pool.record(P1, Measure::Metrics(bad)),
            Some(Err(LayoutError::InvalidMetrics(_)))
        ));
        assert!(matches!(
            pool.slot(),
            Slot::Built { measures, .. } if measures.metrics == Some(metrics())
        ));
    }
}
