# TRD — T-40: easing the roughly 300-second hang of an interrupt during a claude hook — end only the leftovers after the interrupt inside our Job (S21)

> Status: ★**Landed (2026-10-01) — implementation ①②③ committed (`ccde81a` · `235cbca` · `e75f955`) · GUI measurement G1 PASS · decision = ADR-0262**★ (partially supersedes ADR-0257 · ADR-0238 · ADR-0244). ④ In the documentation landing I fixed the statements the implementation review found — §3-2 write-list failure · §3-4 launcher · walk step cap · hook shape · taskkill shape · §3-7 · §8 ⑰ⓑ same scale · §3-8 panic level (including the same statements in §2 · §3-3 · §6) — each fixed spot is marked 「착지 고침」 (landing fix). The known limits K1–K9 left by the implementation review = §8 ㉔ (user decision 2026-10-01 「일단 재현 가능한것만 구현하자. 좀 현실적인 사항으로 단순화」 (implement only what is reproducible first; simplify to something realistic) — recorded without code). ★§5 real-process test 13 was not built because of that decision · §7 G1b · G3–G6 were not run★. Below is the pre-landing header (unchanged).
>
> 14th edition header: **draft 14th edition · birth record after the interrupt + snapshot at Esc · before review (2026-09-30)** — the 14th edition = the 13th edition plus three fixes from the confirmation round: ⓐ ★「쓰기 전 탄생은 후보가 아니다」 (a birth before the write is not a candidate) is established not by the moment notifications pile up (there is no document) but by the **list of Job member numbers taken at write confirmation (write list W)** — the 13th edition's marker packet was withdrawn★ ⓑ start the listening thread **first** and only then attach the Job to the port (if attaching fails the thread ends — there are no notifications to pile up) ⓒ anything born between the write and W drops out (a miss — measured margin · test). The 13th edition = the 12th edition plus round-4 fixes: ⓐ (replaced by the 14th edition) ⓑ if detaching the port fails, the listening thread keeps taking notifications out and discarding them · a panic guard for the listening thread ⓒ creation order also on the taskkill link ⓓ if the end request is refused, the whole sweep stops ⓔ if 「묵은」 (stale), no episode is set up · this guard of the opener covers every abnormal exit ⓕ record details (boundary when turned on · overflow reset · if the record disappears, `snapshot_failed(port)`) ⓖ one confirmation deadline per round ⓗ the creation-order check is not a user decision but a **[Pick]** (the user was told) ⓘ §9 by ADR line ⓙ re-notification at attach time for the first episode ⓚ `tear_down_failed_activation` also calls the retirement start. The skeleton of the 12th edition (decision 11 — from the first interrupt to the sweep, anything born by join notification is opened at once and recorded · ancestors from before the interrupt by snapshot at Esc · in every link parent creation ≤ child creation · confirm after ending and pick again at most 3 rounds · claude's `taskkill /PID <꼭대기>` · only births after the write are candidates · a single opener · port lifetime · retirement at the first line of `kill_agent` · the write-confirmation panic guard) is unchanged. ★Two new `windows` features (`Win32_System_IO` · `Wdk_System_Threading`) = a dependency change · §10★. Edition history = `git log -- docs/process/S21-chat-ux/trd-t40.md`. No code change. Internal implementation that the user does not feel was chosen by this document (**[Pick]**).
>
> **Inputs:** tracking [`docs/tracking.md` T-40](../../tracking.md) · research [`docs/research/claude-interrupt-hook-hang-2026-09-29.md`](../../research/claude-interrupt-hook-hang-2026-09-29.md) · the preceding TRD [`trd.md`](trd.md) §3-4 (ADR-0238). **Reading baseline** — code = the stage-1 commit `9fa9215` (**HEAD**) · the 5th edition's stages 2 and 3 = `refs/backup/t40-clock-design-20260930` (**backup**). The `windows` crate = this PC's `windows-0.58.0` source. Measurements = `20260929-t40-spike/` · `20260930-msys-ppid-spike/` · `20260930-t40-provenance-spike/` in `.claude/handoff/attachments/` (`WRITEUP.txt` · `main.log` · `pilot.log` · ★`procs/p1..p6.procs.tsv` (main) · `procs/pilot-p1.procs.tsv` (pilot) — both preserved★ · `probe/`). claude internals = reading the claude 2.1.284 bundled code (2026-09-30). MS Learn = `JOBOBJECT_ASSOCIATE_COMPLETION_PORT` · I/O Completion Ports (§3-5).
>
> **Anchors:** ADR-0001 · ADR-0004 · ADR-0006 · ADR-0012 · ADR-0175 · ADR-0217 · ADR-0218 (★hold identity by an open handle★) · ADR-0230 · ADR-0238 (★decisions 2 · 3 · 5 amended★) · ADR-0244 · ADR-0245 · **ADR-0257** (partial supersede · §9).
>
> Notation: **[Pick]** · **[User]**. **Episode** = one round of interrupting. **Snapshot** = what holds our Job's members at the first interrupt. **Birth record** = what holds the processes that arrived by join notification during that episode, opened at once (sequence number `seq`). **Facts** = parent PID · creation time · executable · command line (read once through the held handle). **Known link** = a process that is in the snapshot or the birth record. **Hook runner** = the `…\Git\bin\bash.exe -c "bash …"` that claude spawns for each hook.

---

## 0. Conclusion (first)

| What | Where | Gist |
|---|---|---|
| **Interrupt record** | `GateCell` in `backend/claude/leftover.rs` | door lock section ① → (new episode · there is a single opener) outside the lock, turn on the birth record and take the snapshot → open in section ② (if the turn closed in between, no line is given · if the record number · round changed, only the line is given and no episode is set up). The N seconds count from the **moment the line is written** to stdin · ★right after the write, the writer takes the Job member number list W once — a candidate must be outside W★ |
| **Birth record** | the listening thread (one per incarnation · rises at the first interrupt · ★the Job is attached to the port after it rises★) | waits for Job join notifications and, if recording, opens at once, reads the facts and holds it (end notification discarded · decision 6). If not recording, discards it (decision 7). On failure, detaches the port and that incarnation does not record |
| **N-second check** | worker thread | the sweep at written interrupt + N. Picks → ends (in sequence order · re-check + `TerminateProcess` inside the door lock) → confirms the end → picks again (up to 3 times) |
| **Picking** | same file | candidate = a recorded birth · ★number not in the write list W (= not something that was alive before the line was written)★ · outside the snapshot · alive. What is ended is whatever fills all of: ① parent is a known link · ended (decision 11) ② a hook copy (same executable as the parent, and either the same command line or the parent is `-c "bash …"`) (decision 10) ③ every link up to claude is known · ended · **parent creation ≤ child creation** (decisions 9 · 11 · 5) ④ the top = the hook runner, and in this episode there is a **`taskkill /PID <꼭대기> /T /F`** birth that claude spawned, and that taskkill was born later than the top (decision 10) |
| **Retirement** | channel · session · manager | ★at the start of `kill_agent` (before permission is revoked · before `Exiting`), the manager calls the channel's 「물러남 시작」 (retirement start) through the session★ — the read-only `RetiringSignal` that the channel holds is set · the sweep and the re-check watch it |
| **OS piece** | `platform/windows.rs` | list (stage 1) · holding (`pin_member` — membership · facts · choosing the end permission) · join notification port (`watch_births` once — the API enforces the order port → start listening thread → attach · `unwatch_births` · `next`) |
| **Notice · replacement · off switch** | none [decisions 1·2·3] | the screen is the same as a usual interrupt · only a warn log |

- **Checked against the measured hangs (inference — applying the rule to the records): 10/10** — for the 7 runs that have tables (09-30 A 5 · background p3bg · pilot p1bg), the chain · creation order · hook copy · `taskkill /PID <꼭대기>` (7/7 · twice each) all stand up in the table, and the 3 runs of 09-29 are viewed as the same shape (§3-0). ★The re-runnable cross-check = `20260930-t40-provenance-spike/r4sim.py` — applying the rule to all 35 taskkill episodes of the 7 tables picks only the 7 owners + p3bg's 24776 in round 2 and picks nothing else★. Condition = open a briefly-lived middle link in time (0 failures to open among 4322 in the harness).
- **Implementation order** (§6): ① stage-1 removal · OS piece · retirement · write confirmation → ② pure core → ③ wiring → ④ documentation. **A new ADR is needed** (§9). **User confirmation = 1 item** (§10 — two features).

---

## 1. User decisions (2026-09-29 · 2026-09-30)

