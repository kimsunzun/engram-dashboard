//! 셸의 사용량 관심 — 레이아웃에서 데몬 사용량 구독 집합을 계산하고, 그 구독을 데몬과 맞춰 두는 상태기
//! (TRD S21 usage-limit-slot §1-7).
//!
//! ## 무엇을 소유하나
//! - **관심** = 숨김 표시가 없는 창마다 그 창 **활성 탭**의 사용량 슬롯이 켠 회사의 합집합
//!   ([`interest_from_layout`]). 최소화는 셸이 추적하지 않으므로 보임이다(사용자 결정 2026-09-27). 어느 창의
//!   탭도 아닌 뷰(슬롯 옮기기의 임시 뷰)는 빠진다. 모델에만 남은 창(OS 창 없이 모델에 남은 창 — `layout::apply`
//!   의 `WindowHost` doc 이 적은 미해결)은 보임으로 센다.
//! - **방송 받을 창** = 어느 탭이든 사용량 슬롯이 있는 창, 숨은 창 포함([`usage_slot_windows`]) — 다시 보이면
//!   곧바로 최신이다.
//! - [`UsageInterest`] — 그 둘의 마지막 값 · 이 소켓에 마지막으로 보낸 집합(`sent`) · 줄임 지연 · 이 소켓에서
//!   받은 스냅숏 캐시.
//!
//! ## ★순수 — Tauri·소켓·tokio·시계 모름★
//! 시각은 호출자가 넣는다(`now`). 메서드는 결과만 돌려주고 소켓 쓰기·emit·타이머 기동은 호출자 몫이다 —
//! 그래서 시험은 가짜 시각으로 결정론이다(`cargo test -p engram-dashboard --test lib_unit`).
//!
//! ## ★한 직렬 경로 — 보낼 집합은 연결 태스크만 꺼낸다★
//! 관심을 바꾸는 쪽(레이아웃 재계산 · 숨김 · 줄임 타이머)은 집합 없는 넛지만 넣고, 연결 태스크가 그것을 꺼낼
//! 때 [`UsageInterest::sync`] 로 **그때의** 관심을 읽어 보낸다 — 읽는 순서 = 보내는 순서라 소켓의 마지막 한
//! 장이 늘 최신 관심이다. 호출자가 락 안에서 집합을 읽어 락 밖에서 보내는 모양은 두 변경이 읽기와 전송 사이에서
//! 엇갈리면 옛 집합이 마지막으로 닿는다 — 되살리지 말 것(TRD §3 #74). 그래서 [`InterestAction`] 은 집합을
//! 싣지 않는다.
//!
//! ## ★소켓 표식 가드★
//! 연결 태스크가 부르는 메서드는 모두 자기 소켓 표식(`socket_epoch` — ADR-0195 · `0` = 소켓 없음)을 싣고,
//! 지금 표식이 아니면(여닫기의 열기는 본 가장 큰 표식보다 크지 않으면) 아무것도 안 한다 — 승계로 밀려난
//! 태스크의 늦은 호출이 새 소켓의 `sent`·캐시를 덮지 못하게. 대소 비교가 뜻을 갖는 것은 표식 채번기가
//! 되감기지 않기 때문이고(`lifecycle.rs`), 그래서 셸 프로세스 수명 안에서만 성립한다.
//!
//! ## 락 차례
//! 이 상태를 감싸는 락은 `ViewManager → 사용량 관심 → lifecycle` 차례의 가운데다 — 잎이 **아니다**(넛지를
//! 넣는 자리가 lifecycle 락을 잡는다). 이 파일은 락을 모른다.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use engram_dashboard_protocol::{
    AgentBackendKind, UsageLimitSnapshot, UsageVendorState, UsageWindow,
};

use crate::layout::manager::WindowLabel;
use crate::layout::{LayoutNode, SlotContent, ViewManager};

/// 관심이 줄 때 데몬에 보내기까지 기다리는 시간 — 탭을 다른 창으로 떼는 동안(옛 창에서 빠짐 → 새 창에
/// 듦)처럼 곧 되돌아오는 줄임을 데몬에 보내지 않으려는 창이다. 늘어남은 기다리지 않는다.
pub const USAGE_INTEREST_SHRINK_DELAY: Duration = Duration::from_millis(1500);

/// 숨김 표시가 없는 창마다 **활성 탭**의 사용량 슬롯이 켠 회사의 합집합.
pub fn interest_from_layout(
    mgr: &ViewManager,
    hidden: &BTreeSet<WindowLabel>,
) -> BTreeSet<AgentBackendKind> {
    let mut interest = BTreeSet::new();
    for (label, tabs) in &mgr.windows {
        if hidden.contains(label) {
            continue;
        }
        let Some(view) = mgr.views.get(&tabs.active) else {
            continue;
        };
        for_each_usage_slot(&view.layout, &mut |show_claude, show_codex| {
            if show_claude {
                interest.insert(AgentBackendKind::Claude);
            }
            if show_codex {
                interest.insert(AgentBackendKind::Codex);
            }
        });
    }
    interest
}

/// 어느 탭이든(활성 아닌 탭 · 숨은 창 포함) 사용량 슬롯이 있는 창 — 켠 회사가 하나도 없는 슬롯도 든다.
pub fn usage_slot_windows(mgr: &ViewManager) -> BTreeSet<WindowLabel> {
    mgr.windows
        .iter()
        .filter(|(_, tabs)| {
            tabs.tabs.iter().any(|view| {
                mgr.views.get(view).is_some_and(|view| {
                    let mut found = false;
                    for_each_usage_slot(&view.layout, &mut |_, _| found = true);
                    found
                })
            })
        })
        .map(|(label, _)| label.clone())
        .collect()
}

