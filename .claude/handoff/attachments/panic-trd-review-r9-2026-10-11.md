# Panic TRD v9 — review round 9 (2026-10-11) · deep: codex Designer (blind) FIX (1) · Architect-breaker FIX (1 minor + 5 nits) · concurrency lens FIX (4 low; "no BLOCK, no race, no double entry, no new unbounded path under (바)=감시"). Aggregate = FIX. No BLOCK. (라′)/(바′) framing judged fair by codex; facts verified (tauri/tao lines, P1 layer-2 zombie against code). Snapshot = 0b697a85.

## Main corrections for v10
- R1 (codex) Non-panic shell exits still call `AppHandle::exit` — setup failure `src-tauri/src/lib.rs:237`, tray quit `src-tauri/src/tray/actions.rs:192`. If an escalation is active and tauri's `request_exit` fails, tauri falls back to `process::exit` (`app.rs:574–579`) → ExitProcess ends the watchdog first. Fix: one shell wrapper for these callers — if `esc.is_pulled()` → do nothing (the pulled path owns the end); else `app.exit(code)`. Residual race (pull lands between the check and tauri's fallback) listed in §4-11. U6 test for the wrapper.
- R2 (arch N1 · lens F1 — both) `claim_reason_log` semantics: the tail head takes the claim ONLY when it actually prints a reason — `if reason().is_some() && claim_reason_log()` (evaluation order pinned); otherwise it logs a reason-less summary without claiming and leaves the claim to `before` (`!stalled && claim_reason_log()`). Cost: possible duplicate line for observer-only pulls (same as 8판). Fix `:548`, `:477` wording; U1/U5 tests pin it.
- R3 (arch N4 · lens F3 — both) Shell `Exit` entry branch: skip its reason log when `wait_reason` reports `stalled` (same guard as the daemon `before`) — keeps the branch ≈2 s, under `LOCK_WAIT` 3 s (avoids opening the run-lock overlap residual). M18, §4-9 `:601`, U6 test.
- R4 (arch N2 · N3 · lens F2 · F4) (라′) framing completeness:
  - B's benefits: B's hook terminates before any unwinding, so B ALSO closes (바)(2) both cases (main thread blocked on the panicking thread; main-thread unwind stalling in a WebView2/COM `Drop`), posted-closure caveats ① (`LoopDestroyed` race) and ② (modal loop), and the dependency on tao rethrowing to the `main.rs` catch. Under A these fall to the 5 s watchdog; under (바)=미룸 유지 they are unbounded with A but closed with B; only (바)(1) (hook (e) stall) remains for both.
  - B and the user decision 「셸에도 같은 훅」: "B 는 판정 · 깃발 · 사유 · 로그는 데몬과 같고 끝만 다르다 — 사용자 결정 「같은 훅」과 어긋나는지 함께 묻는다" — put it in the B row / 묶임, not only §8-1.
  - B's hook order: terminate after (f) to keep the debug-build stderr backtrace §5 row 8 relies on (release has no stderr).
  - "B 면 바뀐다" list adds §4-3 hook pseudo-code, §4-4 당겨진 끝 shell paths, §3-2 M7/M18, §0 ③④.
  - (라)=S1a′ + B: the shell observer still exists → observer pulls (no hook) would be ended only by the watchdog; fix = under S1a′ + B the shell observer calls `terminate_self` directly (lock-free, log-free; observer contract permits). Under S1a + B no gap.
- R5 (lens P1 optional · lens cost) (바′) framing:
  - Add: the layer-2 zombie stays in `sessions`, so re-activating that agent returns the stale live session (`manager.rs:1681-1687`) — the user cannot restart that one agent, only the daemon.
  - Correct the 넣음 cost: no per-request timer or per-entry arming is needed — the hook already runs before unwinding starts; it can CAS-claim a slot in a fixed global atomic table and write `Instant::now()` (no alloc, no lock), keep the slot index in TLS; the catch arm / completion check clears it; the existing watchdog thread scans the table. Panic-free long requests are never timed. Fix the conflicting cells (`:338`, `:340`).
- R6 (arch N5) M7 "101 = 상수 하나" → "crate 마다 상수 하나(데몬 · 셸)" (shell has no runtime dependency on daemon — ADR-0270; base = no policy; platform = OS only).
- R7 (arch N6) "stalled ⇒ … 확실하다" → "가능성이 높다" (a second hook entering near the deadline also sets it).

## Verified sound (keep)
P1 layer-2 consequence against code (pump `finish` outside catch `pty.rs:298-342`; `join_pump` `recv_timeout` `output_core.rs:822-831`; `kill_agent` continues without removing the session `manager.rs:3049-3074`; only production `Reap` sender = `on_terminal` from `finish` `manager.rs:2084-2104`) · P2 · P4 (ADR-0269 ② ③ hold) · P5–P9 · (라′)=B needs no Amends (0288/0290 originals are daemon-only; no shell/same-hook decision recorded) · (라′) tauri/tao lines · daemon termination identical under A/B · B persists nothing at risk.

## Cost
codex 77k / 2 · architect 212k / 15 · lens ~? / ?.
