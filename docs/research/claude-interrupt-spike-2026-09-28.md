# claude stream-json interrupt spike — B2 (S1–S8) (2026-09-28)

- **Status:** measurement complete · before it is reflected in the TRD. These are results measured by **taking the table (S1–S8) and 「S1 기록 추가」 (S1 record addition) in `docs/process/S21-chat-ux/trd.md` §3-5 as the task list, as is**. ★This document did not edit the TRD or ADR bodies★ — linking the slot 「결과 = 새 보고서 `…-2026-09-2x.md`」 (result = new report …-2026-09-2x.md) in TRD §3-5 to this file is for the next revision (no orphans). Main's additional instruction (is a truncated complete `assistant` line one block or several) is written in §6 S5 as well.
- **Measured on:** 2026-09-28 (KST 00:37–00:46 · log times are UTC 2026-09-27T15:37–15:46)
- **Version:** `claude --version` = `2.1.280 (Claude Code)` — `claude` on PATH → `%APPDATA%\npm\claude.cmd` → `…\@anthropic-ai\claude-code\bin\claude.exe`. Model = the CLI default (`model` in `system/init` = `claude-opus-5-5[1m]`).
- **Spawn arguments:** the JSON mode of `build_spec` in `backend/claude/mod.rs` as is — `cmd.exe /c claude --permission-mode bypassPermissions -p --input-format stream-json --output-format stream-json --replay-user-messages --verbose --include-partial-messages --session-id <uuid>` · env `MAX_THINKING_TOKENS=8000` · no control channel (no `--mcp-config` etc.) · no `extra_args`. Same conditions as B1 (`partial_stream_p1`). The environment variables of the parent Claude Code session (`CLAUDE*` · `AI_AGENT` etc.) were cleared — a claude launched by the daemon does not inherit them.
- **Interrupt line:** exactly the shape of ADR-0238 decision 1 — `{"type":"control_request","request_id":"interrupt:<uuid v4>","request":{"subtype":"interrupt"}}\n`. `reason` and `cancel_queued` were not included. ★Every interrupt was written **directly** by the harness — none went through our turn-open gate (which does not exist yet)★. Whether the gate would have been open at that moment is re-measured with the decoder in §9 S8.
- **Method:** a node harness that imitates our spawn arguments and wire format launches the CLI directly, writes a JSON line to stdin at a chosen point (mostly **synchronously inside the stdout handler that received that line**), and records every stdout line together with a **monotonic clock (ms, from harness start)** as `{t,dir,chunk,line}` JSONL (the Phase 0 harness was reused). The harness ran in the background of this worker's shell and its output was received only as files. The app, the daemon and other worktrees' processes were not touched. Not one line of product code was changed.
- **S8 decoder:** a small binary attached to a scratch **copy (snapshot)** of the working tree's `engram-dashboard-agent` · `-command` · `-base` (including B1's uncommitted `claude/mod.rs` — sha256 `8fc220d6…`) called `backend::output_decoder` (the live decoder) + `backend::turn_classifier` **as the real thing**. Since the turn-open gate has no code yet, TRD §3-4's definition (of the events of one line, if any is `Progress` it opens · it closes at the `result` line) was imitated on top of it. The build was run with `scripts/run-detached.ps1` (`__EXIT=0`).
- **Harness and raw log location (session scratch — volatile):** `C:\Users\<user>\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\3bacb436-cde6-47e8-a754-31810509c430\scratchpad\b2\`
  - harness `harness.cjs` (scenarios `s1` · `s3` · `s4` · `s5` · `s6` · `s7`) · summary `sum.cjs` · fixture processing `mkfixture.cjs` · gate trace `s8ws\gate-trace\src\main.rs`
  - raw logs: `logs\claude-s1-2026-09-27T15-37-24-325Z.jsonl` · `…-s3-…T15-41-22-719Z` · `…-s4-…T15-41-53-339Z` · `…-s5-…T15-44-47-625Z` · `…-s6-…T15-42-33-918Z` · `…-s7-…T15-43-28-073Z`
  - ★Only S1 stays in the repository★ — fixture `crates/engram-dashboard-agent/src/backend/claude/fixtures/interrupt_s1.jsonl` (202 lines). 「픽스처 N」 (fixture N) below is the **1-based** line number of that file. The other trials are written by the **0-based** line number of the raw log (in the style 「s3:43」), and the needed lines are excerpted in this document.
- **Confidence legend:** certain (a fact printed directly in the raw log, repeatedly observed) · likely (observed once or twice + fitted with the vendor schema reading) · uncertain (inference · single observation)

## 0. Conclusion (first)

> ★**Stop — one thing the TRD did not expect (display)**★: for every interrupted turn the CLI emits a synthetic user line `{"type":"user", …, "content":[{"type":"text","text":"[Request interrupted by user]"}]}` **before** the `result` (`…for tool use]` if a tool was running). It has no `isReplay` and the uuid is newly drawn by the CLI. It appeared in **all 14 interrupted turns observed** and it also stays in the transcript. Today's decoder emits it as `Structured{user}` and the accumulator draws it as a **user bubble** (`structuredAccumulator.ts:224-244` — the uuid is neither in the pending registry nor in 「본 것」 (seen), so it is drawn as is). So TRD §3-4's 「누산기가 중단 행과 구분선을 그린다 — codex 끊김과 같은 모양 · 프론트·선 타입 무변경」 (the accumulator draws the interrupt row and the divider — same shape as a codex interrupt · no change to frontend or wire types) does not hold — one more 「[Request interrupted by user]」 bubble stands on top of the interrupt row (in the resume history as well). ★Turn observation, the gate and the stop after an error are unaffected★ (it is a progress line inside the turn · before the `result`). Since this falls under the brief's second stop condition, **the claude part of B3 does not start before main decides.** Details = §10-1.

| # | Result | Verdict |
|---|---|---|
| **S1** | mid-text · mid-tool · before the turn opens · a new turn after idle — **all 14 interrupts** got a `result` (9 ms – 1.0 s after the interrupt · an interrupt before `init` waits for that turn's `init`) | ✅ pass |
| **S2** | the interrupted `result` = `subtype:"error_during_execution"` · `is_error:true` · **`terminal_reason:"aborted_streaming"`** (streaming text · thinking · tool input) \| **`"aborted_tools"`** (running a tool) — 14/14. `subtype:"interrupted"` never appeared | ✅ `terminal_reason` tells them apart → **the ④ spare marker is not built** |
| **S3** | response `still_queued:["<B>"]` → right after A's `result`, **B `started`** (new turn) → B answers → `completed`. B has no `cancelled`/`discarded` | ✅ pass |
| **S4** | interrupts right after `started` · before `system/init`, 3 times (including a session's first command) + once right after the echo — **4/4 stopped**. The order in 2.1.280 is `queued → started → init → 되울림` (the echo is **after** init) | ✅ pass — **the point where the gate opens stays as is (`Delivered` at `started`)** · no need to delay |
| **S5** | a truncated complete `assistant` line comes **only when the open block is a text block** — **one** block (`aborted:true` · the same body as the streamed text · without `content_block_stop` · before `result`). For an open tool-input or thinking block, a complete line **does not come at all** | record — **single-block** |
| **S6** | idle interrupts before the first input · after a finished turn — only one `control_response {success, still_queued:[]}` line. The following turn is normal | ✅ pass |
| **S7** | interrupts at +0 · +300 · +800 · +1500 ms after A's `result` — **in 4/4 B's turn was interrupted** (`aborted_streaming` · error row · no stop — told apart by the S2 verdict). The gap A `result` → B `started` = 2.3–3.6 ms | record — accepted (TRD condition met) |
| **S8** | fed six logs (including 14 interrupts) to the real decoder — the lines that open the gate are **all that turn's `command_lifecycle started`**, the lines that close it are all that turn's `result`. Lines that open outside a turn: 0 | ✅ pass |

- **Exit condition (§3-5): met.** S1 · S2 · S3 · S4 · S6 · S8 pass, S5 · S7 recorded. The three invariants we rely on (two turn-observation points · 30-minute fail-open / stop after an error / one stream of pending input) all stand on this measurement — but **the stop after an error stands only once B3's `result` classification is in** (today's code reads an interrupted `result` as `Failed` + `Ended(Clean)` — §9).
- **S1 record addition:** a stream-json interrupt also **kills a running Bash tool** — not one tick increased after the second of the interrupt, and the tool's two `bash.exe` were gone at +3 s. `system/task_notification status:"stopped"`. That tool's `tool_result` = `is_error:true` · body = a **string** (not an array) 「The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.」 — the **same** as the interactive CLI record (TRD §11 ⑪). Details = §2-2.
- **Main's additional instruction — verdict: single-block.** The 23 complete `assistant` lines observed (including 1 truncated) are **all one block**. In this measurement B3 has no need to align `content[i]` to block number i. Details = §6.

## 1. Common — the tail shape of an interrupted turn (14/14 · certain)

Whatever was running at the moment of the interrupt, the tail had the same order:

```
>> control_request interrupt 씀
   control_response {"subtype":"success","request_id":"interrupt:<우리 uuid>","response":{"still_queued":[…]}}   ← +1–112 ms (첫 입력 전 한가 끊기만 385 ms)
   (글 블록이 열려 있었을 때만) assistant {… "aborted":true, content:[열린 글 블록 하나]}
   (도구가 돌고 있었을 때만) system/task_notification status:"stopped" · user tool_result is_error:true
   user {"content":[{"type":"text","text":"[Request interrupted by user]"}]}   ← 도구 중이면 "…for tool use]" · isReplay 없음 · 새 uuid
   result {"subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming"|"aborted_tools", "errors":["[ede_diagnostic] …"], …}
   command_lifecycle cancelled <그 턴을 연 명령의 uuid>                        ← result 뒤 1–7 ms
