//! # 대기 입력 명부 — 화신 하나의 「아직 전달되지 않은 입력」 목록
//!
//! 링에 선 [`QueuedInputEvent`] 를 링 순서대로 환원해 목록(항목)과 묘비를 쥔다. 코어가 replay 락 안에서
//! 먹이므로 **명부 상태 = 링 접두의 환원값**이다 — 명부를 바꾸는 다른 길을 만들지 않는다(세션의 취소·분류도
//! 사건을 emit 할 뿐이다).
//!
//! - ★순수 · 잎★: 환원기는 밖을 부르지 않는다 — 락 순서 `input_order → replay → queued_inputs` 의 끝이다.
//! - ★규칙은 이력이 아니라 지금 상태로 선다★ — 「앞에 무엇이 왔었나」를 되짚지 않으므로 같은 사건열이 늘
//!   같은 상태로 떨어진다. 취소 대기의 두 칸(`answer` · `vendor_closed`)도 상태다.
//! - ★프론트 누산기와 한 벌이다★: 같은 규칙을 TS 로 다시 짜고, 둘 다 골든 `queued_input_golden.json`(이
//!   파일 옆)을 먹는다. 규칙을 고치면 골든과 TS 판을 같은 변경에서 고친다 — 갈라지면 한쪽이 빨개진다.
//! - ★묘비는 생산자 결함에 대한 둘째 방어다★: 정상 링에서는 같은 id 의 종결이 `Queued` 보다 먼저 서지
//!   않는다. 그래서 명부(화신 전체)와 누산기(링 창)의 묘비 집합이 달라도 환원 결과가 갈리지 않는다.
// ADR-0231

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::types::{DeliveredCopy, DropCause, QueuedInputEvent};

/// 화신당 묘비 상한 — 넘치면 가장 먼저 든 묘비부터 버린다(FIFO).
/// ★골든 파일 머리(`tombstone_cap`)에 같은 값이 있다★ — TS 판이 자기 상수를 그것과 대조한다.
// ADR-0231
pub const TOMBSTONE_CAP: usize = 1024;

/// 취소 대기 항목의 「응답」 칸 — 우리 취소 요청이 어떻게 됐나.
/// 「뺐다」(`removed:true`)는 칸에 남지 않는다 — 그 응답이 곧바로 취소됨으로 닫는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelAnswer {
    /// 아직 응답이 없다.
    Unanswered,
    /// 성공 응답 `removed:false` 를 받았거나, 요청 자체가 실패했다(`CancelFailed`).
    NotRemoved,
}

/// 목록 항목의 상태 — 둘 다 비종결이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPhase {
    Queued,
    /// 취소 대기. `vendor_closed` = 그 id 의 벤더 닫힘(`Dropped{Unknown}`)을 이미 봤다 — 그것이 우리
    /// 취소였는지는 `answer` 가 가른다.
    Cancelling {
        answer: CancelAnswer,
        vendor_closed: bool,
    },
}

/// 목록 한 줄 — 명부의 환원 상태를 빠짐없이 싣는다. 그래야 스냅숏 위에 뒤 사건을 다시 환원한 결과가
/// 명부와 같다(취소 대기의 두 칸이 빠지면 늦게 온 응답·판명의 결말이 갈린다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedRow {
    pub id: String,
    pub text: String,
    pub phase: RowPhase,
}

/// 한 id 가 종결에 닿은 결말.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 벤더가 받았다(너무 늦은 취소 포함).
    Delivered,
    /// 취소됐다 — 벤더가 그 글을 쓰지 않았다.
    Cancelled,
    /// 전달되지 않았다. `Withdrawn` 은 여기 오지 않는다(그 원인은 늘 `Cancelled` 다).
    Discarded(DropCause),
}

impl Verdict {
    /// ★되살림 정의 — 한 벌★: 종결 원인이 `Unknown` 이면 되살림 가능 묘비다. 어느 사건이 그 모름을
    /// 만들었는지(벤더 닫힘 · 취소 응답 · 수락 모름)와 무관하다 — 「벤더가 준 사실이 모름을 이긴다」.
    fn resurrectable(self) -> bool {
        self == Verdict::Discarded(DropCause::Unknown)
    }

