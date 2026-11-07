//! Integration test of the daemon client's pending slot succession (`daemon_client::connection::register_pending`) —
//! zero sockets · daemons · Tauri.
//!
//! ★Why this file is in `tests/` (the integration target)★: what it measures is **wiring that requires a real socket**, so an integration harness that
//! cuts that spot off with a port is the right place (so the harness itself uses no socket · daemon · Tauri at all).
//! ★It is "requires", not "cannot be stood up"★ — `src/daemon_client/tests.rs` actually runs that wiring over a loopback WS. What is bought here is
//! determinism and speed without a socket, not the only possibility.
//! Pure unit assertions go in `#[cfg(test)]` next to the module and those run with `--test lib_unit`
//! (the decision that set up that target = ADR-0174 · the current state = `.claude/skill-bindings/qa.md` 「모듈별 명령」).
//! Run: `cargo test -p engram-dashboard --test daemon_client_pending` (it starts no child process at all, so
//! do not add `-- --test-threads=4` — canonical decision rule = `.claude/skill-bindings/qa.md` 「모듈별 명령」).
//! CI also has a dedicated step that calls only this target (`.github/workflows/ci.yml`).
//!
//! ## ★What it guards — overlapping numbers do not kill the connection★
//! The key of this map (`request_id`) is no longer a value we make. The number of `AgentCommand::Command { envelope }` belongs to
//! **the outer caller that built the envelope**, and the webview carries it as is with `forward_daemon_command`.
//! If an overlapping number hit the old code's `debug_assert!(false)`, the connection task would panic and **every later command would be cut off
//! with no reconnect either** (debug-build GUI measured 2026-08-18 — `tab.list` + `window.list` with the same number from the screen).
//! So what is measured here is three things: no panic · the old waiter **wakes up with an error** (no permanent hang) ·
//! the new request **succeeds to that number**.
//!
//! ★Why no separate panic assertion★: if the `debug_assert` came back to life, that test of this target (debug build = `debug_assertions`
//! on) **dies on the spot**. That is, the assertions below are themselves the panic-regression detector.
//!
//! ## ★What is not covered (honesty)★
//! The connection task's `select!` arm itself (the wiring that carries a command from the socket to this function) requires a real socket, so it does not
//! run here — the same residual that the `tests/layout_commands.rs` header records. What is measured here is
//! the **rule body** that arm calls.

use engram_dashboard_lib::daemon_client::connection::{
    register_pending, CommandReply, PENDING_SUPERSEDED,
};
use engram_dashboard_lib::daemon_client::protocol_state::{take_pending, PendingMap};
use engram_dashboard_protocol::{AgentEvent, RequestId};
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;

// The same type as the map the connection task carries — the value is a oneshot for request/response correlation (`connection::CommandReply`).
type Pending = PendingMap<CommandReply>;

// This harness's generation value — used only as a log field, so any value would do, but pinning it makes failure logs readable.
const GEN: u64 = 7;

// Hang one slot and return that request's waiter (the receiving end).
fn register(
    pending: &mut Pending,
    rid: RequestId,
) -> oneshot::Receiver<Result<AgentEvent, String>> {
    let (tx, rx) = oneshot::channel();
    register_pending(pending, rid, tx, GEN);
    rx
}

// Not yet replied = waiting with the slot alive (distinguish from `Closed` — that one means the slot is gone).
fn assert_still_waiting(rx: &mut oneshot::Receiver<Result<AgentEvent, String>>, what: &str) {
    match rx.try_recv() {
        Err(TryRecvError::Empty) => {}
        other => panic!("{what} 는 답장 전까지 살아서 대기해야 한다: {other:?}"),
    }
}

// ── Succession ────────────────────────────────────────────────────────────────────

