//! # engram-dashboard-transport — reusable transport lib (ADR-0177)
//!
//! ★Owns★: frame boundaries · request/response correlation · role distinction · stream bookkeeping (generation · sequence · resume · duplicate removal ·
//! hole finding and re-request) · connection setup and the **opaque handshake** · bidirectional keepalive · write deadline · backoff + jitter reconnect · a
//! registry of N peers with one supervisor per peer · fan-out · bounded queue and backpressure signal · structured connection events (ADR-0182).
//!
//! ★Does not know the consumer's packet vocabulary★ — hidden behind the associated types of [`Wire`]. The envelope, pairing-number generation, address
//! parsing and version negotiation are outside this crate (ADR-0177 decisions 4·5).
//!
//! ```text
//! ① Types are generic, resources are dyn.
//!      generic = Wire (the consumer's packet vocabulary)
//!      dyn     = Dialer/Listener/Link (transport) · Clock (time)
//! ② The pure layer and the driving layer are split by file (gate 3 below is that wall).
//! ③ Peers are N from the start.
//! ④ The only thing going up is Incoming — packets and events ride the same flow, with no side channel.
//! ```
//!
//! ## Entry points
//!
//! [`Registry::new`] · [`Registry::add`] (brings a peer up; gives its [`Peer`] and [`Inbox`]) · [`Registry::attach`] (an accepted link, same shape, no
//! redial, leaves the registry when it ends; the consumer runs the accept loop) · [`Peer`] methods = operating surface · [`Inbox::recv`] = the only
//! receiving surface (one per handle) · [`Registry::close_all`] closes everything and waits. ★What is sent is bound to one connection★ — `request` ·
//! `notify` to the generation current at the call, `request_in` · `notify_in` to the caller's ([`Arrival::generation`]); nothing goes out if it is not
//! current (TRD 3 §3-0 ③, `peer.rs` header). [`machine`] · [`pending`] · [`stream`] are public on purpose (ADR-0012) — pure layers for observation or
//! a consumer's own harness; the supported surface is the entry points.
//!
//! ## Receiver-side invariants (TRD 3 §3-0 ①)
//!
//! A slow receiver is never discarded on; this crate stops reading that peer (backpressure).
//!
//! 1. ★Do not await [`Peer::request`] on the task that drains the [`Inbox`]★ — backpressure blocks the reply too, so it hangs until
//!    [`Policy::request_timeout`] (or the request's own `reply_deadline`).
//! 2. ★While held by backpressure, that peer's silence decision and the resume window's deadline stop★ (we blocked it); the window may close later than the
//!    held time, never earlier (`Supervisor::track_pause`). ★Exception★: a window deadline passing during a write can close it before a replay already in the
//!    read-ahead queue (「known limits」). Our ping still goes out.
//! 3. Events and messages ride one line per supervisor in produced order (`peer.rs` header); a peer's close follows what preceded it, and is read only after
//!    those leave the ordering area (and bytes held fall below the read share).
//! 4. The daemon write deadline ↔ shell backpressure interlock (TRD 3 R7) cannot be measured in-memory (no TCP buffer); a real-socket test covers it (TRD 3 §6-2 U4).
//! 5. ★Drain the inbox to the end (`None`) — [`Peer::state`] alone never sees the end★: an undrained inbox stops reading that peer, including its close. The end is
//!    the `None` after [`TransportEvent::Closed`]. ★A `None` without `Closed`, unless the receiver called [`Inbox::close`]★ means the supervisor ended without
//!    normal cleanup (cleanup passed its cap, or panic · runtime shutdown). ★If the receiver drops the inbox or calls [`Inbox::close`], the supervisor folds
//!    at once★ — even when idle, no redial; it may already be dialing or in the handshake (spawned immediately), and that attempt is discarded. ★Accepted
//!    outgoing work is discarded too★ (queued commands rejected, the frame being written dropped — counted, nowhere to report). So a receiver that wants to
//!    send and leave drains to `Closed` · `None` first and only then drops; the same for [`Peer::close_after_flush`] · [`Registry::close_all`].
//!
//! ## Isolation gates — each is a different axis and the list below is canonical
//!
//! **① Cap on direct workspace dependencies** (resolved dependency graph):
//! `cargo tree -p engram-dashboard-transport --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **exactly 1 line = itself**.
//! ★Do not replace this with grep on the manifest text, nor shrink the flags★ — rename · `[dependencies.<name>]` · `[build-dependencies]` · inactive target ·
//! `optional` all slip through. ★Two holes★ (members are identified by the `engram-dashboard` **name prefix** — ADR-0151 「the rename trap」): a member without
//! the prefix passes untouched; and if this crate drops the prefix, other crates' cap gates and messaging name regex can no longer see edges to it (list in
//! `ci.yml` — the messaging isolation step — and `docs/testing-strategy.md`; not copied here).
//!
//! **② Tauri import 0** (ADR-0003): `rg "^\s*use tauri" crates/engram-dashboard-transport/src/` → **0 lines**. ① does not cover third-party `tauri`.
//!
//! **③ Pure-layer isolation** (`pending.rs` · `stream.rs` · `machine.rs`):
//! `rg "^(?:[^/]|/[^/])*?\b(tokio|futures_util|futures|engram_dashboard_[a-z_]+|std\s*::\s*net)\s*::"`
//! `crates/engram-dashboard-transport/src/pending.rs crates/engram-dashboard-transport/src/stream.rs`
//! `crates/engram-dashboard-transport/src/machine.rs` → **0 lines**.
//! ★Anchoring on `use` lines is pierced by an inline full path★ (`tokio::spawn(…)`). The prefix `(?:[^/]|/[^/])*?` means 「comment marker not yet met」 (this
//! header's self-quotation is not a false positive; precedent = `replay_flight` purity gate, ADR-0175 decision 3). ★No compiler backstop★ — this regex is the
//! only wall. ★A missing file reads as a pass★ — after a rename check with `test -f`. ★Keep the `\s*` around `::`★ (catches `tokio :: spawn`; reason in
//! `.github/workflows/ci.yml`). Known limit: aliases (`use tokio as rt;`), final-segment imports (`use std::net;`) and multi-item groups are not seen
//! (`docs/todo/observed-only.md`).
//!
//! **④ Each of the three feature combinations compiles** (`-p`), decided by **success**, not line count:
//! `cargo test -p engram-dashboard-transport` (0 features) · `cargo test -p engram-dashboard-transport --all-features` (`test-support` · `ws`) ·
//! `cargo check -p engram-dashboard-transport --features ws` (the consumer's operating combination).
//! Code behind a cfg gate gives no error, so do not shrink this to one line. ★The third line measures whether the adapter compiles **without the test-only
//! harness**★ — `testing` is behind `any(test, feature = "test-support")`, so an adapter reference to `crate::testing` dies only there. It is `check`, not `test`;
//! ★do not read it as 「0 tests」★ — `cfg(test)` exposes the harness, so `--features ws` runs the unit tests (the zero is only the four real-socket suites).
//!
//! **⑤ The four real-socket suites are not empty** (`tests/`):
//! `cargo test -p engram-dashboard-transport --all-features --test <ws_dial|ws_reject|ws_write_deadline|ws_accept>`
//! `-- --list` → each has **at least 1 line ending in `: test`**. ★④ cannot see this axis★ — the four files sit behind the file-level
//! `#![cfg(all(feature = "ws", feature = "test-support"))]`; if that turns false they compile with 0 tests and succeed. The suite-name list has three copies
//! (this line · `transport gate 4c` in `ci.yml` · the transport ⑤ line of the qa binding) — change all three together. The minimum stays 1, not the actual count, so
//! only a completely empty suite or a vanished target is caught.
//!
//! **Consumer side — `test-support` stays out of release graphs.** Turn the feature on only in `[dev-dependencies]`; `.github/workflows/ci.yml` `shell gate 5`
//! holds the shell's release graph at 0 lines of it (the daemon line comes with TRD 3 §6-3 U4).
//!
//! ## Known limits (do not read as guarantees)
//!
//! Mechanics live at the defining item; each bullet states the limit and points there.
//! - ★**Late-reply misdelivery cannot be made impossible — and we do not try**★: a consumer type that **reuses** [`Wire::Tag`] can get a late reply matched to a
//!   new request once the retired-tag FIFO ring ([`Policy::retired_tags`], default 256) has forgotten that number. Forgetting is an intended trade (a
//!   no-forget design blocks legitimate reuse forever or grows unbounded); keeping the bounded ring is the user's decision — the real `Tag` is a UUID. ★Do not
//!   "fix" it★ — see [`Pending`].
//! - ★**Overlapping live numbers fail both sides**★ — the old waiter gets [`RequestError::Superseded`], the new request [`RequestError::TagRetired`] (the wire
//!   cannot tell a late reply from the later request's match). ★Differs from 「the new one takes over」 in TRD §4-1★.
//! - ★**A close code reaches the peer in few places**★ — RFC 6455 §7.4.1 reserves `1006`. Disconnects push to the end to carry the code (`Supervisor::drop_link`), but
//!   `Policy::write_deadline` cuts that wait on a congested socket. [`Peer::close_after_flush`] gives the close frame only the shorter of `write_deadline` and the
//!   remaining [`Policy::flush_timeout`]; a handshake rejection pushes only within the remaining handshake deadline (`handshake_once`). Delivery is never certain;
//!   [`LinkRead::Closed`] is the only channel carrying the code.
//! - ★**[`TransportEvent::Disconnected`] carries only [`DisconnectCause`]★ — neither the link's text nor the peer's close code.
//! - ★**No `tracing`**★ — the event stream ([`TransportEvent`]) is that place; with no event reader no trace is left.
//! - ★**Jitter is deterministic under a manual clock**★; its variance is observable only on a real socket.
//! - ★**`oldest_valid` of [`TransportEvent::StreamTruncated`] is observed, not told by the peer**★ — [`Wire`] has no entrance for it. Out-of-order fragments are
//!   **held** while a re-request awaits `after + 1`, and on window close (deadline · [`Policy::resume_buffer`] · byte cap · eviction · cleanup) it raises that number
//!   while releasing in order. Using the peer's value (`SubscribeAck.oldest_seq`) needs one more `Wire` field — ★a breaking contract change; needs the user's approval★.
//! - ★**While holding, delivery is delayed**★ — max delay = [`Policy::resume_timeout`] + backpressure time; max load = [`Policy::resume_buffer`] per stream · window
//!   cap per supervisor (see `budget.rs`). ★A new hole that would exceed the window cap opens no window★ — it reports [`TransportEvent::StreamTruncated`] at once and
//!   continues from that fragment; the cap is shared by all of a supervisor's streams.
//! - ★**A replay already in the read-ahead queue cannot reach a window whose deadline passed during a write — even without backpressure**★ (`next_turn` in `peer.rs`
//!   checks the timer before the frame). TODO(stream-replay-vs-timer): decide the priority of frame versus deadline.
//! - ★**Stream ledger cut at [`Policy::max_streams`]**★ — overflow forgets the least recently used ([`TransportEvent::StreamForgotten`], held fragments returned);
//!   ★a forgotten stream that reappears starts from scratch, so progress rewinds★ (preventing it would defeat the cap).
//! - ★**The reconnect budget has two axes**★ — 「cannot connect」 and 「connects but does not last」 (consecutive count failing [`Policy::live_dwell`]); either running out
//!   gives up; both use [`ReconnectPolicy::max_attempts`].
//! - ★**The upward line is separate per handle; full = stop reading that peer (no discard, no disconnect)**★ ([`Registry::add`] sets up one line per call;
//!   [`Policy::inbound_queue`]). A noisy peer cannot push out a quiet one (ADR-0180 decision 8). Bytes per supervisor are bounded by 16 MiB + one message (`budget.rs`);
//!   the one-message cap is the adapter's job (`ws::AcceptPolicy::max_message_bytes` · `ws::WsDialer::new`; TRD 3 §5 15). On our-side link discard, what the
//!   read-ahead queue already holds is delivered before the disconnect event, the rest unread is discarded uncounted (`Supervisor::salvage_read_ahead`); under
//!   backpressure the peer's close may be noticed first as a write failure — the cause is [`DisconnectCause::PeerClosed`] only if it was already read-ahead.
//! - ★**[`Peer::close`] has no completion signal**★ — to wait use the future [`Peer::close_after_flush`] returns. Re-raising a label overlaps harmlessly via the
//!   **generation floor**; the old inbox's `None` after [`TransportEvent::Closed`] signals the old supervisor's end, with no order between old and new inboxes (merging
//!   reader: TRD 3 §3-3). ★A supervisor in cleanup stays until it has loaded everything it holds★ (cap `CLOSE_DRAIN_LIMIT`, `peer.rs`), so an undrained old inbox piles
//!   them up — do not await [`Peer::close_after_flush`] · [`Registry::close_all`] on the task that drains it.
//! - ★**After the app closes by itself, the handle-side loss count drops out of the report — decided not to fix**★: a loss count after the app said it would close has no
//!   reader and a clean exit takes priority; [`TransportEvent::Dropped`] there carries only the supervisor-counted share. Reconnect · link drop are unaffected.
//! - ★**The harness settle family ([`testing::settle`] · [`testing::settle_until`]) is current_thread-only**★ (panics on violation); the other in-memory parts and
//!   [`PermitPool`] also run on multi_thread with real-time waits.
//! - ★**The only receiving-side permit is the upgrade permit**★ — [`Registry::attach`] holds it until the operating phase and also bears the auth-wait permit's share
//!   (TRD 3 §5 14); accepted peers are uncapped while operating. [`Accepted`] comes from `ws::WsListener` and in-memory `testing::MemoryListener`.
//!   The auth-wait and operating permits do not exist yet (3-3 U4).
//! - ★**An accepted peer's label is assumed not reused**★ — accepted peers stay out of the generation table, so a label never `add`ed and absent from the registry
//!   restarts at generation 1 on re-attach or [`Registry::add`] ([`Registry::attach_with_policy`] · [`Registry::add_with_policy`]); fine with a new label per
//!   connection (TRD 3 §3-1 ① `conn-<ConnId>`).
//! - ★**One adapter**★ — [`ws`] (feature `ws`); its limits are canonical in the `src/ws.rs` header 「known limits」 · 「link properties」, not copied here.
//!
//! ## Standalone verification (ADR-0012)
//!
//! `cargo test -p engram-dashboard-transport --all-features` needs no other crate or process. In-memory suites turn time by hand on [`testing::ManualClock`] and
//! wait for nothing (ADR-0177 decision 8). ★The four real-socket suites (`ws_dial` · `ws_reject` · `ws_write_deadline` · `ws_accept` in `tests/`) use real time and
//! real TCP★, behind `all(feature = "ws", feature = "test-support")`, ports requested as 0. `ws_dial` · `ws_write_deadline` measure what memory cannot (e.g.
//! whether the write deadline cuts in front of the kernel buffer); `ws_reject` re-measures that the close code and message cross the kernel (reason in its header);
//! `ws_accept` measures the accept policy on a real HTTP upgrade (TRD 3 §6-0 U7; header of `tests/ws_accept.rs`). Moving them in-memory removes what they measure.
//! `ws_dial` · `ws_write_deadline` are the two ADR-0177 decision 8 lists as needing a real socket; `ws_reject` has no decision record yet.
//! ★Do not add `-- --test-threads=4`★ — this crate spawns no child process.
// hangul-ok-begin
//! It is exactly the decision rule of `.claude/skill-bindings/qa.md` 「모듈별 명령」, the same situation as `command` · `protocol` · `messaging` · `net`.
// hangul-ok-end