    fn of_drop(cause: DropCause) -> Self {
        match cause {
            DropCause::Withdrawn => Verdict::Cancelled,
            other => Verdict::Discarded(other),
        }
    }
}

/// 명부 본체 — 항목(비종결 · 든 순서) + 묘비 + 화신 칸 둘.
///
/// id 하나의 자리는 셋 중 하나다: **항목** · **묘비**(종결 — 본문 없이 id 와 「되살림 가능」 표시 하나)
/// · **모름**(둘 다 없음).
#[derive(Debug, Default)]
pub struct Registry {
    items: Vec<QueuedRow>,
    tombstones: Tombstones,
    /// 「받음 불가 판명」 표식 — `AckUnavailable` 을 환원했다. ★환원 규칙은 이 칸을 읽지 않는다★ — 판명
    /// 뒤에 늦게 선 `Queued` 를 코어가 바꿔 적는 재료일 뿐이다.
    ack_unavailable_seen: bool,
    /// 마지막으로 환원한 목록 사건의 seq(버린 사건 포함 — 그것도 링 접두에 들었다).
    as_of_seq: Option<u64>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 링의 `seq` 자리에 선 사건 하나를 환원한다. 반환 = 이 사건으로 결말이 선 id 와 그 결말(환원 순서).
    ///
    /// ★항목이 닫힌 것만 돌려주지 않는다★ — 모르는 id 에 묘비만 남긴 종결, 되살림(`Delivered`), 받음 뒤
    /// 거절의 고쳐 읽기(`Discarded(Rejected)`)도 결말로 돌려준다. 항목이 있었는지는 호출자가 환원 전
    /// 상태로 가른다.
    /// ★호출 전제 = `seq` 는 호출마다 커진다(링 순서)★.
    pub(crate) fn reduce(&mut self, seq: u64, event: &QueuedInputEvent) -> Vec<(String, Verdict)> {
        debug_assert!(
            self.as_of_seq.is_none_or(|last| seq > last),
            "링 순서가 아닌 환원: {seq} <= {:?}",
            self.as_of_seq
        );
        self.as_of_seq = Some(seq);
        let mut closed = Vec::new();
        match event {
            QueuedInputEvent::Queued { id, text } => self.on_queued(id, text),
            QueuedInputEvent::CancelRequested { id } => self.on_cancel_requested(id),
            QueuedInputEvent::CancelAnswered { id, removed: true } => {
                self.on_removed(id, &mut closed)
            }
            QueuedInputEvent::CancelAnswered { id, removed: false }
            | QueuedInputEvent::CancelFailed { id } => self.on_not_removed(id, &mut closed),
            QueuedInputEvent::Delivered { id } => self.on_delivered(id, &mut closed),
            QueuedInputEvent::Dropped { id, cause } => self.on_dropped(id, *cause, &mut closed),
            QueuedInputEvent::AckUnavailable { delivered } => {
                self.on_ack_unavailable(delivered, &mut closed)
            }
        }
        closed
    }

    /// 열린 항목(든 순서 — 가장 오래된 것이 앞).
    pub fn rows(&self) -> &[QueuedRow] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn ack_unavailable_seen(&self) -> bool {
        self.ack_unavailable_seen
    }

    /// `None` = 이 화신의 목록 사건을 아직 하나도 환원하지 않았다.
    pub fn as_of_seq(&self) -> Option<u64> {
        self.as_of_seq
    }

    /// `AckUnavailable` 이 받음으로 닫는 항목(= 열린 항목 전부)의 본문 사본 — 그 사건의 `delivered` 를
    /// 링에 적기 전에 채울 때 쓴다(디코더는 빈 채로 낸다).
    pub fn open_copies(&self) -> Vec<DeliveredCopy> {
        self.items
            .iter()
            .map(|row| DeliveredCopy {
                id: row.id.clone(),
                text: row.text.clone(),
            })
            .collect()
    }

