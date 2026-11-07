# Mid-turn input — M16 vendor measurement (codex answer-end boundary) (2026-09-26)

- **Status:** measurement complete · not yet reflected in the TRD. The result of measuring the **M16** row of `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` §3-3, taken as the task exactly as written. ★This document did not edit the TRD body★ — writing it back into the 「답 구간」 column of the §3-3 · §5-5 interval table is for the next revision (no orphans: a link from the TRD §3-3 M16 row to this document is needed).
- **Consumers:** the TRD §5-5 interval table 「답 구간」 column · §9 P7 (measurement) → P8 (`AtEarliestBoundary`). Implementation plan = `.claude/handoff/attachments/20260926-midturn-impl/plan.md` §5.
- **Preceding reports:** [`mid-turn-phase0-measurements-2026-09-25.md`](mid-turn-phase0-measurements-2026-09-25.md) (spawn arguments · wire format §1 · M7 method §8) · [`mid-turn-phase0b-measurements-2026-09-26.md`](mid-turn-phase0b-measurements-2026-09-26.md) (M10 — `tokenUsage` anchor · record-only branch §3 · ESM trap §4). Not repeated here.
- **Date measured:** 2026-09-26 (KST 03:27–03:41 · log times are UTC 2026-09-25T18:27–18:41)
- **Target version:** codex = `codex-cli 0.156.1` (confirmed in all 5 runs by `thread.cliVersion`) · model `gpt-6-luna` · effort `low`. ★Newer than the TRD's reading baseline (0.154.0)★ — the results and source line numbers below are all 0.156.1.
- **Harness · raw log location (inside the repository · tracked):** `.claude/handoff/attachments/20260925-midturn-phase0/`
  - Harness: `codex_harness.js` scenario `M16` (added this time — existing scenarios are untouched) · summarizer: `m16codex.js`
  - Raw logs: `logs/codex-M16-A-2026-09-25T18-28-48-542Z.jsonl` · `logs/codex-M16-B-2026-09-25T18-32-39-454Z.jsonl` · `logs/codex-M16-C-2026-09-25T18-38-19-822Z.jsonl` (the main run set) · `logs/codex-M16-A-2026-09-25T18-27-15-645Z.jsonl` · `logs/codex-M16-C-2026-09-25T18-37-31-995Z.jsonl` (two pilots — combined into the tables)
  - ★The OS user name in the logs was replaced with `<user>`★ (it was in the work folder and rollout paths). All other fields are as the original.
- **Confidence legend:** certain (a fact printed directly in the raw log, repeated observation) · likely (repeated observation + source reading fit together · the vendor's internal times cannot be seen from outside) · uncertain (inference · single shot)

## 0. Conclusion (first)

1. **Green — all three cases have a window.** A steer handed over in that very handler that receives the answer-end signal got into a later sampling of the same turn in **40/40**, and the model answered that text (A, a long answer without tools, 14/14 · B, the last answer after tools, 12/12 · C, a `plan` turn, 14/14 · +0 ms 20/20 · +1 ms 20/20). The only misses are **the steers deliberately delayed from the signal**, and all of those were written 1.5 ms or more after receiving `thread/tokenUsage/updated` (10/10 `-32600`). → **P8 also holds the answer interval** (TRD M16 row 「녹 = 답 구간도 붙든다」).
2. **Window = the answer-end signal → the `tokenUsage` notification of that sampling + about 0.2–0.4 ms.** The window's length is set by how long after the signal `tokenUsage` arrives, and that value was **A 9.4–66.3 ms (median 30) · B 28.2–96 ms (median 40.6) · C 27.1–66.7 ms (median 36.4)** (all 74 times). ★**Narrowest window = 9.4 ms (A)**★ — this is the value to compare M15's **answer-end reaction latency** against (M15 has not been measured yet — after P2e2). It is wider than the tool-end window (M7 minimum 6 ms).
3. **The `plan` end was reproduced — but in a mode our app cannot reach on its own.** A `plan` item appears only in Plan collaboration mode, and that mode is turned on only through experimental APIs (`capabilities.experimentalApi` in `initialize` + `turn/start.collaborationMode`). Our codex backend sends neither (0 hits for `experimentalApi`·`collaborationMode` in `crates/engram-dashboard-agent/src/backend/codex/`). Since it was reproduced and is green, **`plan` is treated as the same answer-end signal as `agentMessage`** — there is no basis to push it down to 「그 밖」 (other).
4. ★**Three new facts that bear on the design**★
   - **The vendor's final check for pending input is two checks — inside `run_turn` (`core/src/session/turn.rs:555-567`) and, after `run_turn` returns, in the task loop (`core/src/tasks/regular.rs:120`).** If the second check sees pending input, it **runs `run_turn` again with empty input inside the same turn.** That is why a steer written 0.1–0.2 ms after `tokenUsage` also got an answer in the same turn (10/10 · the echo came 27–28.6 ms after `tokenUsage` — separate from the 1.8–3.6 ms of a steer that got in earlier). The place Phase 0b §3 wrote as 「마지막 대기 입력 확인」 ("the last pending-input check") is the first one (§9).
   - **The `plan` item id is reused within a turn** — one `<턴 id>-plan` receives `item/started` → `item/completed` again on every sampling (in C, all 22 turns where a follow-up sampling ran had two pairs with the same id). Any place that holds state by item id must survive 「끝난 id 가 다시 시작된다」 ("a finished id starts again").
   - **A late steer's `-32600` arrived before `turn/completed` again** — 2 times out of 10 (−0.2 ms · −1.2 ms). Same phenomenon as the −0.1 ms of Phase 0b M10 (the TRD's handling, not assuming a `-32600` order, is correct).
