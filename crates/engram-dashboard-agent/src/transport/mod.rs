//! AgentTransport — one agent's data channel + resource control seam.
//!
//! A transport only needs to produce bytes/events and hand them to `OutputCore::emit`/`finish`.
//! Shared logic such as output fanout · terminal transition · subscription is handled by OutputCore, and the transport
//! is responsible only for the lifetime of its own resources.
//!
//! tauri import 0.

use std::sync::Arc;
use std::time::Duration;

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
/// ★Why this trait is needed (ADR-0004 isolation)★: the transport (StdioTransport) is a **dumb pipe**, so
///   it must not know what schema the child stdout bytes are (claude stream-json / codex protocol / plain text).
///   But the json mode must refine those bytes into structured OutputEvents — that parsing knowledge is owned by
///   the backend (for claude, `ClaudeStreamDecoder`, in backend/claude/ alone). So the parsing logic is hidden behind this
///   trait and **the transport only has one injected and applies it "without knowing which decoder it is"**.
///
/// ★Lifetime · state (owned solely by the pump thread)★: the decoder holds **mutable state** such as a partial-line buffer for line reassembly,
///   and the pump thread (single) owns it exclusively by `&mut` — so it requires only `Send` (move to a thread)
///   and not `Sync` (no shared access). Epoch replacement = new transport = new decoder so
///   reset is automatic.
///
/// It produces only agent domain types (OutputEvent) — unrelated to Serialize (ADR-0003: the agent does not know the wire).
pub trait OutputDecoder: Send {
    /// Push in byte chunks of arbitrary size, and return only the events **completed** by this chunk.
    /// An incomplete tail (a partial line with no newline, etc.) is kept by the implementation in an internal buffer and combined with the next chunk.
    fn decode(&mut self, chunk: &[u8]) -> Vec<OutputEvent>;

    /// Called at most once at stream end — handles the remaining line not terminated by a newline.
    fn flush(&mut self) -> Vec<OutputEvent>;

    /// **Take out and empty** the usage-limit observations collected while interpreting so far (in arrival order). The caller is the output pump and
    /// it calls this right after every call of `decode` · `flush` — so the amount the implementation piles up is one chunk's worth.
    /// ★Default = an empty vector★: decoders of streams that do not carry usage do not implement it.
    /// ★Why it is not emitted as an `OutputEvent`★: usage is account-level state so it must not ride per-agent replay · subscriber fan-out
    ///   — the pump passes it separately through [`crate::types::StatusSink::usage_observed`].
    /// ★A wrapping decoder must pass this method through too★ — it has a default body so it compiles even if missed. Then a pass-through
    ///   wrapper silently loses observations, and a blocking wrapper ([`crate::usage::UsageGate`]) never empties the inside so that
    ///   vector grows for the whole agent lifetime.
    fn take_usage(&mut self) -> Vec<crate::usage::UsageObservation> {
        Vec::new()
    }
}

// ★Do not revive here the seam that flows PTY output bytes into a backend closure (ADR-0217/ADR-0218)★ —
//   its only consumer was codex's TUI status-line scanner, and it was withdrawn together when that recovery moved from screen reading to
//   **lock ownership decision** (today that recovery does not pass through the path at all). Reviving it brings back a structure where someone else's code runs above the pump (the only PTY
//   reader) — taking a lock or panicking there become child
//   backpressure and 「멀쩡한 세션이 Failed 로 보고」 (a healthy session reported as Failed) respectively.
// ADR-0217
// ADR-0218

pub trait AgentTransport: Send + Sync {
    /// Start the output pump/stream → connect to the core. Called once right after spawn.
    fn start(&self, core: Arc<OutputCore>);

    /// ★The acceptance meaning is **「받았다」 (received), not 「갔다」 (gone)**★: `Ok` = we accepted all of it with its order fixed,
    ///   `Err` = we did not accept it (a partial acceptance is not reported down as `Ok`). Today all three implementations put it into a bounded queue
    ///   and a dedicated writer pulls and writes, so **this call does not wait for the OS write** — that is the point of
    ///   this seam (even if the child does not read stdin, the caller does not hang with it).
    /// ★Its price = a write that fails after being accepted has no way back to this caller★. Where that failure appears is
    ///   **the `Err` of the next call**, the log, and usually the terminal transition that follows soon. The source of record for the contract · cap · handling is
    ///   the [`input_queue`] module header (console family) and the `send_input` doc of the codex path.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;

