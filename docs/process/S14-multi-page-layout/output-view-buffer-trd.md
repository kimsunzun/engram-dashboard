# TRD — S14 module ① output-plane redesign: output unit = independent per View

**Status:** review reflected (`/review trd full` FIX reflected) — awaiting coder (concurrency-critical → `/review code deep` gate)
**Written:** 2026-06-30
**Basis:** ADR-0040 (output unit = independent per View) · PRD `output-view-buffer-prd.md` · ADR-0036/0037 (transport relay · semantics Rust alone) · ADR-0006 (lock order) · ADR-0007 (epoch) · ADR-0001/0005 (kill · finalize daemon-owned) · ADR-0012 (seam)
**Investigation:** Explore output-plane precision map (2026-06-30).
**Review:** `/review trd full` = Codex (Designer doc-aware) + opus (Architect-breaker doc-aware) **both FIX, no BLOCK**. 5 FIXes reflected: ① separate the resubscribe/read two axes ② send outside the lock + a single lock ③ epoch tagging (not a lock) ④ clean up the buffer on terminal ⑤ common extraction = data structures only.

---

## Current structure (starting point — Explore collection)

- **Daemon ReplayBuffer** (`crates/engram-dashboard-core/src/agent/output_core.rs:415–461`): `VecDeque<OutputChunk{seq:u64, data:Vec<u8>}>` + `total_bytes`. Beyond the dual cap **`max_bytes=2MB` OR `max_events=4096`**, `pop_front` evicts (oldest_seq↑). `subscribe_from(after_seq)` (:316–394): epoch mismatch/None→`FromOldest`; `Some(s)`→buffer empty=`Resumed,0` / `s<oldest`=`Truncated` (from oldest) / `s>=oldest`=`Resumed` (tail). `subscribe_from` does on_ready→replay send while holding the subscribers lock (C4, :374–391) — entangled with finalize (ADR-0005).
- **src-tauri output flow** (`connection.rs:880–912`): `decode_frame→OutputFrame{agent_id,epoch,seq,data}`→`decide_epoch` (Deliver/DropEpochMismatch; passes if `st.epoch=None`)→`router.targets→Vec<WindowLabel>`→`fan_out_per_window` (per-window dedup + **holding only 1 lock (registry) during the synchronous send**, :1060–1110 — confirmed no ADR-0006 violation).
- **Reconnect** (`connection.rs:788–832`): `router.current_agents()`→`min_render_seq`→`resubscribe_params`→wire `Subscribe{epoch,after_seq}`. On a SubscribeAck epoch change `reset_all_windows_for_agent` (:862–870). subs.retain (:832) cleans up based on the router.
- **main_loop = a single actor** (select! task) — the Binary arm · Text (SubscribeAck) arm · cmd_rx arm are **serial**. (The basis of the epoch correction.)
- **Frontend** (`tauriTransport.ts`): `outputChannel.onmessage(bytes)`→decode→messageCb. **No dedup** — Rust is the sole source of truth (ADR-0037).

---

## New structure — shared buffer + per-view cursor (★two axes separated)

Create a **per-agent shared output buffer** in src-tauri. Switch from "frame → each window's Channel (per-window dedup)" to **"frame → shared-buffer append → each window reads from its own cursor → (outside the lock) Channel send"**. Insertion point = right after the epoch handling of the `main_loop` Binary arm, before the fan-out.

**★Core: separate the two axes (review FIX 1).**
- **Axis A — daemon↔client synchronization:** the daemon resubscribe `after_seq` = **the shared buffer's latest seq**. The daemon sends only what is not in the client buffer, which is appended continuing the buffer (no duplicates).
- **Axis B — per-window lossless read:** each window reads the shared buffer from `cursor` with its **per-view cursor**. If the cursor is within the buffer [oldest~latest], it reads that range losslessly.
- These two are **different axes** — do not merge them. (The draft merged them into a single "buffer latest", producing the contradiction of loss/duplication for unrendered windows — the convergent point raised by opus and Codex.)

