# Mid-turn input — Phase 0 vendor measurements (M1–M8) (2026-09-25)

- **Status:** Measurements complete · not yet reflected in the TRD. This document measured the table (M1–M8) of `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` §3-3 **as its task list, unchanged**. ★This document did not edit the TRD body★ — rewriting §3 and pinning fixtures is for the next revision (no orphans: a link from TRD §3-3 to this document is needed).
- **Measured on:** 2026-09-25 (KST 03:29–03:40 · log timestamps are UTC 2026-09-24T18:29–18:40)
- **Versions (the installs our app launches):**
  - claude = `2.1.280 (Claude Code)` — `claude` on PATH → `%APPDATA%\npm\claude.cmd` → `node_modules\@anthropic-ai\claude-code\bin\claude.exe`. **Same as** the TRD's reading baseline.
  - codex = `codex-cli 0.156.1` — `codex` on PATH → `codex.cmd` → node → `codex.exe`. ★**Newer** than the TRD's reading baseline (0.154.0)★ — all codex results below are measured on 0.156.1 (not re-measured on 0.154.0).
- **Method:** a node harness that imitates our spawn arguments and wire format exactly launches the CLI directly, writes JSON lines to stdin at fixed moments, and records every stdout line as JSONL together with a **monotonic clock (ms, from harness start)**. Every run was done outside the shell tree with `scripts/run-detached.ps1`, and completion was decided by an `__EXIT=0` marker. The app, the daemon and other worktree processes were not touched.
- **Harness and raw log location (session scratch — may vanish):** `C:\Users\<user>\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard-wt1\6847046e-28fb-4531-a649-281ac6ab97c0\scratchpad\phase0\`
  - Harness: `claude_harness.js` (scenarios M1 · M3 · M3B · M7) · `codex_harness.js` (M6 · M7) · analysis: `sumclaude.js` · `sumcodex.js` · `m7claude.js` · `m7codex.js`
  - Raw logs: `logs\claude-M1-2026-09-24T18-29-52-786Z.jsonl` · `logs\claude-M3-2026-09-24T18-32-48-267Z.jsonl` · `logs\claude-M3B-2026-09-24T18-38-29-346Z.jsonl` · `logs\claude-M7-2026-09-24T18-35-19-085Z.jsonl` · `logs\codex-M6-2026-09-24T18-32-49-086Z.jsonl` · `logs\codex-M7-2026-09-24T18-35-20-013Z.jsonl`
- **Confidence legend:** certain (a fact printed directly in the raw logs, observed repeatedly) · likely (observed once or twice + consistent with a code reading) · uncertain (inference · single shot · cannot be decided from the outside)

## 0. Conclusion (first)

1. **The premise of the claude path (S2) holds in our mode.** `command_lifecycle` comes out as is in `-p` stream-json (M1), and `cancel_async_message` is accepted without `initialize` (M3). Cancelling an item that is in the queue closes with `cancelled:true` + lifecycle `cancelled`, and it does not go to the model.
2. ★**TRD §3-1 ③ 「아직 안 들어온 uuid 는 취소 예약 → 나중에 `cancelled`」 (a uuid that has not arrived yet gets a cancel reservation → later cancelled) is wrong for mid-turn (the fold path)**★ — when the cancel was sent first and the text written later, the text was **folded and delivered** (`started`) after `cancelled:false`. A reserved cancel is confirmed only on the between-turns drain (new turn start) path (code reading). However, our design always writes in the order 「글 → 취소」 (text → cancel), and the CLI loads stdin synchronously in order (even when sent in the same write: `queued` → `cancelled`), so the ⓒ race **did not occur naturally.** The rule 「결말은 수명주기로만」 (the outcome is read from the lifecycle only) still holds.
3. ★**M7 came out opposite for the two backends**★ — handing over after seeing a tool finish (S1): **claude 0/10** (always missed, as the TRD predicted) · **codex 10/10** (the opposite of the TRD prediction 「거의 늘 놓친다」 (almost always misses) — for 6–64 ms after the tool-completion notification, the pending-input check is not running yet). The decision was already made as 「즉시 넘기기」 (hand over immediately), so the design does not change — it is material showing that codex S1 is a usable candidate in a later refinement.
4. **All assumptions on the codex (0.156.1) side held** (M6): two steers → two `userMessage` carrying `clientId` after the tool · no echo on interrupt (not in the history either) · a steer at the moment the turn just ended → `-32600 "no active turn to steer"` · `thread/items/list` carries `clientId`. The turn-end 「기록만」 (record-only) branch was **not hit** (both times it was consumed by a follow-up sampling in the same turn).
5. **Two new findings** — ① claude `system/init` comes out **every time a turn starts**, not at startup (there is none before the first input) ② in codex the shell command of an interrupted turn **keeps running**, and 17 seconds after the interrupt an **`item/completed` carrying the old turn id** arrived after other turns had ended (affects the §5-5 tool count).

## 1. Spawn arguments and wire format (for reproduction)

### claude — `ClaudeBackend::build_spec` JSON mode (`crates/engram-dashboard-agent/src/backend/claude/mod.rs:155-316`)

```
cmd.exe /c claude --permission-mode bypassPermissions -p --input-format stream-json
  --output-format stream-json --replay-user-messages --verbose --session-id <새 uuid> --model haiku