5. **There is almost no reader lag ahead of the answer-end signal (material to hand to M15).** In all 74 times, **0** lines arrived in the 20 ms before the signal, and the last delta preceded the signal by 40 ms or more. The signal line comes alone on a quiet pipe — the reader-lag term in the answer-end reaction latency will be small (inference · uncertain — the daemon path was not measured).

## 1. Method

**Harness:** added scenario `M16` to Phase 0's `codex_harness.js`. Spawn arguments and handshake are the same as the preceding report §1 (`codex app-server --stdio -c model=gpt-6-luna -c model_reasoning_effort=low -c notify=[]` · `initialize` → `initialized` → `thread/start`). Only case C carries `capabilities:{experimentalApi:true}` in `initialize` (a separate process — A·B are under the same conditions as our app). One case = one process · one thread · 23 turns (2–3 for the pilots).

**Cases (turn prompts):**

| Case | Turn | Answer-end signal |
|---|---|---|
| A — long answer without tools | 「도구 없이 등대 역사를 150 단어쯤 한 문단으로 · 끝줄 END-k」 (a paragraph of about 150 words on the history of lighthouses, without tools · last line END-k) | the `agentMessage` `item/completed` of that turn · running tools 0 |
| B — last answer after tools | 「`Start-Sleep -Seconds 1` 을 돌리고, 끝나면 도구 없이 80 단어 문단 · 끝줄 END-k」 (run Start-Sleep -Seconds 1, and when it finishes, an 80-word paragraph without tools · last line END-k) | the first `agentMessage` `item/completed` after at least one tool has ended · running tools 0 |
| C — `plan` turn | `turn/start.collaborationMode = {mode:"plan", settings:{model, reasoning_effort:"low", developer_instructions:null}}` + 「질문·도구 없이 README 추가 계획을 `<proposed_plan>` 안에 바로 써라」 (without questions or tools, write the README addition plan directly inside proposed_plan) | `plan` `item/completed` · running tools 0 |

- Running tool = an item of that turn id whose `item/started` has come but not its `item/completed` (output·echo kinds — `userMessage`·`hookPrompt`·`agentMessage`·`plan`·`reasoning`·`functionCallOutput`·`subAgentActivity`·review markers·`contextCompaction` — are excluded). The stdout handler updates it line by line **before** the waiters.
- Steer text = 「마지막에 KIWI*k* 라는 낱말이 든 짧은 줄을 하나 덧붙여라」 (append one short line containing the word KIWI*k* at the end) (a different *k* for each attempt).