/// ★Core regression★ Even if two requests come in with the same number, (1) it does not die (2) the old waiter wakes up with an error, and
/// (3) the new request succeeds to that number.
///
/// The old code panicked here with `debug_assert!(false)` — that panic folded the whole connection task and
/// the shell↔daemon was cut off permanently. If revived, this test dies at (1).
#[test]
fn duplicate_request_id_supersedes_instead_of_panicking() {
    let mut pending = Pending::new();
    let rid = RequestId::new();

    let mut old = register(&mut pending, rid);
    let mut new = register(&mut pending, rid); // The same number — a value decided by the outer caller, so it can overlap.

    // (2) The old waiter **wakes up with an error** — if dropped quietly, that caller hangs permanently.
    match old.try_recv() {
        Ok(Err(msg)) => assert_eq!(
            msg, PENDING_SUPERSEDED,
            "옛 대기자가 받는 문구는 그 상수 하나여야 한다"
        ),
        other => panic!("옛 대기자는 Err 로 깨어나야 한다: {other:?}"),
    }
    assert_still_waiting(&mut new, "새 대기자");

    // (3) The slot left in the map is **one** and it is the new request's — a reply with that number releases the new waiter.
    assert_eq!(pending.len(), 1, "같은 번호는 한 칸을 나눠 쓴다");
    take_pending(&mut pending, &rid)
        .expect("승계한 슬롯이 그 번호로 잡혀야 한다")
        .send(Ok(AgentEvent::Ack { request_id: rid }))
        .expect("새 대기자가 살아 있어야 한다");
    match new.try_recv() {
        Ok(Ok(AgentEvent::Ack { request_id })) => {
            assert_eq!(request_id, rid, "승계한 쪽이 그 번호의 답장을 받는다")
        }
        other => panic!("새 대기자가 답장을 받아야 한다: {other:?}"),
    }
}

/// After succession, **even if a reply with the same number comes again** nobody is left hanging — the second is ignored as an unpaired reply.
///
/// The daemon builds a typed answer for an overlapping number (`Seat::Conflict` of `command_delivery` →
/// `REQUEST_ID_CONFLICT`, `Seat::Taken` → `REQUEST_ID_CONFLICT`/`OUTCOME_UNKNOWN`). That answer comes back into this single field,
/// so that after one reply releases the slot the rest have nowhere to go is the normal state of this path.
#[test]
fn second_reply_for_same_id_finds_no_slot() {
    let mut pending = Pending::new();
    let rid = RequestId::new();

    let mut old = register(&mut pending, rid);
    let mut new = register(&mut pending, rid);

    assert!(
        matches!(old.try_recv(), Ok(Err(_))),
        "옛 대기자는 이미 깨어났다"
    );
    take_pending(&mut pending, &rid)
        .expect("승계한 슬롯")
        .send(Ok(AgentEvent::Ack { request_id: rid }))
        .expect("새 대기자 생존");
    assert!(
        matches!(new.try_recv(), Ok(Ok(_))),
        "첫 답장이 새 대기자를 푼다"
    );

    // The second reply — there is no slot to take out (ignored). The old slot must not come back to life here.
    assert!(
        take_pending(&mut pending, &rid).is_none(),
        "짝 없는 답장은 꺼낼 슬롯이 없다"
    );
    assert!(pending.is_empty(), "맵에 좀비 슬롯이 남지 않는다");
}

/// Different numbers do not wake each other — if succession is widened to 「항상 옛것을 버린다」 (always discard the old one), this assertion breaks.
#[test]
fn distinct_request_ids_coexist() {
    let mut pending = Pending::new();
    let (a, b) = (RequestId::new(), RequestId::new());

    let mut wait_a = register(&mut pending, a);
    let mut wait_b = register(&mut pending, b);

    assert_eq!(pending.len(), 2, "번호가 다르면 칸도 다르다");
    assert_still_waiting(&mut wait_a, "a");
    assert_still_waiting(&mut wait_b, "b");

    // Each is released only by the reply of its own number.
    take_pending(&mut pending, &a)
        .expect("a 슬롯")
        .send(Ok(AgentEvent::Ack { request_id: a }))
        .expect("a 대기자 생존");
    assert!(matches!(wait_a.try_recv(), Ok(Ok(_))), "a 만 풀린다");
    assert_still_waiting(&mut wait_b, "b");
}

/// Even succeeding a slot whose waiter is already gone (caller canceled) proceeds quietly — it does not die because there is nowhere to wake.
#[test]
fn supersede_survives_dropped_waiter() {
    let mut pending = Pending::new();
    let rid = RequestId::new();

    drop(register(&mut pending, rid)); // The old waiter left first.
    let mut new = register(&mut pending, rid);

    assert_eq!(pending.len(), 1);
    assert_still_waiting(&mut new, "새 대기자");
    take_pending(&mut pending, &rid)
        .expect("승계한 슬롯")
        .send(Ok(AgentEvent::Ack { request_id: rid }))
        .expect("새 대기자 생존");
    assert!(matches!(new.try_recv(), Ok(Ok(_))));
}