```

- The response (`control_response`) is **always before the `result`** — it matches the vendor schema description (「on a clean interrupt this receipt is written before the interrupted turn result」).
- After an interrupted turn, `message_delta` · `message_stop` · the `content_block_stop` of the open block **do not come.**
- The `command_lifecycle` of the command that opened the turn is **`cancelled`**, not `completed`, and comes **after** the `result`. In our translation table it is `Dropped{Unknown}`, and since it is an id already tombstoned as `Delivered` it is a **no-op** (`Some(_) => {}` at `queued_input.rs:320-332`) · the turn classifier gives `None` (`claude/mod.rs:566`). It does not reopen the gate (§9).
- Vendor schema (the zod description inside the installed `claude.exe` — reading · likely): `interrupt_receipt_v1` = 「응답 `still_queued` = 이 끊기를 **살아남는** 비동기 사용자 메시지의 uuid — **These WILL run**」 (response still_queued = the uuids of async user messages that **survive** this interrupt — **These WILL run**). Same as the meaning of ADR-0238 decisions 1 · 6 (pending input runs in the next turn). Even though the request's `reason` (optional · `@internal`) was not sent, the tool was closed in the 「사용자 거절」 (user rejected) shape.

## 2. S1 — interrupt while text is streaming · while a tool is running

**Setup (`s1` log · one process):** ① S1a = 「인쇄술의 역사 1500 단어 에세이 · 도구 없이」 (an essay of 1500 words on the history of printing · no tools) → interrupt 2.5 s after the first `text_delta` ② S1b = 「Bash 로 `for i in $(seq 1 90); do echo "<표식> tick $i"; echo "tick $i $(date +%s)" >> <틱 파일>; sleep 1; done` 을 전경으로」 (run the for i in $(seq 1 90) loop with Bash in the foreground) → interrupt when the tick file reached 5 lines after the `tool_use` ③ the later turn = 「Reply with exactly: AFTER-OK」.

### 2-1. S1a — while text is streaming (fixture 1–139)

```
t(ms)      픽스처  줄
10129.5    2       command_lifecycle started 753cc4e5
11092.1    3       system/init
14530.5    5       user isReplay=true (A 되울림)
14531.0    7       stream_event content_block_start #0 thinking   … 생각 ~106 s
120400.4   85      stream_event content_block_start #1 text
120401.3–122656.7  86–134   text_delta 49 줄
122902.2   —       >> interrupt:f453a7a6 씀   (첫 글 델타 2.5 s 뒤)
122905.0   135     control_response {"subtype":"success","request_id":"interrupt:f453a7a6-…","response":{"still_queued":[]}}
122937.0   136     assistant msg_011CfUC9… {"stop_reason":null,"aborted":true, content:[{type:"text", 927 자}]}
122940.0   137     user {"content":[{"type":"text","text":"[Request interrupted by user]"}], "uuid":"f1ab3f56-…"}
122951.4   138     result {"subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming","stop_reason":null,"num_turns":2,"errors":["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=null"]}
122953.7   139     command_lifecycle cancelled 753cc4e5
```

**Verdict: pass** — `result` 49 ms after the interrupt.

### 2-2. S1b — while a Bash tool is running (fixture 140–188) · S1 record addition

```
t(ms)      픽스처  줄
127964.6   141     command_lifecycle started f127a5e7
140151.1   178     assistant tool_use Bash {"command":"for i in $(seq 1 90); …"}
152469.0   182     system/task_started {"task_type":"local_bash","is_backgrounded":false}
155964.4   —       (프로세스 조회) 표식을 가진 bash.exe 2 개 — pid 44372 → 자식 4104
155964.5   —       >> interrupt:deae228f 씀   (틱 파일 6 줄)
155972.0   183     control_response {… "still_queued":[]}
155975.5   184     system/task_notification {"status":"stopped","summary":"Run 90-second tick loop writing to log file"}
155988.4   185     user tool_result {"is_error":true,"content":"The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.","tool_use_id":"toolu_01EG…"}
                            + 줄 수준 "tool_use_result":"User rejected tool use" · "tool_result_meta":[{"id":"toolu_01EG…","non_execution_kind":"user-rejected"}]
