# ADR-0043: mount-replay = via the actor + deliverable gate + fresh branching of assign/register

- 상태: 확정 (2026-07-01, basis: S14 module ① fifth cross-family review BLOCK + re-verification + cdp measurement)
- 관련: ADR-0040 · ADR-0006 · ADR-0007 · ADR-0042 · crates/engram-dashboard-core/src/output_view_store.rs · src-tauri/src/daemon_client/connection.rs · src-tauri/src/output_channel.rs · step-log S14 · Amended by ADR-0046 (the clause on the deliverable gate · mirror cursor mechanism: discarded → replaced by the view's buffering phase + gen fence — the mount-replay principle itself is inherited as a full re-replay)

## 맥락
When a new window/slot starts viewing an agent, the buffer at that moment must be **replayed immediately** (acceptance criterion 5 — zero empty screens even for a quiet agent or a window waiting for reconnect; if we only wait for the on_frame fan-out, the screen is empty until the next output). Where and how is this mount-replay done? And the core invariant of the output plane, **"only the cursor of a slot whose delivery succeeded advances" (cursor advance ⟺ Channel delivery)**, must be kept at every entry point — since the core does not know the Tauri registry (whether a Channel is registered) (ADR-0012), src-tauri injects "the set of currently registered windows" (deliverable).

## 결정
Settled in three clauses.

1. **Via the actor (order serialization)** — mount-replay is not a separate path; it is processed **serially on the same single thread** as `on_frame` (live) in the select! arm (`ReplaySlots`) of the output plane connection actor. The replay snapshot is flushed outside the buffer lock (ADR-0006). → prevents order inversion between replay and live.

2. **Fresh branching of the assign / register triggers** — the two triggers are distinguished:
   - **Assign trigger** (layout delta, `fresh:false`) = `subscribe` — creates the cursor + replays **only when there is none** (untouched if it exists).
   - **Register trigger** (registering the window output Channel = webview mount/reload, `fresh:true`) = `resubscribe_slot` — even if a cursor exists, reset it to None and replay everything (a Channel swap = a viewer restart, so resuming from a stale position is forbidden).

3. **Deliverable gate — cursor advance ⟺ Channel delivery (uniform across the three paths)** — the cursor advance/snapshot is done only for a slot whose window Channel is currently registered in the registry (deliverable). **Membership (creating a cursor entry) is independent of deliverable; only advance/emit is gated.** It applies identically to **all three paths**: `on_frame`, mount-replay (`subscribe`/`resubscribe_slot`), and the `reconcile_slots` recovery. The reconcile recovery decision is based not on the presence of an entry but on the **cursor value (delivery progress)**: value None + deliverable = recover (full replay) / value Some = untouched / not deliverable = keep membership only.

## 거부한 대안
- **Flush mount-replay directly outside the actor** — order inversion with `on_frame` (live) is possible (replay arrives after live). Actor serialization guarantees the replay → live order.
- **Unconditional fresh reset on both triggers (the old implementation)** — in a normal mount (assign → register), the same buffer is fully flushed twice in a row, and no-duplication depends only on the frontend dedup (against the spirit of ADR-0037). With fresh branching, a normal mount = one full replay.
- **Apply the deliverable gate only to on_frame + reconcile decides by "entry exists = delivered"** — when assigned to an unregistered window only the cursor advances and the output is lost to a flush registry miss, yet reconcile misjudges "entry exists" as delivery complete and cannot recover → if even the register trigger is dropped by a full channel, a **permanent empty screen**. (The BLOCK that the fifth cross-family review found by convergence — the direct motive for this ADR.)
- **The min_render_seq (aggregating the most-lagging window) resubscribe model** — the model ADR-0040 discarded. No revival (axis A is the buffer's newest seq).

## 근거
The fifth adversarial review (opus doc-aware + Codex blind) **found the missing deliverable gate as a converged BLOCK** (cross-family) → a fix propagating the gate uniformly to mount/reconcile → the fresh adversary re-verification confirmed **BLOCK closed, zero new defects**. cdp measurement (TauriTransport runtime path): ① live delivery — after the renderer registers, the resize → redraw output arrives on the new plane ② full buffer replay — on re-registering the window output Channel, seq 0~2 (183 bytes) were fully redelivered end-to-end. Core regression tests: not-deliverable membership-only (zero advance) / value None + deliverable recovery / normal mount once / reconcile idempotent.

## 영향 / 불변식
- `output_view_store.rs`: `subscribe`/`resubscribe_slot`/`reconcile_slots` take `deliverable: &HashSet<S>` and gate only advance/emit (membership is independent). The reconcile recovery is based on the cursor value. The on_frame invariant header is the canon of "cursor advance ⟺ delivery".
- `connection.rs`: the `ReplaySlots` arm + `resubscribe_and_sweep` build deliverable from `registered_labels` **before the buffer lock** and inject it (ADR-0006: zero nesting of buffer lock ⊃ registry lock, the snapshot is flushed outside the lock).
- `output_channel.rs`: the pure helper `build_deliverable` (unifying the 3 call sites · blocking drift).
- **If violated:** order inversion (replay outside the actor) / duplicate replay (unconditional fresh) / permanent loss (missing gate · entry-based reconcile).
