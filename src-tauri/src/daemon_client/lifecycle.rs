//! Connection lifecycle lock — the single source that blocks the TOCTOU of the generation guard (S14 module ① T2, ADR-0036).
//!
//! ## Why this file is separate (load-bearing — concurrency-critical)
//! The first implementation of the generation guard (openGen seed, Fix B) kept only `generation: AtomicU64` as an atomic,
//! and did "am I current" (load) and the shared-state change that hangs on it (watch `state_tx.send` / `cmd_tx`
//! store) as **two separate operations**. SeqCst guarantees only a global order of a single atomic and **cannot bind check +
//! change into one**. So:
//!
//! ```text
//!   stale task:  load() == my_gen  ── true (아직 current)
//!                ── 여기서 preempt ──
//!   다른 스레드:  close()/connect() 가 generation bump + state 갈아끼움
//!   stale task:  state_tx.send(...) ── stale 인데 current 의 상태를 clobber!
//! ```
//!
//! In tokio multi-thread, `close()` (synchronous, any thread) runs truly in parallel with the connection task (worker thread), so it is a
//! reachable TOCTOU. SeqCst alone cannot stop it (found by Codex blind, confirmed by main).
//!
//! ## The fix — atomize "check + change" under one lock
//! `generation` (demoted to plain u64) · `cmd_tx` (Option<Sender>) · the watch `state_tx` are **unified under a single
//! `Mutex<Lifecycle>`**. Every guarded transition passes through one method of this module.
//! Since comparison and change are inside the same critical section, no other thread can change the generation in between → no clobber is possible.
//! The socket marker that came later (ADR-0195) lives under the same lock for the same reason — the source of truth for that list is `LifecycleInner`
//! and it is not recounted here.
//!
//! ## ★ADR-0006 invariant — do not hold the lock across .await★
//! This lock's critical section contains **only pure synchronous code**. watch `send` · cmd_tx replacement · u64 compare/increment are
//! all synchronous so OK inside the lock. Awaits such as socket `sink.close().await` · `stream.next().await` · task `spawn` must
//! be called only after the lock is released (after the method has returned and the guard dropped). No method of this module does
//! `.await` internally (all are `&self` synchronous methods) — so a caller cannot hold the lock across an await.
//!
//! ## Instrumentation location (observability)
//! Methods of this module return the guard decision (bool) to the **caller**, and stale-discard · transition logs are
//! left by the caller (connection.rs run_connection / mod.rs start_connection·close) together with my_gen and context — this keeps
//! flat events (convention §format, no spans) and is meant to avoid double logging the same guard firing on both the lifecycle/caller
//! sides. So this file itself has no tracing calls.
//!
//! ## The possibility of introducing loom
//! Deterministic interleaving verification (loom) is the canonical tool for this class of TOCTOU defect. Currently ① deterministic unit tests
//! (tests.rs `guard_*`) verify the guard's *logical contract* (stale→reject, current→allow) and ② the integration wiring of the real socket race
//! is covered by single-shot deterministic regression tests. But a deterministic proof of the *atomicity* of compare+change
//! itself (that it truly does not break under concurrent threads) does not exist yet — that is on hold because of weight (introducing loom-only atomic/sync abstractions + keeping std concurrently)
//! (a low-ROI judgment:
//! docs/research/toctou-concurrency-test-verification-research-2026-06-28.md). If the lifecycle is abstracted with loom's
//! `loom::sync::Mutex` (cfg(loom) feature), the atomicity of this lock can be proven deterministically —
//! reconnect · backoff · in-flight cancel (T4) have joined and the concurrency surface has already grown, so it is highly worth revisiting.

use std::sync::Mutex;

use tokio::sync::{mpsc, watch};

use super::connection::ConnectionCommand;
use super::ConnectionState;

// The only access that happens outside the `inner` lock = the `state_rx.borrow()` fast read (`DaemonClient::state` —
// the watch's own synchronization).
pub(crate) struct Lifecycle {
    inner: Mutex<LifecycleInner>,
}