155990.4   186     user {"content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}
155997.4   187     result {"subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_tools","stop_reason":"tool_use","num_turns":3}
155998.9   188     command_lifecycle cancelled f127a5e7
+0 … +16.1 s  —    틱 파일 줄 수 = 6 에서 한 번도 안 늘었다(1 s 간격 17 표본)
+3 s · +16 s  —    표식을 가진 프로세스 0 개
```

- **The Bash tool stopped and the process died (certain).** The last tick `tick 6 1790523600` (= 15:40:00 UTC) is the very second of the interrupt (`timestamp` of the `tool_result` = 15:40:00.311Z). Same as Esc in interactive Claude Code (TRD §9 comparison).
- **`tool_result`:** `is_error:true` · `content` is a **string**. The body is the same sentence as in the interactive CLI record. At line level, `tool_use_result:"User rejected tool use"` · `tool_result_meta[].non_execution_kind:"user-rejected"` also come (a structured marker — usable for T-43. This display rule (TRD §11 ⑪) does not rely on it).
- **Verdict: pass** — `result` 33 ms after the interrupt.

### 2-3. The later turn (fixture 189–202)

An ordinary turn after the two interrupts ended as `started → init → 되울림 → 글 → result success(terminal_reason:"completed") → completed`. The interrupt left no state in the CLI.

## 3. S2 — does the interrupted `result` tell interruption apart

All 14 interrupted `result`s:

| Moment of the interrupt | Count | `subtype` | `is_error` | `terminal_reason` | `stop_reason` |
|---|---|---|---|---|---|
| text streaming · thinking streaming · tool **input** streaming · before the turn opens | 12 | `error_during_execution` | `true` | **`aborted_streaming`** | `null` |
| tool **running** (S1b · S3) | 2 | `error_during_execution` | `true` | **`aborted_tools`** | `"tool_use"` |

- The `result` of every normal turn was `terminal_reason:"completed"`.
- ★`subtype` cannot tell interruption apart★ — `error_during_execution` is the same word as a real execution error. `terminal_reason` tells it apart. So of the two cells of `interrupted(result)` in TRD §3-4, the **`terminal_reason` cell is the one that actually works**, and the `subtype == "interrupted"` cell never matched on this installed version (it does no harm to keep it — for older CLIs).
- ★With today's code this `result` is an error★ — since `is_error:true`, it emits `Error(RESULT_FAILURE_DETAIL …)` + `MessageDone`, and the classifier records `[Failed, Ended(Clean)]` (`signals=` in the §9 trace). Until B3's classification is in, a single Esc sets up a stop after an error — exactly as the TRD expected.
- The two existing tests (`result_interrupted_subtype_*` — `subtype:"interrupted"`) are a **hand-built shape**. The measured shape (`error_during_execution` + `terminal_reason`) is in the `interrupt_s1.jsonl` fixture at 138 · 187.
- **Verdict: pass — the ④ spare marker (`interrupt_sent`) is not built** (the condition of ADR-0238 decision 5, 「두 칸 모두 못 가를 때만」 (only when both cells cannot tell them apart), does not hold).

## 4. S3 — interrupt after queueing B during a tool (`s3` log)

**Setup:** A = Bash `for i in $(seq 1 40); do echo tick $i; sleep 1; done` → 3 s after the `tool_use`, write B (「Reply with exactly: B-RAN-S3」) → 1 s after B is `queued`, interrupt.

```
t(ms)     s3:줄  줄
16235.2   40     >> B 씀 (6b15c7c8)
16239.6   41     command_lifecycle queued 6b15c7c8
17239.8   42     >> interrupt:154ce007 씀
17247.5   43     control_response {"subtype":"success","request_id":"interrupt:154ce007-…","response":{"still_queued":["6b15c7c8-41e0-4c9d-a5cc-f3b48654d405"]}}
17258.7   44     user tool_result is_error:true "The user doesn't want to proceed with this tool use. …"
17260.5   45     user "[Request interrupted by user for tool use]"
17266.0   46     result error_during_execution · is_error:true · terminal_reason:"aborted_tools"
17268.1   47     command_lifecycle cancelled 013fa71d   (A)
17269.4   48     command_lifecycle started 6b15c7c8     (B — 새 턴)
17927.4   49     system/init
20657.0   51     user isReplay=true uuid=6b15c7c8 "Reply with exactly: B-RAN-S3"
20685.0   55     assistant text "B-RAN-S3"
21171.0   59     result success · terminal_reason:"completed"
21171.7   60     command_lifecycle completed 6b15c7c8
```

- B was in `still_queued`, was `started` as a new turn 3.4 ms after A's `result`, and answered. B has no `cancelled` · `discarded`.
- In this trial, even though the tool had not yet emitted `task_started`, `terminal_reason` was `aborted_tools` (it seems to be written that way once the tool-call stage has been entered · single observation).
- **Verdict: pass** — the meaning of ADR-0238 decisions 1 · 6 (only the running turn stops and pending input goes to the next turn) is the same as the CLI behavior.

## 5. S4 — interrupt right after a new turn opens · before `system/init` (`s4` log)

**Setup:** A = 「1 부터 150 까지 영어 단어로 한 줄씩」 (English words from 1 to 150, one per line) (it takes tens of seconds if not interrupted) → write the interrupt **synchronously inside the handler that received A's `command_lifecycle started` line** (S4a1 = the session's first command · S4a2 · S4a3 = later turns). S4b = interrupt inside the handler that received **A's echo line**.

```
시행   started   >>끊기    응답      init      되울림     [Request…]  result(aborted_streaming)
S4a1   8457.4    8457.6    8468.6    8506.8    8546.9     8547.4      8572.0      (s4:12–19 · 세션 첫 명령)
S4a2   13595.7   13596.0   13707.9   14554.4   14588.2    14588.6     14594.3     (s4:24–31)
S4a3   19606.2   19606.3   19674.4   20210.6   20235.4    20235.7     20240.4     (s4:36–43)
S4b    25253.4   27310.4   27311.9   26365.2   27310.3    27315.7     27319.9     (s4:48–57 · 되울림 뒤 끊기 — 그 사이 message_start · thinking 블록 시작만 옴)
```

- **All four stopped** — `result` came without any `assistant` line or text delta. Interrupts that arrived **before** `system/init` (S4a1–a3 — the response came before init) were not ignored either. The session's **first** command (S4a1) is the same.
- **Order:** 2.1.280 is `queued → started → system/init → 되울림` — ★the echo comes **after** init★. The premise of TRD §3-5 S4 (「되울림이 init 보다 먼저」 (the echo before init)) differs from this installed version, but the **earliest** line that opens the gate is not the echo but `started` (`Delivered` — the progress signal of TRD §3-4), and the interrupt at exactly that point took effect, so the conclusion is the same.
- Vendor schema reading (likely): an interrupt that reaches the first command's 「prewait window」 is caught as a **latch**, and it is written that 「그 일을 싣고 무장하는 첫 턴이 이미 끊긴 채 시작한다」 (the first turn that carries that work and arms starts already interrupted) (it looks like the fix of SDK issue #429). The later turns (S4a2 · a3) had the same outcome.
- **`system/init` comes every turn** (the record request of TRD §12 round 1 low 1): in all six logs the `init` count = the `result` count (3/3 · 2/2 · 4/4 · 3/3 · 2/2 · 8/8). The `started → init` gap = 49 ms – 5.7 s (mostly 0.5–1.1 s).
- **Verdict: pass — the point where the gate opens is not delayed** (the ★correction★ condition of ADR-0238 decision 3 does not hold · the bit 「마지막 `result` 뒤 init 을 봤나」 (did we see init after the last result) is also unnecessary).

## 6. S5 — the truncated `assistant` line just before the interruption (+ main's additional instruction: one block or several)

### 6-1. When a text block is open (S1a · fixture 85–138)

- The open block = `#1 text` (`#0 thinking` is already its own complete line · closed by `content_block_stop`).
- After the interrupt, **one complete `assistant` line** comes (fixture 136): `content` length **1** = only that text block · `message.stop_reason:null` · line-level **`"aborted":true`** · the 927-character body = **not one character different from the text joined from the 49 streamed delta lines** (`startsWith` true · tail 0 characters).
- Position: that block's **`content_block_stop` never comes** (neither before nor after) · **before the `result`** (a line 34.8 ms later → `result` 14.4 ms after that).
- decoder (ADR-0240): at the moment the complete line arrives, `partial.open == Some(1)` and 1 is the streamed block, so it discards the text — no text is lost (the streamed text is everything). In the §9 trace this line emitted no event.

### 6-2. When a tool-input block is open — after a streamed text block (S5a1 · S5a2 · `s5` log)

**Setup:** 「도구를 쓰기 전에 한 문장으로 알리고, Write 도구로 80 줄 넘는 시를 파일에 써라」 (announce in one sentence before using a tool, and write a poem of over 80 lines to a file with the Write tool) → after seeing text deltas in the same message, the `tool_use` block opens, and the interrupt is sent inside the handler that received 12 `input_json_delta` lines.

```
S5a1 (s5:72–99)
38768.6  72  assistant msg_011CfUChZ2i… [thinking]          ← #0 완결 줄
38769.1  73  content_block_stop #0
38769.3  74  content_block_start #1 text
38770.3–38970.9  75–79  text_delta 5 줄 "poem-S5a1.txt 파일에 시를 작성하겠습니다."
39160.4  80  assistant msg_011CfUChZ2i… [text]              ← #1 완결 줄 (content 1 개)
39160.6  81  content_block_stop #1
39161.1  82  content_block_start #2 tool_use
39161.4–39649.5  83–94  input_json_delta 12 줄
39649.8  95  >> interrupt 씀
39654.0  96  control_response {… "still_queued":[]}
39663.1  97  user "[Request interrupted by user]"
39707.5  98  result error_during_execution · terminal_reason:"aborted_streaming"
39710.8  99  command_lifecycle cancelled 7e3624cf
```

S5a2 (`s5:114–140`) has the same shape — #1 text complete line (121) → `content_block_stop #1` (122) → `#2 tool_use` start (123) → 12 deltas → interrupt (136) → response · `[Request…]` · `result aborted_streaming`. **The complete line of the open `tool_use` block did not come either time** — the tool input being streamed remains nowhere (no `ToolCall` event · the file was not written either). Since the tool had not yet run, `terminal_reason` is `aborted_streaming`, not `aborted_tools`.

### 6-3. When a thinking block is open (S5b · `s5:148–160` · S4b)

S5b = interrupt after 3 `thinking_delta` lines in the message's first block `#0 thinking` → response · `[Request…]` · `result aborted_streaming`. **No complete line.** S4b (the moment the thinking block had just opened) is the same.

### 6-4. Verdict — **single-block**

- **All 23** complete `assistant` lines observed have `content` length 1 (all six logs counted — 6 thinking · 11 text · 6 tool · including 1 truncated).
- A truncated complete line comes **only when the open block is text**, and what that line carries is that one text block. It does not re-carry blocks closed earlier (streamed text · thinking), and for an open tool-input or thinking block it does not carry anything (there is no complete line at all).
- So this measurement gives B3 no ground to align `content[i]` to block number i. What remains uncertain: an interruption in a state where **two or more text blocks are opened in a row** within one message could not be produced (only one block is open at a time, so it is structurally rare).

## 7. S6 — interrupt when there is no turn (`s6` log)

```
s6:9   4014.1   >> interrupt 씀 (첫 입력 전 — 세션에 턴이 한 번도 없었다)
s6:11  4399.1   control_response {"subtype":"success","request_id":"interrupt:bec5b7a2-…","response":{"still_queued":[]}}
                … 8 s 동안 다른 줄 없음(SessionStart 훅 응답 한 줄만 — 잡음)
s6:13–29        턴 1 「Reply with exactly: HELLO-S6」 → started → init → 되울림 → 글 → result success → completed
s6:31  27426.8  >> interrupt 씀 (끝난 턴 뒤 5 s 한가)
s6:32  27490.5  control_response {"subtype":"success","request_id":"interrupt:fbd32172-…","response":{"still_queued":[]}}
                … 10 s 동안 다른 줄 없음
s6:34–48        턴 2 「Reply with exactly: AFTER-S6」 → 정상 success
```

- An idle interrupt emits only one response line — no `assistant` · `user` · `stream_event`. The following turn was not interrupted (an interrupt before the first input did not kill the first turn with a latch — this fits the schema description that the latch applies only to work that was pending at that moment).
- ★The CLI does not reject an idle interrupt★ — it is always `success`. Only our turn-open gate can say 「끊을 턴이 없다」 (there is no turn to interrupt) (exactly what TRD §3-4 assumed).
- **Verdict: pass.**

## 8. S7 — interrupt right after A's `result` · the real size of the residual race (`s7` log)

**Setup:** A = Bash `sleep 3` → **inside the handler that received A's `tool_result`**, write B (「1 부터 60 까지 영어 단어로」 (English words from 1 to 60)) (Phase 0 M7 — it misses folding and becomes a new turn) → after receiving A's `result`, interrupt at +delay. The 「문」 (gate) column = the value the §9 imitation gate would have held, from the lines received up to the moment the interrupt was written.

| Trial | A `result` | B `started` | Interrupt written | Gate then | Response | B `result` | B shape |
|---|---|---|---|---|---|---|---|
| S7-0 (`s7:49–59`) | 14311.6 | 14314.2 | 14311.8 | **closed** | 14354.0 | 14846.4 `aborted_streaming` | echo · `[Request…]` · 0 text · `cancelled` |
| S7-300 (`s7:92–102`) | 28741.1 | 28743.5 | 29041.6 | open (B) | 29067.6 | 29488.0 `aborted_streaming` | same |
| S7-800 (`s7:134–144`) | 43162.4 | 43164.7 | 43961.9 | open (B) | 43962.8 | 43968.6 `aborted_streaming` | same |
| S7-1500 (`s7:177–187`) | 59761.9 | 59765.5 | 61263.4 | open (B) | 61316.9 | 61429.0 `aborted_streaming` | same |

- **In all four, B's turn was interrupted.** B's text shows as the echo (a user bubble), there is not one character of answer, and the `result` is `aborted_streaming` — by the S2 verdict it becomes an **interrupted turn** (error row · no stop after an error). Pending registry: B is closed by `started` (`Delivered`) so the bubble stands in the received slot, and the following `cancelled` is a no-op on a tombstone. → **TRD's acceptance condition (error row · no stop) met.**
- **The gap A `result` → B `started` = 2.6 · 2.4 · 2.3 · 3.6 ms.** If there is a pending B, the gate reopens to B almost immediately at A's end. So a late Esc aimed at A's end effectively interrupts B (in S7-300 · 800 · 1500 the gate was open **for B's turn**, so our implementation would have sent it too — from the back end at that moment it is a legitimate interrupt).
- S7-0 was written at a moment when the gate was closed (0.2 ms after A's `result` · before B `started`) — **our implementation would have rejected it with CONFLICT and B would have run**. The line the harness wrote directly was processed by the CLI **after** B's turn had started (the response came 40 ms after B `started`) and interrupted B. That is, this is the shape in which the residual race of TRD §3-4, 「줄이 A 의 턴에 읽혔지만 CLI 에 닿기 전에 B 가 열렸다」 (the line was read in A's turn but B opened before it reached the CLI), **actually interrupts B** (the vendor line has no turn id).
- **Verdict: record — accepted.**

## 9. S8 — gate trajectory (real decoder · `gate-trace`)

The six logs were fed to the real decoder one line at a time; if any event of a line was read by the classifier as `Progress` the gate opened, and it closed at the `result` line.

| Log | Lines that opened (all) | Lines that closed (all) | Lines that opened outside a turn |
|---|---|---|---|
| S1 fixture | 2 · 141 · 190 = each turn's `command_lifecycle started` | 138 · 187 · 201 = each turn's `result` | 0 |
| s3 | 12 (A) · 48 (B `started` — a new turn after A's `result`) | 46 · 59 | 0 |
| s4 | 12 · 24 · 36 · 48 | 19 · 31 · 43 · 57 | 0 |
| s5 | 12 · 103 · 144 | 98 · 139 · 159 | 0 |
| s6 | 15 · 36 (the two interrupts were written with the gate closed) | 28 · 47 | 0 |
| s7 | four A · four B — all `started` | all `result` | 0 |

- **The line that opens the gate was, without exception, that turn's `command_lifecycle started`** (→ `Delivered`) — before the echo · `init` · the first text. The `cancelled` (→ `Dropped`) that comes after the end · the `control_response` of an idle interrupt · `system/*` did not open it.
- The synthetic `[Request interrupted…]` line is `Structured{user}` → `Progress`, but **always before that turn's `result`**, so it does not touch an already open gate.
- The gate at the moment the interrupt was written: S1 · S3 · S4 · S5 were all **open** (our implementation would have sent it too) · the two in S6 were **closed** (our implementation: CONFLICT) · S7 is the §8 table.
- The signal today's classifier gave to the interrupted `result` is always `[Failed, Ended(Clean)]` — before B3 the stop after an error stands (§3).
- Per-line events of the fixture tail (today's decoder as is): interrupt response (135 · 183) → **0 events** (confirms TRD §3-4 「번역하지 않는다 — 이미 `None`」 (not translated — already None)) · truncated complete line (136) → **0 events** (discarded since it is streamed text) · synthetic line (137 · 186) → `Structured{kind:"user"}` (that line's uuid is carried) · tool result (185) → `Structured{kind:"user"}` · interrupted `result` (138 · 187) → `[Usage?, Error("claude stream-json result reported failure (subtype=error_during_execution)"), MessageDone]` · the following `cancelled` (139 · 188) → `QueuedInput(Dropped{cause: Unknown})`.
- **Verdict: pass.**

## 10. What conflicts with TRD §3-4 · ADR-0238, or is new

1. ★**The synthetic user line `[Request interrupted by user]` / `…for tool use]` — the TRD did not expect it (reason to stop)**★ — it comes before the `result` in 14/14 interrupted turns (§1) and stays in the transcript as a `user` line (the 0-based 21st · 33rd lines of the S1 session transcript — confirmed). On today's path it becomes a **user bubble** both live and in the resume history. The screen of TRD §3-4 「codex 끊김과 같은 모양 · 프론트·선 타입 무변경」 (same shape as a codex interrupt · no change to frontend or wire types) · U8 (interrupt line emphasis) differs by exactly this one bubble. For an interrupt during a tool the screen order becomes 「붉은 `Error` 도구 행 → `[Request interrupted by user for tool use]` 말풍선 → 중단 행」 (red Error tool row → [Request interrupted by user for tool use] bubble → interrupt row) (the shape of TRD §11 ⑪ + a bubble). The invariants are unaffected (§9). **How it should look (leave as is · the decoder filters it · draw it differently) is user experience, so main · user must decide** — if it is filtered, what to recognize it by (the vendor string · a text-only `user` line without `isReplay` · position) is also open. This document does not choose.
   - Verdict = ADR-0243 (2026-09-28).
2. **S4's premise order differs (not a stop)** — in 2.1.280 the echo is after init. The line that opens the gate is `started`, and the interrupt there took effect — no correction needed (§5).
3. **The `subtype` of the interrupted `result` is `error_during_execution`, not `interrupted` (the TRD expected the `terminal_reason` cell)** — B3 pinning `TurnEnd{Interrupted}` with fixtures 138 · 187 becomes the measured basis. The two hand-built `subtype:"interrupted"` tests remain only for older CLIs.
4. **The `cancelled` of the command that opened the turn comes after the `result` (harmless)** — a no-op on a tombstone · classifier `None` · the gate does not open (§1 · §9). The TRD did not list this line, but the translation table (「`cancelled` → 모름(끊기·…)」 at `claude/mod.rs:1330`) already covers it.
5. **The tool input being streamed vanishes without a trace** — an open `tool_use` block ends with neither a complete line nor a `content_block_stop` (§6-2). The screen has no tool row at all (only the streamed text · the interrupt row).
6. **The residual race is not narrow** — if there is a pending B, the gap with the gate closed between A's end and B's start is only 2–4 ms, so a late Esc aimed at A's tail effectively interrupts B (§8). The outcome is the shape the TRD accepted (interrupted turn · not an error · stop unchanged).
7. (side note) If stdin is closed with the last turn interrupted, the process ended with **exit code 1** (s4 · s5 · s7 — s1 · s3 · s6, whose last turn succeeded, gave 0). Our kill path does not rely on the exit code — record only.

## 11. Fixture

- `crates/engram-dashboard-agent/src/backend/claude/fixtures/interrupt_s1.jsonl` — 202 lines. Source = `dir:"out"` lines 11–223 · 226–291 · 311–324 (0-based) of `claude-s1-2026-09-27T15-37-24-325Z.jsonl`. On top of the common processing in `fixtures/README.md` (removing four noise lines · setting the `system/init` installed-environment lists to the same neutral values as B1 · privacy placeholders), plus: scratch paths → `C:\work\proj` (cwd) · `C:/work/logs` (tick file) · ★the 11 `input_json_delta` lines carrying the path (original 251–261) were merged into one line `/work/logs/`★ (confirmed that the concatenated argument equals the `input` of the complete `tool_use` · the decoder does not read this delta — tool-input deltas 35 → 25 lines). `system/status` · `system/task_started` · `system/task_notification` are as measured.
- Fed again through the real decoder and confirmed that the gate trajectory equals the original log (opening lines 2 · 141 · 190 · closing lines 138 · 187 · 201).
- ★`fixtures/README.md` was not edited by this work★ (another worker has uncommitted changes) — main adds one table line.
