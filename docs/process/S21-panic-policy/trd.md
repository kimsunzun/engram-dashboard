# TRD — Panic policy implementation: narrow containment · immediate-exit hook · unwind switch (S21)

> **Status: edition 11, revision 4 (2026-10-12 · user decision A) · implementation U1~U5 landed (2026-10-12 — commits are in the step-log).** All user decisions done (§3) · `/review trd full` round 3 done (1 = FIX · FIX → 2 = blind PASS · doc-aware FIX → 3 = doc-aware PASS · the last minor item applied). Revision 2 (review 1) = removed `usage.get` from containment (the allowlist has two entries) · the mark-off primitive and its four places · the invariant 「표시 범위 ⊆ catch 클로저」 · U3 things to verify. Revision 3 (review 2) = added the condition 「효과 없음」 to the allowlist · added Idle coalescing to the things that do not turn the mark off. Revision 4 = on a pump panic, dropped the explicit kill and use the existing cleanup path + a test (user decision A — §4-2).
> **Withdrawn — ADR-0293 (2026-10-12):** the command-side containment (§4-2 B1 — the two allowlisted read commands) was withdrawn. The only contained places are the four on the agent side (A1~A4); the bus command execution span is not contained. The allowlist · B1 · the command's `INTERNAL` fold · 「읽기 명령 둘」 · 「다섯 자리」 wording in this document is history (§0 ① · §1 · §3 (마) · §4-2 · §4-3 · §4-6 ① · ② · §5 U2 · §7 · §8 — to find them: `rg "허용 목록|B1|INTERNAL|다섯" docs/process/S21-panic-policy/trd.md`).
> **History:** edition 10 = `git show a722a879:docs/process/S21-panic-policy/trd.md` (shutdown tail · watchdog thread · decisions (다)(바)(바′)(라′) · the compile guard is void under the new direction) · edition 11 draft = `bedda3f3` ((가) lock poisoning · (나) multi-step spans · request-side entries R1~R7 — withdrawn by user decision ②).
> **Frame:** ADR-0288 · ADR-0290 + user decisions 2026-10-10 · 2026-10-11~12 · 2026-10-12 ② (§1). ★The three ADRs (0288 ~ 0290) exist only on branch `origin/v0.3.3/feat/panic-policy`★ — bring that branch in before committing this TRD, or the links will not resolve. The implementation ADR = ADR-0292 (planned).
> **Replaces the old record:** `docs/process/S20-command-bus/trd.md` §4 ⑨ 「릴리즈 `panic = "abort"` 유지」. The convention of that section (a command handler fails by value) stays.
> **Notation:** measured = tree `743530fb` (the code is the same as `2c25cdd`, which edition 10 measured — the commits in between are docs only). `crates/engram-dashboard-` in paths is shortened to `agent/` · `daemon/` · `base/` and so on. 「표시」 = the 「가두는 중」 mark (§4-3). 「가안」 = name · shape not final. Where no confidence label is given, it is certain.

---

## 0. Conclusion

```
① 가두는 것은 둘뿐이다 — 에이전트 하나만 섬기는 펌프 셋 · 라이터, 그리고 허용 목록의 읽기 명령 둘
   (agent.list · agent.listQueuedInputs)의 본문. 가둔 패닉 = 그 에이전트 Failed(CLI 는 기존 정리 경로로 끝난다) · 그 명령 INTERNAL 답.
② 그 밖의 패닉 = 사유를 로그하고 곧바로 abort — 오늘 배포판과 같은 결과. 가둔 코드 안에서도 여러 에이전트가
   함께 쓰는 상태를 고치는 호출은 표시를 꺼서 같은 길로 보낸다.
③ 둘을 가르는 것은 스레드 지역 「표시」 하나이고, 표시는 늘 그것을 잡는 catch 클로저 안에서만 선다.
④ 순서 = 원시 → 가두는 자리 → 훅 → 확인 · 감사 → 프로필 한 줄(§5).
   앞 단위는 abort 아래 아무 일도 안 한다 — 어디서 멈춰도 배포판은 오늘과 같고, 문제가 나면 마지막 한 줄만 되돌린다.
```

---

## 1. The settled frame