struct LifecycleInner {
    // plain u64 — comparison+increment are synchronous operations inside this lock, so no atomic is needed.
    generation: u64,
    // ★Single task ownership★: invoke only sends ConnectionCommand here, and processing is by the connection task alone (T6).
    cmd_tx: Option<mpsc::Sender<ConnectionCommand>>,
    // ★ADR-0195 — the marker of the live socket. `0` = no live socket (command endpoint closed)★. What this marker exists to
    // prevent is canonical in the doc of [`ConnectionCommand`].
    //
    // ★The point of this field is that it sits under the same lock as `cmd_tx`★: 「어느 채널인가」 (which channel) and 「어느 소켓인가」 (which socket) must be read together in one
    // critical section, so that an outgoing command can be stamped with the marker of the socket that was alive *then*.
    //
    // ★The `epoch` in the name is **not ADR-0163's agent incarnation marker**★ — the only thing telling the two apart is the `socket_`
    // prefix, and in `connection::main_loop` this value and the incarnation-side `known_epoch(subs, agent_id)` stand
    // side by side within a few lines. The axes differ: this one points to **one socket**, and outside the shell it is carried only in the
    // usage payload going to the webview (broadcast · cache pull — `super::usage_interest`) — it is neither on the daemon wire nor on disk
    // (TRD S21 usage-limit-slot §1-7 「소켓 표식」). So using `0` here as 「없음」 (none) has nothing to do with the
    // special treatment of `0` that ADR-0163 forbids on the daemon wire: this value is not a random number but a number the numbering counter below counts inside the shell process,
    // so `0` is never drawn, and the webview also reads `0` as 「소켓 없음」 (no socket). That the magnitude is the socket order
    // is also because that counter is never rewound, so it holds only within the shell process lifetime.
    socket_epoch: u64,
    // Marker numbering counter — **does not go back** even when the endpoint is closed. If the field above were folded to `0` and counted again from there, every socket of one
    // connection task would get the same marker (1), and a command carried over a reconnect would **end up matching** the new socket
    // (the very run ADR-0195 tries to prevent). `0` is a sentinel, so it is skipped on wraparound.
    socket_epoch_seq: u64,
    state_tx: watch::Sender<ConnectionState>,
    /// ★Reconnect cancel signal (T4 — fix for the in-flight cancel defect)★. On every generation bump (succeeding connect/ensure ·
    /// close), send the new generation value to wake the await of an in-progress reconnect task immediately —
    /// the aim is to close the window in which a stale task occupies · talks over the socket after close/succession (found by Codex).
    /// ★watch, not Notify★: (a) cancel-safe (if another arm of select! wins, changed() is discarded
    /// without side effects) (b) **preserves the last value** so that a late-subscribing reader can still read *the current generation value
    /// itself* via borrow (Notify has no value, so it cannot see "what is current").
    cancel_tx: watch::Sender<u64>,
    // ★closedByUser guard (T4 — counterpart of wsTransport `closedByUser`)★. If true, the reconnect loop stops immediately
    // (no reconnect on disconnect) — the task-lifetime edition of ADR-0021, under which commands/reconnects must not respawn the daemon
    // (stay off, recovery only by explicit connect).
    closed_by_user: bool,
}

// The guard decision of one tick of the reconnect loop (an atomic snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconnectVerdict {
    // I am current + the user did not close → proceed with the reconnect attempt/backoff.
    Proceed,
    // Stale (a newer connect/close raised the generation) or a user close → stop reconnecting (exit quietly).
    Stop,
}