fn for_each_usage_slot(node: &LayoutNode, visit: &mut dyn FnMut(bool, bool)) {
    match node {
        LayoutNode::Slot {
            content:
                SlotContent::Usage {
                    show_claude,
                    show_codex,
                },
            ..
        } => visit(*show_claude, *show_codex),
        LayoutNode::Slot { .. } => {}
        LayoutNode::Split { a, b, .. } => {
            for_each_usage_slot(a, visit);
            for_each_usage_slot(b, visit);
        }
    }
}

/// 줄임 지연 한 번의 세대. 지연 중에 관심이 다시 바뀌면 새 세대가 서거나 지연이 무효가 되어, 뒤늦게 깬 옛
/// 타이머는 [`UsageInterest::deferral_elapsed`] 에서 `false` 를 받는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShrinkGen(u64);

/// 관심 재계산이 호출자에게 시키는 일 — `sent`(이 소켓에 마지막으로 보낸 집합)와 비교한 결과다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterestAction {
    /// 관심 = `sent` — 할 일이 없다. 대기 중이던 줄임도 무효가 됐다.
    Unchanged,
    /// 관심에 `sent` 밖의 회사가 있다 — 곧바로 넛지 한 장.
    Nudge,
    /// 관심 ⊊ `sent` — `deadline` 에 [`UsageInterest::deferral_elapsed`]`(gen)` 를 불러 `true` 면 넛지 한 장.
    Defer { gen: ShrinkGen, deadline: Instant },
}

/// [`UsageInterest::on_snapshot`] 의 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotOutcome {
    /// 그 스냅숏을 전달할 창(= [`usage_slot_windows`] 의 마지막 값). revision 과 무관하게 전달한다 — 가르는 것은
    /// 웹뷰다. 지금 소켓의 스냅숏이 아니면 빈다.
    pub labels: Vec<WindowLabel>,
    /// 같은 소켓에 곧바로 쓸 `UsageSubscribe` 집합 — 데몬이 실어 온 구독 집합이 셸이 데몬에 있기를 바라는
    /// 집합과 다를 때만 선다. 다를 때마다 선다(억제 없음 — [`UsageInterest::on_snapshot`]).
    pub resend: Option<BTreeSet<AgentBackendKind>>,
}

/// 웹뷰 pull 에 건넬 것 — 캐시 전부(핸드오프 보정 뒤, 회사 순)와 지금 소켓 표식. 소켓이 없으면 `0` 과 빈 목록.
#[derive(Debug, Clone, PartialEq)]
pub struct WebviewUsageSnapshot {
    pub socket_epoch: u64,
    pub snapshots: Vec<UsageLimitSnapshot>,
}

#[derive(Debug)]
struct Cached {
    snapshot: UsageLimitSnapshot,
    received: Instant,
}

/// 셸 하나의 사용량 관심 상태. 셸이 여는 데몬 연결은 하나라 캐시 열쇠 (연결, 회사) 의 연결 칸은 「지금 소켓」
/// 곧 `socket_epoch` 다 — 소켓이 바뀌면 캐시를 비운다.
#[derive(Debug, Default)]
pub struct UsageInterest {
    /// 셸이 숨긴 창(트레이). 소멸한 창의 label 은 다음 재계산이 지운다.
    hidden: BTreeSet<WindowLabel>,
    interest: BTreeSet<AgentBackendKind>,
    usage_windows: BTreeSet<WindowLabel>,
    socket_epoch: u64,
    max_seen_socket_epoch: u64,
    sent: BTreeSet<AgentBackendKind>,
    cache: BTreeMap<AgentBackendKind, Cached>,
    shrink_seq: u64,
    /// ★불변식 — 서 있으면 `interest ⊊ sent` 다★(줄임을 세우는 것도 무르는 것도 재계산 · 보냄 · 소켓 여닫기뿐).
    shrink_pending: Option<ShrinkGen>,
}

impl UsageInterest {
    pub fn new() -> Self {
        Self::default()
    }

    /// 레이아웃·보임이 바뀐 뒤 관심과 방송 받을 창을 새로 계산한다 — ViewManager 락 안에서 부른다.
    pub fn recompute(&mut self, mgr: &ViewManager, now: Instant) -> InterestAction {
        // 팝업 label 은 다시 쓰이지 않아 남겨 두면 숨긴 채 닫힌 팝아웃마다 한 칸씩 샌다. OS 창은 모델 창보다
        // 먼저 서지 않으므로 모델에 없는 label 의 숨김 표시는 가리킬 창이 없다.
        self.hidden.retain(|label| mgr.windows.contains_key(label));
        self.interest = interest_from_layout(mgr, &self.hidden);
        self.usage_windows = usage_slot_windows(mgr);
        if !self.interest.is_subset(&self.sent) {
            self.shrink_pending = None;
            InterestAction::Nudge
        } else if self.interest != self.sent {
            self.shrink_seq += 1;
            let gen = ShrinkGen(self.shrink_seq);
            self.shrink_pending = Some(gen);
            InterestAction::Defer {
                gen,
                deadline: now + USAGE_INTEREST_SHRINK_DELAY,
            }
        } else {
            self.shrink_pending = None;
            InterestAction::Unchanged
        }
    }

    /// 셸이 창 `label` 을 숨기거나(`false`) 다시 보인 뒤(`true`) — 숨김 표시를 고치고 [`Self::recompute`] 와 같다.
    pub fn set_visible(
        &mut self,
        label: &str,
        visible: bool,
        mgr: &ViewManager,
        now: Instant,
    ) -> InterestAction {
        if visible {
            self.hidden.remove(label);
        } else {
            self.hidden.insert(label.to_owned());
        }
        self.recompute(mgr, now)
    }