    /// 그 id 의 묘비. `Some(true)` = 되살림 가능 · `None` = 묘비가 없다(항목이거나 모름).
    pub fn tombstone(&self, id: &str) -> Option<bool> {
        self.tombstones.get(id)
    }

    pub fn tombstone_count(&self) -> usize {
        self.tombstones.order.len()
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|row| row.id == id)
    }

    /// 항목을 지우고 묘비로 접는다 — 본문은 화신 수명 내내 쥐지 않는다.
    fn close(&mut self, at: usize, verdict: Verdict, closed: &mut Vec<(String, Verdict)>) {
        let row = self.items.remove(at);
        self.tombstones.insert(&row.id, verdict.resurrectable());
        closed.push((row.id, verdict));
    }

    fn on_queued(&mut self, id: &str, text: &str) {
        // 이미 항목이면 무동작(재방출 없음 — 둘째 방어) · 묘비면 버린다(늦은 `Queued` 가 영구 항목을 만들지
        // 않게).
        if self.position(id).is_some() || self.tombstones.get(id).is_some() {
            return;
        }
        self.items.push(QueuedRow {
            id: id.to_owned(),
            text: text.to_owned(),
            phase: RowPhase::Queued,
        });
    }

    fn on_cancel_requested(&mut self, id: &str) {
        if let Some(at) = self.position(id) {
            let row = &mut self.items[at];
            if row.phase == RowPhase::Queued {
                row.phase = RowPhase::Cancelling {
                    answer: CancelAnswer::Unanswered,
                    vendor_closed: false,
                };
            }
        }
    }

    /// `removed:true` = 벤더가 그 글을 자기 큐에서 뺐다 — 다시 배출될 수 없으니 수명주기 줄을 기다리지 않고
    /// 닫는다. 기다리면 그 줄이 끝내 안 오는 날, 모든 창에서 빠진 항목이 목록을 쥔 채 남아 우편을 막는다.
    fn on_removed(&mut self, id: &str, closed: &mut Vec<(String, Verdict)>) {
        if let Some(at) = self.position(id) {
            if matches!(self.items[at].phase, RowPhase::Cancelling { .. }) {
                self.close(at, Verdict::Cancelled, closed);
            }
        }
    }

    /// `removed:false` · 요청 실패. ★목록으로 되돌리지 않는다★ — 되돌리면 ✕ 로 모든 창에서 빠진 항목이
    /// 다시 그려진다.
    fn on_not_removed(&mut self, id: &str, closed: &mut Vec<(String, Verdict)>) {
        let Some(at) = self.position(id) else {
            return;
        };
        match self.items[at].phase {
            RowPhase::Queued => {}
            RowPhase::Cancelling {
                vendor_closed: false,
                ..
            } => {
                self.items[at].phase = RowPhase::Cancelling {
                    answer: CancelAnswer::NotRemoved,
                    vendor_closed: false,
                };
            }
            // 벤더가 이미 닫았는데 우리 취소가 아니었다.
            RowPhase::Cancelling {
                vendor_closed: true,
                ..
            } => self.close(at, Verdict::Discarded(DropCause::Unknown), closed),
        }
    }

    fn on_delivered(&mut self, id: &str, closed: &mut Vec<(String, Verdict)>) {
        if let Some(at) = self.position(id) {
            self.close(at, Verdict::Delivered, closed);
            return;
        }
        match self.tombstones.get(id) {
            None => {
                self.tombstones.insert(id, false);
                closed.push((id.to_owned(), Verdict::Delivered));
            }
            // 「모름」으로 버린 항목에 벤더 받음이 뒤늦게 왔다. 말풍선은 뒤따르는 벤더 에코가 그린다.
            Some(true) => {
                self.tombstones.clear_resurrectable(id);
                closed.push((id.to_owned(), Verdict::Delivered));
            }
            // 같은 id 의 둘째 `Delivered`(codex 는 `item/started`·`item/completed` 양쪽에서 낸다)도 여기서 끝난다.
            Some(false) => {}
        }
    }

    fn on_dropped(&mut self, id: &str, cause: DropCause, closed: &mut Vec<(String, Verdict)>) {
        if let Some(at) = self.position(id) {
            match (self.items[at].phase, cause) {
                (_, DropCause::Withdrawn) => self.close(at, Verdict::Cancelled, closed),
                (RowPhase::Queued, cause) => self.close(at, Verdict::Discarded(cause), closed),
                (
                    RowPhase::Cancelling { .. },
                    cause @ (DropCause::Interrupted | DropCause::AgentEnded | DropCause::Rejected),
                ) => self.close(at, Verdict::Discarded(cause), closed),
                // 실패·끊긴 턴이 닫았다 — 우리 취소는 이미 「못 뺐다」로 답했다.
                (
                    RowPhase::Cancelling {
                        answer: CancelAnswer::NotRemoved,
                        vendor_closed: false,
                    },
                    DropCause::Unknown,
                ) => self.close(at, Verdict::Discarded(DropCause::Unknown), closed),
                // 우리 취소가 뺐다면 그 응답이 같은 ms 에 뒤따른다 — 목록엔 취소 대기 그대로 두고 기다린다.
                (
                    RowPhase::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false,
                    },
                    DropCause::Unknown,
                ) => {
                    self.items[at].phase = RowPhase::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: true,
                    };
                }
                // 벤더 닫힘을 이미 봤다 — 둘째 닫힘은 새 사실이 없다.
                (
                    RowPhase::Cancelling {
                        vendor_closed: true,
                        ..
                    },
                    DropCause::Unknown,
                ) => {}
            }
            return;
        }
        match self.tombstones.get(id) {
            None => {
                let verdict = Verdict::of_drop(cause);
                self.tombstones.insert(id, verdict.resurrectable());
                closed.push((id.to_owned(), verdict));
            }
            // 받음 뒤 거절 — 벤더 `refused` 는 `started` 뒤의 종결로도 온다. 결말을 고쳐 읽고 되살림을 막는다.
            Some(_) if cause == DropCause::Rejected => {
                self.tombstones.clear_resurrectable(id);
                closed.push((id.to_owned(), Verdict::Discarded(DropCause::Rejected)));
            }
            Some(_) => {}
        }
    }

    /// 열린 항목 전부를 그 링 자리의 받음으로 닫는다 — 벤더는 이미 받았고 자리만 판명 순간이다. 취소 대기도
    /// 응답을 기다리지 않는다: 옛 CLI 는 취소 줄을 말없이 버릴 수 있어, 기다리면 그 항목이 에이전트가 끝날
    /// 때까지 남아 우편을 막는다. 사본의 id 는 그 뒤 `Delivered` 와 같게 환원한다(항목이던 것은 방금 닫혀
    /// 무동작 · 모르는 id 는 묘비만).
    fn on_ack_unavailable(
        &mut self,
        delivered: &[DeliveredCopy],
        closed: &mut Vec<(String, Verdict)>,
    ) {
        self.ack_unavailable_seen = true;
        while !self.items.is_empty() {
            self.close(0, Verdict::Delivered, closed);
        }
        for copy in delivered {
            self.on_delivered(&copy.id, closed);
        }
    }
}

