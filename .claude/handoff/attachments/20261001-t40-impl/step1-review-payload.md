# T-40 step ① — review payload

Diff: `git diff HEAD -- crates/` (saved copy: `step1.diff` in this folder; HEAD = 5182039, code base = 9fa9215). 12 files, +1657/−684, all in `crates/engram-dashboard-agent`.

## Task / completion criteria
TRD `docs/process/S21-chat-ux/trd-t40.md` §6 step ① (sub-steps 1–8), with interface shapes fixed by §6 「접점」 and OS behaviour by §3-5, tests by §5 (items 1–11). No user-visible behaviour change in step ① (wiring is step ③; new items are dead code until then). Each sub-step compiles alone. Only new deps = `windows` features `Win32_System_IO`, `Wdk_System_Threading` (Cargo.lock unchanged).

## Change summary (by sub-step)
1. Removed `root_attached` (stdio field/block, `ProcessGroup::new(job)`, accessor, 2 test asserts). `detached()` kept (cfg(test)) — used by cross-OS test.
2. Removed `console_wrapper_depth` + test (backend/mod.rs back to 9fa9215~1).
3. stdio process-group test rewritten without clock (100 ms hold, member snapshot before gate); removed process_tree step-1 additions (`system_time_to_filetime`, `walk_levels`, `children_from_table`) + 11 tests (file back to 9fa9215~1).
4. windows.rs: `ProcessFacts` (now defined in process_group.rs, imported), `PinnedMember{exited,wait_exit,facts,terminate_raw,classify}`, `pin_member(pid,kill)` (open → IsProcessInJob same handle → facts once; 87/not-ours → Ok(None); access denied → Err), `watch_births(start)` (port → start(Arc) → associate only if Ok; own Arc held until association returns), `unwatch_births` (NULL port), `BirthPort{next}` → `PortEvent::{Joined,Other,Timeout}`; local `repr(C)` PROCESS_BASIC_INFORMATION (ppid trusted only if ReturnLength == size); cmdline buffer header+64 KiB; shared kill helpers `terminate_handle`/`classify_terminate` (ADR-0257 anchor). Tests 6, 7a, 7b, 8(#[ignore] Git Bash; shape `bash -c "read _; exec /usr/bin/bash x.sh"`, DETACHED_PROCESS).
5. `AgentTransport::begin_retire(&self) {}` default; stdio `retiring: Arc<AtomicBool>` set by `begin_retire` and FIRST line of `shutdown`; `Session::begin_retire`; `kill_agent` calls it right after `get_session` (no lock), before revoke/set_intent/Exiting/kill; `tear_down_failed_activation` calls it right after the incarnation-marker check. Trait doc adds contract: implementors must not emit externally observable outcomes from it (it precedes `set_intent`). Tests: order log [begin_retire, revoke, exiting, shutdown]; mismatched marker → nothing.
6. process_group.rs: `RetiringSignal(Arc<AtomicBool>)` (`of`, `is_set` Acquire, no setter); traits `Pinned`, `Births` per §6; `ProcessGroup{job: Weak, retiring}` with `member_pids`, `pin` (Box<dyn Pinned>, Windows newtype `JobPin` mapping MemberOutcome→MemberKill), `watch_births`, `unwatch_births`, `retiring`; job gone → list Ok([]), pin Ok(None), watch/unwatch Err(GROUP_GONE = NotConnected, start not called); non-Windows → Ok([])/Ok(None)/Err(Unsupported). Removed `verify`, `Verify`, `ReadyKill`, `JobMember`, `verify_member`, `MemberCheck`, `VerifiedMember`, HEAD `:381` test; ported HEAD tests onto `pin`.
7. `OnWritten` (in input_queue.rs), `InterruptOut{bytes,on_written}` + `InterruptLine` (stdio.rs); queue pending holds `(bytes, Option<OnWritten>)`, `push_with`, `pop` returns pair, `drain` write → mark_written → callback in own catch_unwind (panic → warn, continue), no queue lock held; close drops pending outside lock; refused push / write failure drop callback uncalled. claude `interrupt_line` returns `InterruptOut{bytes, on_written: None}`. pty.rs untouched.
8. Header docs: platform/mod.rs, input_queue.rs, stdio.rs.
Fix: `a_dead_parent_held_open_is_off_the_list_and_reads_dead` made deterministic (P kept alive on a 2nd `set /p` until the test kills it; `try_wait` precondition).

## Gates already run by coders (not a substitute for /qa)
`cargo test -p engram-dashboard-agent --no-fail-fast -- --test-threads=4` → lib 1118 passed / 0 failed / 2 ignored, all integration binaries green; `cargo test --workspace --no-run` OK; `cargo fmt --check` OK; stdio.rs & claude/mod.rs CRLF kept; `rg "^\s*use tauri"` agent = 0. Non-Windows cfg paths NOT compiled (no target installed).

## Coder self-check findings (for reviewers to judge)
- F1 NULL-port detach + re-report of existing members on association verified only on Win11 26200.
- F2 `pin_member` on an already-exited-but-not-reaped process returns Some with facts from a dead process (cmdline likely empty) — untested.
- F3 `pin_member` access-denied → Err (not None); step ② must treat as chain-link-only.
- F4 `begin_retire` now precedes `set_intent` — the session doc for `termination_intent` says intent is set before any kill-caused observation; contract moved into the trait doc instead.
- F5 `watch_births` holds a strong Job Arc during `start` → a blocking `start` delays Job close (KILL_ON_JOB_CLOSE).
- F6 Fixed interface lacks a side-effect-free "group gone?" check; reaper drops a naturally-exited session without shutdown, so `retiring` never rises → a listener could spin (to be added in step ② as `is_gone()`).
- F7 A panic inside a payload's own Drop in the queue reaches the outer guard and closes the queue (left as-is).
- F8 `wait_drained` may return before a chunk's callback has run (mark_written precedes callback per TRD).
- F9 A test-only weak spot: `parent_is_dead` returns false when pinning the parent fails.
- F10 One coder (chunk C2) reported the suite green while its new assertion could never hold (5/5 fail on re-run) — treat coder "green" claims with suspicion.
- F11 Known flaky, unrelated: `tests/submit_delivery_boundary.rs` PTY timing under load.

## Relevant invariants / ADRs (doc-aware lens)
CLAUDE.md 「핵심 불변식」 (kill causality ADR-0001, lock order ADR-0006/0231, ownership split), 「코어 격리」 ADR-0003, 「백엔드 확장」 ADR-0004, 「플랫폼 중립」 ADR-0230, ADR-0218 (hold identity via open handle), ADR-0238, ADR-0257 (partially superseded by this design — step ④), ADR-0012 (seams/tests).