| Source | Content | This TRD |
|---|---|---|
| ADR-0288 decisions 1 · 2 | A panic in one request · one agent fails only that one | **Narrowed** (user decision ②) — requests: two read commands · agents: pumps and writers only (§4-2) |
| ADR-0288 decision 3 · ADR-0290 decision 2 | Shared-state panic · uncontained panic = the hook pulls the existing normal-shutdown path | **Revised** — log the reason → exit immediately (§4-4) |
| ADR-0288 decision 4 | Rust abort (double panic etc.) = just dies · agents are cleaned up by the Job | Kept |
| User decision 2026-10-10 | unwind scope = the whole workspace (daemon · shell · CLI) · the same hook in the shell | Kept — the shell gets the hook only (§4-5) |
| User decision 2026-10-11~12 | uncontained panic = exit immediately (「복잡하게 할 필요없음」) · TRD · implementation in large chunks | Kept |
| User decision ② 2026-10-12 | Containment is only for units that do not change state or are isolated — units that change state are not contained | This revision — the lock-poisoning policy · the multi-step span device are no longer needed |

- **Basis (`docs/research/panic-practice-peers-2026-10-12.md`):** 「가두지 못한 패닉 = 끝」 is a common shape (tikv · materialize · zed · zellij — conclusion 1). The contained range is usually narrow — rust-analyzer contains only read-only requests and leaves state-changing handling 「isn't guarded by `catch_unwind`」 (its repo `crates/rust-analyzer/src/handlers/dispatch.rs:34-44` — conclusion 2). 「표시 + 표시 밖이면 훅이 끝냄」 has a precedent in materialize — a per-thread counter `CATCHING_UNWIND` · abort if outside it (`src/ore/src/panic.rs` — conclusion 3). If the state-changing side is not contained, no lock-poisoning rule is needed (conclusion 4 · rust-analyzer's solution).
- **Requirements that went away with it:** the statement in the 「영향」 of ADR-0290, 「정상 종료 경로는 오염된 공유 락에서도 끝까지 가야 한다」, existed because the panic exit took that path. The panic no longer takes that path — poisoning tolerance of StopDaemon itself is out of scope (§7).

---

## 2. Current state (measured)

- **Profile:** `Cargo.toml:39-44` `[profile.release]` — `panic = "abort"` (`:40`). It is the only one in the workspace, so the daemon · shell · CLI change together. The debug · test builds are already unwind.
- **Hook:** only the daemon has one — `daemon/src/lib.rs:129-151` (log + previous hook) · installed at `:458` (after logging init `:453`) · the only caller is production `daemon/src/main.rs:13`, so the test server has no hook. The shell and CLI have none. The std hook runs **on the panicking thread, before unwinding**.
- **Log self-deadlock (known, unfixed):** if a thread panics while holding the log file Mutex, the hook's log waits on itself — `base/src/logging/mod.rs:110-115`.
- **Containment that already exists:** three pumps (`agent/src/transport/pty.rs:298` · `stdio.rs:241` · `agent/src/backend/codex/transport.rs:4018`) · the writer (`agent/src/transport/input_queue.rs:288`) · the bus command body `guard_panic` (`command/src/route.rs:20` — it wraps every blocking handler `command/src/table.rs:339`). The deployed build is abort, so none of them catches anything now. The pump's end (`finish`) runs **outside** the catch (`pty.rs:342` · `stdio.rs:308` · `codex/transport.rs:4021`).
- **Daemon command bodies run synchronously:** the daemon's own commands (`agent.*` · `usage.*`), wherever the entry is (bus stage-1 blocking `daemon/src/command_delivery.rs:1361` · `/control/agent` · `/control/call`), pass through `call_daemon_command` (`daemon/src/control/commands.rs:163`), and the `drive_to_completion` inside it (`:250`) finishes the future with `now_or_never` **on the calling thread** — `guard_panic` runs synchronously on that blocking thread.
- **`Read` declaration versus reality:** the daemon has three `Read` commands (`agent/src/commands.rs:212` `agent.list` · `:297` `agent.listQueuedInputs` · `:344` `usage.get`). But `usage.get` starts a lookup (`:334-337`) — the usage book sets `in_flight` (`daemon/src/usage_service/book.rs:279-282`), attaches joiners, emits to all subscribers, and starts a lookup thread · CLI (`daemon/src/usage_service/mod.rs:341-386`). The effect value is already the contract that decides 「id 를 붙들어 재실행을 막을까」 (`daemon/src/control/commands.rs:286-291`).
- **Job:** the pump's child is the root `cmd /c` and the real CLI is below it (ADR-0262 K1) — it is the Job that ends the CLI: the group owner held by the channel (`agent/src/transport/pty.rs:62` · `stdio.rs:88` · `codex/transport.rs:1399`) creates the Job with `KILL_ON_JOB_CLOSE` (`platform/src/group/windows.rs:153`) and its drop closes the Job (`:601`). The reaper only releases the session `Arc` (`agent/src/reaper.rs:65-70` — other holders exist only for the brief takeover decision, ≤ ~100ms).
- **Record of abort assumptions:** 57 lines of code · 30 files + seven ADRs + several TRDs — the list and the regex are in edition 10 §2-8.
- **CLI:** single-threaded (no threads · tokio in `cli/src`). **Exit code:** no place in production code decides on a process exit code.

