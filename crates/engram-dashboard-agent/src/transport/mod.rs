//! AgentTransport — one agent's data channel + resource control seam. A transport only produces bytes/events for
//!   `OutputCore::emit`/`finish`; fanout · terminal transition · subscription are OutputCore's, and the transport owns only its own
//!   resources' lifetime. Zero tauri imports.

use std::sync::Arc;
use std::time::Duration;

use crate::launch::Notice;
use crate::output_core::OutputCore;
use crate::types::{InputEvent, OutputEvent, PtyError, TransportCaps, TurnInput, Withdraw};

pub mod api;
pub mod input_queue;
pub(crate) mod process_group;
pub mod pty;
pub(crate) mod spawn;
pub mod stdio;

/// Output bytes → OutputEvent refinement seam (backend-agnostic — ADR-0004/0045).
///
/// ★Why (ADR-0004)★: the transport (StdioTransport) is a **dumb pipe** and must not know the child stdout schema (claude stream-json /
///   codex protocol / plain text), yet json mode needs structured OutputEvents; that parsing knowledge is the backend's (claude:
///   `ClaudeStreamDecoder`, in backend/claude/ alone). It hides behind this trait and **the transport applies an injected one "without
///   knowing which"**.
/// ★State (owned solely by the pump thread)★: the decoder holds **mutable state** (partial-line buffer) owned exclusively by the single
///   pump thread via `&mut` — `Send` only, not `Sync`. A new epoch = new transport = new decoder, so reset is automatic.
/// Produces only agent domain types (OutputEvent) — no Serialize (ADR-0003: the agent does not know the wire).
pub trait OutputDecoder: Send {
    /// Push in byte chunks of any size; return only the events **completed** by this chunk. An incomplete tail (a partial line without
    ///   newline) stays in the implementation's buffer and joins the next chunk.
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent>;

    /// Called at most once at stream end — handles the remaining line not terminated by a newline.
    fn flush(&mut self) -> Vec<OutputEvent>;

    /// **Take out and empty** the usage-limit observations collected so far (arrival order). The output pump calls it right after every
    ///   `decode` · `flush`, so the pile is one chunk's worth.
    /// ★Default = empty vector★: decoders of streams without usage do not implement it.
    /// ★Not an `OutputEvent`★: usage is account-level state and must not ride per-agent replay · subscriber fan-out — the pump passes it
    ///   through [`crate::types::StatusSink::usage_observed`].
    /// ★A wrapping decoder must pass this through too★ — the default body compiles even if missed, then a pass-through wrapper silently
    ///   loses observations and a blocking wrapper ([`crate::usage::UsageGate`]) never empties the inside, growing the vector for the
    ///   agent's lifetime.
    fn take_usage(&mut self) -> Vec<crate::usage::UsageObservation> {
        Vec::new()
    }
}

// ★Do not revive the seam that flows PTY output bytes into a backend closure (ADR-0217/ADR-0218)★ — its only consumer was codex's TUI
//   status-line scanner, withdrawn when recovery moved from screen reading to **lock ownership** (now not through the path at all).
//   Reviving it puts someone else's
//   code above the pump (the only PTY reader): a lock there becomes child backpressure, a panic a 「healthy session reported as Failed」.
// ADR-0217
// ADR-0218

pub trait AgentTransport: Send + Sync {
    /// Start the output pump/stream → connect to the core. Called once right after spawn.
    fn start(&self, core: Arc<OutputCore>);

    /// ★Acceptance means **「received」, not 「gone」**★: `Ok` = all accepted with order fixed, `Err` = not accepted (a partial acceptance is
    ///   never `Ok`). All three implementations put it into a bounded queue drained by a dedicated writer, so **this call does not wait
    ///   for the OS write** — the
    ///   point of this seam (a child not reading stdin does not hang the caller).
    /// ★Price: a write that fails after acceptance cannot return to this caller★ — it appears as **the next call's `Err`**, the log, and
    ///   usually the terminal transition. Contract · cap · handling: the [`input_queue`] module header (console family) and the codex
    ///   path's `send_input` doc.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;