```
[데몬]  단일 무손실 스트림 (에이전트당 1구독, after_seq=버퍼 최신)
   ▼ OutputFrame{seq, epoch, data(원본 binary frame bytes)}
[src-tauri 공유 버퍼]  AgentBufferStore
   · content:  HashMap<AgentId, BoundedSeqLog>   — 콘텐츠 에이전트당 공유 1벌 (core 순수)
   · cursors:  HashMap<SlotId, {agent_id, cursor}> — 보는 단위(slot) 1차 키 (core 순수)
   · epoch 태그
   ▼ [락 안] 그 agent 보는 slot 중 cursor<최신인 것의 (slot, bytes) snapshot → [락 밖] Channel send
[main/popup/tree창의 각 slot=xterm]  순수 렌더러 (raw → xterm)
```

### 1. Shared buffer model + lock discipline (decision points 1·3 — FIX 2)

- **Data structure (★cursor key = viewing unit, content = shared per agent — user correction):** two core-pure pieces —
  - `BoundedSeqLog<Chunk>` = **content, one shared copy per agent** (VecDeque + dual cap `min(2MB,4096)` + evict + oldest/latest seq; **data structure only**).
  - `SlotCursorMap` = **viewing unit (slotId) primary key → `{agent_id, cursor}`** (1:1 with ViewManager leaf). If N slots watch the same agent, N cursors + 1 copy of content.
  - Both are **Tauri-independent** (slotId/agentId generic — 0 Tauri types leaking into core, ADR-0012 / Codex FIX). Storage unit = **the original binary frame bytes**.
- **Ownership:** src-tauri `AgentBufferStore` = `Arc<Mutex<{ content: HashMap<AgentId, BoundedSeqLog>, cursors: HashMap<SlotId, ViewCursor{agent_id, cursor}> }>>`. Put **content and cursors under one lock** (separating them into separate maps inverts order).
- **fan-out reverse lookup:** when an agent's output arrives → find the slots watching that agent (match agent_id in `cursors`, or use the `OutputRouter`'s agent→targets — ADR-0036) and send from each cursor.
- **★Lock discipline (ADR-0006, FIX 2):** the buffer lock section does only **append + cursor update + collecting the `(WindowLabel, bytes)` snapshot to send**. **Channel `send` happens after releasing the lock** (isomorphic to the daemon `output_core` C4 pattern). → the nesting of buffer lock ⊃ registry lock disappears, **blocking the lock-order-inversion deadlock** between the fan-out path and the subscribe path. The registry (holding Channels) is taken only briefly at send time.

### 2. New window mount → buffer replay (decision point 2 / screen identity)

- `subscribe_output` invoke (a slot is assigned an agent): inside the buffer lock — if that agent's `content` buffer does not exist, create it (filled by the daemon `subscribe_from` FromOldest); if it exists (another slot already watches it) reuse it + `cursors.insert(slot_id, {agent_id, oldest_seq})` (from the beginning = PRD §3-1) + **the snapshot of [oldest~latest] bytes to replay** → send to that slot's Channel outside the lock. For an agent already being watched, 0 daemon re-requests.
- Raw bytes replay in order → xterm render + scrollback. **Screen identity** (PRD §5-6): the same raw chunk order as the daemon, so ANSI/partial UTF-8/CR preservation is **isomorphic to the current fan-out** (no worsening).
- **A new window during reconnect wait** (acceptance criterion 5): buffer preserved → immediate replay even while disconnected (0 empty screens).

### 3. Reconnect (decision point 3 — ★crux, FIX 1)

- **Axis A:** buffer preserved (even when disconnected). On reconnect the daemon `after_seq` = **the buffer's latest seq** → the daemon sends only what comes after and it is appended (no duplicates).
- **Axis B:** each window's cursor is **preserved regardless of reconnect**. Even if an unrendered window (cursor=oldest) fails to get the daemon replay and is disconnected then reconnects, that window **reads from the client buffer's oldest with its own cursor** → lossless (because it is in the buffer). = the current `should_deliver` "unrendered retry" (T7b) is naturally preserved by cursor reads.
- **gap (Truncated):** if the disconnection is long and `버퍼 최신 < 데몬 oldest`, the daemon sends `Truncated` → receive from the daemon's oldest and rebuild the buffer on that basis (oldest↑), **clamp every window's cursor to `max(cursor, 새 oldest)`**. The span in between is an unavoidable loss the daemon also evicted (inside the PRD §3-1 "retention lower bound" definition, truncation not displayed).
- `min_render_seq` (summing the most-lagging window) is **discarded** — the resubscribe criterion is the buffer's latest (axis A), not a window.