env += MAX_THINKING_TOKENS=8000
```

- **Left out:** `--mcp-config` · `--append-system-prompt-file` · `--settings` · `--disallowedTools SendMessage` · `--allowedTools …`, which are attached only when a control endpoint exists. All of them are MCP, priming and permission declarations, so they were judged unrelated to the command queue · fold · cancel machinery (uncertain — on the day a hook is carried in our `--settings` fragment, the 「접기 전 지연」 (pre-fold delay) below may change).
- **Added:** `--model haiku` (in the extra_args slot). Measured model = `claude-haiku-4-5-20251001`. Queue · fold · lifecycle · cancel are machinery inside the CLI, so they are independent of the model — the model affects only whether it follows instructions (not used for decisions) and response latency.
- **User settings were applied as is** — the same conditions as the claude our app launches (user `settings.json` hooks · installed plugin hooks · global instructions). This bears on the 「접기 전 지연 ~0.57 s」 (pre-fold delay ~0.57 s) in M3 below.
- stdin user line (same key order as `wrap_user_turn`): `{"type":"user","message":{"role":"user","content":[{"type":"text","text":…}]},"uuid":"<uuid>"}\n`
- Cancel line (exactly the shape of TRD §5-4): `{"type":"control_request","request_id":"cancel:<uuid>","request":{"subtype":"cancel_async_message","message_uuid":"<uuid>"}}\n`
- Work folder = `cwd-claude` in the scratch (outside the repository) · a new session id every run. `--resume` was not used.

### codex — app-server mode (`backend/codex/mod.rs:807-815` · `thread_open` `:125-152` · channel handshake `transport.rs:827-852`)

```
cmd.exe /c codex app-server --stdio -c model=gpt-6-luna -c model_reasoning_effort=low -c notify=[]
```

- The three `-c` flags are in the extra_args slot (the channel our argv allows). They set a cheap model · low reasoning · and **blocked the user `config.toml` `notify` (desktop app launch) from running every turn**. The steer and pending-input machinery is on the app-server side, so it is independent of the model.
- Handshake: `initialize {clientInfo:{name:"engram-dashboard",version:"0.1.0"}}` → `initialized` notification → `thread/start {cwd, approvalPolicy:"on-request", sandbox:"workspace-write"}`. Server requests were made to be refused with `-32601`, as our `Reader::refuse` does (not a single one arrived in this run).
- No `jsonrpc` field in the envelope (same as our `request_line`). `clientUserMessageId` was carried in `turn/start` as designed (today's code does not carry it — `protocol.rs:537`).
- `turn/steer`: `{"threadId":…,"clientUserMessageId":<uuid>,"input":[{"type":"text","text":…}],"expectedTurnId":<활성 턴 id>}` — same as the 0.156.1 schema (`app-server-protocol/src/protocol/v2/turn.rs` `TurnSteerParams`).
- The thread was newly created in the user's `CODEX_HOME` (existing threads were not touched — §10 below).

## 2. M1 — does claude `command_lifecycle` come out in our mode

**Setup:** first line A (uuid `f53b8bb3…`) = 「Bash 로 `sleep 20` 을 돌리고 DONE-A 로 답하라」 (run sleep 20 with Bash and answer DONE-A) → 3 s after seeing `tool_use`, B (uuid `44fcee51…`) = 「답에 PINEAPPLE 을 넣어라」 (put PINEAPPLE in the answer).

**Evidence** (`logs\claude-M1-…jsonl`, t = ms):

```
4010.3   >> A 씀
6923.1   {"type":"command_lifecycle","command_uuid":"f53b8bb3-…","state":"queued","uuid":…,"session_id":…}
6924.4   command_lifecycle started f53b8bb3
7580.3   system/init  (capabilities 포함)
9680.5   assistant tool_use Bash {"command":"sleep 20"}
12681.2  >> B 씀 (도구 도는 중)
12682.1  command_lifecycle queued 44fcee51        ← 쓴 뒤 0.9 ms
32271.3  user tool_result
32842.6  user isReplay uuid=44fcee51 ("timestamp":"…18:30:05.469Z" = 쓴 시각)
32843.7  command_lifecycle started 44fcee51      ← 도구 결과 뒤 572 ms
35239.6  assistant text "DONE-A PINEAPPLE"
35663.3  command_lifecycle completed 44fcee51
35666.1  result/success
35667.2  command_lifecycle completed f53b8bb3
```

**Result: green.** The lines come out and the order is `queued` → `started` → `completed`. Position of the terminal lines relative to `result`: **the `completed` of the folded-in B is right before `result` (3 ms)**, the `completed` of A, which opened the turn, is right after `result` (1 ms). The field name is `command_uuid` (our uuid) · the line's own `uuid` is a value the CLI freshly generated. After delivery the model reflected B (PINEAPPLE).
- A, sent while idle, also gets a lifecycle (`queued`→`started` within 1 ms) — this is the branch where TRD §5-4's 「Direct 로 보낸 입력의 `started` 는 모르는 id 라 버린다」 (the started of input sent through Direct is dropped as an unknown id) is actually needed.
- Observed repeatedly in the same shape for all 32 uuids in M3 · M3B · M7.

**Confidence:** certain. **TRD 「빨가면」 (if red):** not applicable — the §4-3 claude recommendation (S2) is kept.

## 3. M2 — does the `isReplay` echo come when written or when folded

**Result: it never comes when written. It comes when consumed** — but **the order against `started` flips depending on the consumption path:**

| Path | Echo position | Observed in |
|---|---|---|
| Folded mid-turn (tool boundary) | **right before** `started` (usually 0.4–1.1 ms · only tb2, where a cancel intervened, 44 ms) | M1 (replay at +572 ms → started +1 ms) · M3 c2 · M3B tb2–tb4 |
| Boundary missed, drained as a new turn at turn end | **about 1.5–2.1 s after** `started` (`system/init` in between) | all 10 of M7 (e.g. trial 1 started +1799.5 · replay +3588.7, relative to tool_result) |
| Cancelled | **does not come** | M3 a · c1 · b1–b4 · M3B tb1 |

- The `timestamp` field of the echo line is **the time it was written (loaded)**, not the consumption time (M1: 18:30:05.469Z = the moment B was written, the line arrived 20 s later).
- A `user` line (echo) **does come** for folded-in input too.

**Confidence:** certain (observed repeatedly on all three paths). **TRD 「빨가면」 (if red):** only the numbers are recorded — ★but the §5-7 accumulator must accept **both** orders★: on the fold path the echo comes before `Delivered`, so 「대기 uuid 억제」 (suppress pending uuids) is needed, and on the new-turn path it comes after `Delivered`, so uuid dedup takes care of it. The TRD design absorbs both.

## 4. M3 — `cancel_async_message` (our mode · without `initialize`)

**Is it accepted:** yes. The 「not supported in this context」 error never came. Response envelope (verbatim):

```
{"type":"control_response","response":{"subtype":"success","request_id":"cancel:6d3edeb9-…","response":{"cancelled":true}}}
```

The result value is nested in `response.response.cancelled`. On a successful cancel, the lifecycle `cancelled` line comes out **before the response** (same ms).

**Three races — what was tried and what was hit:**

| Attempt | Setup | Observed (t = ms) | Verdict |
|---|---|---|---|
| **ⓐ while in the queue** (trial a) | B written during a tool → cancel 1 s after `queued` was confirmed | 13827.9 cancel written → 13829.1 `LC cancelled` → 13829.1 response `true` · answer "DONE-a" (no word) | **hit** — `true` + `cancelled`, did not go to the model |
| ⓐ variant — text + cancel in the same write (c1) | both lines written at once | 35671.1 written → 35672.0 `queued` → 35672.2 `cancelled` → 35672.3 `true` | aimed at ⓒ but **fell into ⓐ** — the CLI **loads the lines synchronously** in order |
| ⓐ variant — cancel at +0/150/300/450/540 ms after the tool result (b1–b4 · tb1) | B was loaded during the tool | all five times `cancelled` + `true`, did not go to the model (b1 was handled 13 ms late because the CLI was busy) | **ⓐ** — it could still be taken out of the queue **even up to 540 ms** after the tool result |
| **ⓑ mid-fold** (tb2) | cancel at tool result +580 ms | 28505.4 B echo (= fold in progress) → 28515.4 cancel written → 28549.3 `LC started` → 28560.9 response `false` · the word is in the answer | **hit on the wire** — sent between the echo and `started`, and the result was `false` → `started` (delivered). But whether it took the `isFoldInFlight` branch or the 「이미 꺼냄」 (already taken out) branch inside the CLI **cannot be told from the outside** (the response came after `started`) |
| Too late (tb3 · tb4) | +620 · +660 ms | the cancel arrived after `started` → `false` | 「이미 꺼냄」 — matches the outcome `started` |
| **ⓒ right after writing, the CLI has not read it yet** | cannot occur naturally (c1 above) — instead **the cancel first, the text 0.7 s later** to hit the reserved-cancel machinery directly (c2) | 55629.5 cancel → 55630.1 `false` → 56330.3 B written → 56331.1 `queued` → 66873.9 echo → 66874.6 **`started`** · answer "DONE-c2 WORDC2" | ★**the reserved cancel did not catch it — it was delivered**★ |

- **Why ⓒ did not catch (code reading, likely):** the only callers of `consumeCancelPending`, which checks the reservation marker, are the **between-turns drainer** (right before a new turn starts · binary @217253965 · @217254615) and batch settlement; the mid-turn **fold path** (near @209592877) does not look at the marker. That is, 「caught by a pending cancel just before dispatch」 in the description is about **a new-turn drain**.
- **What the pre-fold delay is (code reading, likely):** right after a tool result the fold first snapshots the pending commands (`getCommandsByMaxPriority`), then waits for each command's **「prompt.submit at the fold」 async handling** (`ah` → `Ed`, @209442306), and at the end keeps **only what is still in the queue** (the `Oi` filter = a check against `getCommandQueue()`, @209441195). So a cancel during this interval (here ~0.55–0.66 s) is accepted as `true` and there is no double delivery. The cause of this interval being long is **presumed to be the installed plugin's `UserPromptSubmit` hook** (uncertain — the hook-run event is not printed in the stream, so it could not be checked directly). In an environment without hooks this window may be shorter.
- **One side effect:** a too-late cancel (ⓑ · 「이미 꺼냄」) and ⓒ **leave a reserved-cancel marker** in the CLI (`markCancelPending` — unconditionally when the result is empty and a fold is not in progress). It is harmless unless the same uuid is reused (likely).

**Result: green (the cancel feature) · the TRD §3-1 ③ description is red.** ⓐ certain · ⓑ only the on-the-wire outcome is certain (the internal branch is uncertain) · ⓒ the natural race cannot occur (certain) / the reserved cancel does not catch mid-turn (likely — one observation + code).
**TRD 「빨가면」 (if red):** not applicable (a claude cancel does not drop to the codex grade) — no reason to reopen the S5 move. However, the parenthesis in the §3-1 ③ row and in §0-decision 2 (「stdin 을 아직 안 읽었으면 취소 예약만 걸고 false 를 답한 뒤 나중에 `cancelled` 를 낸다」 — (if stdin has not been read yet, it only sets a cancel reservation, answers false, and later emits cancelled) must be corrected to **「턴 사이 배출 경로에서만 — 턴 도중 접기에선 예약이 안 걸려 `started` 로 끝난다」** (only on the between-turns drain path — on a mid-turn fold the reservation does not catch and it ends with started). The rule of deciding the outcome from the lifecycle only gave the right answer in this observation too (`false` then `started` → Delivered).

## 5. M4 — when does `system/init` come and does it carry `capabilities`

**Result:**
- **It does not come at startup.** In the 4 s before the first input, stdout had only `system/hook_started`·`hook_response` (SessionStart hooks) (M1 0–4010 ms).
- It comes **once every time a turn starts** — **0.58–0.86 s** after the `started` of the command that opened the turn (the first 5 turns of M7: +749 · +587 · +612 · +855 · +584 ms). Number of `init` per run = number of `result` (M1 1/1 · M3 7/7 · M7 20/20 · it does not come for folded-in input).
- `capabilities` = `["interrupt_receipt_v1","interrupt_cancel_queued_v1","msg_lifecycle_v1","mcp_read_resource_v1","mcp_tool_ui_meta_v1"]` — `msg_lifecycle_v1` is present.

**Confidence:** certain. **TRD 「빨가면」 (if red):** there is no need to change it via a version check. However, §5-4 「탐지」 (detection) has one gap — the flag goes up only **~0.6–0.9 s after the first turn's `started`**. Text typed in between is misclassified as idle and goes to today's behavior (the bubble at the sent position). The lifecycle line comes right after the CLI reads the first input, but measured from the time we wrote it (M1 4010.3 ms) that is about 2.9 s later (6923.1 ms — including CLI startup. Corrected 2026-09-25 — checked against the M1 log while writing TRD edition 2). So **using the first `command_lifecycle` line as the detection signal as well** only narrows that gap and does not remove it (the design judgment is the TRD's).

## 6. M5 — transcript shape of folded-in input

**Result (M1 session `561e36c6-….jsonl`):** the folded-in B is left only as **a single `attachment` line**, not as a `user` line:

```
{"type":"attachment", "attachment":{"type":"queued_command",
  "prompt":[{"type":"text","text":"When you reply, also include the word PINEAPPLE."}],
  "source_uuid":"44fcee51-376b-4de7-9532-e9608f9de0a0", "commandMode":"prompt",
  "timestamp":"2026-09-24T18:30:05.469Z"}, "uuid":"c1d33099…", "parentUuid":"f158675a…"}
