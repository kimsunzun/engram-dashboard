# Panic TRD v8 — review round 8 (2026-10-11) · deep: codex Designer (blind) BLOCK (1 + 1 framing) · Architect-breaker FIX (6 nits, "no BLOCK, no user decision") · concurrency lens FIX (1 user-fact + 3 nits, "no race / double entry / self-wait; every escalating path bounded under (바)=감시"). Aggregate = BLOCK (provisional) — the BLOCK is a REVIEWER CONFLICT on a residual's scope → USER sub-question (라′). Snapshot = d2a65750.

## USER sub-question (라′) — reviewer conflict
- Conflict: codex — a shell plugin `on_event(Exit)` panic (tauri `app.rs:1431–1433`, runs before our callback) is caught and discarded by tao (`runner.rs:95–109, 242–254`), then tao calls `process::exit` (`event_loop.rs:225–230, 283–287`) → `ExitProcess` terminates the watchdog first and may hang in DLL detach → an escalating shell panic WITHOUT a bound. §4-11 accepted only "exit code 0, no Final". Claude reviewers — residual (a) is the same `ExitProcess` path every normal app quit takes today; accepted.
- Options for the user:
  - (A, main recommendation) Keep the design; restate residual (a) honestly: unbounded only if DLL detach hangs, which is the same risk as every normal quit today; window = a panic inside a plugin's Exit handler (single-instance plugin etc.) — very narrow.
  - (B, codex's simpler alternative) Shell hook terminates directly: escalating shell panic → pull → reason → (e) log → `terminate_self(101)` from the hook itself (watchdog covers a stalled log). Removes the waker / posted closure / Exit re-check machinery for panics. Cost: no tray cleanup on panic → ghost tray icon until hover (Windows); panicking thread's caller never gets an INTERNAL reply.
- §0 table + ask order: (라′) right after (라).

## Main corrections for v9
- P1 (lens N1 · codex #2) (바′) facts: trigger (ii) at LAYER 2 (pump, writer, PTY watcher, stderr drain, leftover bodies — pump `finish` sits outside `contain`, `:553`) → destructor stall while the pump body unwinds → `finish` never runs → no terminal transition (ADR-0005 pump-only) → reaper never gets done → `kill_agent` gives up after `join_pump(5s)`, agent stuck in Exiting; `inputs_pending`/`last_end_failed` have no 30-min cap → mail to it blocked → zombie agent until restart. Also (codex) a destructor stalled during unwind may retain a shared lock → other requests needing it hang too; and the file sink is only one of the logging locks (§4-3 lists reload/EnvFilter RwLocks, stdout). Restate the user's real trade-off: "one hung request / one zombie agent / possibly requests blocked on a retained lock" vs "restart after N s"; if 넣음, say how the deadline is armed before unwind can stall (at boundary entry, cancelled on completion). Recommendation stays; framing must carry these. Update §3-1 (바′) table + rationale, §0 6′, §4-11, §5.
- P2 (lens N3) (바′) (i) names the 8판 completion-check log as well as the catch arm.
- P3 (arch N1 · lens N2) Daemon `before` "reason line not yet logged": add a first-wins primitive `Escalation::claim_reason_log() -> bool` (AtomicBool swap) used by the tail head and by `before`; drop the log from `decide_exit`'s `Err(_)` arm (keep `decide_exit` pure). Optional (adopt): `wait_reason` reports whether it timed out with the counter > 0; the daemon `before` then skips its log (certain to block on the stuck sink).
- P4 (arch N2) `end_pulled` takes `terminate: impl FnOnce()` with the caller binding 101 (no exit policy in base — ADR-0269 ②); M7 keeps "101 = one constant" at the call sites.
- P5 (arch N3) Wrong M-ids: `:573`, `:574`, `:657` say M7 for the run-lock / exit-session decision → M18.
- P6 (arch N4) Header 7판 요지 ① 「일반 멎음 감시 = ADR-0288 이 미룬 장치」 → tag "(8판 정정 — §3-1 (바′) 근거)".
- P7 (arch N5) Header 표기: add the 8판 measurement tree (`fc5a1a9a`).
- P8 (arch N6) §8-1 `:731` clause joiner: name it — e.g. 「결정 1과 2의 범위와 영향 서술, 결정 3의 전용 장치, 미룬 시간 제한」 (no ` · `, which the script rejects).
- P9 (lens N4) (a) the completion check sits inside the boundary's catch region or is panic-free — state which (sink tolerates poison `logging/mod.rs:130`); (b) the T4 attempted flag = shared `AtomicBool` with `swap(true)` (posted closure needs `Send`).

## Verified sound (keep)
No double entry of `end_pulled` (daemon: one caller, mutually exclusive outcomes; shell: all callers on main thread, terminate never returns) · no self-wait · T2 multi-byte safe · T3 per-poll inside `Contained` · F1/F2 (ADR-0288 text) /F3 · T4/T5/D1–D3 · clause phrases satisfy `flow.md:33` · termination tables (daemon + shell) bounded under (바)=감시 except listed residuals.

## Cost
codex 78k / 2 · architect 209k / 24 · lens ~? / ?. Tip: §10-1 should cite the single sentence that now owns each moved responsibility; M-cells are 8–10k chars and mostly repeat §4.
