# T-40 step ② — review payload

Target: working tree vs HEAD `ccde81a` (step ① committed), frozen snapshot `refs/backup/t40-step2-final` (= a8cfa85). Diff copy: `step2-u5.diff` here. Files: `backend/claude/leftover.rs` (new, 6535 lines: production 1–2552 — rules 1–305, recording 306–941, gate/worker/pass 942–2552; tests 2553–end), `backend/claude/mod.rs` (+`mod leftover;`), `platform/process_group.rs` (`is_gone` + test helper re-export), `platform/windows.rs` (test ⑧ unwatch), `transport/input_queue.rs` (doc link).

## Task / completion criteria
TRD `docs/process/S21-chat-ux/trd-t40.md` §6 step ② (`leftover.rs` + tests, no wiring), implementing §3-1…§3-4, §3-8 logs, §5 tests. Not wired: no user-visible change. `#![allow(dead_code)]` until step ③.

## Chunks
- ②a pure rules: `Link`, `Chain`, `Verdict`, `select`, `candidates`, `is_hook_command`, `taskkill_names`, `ANCESTOR_MAX`.
- ②b recording: `Birth`, `Recorder`, `BirthWatch::ensure(_with)`, listener `listen`, `LogTag`, private→pub(super) `trait Group`, `ProcessGroup::is_gone` (strong_count==0, no upgrade).
- ②c1 gate: `GateCell<G>`, `GateState`, `Episode`, `Mark`, `Snapshot`, `SnapState`, interrupt two sections + `Opener` guard, `take_snapshot` (PIN_MAX 256, causes), W via on_written, `note_written`, decoder hooks, `next_step`, worker + `WorkerGuard`, `precheck`, `still_due`/`commit_check`/`note_kill`, `end_pass`, `abandon`, `LeftoverClock{mono_now,sleep,spawn}`.
- ②c2 pass: `run_pass` → `precheck` → `kill_rounds` (PASS_ROUNDS 3, KILL_CONFIRM 200 ms shared per round, recheck outside lock, commit = one terminate under gate lock) → `end_pass` → `log_pass`.

## Coder-declared deviations / interpretations (judge them)
- ②a: launcher row gives AncestorAlive{claude} when claude alive (TRD said NotHook; both misses); ANCESTOR_MAX counts A₁…Aₖ₋₁ ≤ 16; `is_hook_command` rejects CR/LF in S; `taskkill_names` accepts only exact `/PID n /T /F`; `select` → Unknown if root_pid==0; `parent_of` tie → snapshot entry, latest create ≤ child.
- ②b: drain-only mode ends on a second `next` error (TRD silent; avoids busy-spin; port may be dropped while attached — debug log); retiring exit unwatches before drop; `Unsupported` treated like GROUP_GONE (no latch); debug `born_ago_us` uses SystemTime (log only, not decisions — check vs §3-6); listener `stop(self.active())` on exit may miss a record started in between (claimed benign: missed cleanups only).
- ②c1: on_written also on continuing interrupts (TRD §3-2/§5 consistent); section ① bumps `last_mono` on continuing interrupt (only delays a pass); an unlisted first write doesn't latch (later W becomes first); generics `Cleaner<G>`/`GateCell<G>`; worker panic logged `error!` (orchestrator: logging convention wins over TRD §3-8 warn); `debug_assert!(why != Stale::Live)` on an unreachable branch; a live unwritten episode keeps snapshot handles until turn end/new input/retiring; `Failed(Port)` episode still turns the record on.
- ②c2: "round without Cleanup stops" = no NEW Cleanup (committed ones not re-selected); `rounds_exhausted` only if a 4th select-only round finds a new Cleanup; `unconfirmed` = terminated − confirmed; per-rule counts from the last round; extra stop reasons `record_gone` (debug), `released` (warn, unreachable); no-kill pass after an earlier killing pass of same episode still warns; `waited_ms` from `first_mono`.
- ②c2 suspicion: terminate failure path builds a `windows::core::Error` inside the gate lock — the step ① kill-safety reviewer verified (windows-result 0.2) `BOOL::ok` → `Error::from_win32` reads GetLastError only, no GetErrorInfo/alloc; re-confirm if in doubt.

## Gates run by coders (not a substitute for /qa)
Agent suite `--no-fail-fast -- --test-threads=4`: lib 1204 passed / 0 failed / 2 ignored; leftover tests alone 84 passed; workspace `--no-run` OK; fmt OK. Mutation checks by ②c2: per-kill deadline → test fails; classify inside lock → 9 tests fail.

## Known open issue (not part of this diff)
Step ① test `a_dead_parent_held_open_is_off_the_list_and_reads_dead` intermittently sees an empty member list under parallel load — being investigated separately.

## Carry constraints
`orchestrator-notes.md` (same folder): release panic=abort; pin Err → re-pin kill=false (chain-only); `start` keeps port Arc clone; detach before dropping last port Arc; `is_gone`.

## Invariants / ADRs (doc-aware)
CLAUDE.md 「핵심 불변식」 (lock order ADR-0006/0231 — two NEW leaf locks, never nested; kill causality ADR-0001; ownership split), 「백엔드 확장」 ADR-0004 (claude knowledge only in backend/claude), 「플랫폼 중립」 ADR-0230, ADR-0218 (identity via open handle), ADR-0238 (TurnGate → GateCell later), ADR-0257 (partially superseded), logging conventions `docs/reference/logging-conventions.md`, comment conventions.
