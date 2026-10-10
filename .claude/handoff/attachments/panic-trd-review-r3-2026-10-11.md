# Panic TRD v3 — review round 3 (2026-10-11) · deep: codex Designer (blind) BLOCK · Architect-breaker FIX (12) · concurrency lens FIX (7, "no BLOCK"). Aggregate = BLOCK (provisional — autonomous, heavier verdict). codex BLOCK #1 independently confirmed by architect N4 + lens N2. All findings are design/doc corrections — NO new user decision. Round-2 items: all closed except partial = UC (gate ② covers /mcp), D6 (M22 shape), D9 (test can't fail). Snapshot = d73aaceb.

## Main decisions for v4 (all design corrections — main decides, report)

### T — termination guarantee (codex #1 #2 · arch N4 · lens N1 N2)
- T1 Shell gets an INDEPENDENT watchdog thread spawned at hook install — same shape as the daemon's (M19). The shell waker keeps only the `app.exit(101)` call (may block — no longer matters). Rationale: tauri 2.11.3 `AppHandle::exit` (`app.rs:574-579`) on failed `request_exit` runs `log::error!` (→ tracing via LogTracer, `base/src/logging/mod.rs:422`) → `cleanup_before_exit()` (`tray.icons.lock().unwrap()` `app.rs:1110`) → `process::exit` on the caller thread, so a deadline on the same thread is not a bound. The 3판 claim "그 밖은 같다" for the separate-thread alternative is wrong.
- T2 Watchdog exit action = `TerminateProcess(GetCurrentProcess(), 101)` via a new platform helper (OS code → platform crate, ADR-0266; ~10 lines; non-Windows = `libc::_exit`-equivalent or `process::abort`-free immediate exit — keep cfg inside platform). Skips DLL detach / loader lock / atexit / TLS dtors / std `rt::cleanup` Once. Nothing lost: reason already stored, file sink unbuffered, Job kill-on-close reaps children. Tail + M6 normal paths keep `process::exit(101)`. Correct §9-12 (was from memory) → note std `process::exit` may block behind another thread's exit / loader lock (inference, unverified — rust-src absent).
- T3 Hook order: (a) decide escalation WITHOUT allocation (payload downcast `&str`/`String` borrow + `is_poison_panic` substring check + depth read) → (b) shell: `MAIN_ESCALATED` SeqCst → (c) `pull()` → (d) then format/store first reason (OnceLock) + site → (e) log. Resolves codex #2 (pre-latch work unbounded) vs round-1 lens F3 (reason lost to a fast tail): the tail, before its final `exit`, waits for the reason OnceLock up to a short bound (draft 1 s, polling) then logs whatever it has. Missing reason = accepted diagnostic loss.
- T4 Watchdog spawn only `cfg(panic = "unwind")` builds (arch N11) — under abort it has no function; spawn failure → boot `Err(1)` stays.

### Gates (다) (arch N1 N10)
- G1 Gate ② = control routes only (`route_layer` on `ControlRoute::ALL` paths or path check) — NOT the whole app: `mcp_server.rs:1372` `nest_service(MCP_PATH)` + layers `:1401-1407` would reject rmcp session DELETE/SSE teardown. MCP rejection stays at gate ③ (tool wrapper) only → N1a tool-error shape.
- G2 Gate ① (`on_text` after parse) allowlists cleanup-type commands (StopDaemon, unsubscribe, viewport release) — keep the "never in cleanup steps" rule true.

### M21 sink (arch N3 N6)
- S1 U4 lands contain + drop-the-notification only; the explicit `pull` moves to U5 (where its consumer, the accept-loop interval arm / watchdog, lands). Fix §7-2 stop-anywhere text.
- S2 Before pulling, M21 records the caught message as the first reason (under `contain` the hook saw contained=true and recorded nothing for non-poison panics).
- S3 §4-2 layer table: add row "상태 싱크 broadcast — 가둔 뒤 당김(3층)"; state real exposure is poison-only (`net/src/ws.rs:208-217` is `.expect` + `try_send` + `warn!`).