---

## 3. User decisions (2026-10-12 — all done)

> (가) · (나) were removed by user decision ②, and (라) the shell was settled by ② as S1 (hook only · shell panic = exit immediately) (§4-5). (마) · (사) were decided as recommended — the table · section below stay as their basis. For ending the CLI on a pump panic, the user decided between the reviewers' opposing positions (§4-2).

| Group | Decision | User's words |
|---|---|---|
| (마) How a contained agent panic is announced | **N2a current surface** — the screen shows `Failed` · input is rejected as before · details go to the log (§4-4 the hook logs regardless of the mark) | 「그냥 내부에서 대되 로그는 남겨지는건 맞지?」 |
| (사) Means of exit | **`std::process::abort`** | 「추천으로해」 |
| Ending the CLI on a pump panic | **A existing cleanup path** — no explicit kill · confirm with a test · if it fails, B | 「A안」 |

### (마) Announcing a contained agent panic

The bus command is already fixed — `INTERNAL` 「command handler panicked: …」 (ADR-0159). What is left is the agent side.

| Option | What the user · LLM sees | Cost |
|---|---|---|
| **N2a current surface (recommended)** | pump = that agent `Failed` (「pump panicked: …」) · writer = input rejection reason | 0 |
| N2b add a strip notice | the above + strip 「에이전트 X 가 내부 오류로 멈췄다」 | a new notice mechanism (the bundle ADR-0288 deferred) |

- Recommendation N2a — no new screen mechanism. An immediate exit looks the same as today's 「데몬 꺼짐」 (no reason is shown — a deferred mechanism).

### (사) Means of exit

| Option | Result | Cost |
|---|---|---|
| **abort — `std::process::abort` (recommended)** | The same end as today's deployed build (Windows exit code `0xC0000409` — the measured comment at `command/src/testing.rs:25-27`) | zero new code · std only · looks the same before and after the switch |
| own `TerminateProcess` (code 101) | Ends without DLL-detach notifications · the exit code can be chosen | one platform helper (OS code — ADR-0266) · injected into the hook · the deployed build's exit code changes to 101 |
| `process::exit(101)` (not recommended) | — | `ExitProcess` ends the other threads first and then runs the DLL-detach notifications, where it may hang (MS docs order · likely · not measured) |

- Whether Windows Error Reporting (WER) reacts differently to the two options is unknown (uncertain) — the abort side is the same as today.

---

## 4. Design

### 4-1. Containment criteria

Contain only when all four hold. If any one is missing, do not contain → exit immediately.

1. It is the **outermost entry** of that unit of work.
2. There is a **way to report the failure** (`Failed` state · error reply — (마)).
3. The unit **does not change state, or changes only its own state**. A unit that changes shared state is not contained (user decision ②) — a shared-state call inside a contained unit turns the mark off (§4-2 mark-off).
4. After that unit is discarded, **the rest is sound** (no dangling thread · surviving child · filling pipe is left).

### 4-2. Containment table