    /// Hand over one turn with its origin — the session calls it only under `MidTurnPolicy::TransportOwned`; acceptance meaning as
    ///   [`AgentTransport::send_input`].
    /// ★Default = the `send_input(Raw)` body★ — paths that do not classify turns (PTY · stdio) do not implement it. Adding an `InputEvent`
    ///   variant was rejected: it would break those paths' irrefutable pattern (`let InputEvent::Raw(b) = input`).
    // ADR-0231
    fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
        self.send_input(InputEvent::Raw(turn.body))
    }

    /// Withdraw one pending input — the route for the session's cancel under `MidTurnPolicy::TransportOwned`. List events (withdrawal ·
    ///   cancel request) are emitted **by the path** — only it knows whether it holds the item or handed it over. ★Default = `NotHeld`★ (a
    ///   path holding nothing).
    // ADR-0231
    fn withdraw(&self, _id: &str) -> Withdraw {
        Withdraw::NotHeld
    }

    /// Ids of pending inputs held as acceptance unknown — handed over where it is unknown whether the vendor received them (the list query
    ///   overlays them as `unconfirmed`); that transition has no list event, so the registry keeps `Queued`.
    /// ★Path state at the query moment★ — not atomic with the registry snapshot; the caller calls it **after** releasing the registry lock
    ///   (locks never overlap — ADR-0006).
    /// ★Default = empty list; no path overrides it today★ — the codex path also withdrew the acceptance-unknown stage (ADR-0235). This
    ///   verb and the list word `unconfirmed` stay dormant.
    // ADR-0231
    fn unconfirmed_inputs(&self) -> Vec<String> {
        Vec::new()
    }

    /// ★Wait until the input accepted up to this call **has actually gone out**★ — `Ok` = gone out.
    ///
    /// ★Why★: [`AgentTransport::send_input`]'s `Ok` is 「received」, not 「gone」; one caller **must not put that difference on the delivery
    ///   receipt** — mail delivery (ADR-0088's "transmission failure" vs "model ignored" would flip). Only it passes this verb.
    /// ★★The keystroke path must not call this★★: waiting would bring back the head-of-line blocking the queue removed. Before adding a
    ///   call site, answer whether it is **already a path that accepts blocking**.
    /// ★Default `Unsupported` is intended — never disguised as `Ok`★: a path with no means to confirm write completion returning `Ok`
    ///   would be that flip; it is more honest to **say** it has none, and the caller treats it as 「knowledge at the same level as before」
    ///   (judged at the call site).
    fn flush_input(&self, _timeout: Duration) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "flush_input (this link has no means of confirming write completion)".into(),
        ))
    }

    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;

    /// ≠kill. Stops only the work in progress — the process stays alive.
    fn interrupt(&self) -> Result<(), PtyError>;

    /// A notice that this path will retire soon — called with no lock at the first step where the manager decides to end this incarnation
    ///   (before revoking permissions · `Exiting` · [`Self::shutdown`]); a path with background work can stop it.
    /// ★Does not reap resources — that is [`Self::shutdown`]'s job (ADR-0001 kill causality unchanged)★. Must not block (the kill path
    ///   stands behind it) and must be idempotent.
    /// ★Emits no externally observable outcome here (state transition · connection outcome delivery · input rejection)★ — this notice
    ///   comes **before** the user's termination intent ([`crate::session::AgentSession::termination_intent`]) stands; an outcome seen
    ///   first would make the
    ///   activation decision read the user's cancel as a failure.
    /// ★Not every ending path calls this first★ — a path needing to know also sets the same in [`Self::shutdown`]. Default = no-op.
    fn begin_retire(&self) {}

    /// Forced resource termination (idempotent). Waiting for pump termination is not done here (it is core.join_pump's job).
    fn shutdown(&self);

    fn capabilities(&self) -> TransportCaps;
}

/// The place where this path **delivers** the connection outcome. `None` = there is no connection to establish (PTY · stdio are usable
///   once the process is up).
///
/// ★★Not put in a cell to **wait to be read** — the reason this port exists★★: the old shape kept connection state in a cell the
///   supervisor **read** in a loop, and the gap caused four defects — the path moving between read and act left the supervisor deciding on
///   a vanished fact
///   (① cleanup of a reaped incarnation kills a live successor ② `Ready` after reading `Establishing` still ends in deadline failure ③ a
///   `Down` from a user kill recorded as a resume failure ④ `Up` remaining after stdout EOF read as success). A **delivered** value is
///   never withdrawn.
/// ★The incarnation marker is **stamped by the assembly point**★ — the port is `Fn(LinkResolution)` and the path does not know the
///   incarnation (as [`SessionIdSink`]); consumers read the marker **only as match/mismatch** (ADR-0163/0164 — deriving 「newer」 by
///   magnitude brings ① back).
/// ★★The contract is **at most once** (zero-or-once), not 「exactly once」★★: **Once** — when the connection yields an outcome, never after.
///   **Zero** — the user turned this incarnation off; a handshake released by kill is not 「the peer answered」 (delivering it would record
///   the user's cancel
///   as a resume failure — defect ③).
/// ★★**The receiver must handle 「zero」**★★: this spot once said 「exactly once」 and 「not called if turned off」 together; a consumer
///   trusting the former — the channel-gone branch of [`crate::manager::AgentManager::link_activation_verdict`] — read the dropped sending
///   end as 「no outcome」 and
///   **recorded the user's cancel as an activation failure**; that branch now checks the termination-intent latch **first**.
/// ★Only the message is suppressed, **not the channel-gone**★: the holder is a single writer thread whose end drops the sender, which
///   cannot be suppressed — so 「zero」 is observable as **channel-gone**, not an outcome.
/// ★Delivery does not **drive** termination★ — the path still kills nothing and does not call `finish` (the ADR-0199 line): stating an
///   outcome ≠ enforcing it.
pub type LinkSink = Arc<dyn Fn(LinkResolution) + Send + Sync>;