/// 묘비 집합 — 든 순서를 쥐는 FIFO. ★집합이다★: 이미 있는 id 를 다시 넣으면 무동작이고 자리도 안 옮긴다.
#[derive(Debug, Default)]
struct Tombstones {
    order: VecDeque<String>,
    resurrectable: HashMap<String, bool>,
}

impl Tombstones {
    fn get(&self, id: &str) -> Option<bool> {
        self.resurrectable.get(id).copied()
    }

    fn insert(&mut self, id: &str, resurrectable: bool) {
        if self.resurrectable.contains_key(id) {
            return;
        }
        if self.order.len() >= TOMBSTONE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.resurrectable.remove(&oldest);
            }
        }
        self.order.push_back(id.to_owned());
        self.resurrectable.insert(id.to_owned(), resurrectable);
    }

    fn clear_resurrectable(&mut self, id: &str) {
        if let Some(flag) = self.resurrectable.get_mut(id) {
            *flag = false;
        }
    }
}

/// 화신 하나의 명부 — 코어가 이 Arc 를 소유하고, 세션은 코어를 거쳐 닿는다(`AgentSession::queued_inputs()`).
/// 읽는 쪽(목록 조회 · 취소 검증)은 이 락 하나만 잡는다.
// ADR-0231
#[derive(Debug, Default)]
pub struct QueuedInputs {
    registry: Mutex<Registry>,
}