| # | Place | Unit | Failure notice | To do |
|---|---|---|---|---|
| A1 | PTY pump (`agent/src/transport/pty.rs:298` · thread `:269`) | one agent | `Failed` | replace the current `catch_unwind` with `contain` |
| A2 | stdio pump (`agent/src/transport/stdio.rs:241`) | one agent | `Failed` | same |
| A3 | codex reader pump (`agent/src/backend/codex/transport.rs:4018` · thread `:4017`) | one agent | `Failed` | same |
| A4 | writer `input_queue::drain` (`agent/src/transport/input_queue.rs:288` — called by the PTY · stdio writer threads `pty.rs:208` · `stdio.rs:208`) | that agent's input | queue closed · input rejection reason (`:273-279`) | `catch_unwind` → `contain` |
| B1 **withdrawn — ADR-0293** | ~~allowlisted command body — `agent.list` · `agent.listQueuedInputs`~~ — withdrawn: the bus command execution span is not contained (the remaining cells of this row are history) | one command | `INTERNAL` | wrap the table call + body drive of `call_daemon_command` (`CommandTable::call` + `drive_to_completion` — `call_and_drive` in `daemon/src/control/commands.rs`) in `contain` — only when the name is in the allowlist · argument checks (`check_args` · `admit_input`) are outside the mark (U2 review round 1 widened it to include the table call) |

- **[Withdrawn — ADR-0293 · this item and the next are history]** **B1 = a name allowlist, not an effect predicate.** `usage.get` is declared `Read` but changes the shared usage book (§2) — if contained, a panic after the decision (`Start`) leaves `in_flight` set forever. If we split by `effect == Read`, any `Read` command added later would be contained silently. The list is one daemon-side constant, and a test pins that list (§5 U2). ★The condition for being on the list = the body has no effect★ — the thin-span fold below for B1 is `INTERNAL` (「확실히 실패」) because the two bodies change nothing. The stage-1 rule is that a panic after the body may have applied part of the work, so it is `OUTCOME_UNKNOWN` (`daemon/src/command_delivery.rs:1387-1391` · `fold_local` `:1401`). To put a command with effects on the list, decide that fold again first.
- **[Withdrawn — ADR-0293]** **Scope of B1:** the mark stands at the one place that calls the command body, so even when the same command arrives over control HTTP (`/control/agent` · `/control/call`), its body is contained in the same way. That HTTP handler itself · other commands · stage-2 delivery (`route` at `daemon/src/command_delivery.rs:1820`) are outside the mark. For a handler panic, the inner `guard_panic` answers `INTERNAL` first, and a panic in the thin outer span (the `now_or_never` machinery) is folded to the same `INTERNAL` by our `contain` — so there is no gap between the mark and the catching place. This is the third net on the stage-1 path, so U2 fixes the comment 「세 번째 그물을 세우지 않는다」 at `command_delivery.rs:1397`.
- **Ending the CLI on a pump panic (A1~A3) — no explicit kill:** the panic branch calls only `finish`. The CLI ends by the existing path — `finish` → `Failed` → the reaper removes the session → last `Arc` drop → channel drop → group owner drop → `KILL_ON_JOB_CLOSE` (§2). ★Reviewers disagreed (blind = explicit kill / doc-aware = the existing cleanup path is enough) → user decision A (2026-10-12): existing path · confirm with a test · B if it fails★. U4 measures it for each pump kind. **B (only if the test fails):** add platform `GroupRef::terminate` to kill the group in the panic branch, and remove that branch from ADR-0199's 「통로는 죽이지 않는다」 (`codex/transport.rs:118-125`).

**Mark-off (`uncontained`) — shared-state mutations called from contained code:** a panic inside = exit immediately.

| # | Place | What it reaches |
|---|---|---|
| a | the codex reader's first-turn port call (`codex/transport.rs:3671` — `witness_first_turn` `:3641`) | latch commit (`agent/src/manager.rs:589-612`) → `ProfileRegistry::commit_session_id` (`agent/src/profile.rs:853`) — the shared profile table |
| b | one place inside `OutputCore::report_usage` (`agent/src/output_core.rs:393-397` — called by the stdio · codex pumps `stdio.rs:262` · `:286` · `codex/transport.rs:3707` · `:3818`) | `UsageService::observe` (`daemon/src/usage_service/observe.rs:40-41` → `mod.rs:204`) — a book shared per account |
| c | what the writer calls after a chunk (`input_queue.rs:298-309` — inside that inner catch) | claude leftover cleanup `written` (`agent/src/backend/claude/leftover.rs:1857-1872`) — it modifies the door `GateCell` |
| d | the claude decoder's door open/close (`agent/src/backend/claude/mod.rs:1200-1204` — inside A2) | `open_turn` · `deliver` · `close_turn` of `GateCell` (`leftover.rs:1927-1956`) |