/// How the connection ended — ★only two values and both are **confirmed**★ (「not yet」 is not delivered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// The peer **signaled that it is ready to receive input** = activation established (user decision).
    /// `notices` = run notices the link raised while setting up (codex: the post-open hook check) — the manager appends
    ///   them to the launch's notices, outside the notice fingerprint, so they are given on every launch they occur
    ///   (ADR-0341 decision 5 as amended by ADR-0420). Empty for most links and launches.
    // ADR-0420
    Ready { notices: Vec<Notice> },
    /// The connection could not be established. `reason` = human-readable and the input to backend classification — this path's failure
    ///   text is a JSON-RPC error on stdout, in neither the console tail nor the stderr diagnostic tail.
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ControlCaps, InputCaps, InputOrigin, OutputCaps};
    use std::sync::Mutex;

    struct RawOnly(Mutex<Vec<Vec<u8>>>);
    impl AgentTransport for RawOnly {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.0.lock().unwrap().push(bytes);
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: true,
                    structured: false,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    #[test]
    fn the_default_send_turn_hands_the_body_to_send_input_byte_for_byte() {
        let t = RawOnly(Mutex::new(Vec::new()));
        for origin in [InputOrigin::User, InputOrigin::Mail] {
            t.send_turn(TurnInput {
                id: "id-1".into(),
                body: b"echo hi\r\n\x03".to_vec(),
                origin,
            })
            .unwrap();
        }
        assert_eq!(
            *t.0.lock().unwrap(),
            vec![b"echo hi\r\n\x03".to_vec(), b"echo hi\r\n\x03".to_vec()],
            "the default implementation passes only the body as is, regardless of origin — id and origin are not mixed into the bytes"
        );
    }

    #[test]
    fn the_default_withdraw_holds_nothing() {
        let t = RawOnly(Mutex::new(Vec::new()));
        assert_eq!(t.withdraw("id-1"), Withdraw::NotHeld);
        assert!(t.0.lock().unwrap().is_empty(), "withdrawing writes nothing");
    }

    #[test]
    fn the_default_transport_holds_no_unconfirmed_input() {
        let t = RawOnly(Mutex::new(Vec::new()));
        assert!(t.unconfirmed_inputs().is_empty());
    }
}

/// A fake shared by the containment tests of the three pumps (PTY · stdio · codex reader) — a subscriber panicking on the first frame
///   raises a panic contained inside the pump body and records the marker then and when `finish` announces the state.
// ADR-0292
#[cfg(test)]
pub(crate) mod pump_marks {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use engram_dashboard_base::panic::is_contained;

    use crate::output_core::{OutputCore, TurnWiring};
    use crate::types::{
        AgentId, AgentInfo, AgentStatus, OutputFrame, OutputSink, SinkError, SinkId, StatusSink,
    };

    #[derive(Default)]
    pub(crate) struct Marks {
        fired: AtomicBool,
        at_send: Mutex<Vec<bool>>,
        at_status: Mutex<Vec<(AgentStatus, bool)>>,
    }

    impl StatusSink for Marks {
        fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
            self.at_status
                .lock()
                .unwrap()
                .push((status, is_contained()));
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    struct PanicAtFirstSend(Arc<Marks>, SinkId);

    impl OutputSink for PanicAtFirstSend {
        fn send(&self, _frame: OutputFrame<'_>) -> Result<(), SinkError> {
            if self.0.fired.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            self.0.at_send.lock().unwrap().push(is_contained());
            panic!("subscriber panic inside emit");
        }
        fn sink_id(&self) -> SinkId {
            self.1
        }
    }

    pub(crate) fn core(marks: &Arc<Marks>) -> Arc<OutputCore> {
        let core = Arc::new(OutputCore::new(
            AgentId::new_v4(),
            1,
            marks.clone(),
            TurnWiring::detached(),
        ));
        core.subscribe(Arc::new(PanicAtFirstSend(marks.clone(), SinkId::new_v4())));
        core
    }

    /// The pump body panicked inside the marker, that panic was folded into one `Failed`, and `finish` announced outside the marker.
    pub(crate) fn assert_contained_body_and_unmarked_finish(marks: &Marks) {
        assert_eq!(
            *marks.at_send.lock().unwrap(),
            vec![true],
            "the pump body did not run inside the marker"
        );
        match marks.at_status.lock().unwrap().as_slice() {
            [(AgentStatus::Failed { message }, marked)] => {
                assert!(message.starts_with("pump panicked: "), "{message}");
                assert!(
                    message.contains("subscriber panic inside emit"),
                    "{message}"
                );
                assert!(
                    !marked,
                    "finish ran inside the marker — if a panic in it is contained, `Running` remains"
                );
            }
            other => panic!("a contained panic must be Failed once: {other:?}"),
        }
    }
}
