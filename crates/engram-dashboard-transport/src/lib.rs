//! # engram-dashboard-transport — reusable transport lib (ADR-0177)
//!
//! ★What it owns★: frame boundaries · request/response correlation · role distinction · stream bookkeeping (generation · sequence · resume · duplicate
//! removal · **hole finding and re-requesting from that point**) · connection setup and the **opaque handshake** · **bidirectional keepalive** ·
//! **write deadline** · **backoff + jitter reconnect** · **a registry of N peers and a supervisor per peer** · fan-out · bounded queue and backpressure
//! signal · **structured emission of connection events** (ADR-0182).
//!
//! ★What it does not know = the consumer's packet vocabulary★ — all of it is hidden behind the associated types of [`Wire`]. The envelope, the generation of
//! pairing numbers, address parsing and version negotiation are all outside this crate (ADR-0177 decisions 4·5).
//!
//! ## Canonical summary
//!
//! ```text
//! ① 타입은 제네릭, 자원은 dyn.
//!      제네릭 = Wire(소비자의 패킷 어휘)
//!      dyn    = Dialer/Listener/Link(전송) · Clock(시간)
//! ② 순수 층과 구동 층을 파일로 가른다(아래 게이트 3 이 그 벽이다).
//! ③ 상대는 처음부터 N 개다.
//! ④ 위로 올라가는 것은 Incoming 하나뿐이다 — 패킷과 사건이 같은 흐름을 타고 옆 채널을 두지 않는다.
//! ```
//!
//! ## Entry points
//!
//! [`Registry::new`] sets up the registry · [`Registry::add`] brings a peer up and gives its [`Peer`] and inbox [`Inbox`] ·
//! [`Registry::attach`] puts an accepted link (the [`Accepted`] a [`Listener`] produced) on in the same shape (it does not redial, and
//! when it ends it comes off the registry — the consumer runs the accept loop) · [`Peer::request`]/[`Peer::notify`]/[`Peer::resume_stream`]/
//! [`Peer::reconnect_now`]/[`Peer::drop_link_in`]/[`Peer::close`]/[`Peer::close_after_flush`] are the operating surface · [`Inbox::recv`] is the only receiving
//! surface — one per handle, and only what that one supervisor produced arrives. When folding the registry, [`Registry::close_all`] empties and
//! closes everything and waits for the end.
//! ★What is sent is bound to one connection★ — `request` · `notify` are bound to the current generation at the moment of the call, [`Peer::request_in`] ·
//! [`Peer::notify_in`] to the generation the caller gives (the [`Arrival::generation`] of a received message), and nothing
//! goes out if that connection is not current (TRD 3 §3-0 ③ — `peer.rs` header).
//!
//! ★That [`machine`] · [`pending`] · [`stream`] are public is intended★ — being pure layers, a consumer can run them
//! as they are in its own harness (ADR-0012). The **supported surface** is the entry points above, and the pure layers are for observation.
//!
//! ## Receiver-side invariants (TRD 3 §3-0 ① — including what the consumer bears)
//!
//! If the receiver falls behind, this crate does not discard but stops that peer's reading (backpressure — the line item under 「알려진 한계」 (known limits) below).
//! These are the rules that follow from that choice.
//!
//! 1. ★Do not await [`Peer::request`] on the task that drains the [`Inbox`]★ — if backpressure stops that peer's reading,
//!    the reply cannot get in either, so that request hangs until its deadline ([`Policy::request_timeout`]) and ends in a timeout.
//!    Make the request on a separately spawned task.
//! 2. ★While held by backpressure, that peer's silence decision and the resume window's deadline stop★ — we are the ones who blocked it. The supervisor
//!    restarts the count, taking the moment it notices the release as the last receipt, and an open window closes later by the time it was held — a hold that started while the supervisor
//!    was waiting to write, or that started and ended in that time, is held time too. The window may close later than that (it noticed the release
//!    late · a hold from before the window opened also pushes it back — erring on the side where nothing is lost) but never earlier. ★One exception★ — regardless of holds,
//!    if the window deadline passes during a write and a replay entered the front queue in the meantime, the window may close before that replay
//!    (「알려진 한계」 (known limits) below). Our ping goes out during that time too.
//! 3. Events · messages ride one line per supervisor (that handle's inbox) in the order that supervisor produced them (the supervisor's ordering area —
//!    `peer.rs` header). If the peer sends and closes right away, the close comes after what preceded it — if the line is full, the close is read only after those
//!    leave the ordering area (not until the receiver has taken them all out), and if the bytes held are at or above the read share (the line item under 「알려진 한계」 (known limits)),
//!    only after they come down below it.
//! 4. The place where the daemon's write deadline and the shell's backpressure interlock (TRD 3 R7) cannot be measured with the in-memory harness — it has no TCP buffer.
//!    It is the job of a real-socket test (TRD 3 §6-2 U4).
//! 5. ★Drain the inbox to the end (`None`) — do not see the end only through [`Peer::state`]★ — if not drained, the supervisor
//!    does not read that peer and cannot even read the peer's close, and `state()` never sees the end. The end is the `None` after [`TransportEvent::Closed`].
//!    ★A `None` without `Closed` — unless the receiver itself called [`Inbox::close`] — means the supervisor
//!    ended without a normal cleanup★: cleanup went past its cap and discarded what was left, or the task ended abnormally (panic ·
//!    runtime shutdown). Do not read it only as a cap overrun. A `None` after calling `close` comes even while the supervisor is still running
//!    ([`Inbox`]). ★Conversely, if the receiver drops the inbox or calls [`Inbox::close`], the supervisor folds at once★ — even with nothing
//!    to put in (the peer is quiet) it does not wait and does not redial. If the supervisor has not begun dialing, it does not dial at all —
//!    the supervisor is spawned right away by [`Registry::add`] · [`Registry::attach`], so a wait between that call and the drop, or
//!    a multi-thread runtime, means it may already be dialing (or running the handshake, for an accepted peer). If it is in the middle of dialing · the handshake,
//!    the attempt is discarded as the close wins — if the handshake has just finished and that finish wins, it draws a generation,
//!    enters the operating phase, sends a close right away and folds (no harm). The supervisor watches for the inbox
//!    closing in the wait loop and at every write — the only thing it does not watch is the one close sent when discarding the link (at most
//!    [`Policy::write_deadline`] · `peer.rs` header). ★Outgoing work already accepted is discarded too★ — commands in the queue are rejected and
//!    the frame being written is discarded (counted but nowhere to report). So a receiver that wants to send and leave drains
//!    to `Closed` · `None` first and only then drops — when closing with [`Peer::close_after_flush`] · [`Registry::close_all`] too, it holds and drains to that end.
//!
//! ## Isolation gates — each is a different axis and the list below is canonical
//!
//! **① Cap on direct workspace dependencies** (scope: the resolved dependency graph):
//! `cargo tree -p engram-dashboard-transport --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **exactly 1 line = itself**.
//! ★Do not replace this with grep on the manifest text★ — rename · the `[dependencies.<이름>]` table form ·
//! `[build-dependencies]` · an inactive target · `optional` all slip through in normal Cargo syntax. Do not
//! shrink the flags either (same reason as the `command` · `base` · `messaging` cap gates).
//! ★Two holes remain★ — because members are identified by the `engram-dashboard` **name prefix** (ADR-0151 「개명 함정」 (the rename trap)).
//! (a) Depending on a workspace member without that prefix passes this gate untouched. (b) If this crate drops that
//! prefix, this gate turns red (`-p` dies or its own line loses the prefix), but the cap gates and messaging name
//! regex of **other** crates can no longer see edges going to this crate (the list is in CLAUDE.md
//! 「빌드·검증 명령」 (build and verification commands) — not copied here).
//!
//! **② Tauri import 0** (ADR-0003):
//! `rg "^\s*use tauri" crates/engram-dashboard-transport/src/` → **0 lines**.
//! ★Do not read ① as covering this axis★ — ① counts only workspace members, so the third-party `tauri`
//! passes as is. The two gates do not overlap.
//!
//! **③ Pure-layer isolation** (scope: `pending.rs` · `stream.rs` · `machine.rs`):
//! `rg "^(?:[^/]|/[^/])*?\b(tokio|futures_util|futures|engram_dashboard_[a-z_]+|std\s*::\s*net)\s*::"`
//! `crates/engram-dashboard-transport/src/pending.rs crates/engram-dashboard-transport/src/stream.rs`
//! `crates/engram-dashboard-transport/src/machine.rs` → **0 lines**.
//! ★Anchoring only on `use` lines is pierced by one inline full path★ — if the body uses a full path like `tokio::spawn(…)`,
//! no import appears. The prefix `(?:[^/]|/[^/])*?` means 「아직 `//` 를 안 만났다」 (have not yet met the comment marker) and so catches **only outside comments**,
//! which is why this header's self-quotation is not a false positive (precedent = the `replay_flight` purity gate, ADR-0175 decision 3).
//! ★This gate has no compiler backstop★ — being in the same crate, a single `tokio` line just compiles.
//! This regex is the **only wall**. ★If a file disappears the match is 0 and reads as a pass★ — after renaming or
//! moving, first check with `test -f` that the three files exist.
//! ★Do not strip the `\s*` around `::`★ — it catches the spaced form (`tokio :: spawn`) — the reason is in the comment of the `replay_flight` purity gate in
//! `.github/workflows/ci.yml`.
//! ★Known limit = names that are not in the `<이름>::` shape★ — aliases (`use tokio as rt;` · `extern crate tokio as rt;`), final-segment
//! imports (`use std::net;`) and multi-item groups (`use std::{net::TcpStream, time::Instant};`) are not seen
//! (`docs/todo/observed-only.md`).
//! ★The draft in TRD §9-2 aimed only at `machine.rs`★ — the file table marks all three ★pure★, so it was widened to three.
//!
//! **④ Each of the three feature combinations compiles** (scope: `-p`):
//! `cargo test -p engram-dashboard-transport` → **PASS only if it succeeds** (0 features)
//! `cargo test -p engram-dashboard-transport --all-features` → **PASS only if it succeeds** (`test-support` · `ws`)
//! `cargo check -p engram-dashboard-transport --features ws` → **PASS only if it succeeds** (the consumer's operating combination)
//! ★The way of deciding differs from the three above★ — ① · ③ read by line count and ② by whether there is a match, but this one reads by **success or failure**.
//! Code hidden behind a cfg gate produces no error, so do not shrink this to one line.
//! ★What the third line measures = whether the adapter compiles **without the test-only harness**★ — the `testing` module declaration is
//! behind `any(test, feature = "test-support")`, so this combination does not have it. So if the adapter references `crate::testing`,
//! it dies at this line while the two above stay green — the first line does not compile the adapter at all, and the second
//! compiles it with that feature on (measured: put that reference in the lib and the two above are green and only this line
//! dies with an unresolved import). Since all it measures is that compilation holds, it is `check`, not `test`.
//! ★Do not read it as 「이 조합에는 돌 테스트가 0개다」 (this combination has 0 tests to run) — that is false★ — `cfg(test)` exposes the harness too, so
//! `cargo test -p engram-dashboard-transport --features ws` runs the unit tests as usual. The zero is only
//! the four real-socket suites in `tests/`.
//!
//! **⑤ The four real-socket suites are not empty** (scope: `tests/`):
//! `cargo test -p engram-dashboard-transport --all-features --test <ws_dial|ws_reject|ws_write_deadline|ws_accept>`
//! `-- --list` → each has **at least 1 line ending in `: test`**.
//! ★④ cannot see this axis★ — the four files are behind the file-level `#![cfg(all(feature = "ws", feature = "test-support"))]`, so if that condition
//! goes false (a name typo · a rename · folding into another feature), the four binaries **compile with 0 tests
//! and succeed**, and since ④ decides by success, it passes as is.
//! ★The suite-name list has three copies★ — this line · `transport gate 4c` in `ci.yml` · the transport ⑤ line of the qa binding. If a suite is
//! added or removed, fix all three in the same change (a suite outside the list is not seen by this gate).
//! ★The expected value is a minimum of 1 and is not raised to the actual count★ — raising it would make two canonical sources of the count. So this gate
//! catches only a suite being **completely** empty or the target disappearing, and cannot catch 3 shrinking to 1.
//!
//! **Consumer side — `test-support` stays out of release graphs.** The harness ([`testing`]) is for consumers' tests
//! only: a consumer turns the feature on in its `[dev-dependencies]`, never in `[dependencies]`. Unlike ①–⑤ (which
//! watch this crate) this is checked on the consumer: `.github/workflows/ci.yml` `shell gate 5` holds the shell's release
//! graph at 0 lines of the feature, paired with ≥ 1 in its dev graph (the daemon line comes with TRD 3 §6-3 U4).
//!
//! ## Known limits (do not read as guarantees)
//!
//! - ★**It cannot make misdelivery of a late reply impossible — and does not try to**★. If the consumer
//!   picks a type that **reuses** [`Wire::Tag`], this crate cannot prevent it. Mechanism: numbers retired by
//!   deadline expiry go into a **FIFO bounded ring** ([`Policy::retired_tags`], default 256) and are rejected with
//!   [`RequestError::TagRetired`] if they come again. After 256 more retire, the oldest number is
//!   **forgotten**, and ★a late reply for that number arriving after that is delivered as the match of a **new request** that used the same number★
//!   (the correlation key is only that number, so the wire has no material to tell the two apart).
//!   ★**Forgetting is not a defect but an intended trade**★ — without forgetting, a **legitimate** reuse of that number is also
//!   blocked forever, and memory without a cap grows without bound. **Two reviewers split at this point, and leaving the bounded ring in its current
//!   shape is the user's decision** (2026-09-06) — our real `Tag` is a UUID so a legitimate collision does not
//!   happen, and the remaining hole needs **a consumer bug + a late reply by one ring's worth** at the same time.
//!   ★A later session must not misread this as a mistake and 「고치지」 (fix) it★ — details in the [`Pending`] rustdoc.
//! - ★**If live numbers overlap, both sides fail**★ — the old waiter gets [`RequestError::Superseded`] and
//!   the new request gets [`RequestError::TagRetired`]. Since the correlation key is that one number alone, **the wire has no means to tell a late
//!   reply to the earlier request from the later request's match**, and rather than misdeliver, it fails both.
//!   ★This differs from 「새 것이 승계」 (the new one takes over) in TRD §4-1★.
//! - ★**The place where a close code reaches the peer is narrow**★ — RFC 6455 §7.4.1 pins `1006` down as a reserved
//!   value that cannot be sent on the wire. **When we disconnect (exit · reconnect), we push to the end to carry the code**
//!   (canonical place = `Supervisor::drop_link`), but on a congested socket `Policy::write_deadline` cuts that wait
//!   and it disconnects without a code. ★When closing with [`Peer::close_after_flush`] it is narrower still★ — the close frame is given only the shorter of `write_deadline` and
//!   the remaining drain deadline ([`Policy::flush_timeout`]), and if earlier writes used up that deadline it is 0, so it just tries one
//!   push — a slow-reading peer does not receive the close code it was given. So **there is no place where the code is certain to be delivered** —
//!   a handshake rejection also pushes only within the remaining handshake deadline, and if the last frame before the rejection could not get out, the close
//!   is skipped entirely (canonical place = `handshake_once`). [`LinkRead::Closed`] is the only channel that carries that code.
//! - ★**A disconnect while operating does not carry the reason **text****★ — all [`TransportEvent::Disconnected`] carries is
//!   [`DisconnectCause`], so neither the text the link gave nor the close code the peer sent has anywhere to go.
//! - ★**There is no `tracing`**★ — the event stream ([`TransportEvent`]) is that place. **While the consumer does not read events,
//!   no trace is left at all.** The consumer lays down the bridge.
//! - ★**Jitter is deterministic under a manual clock**★ — the random source is the clock's low-order nanoseconds + a per-peer seed, so it is the property the harness
//!   wants, but in exchange the *variance* of jitter can be observed only on a real socket.
//! - ★**`oldest_valid` of [`TransportEvent::StreamTruncated`] is an observed value, not a value the peer told us**★ —
//!   the six fields of [`Wire`] have no entrance to carry 「지금 유효한 가장 오래된 번호」 (the oldest number valid now). After issuing a re-request,
//!   while waiting for its answer (fragment `after + 1`), it **holds** out-of-order fragments that arrive, and when the window closes
//!   (deadline exceeded · [`Policy::resume_buffer`] saturated · held-bytes cap · stream eviction · supervisor cleanup) it raises that number **while releasing
//!   in order from its first number**. So the value is true — it really delivers from that number. Progress is pushed to
//!   the last number held, so a late replay does not raise a number already released again. To use the peer's authoritative value (`SubscribeAck.oldest_seq`
//!   in our wire), `Wire` would need one more field, and ★that is a contract change that breaks
//!   every consumer, so it needs the user's approval★.
//! - ★**While holding, delivery is delayed**★ — to keep order, fragments of that stream are not raised until the window closes.
//!   Max delay = [`Policy::resume_timeout`] + the time held by backpressure in the meantime (it is delayed further by as much as the receiver is behind —
//!   「받는 쪽 불변식」 (receiver-side invariants) 2 above), max load = [`Policy::resume_buffer`] per stream · window cap 8 MiB per supervisor (half the byte
//!   budget of a stream-using supervisor — the line item below · beyond that the window closes as truncated). ★If the fragment that would exceed the window cap is a new hole, the window is not opened at all★ — it reports
//!   truncation ([`TransportEvent::StreamTruncated`]) at once without a re-request and continues from that fragment. ★The window cap is shared by all
//!   of a supervisor's streams★ — if one stream's open window is filling the cap, another stream's new hole becomes
//!   truncation at once, with no chance to fill it.
//! - ★**If the window deadline passes during a write and a replay entered the front queue in the meantime, that replay cannot reach the window — even
//!   without backpressure**★ — when the deadline has passed, the operating loop looks at the timer before the frame (`next_turn` in `peer.rs`). The first
//!   turn after the write ends sees the window deadline passed, closes the window as truncated, and the replay processed after that is discarded as a past number.
//!   The write does not need to be longer than the window deadline — one write that crosses the open window's deadline is enough. Time held by backpressure
//!   is subtracted separately from this (「받는 쪽 불변식」 (receiver-side invariants) 2 above).
//!   TODO(stream-replay-vs-timer): decide the priority of frame versus deadline in that case.
//! - ★**The stream ledger is cut at [`Policy::max_streams`]**★ — on overflow it forgets starting from the **least recently used** and
//!   reports with [`TransportEvent::StreamForgotten`] (fragments being held are returned with it).
//!   ★If a forgotten stream appears again later, the ledger starts from scratch, so **progress rewinds**★ — preventing that
//!   would require remembering what was forgotten, and then the reason for having a cap disappears.
//! - ★**The reconnect budget has two axes**★ — 「못 붙는다」 (cannot connect; a successful connection resets it to 0) and 「붙어도 안
//!   버틴다」 (connects but does not last; the consecutive count that failed to fill [`Policy::live_dwell`]). Whichever runs out, it gives up, and both
//!   use [`ReconnectPolicy::max_attempts`] as the cap.
//! - ★**The upward line is separate per handle, and when it fills it does not discard — instead it stops that peer's reading**★ —
//!   every time [`Registry::add`] brings one up it sets up one line and returns its receiving end ([`Inbox`]) (bringing up the same label again
//!   is a new line too). So a noisy peer cannot push out a quiet one (ADR-0180 decision 8), and leaving one inbox
//!   undrained stalls only that peer. When the line ([`Policy::inbound_queue`] — per-peer override applied) fills, that peer's supervisor holds the items in its own
//!   ordering area and loads them in the order produced as room opens (`peer.rs` header) — neither packets nor lifecycle events are
//!   discarded, and a full line does not cut the connection. While the ordering area holds, the supervisor takes no more of that peer's frames, so
//!   when the front queue ([`Policy::frame_buffer`]) fills, link reading stops, and it also stops reading the link when the bytes held are at or above the read share.
//!   For bytes, one tally per supervisor (budget 16 MiB) covers the front queue · ordering area · line · resume window together. A supervisor that
//!   has received a stream fragment even once splits that budget in half into read share and window cap (8 MiB · 8 MiB), and a supervisor that has not (the two consumers not
//!   using the stream layer — ADR-0310 decision 16) has the whole 16 MiB as its read share (window cap 0 · `budget.rs`). So when the receiver
//!   falls behind, only that peer
//!   falls behind and what is behind stays on the link side. ★Stream fragments the window holds are removed from the read share★ — if those fragments stopped reading,
//!   the replay that would fill the hole could not be read and a hole that could have been filled would become a loss. The ceiling of bytes one supervisor holds is 16 MiB +
//!   one message (TRD 3 §3-0 ① 「B + M」 — the canonical source of the reason = the `budget.rs` header). The count ceiling is the two lengths above + the ordering area (what one
//!   frame or one turn of the timer yields — including the fragments held by a window that closes and releases on that turn — and a few lifecycle events,
//!   and what was salvaged from the front queue when the link was discarded) + the fragments the window holds (fewer than [`Policy::resume_buffer`] per stream).
//!   ★The cap for one message is the adapter's job★ — the `ws` adapter applies the value the consumer gave (`ws::AcceptPolicy::max_message_bytes` ·
//!   `ws::WsDialer::new`) to the receiving tungstenite settings (TRD 3 §5 15). So the ceiling over that adapter is 16 MiB + that
//!   value (32 MiB at the operating value of 16 MiB).
//!   ★When our side discards the link (the app closed it · redial · write failure and so on), what was already read (the front queue) is handed over
//!   before the disconnect event★ — it folds the read task and drains that line without waiting and sends it down the same path as while operating (replies to
//!   the waiting callers · everything else onto the line with the current generation marker · even past one frame that could not be decoded). What is left on the link (not yet
//!   read) is discarded together and neither counted nor reported.
//!   ★Under backpressure, the supervisor may first notice through a write failure that the peer sent its last words and closed★ — if that close is queued
//!   behind the backed-up items and unread, and the ping write fails first, only what was already read into the front queue survives via the drain above.
//!   Frames left on the link are discarded together (the last words too, if they were still there). The disconnect cause is the peer's close
//!   ([`DisconnectCause::PeerClosed`] — it came first) if that close was already in the front queue, and the write-side end
//!   ([`DisconnectCause::WriteFailed`] · [`DisconnectCause::WriteDeadline`] if it was the deadline) if it was still on the link.
//! - ★**[`Peer::close`] has no completion signal**★ — at the time it returns, the supervisor may still be closing the link. To
//!   wait for the end, use the future [`Peer::close_after_flush`] returns (it drains and closes).
//!   When the same label is brought up again, that overlap is harmless thanks to the **generation floor**, not to ordering.
//!   On the reading side, the end of that handle's inbox (the `None` after [`TransportEvent::Closed`]) signals the supervisor's end. The old
//!   supervisor puts only into the old inbox so it does not mix with the next supervisor of the same label, but there is no order between the two inboxes —
//!   the old end may be read later than the new supervisor's events (the rule the merging reader filters by = TRD 3 §3-3).
//!   ★A supervisor in cleanup stays until it has loaded everything it holds★ — if the receiver holds the old inbox without reading, such supervisors
//!   pile up, and each ends only after its own cap (`CLOSE_DRAIN_LIMIT` in `peer.rs`) passes — what is left then is discarded neither
//!   counted nor reported. If the old inbox is dropped, that cleanup ends without waiting. ★So the supervisor's end depends on the receiver
//!   draining or dropping that inbox★ — do not await what waits for that end (the future of [`Peer::close_after_flush`] · [`Registry::close_all`]) on
//!   the task that drains that inbox: the one who would drain is standing in that wait, so it hangs up to the cap (for `close_all`, up to its
//!   deadline).
//! - ★**When the app closes by itself, the handle-side loss count drops out of the report — we decided not to fix this**★ —
//!   the [`TransportEvent::Dropped`] report in the cleanup stage after [`Peer::close`] carries only the share the supervisor counted (it goes through the ordering area and
//!   is loaded before `Closed` — if the receiver does not drain it within the cap, it disappears with them), and the share counted on the handle side (what could not be
//!   put in because the queue was full · what was rejected while disconnected) is not even summed at that stage. It does not apply to a reconnect · link drop — the next
//!   turn sums it. **A loss count after the app said it would close has no reader and a clean exit takes priority** (user
//!   decision 2026-09-26).
//! - ★**Only the harness's settle family ([`testing::settle`] · [`testing::settle_until`]) is current_thread-runtime only**★
//!   — it synchronizes only by cooperative yielding and catches violations of that by panicking on its own. The in-memory parts (`MemoryNetwork` · `memory_link` ·
//!   `ManualClock` …) and [`PermitPool`] are also used on multi_thread, with waits that carry real-time deadlines (the
//!   3-0 U6b test in `registry.rs`).
//! - ★**The only permit on the receiving side is the upgrade permit**★ — [`Registry::attach`] holds the accepted link's upgrade permit until it enters the operating phase,
//!   so it bears the auth-wait permit's share as well (TRD 3 §5 14). The auth-wait permit · the operating permit do not exist yet (3-3 U4) — so there is no cap on the number of accepted
//!   peers while operating. The listeners that produce [`Accepted`] are two: the real-socket `ws::WsListener` and the in-memory
//!   `testing::MemoryListener`.
//! - ★**It assumes the label of an accepted peer is not reused**★ — an accepted peer does not enter the generation table (so it does not grow with the number of connections), so
//!   for a label that was never brought up by `add` and is not in the registry now, the operating generation restarts from 1 — both when putting an accepted label that
//!   came off the registry back on and when [`Registry::add`]ing that label ([`Registry::attach_with_policy`] ·
//!   [`Registry::add_with_policy`]). It does not arise if each connection uses a new label (TRD 3 §3-1 ① `conn-<ConnId>` — ConnId is monotonic).
//! - ★**There is only one adapter and its limits are canonical in that file's header**★ — [`ws`] (feature `ws`) is the only
//!   real transport implementation, and the canonical source is the two sections 「알려진 한계」 (known limits) · 「통로 성질」 (link properties) in the `src/ws.rs` header (**unverified** items are
//!   in there too). ★Do not copy any part of that list here★ — a hand-moved list goes stale, and if only four are written
//!   the next session reads those four as all of them.
//!
//! ## Standalone verification (ADR-0012)
//!
//! `cargo test -p engram-dashboard-transport --all-features` runs with this crate alone — it calls no other crate and
//! no external process. The in-memory suites turn time by hand on [`testing::ManualClock`] and
//! **wait for nothing** (ADR-0177 decision 8). ★But the four real-socket suites masked by the feature run on **real time and
//! real TCP sockets** and really wait★.
//!
//! Those four are `ws_dial` · `ws_reject` · `ws_write_deadline` · `ws_accept` in `tests/`, all
//! behind `all(feature = "ws", feature = "test-support")`. Ports are **requested as 0 and read back**.
//! ★The four do not exist for the same reason★:
//! - `ws_dial` · `ws_write_deadline` = **what the in-memory harness cannot measure**. Among them, **whether the write deadline
//!   really cuts in front of the kernel buffer** cannot in principle be built with a harness. These are the two that ADR-0177 decision 8 listed as needing a real socket.
//! - `ws_reject` = **deliberately re-measuring a decision the harness already makes**. The harness's rejection hands over the close **as a value**,
//!   so whether the close **code and message cross the kernel** cannot be measured by value passing — it is a property of the adapter. The canonical reason is
//!   the header of `tests/ws_reject.rs`, and ★this suite is outside the list above so there is no decision record yet★.
//! - `ws_accept` = whether the receiving-side accept policy holds on **a real HTTP upgrade and real TCP** — whether the 403 reaches the peer · whether the deadline really
//!   closes the TCP · whether the size cap went into the tungstenite settings (both directions) · whether the permit count is right in the real accept order · whether the accepted link
//!   goes onto the registry and requests flow both ways. The header decision table itself is measured by the `ws.rs` unit tests without a socket. The requirement to measure it on a real socket
//!   is the TRD 3 §6-0 U7 row (canonical reason = the header of `tests/ws_accept.rs`).
//! ★Do not read it as 「인메모리 하네스로 옮기면 빨라진다」 (moving them to the in-memory harness makes it faster)★ — the moment you move them, what they measured disappears.
//!
//! ★Do not add `-- --test-threads=4`★ — this crate spawns no child process at all.
//! It is exactly the decision rule of `.claude/skill-bindings/qa.md` 「모듈별 명령」 (per-module commands), the same situation as `command` · `protocol` · `messaging` · `net`.

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