    /// 줄임 타이머가 기한에 부른다. `true` = 그 줄임이 아직 유효했다 → 넛지 한 장.
    ///
    /// ★대기는 전달이 아니라 기한에 끝난다★ — 그 넛지가 사라져도(명령 채널 포화) 대기는 이미 끝났으므로 다음
    /// 스냅숏의 대조가 관심과 비교해 고친다([`Self::on_snapshot`]).
    pub fn deferral_elapsed(&mut self, gen: ShrinkGen) -> bool {
        if self.shrink_pending == Some(gen) {
            self.shrink_pending = None;
            true
        } else {
            false
        }
    }

    /// 연결 태스크가 소켓 `socket_epoch` 에 쓸 `UsageSubscribe` 집합 — `None` = 쓸 것 없음.
    ///
    /// - `force` = ⟳ 새로고침 프레임을 쓰기 직전 — 늘 관심을 준다(같은 집합이면 데몬에서 멱등).
    /// - 넛지를 꺼낼 때(`force = false`): 대기 중인 줄임이 있으면 관심에 `sent` 밖 회사가 있을 때만(순수 줄임은
    ///   기한의 몫), 없으면 관심 ≠ `sent` 일 때만.
    ///
    /// `Some` 이면 `sent := 관심` · 대기 중인 줄임 무효. `sent` 기준이라 거푸 불러도 둘째부터 `None` 이다(멱등 —
    /// 새 소켓의 `cmd_tx` 저장 직후 넛지가 여기 기댄다). 지금 소켓이 아니면 `None`.
    pub fn sync(&mut self, socket_epoch: u64, force: bool) -> Option<BTreeSet<AgentBackendKind>> {
        if !self.is_current(socket_epoch) {
            self.log_foreign("sync", socket_epoch);
            return None;
        }
        let send = force
            || if self.shrink_pending.is_some() {
                !self.interest.is_subset(&self.sent)
            } else {
                self.interest != self.sent
            };
        if !send {
            return None;
        }
        self.sent = self.interest.clone();
        self.shrink_pending = None;
        Some(self.interest.clone())
    }

    /// 새 소켓이 명령 창구를 연 뒤, 명령 채널을 읽기 전에 부른다 — `Some` 이면 그 집합을 새 소켓에 직접 한 장.
    ///
    /// 본 가장 큰 표식보다 큰 표식에서만 움직인다(그 밖엔 `None` · 아무것도 안 바뀜). 움직이면 캐시를 비우고
    /// `sent := 관심` · 대기 중인 줄임 무효이며, 관심이 비었으면 보낼 것이 없어 `None` 이다(새 연결의 데몬 쪽
    /// 구독은 빈 집합이다).
    pub fn on_socket_open(&mut self, socket_epoch: u64) -> Option<BTreeSet<AgentBackendKind>> {
        if socket_epoch <= self.max_seen_socket_epoch {
            tracing::debug!(
                socket_epoch,
                max_seen = self.max_seen_socket_epoch,
                "사용량 관심: 본 적 있는 소켓 표식의 열기 — 무시(승계로 밀려난 태스크)"
            );
            return None;
        }
        self.max_seen_socket_epoch = socket_epoch;
        self.socket_epoch = socket_epoch;
        self.cache.clear();
        self.sent = self.interest.clone();
        self.shrink_pending = None;
        (!self.interest.is_empty()).then(|| self.interest.clone())
    }

    /// 소켓 `socket_epoch` 가 끝났다(사유 무관). 지금 소켓일 때만 캐시를 비우고 `sent := ∅` · 소켓 없음으로.
    /// 대기 중인 줄임도 무효다 — 줄일 `sent` 가 사라졌고, 새 소켓은 열 때 관심을 통째로 다시 보낸다.
    pub fn on_socket_lost(&mut self, socket_epoch: u64) {
        if !self.is_current(socket_epoch) {
            self.log_foreign("on_socket_lost", socket_epoch);
            return;
        }
        self.cache.clear();
        self.sent.clear();
        self.shrink_pending = None;
        self.socket_epoch = 0;
    }

