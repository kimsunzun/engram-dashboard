# How input sent mid-turn is displayed and delivered — chat and agent client survey (2026-09-25)

- **Status:** adversarial review incorporated (verdict BLOCK → all 4 findings accepted and corrected — §8) · decision pending
- **Method:** `/research` medium · design-decision mode · 4 collection tracks (direct peer GUIs · terminal agent CLIs · general chat UIs · the protocols of our two backends) · main grounding (10 load-bearing claims checked against primary sources) · one cross-family adversarial review (codex)
- **Confidence legend:** certain (two or more independent primary sources) · likely (one primary source or partial support) · uncertain (inference or secondary source) · contested (a reasoned rebuttal exists)
- **Trigger:** user request (2026-09-25) — text sent in codex chat mode shows up late → 「먼저 띄우고 진짜가 오면 지운다」 (show it first, remove it when the real one arrives), and do claude in the same pass.

## 0. Conclusion (first)

1. **Not blocking sends mid-turn is the current practice.** Almost every agent tool allows it (Claude Code · Codex CLI/app · t3code · paseo · Cline · Roo · Zed · Cursor · OpenCode). The ones that block are the old defaults (plain ChatGPT chat · Vercel AI SDK examples). — likely
2. **A sent message is shown immediately, but shown separately as 「아직 안 보냄/안 받음」 ("not yet sent/received") — and the official message's position in the conversation is the delivery time, not the send time.** Human messengers show it in the send position with a pending marker (clock · gray) and it becomes official after server confirmation (the Matrix spec requires immediate local echo as a MUST). Agent tools put pending items in a **list above the input box or at the very bottom of the conversation** in gray, labeled 「queued」. The moment it becomes official differs:
   - **Codex TUI** — a mid-turn steer is not put into the history when sent. It sits in a panel above the input box, and goes into the history the moment the server returns the `userMessage` carrying the `clientId` (= the core has **consumed** it). — likely (source)
   - **t3code** — the pending row sits at the end of the timeline and is erased **the moment it is taken off the queue and sent** (tool boundary · turn end). In its place an optimistic user row with a new id appears, and when that id shows up in the server thread it is replaced by the server row. The switch point is not consumption but **the moment we sent it**. — likely (source)
   - **Claude Code** — keeps the pending item above the input box (from 2.1.275 「대화 안, 스피너 위」 ("inside the conversation, above the spinner")) and keeps it gray 「Claude 가 응답을 시작할 때까지」 ("until Claude starts responding"). **Where it is placed after that start is not in the documentation.** — likely (docs · changelog) · final position uncertain
   - What is commonly confirmed = **it is not fixed officially at the send position** (two sources: Codex TUI · t3code). The exception is Orca (to avoid flicker). — likely
3. **A majority default to 「다음 도구 경계에서 끼워 넣기(steer)」 ("insert at the next tool boundary (steer)").** Claude Code (Enter) · Codex (Enter=steer since 0.98.0, Tab=wait until the end of the turn) · paseo · Cursor. Waiting (until the end of the turn) is the default in t3code · Zed · Cline. Interrupt is always a separate key. — likely
4. **The key to merging is a client-made id.** Content comparison is used only on old-server fallbacks (Codex TUI) or id-less PTY paths (Orca). — likely
5. **Both of our backends can support this shape.** The claude CLI **folds mid-turn stdin input into the next tool boundary by itself** and reports that progress through `command_lifecycle` (queued → started → completed/cancelled …) carrying our uuid. The codex app-server inserts it with `turn/steer` (or an in-progress `turn/start` on 0.148.0 and up), and when it is consumed it returns the `userMessage` carrying the `clientId`. — likely

## 1. Checking the user's hypotheses

| Hypothesis | Verdict | Basis |
|---|---|---|
| 「일반 채팅은 그냥 뿌린다」 (general chat just dumps it) | **partially supported** — human messengers dump it immediately but mark it **as pending**. For AI chat the old default was 「보내기 막기」 ("block sending"); agent tools show it immediately but put it **outside the conversation (bottom · above the input box) as pending** | §2-3 |
| 「터미널은 그 상황에서 치면 대기가 있다」 (in a terminal, typing in that situation gives a wait) | **supported** — Claude Code lines it up in gray above the input box, and Codex shows it in a panel above the input box split into 「다음 도구 호출 뒤 제출」 (submit after the next tool call) and 「턴 끝에 제출」 (submit at turn end) | §2-2 |