- Reason for c · d: leftover cleanup (ADR-0262) has two guards that rely on abort (K7 of that ADR) — containing would break that premise. The inner catch of c becomes unreachable in production (it stays alive in the test build, which has no hook).
- **What does not turn the mark off:** the places shared by several agents that the pump's `emit` calls — the turn observation table · the pending list table (`agent/src/turn.rs:94` · `agent/src/inputs_pending.rs:21` — `output_core.rs:432`) · the Idle coalescing that the turn-end · list-clear notices reach (`IdleCoalescer::claim` `daemon/src/messaging_host.rs:384-389`) — have no panic source, because the critical section is a plain `HashMap` operation on that one agent's entry (review confirmed).

**What is not contained (= exit immediately):** WS command handler bodies · detached command tasks (`daemon/src/agent_conn.rs:314` · `:375` · `:336` — by default they are assumed to change state · if a read-only WS command is confirmed, add it then) · activation (`daemon/src/connection_core.rs:1366` · `:1760`) · bus commands outside the allowlist (including `usage.get`) · control HTTP · MCP tools (`daemon/src/control/mcp_server.rs`) · the codex writer and its session-id recording (`record_session_id` `codex/transport.rs:2393` · its catch `:2420` — `writer_loop` `:2484` calls it at `:2521` · a different path from a above) · PTY watch · stderr drain · claude leftover cleanup thread · messaging delivery · reaper · sweep · session tracking · usage · accept loop · main future · net · rmcp · hyper internals · the whole shell. The existing `catch_unwind` at these places (reaper `agent/src/reaper.rs:208` etc.) becomes code that production never reaches, but the test build has no hook and it stays alive, so it is not removed (§4-6 ②). ★Exception — the catch of the daemon sweep (`daemon/src/lib.rs` 6.6) is not reached even in the test build, so it was removed★: that task is started only by `run()`, which installs the hook first, and the test server does not start it (U4-C review confirmed · user decision 2026-10-12).

### 4-3. The mark — new base resident `panic` (tentative)

- One thread-local depth (const `Cell` · `try_with` — readable even while the thread is being torn down · if it cannot be read, 「표시 없음」).
- Primitives (tentative names): `contain(f) -> Result<T, Caught>` (raises the depth and catches in one body) · `uncontained(f)` (depth 0 inside it · restored when it ends) · payload → sentence (seven copies now — edition 10 §2-2).
- ★**Invariant — mark range ⊆ the catch closure that catches it.**★ No separate depth guard stands at the thread head. The pump's `finish` is outside the catch (§2), so it runs without the mark — if `finish` panics while marked, nothing can catch it, so that agent stays `Running` with no end transition and the CLI stays alive (ADR-0005 · ADR-0290).
- **No async mark** — every containment place is synchronous (A1~A4 = dedicated threads · B1 = a blocking thread). If an async place is contained later, a thread-local mark is wrong — then add materialize's task-local mark (`CATCHING_UNWIND_ASYNC` · its repo `src/ore/src/future.rs`).
- **The mark does not follow threads** — a thread · task started by a contained unit is outside the mark (= exit immediately · the safe side).
- **Why the mark is needed** — the hook runs before `catch_unwind` (§2). If we install an 「잡히지 않은 패닉 = 종료」 hook without the mark, it ends even the panics we meant to contain.
- Residency conditions (ADR-0269): consumers agent · daemon · shell + payload copy · zero domain knowledge · references no other resident — add `panic` to the gate ③ regex (CLAUDE.md 「빌드·검증 명령」 · copy in `ci.yml`). The command crate cannot call base (zero dependency), so the mark for B1 is set up by the caller (the daemon).

### 4-4. The hook — the daemon · shell install the same one