```

- Position: tool-result `user` line → `prompt_snapshot` attachment → **`queued_command`** → `queue-operation {"operation":"remove","reason":"absorbed_mid_turn"}` → next `assistant`.
- `source_uuid` = our uuid, `prompt` = our body array — all the material the restore seed needs to build a user bubble is there.
- **Cancelled input leaves no body** — only the enqueue/dequeue of `queue-operation` remain (M3 session: enqueue 14 · dequeue 13, `queued_command` only for the delivered c2).
- **Input that missed the boundary and became a new turn is an ordinary `user` line** (M7 session: `user` text 20 = A 10 + B 10, `queued_command` 0).

**Confidence:** certain (the shape) · the effect on restore order was not read. **TRD 「빨가면」 (if red):** the shape was right → the 「복원 seed 에 그 줄을 사용자 말풍선으로 옮기는 조각」 (the piece in the restore seed that carries that line over as a user bubble) of §9 P3 is needed as is (today's `consume_line` drops `attachment`).

## 7. M6 — codex `turn/steer` (0.156.1)

**Setup:** four turns in one thread. T1 = two steers (X=MANGO, Y=LEMON) during `Start-Sleep -Seconds 20` + one steer with the old turn id **inside the very handler** that receives `turn/completed` · T2 = steer Z (PAPAYA) during the same command, `turn/interrupt` 1.5 s later · T3/T4 = steer W at the moment the sampling end is received in a turn without tools (`agentMessage` `item/completed` and `thread/tokenUsage/updated` respectively) · after every turn `thread/items/list {limit:40, sortDirection:"desc"}`.

**Evidence** (`logs\codex-M6-…jsonl`):

```
T1  5939.1  >> turn/steer X      → 5943.5 {"turnId":"01a0d4b1-12c5…"}   (수락 = 전달 아님)
    6938.5  >> turn/steer Y      → 6939.4 {"turnId":…}
    24830.6 item/completed commandExecution
    24835.7 thread/tokenUsage/updated
    24837.8 item/started {"type":"userMessage","clientId":"bb2c04bc…"(X),"content":[{"type":"text","text":"…MANGO…","text_elements":[]}]}
    24843.0 item/started userMessage clientId=fa5e629a (Y)
    25988.3 item/completed agentMessage "DONE-1 MANGO LEMON"
    26023.1 turn/completed
    26023.2 >> turn/steer (옛 턴 id, turn/completed 핸들러 안) → 26026.5 {"code":-32600,"message":"no active turn to steer"}