    /// Hand over one turn together with its origin — the session calls it only when `MidTurnPolicy::TransportOwned`. The acceptance meaning is
    /// the same as [`AgentTransport::send_input`].
    /// ★Default implementation = the body as `send_input(Raw)` as is★ — a path that does not classify turns by itself (PTY · stdio) does not
    ///   implement this. The route of adding a variant to `InputEvent` is not used: it would break those paths' irrefutable pattern
    ///   (`let InputEvent::Raw(b) = input`).
    // ADR-0231
    fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
        self.send_input(InputEvent::Raw(turn.body))
    }

    /// Withdraw one pending input — when the session's cancel is `MidTurnPolicy::TransportOwned`, it comes over here.
    /// List events (withdrawal · cancel request) are emitted **by the path** — only the path knows whether it holds that item or handed it over.
    /// ★Default implementation = `NotHeld`★ — a path that holds nothing.
    // ADR-0231
    fn withdraw(&self, _id: &str) -> Withdraw {
        Withdraw::NotHeld
    }

    /// The ids of pending inputs held as acceptance unknown — items handed over where it is unknown whether the vendor received them (the list query overlays them as `unconfirmed`).
    /// That transition has no list event so in the registry it stays `Queued`.
    /// ★It is the path state at the query moment★ — not one atom with the registry snapshot, and the caller calls it **after** releasing the registry lock
    ///   (the two locks are not held overlapping — ADR-0006).
    /// ★Default implementation = an empty list, and today no path overrides it★ — the codex path also withdrew the acceptance-unknown stage (ADR-0235) and uses the default
    ///   implementation. This verb and the list word `unconfirmed` stay dormant.
    // ADR-0231
    fn unconfirmed_inputs(&self) -> Vec<String> {
        Vec::new()
    }

    /// ★Wait until the input accepted up to the time of this call **has actually gone out**★ — `Ok` = gone out.
    ///
    /// ★Why it exists★: the `Ok` of [`AgentTransport::send_input`] is 「받았다」 (received), not 「갔다」 (gone). There is one caller that **must not put
    ///   the difference between the two on the delivery receipt** — mail delivery (the "transmission failure" and "model ignored" that ADR-0088 wants to tell apart get flipped).
    ///   Only that caller passes this verb and restores the meaning of `Ok`.
    /// ★★The keystroke path does not call this — it must not★★: waiting there would bring back the very
    ///   head-of-line blocking the queue meant to remove. Before attaching this verb to a new call site, first answer whether that call site is **already a path
    ///   that accepts blocking**.
    /// ★The default implementation being `Unsupported` is intended — it does not disguise itself as `Ok`★: if a path with no means to
    ///   confirm write completion returned `Ok`, that would be exactly the flip above. It is more honest for the side that **says** it has no means of confirmation, and
    ///   the caller can see that signal and treat it as 「예전과 같은 수준의 앎」 (knowledge at the same level as before) (that judgment is written by the call site).
    fn flush_input(&self, _timeout: Duration) -> Result<(), PtyError> {
        Err(PtyError::Unsupported(
            "flush_input (이 통로는 쓰기 완료를 확인할 수단이 없다)".into(),
        ))
    }

    /// Preserving cols/rows (atomic storage) is AgentSession's responsibility.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;

    /// ≠kill. Stops only the work in progress — the process stays alive.
    fn interrupt(&self) -> Result<(), PtyError>;

    /// A notice that this path will retire soon — called with no lock at the first step where the manager decides to end this incarnation (before revoking permissions · `Exiting` ·
    /// [`Self::shutdown`]). A path with work running in the background can see this and stop that work.
    /// ★It does not reap resources — that is [`Self::shutdown`]'s job and the kill causality (ADR-0001) stays as is★. It must not block
    ///   (the kill path stands behind it) and must be the same when called many times.
    /// ★It does not emit here the outcomes observable from outside (state transition · connection outcome delivery · input rejection)★ — this notice comes **before** the user's termination intent
    ///   ([`crate::session::AgentSession::termination_intent`]) stands. If an outcome were seen before the intent,
    ///   the activation decision would read the user's cancel as a failure.
    /// ★Not every ending path calls this first★ — a path that needs to know about retiring also sets the same
    ///   thing in [`Self::shutdown`]. Default implementation = no-op.
    fn begin_retire(&self) {}

    /// Forced resource termination (idempotent). Waiting for pump termination is not done here (it is core.join_pump's job).
    fn shutdown(&self);

    fn capabilities(&self) -> TransportCaps;
}