impl Lifecycle {
    pub(crate) fn new() -> (Self, watch::Receiver<ConnectionState>) {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Down);
        let (cancel_tx, _cancel_rx) = watch::channel(0u64);
        (
            Self {
                inner: Mutex::new(LifecycleInner {
                    generation: 0,
                    cmd_tx: None,
                    socket_epoch: 0,
                    socket_epoch_seq: 0,
                    state_tx,
                    cancel_tx,
                    closed_by_user: false,
                }),
            },
            state_rx,
        )
    }

    // Raise the generation and return the new my_gen (connect/ensure entry). Optionally also emits the `set_state` transition
    // under the same lock — binding the entry's "bump + Connecting" into one critical section, so that even in the window where another thread
    // slips in right after the bump and raises the generation again, it acts consistently *with the generation I raised*.
    //
    // ★closedByUser release (T4)★: an explicit connect/ensure entry means the user intends to revive it, so under the same lock
    // it is reset to `closed_by_user=false` (isomorphic to `closedByUser=false` of wsTransport start()) —
    // so that a reconnect stopped by an earlier close can be revived.
    //
    // ★stale cmd_tx cleanup (T4 — Codex FIX)★: when a succession happens, the old connection's cmd_tx is no longer valid,
    // so it is **emptied to None inside the same lock**. Without this, until the new connect handshake finishes (and overwrites the new
    // cmd_tx with store_cmd_if_current), the old (stale) command channel stays alive, and an invoke that
    // came in during that window could send a command over the *dying old connection*. Dropping the Sender (old cmd_tx) here
    // makes the old connection task's cmd_rx EOF → main_loop ends with Closed (no reconnect) → the old socket is cleaned up.
    //
    // ★The command endpoint is also closed inside the same lock (ADR-0195)★: at the moment of succession there is no live socket. The old connection
    // task can close the endpoint only with its own generation (`close_socket_if_current`) and after being succeeded it has no
    // such authority, so if it is not closed here, the endpoint stays open until the new connection stands.
    pub(crate) fn bump_and_capture(&self, set_state: Option<ConnectionState>) -> u64 {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        g.generation += 1;
        let my_gen = g.generation;
        g.closed_by_user = false;
        g.cmd_tx = None;
        g.socket_epoch = 0;
        let _ = g.cancel_tx.send(g.generation);
        if let Some(state) = set_state {
            let _ = g.state_tx.send(state);
        }
        my_gen
    }

    // ★Guarded transition★: publishes the watch state only when `generation == my_gen`. If stale (a superseded generation),
    // it does nothing so as not to clobber the current connection's state. Return true = I am current so I published.
    pub(crate) fn publish_if_current(&self, my_gen: u64, state: ConnectionState) -> bool {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen {
            let _ = g.state_tx.send(state);
            true
        } else {
            false
        }
    }

    // ★A sibling that only decides — it does not publish★: it only checks whether `generation == my_gen` and returns.
    //
    // ★Why it must not be replaced by `publish_if_current`★: when a place that **has already published** the transition (the Down of `close_locked`)
    // rechecks only the generation just before firing outside the lock, replacing it with that would put the same shutdown on the watch **twice**.
    // tokio `watch` raises the version even when the value is the same (value comparison is the job of `send_if_modified`), so a subscriber that was awake
    // between the two sends sees one shutdown as two `changed()`. The question left at this place is only 「발화할까」 (fire or not), not 「발행할까」 (publish or not),
    // so an operation that answers only that question is kept separately.
    //
    // ★Invariant★: the lock is taken and released within this call — the firing after return is an external call outside the lock (ADR-0006).
    // The comparison is match/mismatch only (ADR-0163 — "newer" is not derived from magnitude).
    pub(crate) fn is_current(&self, my_gen: u64) -> bool {
        self.inner.lock().expect("lifecycle poisoned").generation == my_gen
    }

    // ★Guarded cmd_tx store★: stores the sender only when current. If stale it does not store and returns false
    // → the caller must drop the sender (the connection task's cmd_rx EOF → cleanup).
    pub(crate) fn store_cmd_if_current(
        &self,
        my_gen: u64,
        tx: mpsc::Sender<ConnectionCommand>,
    ) -> bool {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen {
            g.cmd_tx = Some(tx);
            true
        } else {
            false
        }
    }

    // Explicit shutdown (close). The generation bump · cmd_tx clearing · closed_by_user · Down publication are inside one lock, so a stale task cannot slip in between the bump and Down
    // and publish Connected (even if it slips in, its publish_if_current sees the already
    // raised generation and swallows it). This Down is close's own intent, so it is always valid without a guard.
    //
    // ★closed_by_user=true (T4)★: an in-progress reconnect task sees Stop at the next `reconnect_guard()` and
    // stops immediately (no reconnect on disconnect — isomorphic to `closedByUser=true` of wsTransport `close()`). Staleness caused by the bump alone
    // might not block a path like "disconnect→reconnect loop re-enters with a new my_gen", so an intent
    // flag is kept together to identify an explicit shutdown permanently.
    //
    // ## ★Return value = the generation this close set up★
    // This module **does not hold** the frontend firing port — that call would become an external call while holding the lock and break ADR-0006
    // (for the same reason as the header 「락을 .await across 보유 금지」, inside this lock only our code runs).
    // So the caller emits the firing **after** this function returns (= after the guard is released). But unlike the watch transition above, that firing
    // needs a guard: if a succeeding connect raises the generation between the return and the firing, the owner of the screen
    // is the newer generation, and a late `down` would then overwrite that owner's `connected`. The key of that recheck
    // ([`Self::is_current`]) is this return value.
    //
    // ★The recheck is a **decision**, not a publication★ — `Down` was already put on the watch inside the one lock above. Using
    // `publish_if_current` there would put one close on the watch as two entries (the source of truth for that reason =
    // [`Self::is_current`]).
    pub(crate) fn close(&self) -> u64 {
        Self::close_locked(&mut self.inner.lock().expect("lifecycle poisoned"))
    }

    // For `DaemonClient`'s `Drop` only — ★**this function** never panics★.
    //
    // A panic inside `Drop` aborts the process if it happens while unwinding. This lock's critical section is pure synchronous
    // code only (header above), so poisoning is practically impossible, but we do not stake an abort on "practically" — a poisoned
    // lock is quietly given up. In that case another panic is already in progress, and the connection task is reaped by runtime shutdown.
    //
    // ★That guarantee covers only **half** of the `Drop` path★ — the other half is **the field drops that follow after
    // this call ends**, and they include the dedicated tokio runtime (`DaemonClient::_owned_rt`). Dropping a runtime inside an async
    // context makes tokio panic, and this function **cannot prevent** that (it is not the place to). That
    // half is blocked by the `Drop` body of `DaemonClient` taking the runtime out directly and folding it away with `shutdown_background` —
    // the source of truth for the explanation is the comment at that place. Do not read "the Drop path does not panic" here as a whole.
    //
    // ★This path **does not return** a generation (the point where it differs from explicit `close`)★ — if the lock is poisoned, the transition itself
    // does not happen, so there is no generation to return, and returning one that does not exist as if it did would make the caller announce a transition that *did not happen* on screen.
    // The outcome of that fork (the Drop path does not fire) is owned by [`super::DaemonClient::close_on_drop`]
    // — it is not written again here.
    pub(crate) fn close_best_effort(&self) {
        if let Ok(mut g) = self.inner.lock() {
            Self::close_locked(&mut g);
        }
    }

    // The body of close — the two entry points (explicit close / Drop) are kept in one place so they do **the same transition**. If written separately,
    // a mismatch quietly arises where only one side fires cancel or only one side sets closed_by_user.
    // Return = the generation this transition set up (the key of the firing guard outside the lock — the `close` comment).
    fn close_locked(g: &mut LifecycleInner) -> u64 {
        g.generation += 1;
        g.cmd_tx = None;
        // ADR-0195: at shutdown there is no live socket — the endpoint is closed inside the same transition.
        g.socket_epoch = 0;
        g.closed_by_user = true;
        // ★Reconnect cancel send (T4)★: with bump + closed_by_user alone, if the reconnect task is *awaiting*, that await (e.g. connect_async) can complete
        //   and open a socket before reaching the next reconnect_guard synchronous check.
        let _ = g.cancel_tx.send(g.generation);
        let _ = g.state_tx.send(ConnectionState::Down);
        g.generation
    }

    // ★Reconnect loop 1-tick guard (T4)★: the reconnect task calls it before every backoff/attempt. It reads `generation == my_gen`
    // and `!closed_by_user` **together under one lock** — if the two are queried separately, a close()/new
    // connect slips in between and a TOCTOU opens where a stale task forces a reconnect on "both old snapshots".
    pub(crate) fn reconnect_guard(&self, my_gen: u64) -> ReconnectVerdict {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen && !g.closed_by_user {
            ReconnectVerdict::Proceed
        } else {
            ReconnectVerdict::Stop
        }
    }

    /// ★Subscription timing is load-bearing★: a tokio `watch::Receiver` sees **only sends after subscription** via `changed()`
    /// (on subscribe it marks the current value as "seen") — a send *before* subscribing cannot be recovered. So the reconnect task must
    /// subscribe **immediately after connected (= the moment my_gen is confirmed as current)**.
    pub(crate) fn cancel_subscribe(&self) -> watch::Receiver<u64> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cancel_tx
            .subscribe()
    }

    // ★Open the new socket's command endpoint (ADR-0195) — only when current★. Return = this socket's marker (always non-zero).
    // If stale it is `None` and nothing changes (a superseded task cannot replace the current one's endpoint).
    //
    // ★Bind the `Connected` publication into the same critical section★: if split, a succession can slip in **at this spot**
    // and make 「화면은 connected 인데 창구는 닫힘」 (the screen says connected but the endpoint is closed).
    //
    // ★It does not eliminate that mismatch itself — it closes this one spot only (a known remainder)★. The other two stay as they were:
    //   (a) At the disconnect edge `connection::connected_lifetime` closes the endpoint first, but the watch reads `Connected`
    //       until `Reconnecting` goes out afterwards (that order is intended — the comment at that spot).
    //   (b) The first connection opens the endpoint here and emits `Connected`, but the caller stores `cmd_tx` after `ready_rx.await`
    //       (`super::DaemonClient::start_connection`) — in between, even though the endpoint is open,
    //       `current_cmd_tx` is `None`. (b) is a shape that existed before this change.
    // ★So `Connected` must not be read as 「지금 명령이 나간다」 (a command goes out now)★ — both `ensure()` and the frontend's
    // `ensureReady()` short-circuit on that value, so both report success in those two windows. The
    // only answer to whether a command goes out is [`Self::current_cmd_tx`].
    pub(crate) fn open_socket_if_current(&self, my_gen: u64) -> Option<u64> {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation != my_gen {
            return None;
        }
        g.socket_epoch_seq = match g.socket_epoch_seq.wrapping_add(1) {
            0 => 1,
            next => next,
        };
        g.socket_epoch = g.socket_epoch_seq;
        let _ = g.state_tx.send(ConnectionState::Connected);
        Some(g.socket_epoch)
    }

    // ★Close the command endpoint (ADR-0195) — only when current★. While closed, `current_cmd_tx` gives `None`, and the
    // caller gets 「연결 안 됨」 (not connected) on the spot (it does not queue while disconnected).
    //
    // ★Why the guard exists★: a superseded connection task also passes this spot when its own socket ends. Without the guard, that
    // end would close the endpoint of **the new connection already standing**.
    pub(crate) fn close_socket_if_current(&self, my_gen: u64) {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen {
            g.socket_epoch = 0;
        }
    }

    // ★The cmd_tx handle of the currently active connection + that socket's marker (T6a — the send_command entry point)★. Clones the stored cmd_tx
    // and returns it with the marker. `mpsc::Sender::clone` is synchronous and light so OK inside the lock — the caller does
    // `send().await` **outside the lock** with the returned Sender (the Sender is independent of the lifecycle lock).
    //
    // ★`None` = there is no socket to receive a command now★ — no connection task · first connection in progress · disconnected and reconnecting ·
    // reconnects exhausted · shut down. The front door of ADR-0195 is this `None`: a command that comes in while disconnected is not queued
    // but sent back immediately here. The ones that open and close the endpoint are [`Self::open_socket_if_current`] ·
    // [`Self::close_socket_if_current`], and succession ([`Self::bump_and_capture`]) and shutdown close it too.
    //
    // ★Even so this return goes stale outside the lock — so the marker is given along with it★. The front door handles *immediacy* and the invariant
    // itself is handled by the receiving side's marker check (the source of truth for the reason = `connection::reject_foreign_command`).
    pub(crate) fn current_cmd_tx(&self) -> Option<(mpsc::Sender<ConnectionCommand>, u64)> {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.socket_epoch == 0 {
            return None;
        }
        g.cmd_tx.clone().map(|tx| (tx, g.socket_epoch))
    }

    #[cfg(test)]
    pub(crate) fn is_closed_by_user(&self) -> bool {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .closed_by_user
    }

    #[cfg(test)]
    pub(crate) fn current_generation(&self) -> u64 {
        self.inner.lock().expect("lifecycle poisoned").generation
    }

    // Test only — an observation point for the *state invariant* of blocking the zombie sender. The returned bool of `store_cmd_if_current` alone cannot
    // prove that "a stale store *did not overwrite* the existing current sender" (cmd_tx is private, Sender has
    // no Eq) — a clone for `same_channel` comparison is exposed to tests only.
    #[cfg(test)]
    pub(crate) fn cmd_tx_snapshot(&self) -> Option<mpsc::Sender<ConnectionCommand>> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cmd_tx
            .clone()
    }
}