```
패닉 → 훅(패닉한 스레드 · 되감기 전)
  ① 사유 줄을 로그한다(스레드 · 문장 · 자리 — 지금 데몬 훅의 줄) · 이전 훅(std 기본 — debug 의 stderr)을 부른다
  ② 표시가 섰으면 돌아간다 → 되감기가 그 contain 까지 가서 실패로 접힌다((마))
  ③ 표시가 없으면 std::process::abort((사))
```

- The hook body lives in base `panic` (tentative) and takes the exit as an argument (so tests do not kill the process). Installed at the daemon's current place (`daemon/src/lib.rs:458`) · at the head of the shell's `run()` (`src-tauri/src/lib.rs:28` · before `Builder` — shell logging init comes after that at `src-tauri/src/state/boot_plugin.rs:89`, so a panic before it ends without a reason line).
- **The hook logs regardless of the mark** — the branch outside the mark logs anyway, so the self-deadlock (§2) must be closed on the log side, and once it is closed there is no reason to stay quiet inside the mark. The panic location (file · line) is visible only in the hook, so places that catch and do not log, such as `guard_panic`, are also recorded this way. A containment place does not log the panic again and only produces the failure surface.
- **The self-deadlock is closed on the log side** — put a thread-local 「이 스레드가 파일에 쓰는 중」 flag in the base `logging` file sink, and drop that line when it is set. If another thread stopped while holding the file lock, only the panicking thread repeats a short `try_lock` (cap 100 ms) and then drops that line (U1 review round 1 — normal writes still wait as before). The rest is in §7.

### 4-5. Shell · CLI · later stages

- **Shell = hook only (settled by user decision ②).** Any shell panic → log the reason → shell exits immediately — the same as today. The daemon · agents continue · the next shell launch asks about abnormal-exit restore. The shell's `Read` commands (`src-tauri/src/layout/commands.rs`) are not contained either (shell bus entry `src-tauri/src/daemon_client/inbound.rs:217`). ★The hook must exist for it to be the same as today★ — if we switch to unwind without the hook, a background-thread panic silently kills only that thread.
- **CLI = no hook.** It is single-threaded, so under unwind a main panic is the end of the process (exit code 101 · default hook stderr) — outwardly the same as today's abort (likely).
- The next stage attaches the transport crate (`crates/engram-dashboard-transport`) to the shell connection layer and brings in new connection · pump tasks — those tasks follow the §4-1 criteria when they arrive (if on the shell side, they are not contained).

### 4-6. Things to verify · audit (not decisions — U4)

