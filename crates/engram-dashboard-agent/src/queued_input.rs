//! # Pending-input registry — the list of an incarnation's 「아직 전달되지 않은 입력」 (inputs not yet delivered)
//!
//! It reduces the [`QueuedInputEvent`] standing in the ring in ring order and holds the list (items) and tombstones. Since the core
//! feeds it inside the replay lock, **registry state = the reduced value of the ring prefix** — we do not make another way to change the registry (the session's cancel · classification
//! only emit events too).
//!
//! - ★Pure · leaf★: the reducer calls nothing outside — in the lock order `input_order → replay → 명부(이 모듈) → 대기 목록
//!   표(`inputs_pending`)` the registry and the table are both leaves under replay (the registry guard is released before writing the table —
//!   `OutputCore::record_list_event`).
//! - ★The rules stand on the current state, not on history★ — since it does not trace back 「앞에 무엇이 왔었나」 (what came before), the same event sequence always
//!   lands in the same state. The two fields of cancel-pending (`answer` · `vendor_closed`) are state too.
//! - ★It is a set with the frontend accumulator★: the same rules are rewritten in TS, and both feed on the golden `queued_input_golden.json` (next to this
//!   file). When a rule changes, the golden and the TS edition are fixed in the same change — if they diverge, one side goes red.
//! - ★Tombstones are a second defense against producer defects★: in a normal ring, the terminal of the same id does not come
//!   before `Queued`. So even if the tombstone sets of the registry (the whole incarnation) and the accumulator (the ring window) differ, the reduction result does not diverge.
// ADR-0231

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use engram_dashboard_base::sync;

use crate::types::{DeliveredCopy, DropCause, QueuedInputEvent};

/// The per-incarnation tombstone cap — on overflow, the tombstone that entered first is dropped first (FIFO).
/// ★The same value is in the golden file header (`tombstone_cap`)★ — the TS edition checks its own constant against it.
// ADR-0231
pub const TOMBSTONE_CAP: usize = 1024;

/// The 「응답」 (answer) field of a cancel-pending item — what happened to our cancel request.
/// 「뺐다」 (removed) (`removed:true`) does not stay in the field — that response closes it as cancelled right away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelAnswer {
    /// No response yet.
    Unanswered,
    /// A success response `removed:false` was received, or the request itself failed (`CancelFailed`).
    NotRemoved,
}

/// The state of a list item — both are non-terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPhase {
    Queued,
    /// Cancel-pending. `vendor_closed` = the vendor closing (`Dropped{Unknown}`) of that id has already been seen — whether that was our
    /// cancel is told apart by `answer`.
    Cancelling {
        answer: CancelAnswer,
        vendor_closed: bool,
    },
}

/// One list line — it carries the registry's reduction state without omission. That way the result of re-reducing later events on top of a snapshot
/// equals the registry (if the two fields of cancel-pending were missing, the outcome of late-arriving responses · findings would diverge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedRow {
    pub id: String,
    pub text: String,
    pub phase: RowPhase,
    /// The path handed it to the vendor (the value of the last `HandedOver`) — it stands apart from the stage (`phase`): a ✕ that came after handing over is both cancel-pending and
    ///   a handed-over row. It is not an outcome (the row stays until receipt · reaping).
    pub sent: bool,
}

/// The outcome by which one id reached terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The vendor received it (including a cancel that came too late).
    Delivered,
    /// Cancelled — the vendor did not write that text.
    Cancelled,
    /// Not delivered. `Withdrawn` does not come here (its cause is always `Cancelled`).
    Discarded(DropCause),
}