    /// 소켓 `socket_epoch` 로 스냅숏 한 장이 왔다 — `subscribed` = 데몬이 그 한 장에 실은 그 연결의 구독 집합.
    ///
    /// - 캐시 = 회사당 **최고** revision — 더 작은 revision 이 늦게 닿아도 바꾸지 않고, 같거나 크면 바꾼다(받은
    ///   시각 = `now`). 데몬이 첫 스냅숏을 명부 락 밖에서 보내 마지막 도착이 최고가 아닐 수 있다.
    /// - ★대조★: 바라는 집합 = 대기 중인 줄임이 있으면 `sent`, 없으면 관심. `subscribed` 가 그것과 다르면
    ///   `resend` = 바라는 집합이다. 대기 중인 줄임이 있으면 대기·`sent` 를 그대로 둔다 — 1.5초 창 안에 닿은 낡은
    ///   한 장이 줄임을 앞당기면 안 된다. 없으면 `sent := 관심`.
    /// - ★어긋날 때마다 되보낸다 — 억제하지 말 것★: 같은 짝을 억제하던 모양은 앞선 토글 왕복이 남긴 낡은 기록과
    ///   데몬이 거절한 교정 재전송이 같은 짝을 다시 싣고 와 필요한 재전송을 막았다(TRD §3 #84). 되먹임이 유한한
    ///   것은 데몬의 같은 집합 교체가 첫 스냅숏을 안 내기 때문이다. 어긋남마다 `debug` 한 줄.
    ///
    /// 지금 소켓이 아니면 아무것도 안 한다(빈 결과 · 캐시 불변).
    pub fn on_snapshot(
        &mut self,
        socket_epoch: u64,
        snapshot: &UsageLimitSnapshot,
        subscribed: &[AgentBackendKind],
        now: Instant,
    ) -> SnapshotOutcome {
        if !self.is_current(socket_epoch) {
            self.log_foreign("on_snapshot", socket_epoch);
            return SnapshotOutcome::default();
        }
        let older = self
            .cache
            .get(&snapshot.vendor)
            .is_some_and(|cached| cached.snapshot.revision > snapshot.revision);
        if !older {
            self.cache.insert(
                snapshot.vendor,
                Cached {
                    snapshot: snapshot.clone(),
                    received: now,
                },
            );
        }
        let subscribed: BTreeSet<AgentBackendKind> = subscribed.iter().copied().collect();
        let shrink_pending = self.shrink_pending.is_some();
        let desired = if shrink_pending {
            self.sent.clone()
        } else {
            self.interest.clone()
        };
        let resend = if subscribed == desired {
            None
        } else {
            tracing::debug!(
                ?subscribed,
                ?desired,
                shrink_pending,
                socket_epoch,
                "사용량 구독 어긋남 — 바라는 집합을 다시 보낸다"
            );
            if !shrink_pending {
                self.sent = desired.clone();
            }
            Some(desired)
        };
        SnapshotOutcome {
            labels: self.usage_windows.iter().cloned().collect(),
            resend,
        }
    }

    /// 웹뷰 pull 이 부른다 — 캐시 전부를 핸드오프 보정해 건넨다(데몬 왕복 없음).
    ///
    /// ★핸드오프 보정★: 받은 뒤 흐른 정수 초 `d` 만큼 상대 칸을 옮긴다 — 창마다(모델별 포함) `age_secs + d` ·
    /// `Failed.next_attempt_in_secs`·`Rejected.retry_in_secs` 는 `− d`(0 포화). 안 하면 오래된 캐시(길게는
    /// 15분)가 값을 그만큼 젊게·대기를 그만큼 길게 보인다.
    pub fn snapshot_for_webview(&self, now: Instant) -> WebviewUsageSnapshot {
        WebviewUsageSnapshot {
            socket_epoch: self.socket_epoch,
            snapshots: self
                .cache
                .values()
                .map(|cached| {
                    handed_off(
                        &cached.snapshot,
                        now.saturating_duration_since(cached.received).as_secs(),
                    )
                })
                .collect(),
        }
    }

    fn is_current(&self, socket_epoch: u64) -> bool {
        socket_epoch != 0 && socket_epoch == self.socket_epoch
    }

    fn log_foreign(&self, call: &'static str, socket_epoch: u64) {
        tracing::debug!(
            call,
            socket_epoch,
            current = self.socket_epoch,
            "사용량 관심: 지금 소켓이 아닌 표식 — 무시"
        );
    }
}