### M22 send_input guard (arch N9 · lens N3 N4)
- I1 Replace the Drop-guard with plain `std::panic::catch_unwind` (NOT `contain` — hook already ran at original depth) around `send_input`; on catch emit `Dropped{Rejected}` (not unwinding → no abort branch), then `resume_unwind` (does not re-run the hook). Removes the P1/P1o skip and the check-then-act race.
- I2 False `Dropped` when the transport already accepted the bytes then panicked later in `send_input` (`pty.rs:360-363`) → accepted residual in §4-11 (rare; vendor `Delivered` on a tombstoned id; user may resend → duplicate input). Do not change the transport trait.

### Doc / ADR (arch N2 N5 N7 N8 N12 · lens N5 N6 N7 · D2 nit)
- A1 (바)=감시 Amends must cite ADR-0288 결정 3 too (watchdog = dedicated device for stage 3); merge with the (다) clause when both apply.
- A2 ADR-0290 「영향」 bullet "unwind 에서도 프로세스가 곧바로 끝나는 자리 = 메인 future 와 그 안의 연결 수락 루프 … 정상 종료를 거치지 않는다" — M6② routes accept-loop panics into the tail → add to the Amends list (verify the full bullet text first).
- A3 `/adr link` cannot add a 2nd clause between the same ADR pair (`~/.claude/skills/adr/feedback.md:151`, unresolved) → §8-1/§8-2: compose ONE merged Amends clause per pair (0292→0288, 0292→0290) at creation; note the tooling limit.
- A4 M20 reader list: add the control-HTTP outer layer + MCP tool wrapper (read `is_pulled()` for (다)) — latch passed in via MCP server state; and the test server (`TestServerHandle`, shares `run_accept_loop` with `tests/ws_e2e.rs` + `src-tauri/src/daemon_client/tests.rs`) — leaked local instance. `DaemonStatusSink::new` has 4 call sites (`lib.rs:334`, `connection_core.rs:3021`, `agent_conn.rs:922`, `status_fanout.rs:90`), §7-4 says three.
- A5 M9/§6 anchor `command_delivery.rs:527` → `:537` (`:527` is the fn signature).
- A6 D9 test: relabel as a no-panic/no-abort/no-hang smoke test of overlapping shutdown; drop the "차례로는 통과로 치지 않는다" overlap claim (child lock serializes `pty.rs:412-414`, `stdio.rs:394-396`; can't fail without a seam).
- A7 `SpawnReservation::drop` `.expect("spawning set poisoned")` (`manager.rs:746-753`) runs during unwind of the (나)=E window → switch to `sync::lock` in U4 (list next to `SessionTracker::drop` in M23 / §4-11 abort bullet).
- A8 (바) table + §5 row 11: "StopDaemon · 트레이 끄기는 건드리지 않는다" overclaims — a pull can land during a StopDaemon tail (observer recovering an earlier poison, unmarked teardown panic, M21) → that stop gets the 30 s cap + exit 101. Behaviour consistent; fix wording.
- A9 D2 nit: the pump checks the reason slot BEFORE the `shutdown`/`try_wait` classification (`pty.rs:321-333`), else Exited{code}.

## Verified sound (keep)
M20 seam shape (one extra param; no other ConnectionCore construction sites) · M23 forget (skips only reaper Stop+join + tracker Drop; flush explicit in tail; guard + daemon.json drop normally after manager) · (iii) `thread::panicking()` · watch Sender Clone · unit order green except S1 · §7-3 overlaps · 28 shell sites · conditional Amends text matches ADR-0288/0290 · M22 lock order (`Dropped` under `input_order`, matches documented order) · replay/golden unchanged · tail ≈15 s + MCP vs 30 s ≈2× margin · shell double-exit both code 101 · modal-loop deferral covered.

## Cost
codex 82k / 2 · architect 252k / 21 · lens ~? / ? (both Claude reviewers: costliest = the 651-line diff, ~60-75k; two reads hit the 25k cap). Next time: pre-split diff into <20k-token chunks indexed by section, or `+`-lines with 1 context line.