impl Verdict {
    /// ★Revival definition — one set★: if the terminal cause is `Unknown`, it is a revivable tombstone. It does not matter which event
    /// produced that unknown (vendor closing · cancel response · acceptance unknown) — 「벤더가 준 사실이 모름을 이긴다」 (a fact the vendor gave beats unknown).
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

/// The registry body — items (non-terminal · in entry order) + tombstones + two incarnation fields.
///
/// The slot of one id is one of three: **item** · **tombstone** (terminal — no body, just the id and one 「되살림 가능」 (revivable) flag)
/// · **unknown** (neither).
#[derive(Debug, Default)]
pub struct Registry {
    items: Vec<QueuedRow>,
    tombstones: Tombstones,
    /// The 「받음 불가 판명」 (acceptance-impossible finding) marker — `AckUnavailable` was reduced. ★The reduction rules do not read this field★ — it is only the material
    /// for the core to rewrite a `Queued` that stood late after the finding.
    ack_unavailable_seen: bool,
    /// The seq of the last list event reduced (including dropped events — those also entered the ring prefix).
    as_of_seq: Option<u64>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reduce one event standing at the ring's `seq` slot. Returns = the id whose outcome was settled by this event and that outcome (in reduction order).
    ///
    /// ★It does not return only item closures★ — a terminal that left only a tombstone on an unknown id, a revival (`Delivered`), and the re-reading
    /// of a rejection after receipt (`Discarded(Rejected)`) are also returned as outcomes. Whether an item existed is told apart by the caller from the pre-reduction
    /// state.
    /// ★Call precondition = `seq` grows with every call (ring order)★.
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
            QueuedInputEvent::HandedOver { id, sent } => self.on_handed_over(id, *sent),
            QueuedInputEvent::Delivered { id } => self.on_delivered(id, &mut closed),
            QueuedInputEvent::Dropped { id, cause } => self.on_dropped(id, *cause, &mut closed),
            QueuedInputEvent::AckUnavailable { delivered } => {
                self.on_ack_unavailable(delivered, &mut closed)
            }
        }
        closed
    }

    /// Open items (in entry order — the oldest first).
    pub fn rows(&self) -> &[QueuedRow] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn ack_unavailable_seen(&self) -> bool {
        self.ack_unavailable_seen
    }

    /// `None` = not a single list event of this incarnation has been reduced yet.
    pub fn as_of_seq(&self) -> Option<u64> {
        self.as_of_seq
    }

    /// Copies of the bodies of the items `AckUnavailable` closes with a receipt (= all open items) — used when filling in that event's `delivered`
    /// before writing it into the ring (the decoder emits it empty).
    pub fn open_copies(&self) -> Vec<DeliveredCopy> {
        self.items
            .iter()
            .map(|row| DeliveredCopy {
                id: row.id.clone(),
                text: row.text.clone(),
            })
            .collect()
    }

    /// That id's tombstone. `Some(true)` = revivable · `None` = no tombstone (an item or unknown).
    pub fn tombstone(&self, id: &str) -> Option<bool> {
        self.tombstones.get(id)
    }

    pub fn tombstone_count(&self) -> usize {
        self.tombstones.order.len()
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|row| row.id == id)
    }

    /// Delete the item and fold it into a tombstone — the body is not held for the whole incarnation lifetime.
    fn close(&mut self, at: usize, verdict: Verdict, closed: &mut Vec<(String, Verdict)>) {
        let row = self.items.remove(at);
        self.tombstones.insert(&row.id, verdict.resurrectable());
        closed.push((row.id, verdict));
    }

    fn on_queued(&mut self, id: &str, text: &str) {
        // If it is already an item, a no-op (no re-emit — a second defense) · if a tombstone, dropped (so a late `Queued` does not create a permanent
        // item).
        if self.position(id).is_some() || self.tombstones.get(id).is_some() {
            return;
        }
        self.items.push(QueuedRow {
            id: id.to_owned(),
            text: text.to_owned(),
            phase: RowPhase::Queued,
            sent: false,
        });
    }

    /// Hand-over marker — if an item, only the marker is swapped regardless of stage (no outcome). An unknown id · tombstone (a marker that came late after receipt) is
    /// a no-op — a marker does not invent a row.
    fn on_handed_over(&mut self, id: &str, sent: bool) {
        if let Some(at) = self.position(id) {
            self.items[at].sent = sent;
        }
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

    /// `removed:true` = the vendor took that text out of its own queue — it cannot be drained again, so we close it without waiting for the lifecycle line.
    /// If we waited, on the day that line never comes, the cancelled item would stay holding the list and block mail.
    fn on_removed(&mut self, id: &str, closed: &mut Vec<(String, Verdict)>) {
        if let Some(at) = self.position(id) {
            if matches!(self.items[at].phase, RowPhase::Cancelling { .. }) {
                self.close(at, Verdict::Cancelled, closed);
            }
        }
    }

    /// `removed:false` · request failure. ★It is not returned to waiting★ — the answer field (`NotRemoved`) tells apart the cause of the vendor closing that follows
    /// (drawing renders this row without a ✕ like a handed-over row — ADR-0235).
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
            // The vendor had already closed it and it was not our cancel.
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
            // A vendor receipt arrived late for an item dropped as 「모름」 (unknown). The bubble is drawn by the vendor echo that follows.
            Some(true) => {
                self.tombstones.clear_resurrectable(id);
                closed.push((id.to_owned(), Verdict::Delivered));
            }
            // A second `Delivered` of the same id (codex emits it on both `item/started` and `item/completed`) also ends here.
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
                // A failed · interrupted turn closed it — our cancel had already answered 「못 뺐다」 (could not remove).
                (
                    RowPhase::Cancelling {
                        answer: CancelAnswer::NotRemoved,
                        vendor_closed: false,
                    },
                    DropCause::Unknown,
                ) => self.close(at, Verdict::Discarded(DropCause::Unknown), closed),
                // If our cancel removed it, its response follows in the same ms — we leave it as cancel-pending in the list and wait.
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
                // The vendor closing has already been seen — a second closing carries no new fact.
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
            // Rejection after receipt — the vendor `refused` also arrives as a terminal after `started`. Re-read the outcome and block revival.
            Some(_) if cause == DropCause::Rejected => {
                self.tombstones.clear_resurrectable(id);
                closed.push((id.to_owned(), Verdict::Discarded(DropCause::Rejected)));
            }
            Some(_) => {}
        }
    }

    /// Close all open items with a receipt at that ring slot — the vendor already received them and the slot is only the moment of finding. Cancel-pending
    /// does not wait for a response either: an old CLI may silently drop a cancel line, so if we waited, that item would remain until the agent ends
    /// and block mail. The copy's id is reduced the same as the `Delivered` that follows (what was an item has just been closed so
    /// a no-op · an unknown id gets only a tombstone).
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