**When the steer is written (three modes):**

| Mode | Per case | When written |
|---|---|---|
| `sig` | 12 (+2 pilot) | immediately inside the very stdout handler that delivers the answer-end signal · alternately after a +0 ms / +1 ms busy wait |
| `tok` | 3 (+1 pilot — A) | immediately inside the handler that delivers the first `thread/tokenUsage/updated` of that turn after the signal (an anchor to measure the end of the window — M10 method) |
| `sweep` | 8 | after `setTimeout(d)` from the signal · d spread evenly over 0.3–1.5 times the median of the gaps seen so far (the actual write time is recorded separately) |

**Verdict (per attempt):**
- **Hit (HIT)** = the steer is accepted with that turn id · the `userMessage` echo with that `clientId` arrives with the **same turn id** · the `agentMessage`/`plan` `item/completed` of the same turn after the echo contains KIWI*k*.
- **Miss** = `late` (the steer got `-32600`) · `record-only` (the echo came but there was no sampling after the echo — M10's record-only) · `followup-ignored` (there was a follow-up sampling but no answer) · `no-echo`.
- **gap** = signal received → the first `tokenUsage` received after it (harness monotonic clock). The vendor emits the two notifications in order through the same channel, so the receive interval is taken to be close to the emit interval (likely).

**The window was first bounded from source (0.156.1):**
- The `item/completed` of the answer item goes out from the stream's `OutputItemDone` (`core/src/session/turn.rs` `try_run_sampling_request` `:2583-2692` — `plan` too, in `maybe_complete_plan_item_from_message` `:2247-2273` at the same place). The stream keeps flowing after that until `ResponseEvent::Completed` (`:2828-2877`), clears the running tools, emits `send_token_count_event` (→ `thread/tokenUsage/updated`, `:3042-3050`), and returns.
- First check = `has_pending_input` in `run_turn` (`:555-567` · `needs_follow_up = model_needs_follow_up || has_pending_input`).
- Second check = the `RegularTask` loop (`core/src/tasks/regular.rs:104-124`): after `run_turn` returns, if `has_pending_input`, it calls `run_turn` again with `next_input = Vec::new()` — same task · same turn id.
- Where the window closes = `active_turn.task.take()` in `on_task_finished` (`core/src/tasks/mod.rs:643-650`) — a steer that gets in between the second check and this is record-only (M10), and after it `-32600` (`core/src/session/turn_input.rs:634-638`).
- So the **window = signal → second check**, and its end is right after the `tokenUsage` notification. Per this reading, `sweep` was spread on both sides of the gap.

## 2. Results by case

(ms · relative to signal receipt · pilots combined — the pilots are A `sig` 2 · `tok` 1, C `sig` 2)

| Case | Mode | Hits | When steer written | gap (signal→`tokenUsage`) | Echo (relative to signal) | Misses |
|---|---|---|---|---|---|---|
| **A** long answer without tools | `sig` +0 | **7/7** | +0.0–0.4 | 10.6–39.3 | 12.6–41.3 | — |
| | `sig` +1 | **7/7** | +1.0–1.2 | ↑ same cell (14 times) | ↑ | — |
| | `tok` | **4/4** | `tokenUsage` +0.1–0.2 | 9.4–47.6 | 36.8–75.4 | — |
| | `sweep` | 4/8 | hit +8.6–30.0 · miss +20.7–46.8 | 15.0–66.3 | 33.2–68.4 | `late` 4 — all written at `tokenUsage` +2.3–29.0 |
| **B** last answer after tools | `sig` +0 | **6/6** | +0.0–0.3 | 29.3–80.4 | 31.4–82.4 | — |
| | `sig` +1 | **6/6** | +1.1–1.3 | ↑ same cell (12 times) | ↑ | — |
| | `tok` | **3/3** | `tokenUsage` +0.1–0.2 | 28.2–43.7 | 57.2–405.7 | — |
| | `sweep` | 5/8 | hit +10.6–51.5 · miss +38.1–60.8 | 32.1–96.0 | 48.4–116.3 | `late` 3 — all written at `tokenUsage` +1.5–6.0 |
| **C** `plan` turn | `sig` +0 | **7/7** | +0.1–0.5 | 30.1–56.6 | 32.0–59.2 | — |
| | `sig` +1 | **7/7** | +1.1–1.2 | ↑ same cell (14 times) | ↑ | — |
| | `tok` | **3/3** | `tokenUsage` +0.1–0.2 | 27.1–49.6 | 54.1–77.2 | — |
| | `sweep` | 5/8 | hit +10.5–46.2 · miss +35.3–53.8 | 30.7–66.7 | 32.7–69.5 | `late` 3 — all written at `tokenUsage` +3.2–17.4 |

- **`record-only` · `followup-ignored` · `no-echo` never happened once.** In all 64 hits the echo carried the ending turn's **own** id, and the follow-up sampling said KIWI*k* (A·B = the second `agentMessage` · C = the second `plan`). Certain.
- **B's signal was always `phase: "final_answer"` and exactly 1 tool had ended in that turn.** The pre-tool explanation message (`phase: "commentary"`) never appeared — that signal could not be measured (§7).
- **C's turns had no `agentMessage` at all** — the model output was only the `plan` item (`item/plan/delta` 62–72 → `plan` `item/completed`). The answer-end signal in plan mode is the single `plan`.
- The steer response (`{"turnId"}`) came 0.6–2.1 ms after writing — always before the echo (same as Phase 0 M6).
- Comparison: tool-end window (M7) = 6–64 ms. The answer-end window is wider toward the lower end.

## 3. Window

**Window = signal → second check.** The value is the gap measured per attempt plus about 0.2–0.4 ms.

| | A | B | C |
|---|---|---|---|
| gap min · median · max (ms, number of attempts) | **9.4** · 30.0 · 66.3 (26) | 28.2 · 40.6 · 96.0 (23) | 27.1 · 36.4 · 66.7 (25) |
| latest steer written delayed from the signal that still hit | +47.8 | +51.5 | +49.7 |
| latest steer written after `tokenUsage` that hit | +0.2 | +0.2 | +0.2 |
| earliest steer written after `tokenUsage` that missed | +2.3 | +1.5 | +3.2 |

- **The end of the window is attached not to the signal but to `tokenUsage`.** The 24 `sweep` attempts split along one line — all 14 written before `tokenUsage` hit, all 10 written 1.5 ms or more after got `-32600`. Even with the same delay d from the signal, it hit or missed depending on that turn's gap (A d≈19: gap 15 → miss · A d≈30: gap 61 → hit). Certain.
- **Width of the end = (0.2, 1.5) ms after `tokenUsage` — measured · about 0.2–0.4 ms — an inference combined with M10.** Here `tokenUsage` +0.1–0.2 hit 10/10, and in M10, +0.4–0.6 was record-only (past the second check) · +1.4 was `-32600`. Likely.
- **The second check widens the window (§0.4).** How many ms after `tokenUsage` the echo arrives splits clearly into two branches — a steer that got in before `tokenUsage` takes **1.8–3.6 ms** (first check → the same `run_turn` goes straight into sampling) · a steer written 0.1–0.2 ms after `tokenUsage` takes **27.0–28.6 ms** (once 362 ms — second check → calls `run_turn` anew and prepares sampling again). It fits the reading that a steer that got in after passing the first check was caught by the second check. Likely (the vendor's internal times cannot be seen). One exception — B `sweep` #122 got in earlier but took 20.3 ms (cause unknown).
- **The record-only window was narrow this time too** — never hit once. For a turn that ends without tools, `tokenUsage` → `turn/completed` was 3.0–4.5 ms (same as M10's 3.3–3.6 ms).
- **Value to compare with M15:** compare the answer-end reaction latency (M15 row — answer-end ① maximum + the reader-lag maximum at that time + ② maximum) with the **minimum window 9.4 ms** (A). Looking at B·C only, the minimum is 27.1 ms. Material for reader lag: in all 74 times, **0** lines arrived within 20 ms before the signal · last delta → signal **40.1 ms or more** (A 42.2–314.2 · B 40.1–1455 · C 131.1–251.6) · number of answer deltas before the signal A 203–227 · B 102–113 · C 62–72 (usually one line came as one stdout chunk). The signal comes alone on a quiet pipe — there is little reason for the reader to be lagging from processing the line before it (inference · uncertain — not measured in our daemon reader).

## 4. Verdict (TRD M16 row)

| Case | Hits (right from the signal) | Window (min · median) | Green/red |
|---|---|---|---|
| A long answer without tools | 14/14 | 9.4 · 30.0 ms | **green** |
| B last answer after tools | 12/12 | 28.2 · 40.6 ms | **green** |
| C `plan` turn | 14/14 | 27.1 · 36.4 ms | **green** — reproduced (experimental API conditions) |

- **§5-5 interval table 「답 구간」 column = 「M16 녹 = 쥔다 → 답 끝 신호에 곧바로」** (M16 green = hold → right at the answer-end signal). P8 also holds the answer interval. `plan` is not pushed down to 「그 밖」 — the end signal was confirmed.
- ★**One caveat — M15 can turn this green into a condition**★: if the answer-end reaction latency exceeds the minimum window of 9.4 ms, text held in the turn with the narrowest window misses the second check. That text then falls to record-only (M10 — the turn-end settlement receives it) or `-32600` (to the next turn) — **it is only one boundary late and is not lost** (the two dispositions of TRD §5-5). Since the M15 row leaves this as 「판정이 아니라 견줄 값」 ("a value to compare, not a verdict"), it is not a stop condition.
- **Leaving TRD §5-5 signal ③ (sampling end = `tokenUsage`) as 「너무 좁아 기대지 않는다」 ("too narrow to rely on") was correct** — the width a steer written at that signal fits in is about 0.2–0.4 ms (§3).

## 5. Reproduction

- **Scenario:** `codex_harness.js` scenario `M16`. Environment variables — `M16_CASE` (`A`·`B`·`C`) · `M16_N` (number of `sig`, default 10 · alternating +0/+1) · `M16_TOK` (number of `tok`, default 3) · `M16_SWEEP` (number of `sweep`, default 6) · `M16_K0` (first attempt number = the *k* of the word KIWI*k*). Log name = `codex-M16-<사례>-<stamp>.jsonl`. The main runs were `M16_N=12 M16_TOK=3 M16_SWEEP=8` · `M16_K0` = A 1 · B 101 · C 301 (pilots = A `2/1/0` · C `2/0/0`, `M16_K0` 1 · 201).
- **Run:** `node codex_harness.cjs M16 <로그 폴더> <작업 폴더>` — model·effort are the 5th and 6th arguments (default `gpt-6-luna` · `low`). For the work folder, a scratch outside the repository was used.
- **Summary:** `node m16codex.cjs <로그>... [verbose]` — one line per attempt + per-case hits by mode · gap distribution · hit/miss steer times · the two echo−`tokenUsage` branches · quiet before the signal · `-32600` versus `turn/completed` order.
- ★**ESM trap**★ (same as Phase 0b §4) — the repository `package.json` is `"type":"module"`, so `node *.js` inside the repository dies with `require is not defined`. **Copy it outside the repository as `.cjs`** and run it there.
- **Outside the shell tree:** every run was started with `scripts/run-detached.ps1 -Command <환경변수를 set 하는 .cmd> -WorkDir <스크래치> -LogFile <로그>` and the end was decided by the `__EXIT=0` marker (all 5 runs `__EXIT=0`). One was run at a time.

## 6. Cleanup · cost

- **Processes:** all 5 codex processes the harness started ended by themselves with 0 once stdin was closed (log `exit:0` 5/5 · forced kills 0). After they ended, `Win32_Process` was scanned and this measurement's harness·`app-server --stdio` were confirmed to be **0**. One `detached-cmd-*.bat` that was running at the time was not one of this measurement's wrappers (6 — including 1 failed pilot), so it was not touched.
- **One first pilot had a wrapper with a wrong path, so node could not find the module and died immediately** (`__EXIT=1` · no child codex). It is not included in the results.
- **Traces left (new threads that may be discarded):** 5 codex threads = `%USERPROFILE%\.codex\sessions\2026\09\26\rollout-…-01a0d9d2-5867-…` · `…-01a0d9d3-c230-…` · `…-01a0d9d7-4836-…` · `…-01a0d9db-bfa2-…` · `…-01a0d9dc-79cf-…`. Existing user sessions were not touched.
- **Cost:** 74 codex turns (mostly 2 samplings) · about 14 minutes · 5-hour window 13% → 14%.

## 7. Limits

- **One model · one version · one PC.** `gpt-6-luna` (low) · 0.156.1. In a server state where the model is slow or sends the response end late, the gap would differ — there is no guarantee that this time's minimum of 9.4 ms is a lower bound (it is the minimum within the sample).
- **B's signal is only the first `final_answer` after tools.** The pre-tool explanation message (`commentary` — it ends when no tool is running and the same sampling goes on to call tools) never appeared, so it could not be measured. Text handed over at that signal should, per source, get into the sampling after the tools (it is already in the pending box before the first check — likely · not measured).
- **The answer after parallel tools · long-output tools was not measured** — that side is measured by M15 at the tool end.
- **C is under experimental API conditions.** Our app does not open that mode. ★Whether that mode carries over when our app does `thread/resume` on a thread that was used in Plan mode in the TUI was not checked★ (uncertain) — if it carries over, `plan` appears in our app too.
- **The width of the end of the window (0.2–0.4 ms) is an inference combined with M10.** From this measurement alone it is (0.2, 1.5) ms.
- **It is the harness's reaction, not the daemon's reaction.** The steer **called the write** 0.0–1.3 ms after the signal (not the time the node pipe write actually went out). The daemon's reaction latency is measured by M15.
- **The user's codex hooks settings were not examined** (same as Phase 0b). It is unknown whether hooks account for the 27 ms of the second-check path.

## 8. Summary table

| M# | Result | Green/red | TRD sections affected |
|---|---|---|---|
| M16 | steers handed over right from the signal hit 40/40 (A 14/14 · B 12/12 · C 14/14 · +0 20/20 · +1 20/20) · window = signal → `tokenUsage` + ~0.2–0.4 ms · minimum **9.4 ms** (A) · median 30–41 ms · the only misses were those written ≥1.5 ms after `tokenUsage` (10/10 `-32600`) · record-only 0 · `plan` reproduced (experimental API) · **the last check is the task loop (`regular.rs:120`)** · `plan` id reuse · `-32600` before `turn/completed` 2/10 | **green** (three cases) · value to compare with M15 = 9.4 ms | §3-3 M16 row · §5-5 interval table 「답 구간」 column · boundary signals ②·③ · §9 P7 → P8 |

## 9. Where it departed from the preceding documents (quotes)

1. Phase 0b §3 「소스(0.156.1)」 — 「**마지막 대기 입력 확인 — `core/src/session/turn.rs:555-567`**」 and the harness comment that wrote the same reading (`codex_harness.js` M10 header — 「the last pending-input check is `turn.rs` run_turn (after sampling)」) → that place is the **first** check. The last check is `core/src/tasks/regular.rs:120` (after `run_turn` returns, if there is pending input, it calls `run_turn` again in the same turn), and the record-only window is between that check and `task.take()`. M10's results (record-only · `-32600`) stand — only the name of the window is corrected. Likely (§3).
2. TRD §5-5 boundary signal ③ — 「**도구가 다 끝난 뒤, 대기분 확인 직전에 온다(확인은 그 뒤 2–9 ms — §3-2)**」 → the 2–9 ms is when the **echo** arrives (M7). The check itself passes within about 0.2–0.4 ms after `tokenUsage` is received — a steer written at `tokenUsage` +1.5 ms was already `-32600`. The conclusion 「그 창은 너무 좁아 기대지 않는다」 still holds.
3. TRD §5-5 interval table 「답 구간」 note — 「**M16(미측정)**」 → green (this document). The basis for boundary signal ②, 「M6 T3, 한 번」 → 40/40.