impl QueuedInputs {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★이 가드를 쥔 채 emit·IO·다른 락을 잡지 말 것★ — 락 순서의 잎이다.
    /// 독은 풀어 쓴다: 환원기는 밖을 부르지 않아 독이 들 길은 환원기 자신의 패닉뿐이고, 그 대가로 목록을
    /// 읽는 호출자까지 죽일 이유가 없다.
    /// ★crate 밖에 열지 않는다(`reduce` 도 같다)★ — 가드로 환원기에 닿으면 replay 락 밖에서 명부를 바꿀 수
    /// 있어 「명부 = 링 접두의 환원값」이 깨진다. 밖에서는 [`Self::snapshot`] 으로 읽는다.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 목록 행과 그 행이 환원한 마지막 목록 사건의 seq 를 **한 락 아래** 함께 읽는다 — 행 = 링 접두
    /// `as_of_seq` 의 환원값이다. seq 는 한 화신 안에서만 견준다.
    pub fn snapshot(&self) -> (Vec<QueuedRow>, Option<u64>) {
        let registry = self.lock();
        (registry.items.clone(), registry.as_of_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const GOLDEN: &str = include_str!("queued_input_golden.json");

    fn text(v: &Value, key: &str) -> String {
        v[key]
            .as_str()
            .unwrap_or_else(|| panic!("골든 사건에 `{key}` 가 없다: {v}"))
            .to_owned()
    }

    fn cause_of(name: &str) -> DropCause {
        match name {
            "Withdrawn" => DropCause::Withdrawn,
            "Interrupted" => DropCause::Interrupted,
            "AgentEnded" => DropCause::AgentEnded,
            "Rejected" => DropCause::Rejected,
            "Unknown" => DropCause::Unknown,
            other => panic!("모르는 원인: {other}"),
        }
    }

    fn cause_name(cause: DropCause) -> &'static str {
        match cause {
            DropCause::Withdrawn => "Withdrawn",
            DropCause::Interrupted => "Interrupted",
            DropCause::AgentEnded => "AgentEnded",
            DropCause::Rejected => "Rejected",
            DropCause::Unknown => "Unknown",
        }
    }

    /// 골든의 사건은 wire `op` 모양 그대로다(`kind` 판별자) — 도메인 타입은 serde 를 달지 않으므로 여기서
    /// 손으로 옮긴다.
    fn event_of(v: &Value) -> QueuedInputEvent {
        match v["kind"].as_str().expect("kind") {
            "Queued" => QueuedInputEvent::Queued {
                id: text(v, "id"),
                text: text(v, "text"),
            },
            "CancelRequested" => QueuedInputEvent::CancelRequested { id: text(v, "id") },
            "CancelAnswered" => QueuedInputEvent::CancelAnswered {
                id: text(v, "id"),
                removed: v["removed"].as_bool().expect("removed"),
            },
            "CancelFailed" => QueuedInputEvent::CancelFailed { id: text(v, "id") },
            "Delivered" => QueuedInputEvent::Delivered { id: text(v, "id") },
            "Dropped" => QueuedInputEvent::Dropped {
                id: text(v, "id"),
                cause: cause_of(&text(v, "cause")),
            },
            "AckUnavailable" => QueuedInputEvent::AckUnavailable {
                delivered: v["delivered"]
                    .as_array()
                    .expect("delivered")
                    .iter()
                    .map(|c| DeliveredCopy {
                        id: text(c, "id"),
                        text: text(c, "text"),
                    })
                    .collect(),
            },
            other => panic!("모르는 사건: {other}"),
        }
    }

    fn verdict_name(verdict: Verdict) -> String {
        match verdict {
            Verdict::Delivered => "Delivered".into(),
            Verdict::Cancelled => "Cancelled".into(),
            Verdict::Discarded(cause) => format!("Discarded:{}", cause_name(cause)),
        }
    }

    /// 행의 JSON 모양 = 목록 조회 응답의 행과 같은 낱말(`state` · `cancel`).
    fn row_json(row: &QueuedRow) -> Value {
        let (state, cancel) = match row.phase {
            RowPhase::Queued => ("queued", Value::Null),
            RowPhase::Cancelling {
                answer,
                vendor_closed,
            } => (
                "cancelling",
                serde_json::json!({
                    "answer": match answer {
                        CancelAnswer::Unanswered => "none",
                        CancelAnswer::NotRemoved => "not_removed",
                    },
                    "vendor_closed": vendor_closed,
                }),
            ),
        };
        serde_json::json!({ "id": row.id, "text": row.text, "state": state, "cancel": cancel })
    }

    fn check_case(case: &Value) {
        let name = case["name"].as_str().expect("name");
        let expect = &case["expect"];
        let mut registry = Registry::new();
        let mut outcomes: HashMap<String, Vec<String>> = HashMap::new();
        for (i, ev) in case["events"]
            .as_array()
            .expect("events")
            .iter()
            .enumerate()
        {
            for (id, verdict) in registry.reduce(i as u64 + 1, &event_of(ev)) {
                outcomes.entry(id).or_default().push(verdict_name(verdict));
            }
        }

        let rows: Vec<Value> = registry.rows().iter().map(row_json).collect();
        assert_eq!(&Value::Array(rows), &expect["items"], "[{name}] 목록");
        assert_eq!(
            registry.tombstone_count() as u64,
            expect["tombstone_count"].as_u64().expect("tombstone_count"),
            "[{name}] 묘비 수"
        );
        for (id, resurrectable) in expect["tombstones"].as_object().expect("tombstones") {
            assert_eq!(
                registry.tombstone(id),
                Some(resurrectable.as_bool().expect("bool")),
                "[{name}] 묘비 `{id}`"
            );
        }
        for id in expect["absent"].as_array().expect("absent") {
            let id = id.as_str().expect("id");
            assert!(
                registry.tombstone(id).is_none() && registry.rows().iter().all(|r| r.id != id),
                "[{name}] `{id}` 는 항목도 묘비도 아니어야 한다"
            );
        }
        assert_eq!(
            registry.ack_unavailable_seen(),
            expect["ack_unavailable"]
                .as_bool()
                .expect("ack_unavailable"),
            "[{name}] 받음 불가 표식"
        );
        for (id, want) in expect["outcomes"].as_object().expect("outcomes") {
            let want: Vec<String> = want
                .as_array()
                .expect("결말 목록")
                .iter()
                .map(|v| v.as_str().expect("결말").to_owned())
                .collect();
            let got = outcomes.get(id).cloned().unwrap_or_default();
            assert_eq!(got, want, "[{name}] `{id}` 의 결말 이력");
        }
    }

    // ── 골든(TS 누산기와 공유) ────────────────────────────────────────────────────

    #[test]
    fn golden_sequences_reduce_to_the_expected_state() {
        let golden: Value = serde_json::from_str(GOLDEN).expect("골든 JSON");
        let cases = golden["cases"].as_array().expect("cases");
        assert!(!cases.is_empty());
        for case in cases {
            check_case(case);
        }
    }

    #[test]
    fn the_golden_header_carries_the_tombstone_cap() {
        let golden: Value = serde_json::from_str(GOLDEN).expect("골든 JSON");
        assert_eq!(golden["tombstone_cap"].as_u64(), Some(TOMBSTONE_CAP as u64));
    }

    /// 골든이 어휘를 빠짐없이 한 번 이상 먹인다 — 새 사건·원인이 골든 없이 들어오는 것을 막는다.
    #[test]
    fn the_golden_feeds_every_event_kind_and_cause() {
        let golden: Value = serde_json::from_str(GOLDEN).expect("골든 JSON");
        let mut seen: Vec<String> = Vec::new();
        for case in golden["cases"].as_array().expect("cases") {
            for ev in case["events"].as_array().expect("events") {
                let kind = ev["kind"].as_str().expect("kind");
                let tag = match kind {
                    "Dropped" => format!("Dropped:{}", ev["cause"].as_str().expect("cause")),
                    "CancelAnswered" => format!("CancelAnswered:{}", ev["removed"]),
                    other => other.to_owned(),
                };
                if !seen.contains(&tag) {
                    seen.push(tag);
                }
            }
        }
        for want in [
            "Queued",
            "CancelRequested",
            "CancelAnswered:true",
            "CancelAnswered:false",
            "CancelFailed",
            "Delivered",
            "Dropped:Withdrawn",
            "Dropped:Interrupted",
            "Dropped:AgentEnded",
            "Dropped:Rejected",
            "Dropped:Unknown",
            "AckUnavailable",
        ] {
            assert!(
                seen.iter().any(|s| s == want),
                "골든에 `{want}` 사건이 없다"
            );
        }
    }

    // ── Rust 판만의 표면(스냅숏 · 사본 · seq) ─────────────────────────────────────

    fn queued(id: &str, text: &str) -> QueuedInputEvent {
        QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        }
    }