// ADR-0177
// ADR-0180
// ADR-0181
// ADR-0182
// ADR-0183
// ADR-0310
mod budget;
pub mod clock;
pub mod event;
pub mod frame;
pub mod link;
pub mod machine;
mod mask;
pub mod peer;
pub mod pending;
pub mod policy;
pub mod registry;
pub mod stream;
pub mod wire;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;

#[cfg(feature = "ws")]
pub mod ws;

pub use clock::{with_deadline, Clock, Elapsed, SystemClock};
pub use event::{Arrival, Direction, Incoming, PeerId, TransportEvent};
pub use frame::{Close, CloseCode, Frame};
pub use link::{
    Accepted, Address, ConnPermit, DialFailure, Dialer, Handshake, HandshakeStep, LinkError,
    LinkInfo, LinkRead, LinkRx, LinkTx, Listener, PermitPool, RejectKind,
};
pub use machine::{
    Action, ConnectFailure, DisconnectCause, GaveUpReason, Generation, Input, Machine,
    MachineEvent, PeerState,
};
pub use mask::mask_reason;
pub use peer::{Peer, SendError};
pub use pending::{Insert, Pending, RequestError};
pub use policy::{DecodeFailurePolicy, Policy, QueueFullPolicy, ReconnectPolicy};
pub use registry::{Inbox, Registry};
pub use stream::{StreamMark, Streams, Verdict};
pub use wire::{DecodeClass, Wire};
