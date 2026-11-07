# U3c — re-review round 2 (delta)

Same role and rules as your round-1 brief (`R\review\brief.md`, R = `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard\31fa411b-c656-4850-8c1d-2128bb9a19dd\scratchpad\u3c`). Read only.

## Target
- Previous snapshot tree `be0622637876ab361d5aa46de14a517c6bd21004` → new snapshot tree `70f29cfafaa9596cb52f354baaaed7e3a3805e74` (working tree equals it). Delta diff: `R\review\u3c-r1-delta.diff` — 3 files: `crates/engram-dashboard-transport/src/peer.rs` (doc + 1 test), `src-tauri/src/daemon_client/connection_v2.rs`, `connection_v2/tests.rs`. Full diff vs base `1d3a8486` for context only: `R\review\u3c-full-r2.diff`.
- `src-tauri/src/daemon_client/mod.rs` and `daemon_client/tests.rs` are unchanged since round 1 — excluded.
- What main asked: `R\review\fix-r1.md` (F1–F9, T1–T7). Coder report: `R\review\coder-report-r1.md` (claims — verify).

## Fix mechanism in short (claims)
- F1: `opened_generation(peer, closes)` binds the generation captured at call start; receivers cloned before the first check; succeeds only when the published generation equals it; `UNSENT_ON_DISCONNECT` once the peer is not live or its generation changed.
- F2: shared counter `queued_fires` (inc by caller before send, cap `QUEUED_FIRES_MAX` = old channel bound; dec by actor on every write/drop incl. flush on `LinkUp` and close).
- F3: a fire is written directly on the caller when the live generation at call is published, the close epoch is current and `queued_fires == 0`; else via the actor. `publish_opened` publishes only once no fire is left in the actor channel, re-checking after each `Fire` the actor takes.
- F4: watch carries `Opened { generation, closes }` where `closes` = the epoch the current handle was added under (not the publish-time epoch); callers read `closes` first → `NOT_CONNECTED` on an older epoch or a `Closed` peer; direct fire path same check.
- F5: `apply` skips `ToDriver` CommandRequest while `current.is_none()`.
- F6: `Peer::request_in` doc + test "first poll queues the request".

## Ask
1. Your round-1 findings, numbered, each with a disposition: `fixed` · `partially` · `not fixed` · `newly broken`.
2. New surface created by the fix — weigh it MORE heavily than the old items: the counter's exactly-once decrement on every path (write, drop, held, flush, close, re-add, actor teardown/client drop — can it leak and permanently block the direct path / the publish, or underflow?); publish hold-back starvation (`publish_opened` waiting on fires → round trips time out); `Opened.closes` = add epoch semantics vs `close(); connect()` re-add ordering (review F1/F2 history); new tests earning their names (would each pass with its property removed?); test timing (50 ms real sleep in the F1 test, usage lock held from the test).
3. Label each new finding `previous finding unresolved` or `new surface from the fix`.

Output: `PASS | FIX | BLOCK` + tagged findings (`must` · `권장` · `hold` · `batched`) as before. Absence claims need the trace.
