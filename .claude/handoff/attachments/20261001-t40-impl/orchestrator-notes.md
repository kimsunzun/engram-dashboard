# T-40 impl — orchestrator notes (carry into step ② / ③ briefs and review)

- Step ②: listener needs a "group gone" check each round (TRD §3-2 약한 손잡이 사라짐). Reaper drops a naturally-exited session without shutdown(), so retiring never rises. Fixed interface lacks it → step ② coder adds a side-effect-free `ProcessGroup::is_gone()` (Weak strong_count==0; non-Windows true) — orchestrator decision (internal).
- Step ②: ProcessGroup is not Clone; share via Arc.
- Step ②: `pin` returns Err on access denied → treat as chain-link only (killable=false), per TRD §5.
- Step ②/③: watch_births holds a strong Job Arc during `start` — start must be only a thread spawn (non-blocking).
- Step ③: begin_retire runs before set_intent → reacting to retiring must not emit status/link/input outcomes (trait doc contract, chunk C1).
- Watch-job gone error kind: GROUP_GONE = NotConnected (not a hard port failure; do not latch port_failed).
- Known flaky: agent tests/submit_delivery_boundary.rs PTY timing under load (unrelated).
- Snapshots: refs/backup/t40-step1-chunk{A,B,C1,C2}.

## From step ① review (2026-10-01)
- Step ② brief: `pin(kill=true)` Err carries no facts → re-pin with `pin(pid, false)` to get ppid/create for chain linking (TRD §5 "사슬은 잇는다").
- Step ②/③ brief: release profile is `panic = "abort"` → every catch_unwind guard (on_written, listener, worker) is debug/test-only; callbacks/listener must be panic-free by construction (no unwrap/expect/indexing; handle poisoned locks).
- Step ②: `is_gone()` needed (confirmed by doc-aware reviewer, note A).
- Step ④: TRD §3-8 lists callback panic as warn; logging-conventions says panic → error. Orchestrator chose error (internal; convention wins) — fix TRD/ADR text in ④.
- Step ④: windows.rs header rule "windows crate + io only, movable whole (user decision 2026-09-29)" kept by keeping windows.rs a leaf (local fact/event types, converted in process_group) — orchestrator choice (a) of doc-aware finding 3.
- Step ② brief: `watch_births` drops its own Arc right after attaching → `start` MUST keep a clone of the port Arc (listener owns it); never drop the last Arc while attached (detach first).
- Step ② optional nits (fold into ② coder): OnWritten doc links pub(crate) RetiringSignal from a pub item (use plain backticks); windows.rs test ⑧ drops port while attached (call unwatch_births before scope end); ProcessFacts::from clones 64 KiB cmdline twice per pin (consuming conversion if convenient).

## Step ② split plan (orchestrator, 2026-10-01) — sequential coders, each compiles alone, all in backend/claude/leftover.rs (+ `mod leftover;`), no wiring
- ②a pure rules: `Verdict`, `select(c: &Link, view: &Chain) -> Verdict`, `is_hook_command`, `taskkill_names`; `Link{pid, facts: ProcessFacts, killable, seq: Option<u64> /*None=snapshot*/}`; `Chain` built from snapshot links + birth links + root pid + injected `exited: &dyn Fn(&Link) -> io::Result<bool>`; parent lookup by pid with parent.create ≤ child.create; tests = TRD §5 「규칙 — 스파이크 모양」 + is_hook/taskkill rows (can port r4sim.py shapes from .claude/handoff/attachments/20260930-t40-provenance-spike/).
- ②b birth recording: `ProcessGroup::is_gone()` (platform, side-effect-free), `Recorder{active, port_failed, inner}`, `Birth{seq, facts, pin, killable}`, start/stop/copy, listener thread (500 ms next, pin kill=true → Err → re-pin kill=false chain-only, dedupe (pid,create), BIRTH_MAX 512 full, per-round catch_unwind, failure mode unwatch/drain-only, exits on is_gone/retiring, start keeps port Arc clone), port acquisition OnceLock + port_failed; tests = §5 탄생 기록 · 듣는 스레드 실패 모드 · 포트 확보 (fake Births/Pinned/group).
- ②c1 gate state machine: GateCell/GateState/Episode/Mark, interrupt-line fn two sections + opener drop guard, snapshot (PIN_MAX 256, Failed causes), note_written(gen, W) with W taken by the on_written callback (member_pids once, no locks), decoder hooks open_turn/deliver/close_turn; worker spawn stubbed behind a seam; tests = §5 상태기계 · 두 구간 · 쓰기 확인 · 스냅숏.
- ②c2 worker + pass: next_step, Ticket, run_pass rounds (PASS_ROUNDS 3, one 200 ms confirm deadline per round, seq order, refused commit ends pass), commit under gate lock (still_due + exactly one terminate_raw), panic guard, logs §3-8 (callback panic = error per convention); tests = §5 판의 차례 · 종료 표식 … · 끝내기 시간.
- Reuse from backup `refs/backup/t40-clock-design-20260930:crates/engram-dashboard-agent/src/backend/claude/leftover.rs` per TRD §6 「백업에서」 keep/drop list.
- Carry notes above (release panic=abort → panic-free by construction; pin Err → re-pin; start keeps Arc; is_gone).
- Step ④: TRD §3-4 「런처가 끼면 T = claude.exe → not_hook」 holds only if claude exited; with claude alive the walk gives AncestorAlive{claude} (both misses — safe). Fix wording in ④. Also ANCESTOR_MAX counts A₁…Aₖ₋₁ ≤ 16; is_hook_command rejects CR/LF in S; taskkill_names exact `/PID n /T /F` only.
- ②a done (leftover.rs pure rules, 27 tests). Snapshot refs/backup/t40-step2-a.
- ②b done (Recorder, BirthWatch, listener, is_gone). Snapshot refs/backup/t40-step2-b. ensure takes &Arc<ProcessGroup>; after listener ends, active=0 → episode sees active≠rec → Failed(port). Private trait Group in leftover.rs = test seam. is_gone uses strong_count (no upgrade).
- ②c1 done (GateCell/GateState, interrupt two sections + opener guard, snapshot, W, decoder hooks, worker, run_pass STUB, precheck/commit_check/note_kill). Snapshot refs/backup/t40-step2-c1. Deviations to review: on_written also on continuing interrupts (TRD-consistent); section ① bumps last_mono on continuing interrupt; unlisted first write doesn't latch; generics Cleaner<G>/GateCell<G>; worker panic = error.
- OPEN: step ① test `a_dead_parent_held_open_is_off_the_list_and_reads_dead` flaked again under parallel load ("산 C 가 명단에 없다: []" — member list empty while child alive). Same as the earlier unexplained symptom 2. Investigate after ②c2 (could be member_pids returning [] spuriously — production impact: W/snapshot fail → safe miss, but must be understood).