/// Tombstone set — a FIFO holding entry order. ★It is a set★: inserting an id that already exists is a no-op and does not move its slot.
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

/// One incarnation's registry — the core owns this Arc, and the session reaches it through the core (`AgentSession::queued_inputs()`).
/// The reading side (list query · cancel validation) takes only this one lock.
// ADR-0231
#[derive(Debug, Default)]
pub struct QueuedInputs {
    registry: Mutex<Registry>,
}

impl QueuedInputs {
    pub fn new() -> Self {
        Self::default()
    }

    /// ★Do not emit, do IO, or take other locks while holding this guard★ — it is a leaf of the lock order.
    /// Poison is unwrapped: the reducer calls nothing outside so the only way poison gets in is the reducer's own panic, and at that price there is no reason
    /// to kill the callers that read the list.
    /// ★Not opened outside the crate (`reduce` too)★ — if the reducer is reached through the guard, the registry can be changed outside the replay lock,
    /// which breaks 「명부 = 링 접두의 환원값」 (registry = the reduced value of the ring prefix). Outside, read with [`Self::snapshot`].
    pub(crate) fn lock(&self) -> MutexGuard<'_, Registry> {
        sync::lock(&self.registry)
    }

    /// Read the list rows and the seq of the last list event those rows reduced together **under one lock** — rows = the reduced value of the ring prefix
    /// `as_of_seq`. seq is compared only within one incarnation.
    pub fn snapshot(&self) -> (Vec<QueuedRow>, Option<u64>) {
        let registry = self.lock();
        (registry.items.clone(), registry.as_of_seq)
    }
}

impl CancelAnswer {
    /// The `cancel.answer` word of the list query response — the bus and WS carry the same word.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unanswered => "none",
            Self::NotRemoved => "not_removed",
        }
    }
}

/// The row state the list query carries — the registry state ([`RowPhase`]) with the path's acceptance-unknown marker laid on top.
///
/// ★`Unconfirmed` is not a reduction state★ — it is the path state at the query moment (`AgentTransport::unconfirmed_inputs`), so
/// it is not one atom with the row snapshot (`as_of_seq`). The reattach comparison reads it as `Queued`.
/// `Sent` = a `Queued` row where the hand-over marker (`QueuedRow::sent`) stands — it is a reduction state, so it is one atom with `as_of_seq`. ★The marker of a cancel-pending row
///   is not carried★ (the `cancelling` word wins — the price of not growing the row shape).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListedState {
    Queued,
    Sent,
    Unconfirmed,
    Cancelling {
        answer: CancelAnswer,
        vendor_closed: bool,
    },
}

impl ListedState {
    /// The `state` word of the list query response.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::Unconfirmed => "unconfirmed",
            Self::Cancelling { .. } => "cancelling",
        }
    }
}

/// One line of the list query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedRow {
    pub id: String,
    pub text: String,
    pub state: ListedState,
}

/// The answer of one list query — the bus `agent.listQueuedInputs` and the WS list query carry this same value.
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedListing {
    pub rows: Vec<ListedRow>,
    /// The seq of the last list event the rows reduced. `None` = this incarnation has no list event yet. Compared only within `epoch`.
    pub as_of_seq: Option<u64>,
    pub epoch: u32,
    /// Exactly the turn observation table's `last_end_failed` — false if the table has no entry.
    pub stopped_after_error: bool,
}

