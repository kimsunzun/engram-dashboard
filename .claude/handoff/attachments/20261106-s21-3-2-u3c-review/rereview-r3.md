# U3c — re-review round 3 (delta)

Same role and rules as your round-1 brief (`R\review\brief.md`, R = `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard\31fa411b-c656-4850-8c1d-2128bb9a19dd\scratchpad\u3c`). Read only.

## Target
- Previous snapshot tree `70f29cfafaa9596cb52f354baaaed7e3a3805e74` → new snapshot tree `78a9ed9522d780e184a66a357579a7441359c5ea` (working tree equals it). Delta diff: `R\review\u3c-r2-delta.diff` — 2 files: `src-tauri/src/daemon_client/connection_v2.rs`, `connection_v2/tests.rs`. Full diff vs base `1d3a8486` for context: `R\review\u3c-full-r3.diff`. `mod.rs`, `daemon_client/tests.rs`, transport `peer.rs` unchanged since round 2 — excluded.
- What main asked (prescriptive — the design itself is under review too): `R\review\fix-r2.md`. Coder report: `R\review\coder-report-r2.md` (claims — verify).

## Round-2 findings to dispose (`fixed` · `partially` · `not fixed` · `newly broken`)
1. Publish stays held back after a stale fire is dropped (counter hold-back) → round trips time out.
2. Caller decision vs actor publish not atomic → same caller's round trip overtakes its own fire; and while published, an actor-path fire overtaken by the same caller's round trip.
3. The F1 test did not prove F1 (waiter failed before g+1 existed).
4. The fire half of the F4 close test was racy.

## Weigh the new surface more heavily
- The `Gate` (`opened` + pending fires under one std mutex): leaf-lock claim (nothing else taken under it; registry/usage read before; `notify_in` only), poisoning/panic paths (ADR-0292/0293), ADR-0195 (never held while disconnected — a fire queued an instant before close stays pending until the next `LinkUp` drops it: acceptable?), cap behaviour, the pending drain on `LinkUp` (== g written, older dropped, newer kept) and close clearing.
- `opened` watch now carries `()` as a wake-up only: any lost wake-up between the gate decision and `changed()`?
- The `biased` `tokio::select!` in `opened_generation` (production change): can a ready outcome starve the timeout, or the reverse?
- New/replaced tests: does each fail with its property removed (the coder states one uncaught mutant — drain after signalling the publish)?
Label each new finding `previous finding unresolved` or `new surface from the fix`.

Output: `PASS | FIX | BLOCK` + tagged findings (`must` · `권장` · `hold` · `batched`). Absence claims need the trace. Only list wording items not already in main's batch (send_command/opened_generation "old driver's text", `SharedUsageInterest` lock doc, "the next one registers", the report's fire-order-vs-Unsubscribe line).