## USER DECISION 2026-10-01 (step ② review)
「일단 재현 가능한것만 구현하자. 좀 현실적인 사항으로 단순화」 — fix only review findings that are reproducible / realistic; theoretical paths → record as known limits (ADR/TRD §8 in step ④), no code.
Recorded as known limits (codex ② BLOCKs, not fixed):
- K1 `ancestors` treats any root child as claude (codex A #1) — unrealistic: root `cmd /c` spawns only claude (+conhost); snapshot Failed(shape) if >2 root children at Esc.
- K2 listener exits when unwatch fails on retiring / second next error in drain-only (codex A #2) — detach failure never observed; on retiring the Job is about to end.
- K3 Busy (crossed) Esc line has no on_written → pass may fire < 3 s after that later line (codex B #1) — window = opener's setup time (ms), needs two Esc within it; kill still requires all rules.
- Rejected (not a defect): record-lock allocation (codex A #3) — TRD §3-3 allows allocation under the record lock; blind contract was mis-worded (corrected).
- Test flake root cause: test race (parent killed while ping child in its first ms → child dies 0xC000010A, job truly empty); test fixed with a readiness file; no production impact.
- ② review result: doc-aware FIX (no invariant violation; all coder deviations accepted), kill-safety PASS, codex A/B BLOCK → K1–K3 known limits by user decision. Fix round = trivial only: (a) stale header comment leftover.rs:22, (b) end_pass return value unused → drop it (finish_pass second tuple elt), (c) precheck treats recorder.port_failed() as Snapshot(Port).
- Known limits to record in ④ (TRD §8 / ADR): K4 equal creation tick passes parent rule (ADR-0257 d9 "같은 눈금 포함"; TRD §3-7/§8⑰ⓑ claim strictly younger — fix wording); K5 episode with no W keeps snapshot/record handles until turn end (~300 s in hang; bounded per incarnation); K6 port dropped while attached on drain-exit / retiring-detach-fail (debug log); K7 two guards rely on panic=abort (OnceLock in acquire, WorkerGuard after Exit); K8 episode-open log under-reports re-report count; facts copied 3× per birth; random test never commits / no panic op.
- Deferred to ③: TRD §5 real-process tests 12 (R/B/P → not_hook_copy, no_new_member, parent_alive) and 13 (#[ignore] MSYS loop/pipe).
- K9 (③ codex FIX, not fixed — user rule): section ① bumps last_mono on each continuing Esc before its line is written → repeated unwritten Esc could keep postponing the pass. Unrealistic: chat UI ignores Esc while 「중단하는 중…」 (FE-1.4); only delays, never wrong kill.
- ③ review: Claude PASS (notes N1 test 13 skipped by decision; N2 first-Esc work on daemon connection task — G1 measure; N3 Esc CONFLICT if turn closes between sections — record in ADR-0238 amendment; N7 interrupt_line doc overstates (Busy/stale lines have no episode) and N8 move_gate doc wording — fix in ④), codex FIX → K9 known limit. Snapshot refs/backup/t40-step3.
- ③ committed. G1 real-app: 35 Esc@200ms trials, 11 hangs → 11/11 released at 3.03–3.09 s, follow-ups 11/11, G2 ok (also cleaned wiki-preconsult plugin hook copies — any hook), controls 0 warns, G7 ok. w_us 6–26 µs; write→holder birth 320–504 ms; terminate_max_us 131–195; births 40–75. Logs: %TEMP%\t40-impl\g1\. G1b, G3–G6 not run.
- Side observation (pre-existing, not T-40): first turn of a fresh agent — Esc during UserPromptSubmit hook does nothing (turn gate opens only on `started`/progress). Record in tracking/ADR note in ④.