T2  33147.5 >> turn/steer Z → 수락 · 34648.9 >> turn/interrupt → 34674.0 turn/completed status "interrupted"
    (Z 의 userMessage 에코 없음 · items/list 에도 Z 없음)
    51852.1 item/completed commandExecution (T2 의 턴 id · status "completed") ← 끊기 17 s 뒤, T3·T4 가 끝난 뒤
T3  40381.3 item/completed agentMessage → 같은 핸들러에서 steer W → 40459.3 userMessage 에코 → 두 번째 agentMessage "…A COMET…" → 42144.6 turn/completed
T4  47658.8 tokenUsage/updated → 같은 핸들러에서 steer W → 47688.7 에코 → 두 번째 agentMessage "…COMET…" → 49228.8 turn/completed
```

**Result (per item):**

| TRD M6 question | Result | Confidence |
|---|---|---|
| Do two steers during a tool → two `userMessage` each carrying `clientId` come after the tool | **Yes** — in the order typed, after the tool finished and before the next sampling | certain |
| Is there no echo on interrupt | **None** — no echo, and not in `thread/items/list` either (silently discarded) | certain |
| −32600 when the turn has just ended | **Yes** — a steer sent in the very handler that received `turn/completed` got `-32600 "no active turn to steer"` | certain |
| Does `thread/items/list` carry the `clientId` of an item that entered by steer | **It does** — for both X · Y · W, and for the first input opened with `turn/start` too | certain |
| Is input that was 「기록만」 (record-only) at turn end echoed before `turn/completed` | **That branch was not hit** — steers sent at two sampling-end points (T3·T4) were both consumed as a **follow-up sampling of the same turn** (echo → the model reflected it → `turn/completed`). This means the record-only window is even later than `tokenUsage/updated`, and this method did not reach it | uncertain (branch not observed) |

- `turn/start` accepts `clientUserMessageId` and attaches `clientId` to the first-input echo as well (confirmed on 0.156.1 — today our code does not carry it).
- The steer success response (`{"turnId"}`) arrives within 1–5 ms, **much earlier** than the echo — TRD §5-5 「성공 응답은 무동작(전달 판정은 에코가 한다)」 (the success response is a no-op; the echo decides delivery) is the right choice.
- ★**New finding — the tool of an interrupted turn keeps running**★: even after the `turn/interrupt` response and `turn/completed(interrupted)`, the shell command ran to the end (20 s), and its `item/completed` (**old turn id**, status `completed`) arrived **after the next two turns ended**. If our decoder counts it as a tool of the new turn, the §5-5 tool count goes off.

**TRD 「빨가면」 (if red):** two things to fix in the §5-5 dispositions — ① the tool count must be **filtered by turn id** (an `item/*` that is not of the current turn is excluded from the count — 「턴 경계에서 0」 (zero at the turn boundary) alone lets a late-arriving `completed` produce a negative) ② the branch 「정상 `turn/completed` 인데 넘긴 항목의 에코가 없음 = `Dropped{Unknown}`」 (a normal turn/completed but no echo of the handed-over item = Dropped{Unknown}) — **its occurrence condition could not be produced** this time — the basis for it standing without double delivery (「기록된 입력은 완료 알림 전에 에코된다」, recorded input is echoed before the completion notification) is still unconfirmed.

## 8. M7 — the rate at which the boundary is missed when handing over after seeing a tool **finish** (S1)

**Setup:** every turn 「짧은 명령(2 s)을 돌리고 OK-k 로 답하라」 (run a short command (2 s) and answer OK-k). B is handed over **immediately inside the very stdout handler** that receives the tool-completion line (claude = `tool_result` `user` line · codex = `commandExecution` `item/completed`) (claude = stdin user line · codex = `turn/steer`). 10 times in one session.

**Decision criteria:**
- claude: a hit if B's `started` is before the first `assistant` after the tool (folded at the same boundary).
- codex: a hit if B's `userMessage` echo is before the start of the first `agentMessage`/`reasoning` after the tool and **there is no follow-up sampling**. ★Whether the model said KIWI was not used as a criterion★ — even though codex hit all 10 times, the answer was only "OK-k" (a model choice that clashed with the instruction 「exactly」). That the KIWI user message is in the history before the `OK-k` answer in the rollout file was checked separately for trials 1–3 (4–10 were decided only by echo order).

**claude — 0/10 hits** (`m7claude.js`, ms relative to the tool_result time):

| trial | written | `queued` | `started` | first assistant after the tool | number of `result` |
|---|---|---|---|---|---|
| 1 | +0.3 | +18.0 | +1799.5 | +1302.1 | 2 |
| 2 | +0.0 | +19.2 | +1440.1 | +951.5 | 2 |
| 3 | +0.1 | +14.4 | +1538.8 | +952.4 | 2 |
| 4 | +0.1 | +30.8 | +2219.4 | +1533.9 | 2 |
| 5–10 | +0.0–0.2 | +11.7–14.1 | +1386–1658 | +914–1171 | 2 |

→ Even though we wrote within 0.3 ms, the CLI read that line only **12–31 ms later** (the fold snapshot had already been taken by then), and B went out as a new turn after `result` every time (`started` = first `result` + 1.5 ms).

**codex — 10/10 hits** (`m7codex.js`, ms relative to the tool `item/completed`):

| trial | steer written | `tokenUsage/updated` | B echo | next sampling start | answer after the tool |
|---|---|---|---|---|---|
| 1 | +0.4 | +9.2 | +12.1 | +978 | 1 ("OK-1") |
| 2 | +0.1 | +6.6 | +8.8 | +1098 | 1 |
| 5 | +0.2 | +54.7 | +63.9 | +1253 | 1 |
| 6 | +0.1 | +53.7 | +57.9 | +1060 | 1 |
| 3·4·7–10 | +0.0–0.3 | +6.1–9.9 | +8.3–13.5 | +954–1078 | 1 |

→ There is a gap of **6–64 ms** from the tool-completion notification to the pending-input check (the check is 2–9 ms after `tokenUsage/updated`), and a steer that came in during it entered the same boundary.

**Confidence:** claude certain (10/10 the same shape + consistent with the code reading) · codex likely — the harness reacted in 0.1–0.4 ms. Whether **our daemon's reaction latency** (pump → channel → write) is shorter than this window (minimum 6 ms) was not measured, and if there are several parallel tools or the model is slow the shape of the window may differ (uncertain). The two M7 runs ran at the same time (different processes — the reaction path is each harness's synchronous handler, so interference is considered small).
**TRD 「빨가면」 (if red):** the §4-2 S1 cell is replaced by the measurement — **claude = 「✕ 0/10 놓침」 (missed 0/10) (reading confirmed) · codex = 「○ 10/10 맞음(창 6–64 ms)」 (hit 10/10 (window 6–64 ms)) (opposite to the reading).** The decision (hand over immediately on both backends) does not change. However, in a later refinement, codex's 「보관 → 도구 완료 시 넘김」 (hold → hand over at tool completion) becomes material showing it is **not one step late** — §0.4 and the §4-2 decision cell 「D1 의 『바로』를 놓친다」 (misses D1's "immediately") were wrong for codex.

## 9. M8 — where is the codex version read (decision on the 0.140.0 floor)

**Result:**
- `initialize` response `userAgent` = `engram-dashboard/0.156.1 (Windows 10.0.26200; x86_64) unknown (engram-dashboard; 0.1.0)`.
  - Shape = `<originator>/<codex 버전> (<OS> <OS 버전>; <arch>) <터미널> (<clientInfo.name>; <clientInfo.version>)`. ★The leading `originator` is **our `clientInfo.name`**★ — codex sets it as the global originator in `initialize` (0.156.1 `app-server/src/request_processors/initialize_processor.rs:124-175`). However, if the env `CODEX_INTERNAL_ORIGINATOR_OVERRIDE` exists, that value becomes the prefix (comment in the same file).
  - The version is between the first `/` and the first space after it (`login/src/auth/default_client.rs` `get_codex_user_agent` — `CARGO_PKG_VERSION`). ★The trailing parentheses also contain **our version (0.1.0)**★ — searching for 「문자열 안 첫 semver」 (the first semver in the string) is correct, but searching for 「마지막 semver」 (the last semver) reads our version.
- **A cleaner source is already in hand:** `thread/start` response `result.thread.cliVersion` = `"0.156.1"`. It is a field our `protocol.rs` `Thread.cli_version` already reads, and `thread/resume` also gives the same `Thread` (schema reading — the resume response was not measured this time).

**Confidence:** certain (measured string + source). **TRD 「빨가면」 (if red):** not applicable — decidable. Recommended order = `thread.cliVersion` → if absent, `^[^/]+/(\d+\.\d+\.\d+)` on `userAgent` → if both fail, hold mode (TRD §5-5 as is).

## 10. Cleanup · cost

- **Processes:** all 6 children the harness launched (claude 4 · codex 2) exited on their own once stdin was closed (log `exit:0`, forced termination 0 times). After they exited, I swept the command lines containing the harness, `app-server --stdio` and the session ids with `Get-CimInstance Win32_Process`, and **0 were left**. The two `node_repl.exe` created after 03:29 have parents that were another `codex.exe app-server` already running before this measurement (03:19 · started 23:26 the day before), so they are not from this run.
- **Traces left (new sessions that may be thrown away):** 4 claude transcripts = `%USERPROFILE%\.claude\projects\C--Users-<user>-AppData-Local-Temp-claude-I--Engram-apps-engram-dashboard-wt1-6847046e-28fb-4531-a649-281ac6ab97c0-scratchpad-phase0-cwd-claude\` · 2 codex rollouts = `%USERPROFILE%\.codex\sessions\2026\09\25\rollout-…-01a0d4b1-1240-….jsonl` · `…-01a0d4b3-6048-….jsonl`. Existing user sessions were not touched.
- **Cost:** claude (haiku) cumulative about $0.28 (32 result turns) · codex (gpt-6-luna, low) 14 turns (M6 4 · M7 10 — amount not tallied). Retries 0 times.

## 11. Summary table

| M# | Result | Green/Red | Affected TRD sections |
|---|---|---|---|
| M1 | `command_lifecycle` comes out in our mode · `queued`→`started`→`completed` · for folded-in input `completed` is right before `result`, for the turn-opening input right after | green | §3-1 row 2 (unmeasured → measured) · §4-3 kept |
| M2 | echo only at consumption — fold path = right before `started` · new-turn path = ~1.5–2 s after `started` · cancelled input does not come · `timestamp` is the write time | green (numbers) | §3-1 row 10 · §5-7 (absorbing both orders confirmed) |
| M3 | accepted · envelope `response.response.cancelled` · ⓐ `true`+`cancelled` (up to 540 ms after the tool result) · ⓑ `false`→`started` on the wire · ⓒ no natural race / **the reserved cancel does not catch mid-turn, so it is delivered** | green (feature) · **red (§3-1 ③ description)** | §3-1 rows 4·5 · head block decision 2 parenthesis · §5-4 (decision rule kept) |
| M4 | no init at startup · at every turn start `started` +0.6–0.9 s · `msg_lifecycle_v1` present | green (one gap) | §3-1 row 8 · §5-4 detection (misclassification gap early in the first turn) |
| M5 | `attachment{type:"queued_command", prompt, source_uuid, commandMode}` · no body for cancelled input · missed input is an ordinary `user` | green | §3-1 row 11 · §5-4 takeover restore · §9 P3 |
| M6 | two steers → two echoes · interrupt = no echo · no history · just-ended turn −32600 · `clientId` in items/list · 「기록만」 branch not observed · **the tool of an interrupted turn gets a late `item/completed`** | green + a new fact | §3-2 · §5-5 (tool count filtered by turn id · basis for the `Dropped{Unknown}` branch unconfirmed) |
| M7 | claude 0/10 (missed) · codex 10/10 (hit, window 6–64 ms) | claude reading confirmed · **opposite to the codex reading** | §0.4 · §4-2 S1 cell · reading-basis line |
| M8 | `userAgent` = `<clientInfo.name>/<버전> (…) … (<name>; <우리 버전>)` · `thread.cliVersion` is cleaner | green | §5-5 floor decision |

## 12. What diverged from the TRD assumptions (quotations)

1. §3-1 ③ — 「**아직 안 들어온 uuid 면 「취소 예약」(`markCancelPending`)을 걸고 `cancelled:false` 로 답한다** — 나중에 들어오는 순간(「caught by a pending cancel just before dispatch」) `cancelled` 로 닫힌다」 (if the uuid has not arrived yet, it sets a cancel reservation (markCancelPending) and answers cancelled:false — the moment it arrives later it closes with cancelled) → mid-turn (fold) it did not close and was **delivered** (M3 c2). The reservation is looked at only on a new-turn drain (code). The same description is also in head block decision 2.
2. §0.4 · §4-2 S1 codex cell — 「**✕ 거의 늘 놓친다 — 도구 끝나자마자 같은 프로세스 안에서 대기분을 확인한다**」 and the reading basis 「codex 의 `has_pending_input` 은 도구 완료 알림을 우리에게 보내는 그 프로세스 안에서 곧바로 돈다」 (almost always misses — it checks the pending input inside the same process as soon as the tool finishes; codex's has_pending_input runs immediately inside the very process that sends us the tool-completion notification) → it hit 10/10. There are 6–64 ms between the completion notification and the check (M7).
3. §3-1 row 1 — 「**그 사이에 I/O 양보가 없다 — 접기 직전의 `await dc(D,kg)` 는 빈 async 함수다**」 (there is no I/O yield in between — the await dc(D,kg) right before the fold is an empty async function) → inside the fold there is an async 「prompt.submit at the fold」 handling (`ah`/`Ed`) per pending command, and here it took ~0.55–0.66 s. ★The conclusion (S1 misses on claude) stands★ — because the snapshot is taken **before** that wait (M7 0/10). What changes is that the window of 「경계 직전까지 취소」 (cancel up to just before the boundary) is wider than thought (M3 b1–b4·tb1).
4. §5-5 tool count — 「**턴 경계에서 0 으로**」 (reset to 0 at the turn boundary) alone is not enough: the `commandExecution` `item/completed` of an interrupted turn arrives with the old turn id after the following turns (M6 T2).
5. Reading-baseline version — the TRD read the codex **0.154.0** source and the installed one is **0.156.1**. M6·M7·M8 are measured on 0.156.1.