## 2. Findings

### 2-1. Direct peers (GUI clients)

| Peer | Send mid-turn | Display | Delivery | Merge | Cancel · failure | Confidence |
|---|---|---|---|---|---|---|
| **t3code** | allowed · setting `followUpBehavior` = queue (default)/steer | a dashed bubble at the **end** of the timeline + a status line (「Sends after the next tool call or when the turn ends」) | sends the pending item **one per tool-call boundary** or at turn end. On the server side the claude adapter treats an in-progress send as a steer, codex as an in-progress `turn/start` | **two steps** — ① at the moment of sending, it erases the pending row from the queue (`queuedMessageStore.ts:84-106`) and creates an optimistic user row with a new id (`messageIdForSend`) (`ChatView.tsx:7785-7803`) ② when that id appears in the server thread, it **removes** the optimistic row (`ChatView.tsx:5769-5795`) | Send now · remove (returns to the input box) · Stop moves the whole queue to the input box · on failure it stays at the front | likely (checked against source — the draft's 「대기 행이 서버 id 도착 때 정식이 된다」 ("the pending row becomes official when the server id arrives") was a misreading, §8) |
| **paseo** | allowed · `sendBehavior` = steer (default)/interrupt/queue | steer·interrupt = an `isPending` row immediately in the stream · queue = a pending track inside the input box | steer = by the in-progress `turnId` (codex `turn/steer`) | carries `clientMessageId` as `messageId` and merges in place with the item that comes back · on RPC rejection the optimistic row is removed | edit = back to the input box · on failure re-inserted at the front | likely (checked against `storage.ts:121,194` · `stream.ts:89-119`) |
| **orca** (chat view over a PTY) | the button turns into Stop, but Enter still sends (presumed) | drawn **exactly like an official user turn** — a comment says the 「queued」 style flickered so it was dropped | typed into the TUI | **content comparison** (normalized body after the send-time boundary + count) | on interrupt the echo is removed | uncertain (the collector only read the code) |
| **vibe-kanban** | a 「Queue」 button | pending inside the input box (not in the conversation) | one slot per session, a follow-up turn only on success | not needed | cancelled by typing | uncertain |
| **Cline** | allowed | not put in the conversation; a **panel above the input box** | queue (VS Code) · the SDK also supports steer | the panel mirrors the server list | cancel per item | uncertain |
| **Claude Code VS Code extension** | allowed | from 2.1.274 「Claude 가 시작할 때까지 대화 맨 아래에서 기다린다」 ("waits at the very bottom of the conversation until Claude starts") | the CLI's queue/steer | unknown | unknown | likely (changelog) |
| **Codex app / VS Code extension** | allowed · Follow-up behavior setting (Steer/Queue) | a pending list (position unknown) | steer = `turn/steer` | unknown | on edit it goes to the back (bug #22895) · a bug where pending items vanish (#31128) | likely (docs) |
| **Zed** | allowed · default queue | pending items in a panel | turn end · per-item Steer is **only for its own agent** (external ACP agents cannot see turn boundaries) | unknown | edit · delete | likely (docs) |
| **Cursor** | allowed · Enter = deliver at the next tool call · Alt+Enter wait · Ctrl+Enter interrupt | unknown | steer by default | unknown | Send now | uncertain (changelog) |
| **Roo Code** | allowed (3.25 and up) | a 「Queued Messages」 card | FIFO when ready | unknown | edit · delete · no reordering | likely (docs) |
| **OpenCode** | allowed | a **QUEUED badge** inside the conversation (at the send position) | next model request | unknown | a bug where the badge never clears (#16856) | uncertain |

**Summary:** the display position has two branches — ① a pending row at the **very bottom** of the conversation (t3code · Claude Code · OpenCode · paseo steer) ② a separate list **above/inside the input box** (Cline · Roo · Zed · vibe-kanban · paseo queue). Only one place **fixes it at the send position as if official**: Orca (the reason is flicker avoidance).

### 2-2. Terminal agent CLIs

- **Claude Code** — Enter does not interrupt; it puts the message in the queue and shows it as a **list above the input box**. 「Sent and queued messages show in gray until Claude starts responding to them」. Delivery: a message entered during a tool call is handed over **the moment that tool call ends, within the same turn**. If it is still there after the turn ends, it is sent automatically in the order typed. Commands and shell are held until the end of the turn. Esc = interrupt and send the pending items next · Ctrl+Enter = interrupt and send now (2.1.275 and up) · Up = pull the pending items back into the input box. — display, delivery timing and keys are **certain** (checked against the original text of the official docs interactive-mode 「Queue messages while Claude works」 + changelog 2.1.274/2.1.275 + the `command_lifecycle` description string in the installed 2.1.280 binary). ★**Where that message is placed in the conversation after the response starts is in no source**★ — uncertain (§8 finding 2)
- **Codex CLI TUI** — Enter (while a turn is in progress) = `turn/steer` immediately · Tab = local pending, a new turn when idle (steer became the default in 0.98.0). The panel above the input box has three sections: 「다음 도구 호출 뒤 제출(Esc 로 끊고 즉시)」 (submit after the next tool call (interrupt with Esc and send immediately)) · 「턴 끝에 제출」 (submit at turn end) (a rejected steer) · 「대기 중 후속 입력」 (follow-up input waiting). **A steer is not put into the history when sent** (`render_in_history = !agent_turn_running`) — it goes into the history when the `userMessage` returned by the server arrives, so **the position is the delivery time**. Merge = match the `clientId` of the **front** of the queue (old servers fall back to comparing body + image count). A plain Esc (interrupt) **restores the pending items to the input box**. — likely (checked against source `rust-v0.154.0` `input_submission.rs:177` · `chatwidget.rs:1287-1336` · the 0.98.0 release notes are quoted by the collector)
- **Gemini CLI** (default turn end · experimental steering) · **Crush** (turn end) · **Amp** (default interrupt, `/queue` to wait) · **OpenCode** (QUEUED badge in the conversation). — uncertain

### 2-3. General chat UIs

- **Human messengers:** immediate local echo + a pending marker + merging with the server copy by client id is the norm — the Matrix spec makes immediate echo a MUST, allows a different appearance before the server response as a MAY, and merges by `transaction_id` · Discord `nonce` · WhatsApp clock icon (secondary source). No official source was found for the failure display or the ordering rule against messages that arrive while pending (only the `chronological`/`detached` options of the Matrix SDK). — likely
- **AI chat:** old default = block sending while streaming (Vercel AI SDK example `disabled={status !== 'ready'}` · plain ChatGPT chat). New OSS UIs (Open WebUI — on by default, a pending area **right above the input box**, and when the response ends it joins the pending items with blank lines and sends them **all at once** · assistant-ui pending list + Steer) keep the pending items outside the conversation. — likely (checked against the original text of the Open WebUI docs)

### 2-4. What our two backends can do

**claude (stream-json chat mode)**
- A `user` line on stdin mid-turn is put by the CLI into its own command queue, and if a turn is in progress it is **folded in at the next tool boundary**. It reports progress as `{"type":"command_lifecycle","command_uuid":<우리 uuid>,"state":…}` — `queued` → `started` → `completed`/`cancelled`/`discarded`/`refused`. 「a command folded into an already-in-flight turn emits 'completed' BEFORE that turn's result frame」. A message without a uuid gets no event. — likely (checked against the original text of the installed 2.1.280 binary · it carries an 「@internal」 marker so it is unknown whether this is a stable contract · the consumption time is a third-party measurement)
- ★**The meaning of the lifecycle states must not be lumped together as 「받음」 ("received")**★ (§8 finding 3 · original text of the binary's description): `started` = **drained** from the queue into the turn, `completed` = the **turn that consumed it ended** (the answer may still come only in a later run). On abnormal termination `started` may get no terminal state or only `queued` may remain, and there is a guideline telling the host to **synthesize** `discarded` for unterminated uuids when the process exits. There is also a warning not to blindly resend on `cancelled`. So the design must handle **「전달됨(started)」 · 「처리된 턴 끝남(completed)」 · 「답이 나옴」** ("delivered (started)" · "the processed turn ended (completed)" · "the answer appeared") separately, and must have rules for recovery, resending and duplicate prevention when no terminal state arrives. — likely
- Whether the `--replay-user-messages` echo comes **when written** or **when consumed** is unmeasured (inference from the minified code says consumption time). — uncertain
- **Our code today:** input goes straight to stdin (no turn gate) · a synthetic echo is shown in the official form at the **send position** the moment it is written · `command_lifecycle` is a type the decoder does not know, so it is dropped · the accumulator keeps the first item per uuid, so the position is fixed where it was sent.

**codex (app-server chat mode)**
- `turn/steer{threadId, clientUserMessageId?, input, expectedTurnId(필수)}` — if there is no active turn or it has ended, −32600 「no active turn to steer」; on an id mismatch an error; review and compaction turns cannot be steered. On 0.148.0 and up an **in-progress `turn/start` is also handled as a steer** and returns the same turn id (`start_or_steer_turn` → `Steered`) — on this path the 「턴이 막 끝났는데 steer」 ("steer just after the turn ended") race is resolved atomically inside the core. — likely (checked against source `v2/turn.rs:277-299` · `turn_processor.rs:636-668` · the 0.148.0 lower bound was checked only against the collector's tags)
- Inserted input is consumed after the current sampling and its tool calls end, and **the moment it is consumed** a `userMessage` carrying the `clientId` (`item/started`·`item/completed`) comes out. — likely (the collector's code reading + vendor tests)
- **On interrupt (`turn/interrupt`), pending input that has not been consumed is dropped without an echo** (`clear_pending`). — likely (checked against `tasks/mod.rs:520-534` — the function body was not read)
- **Our code today:** input during a turn is held in our queue and sent as a new `turn/start` when the turn ends · `clientUserMessageId` is not carried · no synthetic echo (ADR-0198 decided, not implemented).

## 3. Constraints (engram — `CLAUDE.md` · ADR)

| Constraint | Source | Requirement on the design |
|---|---|---|
| LLM-first control · the frontend only renders | CLAUDE.md 「LLM-우선 제어」 | the pending/official state must flow as **backend ring events** — an optimistic state held only in the frontend is a second control path |
| Backend extensibility | CLAUDE.md 「백엔드 확장」 · ADR-0004 | knowledge of `command_lifecycle`·`clientId`·steer lives only inside `backend/claude`·`backend/codex` |
| replay→live · seq dedup | core invariant | after a reconnect replay, the pending/official state must converge to the same result |
| Each pathway decides queue release | ADR-0193 | do not use the echo as evidence that 「턴이 돈다」 ("a turn is running") · steer reopens the 「한 번에 한 턴 시작」 ("start one turn at a time") gate |
| Synthetic echo + `clientId` merge · version floor 0.140.0 | ADR-0198 | use the decision as is, but 「어디에·어떤 모양으로」 ("where and in what shape") is this survey's job |
| Input order during hydration | ADR-0198 impact section · ADR-0204 · ADR-0226 | the problem of text typed during restore being drawn **above** the old conversation — the 「맨 아래 대기」 ("pending at the bottom") shape resolves it naturally (pending is always the tail) |
| claude is judged against its defaults | ADR-0192 | both backends must have the same shape |

## 4. Options (fit against the constraints)

A and C are **two display positions of the same principle** — both are 「대기는 따로, 정식 위치는 전달 시점」 ("pending is kept separate, the official position is the delivery time"), and the only difference is where the pending items go. Only B has a different principle.

| | A. Pending row at the bottom of the conversation (recommended) | B. Fixed officially at the send position | C. Pending list above the input box |
|---|---|---|---|
| Shape | on send, immediately a **gray 「대기」 (pending) at the very bottom of the conversation (below the streaming answer)** · official when delivered | on send, immediately **the official shape at the send position** (the original ADR-0198 plan · claude's behavior today) | not put in the conversation; a **list above the input box** · appears in the conversation when delivered |
| Peers | t3code (timeline end) · Claude Code 2.1.275 and up (above the spinner in the conversation) · OpenCode (badge) | Orca | Codex TUI · Claude Code docs description · Cline · Roo · Zed · Open WebUI · vibe-kanban |
| Order accuracy | ○ official position = delivery time | ✕ mid-turn, the rest of the earlier answer continues below it | ○ |
| Feel of 「바로 뿌린다」 ("dumped right away") | ○ immediately inside the conversation flow | ○ | △ outside the conversation (right above the input box) |
| Row movement | the pending row is always the tail, so the position barely changes when it becomes official | none | disappears from the list and newly appears in the conversation |
| Hydration order problem | ○ pending is always the tail, so it never goes above the old conversation | ✕ exactly as ADR-0198 left it unresolved | ○ |
| Interrupt · failure handling | restore undelivered pending items to the input box (Codex TUI·t3code) or show a failure | it already looks official, so 「안 간 말」 ("a message that never went") remains | restore from the list |
| Implementation cost | large — pending/delivered state as ring events · decode claude `command_lifecycle` · turn the accumulator's 「첫 항목 고정」 ("fix the first item") into 「전달 시점 배치」 ("place at delivery time") | small (codex only as in ADR-0198) | the same state machine as A + one surface above the input box |
| LLM-first control | ○ if it is a ring event | ○ | ○ it just draws the same ring event in a different place (the draft's △ had no basis — §8 finding 4) |

**State machine common to both backends (needed whether A or C):** pending (sent but not delivered) → delivered (claude `started` · codex `userMessage` carrying the `clientId` · or the time we took it off the queue and sent it) → official. Cases where no terminal state arrives (claude abnormal exit·`discarded`/`cancelled` · codex pending input lost to an interrupt) end with **restoring to the input box or a 「안 보내짐」 ("not sent") marker** — no automatic resend (the warning in claude's description · the codex `clientUserMessageId` is not an idempotency key — ADR-0198).

**To decide together (coupled):** the delivery method — ① both backends steer (insert at the next tool boundary · claude's CLI already does so · codex uses an in-progress `turn/start` or `turn/steer`) ② only codex waits until the end of the turn (today). With ①, A's 「받는 순간」 ("moment of receipt") comes at every tool boundary; with ②, at the end of the turn. Interrupt-only actions (Codex Esc · Claude Ctrl+Enter) are a separate matter.

## 5. Rejected candidates (→ ADR rejected-alternative candidates)

- **Blocking sends (disabled while streaming)** — the old default, but it is the opposite of current agent-tool practice, and the claude path already allows it (ADR-0044 mechanism A), so it would be a regression.
- **Merging by content comparison** — already rejected by ADR-0198. Both backends have ids (claude uuid · codex clientId). Orca uses content comparison because it is a PTY with no id.
- **Joining the pending items into one prompt and sending (Open WebUI)** — both backends merge by per-message id, so joining them collapses the ids into one and breaks the merge.

## 6. Open issues · limits

- Where a Claude Code pending item is placed **when it becomes official** (delivery position vs send position) is an inference from the changelog wording — closed source.
- claude `command_lifecycle` carries an 「@internal」 marker — it is unknown whether it is a stable contract. The minimum version is also unknown.
- The timing of the claude echo (`isReplay`) (when written/when consumed) is unmeasured — one measurement would settle it (type during a long turn that uses tools).
- On codex 0.140–0.145, the behavior of an in-progress `turn/start` · the version that introduced `turn/steer` · whether `clientId` survives in the resume history — unconfirmed.
- Human messengers' failure display and ordering rules have no official source.
- Peers not surveyed: Claude desktop app · Conductor · Warp · Continue · Sculptor · Kilo Code.

## 7. Grounding results (main check)

| Claim | Verdict | Checked against |
|---|---|---|
| Codex TUI does not put a mid-turn steer into the history when sent | supported | `input_submission.rs:177`@v0.154.0 |
| Codex TUI merges with the front of the queue by `clientId` | supported | `chatwidget.rs:1287-1336`@v0.154.0 |
| `TurnSteerParams` has `clientUserMessageId` and required `expectedTurnId` | supported | `v2/turn.rs:277-299`@v0.154.0 |
| An in-progress `turn/start` is handled as a steer | supported (0.154.0) · the 0.148 lower bound not checked | `turn_processor.rs:636-668` |
| Pending input is dropped on interrupt | partially supported | `tasks/mod.rs:520-534` (the `clear_pending` call on the abort path — body not read) |
| claude folds mid-turn input into the in-progress turn and emits lifecycle | supported | original string of the installed 2.1.280 binary |
| Claude Code: gray pending · delivered in the same turn when the tool call ends | supported | original text of the official docs interactive-mode |
| t3code: pending row at the timeline end · the optimistic row is removed when the id appears on the server | **downgraded to partially supported** (review rebuttal · main re-checked) — what the server-id comparison erases is not the pending row but the **optimistic row newly created at send time** | `MessagesTimeline.tsx:1770-1772` · `ChatView.tsx:5769-5795` · re-checked `queuedMessageStore.ts:84-106` · `ChatView.tsx:7785-7803` |
| paseo: default steer · keyed by `clientMessageId` | partially supported | `storage.ts:121,194` · `stream.ts:89-119` (the merge function body not read) |
| Open WebUI: right above the input box · joined and sent at once when it ends | supported | original text of the official docs |

## 8. Adversarial review results

- **Reviewer:** codex (cross-family) · effort high · web search on · level 2 (recheck) · 2026-09-25
- **Verdict: BLOCK** — one conclusion (§0.2 「받는 순간 정식」 ("official at the moment of receipt") and its confidence) went beyond its evidence. All 4 findings carried rebuttal sources and **all were accepted and the body corrected** (no contested).

| # | Finding | Type | Severity | Handling |
|---|---|---|---|---|
| 1 | t3code erases the pending row **when it sends** and creates an optimistic row with a new id — the server-id comparison that was cited is the code that erases that optimistic row. It is no basis for 「받는 순간 정식」 ("official at the moment of receipt") | misattribution · overstatement | high | main confirmed by re-checking `queuedMessageStore.ts:84-106`·`ChatView.tsx:7785-7803` → corrected §0.2·§2-1·§7 |
| 2 | The Claude Code docs only say the gray display and do not say **where it is placed** after the response starts — 「확실」 ("certain") is an overstatement | overstatement · logical gap | high | confidence downgraded in §0.2·§2-2, A's peer basis recomputed (the Codex TUI·Claude Code docs description are above the input box → on C's side) |
| 3 | `started`/`completed` of `command_lifecycle` are not 「받음」 ("received") — on abnormal termination no terminal state may arrive and there is a guideline to synthesize `discarded`. The design has no rules for recovery, resending and duplicate prevention | no basis · omission | high | stated the meaning of the states in §2-4, added the common state machine and 「자동 재전송 안 함」 ("no automatic resend") in §4 |
| 4 | C's LLM-first control △ has no basis — drawing the same ring event above the input box is enough | logical gap | medium | corrected the §4 table, compared A·C by visibility·row movement·cost |

- **Grounding spot re-check (reviewer):** Codex TUI no history insertion mid-turn · front-of-queue `clientId` match · `TurnSteerParams` fields · Claude Code gray/same-turn delivery · Open WebUI — supported. The t3code row — not supported (1 above).
- **Conclusion that remains after the correction:** 「대기는 따로 보이고, 정식 위치는 보낸 시점이 아니라 전달 시점」 ("pending is shown separately, and the official position is the delivery time, not the send time") stands on two sources, Codex TUI·t3code. A (bottom of the conversation) and C (above the input box) are display-position choices of that principle, and only B (fixed at the send position) has a different principle. **No re-review was run** (medium single pass) — the corrected parts are a surface the reviewer has not seen.