fn handed_off(snapshot: &UsageLimitSnapshot, elapsed_secs: u64) -> UsageLimitSnapshot {
    let mut out = snapshot.clone();
    let age = |window: &mut UsageWindow| {
        window.age_secs = window.age_secs.saturating_add(elapsed_secs);
    };
    out.five_hour.iter_mut().for_each(age);
    out.weekly.iter_mut().for_each(age);
    out.model_scoped
        .iter_mut()
        .for_each(|scoped| age(&mut scoped.window));
    // 와일드카드를 두지 않는다 — 상대 초를 싣는 상태가 늘면 여기서 컴파일이 깨져야 한다.
    match &mut out.state {
        UsageVendorState::Failed {
            next_attempt_in_secs,
            ..
        } => *next_attempt_in_secs = next_attempt_in_secs.saturating_sub(elapsed_secs),
        UsageVendorState::Rejected { retry_in_secs, .. } => {
            *retry_in_secs = retry_in_secs.saturating_sub(elapsed_secs)
        }
        UsageVendorState::Ready
        | UsageVendorState::NotInstalled { .. }
        | UsageVendorState::NeedsLogin { .. }
        | UsageVendorState::Unavailable { .. } => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use engram_dashboard_protocol::UsageScopedWindow;
    use uuid::Uuid;

    use super::*;
    use crate::layout::manager::ViewId;
    use crate::layout::{tree, SplitDir, MAIN_WINDOW_LABEL};

    use AgentBackendKind::{Claude, Codex};

    const POPUP: &str = "slot-popup-1";

    fn set(vendors: &[AgentBackendKind]) -> BTreeSet<AgentBackendKind> {
        vendors.iter().copied().collect()
    }

    fn labels(names: &[&str]) -> BTreeSet<WindowLabel> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn usage(show_claude: bool, show_codex: bool) -> SlotContent {
        SlotContent::Usage {
            show_claude,
            show_codex,
        }
    }

    fn root_slot(mgr: &ViewManager, view: ViewId) -> Uuid {
        tree::first_slot_id(&mgr.views[&view].layout)
    }

    fn put(mgr: &mut ViewManager, view: ViewId, content: SlotContent) -> Uuid {
        let slot = root_slot(mgr, view);
        mgr.set_slot_content(view, slot, content).unwrap();
        slot
    }

    fn main_active(mgr: &ViewManager) -> ViewId {
        mgr.windows[MAIN_WINDOW_LABEL].active
    }

    /// main 의 활성 탭 = 사용량 슬롯 하나(`show_claude`·`show_codex`).
    fn main_with(show_claude: bool, show_codex: bool) -> (ViewManager, Uuid) {
        let mut mgr = ViewManager::new();
        let view = main_active(&mgr);
        let slot = put(&mut mgr, view, usage(show_claude, show_codex));
        (mgr, slot)
    }

    fn window(age_secs: u64) -> UsageWindow {
        UsageWindow {
            used_pct: Some(10.0),
            resets_at: Some(1_900_000_000),
            age_secs,
            expired: false,
        }
    }

    fn snap(vendor: AgentBackendKind, revision: u64) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor,
            account_key: "default".to_string(),
            five_hour: Some(window(10)),
            weekly: Some(window(20)),
            model_scoped: vec![UsageScopedWindow {
                label: "opus".to_string(),
                window: window(30),
            }],
            plan: None,
            in_flight: false,
            state: UsageVendorState::Ready,
            revision,
        }
    }

    /// 소켓 1 이 열려 있고 `sent` = 관심인 상태(관심은 `mgr` 에서).
    fn connected(mgr: &ViewManager, t0: Instant) -> UsageInterest {
        let mut ui = UsageInterest::new();
        ui.recompute(mgr, t0);
        ui.on_socket_open(1);
        ui
    }

    fn defer_gen(action: InterestAction) -> ShrinkGen {
        match action {
            InterestAction::Defer { gen, .. } => gen,
            other => panic!("줄임 지연이어야 한다: {other:?}"),
        }
    }

    // ── 레이아웃 → 관심 ──────────────────────────────────────────────────────────

    #[test]
    fn the_active_tab_counts_and_an_inactive_tab_does_not() {
        let (mut mgr, _) = main_with(true, false);
        let usage_tab = main_active(&mgr);
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));

        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[]));
        assert_eq!(
            usage_slot_windows(&mgr),
            labels(&[MAIN_WINDOW_LABEL]),
            "활성 아닌 탭의 사용량 슬롯도 방송은 받는다"
        );

        mgr.switch_tab(MAIN_WINDOW_LABEL, usage_tab).unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));
    }

    /// 숨김 표시만 뺀다 — 최소화는 셸이 모르므로 숨김 표시가 없는 창은 (최소화돼 있어도) 보임이다.
    #[test]
    fn only_the_hidden_mark_drops_a_window() {
        let (mgr, _) = main_with(true, true);
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[])),
            set(&[Claude, Codex])
        );
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[MAIN_WINDOW_LABEL])),
            set(&[])
        );
        assert_eq!(usage_slot_windows(&mgr), labels(&[MAIN_WINDOW_LABEL]));
    }

    #[test]
    fn two_windows_union_their_vendors() {
        let (mut mgr, _) = main_with(true, false);
        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(false, true));
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[])),
            set(&[Claude, Codex])
        );
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[POPUP])),
            set(&[Claude])
        );
        assert_eq!(
            usage_slot_windows(&mgr),
            labels(&[MAIN_WINDOW_LABEL, POPUP])
        );
    }

    #[test]
    fn two_slots_in_one_tab_union_and_a_toggle_off_drops_its_vendor() {
        let (mut mgr, slot) = main_with(true, false);
        let view = main_active(&mgr);
        let other = mgr.split_slot(view, slot, SplitDir::LeftRight).unwrap();
        mgr.set_slot_content(view, other, usage(false, true))
            .unwrap();
        assert_eq!(
            interest_from_layout(&mgr, &labels(&[])),
            set(&[Claude, Codex])
        );

        mgr.set_slot_content(view, other, usage(false, false))
            .unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[Claude]));
        assert_eq!(
            usage_slot_windows(&mgr),
            labels(&[MAIN_WINDOW_LABEL]),
            "둘 다 끈 사용량 슬롯도 사용량 슬롯이다"
        );
    }

    /// 슬롯 옮기기 phase A 의 임시 뷰는 어느 창의 탭도 아니다.
    #[test]
    fn a_view_owned_by_no_window_drops_out() {
        let (mut mgr, slot) = main_with(true, true);
        let view = main_active(&mgr);
        mgr.prepare_detached_view(view, slot, "tmp".to_string())
            .unwrap();
        mgr.set_slot_content(view, slot, SlotContent::Empty)
            .unwrap();
        assert_eq!(interest_from_layout(&mgr, &labels(&[])), set(&[]));
        assert_eq!(usage_slot_windows(&mgr), labels(&[]));
    }

    #[test]
    fn a_closed_popup_drops_out_and_its_hidden_mark_is_forgotten() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(true, false));
        let mut ui = connected(&mgr, t0);
        assert_eq!(ui.sent, set(&[Claude]));

        assert!(matches!(
            ui.set_visible(POPUP, false, &mgr, t0),
            InterestAction::Defer { .. }
        ));
        assert!(ui.hidden.contains(POPUP));

        mgr.close_window(POPUP).unwrap();
        ui.recompute(&mgr, t0);
        assert!(ui.hidden.is_empty(), "소멸한 창의 숨김 표시가 남았다");
        assert_eq!(ui.interest, set(&[]));
        assert!(ui.usage_windows.is_empty());
    }

    // ── 늘어남·줄어듦 (가짜 시각) ─────────────────────────────────────────────────

    #[test]
    fn growth_nudges_at_once_and_sync_sends_it_once() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let mut ui = connected(&mgr, t0);
        assert_eq!(ui.sync(1, false), None, "빈 관심 = 보낼 것 없음");

        let view = main_active(&mgr);
        put(&mut mgr, view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
        assert_eq!(ui.sent, set(&[Claude]));
        assert_eq!(ui.sync(1, false), None, "sent 기준이라 둘째부터 없음(멱등)");
    }

    #[test]
    fn shrink_waits_for_its_deadline_and_then_sends_once() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(true, false))
            .unwrap();
        let action = ui.recompute(&mgr, t0);
        let InterestAction::Defer { gen, deadline } = action else {
            panic!("줄임 지연이어야 한다: {action:?}");
        };
        assert_eq!(deadline, t0 + USAGE_INTEREST_SHRINK_DELAY);
        assert_eq!(USAGE_INTEREST_SHRINK_DELAY, Duration::from_millis(1500));
        assert_eq!(ui.sync(1, false), None, "기한 전 = 송신 0");
        assert_eq!(ui.sent, set(&[Claude, Codex]));

        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
        assert!(!ui.deferral_elapsed(gen), "한 세대는 한 번만 끝난다");
        assert_eq!(ui.sync(1, false), None);
    }

    /// 탭을 다른 창으로 떼는 동안처럼 기한 안에 `sent` 로 돌아오면 데몬에 아무것도 안 간다.
    #[test]
    fn returning_to_sent_within_the_delay_cancels_the_shrink() {
        let t0 = Instant::now();
        let (mut mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);

        let gen = defer_gen(ui.set_visible(MAIN_WINDOW_LABEL, false, &mgr, t0));
        let later = t0 + Duration::from_millis(700);
        assert_eq!(
            ui.set_visible(MAIN_WINDOW_LABEL, true, &mgr, later),
            InterestAction::Unchanged
        );
        assert_eq!(ui.shrink_pending, None);
        assert!(!ui.deferral_elapsed(gen), "무효가 된 대기의 뒤늦은 기한");
        assert_eq!(ui.sync(1, false), None);

        // 모델 쪽 떼기도 같다 — 옛 창에서 빠짐(줄임) → 새 창에 듦(원상).
        let view = main_active(&mgr);
        let slot = root_slot(&mgr, view);
        mgr.set_slot_content(view, slot, SlotContent::Empty)
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, later));
        let popup_view = mgr.create_window(POPUP).unwrap();
        put(&mut mgr, popup_view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, later), InterestAction::Unchanged);
        assert!(!ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), None);
    }

    #[test]
    fn a_newer_shrink_supersedes_the_older_timer() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(true, false))
            .unwrap();
        let first = defer_gen(ui.recompute(&mgr, t0));
        mgr.set_slot_content(view, slot, usage(false, false))
            .unwrap();
        let second = defer_gen(ui.recompute(&mgr, t0 + Duration::from_secs(1)));
        assert_ne!(first, second);
        assert!(!ui.deferral_elapsed(first));
        assert!(ui.deferral_elapsed(second));
        assert_eq!(ui.sync(1, false), Some(set(&[])));
    }

    #[test]
    fn growth_during_a_pending_shrink_cancels_it_and_nudges() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        mgr.set_slot_content(view, slot, usage(false, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        mgr.set_slot_content(view, slot, usage(false, true))
            .unwrap();
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        assert!(!ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Codex])));
    }

    #[test]
    fn hiding_defers_but_keeps_the_window_in_labels_and_showing_nudges() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);

        let gen = defer_gen(ui.set_visible(MAIN_WINDOW_LABEL, false, &mgr, t0));
        assert_eq!(ui.interest, set(&[]));
        let out = ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        assert_eq!(
            out.labels,
            vec![MAIN_WINDOW_LABEL.to_string()],
            "숨은 창도 방송은 받는다"
        );
        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[])));

        assert_eq!(
            ui.set_visible(MAIN_WINDOW_LABEL, true, &mgr, t0),
            InterestAction::Nudge
        );
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
    }

    /// 한 직렬 경로 ① — 두 재계산의 넛지가 엇갈려 처리돼도 소켓의 마지막 한 장 = 마지막 관심.
    #[test]
    fn crossed_nudges_still_leave_the_latest_interest_last() {
        let t0 = Instant::now();
        let mut mgr = ViewManager::new();
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);

        let slot = put(&mut mgr, view, usage(true, false));
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge); // 넛지 A
        mgr.set_slot_content(view, slot, usage(true, true)).unwrap();
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge); // 넛지 B

        let mut wire = Vec::new();
        // 연결 태스크가 넛지 둘을 어느 차례로 꺼내든 읽는 것은 그때의 관심이다.
        wire.extend(ui.sync(1, false));
        wire.extend(ui.sync(1, false));
        assert_eq!(wire, vec![set(&[Claude, Codex])]);
    }

    // ── 대조 (데몬이 실어 온 subscribed) ───────────────────────────────────────────

    #[test]
    fn a_mismatch_without_a_pending_shrink_resends_the_interest() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        let view = main_active(&mgr);
        mgr.set_slot_content(view, slot, usage(true, true)).unwrap();
        assert_eq!(ui.recompute(&mgr, t0), InterestAction::Nudge);
        // 넛지가 사라졌다(명령 채널 포화) — 데몬은 여전히 claude 만 구독한다.
        let out = ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        assert_eq!(out.resend, Some(set(&[Claude, Codex])));
        assert_eq!(ui.sent, set(&[Claude, Codex]));
        assert_eq!(ui.sync(1, false), None, "대조가 이미 보냈다");
    }

    #[test]
    fn a_match_resends_nothing() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        let out = ui.on_snapshot(1, &snap(Codex, 1), &[Codex, Claude], t0);
        assert_eq!(
            out.resend, None,
            "새로 든 회사의 첫 스냅숏(subscribed = 관심)"
        );
        assert_eq!(out.labels, vec![MAIN_WINDOW_LABEL.to_string()]);
    }

    /// 한 직렬 경로 ② — 줄임 지연 중 빠질 회사의 방송은 데몬이 아직 옛 `sent` 를 구독한 정당한 것이다.
    #[test]
    fn during_a_pending_shrink_subscribed_equal_to_sent_is_left_alone() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));

        let out = ui.on_snapshot(1, &snap(Codex, 2), &[Claude, Codex], t0);
        assert_eq!(out.resend, None);
        assert_eq!(ui.shrink_pending, Some(gen));
        assert_eq!(ui.sent, set(&[Claude, Codex]));
    }

    /// 1.5초 창 안에 닿은 낡은 한 장은 `sent` 를 되보낼 뿐 줄임을 앞당기지 않는다.
    #[test]
    fn during_a_pending_shrink_a_mismatch_resends_sent_and_keeps_waiting() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));

        let out = ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        assert_eq!(out.resend, Some(set(&[Claude, Codex])));
        assert_eq!(ui.shrink_pending, Some(gen), "대기는 그대로");
        assert_eq!(ui.sent, set(&[Claude, Codex]));
        assert!(ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(1, false), Some(set(&[Claude])));
    }

    /// 줄임 넛지가 기한 뒤에 사라졌다 — 빠질 회사의 다음 스냅숏이 고친다.
    #[test]
    fn a_lost_shrink_nudge_is_healed_by_the_next_snapshot() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        assert!(ui.deferral_elapsed(gen)); // 넛지 한 장 → 채널 포화로 버려짐(sync 없음)

        let out = ui.on_snapshot(1, &snap(Codex, 3), &[Claude, Codex], t0);
        assert_eq!(out.resend, Some(set(&[Claude])));
        assert_eq!(ui.sent, set(&[Claude]));
    }

    /// 끈질긴 어긋남 — 같은 `subscribed` 가 세 번 오면 세 번 되보내고, 한 장마다 debug 한 줄.
    #[test]
    fn a_persistent_mismatch_resends_every_time_and_logs_each() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);

        let lines = Arc::new(Mutex::new(Vec::new()));
        let resends = tracing::subscriber::with_default(DebugLines(lines.clone()), || {
            (1..=3)
                .map(|rev| ui.on_snapshot(1, &snap(Claude, rev), &[Claude], t0).resend)
                .collect::<Vec<_>>()
        });
        assert_eq!(resends, vec![Some(set(&[Claude, Codex])); 3]);
        let mismatch_lines = lines
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.contains("사용량 구독 어긋남"))
            .count();
        assert_eq!(mismatch_lines, 3);
    }

    #[test]
    fn a_snapshot_of_an_older_socket_is_not_reconciled_or_cached() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        assert!(ui.on_socket_open(2).is_some());

        let out = ui.on_snapshot(1, &snap(Claude, 9), &[], t0);
        assert_eq!(out, SnapshotOutcome::default());
        assert!(ui.cache.is_empty());
    }

    // ── ⟳ 강제 ────────────────────────────────────────────────────────────────

    #[test]
    fn force_sends_even_when_sent_matches_and_cancels_a_pending_shrink() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        assert_eq!(ui.sync(1, true), Some(set(&[Claude, Codex])));

        mgr.set_slot_content(main_active(&mgr), slot, usage(false, true))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));
        assert_eq!(ui.sync(1, true), Some(set(&[Codex])));
        assert_eq!(ui.sent, set(&[Codex]));
        assert!(!ui.deferral_elapsed(gen));
        assert_eq!(ui.sync(2, true), None, "지금 소켓이 아니면 강제도 없다");
    }

    // ── 캐시 · 핸드오프 보정 ─────────────────────────────────────────────────────

    #[test]
    fn the_cache_keeps_the_highest_revision_and_equal_replaces() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);

        let mut newer = snap(Claude, 5);
        newer.plan = Some("max".to_string());
        ui.on_snapshot(1, &newer, &[Claude], t0);
        let out = ui.on_snapshot(1, &snap(Claude, 4), &[Claude], t0);
        assert_eq!(
            out.labels,
            vec![MAIN_WINDOW_LABEL.to_string()],
            "전달은 revision 무관"
        );
        assert_eq!(ui.snapshot_for_webview(t0).snapshots, vec![newer]);

        let t1 = t0 + Duration::from_secs(40);
        let mut same = snap(Claude, 5);
        same.plan = Some("pro".to_string());
        ui.on_snapshot(1, &same, &[Claude], t1);
        assert_eq!(
            ui.snapshot_for_webview(t1).snapshots,
            vec![same],
            "같은 revision = 새로 받은 것(받은 시각도 새로)"
        );
    }

    #[test]
    fn the_handoff_moves_relative_seconds_by_the_time_since_receipt() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, true);
        let mut ui = connected(&mgr, t0);

        let mut claude = snap(Claude, 1);
        claude.state = UsageVendorState::Failed {
            next_attempt_in_secs: 60,
            detail: None,
        };
        let mut codex = snap(Codex, 1);
        codex.state = UsageVendorState::Rejected {
            retry_in_secs: 600,
            detail: None,
        };
        ui.on_snapshot(1, &claude, &[Claude, Codex], t0);
        ui.on_snapshot(1, &codex, &[Claude, Codex], t0);

        let pulled = ui.snapshot_for_webview(t0 + Duration::from_millis(90_900));
        assert_eq!(pulled.socket_epoch, 1);
        let [c, x] = pulled.snapshots.as_slice() else {
            panic!("회사 둘: {pulled:?}");
        };
        assert_eq!(c.vendor, Claude);
        assert_eq!(c.five_hour.as_ref().unwrap().age_secs, 100);
        assert_eq!(c.weekly.as_ref().unwrap().age_secs, 110);
        assert_eq!(c.model_scoped[0].window.age_secs, 120);
        assert_eq!(
            c.state,
            UsageVendorState::Failed {
                next_attempt_in_secs: 0,
                detail: None
            },
            "60 - 90 은 0 으로 포화"
        );
        assert_eq!(x.vendor, Codex);
        assert_eq!(
            x.state,
            UsageVendorState::Rejected {
                retry_in_secs: 510,
                detail: None
            }
        );
        assert_eq!(
            ui.snapshot_for_webview(t0).snapshots[0]
                .five_hour
                .as_ref()
                .unwrap()
                .age_secs,
            10,
            "캐시 원본은 보정에 안 닳는다"
        );
    }

    #[test]
    fn without_a_socket_the_pull_is_empty_with_epoch_zero() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = UsageInterest::new();
        assert_eq!(
            ui.snapshot_for_webview(t0),
            WebviewUsageSnapshot {
                socket_epoch: 0,
                snapshots: vec![]
            }
        );

        ui.recompute(&mgr, t0);
        ui.on_socket_open(1);
        ui.on_snapshot(1, &snap(Claude, 1), &[Claude], t0);
        ui.on_socket_lost(1);
        assert_eq!(
            ui.snapshot_for_webview(t0),
            WebviewUsageSnapshot {
                socket_epoch: 0,
                snapshots: vec![]
            }
        );
    }

    // ── 소켓 표식 ─────────────────────────────────────────────────────────────

    #[test]
    fn a_new_socket_clears_the_cache_and_sends_the_interest() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        ui.on_snapshot(1, &snap(Claude, 7), &[Claude], t0);

        assert_eq!(ui.on_socket_open(3), Some(set(&[Claude])));
        assert_eq!(ui.socket_epoch, 3);
        assert_eq!(ui.sent, set(&[Claude]));
        assert!(ui.cache.is_empty());

        let empty = ViewManager::new();
        let mut idle = UsageInterest::new();
        idle.recompute(&empty, t0);
        assert_eq!(idle.on_socket_open(1), None, "빈 관심은 보낼 것이 없다");
        assert_eq!(idle.socket_epoch, 1);
    }

    #[test]
    fn an_open_not_above_the_highest_seen_epoch_changes_nothing() {
        let t0 = Instant::now();
        let (mgr, _) = main_with(true, false);
        let mut ui = UsageInterest::new();
        ui.recompute(&mgr, t0);
        assert!(ui.on_socket_open(5).is_some());
        ui.on_snapshot(5, &snap(Claude, 1), &[Claude], t0);
        ui.on_socket_lost(5);
        assert!(ui.on_socket_open(6).is_some());
        ui.on_snapshot(6, &snap(Claude, 2), &[Claude], t0);
        let sent_before = ui.sent.clone();

        for stale in [5, 6, 0] {
            assert_eq!(ui.on_socket_open(stale), None, "표식 {stale}");
        }
        assert_eq!(ui.socket_epoch, 6);
        assert_eq!(ui.sent, sent_before);
        assert_eq!(ui.cache.len(), 1);
    }

    #[test]
    fn calls_carrying_an_older_socket_epoch_do_nothing() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, false);
        let mut ui = connected(&mgr, t0);
        assert!(ui.on_socket_open(2).is_some());
        ui.on_snapshot(2, &snap(Claude, 1), &[Claude], t0);

        mgr.set_slot_content(main_active(&mgr), slot, usage(true, true))
            .unwrap();
        ui.recompute(&mgr, t0);
        assert_eq!(ui.sync(1, false), None);
        assert_eq!(ui.sync(1, true), None);
        ui.on_socket_lost(1);
        assert_eq!(ui.socket_epoch, 2);
        assert_eq!(ui.cache.len(), 1);
        assert_eq!(ui.sent, set(&[Claude]));
        assert_eq!(ui.sync(2, false), Some(set(&[Claude, Codex])));
    }

    #[test]
    fn losing_the_socket_clears_the_cache_and_sent() {
        let t0 = Instant::now();
        let (mut mgr, slot) = main_with(true, true);
        let mut ui = connected(&mgr, t0);
        ui.on_snapshot(1, &snap(Codex, 1), &[Claude, Codex], t0);
        mgr.set_slot_content(main_active(&mgr), slot, usage(true, false))
            .unwrap();
        let gen = defer_gen(ui.recompute(&mgr, t0));

        ui.on_socket_lost(1);
        assert_eq!(ui.socket_epoch, 0);
        assert!(ui.cache.is_empty());
        assert!(ui.sent.is_empty());
        assert!(!ui.deferral_elapsed(gen), "줄일 sent 가 사라졌다");
        assert_eq!(
            ui.recompute(&mgr, t0),
            InterestAction::Nudge,
            "비연결 중에는 관심이 곧 늘어남이다(넛지는 창구가 닫혀 no-op)"
        );
        assert_eq!(ui.sync(0, false), None, "소켓 없음(0)과는 맞지 않는다");
        assert_eq!(ui.on_socket_open(2), Some(set(&[Claude])));
    }

    // ── 로그 포획 ─────────────────────────────────────────────────────────────

    /// 이 스레드의 debug 줄 메시지.
    struct DebugLines(Arc<Mutex<Vec<String>>>);

    struct Message<'a>(&'a mut String);

    impl tracing::field::Visit for Message<'_> {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                *self.0 = format!("{value:?}");
            }
        }
    }

    impl tracing::Subscriber for DebugLines {
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
            if *event.metadata().level() == tracing::Level::DEBUG {
                let mut message = String::new();
                event.record(&mut Message(&mut message));
                self.0.lock().unwrap().push(message);
            }
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
}