/// The place where this path **delivers** the connection outcome. `None` = there is no connection to establish (PTY · stdio can be used from the moment
/// the process is up, so this axis does not exist at all).
///
/// ★★Putting it in a cell and **waiting to be read** is not done — that is the reason this port exists★★:
///   In the old shape the path kept the connection state in a cell and the supervisor **read** it in a loop. The gap between was exactly four
///   defects — if the path moves between reading and acting, the supervisor decides on a fact that has already vanished.
///   (① the cleanup of a reaped incarnation kills a live successor ② even if `Ready` stands after reading `Establishing`, the deadline
///   confirms failure ③ a `Down` made by a user kill is recorded as a resume failure ④ even after stdout EOF
///   `Up` remains and is read as success.) A value, once **delivered**, is not withdrawn, so that gap does not exist at all.
/// ★The incarnation marker is **stamped by the assembly point**★ — this port is `Fn(LinkResolution)` and the path does not know the incarnation
///   (the same shape and reason as [`SessionIdSink`]). The consumer looks at that marker **only as match/mismatch**
///   (ADR-0163/0164 — deriving 「더 새 것」 (the newer one) by magnitude brings ① back).
/// ★★The contract is **at most once** (zero-or-once) — not 「정확히 한 번」 (exactly once)★★. There are two outcomes:
///   - **Once** — the moment the connection yields an outcome. It is not called after that.
///   - **Zero (零)** — the user turned this incarnation off. A handshake released by kill is not 「상대가 답했다」 (the peer answered), so
///     it is not delivered (if that branch were delivered, the user's cancel would be recorded as a resume failure — defect ③).
/// ★★So **the receiving side must handle 「영」 (zero) — this is the point of this paragraph**★★: this spot once
///   wrote 「정확히 한 번」 (exactly once) and 「껐으면 안 부른다」 (if turned off, not called) together three lines apart, and the consumer written on trust of the former was
///   the channel-gone branch of [`crate::manager::AgentManager::link_activation_verdict`] — when delivery is suppressed
///   the sending end drops, and it read that as 「통로가 결말을 못 냈다」 (the path could not produce an outcome) and **recorded the user's cancel as an activation failure.**
///   Now that branch looks at the termination-intent latch **first**.
/// ★What is suppressed is **only the message, not the channel-gone**★ — the one holding this port is a single writer thread, so when that
///   thread ends the sending end drops. That signal cannot be suppressed, so the observable form of 「영」 (zero) is
///   **channel-gone**. The receiving side must not read it as an outcome.
/// ★Delivery does not **drive** termination★ — the path still kills nothing and does not call `finish` either
///   (the line of ADR-0199). Stating an outcome and enforcing an outcome are different things.
pub type LinkSink = Arc<dyn Fn(LinkResolution) + Send + Sync>;

/// How the connection ended — ★only two values and both are **confirmed**★ (「아직」 (not yet) is not delivered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResolution {
    /// The peer **signaled that it is ready to receive input** = activation established (user decision).
    Ready,
    /// The connection could not be established. `reason` = a human-readable reason and the input to backend classification — this path's failure
    /// text is a JSON-RPC error on stdout so it is in neither the console tail nor the stderr diagnostic tail.
    Failed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ControlCaps, InputCaps, InputOrigin, OutputCaps};
    use std::sync::Mutex;

    /// A path that uses only the default implementation — records only `send_input`.
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
            "기본 구현은 출처와 무관하게 본문만 그대로 넘긴다 — id·출처는 바이트에 안 섞인다"
        );
    }

    #[test]
    fn the_default_withdraw_holds_nothing() {
        let t = RawOnly(Mutex::new(Vec::new()));
        assert_eq!(t.withdraw("id-1"), Withdraw::NotHeld);
        assert!(
            t.0.lock().unwrap().is_empty(),
            "거두기는 아무것도 쓰지 않는다"
        );
    }

    #[test]
    fn the_default_transport_holds_no_unconfirmed_input() {
        let t = RawOnly(Mutex::new(Vec::new()));
        assert!(t.unconfirmed_inputs().is_empty());
    }
}

/// A fake shared by the containment tests of the three pumps (PTY · stdio · codex reader) — a subscriber that panics on the first frame raises a panic contained inside the pump body,
/// and it records the marker at that time and the marker when `finish` announces the state.
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

    /// It records the marker and panics only on the first frame — later frames are only received.
    struct PanicAtFirstSend(Arc<Marks>, SinkId);

    impl OutputSink for PanicAtFirstSend {
        fn send(&self, _frame: OutputFrame<'_>) -> Result<(), SinkError> {
            if self.0.fired.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            self.0.at_send.lock().unwrap().push(is_contained());
            panic!("구독자가 emit 안에서 panic");
        }
        fn sink_id(&self) -> SinkId {
            self.1
        }
    }

    /// A core with the subscriber attached that announces the state through `marks`.
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
            "펌프 본체가 표시 안에서 돌지 않았다"
        );
        match marks.at_status.lock().unwrap().as_slice() {
            [(AgentStatus::Failed { message }, marked)] => {
                assert!(message.starts_with("pump panicked: "), "{message}");
                assert!(message.contains("구독자가 emit 안에서 panic"), "{message}");
                assert!(
                    !marked,
                    "finish 가 표시 안에서 돌았다 — 그 안의 panic 이 가둬지면 `Running` 이 남는다"
                );
            }
            other => panic!("가둔 panic 은 Failed 한 번이어야: {other:?}"),
        }
    }
}