/// Lay the acceptance-unknown marker on registry rows — only `Queued` rows change (cancel-pending wins · a row with a hand-over marker stays `Sent` —
/// the reduction state beats the marker at the query moment). An id not in the registry is dropped (no row is invented).
pub(crate) fn overlay_unconfirmed(rows: Vec<QueuedRow>, unconfirmed: &[String]) -> Vec<ListedRow> {
    rows.into_iter()
        .map(|row| {
            let state = match row.phase {
                RowPhase::Queued if row.sent => ListedState::Sent,
                RowPhase::Queued if unconfirmed.contains(&row.id) => ListedState::Unconfirmed,
                RowPhase::Queued => ListedState::Queued,
                RowPhase::Cancelling {
                    answer,
                    vendor_closed,
                } => ListedState::Cancelling {
                    answer,
                    vendor_closed,
                },
            };
            ListedRow {
                id: row.id,
                text: row.text,
                state,
            }
        })
        .collect()
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

    /// The golden's events are in the wire `op` shape as is (the `kind` discriminator) — domain types carry no serde, so here
    /// we carry them over by hand.
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
            "HandedOver" => QueuedInputEvent::HandedOver {
                id: text(v, "id"),
                sent: v["sent"].as_bool().expect("sent"),
            },
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

    /// A row's JSON shape = the same words as the list query response row (`state` · `cancel`). The hand-over marker is carried as `"sent": true` only on
    /// standing rows (golden `_format`) — a shape chosen so cases without the marker need not be changed.
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
        let mut json =
            serde_json::json!({ "id": row.id, "text": row.text, "state": state, "cancel": cancel });
        if row.sent {
            json["sent"] = Value::Bool(true);
        }
        json
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

    // ── Golden (shared with the TS accumulator) ────────────────────────────────────────────────────

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

    /// The golden feeds the vocabulary at least once without omission — it blocks a new event · cause from entering without a golden.
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
                    "HandedOver" => format!("HandedOver:{}", ev["sent"]),
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
            "HandedOver:true",
            "HandedOver:false",
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

    // ── Surface of the Rust edition only (snapshot · copy · seq) ─────────────────────────────────────

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
                    sent: false,
                },
                QueuedRow {
                    id: "b".into(),
                    text: "둘".into(),
                    phase: RowPhase::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false,
                    },
                    sent: false,
                },
            ]
        );
    }

    /// Dropped events also entered the ring prefix — the snapshot seq goes up to that slot (the reattach comparison re-reduces only events after this seq,
    /// so stopping at a dropped event would reduce that event twice).
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

    #[test]
    fn the_unconfirmed_mark_lands_only_on_queued_rows_and_invents_none() {
        let row = |id: &str, phase| QueuedRow {
            id: id.into(),
            text: format!("text of {id}"),
            phase,
            sent: false,
        };
        let cancelling = RowPhase::Cancelling {
            answer: CancelAnswer::NotRemoved,
            vendor_closed: true,
        };
        let sent = |id: &str, phase| QueuedRow {
            sent: true,
            ..row(id, phase)
        };
        let listed = overlay_unconfirmed(
            vec![
                row("plain", RowPhase::Queued),
                row("held", RowPhase::Queued),
                row("withdrawing", cancelling),
                sent("pushed", RowPhase::Queued),
                sent("pushed-then-cancelled", cancelling),
            ],
            &[
                "held".into(),
                "withdrawing".into(),
                "pushed".into(),
                "not-listed".into(),
            ],
        );
        let states: Vec<(&str, ListedState)> =
            listed.iter().map(|r| (r.id.as_str(), r.state)).collect();
        assert_eq!(
            states,
            vec![
                ("plain", ListedState::Queued),
                ("held", ListedState::Unconfirmed),
                (
                    "withdrawing",
                    ListedState::Cancelling {
                        answer: CancelAnswer::NotRemoved,
                        vendor_closed: true
                    }
                ),
                ("pushed", ListedState::Sent),
                (
                    "pushed-then-cancelled",
                    ListedState::Cancelling {
                        answer: CancelAnswer::NotRemoved,
                        vendor_closed: true
                    }
                ),
            ],
            "취소 대기가 두 표지를 이기고, 넘김 표지가 수락 모름을 이기며, 명부에 없는 id 는 행이 되지 않는다"
        );
        assert_eq!(listed[1].text, "text of held");
    }

    /// The words are spelled the same as the golden rows (`row_json`) — the list query response carries these words.
    #[test]
    fn the_listing_words_are_pinned() {
        assert_eq!(ListedState::Queued.as_str(), "queued");
        assert_eq!(ListedState::Sent.as_str(), "sent");
        assert_eq!(ListedState::Unconfirmed.as_str(), "unconfirmed");
        assert_eq!(
            ListedState::Cancelling {
                answer: CancelAnswer::Unanswered,
                vendor_closed: false
            }
            .as_str(),
            "cancelling"
        );
        assert_eq!(CancelAnswer::Unanswered.as_str(), "none");
        assert_eq!(CancelAnswer::NotRemoved.as_str(), "not_removed");
    }
}