### 4. Eviction · buffer lifecycle (decision point 4 — user confirmed + FIX 4)

- **evict:** `pop_front` at the daemon-mirror cap (2MB/4096), oldest↑ → the client retention lower bound.
- **★Buffer life = only while assigned to some window (user confirmed).** The client buffer is a "display cache" and the daemon holds the original (while the agent lives), so keep **only 1 copy per unique agent currently assigned to some View**. If N windows watch the same agent: 1 buffer + N cursors. **When all windows let go of that agent (close/reassign/window close) the buffer is discarded** → when assigned again, replay from the daemon `subscribe_from` (FromOldest) into a new buffer (one local loopback hop, lossless). Memory is naturally capped by **the number of unique agents watched at the same time** (even with dozens of agents, only those put on screen). Consistent with the PRD/ADR division "daemon = long-term original / client = view-filling cache".
- **terminal cleanup (absorbs FIX 4):** the "discard if not watched" rule above fundamentally blocks the leak — if an agent is terminal (Killed/Exited) there is no live assignment in any window, so the buffer is discarded (the leak of dead agents resolved automatically, opus's point). Discard trigger = **View assignment release** (close_slot/reassign/window close) — when the last cursor leaves, drop the buffer. Implemented by removing that agent's buffer entry when the `ViewCursorSet` becomes empty (0 cursors).

### 4b. epoch transition (FIX 3 — tagging, not a lock)

- **Correction:** since `main_loop` is a single actor (select! task), the SubscribeAck arm and the Binary arm are **already serial** — "buffer lock serialization" is meaningless (opus's point). It is not a race a lock can block.
- **Tagging:** the shared buffer holds an **epoch tag**. If a Binary frame's `frame.epoch ≠ 버퍼 epoch` → **buffer reset (new stream seq 0) + reset all window cursors, then append** (based on frame.epoch without waiting for SubscribeAck). Consistent with the current `decide_epoch` (passes if st.epoch=None).
- **Needs confirmation (coder spike):** whether the daemon wire guarantees `SubscribeAck → replay → frame` FIFO even right after an epoch transition (opus did not confirm). Tagging is safe regardless of this order, but if it is guaranteed the reset trigger could be placed on the Ack — settled by a spike.

### 5. Reworking output_window_seq.rs (decision point 5)

- The current per-window `render_seq` (`should_deliver`/`mark_rendered`) → absorbed into the per-slot cursor of `SlotCursorMap`. The "unrendered retry" (T7b) is preserved by cursor=oldest reads (§3 axis B). `min_render_seq` → **discarded** (§3 axis A). `reset_all_for_agent` → absorbed into the cursor reset on epoch tagging (§4b).
- **Handling the coder's separated (held-back) part:** the previous session's `output_window_seq.rs` (min model + 14 tests) is **replaced** by this rework (min summation discarded). New = `BoundedSeqLog` + `ViewCursorSet` headless unit tests.

### 6. Resize risk (PRD §7 carry — not a new decision)

A single shared-buffer raw bytes ∩ windows of different sizes → escape conflicts are possible. **The current raw-chunk-identical fan-out is the same, so the same remains unresolved** (no worsening). Retrofit when per-viewport buffer branching is needed (ADR-0040 §impact risk). The resize policy is a separate open item.

### Frontend (unchanged — ADR-0037)

Pure renderer. src-tauri manages all cursors/indices, and the frontend gets only the raw bytes Channel. A new window replay uses the same Channel → no frontend code change.
- **Deadlock is irrelevant to the frontend (load-bearing):** WebView JS has a single-threaded event loop so there are no locks → deadlock (lock inversion) is solely for src-tauri Rust (§1 lock discipline), the frontend is not a defense target. The frontend's concurrency risk is not deadlock but async race/order/stale (Channel callback overlap · resubscribe · revival of a dead connection), and the existing `tauriTransport.ts` guards (generation guard · idempotent registration · single source of truth for connection state, T7c) are **kept** (do not add new ones or delete existing ones). The only thing to watch = the replay→live order, guaranteed by the single Channel FIFO (the frontend writes in the order received).

---

## Module boundaries (DDD — FIX 5: common extraction is data structures only)

- **core** — `BoundedSeqLog<Chunk>` (VecDeque + dual cap + evict + oldest/latest seq, **data structure · pure methods only**) + `SlotCursorMap` (slotId→{agent_id, cursor}, generic). Tauri-independent, headless unit tests. **The daemon `ReplayBuffer` struct signature · synchronization · on_ready · C4 call structure are not shared** (owned by the daemon/client each) — so that the common extraction does not regress the daemon emit hot path and the C4 TOCTOU (:374), only **data structures** are shared (whether the daemon adopts `BoundedSeqLog` is optional after confirming signature safety, not forced).
- **src-tauri** — `AgentBufferStore` (assembles BoundedSeqLog + ViewCursorSet, epoch tag, Arc<Mutex>), `main_loop` Binary arm insertion, `subscribe_output`/reconnect wiring, **Channel send outside the lock** (§1).

---

## Concurrency invariants (★target of `/review code deep` — ADR-0001/0005/0006/0007)

- **Lock order (ADR-0006, FIX 2):** inside the buffer lock = data only (append/cursor/snapshot), Channel send outside the lock → buffer lock ⊃ registry lock nesting 0, inversion deadlock 0. The ViewManager lock (ADR-0035) is also not held on the fan-out hot path (routing snapshot, ADR-0036).
- **epoch (ADR-0007, FIX 3):** single-actor serialization + frame.epoch tagging. The fresh fallback (ADR-0008 new sid) is also absorbed by the tagging reset on a SubscribeAck epoch change.
- **Lifecycle (ADR-0001/0005, FIX 4 + user confirmed):** the daemon reaper/finalize are daemon-owned. The client buffer is **discarded on View assignment release** (0 cursors → buffer drop) — a terminal agent loses its assignment so it is discarded automatically (leak 0). On reassignment a new buffer from the daemon replay.
- **Daemon hot-path non-regression (FIX 5):** common extraction is limited to data structures — daemon emit/C4 unchanged.

---

## Acceptance criteria (PRD §5 + implementation)

PRD §5 1~8 + implementation gates: `cargo test` (root, including `BoundedSeqLog` · `SlotCursorMap` unit tests) + core `rg "use tauri"` 0 + `npm test`/`tsc` + `cdp.mjs` multi-view measurement (2·3 windows on the same agent, opening a new window late, **a new window during reconnect**, **lossless reconnect of an unrendered window**, buffer cleanup after terminal).

---

## Decision point summary (user report / forks)

| # | Decision point | Handling | Fork? |
|---|---|---|---|
| 1 | index↔buffer | seq unit, `BoundedSeqLog` (data structure only in core) | internal (report) |
| 1b | **cursor key unit** | **slot (viewing unit) primary key** + content shared per agent — linked to the ViewManager leaf | user confirmed |
| 2 | xterm↔buffer | raw replay = isomorphic to the current fan-out, screen identity | internal (report) |
| 3 | reconnect | **two axes separated** — daemon sync (buffer latest) ≠ window read (per-view cursor). gap = clamp | internal (crux, deep review) |
| 4 | buffer life | **only while assigned to some window** (discard at 0 cursors, daemon replay on reassignment) — cache/original division, leak blocked automatically | user confirmed |
| 4b | epoch | single-actor serialization + frame.epoch tagging (not a lock) | internal (report, wire-order spike) |
| 5 | output_window_seq | min discarded → absorbed into the cursor, replaces the held-back part | internal (report) |
| 6 | resize | the same unresolved as now, retrofit risk stated | open item |
| lock | lock discipline | send outside the lock + single lock → inversion deadlock 0 | internal (report, deep review) |

**#4 buffer life = user confirmed** — only while assigned to some window (discard at 0 cursors, daemon replay on reassignment). Consistent with the client = cache / daemon = original division + a natural memory cap (the number of unique agents watched at the same time) + automatic blocking of the dead-agent leak. The rest is the correct answer for lossless correctness · isolation, so internal design.
</content>