| # | Decision | Source |
|---|---|---|
| **Adopted plan** | ★Kill only the stuck leftovers inside our Job and keep claude alive★ | research §7 |
| **Picking** | Record the time of the Esc that actually sent the interrupt. If there is no turn end after N seconds, kill the Job members that are not claude · not a direct child · created after the recorded time · alive. Orphans · 「bash」 · CPU are not the main criterion. ★14th edition reading (keep the wording and read it again)★ — 「적은 시각 뒤에 생성」 (created after the recorded time) = among the recorded births, one that was not in the member list (W) taken right after the line was written · 「가장 최근 Esc」 (the most recent Esc) = the most recently written interrupt · 「claude 아님 · 직계 자식 아님」 (not claude · not a direct child) is kept by structure in §3-4 | research §7 |
| **PID rule** | ★Do not pick by PID parent alone — Job list + creation time★. ★14th edition reading★ — candidates come only from our Job's join notifications · membership · parent link by the same handle · the write list W only narrows. ★**[Pick — not a user decision]** use the 「생성 시각」 (creation time) of this rule as the relative order of the two ends of a link: parent creation read through the held handle ≤ child creation (filters out a wrong parent slipping in through number reuse · it only narrows) — the user was told (§10)★ | research §4 |
| **Clock · N** | ~~compare with the system time~~ · N = 3 seconds · monotonic time | research §7 |
| **OS piece** | Keep the Windows-only piece small (list + end one) · no-op on other OSes · make it portable. Holding · facts · notification port are inside that piece | research §7 |
| **Decisions 1 · 2 · 3** | ★No notice (「그걸 왜 알려」, why tell them that) · no replacement (「너무 복잡해」, too complicated) · no off switch (「ㅇㅇ 두지 않고」, yes, don't provide one)★ | 2026-09-29 |
| **Decision 4** | ★If the parent is alive, do not touch it★ (「ㅇㅇ 조건 더해줘 어차피 잘 안일어나는 일이니깐」, yes, add the condition, it rarely happens anyway) — decision 9 widened it to ancestors | 2026-09-29 |
| **Decision 5** | ★Tell 「끊기 뒤에 태어났나」 (was it born after the interrupt) by the **Job join notification** instead of an absolute time★. The user: 「근데 생성된 절대시간말고 상대시간은 모름?」 (don't we know a relative time rather than the absolute creation time?) · 「알림이 오면 그냥 그거 핸들 보관한다음에 그것들만 이터 돌면 되겠네」 (when a notification comes just keep that handle and then iterate over just those). ★Do not tell 「Esc 뒤에 태어났나」 (was it born after Esc) by time (comparing the Esc time and the creation time)★ | 2026-09-30 |
| **Decision 6** | ★Record only creation, and do not handle what leaves★ (「생성된것만 기록하고 빠지는건 처리 안하고 그냥 날리면 되는거지」 (just record only what is created, and leave what drops out unhandled, just let it go) · 「구지 처리하지 않아도 되는건 처리하지 않아도됨. 종료 이벤트까지 받아서 매번 빼고 그러면 번잡하잖아」 (what doesn't need handling needn't be handled; taking the exit events too and removing each time is cumbersome)) | 2026-09-30 |
| **Decision 7** | ★Collect only at Esc · implement in the simplest way★ (「어쨋든 esc 할때만 수집하는게 맞아. 그 안에서는 너가 자유롭게하면됨. 나중에 프로세스를 계속 들고있어야되는 이유가 생기면 그쪽으로 가는게 맞겠지만. 그 안에서는 구현 심플한 쪽으로 하면 될듯」 (collecting only at Esc is right anyway. Inside that you are free. If a reason to keep holding processes arises later, we should go that way, but inside this the simpler implementation will do)) — record · holding only from the first interrupt to the sweep · after attaching the port, notifications outside an episode are taken out and discarded | 2026-09-30 |
| **Decision 8** | ~~Only when the dead parent was there from before Esc~~ (「죽은 부모가 bash 면 esc 전부터가 맞긴하겠구나. 방어 되면 그렇게 진행해.」 (if the dead parent is bash it must indeed be from before esc; if it can be defended go that way)) — ★replaced by decision 11 (every link is known · ended)★ | 2026-09-30 |
| **Decision 9** | ★Climb from the dead parent up to claude and exclude it if alive★ (「ㅇㅇ 알았어. 어쨋든 부모 쭉 타고올라가서 살아있으면 제외한다는거잖아. 딱히 로직 부담도없겠네 상위부모 탐색만 하면되니」 (OK. So climb the parents all the way and exclude if one is alive. No logic burden either, just search the upper parents)) · claude · the root are where it stops | 2026-09-30 |
| **Decision 10** | ★If there is a clean hook marker, add it · if messy, accept the remaining gap and go ahead★ (main plan · user accepted). The user: 「어차피 5분 멈추는것보다는 훨씬 좋은거잖아 이것들이.」 (these are much better than a 5-minute hang anyway) · the marker was clean → hook copy · hook chain (runner shape + claude's `taskkill /PID`) | 2026-09-30 |
| **Decision 11** | ★Record what arrives during the 3 seconds and do not drop what gets deleted — not take one shot and compare★. The user: 「아니 내가 원래 얘기했던게 3초동안 오는거 기록하고 삭제되는건 빼지 말라고 했는데 한번 캡쳐하고 다음번에 비교하라고 해서 그렇게 진행한건데?」 (no, what I originally said was to record what arrives during the 3 seconds and not drop what gets deleted, but you told me to capture once and compare next time so I went that way?) → decisions 5 · 6 revived · 7 unchanged · 8 replaced | 2026-09-30 |

---

## 2. Places that change (baseline = HEAD `9fa9215` · backup)

| File | What | Stage |
|---|---|---|
| `crates/engram-dashboard-agent/Cargo.toml` | **`Win32_System_IO`** · **`Wdk_System_Threading`** added to the `windows` features · comment | ① |
| `…/platform/windows.rs` | **Added:** `pin_member(pid, kill)` · `PinnedMember{exited, facts, terminate_raw, classify, wait_exit}` · `ProcessFacts{ppid, create, image, cmdline}` · `watch_births(start)` · `unwatch_births` · `BirthPort{next}`. **Removed:** `verify_member` · `MemberCheck` · `VerifiedMember` (→ `PinnedMember` · the two end methods live on, moved). `member_pids` (HEAD `:136-218`) · `GetProcessTimes` (now used for creation time) stay | ① |
| `…/platform/process_group.rs` | `ProcessGroup::new(job, retiring)` · `member_pids` · `pin` · `watch_births` · `unwatch_births` · `retiring` · `Pinned` · `Births` · `RetiringSignal` · **Removed:** `verify` · `Verify` · `ReadyKill` · `root_attached()` · tests rewritten | ① |
| `…/platform/process_tree.rs` · `…/platform/mod.rs` | the three that stage 1 added and the test removal · header doc | ① |
| `…/transport/mod.rs` | ★adds `AgentTransport::begin_retire(&self) {}` (default no-op)★ | ① |
| `…/transport/stdio.rs` | `root_attached` removed · ★a `retiring: Arc<AtomicBool>` cell — set by `begin_retire()` and the first line of `shutdown()`★ · `process_group()` = a weak handle + `RetiringSignal` · `InterruptLine` → `InterruptOut` · tests (HEAD `:589` · `:615` · `:725` · `:768`) | ① |
| `…/transport/input_queue.rs` | an optional callback per chunk — `push_with` · `pop` give the pair · `drain` calls it after a successful write · `mark_written`, **inside its own `catch_unwind`** (a panic is logged as error and it continues — §3-8) · a closed one is discarded · the existing tests (`:391` · `:410`) | ① |
| `…/transport/pty.rs` | **not touched** — the shape of `drain` · `push` stays the same and it does not call `pop`. `begin_retire` is a default no-op | — |
| `…/session.rs` · `…/manager.rs` | ★`Session::begin_retire()` (passes it on to the channel) · called from the first line of `kill_agent` (HEAD `manager.rs:2884`) (before permission revocation) and from `tear_down_failed_activation` (HEAD `:2340` — the first line after the incarnation marker comparison)★ | ① |
| `…/backend/mod.rs` | `console_wrapper_depth` and test removal | ① |
| `…/backend/claude/leftover.rs` **new** (from the backup) | episode · opener · snapshot · birth record (listening thread · record lock) · write confirmation · worker (panic guard) · sweep (several rounds) · rules · end confirmation · log | ② · ③ |
| `…/backend/claude/mod.rs` | the backup wiring (`GateSlot` · `open_spawn` · decoder · `interrupt_line` → `InterruptOut`) · `leftover_cleaner` · `TurnGate` (HEAD `:750-800` · test `:5625`) removed and doc · `// ADR-0238` → `GateSlot` · `started` → `deliver` · HEAD tests `:6070-6137` rewritten · `control` comment | ① (only the return shape of `interrupt_line`) · ③ |

Frontend · wire types · protocol · daemon · shell · `base` · the `open_spawn` signature unchanged. The stage-1 additions to `base` (`ProcessStart` · `process_start` · `process_parent_table`) are no longer called by T-40 but remain because they are bodies of existing public functions — `process_creation_time` is called by `backend/codex/mod.rs:1096` · `codex/thread_lock.rs:1236` · `platform/file_holders.rs:190` · daemon `tests/ws_e2e.rs:2445`, and `child_pids` by agent `tests/backend_contract.rs:1431` · daemon `tests/ws_e2e.rs:2469` (`rg` 2026-09-30).

---

## 3. Design

### 3-0. Measurement — the facts this design relies on

**Interrupt spike (09-29 · `20260929-t40-spike/`):** a harness imitating the dashboard spawn · a real hook · interrupt 200 ms after sending · claude 2.1.284 · hang 3/11 · at interrupt +3 s the one thing alive was a hang leftover (`Git\usr\bin\bash.exe <훅>.sh` · depth 5 · parent ended · four members at +3 s — `main.log:92` `:98` · `pilot.log:26`) · what was alive at +3 s was only leftovers in 11/11 · root's children = conhost + claude (4/4) · `result` is the same as a normal interrupt. The pilot's `followUp=TIMEOUT` is a harness parsing defect (`20260930-t40-provenance-spike/evidence-0929/pilot-p1.raw.log:72` → `:92`).

**MSYS parent spike (09-30):** the Windows parent of a `sleep` spawned by a live Git Bash was an already-ended fork in 5 of 7 variants — 「부모 죽음」 (parent dead) is not a discriminator.

**Provenance spike (09-30 · `20260930-t40-provenance-spike/`):** the harness attached a completion port to the Job after `assign` (★at spawn time · this design's 「첫 끊기에 붙임」 (attach at the first interrupt) was not measured★) and one listening thread (concurrency 1 · 50 ms wait) opened each join notification at once with `PROCESS_QUERY_LIMITED_INFORMATION`, read the parent (`NtQueryInformationProcess` 0) · creation time · executable · command line (60) and held the handle (`probe/src/main.rs:373-390` · `:587-610` · `:213` · `:251-298`) — 4322 records · ★0 failed to open · 0 opened but outside the Job★ (`main.log:49` `:99` `:155` `:201` `:246` `:285`).
- ★**Re-runnable cross-check:** `r4sim.py` (same folder — applies the 12th edition rule to the 35 taskkill episodes of the 7 tables · finds the parent by creation order and 3 rounds) picked only the 7 owners + p3bg's 24776 in round 2 and did not pick anything else (the round-4 end-safety review ran it). The narrowing added in the 13th edition (taskkill creation ≥ top creation) is shown by the `tk>=T` column of its output (8/8 true — re-run 2026-09-30). The script's candidate boundary is 「첫 taskkill − 5 ms 뒤 생성」 (created after first taskkill − 5 ms), and the 14th edition's write list W separates at a point earlier than or equal to that (right after the line is written) — the 8 that were picked were born at least 262 ms after that boundary★.
- Hang 5/30 + 2/4 during background repetition · the owner is always `"…\Git\usr\bin\bash.exe" C:/…/hooks/<x>.sh` · all ancestors up to claude ended · the follow-up turn sent next answered 9/9. ★End → `result` 19–35 ms is a value of the **single-owner shape** — in p3bg the owner (32228) had a fork alive at +3 s (24776 · +2756 → +4093 · its child 27524 +3802), and when the harness ended it at +10 s, `result` came 364 ms later (`main.log:145` · `procs/p3.procs.tsv`) — one owner is not the only owner of the pipe★.
- Hook runner = `"…\Git\bin\bash.exe" -c "<hooks.json 명령>"` (claude prepends `bash ` to a bare `.sh`) · Bash tool runner (foreground and background the same) = `-c "source …/.claude/shell-snapshots/snapshot-bash-… && … eval '<명령>' …"` — 291 to 6 · 0 ambiguous. An MSYS fork carries its parent's command line (1820/1820 · 43/43). The event has no PID. At interrupt claude ran `taskkill /PID <실행기> /T /F` for each hook runner (twice) · it did not for a Bash tool runner. The environment difference between hooks and the Bash tool (`CLAUDE_PROJECT_DIR` · `AI_AGENT`) is not used because it needs a PEB read.

**Applying the 14th edition rule to the 10 hangs (inference — applying the rule to the tables · the rule was not actually run):**

| Hang | Chain (creation = relative to the time the line was written · sweep = +3 s) | Known · ended · creation order · hook copy · taskkill | Result |
|---|---|---|---|
| 09-30 p1 · p2 · p4 · p5 (`procs/p{1,2,4,5}`) | owner (+263…+409) ← hook bash D (−80…−12) ← shell wrapper `-c "bash …"` (−132…−88) ← runner T (−187…−172) | D and above snapshot · all ended before +3 s · parent always first · owner = copy of D · 2 `taskkill /PID <T>` | ends 4/4 |
| 09-30 p3t3 (`procs/p3`) | owner (+454) ← D fork (+279 · 250 ms) ← hook bash (−6) ← wrapper ← runner | D = birth record · the rest snapshot · all ended · owner = copy of D · 2 taskkill | ends |
| 09-30 p3bg (`procs/p3`) | owner = hook bash (+1432) ← wrapper (+872 · +1695 ended) ← runner (+75 · +1722 ended) · the owner's fork 24776 (+2756 · alive) | all birth records · the owner is a script the wrapper exec'd (command line differs → the `-c "bash …"` branch) · 2 taskkill · ★round 1 owner → end confirmation → round 2 24776 (parent = the ended owner · copy)★ | ends (2 rounds) |
| pilot p1bg (`procs/pilot-p1`) | owner (+352) ← D hook bash (+16 · +389 ended) ← wrapper (−40) ← runner (−100) | D = birth record · the rest snapshot · owner = copy of D · 2 taskkill | ends |
| 09-29 main 2 · pilot 1 | owner (+203…+244) ← depth-4 hook bash (before the interrupt) ← … | snapshot · four members at +3 s · copy · taskkill is viewed as the 09-30 shape (no table) | ends 3/3 (inference) |

→ **10/10** (7 standing by tables + 3 viewed by shape). That the owner is outside the write list W (§3-2 — the owner was born at least 263 ms after the line was written, and W is taken right after the write) and the creation order stand in all 10 runs. Other post-interrupt births in the same sweep (taskkill · its conhost · the repetition's `sleep`/fork · `cat`/`jq`) drop out because the parent is alive, or it is not a copy, or the top is not a hook runner. The measurement verdict = §7 G1.

### 3-1. Flow at a glance

```
kill_agent 시작 ─ session.begin_retire() → 통로 retiring = true(권한 회수 · Exiting · shutdown 보다 먼저)

Esc → manager → session → StdioTransport::interrupt → 끊기 줄 함수
  구간 ① — 문 닫힘 → None · 산 에피소드 → 이어 적음 · 여는 중(다른 Esc 가 열고 있음) → 줄만 준다
         · 그 밖 → 표식 + 「여는 이」 표시
  (여는 이만) 자물쇠 밖: 포트 · 듣는 스레드 확보(한 번 — 포트 → 스레드 기동 → Job 붙이기) → 기록 켬(R · 남은 기록은 놓은 뒤 버림) → 스냅숏
  구간 ② — 문 닫힘 · 턴 닫힘 → None · 기록 번호 ≠ R · 전달 · 에피소드 차례 바뀜 → 줄만(에피소드 없음) · 아니면 새 에피소드
  (여는 이 가드: ② 에 닿지 못한 모든 출구 · 에피소드를 안 세운 출구 → opening 내림 · 놓은 뒤 stop(R) · 스냅숏 버림)
  → InterruptOut { 줄, 쓰였을 때 = 멤버 번호 명단 W 찍기 → note_written(gen, W?) } → 큐
라이터: 쓴다 → (catch_unwind) W 찍기(아무 락 없이 · 완전하거나 오류) → note_written → W 가 있을 때만 첫 쓰임 · last_mono → 일꾼 기동
듣는 스레드: 가입 알림 + 기록 중 → 곧바로 열어 사실 → (pid, 생성) 이 새로우면 seq 로 기록

(멈춤) 쓰인 지 N 초 → 판(최대 3 차례 · 차례마다 확인 마감 200 ms 하나):
   후보 = 기록된 탄생 · 번호 ∉ W · 스냅숏 밖 · 산 것 → 규칙 ①–④ → 순번 순으로 끝내기(문 자물쇠 안 재확인 — 거절되면 판 전체를 멈춘다)
   → 끝남 확인 → 기록을 다시 복사해 다시 고른다(끝낸 것은 끝난 고리) → 없으면 멈춤
```

### 3-2. Record — two sections · opener · snapshot · birth record · write confirmation

```rust
pub(super) struct GateCell { state: Mutex<GateState>, cleaner: Option<Arc<Cleaner>> }
struct GateState { open: bool, closes: u64, delivers: u64, opening: bool, episode: Option<Episode>, next_gen: u64, worker: bool }
struct Episode { gen: u64, first_mono: Instant, last_mono: Instant, written: bool, w: Option<Arc<[u32]>> /* 쓰기 명단 W — 첫 쓰임 때 · 정렬 */,
                 snap: SnapState /* 인라인: Taken(Arc<Snapshot>) · Failed(사유) · Released */,
                 rec: u64, killed_any: bool, phase: Phase }
struct Mark { closes: u64, delivers: u64, next_gen: u64 }
// 기록 — 또 하나의 잎 · 문 자물쇠와 겹쳐 잡지 않는다 · 독을 견딘다
struct Recorder { active: AtomicU64 /* 0 = 없음 · 쓰기는 자물쇠 안 · 읽기는 어디서나 */, port_failed: AtomicBool, inner: Mutex<RecInner> }
struct RecInner { next_rec: u64, next_seq: u64, births: Vec<Arc<Birth>>, full: bool }
struct Birth { seq: u64, facts: ProcessFacts, pin: Box<dyn Pinned>, killable: bool }
```

- **The interrupt-line function** (the backup's `leftover.rs:889`, split into two sections):
  1. If there is no cleaner, exactly as today.
  2. **Section ①** — door closed → `None` · a live episode → if `Cleaning` then `Waiting` · accepted under that `gen` · ★`opening` true (another Esc is opening) → only the line is given (no `on_written` — the opener's episode takes charge of that turn)★ · anything else → `opening = true` · `Mark`.
  3. **Only the opener**, outside the lock: ⓐ secure the port · listening thread (§3-5 — port → start the thread → attach the Job · look at `OnceLock` together with the hardened failure flag `port_failed` → `Failed(port)`) ⓑ `R = recorder.start()` (`active = R` · reset `full` · the leftover records are taken out and discarded **after releasing**) ⓒ snapshot (below). ★The record is turned on before the snapshot — whatever is born after that is either recorded (after the list) or is in the list (before the list) · if both, the snapshot side wins★.
  4. **Section ②** — `opening = false` · door closed · `closes` changed → `None` (no line) · ★atomic read of `recorder.active` ≠ R · `delivers` · `next_gen` changed → **do not set up an episode and give only the line** (no `on_written` — a miss · debug `stale`)★ · a live episode exists → a branch that cannot be reached because there is one opener (`debug_assert` + log · only the line is given) · anything else → a new episode (the taken snapshot · `rec = R` · `written = false` · `Waiting`).
  5. After releasing: discard the snapshot of the swapped-out old episode and turn its record off → `InterruptOut { bytes, on_written: Some(W 찍기 + note_written(gen, W)) }`.
  - ★**The opener guard (drop guard)**★: set when entering 3 and released only when section ② set up a new episode. On every exit where it is not released (panic · early return · no line · only the line) — take the door lock, lower `opening = false` and, **after** releasing, `recorder.stop(R)` · discard a half-taken snapshot.
- **Taking the snapshot [Pick]:** `L = member_pids()` (only when complete) · `|L| > 256` → `Failed(too_many)` · root ∉ L → `Failed(root)` · for each of L, `pin(pid, kill = false)` — if it is our Job by the same handle, hold it and read the facts · failing to hold the root → `Failed(root)` · more than two held members whose parent is the root → `Failed(shape)` (root's children = conhost · claude — Q2 · inference).
- **Birth record (listening thread) [Pick — decisions 5 · 6 · 7 · 11]:**
  - **One notification:** `next(500 ms)` → if it is not a join notification, discard (exit = decision 6) · if it is a join, atomically read `active` → if 0, discard → otherwise **at once, outside the lock**, `pin(pid, kill = true)` · ★if `Err`, try `pin(pid, kill = false)` once more → if held, it is **link-only** (`killable = false` — it cannot be a candidate and still joins the chain)★ · if `Ok(None)` (vanished · outside the Job), discard → inside the record lock, if `active` is still the same and ★`(pid, 생성 시각)` is new★, insert it with a `seq` (if over 512, `full` · not inserted) → what could not be inserted is discarded after releasing.
  - **Start:** the thread that rose first receives the opener's 「붙었나」 (did it attach) — if `false`, it ends at once (§3-5).
  - **Each round · end:** at every notification · timeout, briefly upgrade the weak Job handle and check whether it is alive and the termination marker (★do not carry the upgraded `Arc` into `next`★) → if it has vanished or the marker is set, release the record with `stop(active)`, close the port and end.
  - ★**Failure mode**★: if `next` has an unrecoverable error or the body of a round panics (`catch_unwind` per round) → `port_failed = true` (hardened — later episodes are `Failed(port)`) · `stop(active)` · log once (error → warn · panic → error (§3-8)) · `unwatch_births()`. If detaching succeeds it ends. ★If detaching fails it does not end but keeps **only taking out and discarding** (no recording · until the weak handle · termination marker is seen — bound by the incarnation lifetime) — because if the port stays attached and nobody takes things out, the kernel queue grows (round 4 codex)★.
  - **Why open at once:** the PID in a notification is guaranteed neither alive nor not reused unless it is opened and held (MS — §3-5). If opened and held at once, it is not reused afterwards and the facts of a parent that ended also remain (harness 0/4322 missed · the briefest-lived middle link = p3t3's D 250 ms).
  - **On · off:** `start()` · `stop(R)` — the places that turn it off = sweep completion · turn end · new input · termination marker set · worker start failure · panic guard · swap-out · opener guard · the end of the listening thread · failure mode. All after releasing the door lock · the taken-out record is discarded after releasing the record lock. The sweep's copy gives an empty one unless `active == rec`.
- ★**Write confirmation — write list W (confirmation round codex BLOCK #1)**★: the writer writes the line (success · after `mark_written` · with no lock · inside its own `catch_unwind`) and calls the callback → ⓐ briefly upgrade the weak Job handle, take **`member_pids()` once** (complete or an error — as in stage 1), sort it and make it `W` (release the upgraded `Arc` at once) ⓑ `note_written(gen, W 또는 실패)` — inside the door lock, if it is the same `gen` and `Waiting`/`Cleaning`, `last_mono = max(…, now)` · ★the first write counts only when there is a W: `written = true` · `w = W` (list failure · handle vanished → that write cannot become the first write · warn — if a later interrupt of the same episode is written with a W, that becomes the first write, and until then there is no sweep · landing fix 2026-10-01)★ · if it is not the first write, the W is handed back and discarded after releasing · the worker's share → start the worker after releasing.
  - ★**Why it is safe — it does not rely on when notifications pile up**★: a candidate C is held by the birth record's handle and is alive at sweep time. If C joined before W was taken, it was also alive when W was taken (an ended process does not come back to life) and the list is complete, so **C's number is in W → not a candidate**. So every candidate is one that joined after W — that is, after the line was written. Since the comparison is by number only, if a number that was in W ends and a birth after the write reuses that number, it drops out (a miss · the safe side). What it relies on = the completeness of `member_pids` (stage 1 — an error if the allocation count and the list count differ) · a process that once joined does not leave the Job (MS docs) · and only the end decision of the held handle.
  - **When notifications pile up · the order they are taken out affects only effectiveness:** even if they pile up late or are taken out late, the candidate decision is the same — you can only miss a briefly-lived link before opening it (§8 ③).
  - ★**Miss window — between the write and W**★: something born after the line was written and before W was taken, and alive at W time, goes into W and **drops out** (a miss). The gap = the time for the writer to return from the write and take the list once (short — estimate · G1 measures the write → W delay). Measured margin: the owner is 263–454 ms after the write (p3bg owner 1.4 s · its fork 2.8 s — §3-0 table) · a hook fork is at least 1 ms later than taskkill and taskkill is spawned after claude reads the line (main measurement). ★The 13th edition's 「흐림이 양쪽 다 사라진다」 (the blur disappears on both sides) was wrong — this gap on the miss side remains★.
  - **A candidate = only a birth whose number is ∉ W** — what is in W and what ended before W was taken is used only as a link. An episode of a line that was not written has no sweep. The callback of a later interrupt in the same episode also takes a W but `note_written` discards it (only the first write's counts — births between later interrupts can still become candidates).
- **decoder:** turn open (only on closed → open) · new input (live `started` — in one section discard the episode and `delivers += 1` · open it if it was closed · B2 measured claude 2.1.280) · turn end (`open = false` · `episode = None` · `closes += 1`).
- **Invariants:** 「산 에피소드면 문이 열려 있다」 (if there is a live episode the door is open) · 「`written` 이면 `w` 가 있다」 (if written then w exists) · 「쓰인 `Waiting`/`Cleaning` 또는 `Spent` 이면 일꾼이 떠 있다」 (if a written Waiting/Cleaning or Spent, the worker is up) · 「`Taken` 스냅숏 · 켜진 기록을 가진 에피소드는 `Waiting`/`Cleaning`」 (an episode holding a Taken snapshot · a turned-on record is Waiting/Cleaning) · 「`opening` 은 여는 이의 두 구간 사이에만 참」 (opening is true only between the opener's two sections) · 「`active` 가 켜져 있으면 그것을 쥔 산 에피소드가 있거나 여는 이가 일하는 중이다」 (if active is on, there is a live episode that holds it or the opener is working).

### 3-3. N-second check · retirement · locks

- **Constants:** `INTERRUPT_LEFTOVER_GRACE = 3 s` · `PIN_MAX = 256` · `BIRTH_MAX = 512` · `ANCESTOR_MAX = 16` · `PASS_ROUNDS = 3` · `KILL_CONFIRM = 200 ms` · listening wait 500 ms.
- ★**Retirement start [Pick — round 3 codex · second finding]**★: add `AgentTransport::begin_retire(&self)` (default no-op — pty · codex channels stay as they are), and `StdioTransport` sets its own `retiring` cell (`Arc<AtomicBool>`) · the first line of `shutdown()` also sets it (another end path). `Session::begin_retire()` passes it to the channel, and **`kill_agent` (HEAD `manager.rs:2884`) calls it before permission revocation · intent · `Exiting` · `session.kill`**, while `tear_down_failed_activation` (HEAD `:2340`) calls it on the first line after the incarnation marker comparison (other end paths are covered by the first line of `shutdown()`). The cell's owner is the channel · the manager calls only a neutral method · the backend reads it through the read-only `RetiringSignal` that `ProcessGroup::retiring()` gives (only `is_set()`) — the manager does not know the backend's door. The remaining overlap = one end right after the re-check read the marker (§8 ⑲).
- **One step `next_step(now, retiring)`:** marker → clear the episode and finish · none · `Done` · an unwritten `Waiting` → finish · a written `Waiting` before → sleep · past → `Cleaning` + `Ticket` → sweep · `Spent` before → sleep · past → `Done` + warn (decision 2).
- ★**The sweep's rounds (round 3 end-safety F3) [Pick]**★: one round = copy the record (clone `Arc`s inside the record lock · an empty one unless `active == rec`) → candidates · rules (§3-4) → confirm ending the `Cleanup`s in **ascending `seq`** (ancestors born first) → confirm the end of those ended in that round through their handles ★within **one deadline (200 ms)**★ (`wait_exit(남은 시간)`) → next round. Something ended is seen in the next round as an 「끝난 알려진 고리」 (ended known link), so the copy it was parent to (p3bg's 24776) becomes a candidate. If there is no `Cleanup` it stops · **at most 3 rounds** — the longest measured is 2 rounds (p3bg: owner → its fork) and one round is left as margin (if exceeded, warn `rounds_exhausted`). What did not end within the deadline is seen as alive in the next round, so what is below it drops out (a miss).
  - ★**If the re-check is refused, the whole sweep stops**★ (`still_due` false by turn end · new interrupt · new input · termination marker — the remaining candidates and the next round are discarded · debug `superseded`).
  - **Upper bound of the sweep time** = 3 rounds × (confirmation deadline 200 ms) + picking · copying · end calls — the waiting is at most 0.6 s per sweep.
- **End confirmation (as in the 5th edition):** inside the door lock read the monotonic time · marker so `still_due` = no marker · same generation · written · `Cleaning` · write + N ≤ now → if true, one `terminate_raw` through the birth record's handle (wrapped by two time reads) · `note_kill` → after releasing, classify · confirm · log.
- **Sweep end:** `Spent`/`Done` · snapshot `Released` · record off — what was received back and the `Ticket` are discarded after releasing the lock. A completed sweep happens once.
- **Panic guard:** unwinding → `worker = false` · `Done` · the snapshot · receive back the record → error after releasing (§3-8) · on normal exit the guard is disarmed.
- ★**Lock rules (canonical — the CLAUDE.md lock-order line carries this)**★
  - **The door lock `GateCell.state` is a leaf: while holding it, do no logging · thread start · emit · sleep · other locks · allocation and release · list queries · holding and handle closing · any other OS call; the only exceptions are ⓐ reading the monotonic time ⓑ exactly one `TerminateProcess` call of the end confirmation. Atomic reads (termination marker · `recorder.active`) are allowed. The snapshot · record number are built outside and moved in, and what was cleared and the sweep's table are discarded after releasing.**
  - **The record lock `Recorder.inner` is also a leaf: while holding it, do only fields · `Vec` insertion · `Arc` cloning · taking out (allocation allowed), and no OS calls · handle closing (the last drop of `Arc<Birth>`) · logging · other locks. Do not hold it overlapping with the door lock. It survives poisoning.**
  - Where they are taken: door = the interrupt-line function · write confirmation (the writer — called with no lock · taking W is outside the lock · only `note_written` takes the door) · decoder · worker · sweep end · confirmation · panic guard · opener guard / record = the listening thread · `start` · `stop` · the sweep's copy.

### 3-4. Picking — the rules

**Order of a sweep [Pick] — if even one does not stand, nothing is ended:** 1 no marker (debug `retiring`) → 2 snapshot is `Taken` and the record is alive (`active == rec` — otherwise the listening thread failed and released the record) (warn `snapshot_failed` (`cause` = list · root · too_many · shape · port)) → 3 the held root is alive (debug `root_gone` · warn `root_unknown`) → 4 rounds (§3-3) — 0 candidates (first round) → warn `no_new_member` (`births` · `full`) · all dropped out → warn `parent_rule` (count per rule) → 5 re-check the root · marker before each end (outside) → 6 confirm · confirmation (error warn · none could be ended warn `all_failed`).

**Candidate** = among this record's (`rec`) births, ★number ∉ write list W (§3-2 — the wall that blocks births before the write)★ · `killable` · that (pid, creation) is not in the snapshot · the held handle has not ended.

**Rules (candidate C · recorded parent D). A known link = a held member of the snapshot or a birth of the same record. ★In every link the parent's creation time ≤ the child's creation time (two kernel stamps read through held handles) must hold — otherwise that link is not known★:**

1. **Parent (decisions 11 · 4):** if D is not a known link (lost · not opened in time · order mismatch · 0) `parent_unknown` · D alive → `parent_alive` · error → `unknown` · ended → next.
2. **Hook copy (decision 10):** the executable paths of C and D are the same and ⓘ C's command line == D's command line (a fork copy — 1820/1820) or ⓘⓘ `is_hook_command(D)` (C is a script that the wrapper exec'd — p3bg) → next · otherwise `not_hook_copy` · an empty field → `unknown`. Exec'd programs (`sleep` · `cat` · a server · a build child) drop out.
3. **Ancestors (decisions 9 · 11):** walk from `A₁ = D` along the recorded parents · stop at the `Aₖ` (claude) where `parent(Aₖ) == 뿌리` and do not decide · `A₁ … Aₖ₋₁` must be known links (including creation order) and ended — alive → `ancestor_alive` · unknown · 0 · cycle · more than 16 (the count of `A₁ … Aₖ₋₁` — landing fix) · k = 1 · reaching the root first → `chain_cut` · error → `unknown`. The top **T = Aₖ₋₁**.
4. **Hook chain (decision 10):** T's executable name is `bash.exe` and `is_hook_command(T)` and ★this record has a birth whose **parent is claude (= `Aₖ`), whose executable is `taskkill.exe`, whose arguments after the program are exactly `/PID <T 의 PID> /T /F` (order · case as is · no extra arguments — landing fix), and whose creation is at least T's creation**★ (the link from that taskkill to T also respects parent · child order — this filters out a taskkill that pointed at an old T through number reuse) → a candidate to end · otherwise `not_hook` (if there is no taskkill, `no_taskkill`).
   - **`is_hook_command(f)`** = the command line is shaped 「첫 토큰 · `-c` · 인자 S」 (first token · -c · argument S) and S starts with `bash ` and contains neither `shell-snapshots` nor a line break (CR · LF) (line break = landing fix · positive · claude 2.1.284's wrapping of `.sh` hooks · 291 to 6 · the negative marker only narrows).
   - **taskkill marker:** at interrupt claude spawns `taskkill /PID <실행기> /T /F` twice for each hook runner that was running (WRITEUP B · 2 per top in the 7/7 hangs that have tables — checked against `procs/*.procs.tsv` 2026-09-30). It did not spawn it for a Bash tool runner. Positive · **only narrows** — if claude changes how it interrupts, everything is missed (safe).
- **Hook-copy check on every link (optional — not adopted):** the shell wrapper → runner link has different executables (`usr\bin` versus `bin`) so a third shape is needed, and rules 2 · 4 · taskkill already tie the chain into a hook tree — there is nothing to gain.
- **claude · wrapper · console host:** in the snapshot so they cannot be candidates · claude's direct children (taskkill · a hook runner started after the interrupt) drop out at 1 because the parent claude is alive · root alive = claude alive (`cmd /c` · Q2) · if a launcher intervenes (root → launcher → claude) the walk stops at the launcher so claude becomes a link — if claude is alive, `AncestorAlive{claude}` · if ended, T = claude.exe → `not_hook` (either way everything is missed · safe · landing fix 2026-10-01).
- **An all-AND filter** (research §7 「고아 여부는 주 기준이 아니다」 (whether it is an orphan is not the main criterion)).

```rust
fn select(c: &Link, esc: &Chain) -> Verdict;   // 순수 — Chain = 스냅숏 + 기록을 (pid, 생성)으로 찾는 보기 · 「끝났나」는 주입
fn is_hook_command(f: &ProcessFacts) -> bool;
fn taskkill_names(f: &ProcessFacts, claude: u32) -> Option<u32>;   // `/PID n /T /F` 의 n
enum Verdict { Cleanup { chain: Vec<u32> }, ParentAlive, ParentUnknown, NotHookCopy, AncestorAlive { pid: u32 }, ChainCut, NotHook, NoTaskkill, Unknown }
```

### 3-5. OS piece — list + holding + facts + join notification

```rust
pub(crate) struct ProcessFacts { pub ppid: u32, pub create: u64, pub image: String, pub cmdline: String }
impl JobObjectHandle {
    pub(crate) fn member_pids(&self) -> io::Result<Vec<u32>>;                              // HEAD 그대로
    pub(crate) fn pin_member(&self, pid: u32, kill: bool) -> io::Result<Option<PinnedMember>>;
    pub(crate) fn watch_births(&self, start: impl FnOnce(Arc<BirthPort>) -> io::Result<()>) -> io::Result<()>;
        // 포트를 만든다 → start(포트) = 듣는 스레드 기동 → 그것이 Ok 일 때만 Job 을 붙인다 · 두 번 부르지 않는 것은 정리기의 OnceLock 이 지킨다
    pub(crate) fn unwatch_births(&self) -> io::Result<()>;        // 포트 연결을 뗀다
}
impl PinnedMember { fn exited(&self) -> io::Result<bool>; fn wait_exit(&self, d: Duration) -> io::Result<bool>; fn facts(&self) -> &ProcessFacts;
                    fn terminate_raw(&self) -> io::Result<()>; fn classify(&self, raw: io::Result<()>) -> io::Result<MemberOutcome>; }
impl BirthPort {                                   // Send + Sync — 듣는 스레드와 watch_births(붙이는 동안)가 Arc 로 쥔다 · 마지막 drop = 포트 닫기
    fn next(&self, wait: Duration) -> io::Result<PortEvent>;   // Joined(pid) · Other · Timeout
}
```

- **Holding · facts:** `OpenProcess(QUERY_LIMITED | SYNCHRONIZE [| TERMINATE])` → 87 gives `Ok(None)` → `IsProcessInJob(h, 우리 Job)` (NULL forbidden) false gives `Ok(None)` → facts once: parent = `InheritedFromUniqueProcessId` of `NtQueryInformationProcess(h, ProcessBasicInformation(0))` (the result struct is this file's `repr(C)` — `Win32_System_Kernel` is not turned on · harness `probe/src/main.rs:251-275`) · creation = the creation FILETIME of `GetProcessTimes(h)` (a function HEAD already uses · the same handle, so it belongs to that process, not to the number) · command line = `ProcessCommandLineInformation(60)` (64 KiB) · executable = `QueryFullProcessImageNameW`. A field that could not be read is an empty field · creation 0 (that link cannot pass the order check and counts as not known). Ended? = `WaitForSingleObject(h, 0)` · confirmation = `WaitForSingleObject(h, 200 ms)`. Ending = HEAD's `terminate_raw` · `LEFTOVER_EXIT_CODE = 0x7440`.
- **Join notification port:** `CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1)` → `SetInformationJobObject(job, JobObjectAssociateCompletionPortInformation, {CompletionKey: 1, CompletionPort})` (the same call as harness `probe/src/main.rs:373-390`) · `next` = `GetQueuedCompletionStatus` → if key 1 · value 6, `Joined(ov as u32)` (not dereferenced) · anything else `Other` · `WAIT_TIMEOUT` → `Timeout` · any other false → `Err`. `watch_births` creates the port, passes one `Arc` to `start` and keeps its own until attaching is done (so even if the thread ends in between, the port handle value is not reused). `unwatch_births` = the same association call with `CompletionPort = NULL` (the detach that MS Learn describes). The constant `JOB_OBJECT_MSG_NEW_PROCESS = 6` (`Win32_System_SystemServices` is not turned on).
- ★**The cleaner's port acquisition — the drainer first (confirmation round codex BLOCK #2)**★: one `OnceLock` per incarnation — the first opener of a new episode makes a `(tx, rx)` pair and calls `watch_births(|port| 듣는 스레드 기동(port, rx, …))`, then reports the result with `tx.send(붙었나)`. The API enforces the order: **port → start the thread (it holds the port and waits on `rx`) → attach the Job only if the start was `Ok`**. So at the moment notifications can start to pile up, a thread that will take them out is already up.
  - Thread start failure → **do not attach** → hardened (`Failed(port)`) — there are no notifications to pile up.
  - Attach failure → hardened · the thread receives `false` and ends (it was never attached so there is nothing to detach).
  - The thread ends only on `false` — on `true`, or if the sender vanished (the opener panicked after attaching and before sending — it may have attached), it runs as usual (if it did not attach, it only spins on timeouts until the weak handle · marker is seen — incarnation lifetime).
  - The code blocks a second `watch_births` (it does not rely on the OS refusing it). ★Only a real OS failure hardens, and the hardened failure is the atomic cell `Recorder.port_failed` (the opener's ⓐ looks at it together with the `OnceLock` value)★ · the listening thread's failure mode → hardened (§3-2) · the channel already having vanished (weak handle) does not harden.
- **What MS Learn says (relied on as is):**
  - 「Processes added to a job at the time a completion port is associated are also reported.」 — ★in the first episode the existing members are reported again (re-notification)★ → if alive they are in W and link-only, and if the same as the snapshot's (pid, creation) they belong to the snapshot side — but they consume that many `BIRTH_MAX` slots and things to take out (§8 ㉒ ⓑ · G1 measures).
  - 「it is best to associate a completion port with a job when the job is inactive … you may miss messages for processes whose states change during the association」 — a birth at the moment of attaching can be missed → a miss.
  - 「messages are intended only as notifications and their delivery to the completion port is not guaranteed」 — a dropped join notification → an unknown link · not a candidate → **a miss (not a wrong kill)**.
  - 「you cannot guarantee that this process is still active or that the identifier has not been recycled … unless you maintain an open handle」 — open and hold at once · reuse before opening is filtered by the creation order (§3-7).
  - 「If the job is nested, the message is sent to every I/O completion port associated with any job in the parent job chain」 — when reported again the (pid, creation) is the same so it is recorded once.
  - Giving the port as `NULL` undoes the association (Windows 8 or later — a documented detach).
- **Neutral handle · channel:** `ProcessGroup { job: Weak (cfg windows), retiring: RetiringSignal }` · `member_pids` · `pin` · `watch_births` · `unwatch_births` · `retiring` · non-Windows shell = list `Ok(빈)` · holding `Ok(None)` · port `Err(Unsupported)`. `process_group()` = `ProcessGroup::new(Arc::downgrade(&job), RetiringSignal::of(&self.retiring))`. Held handles · the port · the marker do not hold the Job (except what the listening thread briefly upgrades each round — §8 ⑲).

### 3-6. Clock

We do not use a comparison with the wall clock · telling 「Esc 뒤에 태어났나」 (was it born after Esc) by time (decision 5). 「끊기 뒤 탄생」 (a birth after the interrupt) = a join notification that arrived after the record was turned on (a candidate is outside the write list W) · 「끊기 전부터」 (from before the interrupt) = the snapshot · W. The kernel creation time is used only to **compare the two ends of one link (parent · child) with each other** and for rule 4's taskkill ≥ T — it is a [Pick] and not a user decision (decision 5 settled only that Esc is not compared with a time). If the clock jumps backwards between two creations, a true parent reads as 「자식보다 늦게」 (later than the child) and is **missed** (the safe side). The monotonic time is used only for N · `waited_ms` · `terminate_max_us` · the confirmation wait.

### 3-7. PID reuse · checked against the research §4 reasoning rules

| Gap | What blocks it |
|---|---|
| Is the candidate ours | only from our Job's join notifications · `IsProcessInJob(우리 Job)` by the opened handle · parent link · W only narrows |
| Reuse between notification and opening | open at once (harness 0/4322) · discard if outside the Job · if it is another new process inside the Job it is recorded with its own facts |
| ★The recorded parent number points at a wrong process (splicing the chain)★ | a held number is not reused after it is held. If the reuse happens **before** holding (an ancestor that ended before the snapshot · a parent that ended before opening), the number's new owner was born later than the ended true parent — that is, later than the child → usually **parent creation > child creation**, so that link is not known → a miss. ★The check is 「부모 ≤ 자식」 (parent ≤ child), so if it is the **same tick** (the reuse happened within the same time tick as the child's creation) it passes — it is not 「엄격히 늦게」 (strictly later) (ADR-0257 decision 9 「같은 눈금 포함」 (same tick included) · landing fix 2026-10-01 · §8 ㉔ K4)★. What remains = that same tick · the case where the clock jumps backwards between the two creations and the order is reversed (§8 ⑰ ⓑ) |
| After opening ~ ending | the same handle (with the end permission already at record time) |
| A middle link born after the interrupt that dies before the sweep | the birth record holds it and keeps its facts |
| Turn end · new interrupt · new input between the re-check and the end | one door-lock section |
| Channel shutdown | the marker is set at the start of `kill_agent` · the re-check reads it · an overlap right after is harmless |
| taskkill points at an old top | taskkill creation ≥ T creation is required — this filters out the shape where an old taskkill points at a T after T's number was reused |
| Two Escs try to open an episode together | the opener is one (`opening` · drop guard) · record number comparison (`active == R` — otherwise only the line, with no episode) |
| A join before / after the line was written | ★write list W (§3-2)★ — something that joined before the write and is alive at sweep time is in W (list completeness · no coming back to life) · if a post-write birth reuses a W number it drops out (a miss) · a birth between the write and W also drops out (a miss) · if the list fails, that write cannot be the first write (no sweep until a later interrupt's W becomes the first write). It does not rely on when notifications pile up |
| Notification loss · the moment of attaching | an unknown link → a miss |
| claude · wrapper · console host | in the snapshot · the walk stops at claude · more than two root children gives `Failed(shape)` |

### 3-8. Logs

Agent attribution = `control.map(|c| c.agent_id)` (ADR-0217) · if none, `root_pid`. All after releasing the locks.

| Event | Level | Fields |
|---|---|---|
| Cleaned up | **warn** | `agent` · `root_pid` · `waited_ms` · `terminated` (PID · parent · `chain` · round) · `rounds` · `terminate_max_us` · `unconfirmed` · `births` · count dropped per rule |
| Could not end (§3-4 reason · `rounds_exhausted`) | **warn** (`reason`) | value per reason |
| Port · listening thread failure (start failure — not attached · attach failure · detached · could not detach and only discards) · write list failure | warn — once | `agent` · `: {e}` |
| ★A caught panic — listening thread · write-confirmation callback · worker★ | ★**error**★ (logging convention — a panic is an error even when isolated · landing fix 2026-10-01 · the 14th edition wrote warn) · ★a release build has `panic = "abort"` so only debug · tests catch it★ | `agent` · message |
| End error · still the same N seconds after the cleanup · worker start failure | warn | `agent` · … |
| Episode opened (including the number of re-notifications for the first episode) · was written (W size · write → W delay µs) · join → open delay · no line · `stale` (only the line) · an interrupt that came while opening · record overflow · `retiring` · `delivered` · `superseded` (sweep stopped) | debug | `agent` · reason |

G2 checks that `resolved_pid` (`session_file.rs:171-180`) is not in `terminated`.

### 3-9. What is not affected

codex · terminal mode (`begin_retire` default no-op) · frontend · wire types · bus (decision 1) · the input queue's user chunks (no callback) · pty channel code · the two turn-observation points (ADR-0127) — unchanged.

---

## 4. Invariant cross-check

| Principle | In this design |
|---|---|
| **Core isolation** (ADR-0003) | all in `agent` · tauri 0 |
| **Backend extension** (ADR-0004) | claude knowledge (interrupt line · turn end · `started` · hook runner · taskkill shape · what to record · clean up) only in `backend/claude`. The channel has only a weak handle · a read-only marker · a 「쓰였을 때 부를 것」 (callback for when it is written) · a neutral `begin_retire` · the manager only calls `begin_retire` and does not know the backend |
| **Platform neutrality** (ADR-0230) | the OS branch = `platform/windows.rs` · `process_group.rs` · the channel accessor · the rules are pure · no `cfg!(windows)` on the calling side |
| **Lock order** (ADR-0006 · ADR-0231) | two new leaves (door · record) — not held overlapping · write confirmation · the listening thread · `begin_retire` are called with no lock (the first line of `kill_agent` has already released the registry lock) → no edge with existing locks |
| **Ownership split** | the Job · `shutdown` · `retiring` cell belong to the transport — to the outside only `Weak` · a read-only marker · a port attached once. The backend holds the list · members · births during an episode and ends them one by one → the CLAUDE.md ownership line (④). The kill causality (ADR-0001) is unchanged — `begin_retire` only sets one cell |
| **ADR-0238** | `TurnGate` → `GateCell` (decision 3) · interrupt record (decision 5) · `InterruptOut` (decision 2) · open/close · rejection as is · if the turn closes between sections, no line |
| **Bottom crate** (ADR-0175) | `base` unchanged |

---

## 5. Tests (ADR-0012)

**Pure — all OSes (fake port · clock · marker · the fake `Pinned` records on drop whether two locks were held):**

| Subject | Case |
|---|---|
| State machine · invariants | after any transition sequence (① · ② · write confirmation · `open_turn` · `deliver` · `close_turn` · step · sweep end · panic) all the §3-2 invariants (including 「`written` 이면 `w`」 (if written then w)) |
| Two sections · opener | `close_turn` → ② `None` · ★`deliver` · `active ≠ R` · another episode → give only the line with no episode · `stop(R)` · discard the snapshot (after releasing)★ · two Escs interleave: the second ① sees `opening` and gives only the line (record · snapshot 0) · a third Esc after the opener's ② is appended · ★opener guard: panic · early return · no line · only the line — on every exit lower `opening` · `stop(R)` · discard a half-taken snapshot · both outside the lock★ |
| ★Write confirmation · write list W (14th edition)★ | if not written there is no sweep · after the confirmation takes W, `last_mono` · worker · ★list failure · handle vanished → the first write does not stand (no sweep · warn)★ · ★a birth that joined before the write and is alive → in W → not a candidate (even if the fake port releases that join **after W** — this measures that it does not rely on notification timing)★ · ★a birth that joined between the write and W → in W → drops out (the miss window)★ · a join after W → a candidate · a W number ends and a post-write birth reuses it → drops out (a miss) · something that joined before the write and ended before W → link-only · the second interrupt's W is discarded (after releasing) · W is taken outside the door lock and discarded outside · the callback panics → error · the queue keeps writing |
| Snapshot | `Failed(list · root · too_many · shape · port)` · the record is turned on before the snapshot · an appended interrupt has list · record-on 0 |
| Birth record | a join while recording → `pin(kill = true)` → `seq` · otherwise → discard · ended → discard · ★`pin(kill = true)` `Err` → link-only (`killable = false`) · not a candidate · it joins the chain★ · ★same (pid, creation) once · same pid with a different creation separately★ · over 512 → `full` · ★`start` clears `full` and discards the leftover records outside · the sweep's copy is empty if `active ≠ rec` · the episode is alive but the record vanished → `snapshot_failed(port)` · it runs even if the record lock is poisoned★ · no OS call · handle closing inside the record lock · weak handle vanished · marker → release the record and end · the listening thread does not wait while holding the upgraded `Arc` (record the strong count while the fake port waits) |
| ★Listening thread failure mode (new)★ | `next` error · round-body panic → `port_failed` hardened · `stop(active)` · log once (`next` error → warn · panic → error) · `unwatch` → if it succeeds, end · ★if it fails it does not end and only takes out and discards (record 0) — ends at the weak handle · marker★ · the next episode's ⓐ sees the hardened cell and gives `Failed(port)` |
| ★Port acquisition — the drainer first★ | only the first opener calls `watch_births` · no second call (code) · ★the fake Job records the call order: port → start the thread → attach★ · ★start failure → attach is not called · hardened★ · ★attach failure → hardened · the thread receives `false` and ends (detach not called)★ · the sender vanishes → the thread runs · the weak handle vanishing does not harden |
| ★Rules — spike shapes★ | p1 · p3t3 · pilot shapes → `Cleanup` · p3bg shape → round 1 owner `Cleanup` → confirmation → round 2 fork 24776 `Cleanup` · ★no taskkill birth · `/PID` is a different number · parent is not claude · taskkill creation < T creation → `NoTaskkill`★ · ★parent creation > child creation (imitating reuse) → `ChainCut`/`ParentUnknown`★ · middle-link notification loss → `ChainCut` · the repetition's `sleep` → `NotHookCopy` · the repetition bash is alive → `AncestorAlive` · taskkill → `ParentAlive` · a subshell after the Bash tool chain is wiped out → `NotHook` · cycle · 17 steps · k = 1 · root first → `ChainCut` · launcher shape → `NotHook` |
| Sweep rounds | ends in `seq` ascending order · ★one round's confirmation shares one deadline (200 ms) (with the fake clock the sum of round waits ≤ 200 ms)★ · did not end within the deadline → alive in the next round · what is below it drops out · exceeding 3 rounds → `rounds_exhausted` · if there is no `Cleanup` it stops · ★re-check refused (turn end · new interrupt · new input · marker) → discard the remaining candidates and the next round and end the sweep (debug)★ |
| `is_hook_command` · `taskkill_names` | runner · wrapper → true · Bash tool · `shell-snapshots` · `node` · `.cmd` · `-lc` · empty field → false / `taskkill.exe /PID 35688 /T /F` → 35688 · missing `/T` or `/F` · another executable → none |
| Termination marker · new input · root re-check · panic guard · end time | as in the 11th edition · ★no method to set it on `RetiringSignal`★ |
| decoder wiring (backup `decoder_with_gate` `:5692`) | progress · `result` · a second `started` → discard · the opening `started` is one section · resume ignored |

**Real processes — each test `#[cfg(windows)]` · four or fewer (including the console host) · helper = HEAD `process_group.rs:176`:**

1. List (HEAD `:498` · `:456` — the helper via `pin_member`'s `exited()`). 2. Ending a held member · exit code · `wait_exit` (HEAD `:346`). 3. Another Job · only the parent Job → `Ok(None)` (`:405` `:425`). 4. `:381` removal. 5. `#[ignore]` Git Bash — an MSYS fork child (`:527`).
6. Facts — a `cmd` X with a door: parent = the test process · creation ≥ the test process's creation · executable · command line (`QUERY_LIMITED`). 7. Join notification — the test holds the port that `start` received · ★if `start` is `Err`, it does not attach (a `ping` started afterwards does not arrive on that port — `Timeout`)★ · if `Ok`, after attaching X (re-notification) is `Joined` · the `ping` that X started `Joined` · the end `Other` · after `unwatch_births` a new member does not arrive. 8. `#[ignore]` Git Bash — a script (exec) under the wrapper `bash -c "bash x.sh"` · a subshell (a copy) · `sleep` (not).
9. Channel — HEAD `:725` without a clock (100 ms kept) · ★`begin_retire()` · `shutdown()` set `retiring()`★ · if the channel is dropped, the list is empty · holding `Ok(None)`. 10. Write confirmation — once after writing · not called if closed · no `push` deadlock inside the callback · the callback panics → the next chunk is also written.
11. manager — `kill_agent` calls `begin_retire` before permission revocation · `tear_down_failed_activation` calls it on the first line after the marker comparison (the fake channel records the call order).
12. `leftover.rs` real port — a B started by R (`cmd` with a `CREATE_NO_WINDOW` door) starts P (ping) after the interrupt and ends → P survives · `not_hook_copy` · only births before the interrupt → `no_new_member` · a child after the interrupt whose parent is alive → `parent_alive`. 13. `#[ignore]` MSYS repetition · pipe → survives.

★There is no real-process test of the ending path★ (under the claude hook-runner shape the orphaned copies exceed four) — pure tests (spike shapes) + §7 G1. `cargo test -p engram-dashboard-agent -- --test-threads=4` · CI runs everything except `#[ignore]`.

---

## 6. Implementation order — the build stands wherever it stops

One coder · sequential. **First:** `git diff HEAD -- crates/` must be empty.

**Contact points:**

```rust
pub(crate) struct ProcessFacts { pub ppid: u32, pub create: u64, pub image: String, pub cmdline: String }
pub(crate) struct RetiringSignal(Arc<AtomicBool>);                          // is_set() 만 · 만드는 것은 통로
pub(crate) trait Pinned: Send + Sync { fn exited(&self) -> io::Result<bool>; fn wait_exit(&self, d: Duration) -> io::Result<bool>;
    fn facts(&self) -> &ProcessFacts; fn terminate_raw(&self) -> io::Result<()>; fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill>; }
pub(crate) trait Births: Send + Sync { fn next(&self, wait: Duration) -> io::Result<PortEvent>; }   // Joined(pid) · Other · Timeout
impl ProcessGroup { fn member_pids(&self) -> io::Result<Vec<u32>>; fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>>;
    fn watch_births(&self, start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>) -> io::Result<()>;   // 포트 → start → Ok 면 붙이기
    fn unwatch_births(&self) -> io::Result<()>; fn retiring(&self) -> RetiringSignal; }
// transport
pub trait AgentTransport { /* … */ fn begin_retire(&self) {} }
pub type OnWritten = Box<dyn FnOnce() + Send>;
pub struct InterruptOut { pub bytes: Vec<u8>, pub on_written: Option<OnWritten> }
pub type InterruptLine = Arc<dyn Fn() -> Option<InterruptOut> + Send + Sync>;
// InputQueue: Inner.pending: VecDeque<(Vec<u8>, Option<OnWritten>)> · push(b) = push_with(b, None) · push_with(b, cb)
//             pop() -> Option<(Vec<u8>, Option<OnWritten>)> · drain: write → mark_written → catch_unwind(cb()) (패닉 = error · 계속 — §3-8)
```

**Assembly (backup `claude/mod.rs:443-449`):** `GateSlot` → `stream_decoder` → `StdioTransport::open` → `leftover_cleaner` (`process_group()?` · `None` if the root is 0 · `Cleaner::new(port, clock, root_pid, retiring, agent)` — the port · listening thread at the first episode) → `GateCell::new` · `slot.get_or_init` → `with_interrupt(interrupt_line(gate))` → `start()`.

**From the backup (`leftover.rs`):** kept = `INTERRUPT_LEFTOVER_GRACE` · `ProcessPort` (modified) · `LeftoverClock` · `SystemClock` · `Phase` · `Episode` · `GateState` · `Accepted` · `Step` · `Ticket` · the transitions · `Commit` · `Cleaner` · `GateCell` · `run_worker` (panic guard) · `Stop` … `PassReport` · `run_pass` (rounds · order swap · the confirmation section keeps the shape of `:1173-1191`) · `log_*`. Removed = the clock · shape · the old picking and its tests. `claude/mod.rs` = same as the 11th edition.

| Stage | Content | Why it stands if stopped here |
|---|---|---|
| ① | the order below | the names that change are called by no one except stage-1 tests · HEAD `interrupt_line` · the first line of `kill_agent` |
| ② | `leftover.rs` + tests · `mod leftover;` | no wiring |
| ③ | wiring · removing `TurnGate` · `started` · anchors · HEAD tests `:6070-6137` | behavior changes for the first time |
| ④ | the new ADR · the ADR-0257 partial-discard stamp · ADR-0238 link · CLAUDE.md (ownership · `TurnGate` → `GateCell` · two new leaves in the lock order · two features in 「의존성」 · `begin_retire`) · `session-path-ownership.md:160` | `/review doc` |

- **The order of ① (each step compiles on its own):** 1. remove `root_attached` — the channel cell · block · `ProcessGroup::new(job, root_attached)` → `new(job)` · `detached()` · `root_attached()` and ★the two test lines that asserted it (HEAD `process_group.rs:144` · stdio `:768`) in the same step★ 2. remove `console_wrapper_depth` 3. fix stdio `:725` to be clock-free and then remove the three `process_tree` ones 4. two `Cargo.toml` features + `windows.rs`'s `pin_member` · facts · `watch_births(start)` · `unwatch_births` · `BirthPort{next}` · tests 6 · 7 · 8 5. `begin_retire` (trait default · `StdioTransport` cell · first line of `shutdown` · `Session` · first line of `kill_agent` · `tear_down_failed_activation`) + test 11 6. `process_group.rs`'s `new(job, retiring)` · `pin` · `watch_births` · `unwatch_births` · `retiring` · `Pinned` · `Births` · `RetiringSignal` + **after** moving the HEAD tests (`:346` `:405` `:425` `:456` `:527`) and helpers (`identity` · `parent_is_dead` · the stdio test's `known` · `still_running`) over to `pin`, remove `verify` · `Verify` · `ReadyKill` · `verify_member` · `MemberCheck` · `:381` + test 9 7. `InterruptOut` · the queue shape (`push_with` · the `pop` pair · `drain`'s `catch_unwind`) + HEAD `interrupt_line` becomes `InterruptOut { bytes, on_written: None }` · ★stdio tests `:589` · `:615` (`pop` gives a pair) · `input_queue.rs` tests `:391` · `:410`★ · test 10 8. header doc.

---

## 7. Verification plan

**Machine gates (`/qa`):** agent tests (`--test-threads=4`) · workspace regression (including manager tests) · `cargo fmt --check` · core-isolation `rg` · generated-file sync.

**GUI measurement (`/qa full` · the real dashboard · claude · hooks):**

| # | Scenario | Pass condition |
|---|---|---|
| G1 | `UserPromptSubmit` hook · Esc about 0.2 s after sending · **10 or more times** | a hung run clears within N + 1 seconds (expected 10/10 · if it does not clear, `reason`) · `terminated` = the hook script's bash (and its copies) · `chain` top = the hook runner · `rounds` · warn 0 on runs that did not hang · the next turn answers · `terminate_max_us` a few ms or less · port failure warn 0 · ★the first episode (the port attached at the first interrupt — the harness attached it at spawn) is also ended★ · ★record: the first episode's birth count · `full` · attach → open delay · ★write → W delay and the margin to the first owner's birth (miss window — §8 ㉓)★ (§8 ㉒ⓑ)★ |
| G1b | Esc with a queued input | the rate discarded by `started` (a miss · §8 ⑱) |
| G2 | log cross-check | no claude PID in `terminated` · exit code `0x7440` · `snapshot_failed` · `root_unknown` warn 0 |
| G3 · G4 · G5 | E-late · B20 · another agent · MCP · build | cleanup 0 · about 20 s · alive |
| G6 | Esc during a Bash tool · daemon MCP · `run_in_background` build · MSYS repetition · pipe — with a hook hang among them | not one background · tool process is ended · only the leftovers are ended |
| G7 | kill the agent during a hung turn | cleanup warn 0 · `retiring` debug |

- If no hang occurs, increase the sample and mark it 「정리 경로 미관측」 (cleanup path not observed) (not PASS). The shape is told apart by the `reason` of the could-not-end warn — all are closed failures.

---

## 8. Risks · known limits

1. **Residual race (ADR-0238 decision 7):** narrowed a little by giving no line if the turn closes between sections.
2. **Missing the turn end:** the next `started` discards it (B2).
3. **A miss (300 seconds as today):** join notification loss · the moment of attaching · a briefly-lived middle link ending before it is opened · the clock jumping backwards between two creations · record overflow · the parent · an ancestor is alive · the chain is not known · hook wrapping · the interrupt method (taskkill) changes · the command line could not be read · `started` of a queued input · port failure · write list failure (no sweep for that write) · end confirmation exceeding 200 ms · fork generations beyond 3 rounds · an owner born between the write and W (㉓) · an owner that reused a W number. None occurred in the 10 measured runs (inference).
4. **Wrapper shape:** a launcher between cmd and claude → everything is missed.
5. **Limits of the snapshot · record:** incomplete list · reuse before opening → the creation order filters it · held count ≤ 256 + 512 · holding time ≈ N seconds · nested-Job re-reports → once only.
6. **The gap of joining right after spawn:** out of scope. 7. **Premises (measured):** the owner = the hook script's bash (a copy, or what the wrapper exec'd) · the chain wiped out · taskkill points at the top. 8. **No replacement:** the follow-up turn 9/9. 9. **Windows only.** 10. **It goes dormant if upstream fixes it.** 11. **taskkill and its conhost:** the parent claude is alive · not a copy.
12. **The cost of closed failures:** snapshot · port · root · worker · panic failures end nothing.
13. **The line does not arrive — closed (write confirmation).** 14. **`TerminateProcess` inside the door lock:** `terminate_max_us`. 15. **The limit of the parent number:** `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` — it ends only if every rule is filled. 16. **The root · claude liveness gap:** ms · checked every round.
17. ★**The remaining wrong-kill gap**★:
    - ⓐ **Window = (the first write's W, the last written interrupt + N + rounds]** — among the joins in it, a hook copy · chain wiped out · top = a hook runner · claude's taskkill pointing at that top — **this is a script · subshell of a hook tree that claude taskkilled and that survived**. Ending it matches the meaning of the interrupt. A Bash tool chain (no taskkill · different wrapping) · exec'd programs · things under a live job drop out. It could get in if claude changed to wrap the Bash tool like a hook and also taskkill it (⑳).
    - ⓑ **Chain splicing (re-evaluation — round 3 F1):** the creation order filters reuse of a parent number. What remains = reuse overlapping with **a clock jump backwards between those two creations**, or ★the reusing new owner's creation being in the **same tick** as the child (the check is 「부모 ≤ 자식」 (parent ≤ child) — the same tick passes · ㉔ K4 · landing fix 2026-10-01)★, and a wrong chain filling every rule — negligible (inference).
18. **A queued input's `started` discards the hung turn (a miss):** G1b.
19. **Channel shutdown overlap:** the marker is set at the first line of `kill_agent` · `tear_down_failed_activation` · the first line of `shutdown`, so the overlap is only once, 「재확인이 읽은 직후」 (right after the re-check read it) · harmless. The listening thread briefly (µs) upgrades the weak handle every round — if the channel vanishes at that moment, the Job handle close (`KILL_ON_JOB_CLOSE`) is delayed by that much.
20. **The shape changes with the claude version:** hooks · Bash tool wrapping · prepending `bash ` · the taskkill method · MSYS command-line copying are undocumented internals (2.1.284) — re-run the provenance spike for each version.
21. **The port is attached once · the drainer first:** it is attached only after the listening thread has risen — if the start fails it is not attached · if attaching fails the thread ends (§3-5). Notifications outside an episode are discarded by the listening thread · if the thread ends by error · panic it detaches and hardens (`Failed(port)`) — ★if detaching fails the thread does not end and only takes out and discards until the weak handle dies or `retiring` is set★ (the queue does not pile up · during one incarnation).
22. ★**List of inferences**★ — ★**What holds up safety (if wrong, something wrong can be ended)**★: ⓐ among claude's direct children, the non-hooks do not use `-c "bash …"` (only this one setting was measured — the taskkill marker is the second wall) ⓑ root's children = conhost + claude (the shape that picks claude — more than two gives `Failed(shape)`) ⓒ a clock jump backwards does not overlap with chain splicing (⑰ ⓑ) — ★the case where the reusing new owner is in the same creation tick as the child is also residual (「부모 ≤ 자식」 lets it pass · ADR-0262 K4)★ ⓓ claude's internal shape (⑳ — undocumented · spike for each version). ★「쓰기 전 탄생은 후보가 아니다」 (a birth before the write is not a candidate) is not here — the write list W (the completeness of the documented API + the held handle) establishes it (§3-2)★.
    **What affects only effectiveness (if wrong, we miss):** ⓔ **the port attached at the first interrupt was not measured by the harness** (the harness attached it at spawn — if, as the documentation says, members already present at attach time are re-notified, the first episode spends `BIRTH_MAX` slots and the queue on re-notifications first · G1 measures) ⓕ join notifications pile up at the place of joining and one thread takes them out in the order they piled up (if late, a briefly-lived link is missed before it is opened) ⓖ the hung owner is not the pipe's only owner (p3bg — the rounds target that) ⓗ opening with the `TERMINATE` permission was measured only on bash owners (if it fails, link-only) ⓘ reading command-line class 60 with `QUERY_LIMITED` is a harness measurement, not a documentation statement ⓙ permissions on other Windows versions ⓚ the write → W gap is much shorter than the owner's birth (㉓). (Detaching the port with `NULL` is a documentation statement so it was left out.)
23. ★**Miss window — between the write and W (confirmation round codex BLOCK #3)**★: something that joined after the line was written and before the writer took W, and is alive at W time, is in W and cannot be a candidate. The gap is the time for the writer to return from the write and make one list call (short — estimate · G1 measures it by the log's write → W delay). Measured margin: the owner is 263–454 ms after the write (p3bg owner 1.4 s · its fork 2.8 s) · a hook fork is at least 1 ms later than taskkill and taskkill is spawned after claude reads the line (main measurement · §3-0 table). Test = the §5 row 「쓰기 확인 · 쓰기 명단 W」 (write confirmation · write list W). ★The 13th edition's 「흐림이 양쪽 다 사라진다」 (the blur disappears on both sides) was wrong on this side — this gap remains, and its direction is a miss★. ★G1 measurement: write → W 6–26 µs · write → owner birth 320–504 ms★.
24. ★**Known limits K1–K9 left by the implementation review (landed 2026-10-01)**★ — user decision 2026-10-01 「일단 재현 가능한것만 구현하자. 좀 현실적인 사항으로 단순화」 (implement only what is reproducible first; simplify to something realistic) · 「현실적으로 가능한것 위주로 단순화 하자」 (let's simplify focusing on what is realistically possible): only findings that reproduce or are realistic are fixed, and the theoretical paths are recorded here **without code** (canonical = ADR-0262 「알려진 한계」 (known limits)).
    - **The list of K1–K9 and the realism judgments are kept only in ADR-0262 「알려진 한계」** — the same list is not written in two places.
    - **What was not built · not run:** §5 real-process test 13 (`#[ignore]` MSYS repetition · pipe) was not built because of the decision above · §7 G1b (a queued input's `started` — ⑱) · G3–G6 were not run (measured = G1 · G2 · G7).
    - **An observation unrelated to T-40 (`docs/tracking.md` T-41):** on a new agent's first turn, an Esc during the `UserPromptSubmit` hook does nothing — the door opens only at the first `started` · progress line (once during G1).

---

## 9. ADR candidates · anchors

> ★Landing = **ADR-0262** (2026-10-01) — that ADR set the replace · amend lines below on ADR-0257 · ADR-0238 · ADR-0244 with partial-discard stamps and two-way links. The code anchor `// ADR-0257` was changed to `// ADR-0262` and remains only on `base`'s stage-1 functions and `INTERRUPT_LEFTOVER_GRACE` (N = 3 seconds — ADR-0257 decision 4 · the two anchors side by side)★.

**New ADR — 「claude 끊기 뒤 잔여물은 첫 끊기부터 판까지 Job 가입 알림으로 기록한 탄생 가운데 줄이 쓰인 직후의 멤버 명단에 없던 것으로서, 끊기 전 스냅숏과 탄생 기록으로 claude 까지 끊김 없이(부모가 먼저 태어난) 알려진 사슬이 모두 끝났고 훅 사본이며 꼭대기가 claude 가 taskkill 한 훅 실행기인 것만 끝낸다 — 벽시계는 쓰지 않는다」** (title in English: after a claude interrupt, end only the leftovers that, among the births recorded by Job join notification from the first interrupt to the sweep, were not in the member list right after the line was written, whose chain up to claude — known without a gap through the pre-interrupt snapshot and the birth record, with parents born first — has all ended, that are hook copies, and whose top is a hook runner that claude taskkilled — the wall clock is not used) (`/adr` numbering). Partially supersedes ADR-0257 · amends ADR-0238 decisions 2 · 3 · 5 · the ADR-0244 decision 2 amendment is kept.

- **What is replaced (ADR-0257 line numbers = this branch's HEAD):** the `:11` 「맥락」 incident-basis line (→ the 14th edition's link is chained only between held handles) · the means of decision 3 at `:26` (「Job 명단 + 생성 시각」 (Job list + creation time) → Job join notification + held handle) · the clock of decision 4 at `:27` (N = 3 seconds stays) · the judgment means of decision 9 at `:32` (creation-order parent judgment → decisions 9 · 10 · 11 + parent ≤ child creation on every link) · all of 「구현 요지」 (implementation gist) `:35`–`:39` · the ADR-0238 decision 3 amendment text at `:42` (→ `GateCell`) · the basis line of 「문턱 뒤 생성」 (creation after the threshold) in 「거부한 대안」 at `:58`–`:60` · 「`push` 실패 되돌리기」 (reverting a failed push) at `:65` (→ write confirmation · write list W) · the 「근거」 raw-material line at `:71` · the `TurnGate.state` leaf line at `:76` (→ two leaves — §3-3) · the closed-failure list at `:77` (→ the §3-4 sweep rounds) · limits ② ③ ⑤ ⑬ at `:78` · the `base` line at `:80` · the code-anchor list at `:81` (`// ADR-0238` → `GateSlot`).
- **Re-read but the text stays:** `:25` decision 2 「적은 시각 뒤에 생성」 (created after the recorded time) = read as a join that was not in the member list (W) right after the line was written (the user's wording stays and the new ADR writes how to read it) · `:28` decision 5 「OS 조각 작게」 (keep the OS piece small) = the piece grew to list · holding · facts · port · ending, but it is all in `windows.rs` in one place · no-op on other OSes · `:77` 「한 에피소드에 판은 한 번」 (one sweep per episode) = one sweep contains up to 3 rounds.
- **Unchanged:** decisions 1 · 6–8 (notice · replacement · off switch) · decision 9's 「부모가 살아 있으면 건드리지 않는다」 (if the parent is alive, do not touch it).
- ★**Reconciling with `:55` 「PID 부모 연결로 고르기(「부모 뒤 생성」 검사를 더해도)」 (picking by PID parent link (even if a "parent-later creation" check is added))**★ — that rejection was about the incident of **chaining even PIDs outside the Job by parent number** (someone else's `link.exe` · 4 of 21 reuses). The 14th edition ① takes candidates only from our Job's join notifications (outside the write list W) ② chains links **only between held handles (snapshot · births of the same record)** and ③ the parent ≤ child creation check **only narrows** candidates — there is no place where the parent link increases candidates. So it does not overturn the rejection and stands inside it.
- **One more ADR-0238 amendment line:** `:66` the lock-order line 「새 락 간선 없음」 (no new lock edges) → 「새 잎 둘(문 · 기록) · 겹쳐 잡지 않는다 · 쓰기 확인은 라이터 스레드에서 명단을 락 밖에서 찍고 문 자물쇠만 잠깐 · 기존 락과 간선 없음」 (two new leaves (door · record) · not held overlapping · the write confirmation takes the list outside the lock on the writer thread and takes only the door lock briefly · no edge with existing locks).
- **Decision [User]:** §1 decisions 5 · 6 · 7 · 9 · 10 · 11 · decision 8 is replaced by 11. ★The creation-order check of links · rule 4's taskkill ≥ T are a [TRD] pick and not a user decision (the main session told the user)★.
- **Rejected alternatives:** wall clock (5th edition) · take one snapshot and compare (7th–10th editions — loses a link that was born after the interrupt and died · decision 11) · the 6th edition's emptying with several threads · parent death alone · is the dead parent bash · one layer of grandparent · collecting between Esc presses too · exit handling · [TRD] a table at sweep time · Toolhelp parent · ending even when the chain is cut · `pid_alive` · re-taking at every Esc · taking inside the door lock · a protected set · wrapper shape · cumulative join count · comparing only the executable · environment variables (PEB) · chaining by stream events (no PID) · an episode age cap · attaching the port at spawn and always listening (decision 7) · writing only the PID at notification time · ★telling 「Esc 뒤에 태어났나」 (was it born after Esc) by creation time (decision 5)★ · ★having the writing side record the candidate boundary (`next_seq`) at write confirmation (12th edition — notifications that piled up in the queue earlier but have not yet been taken out get mixed in after the boundary)★ · ★drawing the boundary by putting a marker packet on the same port (13th edition — there is no documentation that 「가입하는 자리에서 쌓인다」 (they pile up at the place of joining), so it cannot be a basis for safety · replaced by the write list W)★ · starting the listening thread after attaching (up to the 13th edition — start failure + detach failure leaves a queue that nobody takes out) · setting up an episode with a stale snapshot (12th edition — it is not set up and only the line is returned) · a hook-copy check on every link (nothing to gain) · ending everything at once without confirming after the end (misses p3bg's fork generation) · the manager directly setting the backend's door (backend extension).
- **Anchors:** `pin_member` · facts · `watch_births` · `unwatch_births` · `BirthPort` · `ProcessFacts` · `RetiringSignal` · `AgentTransport::begin_retire` · the first line of `kill_agent` · `ProcessGroup` · `StdioTransport::process_group` · `InterruptOut` · the queue's callback · the `leftover.rs` header · `GateCell` · `Recorder` · the listening thread · the opener · the two sections · the snapshot · `note_written` (write list W) · `deliver` · the sweep rounds · end confirmation · `select` · `is_hook_command` · `taskkill_names` · the panic guard · the `open_spawn` assembly · `interrupt_line`. `TurnGate`'s `// ADR-0238` → `GateSlot`.

---

## 10. User decisions needed

> As of the 14th edition: **one user confirmation** (a dependency change). The creation-order check of links · taskkill ≥ T are a [Pick] — the user was told (not a decision).

1. ★**(Confirm) two new `windows` features**★ — `Win32_System_IO` (completion port — the only public means to receive join notifications · decisions 5 · 11) · `Wdk_System_Threading` (`NtQueryInformationProcess` — parent · command line by handle · decisions 10 · 11). No new crate · `Cargo.lock` unchanged.
2. Closed: decisions 1–7 · 9 · 10 · 11 (§1).

---

## 11. Dependencies · places

- ★**Two new features — `Win32_System_IO` · `Wdk_System_Threading`**★ (`windows-0.58.0` — `Win32/System/mod.rs:55` · `IO/mod.rs:34` `CreateIoCompletionPort` · `:70` `GetQueuedCompletionStatus` / `Cargo.toml:398` · `Wdk/System/Threading/mod.rs:34` `NtQueryInformationProcess` · `:192` · `:195`). Not turned on = `Win32_System_Kernel` (replaced by `repr(C)`) · `Win32_System_SystemServices` (replaced by a constant).
- **APIs used:** JobObjects (`QueryInformationJobObject` · `IsProcessInJob` · `SetInformationJobObject` · `JobObjectAssociateCompletionPortInformation`) · Threading (`OpenProcess` · `GetProcessTimes` · `WaitForSingleObject` · `TerminateProcess` · `QueryFullProcessImageNameW`) · IO · Foundation · Wdk.
- **`base` unchanged** — the stage-1 additions remain as bodies of existing public functions (consumers = the `rg` at the end of §2).
- **Fate of the stage-1 code:** unchanged = `member_pids` · `LEFTOVER_EXIT_CODE` · `MemberOutcome` · `terminate_raw` · `GetProcessTimes` use · the channel's `Arc` Job cell · changed = `VerifiedMember` → `PinnedMember` · `ProcessGroup::{new, detached}` · `process_group()` · removed = `verify_member` · `MemberCheck` · `Verify` · `ReadyKill` · `root_attached` (both) · `console_wrapper_depth` · the three `process_tree` ones · those tests · added = `pin_member` · `ProcessFacts` · `watch_births` · `unwatch_births` · `BirthPort` · `Pinned` · `Births` · `RetiringSignal` · `begin_retire` · `InterruptOut` · the queue's callback.
