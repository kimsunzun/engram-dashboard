# Interrupting claude during a hook hangs for about 300 seconds — on Windows a surviving hook child holds the output pipe (2026-09-29)

- **Status:** investigation complete (light · no adversarial review) · decision = user 2026-09-29 (§7) · before TRD · implementation · tracking = `docs/tracking.md` T-40. Main cross-checked the collector reports, and marked with ✓ only the lines main opened itself — a `파일:줄` or statement without the mark is the collector's reading only (upper bound: likely).
- **Date:** 2026-09-29
- **Version:** claude `2.1.283` — the symptom session and all experiment logs (`claude_code_version` in `system/init`). Git Bash = MSYS2 `msys-2.0.dll` 3.6.6.
- **Why:** in the previous session's F2 GUI measurement, Esc right after sending stayed stuck for about 300 seconds (`docs/process/step-log.md` S21 chat screen 「구현 마무리 · GUI 실측」 (implementation wrap-up · GUI measurement) line — observed once · handed off to the next session's discussion).
- **Method:** ① interrupts in a stream-json (`-p`) harness launched with our spawn arguments as they are, varying the hook combination (experiment 1) ② one pilot of the interactive TUI (experiment 2) ③ a harness that kills only the orphans (experiment 3) ④ upstream issue lookup ⑤ peer survey, 2 rounds (collector → main spot check).
- **Sources (session scratch — not committed · volatile):** `C:\Users\<user>\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\8717a9e4-353a-4963-8d10-3e638811be32\scratchpad\`
  - experiment 1 = `hookrepro\driver.cjs` · `matrix.cjs` · `logs\` · experiment 3 = `hookrepro\driver2.cjs` · `logs2\` · experiment 2 = `hookrepro-tty\tty.cjs` · `logs\`
  - round 2 readings of peers = `peers\` (7 clones) · `cca\` (claude-code-action clone) · `ext\` (VS Code extension bundle) · `opcode.rs` · copy of round 1's Zed adapter = `acp.ts`
  - round 1's t3code · vibe-kanban · paseo are local reference clones (`I:\Engram_Workspace\opensource\`) — their `파일:줄` is against that HEAD (`t3code 5378f87f9` · `vibe-kanban 78580443` · `paseo bbe3f17`), and the contents of the ✓ lines were re-matched while writing this record.
- **Confidence legend:** certain (repeated measurement · or primary source + main cross-check) · likely (single measurement · reading · an interpretation where two fit together) · uncertain (inference only). Each claim is written together with **measured / read / inferred**.

## 0. One-line conclusion

> ★**When claude on Windows receives an interrupt during a hook, it kills the hook with `taskkill /T /F`, but in that gap a child forked by Git Bash survives and holds the hook's output pipe for about 300 seconds, and the CLI waits with no time limit until that pipe closes — so in our mode (stream-json `-p`) the `result` of the interrupted turn comes about 300 seconds late.**★ Killing only that leftover brings the turn end within about 0.2–0.4 seconds, and claude stays alive and accepts the next turn (3/3). The dashboard will kill only that leftover inside its own Job (user 2026-09-29 · §7).

## 1. Symptom (measured · previous session's F2 GUI measurement)

- In chat (stream-json) mode, Esc about 0.2 seconds after sending → the interrupt response came, but the `result` came about 300 seconds later. That turn's `hook_cancelled` duration = 300461 ms · the hook = `handoff-trigger.sh` (a user-global plugin hook).
- Meanwhile Esc was ignored (during 「중단하는 중…」 (interrupting…) — ADR-0244) and the pending input stalled.
- Killing and reopening the agent recovered it, and the conversation was intact.
- In the same session, 4 other interrupts during a hook finished in 0.6–0.8 seconds — so it is a race, not every time (§2 ⑦).

## 2. Cause

| # | Claim | Basis | Confidence |
|---|---|---|---|
| ① | The CLI cancels a hook with `taskkill /PID <bash> /T /F` — **twice** (the second about 1.5 seconds later) | measured (experiment 1) | certain |
| ② | A Git Bash fork child created during or after the tree walk survives — parent dead · 2 threads waiting · CPU about 0.09 seconds · no children. About 300 seconds later it ends, leaving `bash.exe: *** fatal error in forked process - WFSO timed out after longjmp` | measured (experiment 1 · the shape of the orphan picked in experiment 3 is in §3-3) | certain (the shape that survives) · likely (creation time = during or after the tree walk) |
| ③ | The hook script was not blocked on its own line | measured (experiment 1) | certain |
| ④ | ★The CLI waits until **all** of the hook's stdout/stderr pipes are closed (the child `close` event), and there is no upper limit★ · the declared hook timeout timer is cleared once the top-level shell ends — so the declared timeout cannot cut this hang | experiment 3 (killing only the orphan gives `result` immediately) · experiment 1 B·C (waited until the surviving child ended) · the upstream #85250 collaborator's description (§5) fits together with them | likely |
| ⑤ | 300 seconds = the MSYS fork handshake wait limit | **inferred** — only the error text + the timing. The MSYS runtime source was not read and no external source states 300 seconds | uncertain |
| ⑥ | Even without a hang, the interrupt waits for the surviving hook child (`sleep 3` → about 3 seconds · `sleep 20` → about 20 seconds) | measured (experiment 1 B · C · B20 · C20) | certain |
| ⑦ | It is a race — it does not happen at every interrupt during a hook | **inferred** — experiment 1 E was 2 of 7 · the symptom session's other 4 interrupts took 0.6–0.8 seconds · E-late (interrupt after 3 seconds) had no hang | likely |
| ⑧ | The orphan itself is common to claude-on-Windows, and the visible 300-second turn hang shows up in the `-p` stream-json we use (not in the TUI) | measured n=1 (experiment 2) | uncertain (provisional) |

## 3. Experiments

### 3-1. Experiment 1 — non-interactive (stream-json `-p`)

**Setup:** the dashboard spawn arguments as they are + `--model haiku --include-hook-events`. The interrupt line = `{"type":"control_request","request_id":"interrupt:<uuid>","request":{"subtype":"interrupt"}}`.

| Case | Setup | Interrupt → `result` (ms) |
|---|---|---|
| A | no hook | 11 · 10 |
| B | read stdin with `cat` → `sleep 3` | 3237 · 3125 |
| C | `sleep 3` (does not read stdin) | 3028 · 3037 |
| B20 / C20 | `sleep 20` | 20109 / 20021 |
| D | `handoff-trigger.sh` alone | 11 runs · 6–365 (no hang) |
| **E** | real environment (4 `UserPromptSubmit` hooks) · interrupt 200 ms after sending | 300619 · 1407 · 492 · 502 · 548 · 300284 · 491 → ★**hung 2 of 7 times**★ |
| E-late | real environment · interrupt 3 seconds after sending | 7 · 10 |
| F / F4 | harmless fork-bundle hook (F = one · F4 = several) | 8 runs · no hang |

- The mechanism of the two hangs = §2 ①–④. The hang-free B · C series is ⑥.

### 3-2. Experiment 2 — interactive TUI (stopped after one pilot)

| Item | Observation |
|---|---|
| Setup | real environment · Esc 199 ms after Enter |
| Prompt return | 250 ms or less — the message is discarded and there is no transcript record |
| Following 「say ok」 | answered in 1.5 seconds |
| Orphan | same shape — the bash of `wiki-preconsult.sh` lived about 300 seconds and stderr had the fork error |

- **Interpretation (n=1 · provisional):** the orphan is common to claude-on-Windows, and the visible 300-second turn hang shows up in `-p` stream-json (the mode the dashboard uses) and not in the TUI. Esc during a hook before submission in the TUI may take a different code path.

### 3-3. Experiment 3 — killing only the orphan

**Setup:** detection = a `control_response` came but there is no `result` for 10 seconds. Picking = a process that is alive · is not claude · and whose recorded parent is dead. Kill = `taskkill /PID x /F` (no `/T`). ★**This picking (recording the PID parent) is not the adopted design — §4**★.

| Trial | Picked | Kill start → `result` | Follow-up turn |
|---|---|---|---|
| E r12 | bash `wiki-preconsult.sh` | 247 ms | ok · 2399 ms |
| E r20 | bash `handoff-trigger.sh` | 223 ms | ok · 2078 ms |
| Emcp r1 | 2 bash | about 420 ms | ok · 2069 ms |

- **Released 3/3** · `claude.exe` stayed alive and the same process accepted the next turn (measured · n=3).
- 18 runs with no hang: `result` 394–918 ms after the interrupt.
- MCP stdio servers (whose parent `claude.exe` is alive) were not picked.
- The shape of the picked orphan: `"C:\Program Files\Git\usr\bin\bash.exe" <hook>.sh` (a fork copy, so its command line equals the parent's) · parent bash dead · all 5 threads waiting · CPU 16–31 ms.

## 4. ★Incident record★ — experiment cleanup killed other people's processes

- **What happened (measured):** the experiment worker's final cleanup picked processes by **PID-parent link** and force-killed them, and among them were processes unrelated to us — MSVC `link.exe` (pid 16924 · about 05:46:29Z) · `vctip.exe` (pid 10432 · about 05:40:47Z) (most likely a build in another worktree) · 13 `sleep.exe` whose owner is unknown · 2 `conhost.exe` · 2 `bash.exe`.
- **The picking itself was not safe either (measured):** even with a 「부모보다 뒤에 만들어졌나」 (was it created after the parent) check added, the PID-parent link picked other people's processes at other moments because of **PID reuse** — 4 of 21 pre-turn dry runs · 3 of 18 at the `result` moment.
- ★**Rule — 「PID 부모만으로 고르지 말 것 — Job 명단 + 생성 시각」**★ (do not pick by PID parent alone — Job list + creation time): on Windows, do not pick processes by parent PID alone. Use the membership the kernel tracks (the Job Object list) and each process's creation time. The picking in §7 stands on this rule.

## 5. Upstream issues (existence confirmed 2026-09-29)

| Issue | Status · date | Content |
|---|---|---|
| [anthropics/claude-code#96476](https://github.com/anthropics/claude-code/issues/96476) | open · 2026-09-23 | A hook timeout kills only the Git Bash launcher, and the orphaned grandchild holds the stdout pipe so the session hangs · killing the orphan released it immediately · there is a comment about 2.1.283 |
| [anthropics/claude-code#96945](https://github.com/anthropics/claude-code/issues/96945) | open · 2026-09-25 | While an orphaned descendant holds the hook stdout, the tool call is not released · it happens with a bare Python hook too (not only a bash problem) |
| [anthropics/claude-code#77078](https://github.com/anthropics/claude-code/issues/77078) | open | Hook processes are left SUSPENDED · a user watchdog (ResumeThread · killing stale hook processes at CPU 0) cut a 13–61 minute hang to 12–16 seconds |
| [anthropics/claude-code#67888](https://github.com/anthropics/claude-code/issues/67888) | closed | claude's own cleanup `taskkill /PID x /T /F` itself hangs and piles up |
| [anthropics/claude-code#85250](https://github.com/anthropics/claude-code/issues/85250) | open · label reproduced | Anthropic collaborator (2026-08-25): 「completion of the hook batch waits on the hook's output pipe reaching EOF rather than on the timeout」 · the declared timeout should end the batch. ★No fix through CHANGELOG 2.1.284★ |
| MSYS/Cygwin 「WFSO timed out after longjmp」 | — | Old cygwin mailing list reports (external kill during fork · long hang). ★**There is no source for the 300 seconds**★ |

## 6. How peers handle it — when no `result` comes after an interrupt

★**No peer keeps claude alive and kills only the descendants** — only the user watchdog (#77078) does that.★

### 6-1. Round 1 (✓ = main matched the line directly)

| Peer | Handling | Basis |
|---|---|---|
| Zed `claude-agent-acp` | After a 30-second grace it closes the turn locally as 「cancelled」 · does not kill · logs 「a new session may be required」 | `acp-agent.ts:354` ✓ (`DEFAULT_FORCE_CANCEL_GRACE_MS = 30_000`) · `:6868` ✓ · triggering issue #680 |
| t3code (`pingdotgg/t3code`) | `interruptTurn` → `stopSessionInternal` → `query.close()` · comment 「interrupt() can acknowledge while resumed background tasks keep the CLI alive」 ✓ · commit `4d12e5222` (#5891 · 2026-08-23). Next message = the reactor that saw the stuck session calls `startProviderSession` → `ProviderService` does `--resume` with the persisted `resumeCursor` | `apps/server/src/provider/Layers/ClaudeAdapter.ts:5289-5296` ✓ · `apps/server/src/orchestration/Layers/ProviderCommandReactor.ts:815` ✓ · `apps/server/src/provider/Layers/ProviderService.ts:1459` ✓ |
| vibe-kanban (`BloopAI/vibe-kanban`) | `control_request` interrupt · container stop waits 5 seconds and then 「Graceful shutdown timed out … force killing」 → `kill_process_group` (`command_group` · a Job Object on Windows — from crate docs only, the source was not read) · the README announces the service sunsetting | `crates/local-deployment/src/container.rs:1438` ✓ |
| paseo (`getpaseo/paseo`) | optimistic `turn_canceled` · interrupt with a 3-second limit · a late result is discarded (「Suppressing stale non-success result…」) · tree kill (`tree-kill` = `taskkill /T /F` on Windows) | `packages/server/src/server/agent/providers/claude/agent.ts:3790` ✓ |
| opcode | only kills (`taskkill /F` · no `/T`) | collector reading |
| Official Python SDK | only a 60-second interrupt-response limit · `close()` escalates terminate → kill at 5-second intervals · direct child only | collector reading |
| Official TS SDK | no upper bound in the docs · open issues [#425](https://github.com/anthropics/claude-agent-sdk-typescript/issues/425) · [#429](https://github.com/anthropics/claude-agent-sdk-typescript/issues/429) · [#444](https://github.com/anthropics/claude-agent-sdk-typescript/issues/444) · [#352](https://github.com/anthropics/claude-agent-sdk-typescript/issues/352) · [#279](https://github.com/anthropics/claude-agent-sdk-typescript/issues/279) | `anthropics/claude-agent-sdk-typescript` |

### 6-2. Round 2 (collector reading except ✓)

| Peer | Handling | Basis |
|---|---|---|
| Official VS Code extension 2.1.284 | `interruptClaude` = `query.interrupt()` · no timer · on close, SIGKILL only the `claude.exe` pid (win32 2 seconds + 5 seconds) · `taskkill` 0 times | bundle reading · `taskkill` occurrence count 0 ✓ |
| claude-code-action (`anthropics/claude-code-action`) | does not interrupt · relies on the job's `timeout-minutes` | `base-action/src/run-claude-sdk.ts:190-210` |
| Claude Desktop Code | 15-minute idle recycle → `taskkill /T /F` | third-party reverse engineering · #68625 |
| claudecodeui (`siteboon/claudecodeui`) | closes locally | — |
| happy (`slopus/happy`) | abort = kill + resume | — |
| sculptor (`imbue-ai/sculptor`) | interrupt → 5 seconds → process-group SIGTERM → 2 seconds → SIGKILL → 2 seconds (POSIX only) | — |
| emdash (`generalaction/emdash`) | goes through `claude-agent-acp` (30 seconds) | — |
| crystal (`stravu/crystal`) | `taskkill /F /T` + a PPID sweep after exit — misses orphans whose parent is dead | — |
| agentapi (`coder/agentapi`) · claude-squad (`smtg-ai/claude-squad`) · JetBrains | interactive PTY | — |
| Cline v3.40 | execa timeout 10 minutes | — |

## 7. Decision (user · 2026-09-29)

- **Judgment:** it is a general defect of claude but shows up in our mode → the dashboard mitigates it.
- **Adopted:** kill only the stuck leftovers inside our Job Object and keep claude alive (not a restart).
- **Picking:**
  - Record the time at the Esc that actually sent an interrupt — a repeated Esc during 「중단하는 중…」 (interrupting…) is ignored (ADR-0244), so it is not recorded, and a later Esc for a new turn overwrites it · no separate reset is needed.
  - After N seconds, **if that interrupt's turn end has not come and N or more seconds have passed since the most recent Esc**: kill a Job member that is not claude · is not a direct child of claude · was created after the recorded time · and is still alive.
  - Whether it is an orphan · whether it is 「bash」 · the CPU state are not the main criteria — there are legitimate orphans (e.g. a dev server), and an idle process looks frozen.
- **Clock:** compare the kernel's creation time with a timestamp taken from the same clock (system time).
- **N = 3 seconds:** normal was measured at 1.4 seconds or less · even a wrong firing only hits processes created after the Esc. Keep the constant in one place and verify it hard in measurement.
- **OS part:** keep the Windows-only part small (the Job member list + ending one) · no-op on other OSes. The user plans to add a separate platform module later (still at the concept stage), so write it so that the OS part can be moved there.
- **Schedule:** put it into this branch `v0.3.3/feat/chat-ux` before the master merge → final real test → merge.
- **To ask in the TRD (open):** ① whether to notify on screen when a cleanup happens ② whether to add a fallback (restart) if the turn end still does not come.

## 8. Limits

- A light investigation · no adversarial review. A `파일:줄` or statement without ✓ is the collector's reading only (upper bound: likely).
- ★The source of the 300 seconds is unknown★ — the MSYS runtime source was not read and there is no external source (§2 ⑤).
- Experiment 2 (TUI) stopped after one pilot — every statement on the TUI side is provisional.
- Experiment 3 has 3 runs — the release is 3/3 but the sample is small. That experiment's picking is not the adopted design (§4), and the picking in §7 has not been measured yet.
- The only evidence for the race frequency is experiment 1 E (2/7) and the symptom session.
- Measured on Windows only.
- vibe-kanban's Windows Job Object was seen only through crate docs, and the TS SDK's upper bound being 「문서에 없다」 (not in the docs) is a confirmation of absence.
- The raw logs and harnesses exist only in session scratch and were not committed (volatile).