1. **[Withdrawn — ADR-0293 · withdrawn together with B1 — the U4-C audit result (neither body changes state) remains as that ADR's basis]** **Do the bodies of the two allowlisted commands really not change state** — look once. The declaration is a contract and the code cannot catch a mismatch (`daemon/src/control/commands.rs:286-291`).
2. **Audit of the abort assumption** — for code outside the mark, 「패닉 = 프로세스 끝」 stays true (the hook ends it). The assumption breaks only for the bodies of the five containment places and the `Drop`s that run during their unwinding. To look at:
   - base `sync` contract 3 (`base/src/sync.rs:10-11` — 「운영 빌드는 오염에 닿지 않는다」) — the pump regains locks through `sync` (`pty.rs:322` etc.), so rewrite it.
   - **The poisoning path of the output core** — `emit` writes while holding `replay` (`agent/src/output_core.rs:422-429` · the `.expect("한 줄 이상")` of the list event `:614` is also inside it), and a panic there poisons `replay` → the `replay.lock().expect` (`:692`) in `finish`, which runs without the mark, panics again and the daemon ends (the safe side).
   - **Comments that become wrong** (fixed in U4-C `c9d15023` — the line numbers of this item are from before that) — the 「(b) 그 재-panic 은 연결 task 안이라 tokio 가 그 task 만 격리」 at `pty.rs:283-297` becomes false once the hook arrives. The 「release=abort」 regex does not catch it, so it is listed separately. Same kind — the 「즉 자식 프로세스는 이미 죽어 있다」 at `agent/src/reaper.rs:65-70` is false on the pump-panic path under user decision A (that path reaches the reaper with the CLI alive, and releasing the session closes the Job and ends it) — rewrite it in U4.
   - Comments · log sentences that literally write 「release=abort」 (e.g. `daemon/src/messaging_host.rs:769` — now `:770`) are rewritten to 「표시 밖 패닉 = 즉시 종료」 (fixed in U4-C `c9d15023`).
   - ADR-0262 K7 · the rollback `Drop` of the messaging kernel runs in uncontained code (or mark-off c · d), so the assumption stays true — out of scope.

### 4-7. Profile switch

Delete the one line `panic = "abort"` at `Cargo.toml:40` (the default is unwind). Since all earlier units do nothing under abort, the rollback = this one line. So no compile guard that 「abort 면 빌드를 멈추는」 is put in. The executable size change is measured in this unit (ADR-0288 「영향」 wrote that it was not measured). ★But if rolled back, the descriptions of contained places in the comments U4 rewrote (「실패로 접힌다」) become false again in the deployed build — to find them: `rg "ADR-0292" crates src-tauri/src`★ (U4-C review recommendation).

---

## 5. Units

| Unit | What changes | Deployed build (abort) | Verification |
|---|---|---|---|
| U1 primitives | base `panic` (`contain` · `uncontained` · payload sentence · hook body — nobody calls it) · `logging` re-entry flag · gate ③ name | same | base unit tests — depth increment/decrement · nesting · return after unwinding · depth 0 and return inside `uncontained` · hook decision (exit injected) · re-entry flag |
| U2 containment places | §4-2 — A1~A4 to `contain` · mark-off a~d · B1 + allowlist constant · the comment at `command_delivery.rs:1397` | same (the catch catches nothing and the panic branch is not reached) | the existing pump · writer panic tests stay green · `finish` runs without the mark · depth 0 inside a~d · **allowlist test** (the list = exactly two · both are in the table and `Read` · the mark stands inside the two bodies and not for other commands) |
| U3 hook | replace the daemon hook with the base hook · install in the shell | same (log → abort) | child-process test (§6). ★Things to verify★ — from here the debug daemon ends on panics that tokio used to isolate: the timer poll during shutdown (`daemon/src/usage_service/mod.rs:320-322`) · the `debug_assert` in a connection task (`daemon/src/agent_conn.rs:108`) etc. `/qa full` · `stop_smoke` may become shaky, so run those two first (the test server has no hook, so unit tests are unchanged) |
| U4 verify · audit | the two items of §4-6 — places to fix · comments · log sentences | same | **CLI cleanup real-process test** — for each pump kind (PTY · stdio · codex), after a contained pump panic, whether the whole CLI process tree (not only the root `cmd /c`) disappears within the time limit (§4-2 A — B if it fails) · an unwinding test for every place fixed |
| U5 switch | one line `Cargo.toml:40` | **unwind** | release build + `/qa full` (real window · real daemon work normally) · size |

- **Fixed order:** U2 before U3 (if reversed, a pump panic that is contained now in debug would end the daemon) · U4 before U5. The agent part and the daemon part of U2 touch different files, so they can run side by side.
- Stopping after any unit leaves the build · regression green and the deployed build the same as today (the 「배포판」 column above).

---

## 6. Test strategy

- **The hook is process-global, and the branch outside the mark ends the process** — so split into two.
  - **In-process:** call the hook body with an injected exit (U1). A test that actually swaps the global hook is serialized by one lock — in the shape of `with_quiet_panic_hook` at `command/src/testing.rs:8-30` (restore after catching, not in `Drop`).
  - **Child process:** the test binary re-launches itself with one environment variable and `--exact <시험 이름>`. The child installs the real hook and (a) panic outside the mark · panic inside `uncontained` → the parent sees 「성공 아님으로 끝남 + 로그 파일의 사유 줄」 (b) panic inside `contain` → sees 「계속 돎」. The repository has no precedent of this shape (a new shape). Put it in the daemon crate — that crate's test line already adds `--test-threads=4` (CLAUDE.md 「병렬은 테스트 바이너리마다」).
- Tests of the containment places run in debug — the test build is already unwind, so measure right from U2.
- **The deployed build's panic path itself cannot be measured by a cargo test** — test · bench targets ignore the profile's `panic` setting (likely — Cargo docs). After the switch to unwind it is the same code as debug, so the debug child-process test stands in for it, and U5 only measures normal release operation.

---

## 7. What remains · out of scope

**Value accepted by user decision ②:** a bug in a state-changing request · activation · `usage.get` = the daemon exits immediately (all agents end · the next start takes over). The same as the current deployed build.

**Residuals accepted:**

- **If the body of an allowlisted command is later changed to change state**, it is contained as is — the allowlist test pins only the names. The comment next to the list constant states that contract.
- **Poisoning chain of a contained pump** — a panic while holding the output core lock spreads to a daemon exit at `finish`, which runs without the mark (§4-6 ②). Accepted because it is the safe side.
- **Fan-out seq hole** — if a contained pump panics in the middle of `fan_out` (`agent/src/output_core.rs:351`), the remaining subscribers do not receive that seq → they go through the existing replay recovery path (that agent is `Failed` anyway).
- **Ending the CLI outside Windows (decision A)** — the group owner does nothing there, so the CLI is likely to stay alive after a contained pump panic (not measured — the U4 test `agent/tests/pump_panic_cli_cleanup.rs` measures only on Windows). The same kind is already written in the header of `agent/src/backend/codex/transport.rs`, 「비-Windows 에는 손자를 거두는 수단이 없다」. When to handle it is not decided.
- **Cases where the hook's log stops** — if the panicking thread holds a tracing internal lock (reload · `EnvFilter`), the hook stops (same as today's hook) · if another thread stops while holding the stdout layer or a tracing internal lock, the hook waits too (the file sink is closed by the cap in §4-4).
- **When log initialization fails entirely** — if the data folder is writable (if not, the daemon stops at startup stage 1 `ensure_data_dir_writable`) but neither its `logs` nor the `%TEMP%` fallback can be opened, a panic outside the mark ends the daemon without leaving a reason line in a file (only stdout · stderr — a daemon launched by the shell has nowhere to leave it). The deployed build is the same today (abort). Closing it needs a startup policy (e.g. do not start without a log place), but ★by user decision (2026-10-12) it is not put in★ — in that rare environment, a daemon that does not start at all does more harm than failing to leave a reason, and most data-folder problems are caught first by the shell's pre-start check (ADR-0135).
- **Rust abort branches** — double panic · a panic escaping a destructor · crossing FFI · shell WebView2 COM callbacks (unchecked) = ADR-0288 decision 4.
- **Missing mark · missing mark-off** — if a new containment place enters without the mark, it exits immediately (the safe side). If contained code newly changes shared state and the mark-off is forgotten, that half-changed state remains — review watches for it.
- **Restart loop on a deterministic panic during boot · the tray icon staying until the mouse hovers after an immediate shell exit (inference)** — the same as today's abort.