    #[test]
    fn snapshot_reads_rows_and_the_last_reduced_seq_together() {
        let inputs = QueuedInputs::new();
        assert_eq!(inputs.snapshot(), (vec![], None));
        {
            let mut registry = inputs.lock();
            registry.reduce(4, &queued("a", "하나"));
            registry.reduce(9, &queued("b", "둘"));
            registry.reduce(12, &QueuedInputEvent::CancelRequested { id: "b".into() });
        }
        let (rows, as_of) = inputs.snapshot();
        assert_eq!(as_of, Some(12));
        assert_eq!(
            rows,
            vec![
                QueuedRow {
                    id: "a".into(),
                    text: "하나".into(),
                    phase: RowPhase::Queued,
                },
                QueuedRow {
                    id: "b".into(),
                    text: "둘".into(),
                    phase: RowPhase::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false,
                    },
                },
            ]
        );
    }

    /// 버린 사건도 링 접두에 들었다 — 스냅숏 seq 가 그 자리까지 간다(재부착 대조가 이 seq 뒤의 사건만 다시
    /// 환원하므로, 버린 사건에서 멈추면 그 사건을 두 번 환원한다).
    #[test]
    fn an_ignored_event_still_advances_the_snapshot_seq() {
        let mut registry = Registry::new();
        assert!(registry
            .reduce(3, &QueuedInputEvent::CancelFailed { id: "ghost".into() })
            .is_empty());
        assert_eq!(registry.as_of_seq(), Some(3));
        assert!(registry.is_empty());
        assert_eq!(registry.tombstone_count(), 0);
    }

    #[test]
    fn open_copies_are_the_rows_an_ack_unavailable_closes() {
        let mut registry = Registry::new();
        registry.reduce(1, &queued("a", "하나"));
        registry.reduce(2, &queued("b", "둘"));
        registry.reduce(3, &QueuedInputEvent::CancelRequested { id: "b".into() });
        let copies = registry.open_copies();
        assert_eq!(
            copies,
            vec![
                DeliveredCopy {
                    id: "a".into(),
                    text: "하나".into()
                },
                DeliveredCopy {
                    id: "b".into(),
                    text: "둘".into()
                },
            ]
        );
        let closed = registry.reduce(4, &QueuedInputEvent::AckUnavailable { delivered: copies });
        assert_eq!(
            closed,
            vec![
                ("a".to_owned(), Verdict::Delivered),
                ("b".to_owned(), Verdict::Delivered)
            ],
            "사본이 이미 닫힌 항목을 두 번 닫지 않는다"
        );
        assert!(registry.is_empty() && registry.ack_unavailable_seen());
    }
}