**When widening:** to contain a place that is not contained, re-check the four in §4-1. Trigger = that place actually appears in the log's reason lines. On the day a state-changing place is contained, the lock-poisoning policy · the multi-step span device (edition 11 draft (가)(나) — `git show bedda3f3:docs/process/S21-panic-policy/trd.md`) are needed again.

**Out of scope:**

- Poisoning tolerance of the normal shutdown (StopDaemon · tray 「데몬 끄기」) · shutdown time limit · T-52 — the panic does not take that path.
- The mechanisms ADR-0288 deferred — shell reason notice · panic indication on the next boot · shell auto-restart · agent PTY as a process outside the daemon (rejected).

---

## 8. ADR · document follow-ups

- **ADR-0292 (planned)** — use `/adr supersede --mode partial` to revise ADR-0288 decisions 1 · 2 (narrow containment — user decision ②) · decision 3 and ADR-0290 decision 2 (uncontained panic = log the reason → exit immediately · do not pull the normal shutdown path). Body = containment criteria (§4-1) · containment table · allowlist · mark-off · on a pump panic the CLI uses the existing cleanup path (§4-2 — decision A · B if the test fails) · mark invariant · hook shape · the three answers of §3 · replacement of the profile decision of S20 TRD §4 ⑨ · research basis (§1). Timing = before U1. Precondition = ADR-0288 ~ 0290 are on this branch.
- base `sync` header contract 3 · the base gate ③ line of CLAUDE.md 「빌드·검증 명령」 · one step-log line — in the same change as the relevant unit.
