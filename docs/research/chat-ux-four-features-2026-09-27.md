# Survey of 4 chat-screen features — Esc interrupt · scroll following · grouping tool calls · claude text streaming

| Item | Value |
|---|---|
| **Status** | medium 1 pass — 4 collection branches (1 per feature) · main grounding · cross-family adversarial review (§6) |
| **Date** | 2026-09-27 |
| **Method** | close reading of this repository's code · close reading of the reference clones (`I:\Engram_Workspace\opensource\` — t3code·paseo·orca·cline·vibe-kanban) · web · `gh api` lookups · 1 measured claude 2.1.280 stream sample (scratch, outside the repository) |
| **Confidence legend** | **certain** = two or more primary sources, or a measurement + main cross-check · **likely** = one source + main cross-check, or only the collector's close reading · **uncertain** = circumstantial only · **grounding ✓** = the main agent opened the cited `파일:줄` (file:line) itself and confirmed that it matches the claim |
| **Why** | The design premise for the next feature bundle (TRD = `docs/process/S21-chat-ux/trd.md`). The user said the handoff note's proposal is enough for a PRD (2026-09-27) and will be briefed starting from the TRD |

> ★**Read this first — two premises of the handoff note are wrong**★
>
> 1. ★**「끊기는 백엔드에 이미 있다」 (interrupt already exists in the backend) is true only for codex**★ — claude JSON runs on `StdioTransport` (`crates/engram-dashboard-agent/src/backend/claude/mod.rs:428`), whose `interrupt()` returns `Unsupported`, and its capability flag is also `interrupt: false` (`transport/stdio.rs:371-375` · `:441`, grounding ✓). On top of that, **there is not a single place in the screen that calls interrupt, and it is not in the command table either** — only the client method (`src/api/protocolClient.ts:988`) exists (likely). So Esc is not one key but **a claude interrupt implementation + command registration + a key**.
> 2. ★**codex does not announce the end or the error of a tool call live**★ — the translator emits a tool call only on `item/started` and deliberately drops `item/completed` (`backend/codex/decoder.rs:256-263`, grounding ✓). So 「묶음 요약에 오류 표시」 (showing errors in the group summary) **cannot be done on codex without a wire change.**

---

## 0. Conclusion

| Feature | Recommendation | Size | Where a user decision remains |
|---|---|---|---|
| **Scroll following** | One hook shared by both slots + a pure decision core (written ourselves) | small — frontend only | the shape of the 「맨 아래로」 (to the bottom) button · re-sticking on send |
| **Esc interrupt** | only inside the chat pane · only while running · keep the input text · register as the command `agent.interrupt` · **claude is conditional on a spike first** (§6) | medium — **claude interrupt is new backend work** | whether to include claude (codex first only?) |
| **Grouping tool calls** | Group with a pure function at render time, and have **each backend's translator carry the kind (search · read …) as a neutral value** | medium — 1 wire-type field + both translators + frontend | whether to do codex error display this time |
| **claude streaming** | Add the flag + per-block state in the translator + 「흘린 블록의 완결 본문은 버린다」 (drop the completed body of a block that was already streamed) | medium — backend only, frontend and wire types unchanged | none (internal implementation) |

---

## 1. Scroll following

### 1-1. Current code

- Both places **scroll to the bottom unconditionally** — `src/components/slot/RichSlot.tsx:288-291` (every time `[items]` changes) · `src/components/slot/DomSlot.tsx:182-185` (`[text]`). grounding ✓. The scroll node is the Viewport of the Radix ScrollArea (ADR-0053).
- RichSlot does `setItems([...acc.snapshot()])` every frame, so each streaming delta triggers the effect again (`RichSlot.tsx:215`, likely). No virtualization.
- **Places where the height changes without a new item** (likely): expanding a thinking block (`chat/ThoughtRow.tsx` — it has **its own scroll area** inside) · the waiting list above the input box (a **sibling outside the ScrollArea**, so it shrinks the viewport, `RichSlot.tsx:458`) · collapsing/expanding this round's tool group.
- Tabs are kept alive with `display:none` (`src/components/layout/WindowLayout.tsx:191-201`, ADR-0056) — while hidden the height is 0, so the check 「바닥과의 거리 ≤ 문턱」 (distance to the bottom ≤ threshold) becomes a false positive. Whether `scrollTop` is preserved across `display:none` is **unknown** (needs a GUI measurement).
- While the empty state is shown, the ScrollArea itself is taken down (`RichSlot.tsx:397`) — the hook has to survive the viewport disappearing and being attached again as a new node.
- Scroll-related tests 0 · commands 0 (`rg -i scroll src/commands` → 0).

### 1-2. Peers

| Peer | 「바닥」 (bottom) threshold | Telling user intent | In-place growth | Button |
|---|---|---|---|---|
| VS Code chat (MIT) | 2px | scroll position + a lock flag, held while text is selected | captures whether it was at the bottom **before the growth** and reveals afterwards | a chevron when not at the bottom, no count |
| t3code (MIT) | 40px | wheel·touch·pointer·navigation keys | the list library's keep-at-end | a 「끝으로」 (to the end) pill — **only the showing is delayed 150ms** (prevents a flicker on tab switch) |
| paseo (Apache-2.0) | 64px | it releases only with **evidence of upward input** · input that an inner scroller would take is ignored | ResizeObserver | a chevron when not at the bottom · **it re-sticks on send** |
| cline | 10px | wheel up → automatically off · turned on again when a new turn arrives | `overflowAnchor:none` | an ArrowDown round button (cline-hub) |

(Source `파일:줄` (file:line) = the collection record. The collector's close reading — likely.)

- **`use-stick-to-bottom`** (MIT, 778★, maintained) is made for exactly this problem, but its wheel-escape decision looks for an ancestor whose computed `overflow` is `scroll`/`auto`, and the Radix viewport is likely to be computed as `hidden scroll`, so **the wheel escape may not trigger** (uncertain — not run). It also attaches a document-wide mousedown listener.
- A CSS `overflow-anchor` pin · `column-reverse` do not give a 「지금 붙어 있나」 (is it stuck now) state, so they cannot be used for the button or an LLM query.
- ★**Not a single peer showed the button 「새 내용이 왔을 때만」 (only when new content arrives)**★ — all of them show it 「바닥이 아니면」 (when not at the bottom) and attach no count.

### 1-3. Options

1. **(Recommended) A self-written shared hook + a pure core** — the stuck flag changes only by scroll events (comparing with the value we wrote, to filter out our own scrolls) and by upward input (wheel·PageUp·Home…). The content and viewport ResizeObservers scroll down again only while it is stuck. **It does not measure again after growth** — measuring would read the streaming growth as 「사용자가 위로 올렸다」 (the user scrolled up) (the most dangerous pitfall).
2. `use-stick-to-bottom` — the suspected Radix mismatch above · its own spring scroller · a global listener.
3. CSS only — there is no state.
4. Introduce virtualization — not needed now.

---

## 2. Esc interrupt

### 2-1. Current code

- The interrupt call path is the same for all three backends (`connection_core.rs:1222-1227` → manager → session → transport, likely). Only a client holding the **input lease** gets through.
  - codex: sends a real `turn/interrupt` · with no open turn it is `Unsupported("중단할 턴이 없다")` (`backend/codex/transport.rs:4041-4076`, likely).
  - claude JSON: always `Unsupported` (grounding ✓).
  - PTY: `0x03` (Ctrl-C). Out of scope this time.
- ★**claude has an interrupt control line**★ — the official Agent SDK sends `control_request {subtype:"interrupt"}` to the CLI (`anthropics/claude-agent-sdk-python` `src/claude_agent_sdk/_internal/query.py:792-794`, grounding ✓ — the SDK launches the CLI with the same `--input-format stream-json` as ours). This repository already uses a `control_request cancel_async_message` of the same shape (`claude/mod.rs:694-722`, likely). ★**What the CLI does with the pending input it has already received after an interrupt is unknown**★ — the interactive-mode docs say 「대기 메시지가 있으면 다음에 보낸다」 (if there is a pending message, it is sent next) (https://code.claude.com/docs/en/interactive-mode), but it is unmeasured in stream-json.
- On the screen side: the `onKeyDown` of the chat input stops the propagation of every key (`RichSlot.tsx:471-481`, grounding ✓) · global shortcuts filter out input elements and the inside of `.xterm` (`src/commands/keybindings.ts:7-35`, grounding ✓ — ★a load-bearing guard★). So Esc has to be hooked **inside the chat slot, not in the global shortcut table**.
- The 「턴이 도는 중」 (turn is running) signal is only a view-local value — `streaming = awaiting || (!turnDone && items>0 && !historyPending)` (`RichSlot.tsx:370`, likely).
- There are three popovers closed by a document-wide Esc (`AgentMonitoringPicker` · the `AgentList` row menu · `PresetPalette`) — since the input box stops propagation, **the input box's Esc may take it first** (nothing floats above the chat right now — likely).

### 2-2. Peers

| Peer | Key | Scope | Input text | When not running | Twice |
|---|---|---|---|---|---|
| Claude Code CLI | Esc | the prompt (a dialog goes first) | kept | — | Esc Esc = clear text/rewind |
| codex TUI | Esc | the input field | kept | — | twice on an empty field = edit the previous message |
| **paseo** (nearest GUI) | Esc | **only the pane of the active input field** · terminals excluded | **kept** (e2e test) | lets the key pass through | none |
| t3code | no default (rebindable) | — | — | — | — |
| orca | does not hook it (passes it to the PTY) | — | — | — | — |
| Cursor | Ctrl+Shift+Backspace | — | — | — | — |

(The collector's close reading · docs — likely. The Claude Code docs wording is certain.)

- Trend: the terminal family (Claude Code·codex·paseo) is **a bare Esc · input-field scope · text kept · do nothing when not running**. Those that live inside an editor (Cursor·Zed·t3code) avoid it because Esc is already 「포커스 해제」 (release focus).
- **Claude Code kills a running Bash tool on Esc** (measured 2026-09-27 · interactive CLI — certain): a command that printed a line for 90 seconds stopped writing the moment Esc was pressed at the 10-second mark, and that process was gone afterwards · the screen showed 「Interrupted」 immediately. With codex, an interrupted running command keeps running (TRD §9 F2). Whether our stream-json `control_request` interrupt does the same is untested (TRD §3-5 S1 record).

### 2-3. Options

- Scope: **S1** only when the input box has focus · **S2 (recommended)** anywhere inside that chat pane (input box + conversation body — the pane root takes focus on click) · S3 global (an exception in the guard — rejected, it opens the load-bearing guard above).
- Firing conditions (recommended, peer consensus): not composing · no popover · running · capability `control.interrupt` true · a bare Esc (no modifier key) · not a key repeat. The input text is always kept. Pressing twice is **deferred** (a separate feature).
- Registration (§5): put the **command `agent.interrupt {agentId}`** in the frontend command table. Whether to add a daemon bus counterpart follows the `agent.cancelQueuedInput` precedent (the daemon declares the name and the frontend invokes the same name without `help` — `src/commands/agentCommands.ts:127-158`). Putting it in the global shortcut table (`BINDINGS`) is rejected — both the guard and the input box's propagation stop would have to be lifted.

---

## 3. Grouping tool calls

### 3-1. Current code

- The wire type `StructuredEvent` (`crates/engram-dashboard-protocol/src/messages.rs:711-788`) has `ToolCall{name,args_json,id,…}` and **no tool-result variant** (likely).
  - claude: `tool_use` → `ToolCall` (name = the vendor tool name) · `tool_result` (including `is_error`) goes as the whole user block in `Structured{kind:"user"}` and **the frontend parses that vendor shape** (`StructuredTextView.tsx:96-111` — a place where vendor knowledge has already leaked into the frontend).
  - codex: six kinds of tool items (`decoder.rs:270-281`) · name = for MCP the `tool` field, otherwise the item type (`commandExecution`·`fileChange`·`webSearch`…) · the `commandActions` that would tell 「검색이냐 읽기냐」 (search or read) is inside `args_json` (past the size cap it is only a `{type,id}` stub).
- **There is no normalized kind value** — the icon is guessed from a substring of the name (`StructuredTextView.tsx:150-167`).
- Accumulator: `ToolCall` → one `{kind:'tool',…}` item (`structuredAccumulator.ts:143-151`, grounding ✓) · turn end = `MessageDone`/`TurnEnd` → a divider item. Tool rows are **collapsed by default**, and the expanded state is the row component's `useState`, so it disappears on resubscribe (replay).
- The invariant that the rail (`chat/railPositions.ts`) and the DOM row kinds must match (ADR-0051 — `rowKindOf` ↔ `renderItem`).

### 3-2. Peers

- **codex TUI** (`openai/codex` `codex-rs/tui/src/exec_cell/model.rs`) — groups **only consecutive 「탐색」 (exploration) calls** (read·list·search) and breaks on another call · 「Exploring」 while running, 「Explored」 when done · the failure count in red at the head · hidden thinking is absorbed into the group (likely).
- **Claude Code CLI** (`CHANGELOG.md`) — a read·search group in the present tense while running and the past tense when done · expanded with Ctrl+O · 「묶음이 화면 밖에서 끝날 때 스크롤이 튄다」 (the scroll jumps when a group ends off-screen) was fixed in 2.1.83 (certain — the change-log wording).
- **t3code** — **derived at render time** (`apps/web/src/components/chat/MessagesTimeline.logic.ts:1198-1330`) · error-tone items and agent spawns break the group and stand apart · 「Read 3 files, searched code 2 times and ran 1 command」 · expanded state = keyed by the first call's id · **it stops following the end when expanding** (likely).
- vibe-kanban — **the backend normalizes the action kind** and the frontend gathers 2 or more consecutive of the same kind · cline — only 「가벼운」 (light) tools (read·list·search), leaving a running one outside · paseo — a set of expanded group ids.

### 3-3. Options

- Where to group: **(recommended) a pure function at render time** (on replay the same items give the same groups — t3code·vibe-kanban·cline) · the accumulator emitting group items (rejected — it complicates resupply idempotency and the pending-input machinery) · the backend grouping (rejected — it loads a display concern onto the wire).
- Kind decision: a frontend table (fast, but vendor schemas leak further into the frontend — a 「백엔드 확장」 violation) · **(recommended) the translator carries a neutral kind on `ToolCall`** (an optional field — an old daemon sends it empty and the frontend sees it as 「기타」 (other). Precedent: vibe-kanban·codex TUI).
- Error·end display: claude works with today's data (`tool_result.is_error`). For codex, `item/completed` must be translated and emitted anew — creating a new wire variant means **an old shell meeting a new daemon draws a 「지원 안 되는 사건」 (unsupported event) line for every result** (the default branch of `structuredAccumulator.ts`, likely).

---

## 4. claude text streaming

### 4-1. Current code

- The JSON spawn arguments lack `--include-partial-messages` (`claude/mod.rs:184-194`, grounding ✓).
- The translator drops unknown line types — `stream_event` is among them (`claude/mod.rs:1004-1006` `_ => {}`, grounding ✓). **Adding only the flag leaves the screen unchanged.**
- The translator's line handling (`consume_line`) has no per-line state (an associated function) — the resume record (transcript) path calls the same function. Streaming needs state (the current message id · the kind per block number · whether it was streamed) (likely).
- A completed `text` block goes out even now as `TextDelta{text: 전문}` (`claude/mod.rs:1077-1092`, likely).
- **The frontend works unchanged** — the accumulator's `TextDelta` branch looks at neither the backend nor the `message_id` and appends to the last text item (`structuredAccumulator.ts:130-141`, grounding ✓). Conversely, **the frontend has no deduplication, so removal is the translator's job.**
- codex precedent: it emits deltas and **does not translate the completed body live** (only in the history-restore path — `codex/decoder.rs:246-251`, grounding ✓).

### 4-2. Vendor format (measured — claude 2.1.280, a 40-line sample)

- Wrapper `{"type":"stream_event","event":{…},"session_id","parent_tool_use_id","uuid"}`.
- Order: `message_start` (message id) → `content_block_start idx` → deltas → **a completed `assistant` line containing only that block** (same id) → `content_block_stop` → … → `message_delta` → `message_stop` → (for a tool: `user` result → a new `message_start`) → `result`.
- **Deltas carry no message id, only the block number** — the id from `message_start` must be remembered.
- The official docs state the same contract — one completed message for every non-empty block, before that block's `content_block_stop` (https://code.claude.com/docs/en/agent-sdk/streaming-output). → measurement + docs = **certain**.
- Subagent deltas: the docs say 「전달되지 않는다」 (they are not delivered), while the t3code comment says 「온다 — 버린다」 (they come — discard them) (`t3code/apps/server/src/provider/Layers/ClaudeAdapter.ts:2866-2889`). **Uncertain — either way, a text delta whose `parent_tool_use_id` is non-empty is dropped.**
- The thinking deltas in this sample were empty strings (taken without our mode's `MAX_THINKING_TOKENS`) — whether thinking flows in our mode is unknown.
- Whether `stream_event` lines enter the resume record file was **not confirmed** (the collector was denied access). The existing record samples were taken without the flag.

### 4-3. Peers

- **t3code** — per block a 「델타를 냈나」 (did it emit a delta) flag · on completion **if it did, drop; if it did not, emit the completed body** (`ClaudeAdapter.ts:2260-2290`) · subagent text·thinking deltas are dropped.
- paseo — treats the completed body as an absolute snapshot and emits only the tail after the length already emitted (on a prefix mismatch, from 0 — a duplicate corner case).
- vibe-kanban — **replaces** the same-position item with the completed one (a JSON patch — needs an addressed screen item).

### 4-4. Options

- Deduplication: **(recommended) per block — drop the completed body of a streamed `(메시지 id, 블록 번호)`, and emit the completed body if it was not streamed** (the same shape as the t3code · codex live rule · wire types and frontend unchanged) · replace (a new wire variant + replay would have to rewind to the replacement) · tail comparison (more state and a duplicate corner case).
- Scope: **(recommended) text only** — thinking stays as completed blocks as now · ignore `input_json_delta` (a tool is enough as the `ToolCall` of the completed line).
- Merging: merging neighboring deltas within the same translation call is nearly free, but codex does not do it either — until ring pressure (`REPLAY_MAX_EVENTS=4096`) is measured, leaving it out is simpler.
- Pitfalls: moving the turn end to `message_stop` means **a turn ends at every tool call** — the turn end remains the single `result` line. Emitting the `stream_event` auxiliary lines as `Structured` makes turn observation read them as progress (the 「턴 관측」 invariant — a 30-minute stall path).

---

## 5. Grounding results

What the main agent opened and cross-checked against the cited `파일:줄` (file:line) itself = the 10 items marked 「grounding ✓」 in the tables. All **supported**. The remaining `파일:줄` (file:line) are only the collector's close reading (likely). Found during the cross-check: the actual line of the codex completed-body comment is 3 lines below the collection record (`:243-249`), at `:246-251` — the body gives the corrected lines.

## 6. Cross-family adversarial review

**Decision = BLOCK** (codex, effort high · verification level 2~3 · 2026-09-27). Of the 7 priority-check items, all that could be checked were **supported** (claude JSON cannot interrupt · codex has no live tool end · the block-wise stream order · the frontend `TextDelta` appending · the global shortcut guard and the input box's propagation stop). It judged that the Radix wheel suspicion is correctly marked 「미검」 (untested). **All the BLOCK reasons are omissions in the claude Esc interrupt** — not a rebuttal of a claim but missing facts that change the recommendation:

1. **The interrupt response carries 「그래도 돌 대기 메시지」 (the pending messages that will still run)** (`still_queued`) · `cancel_queued`, which clears the pending ones too, is an optional capability (`anthropics/claude-agent-sdk-typescript` CHANGELOG). → It must state explicitly 「지금 턴만 끊나 / 대기까지 끊나」 (does it cut only the current turn / the pending ones too). Whether our intent (the same 「지금 턴만」 (only the current turn) as codex ADR-0235 decision 4) matches this is to be seen in the spike. Severity high.
2. ★**The `result` of an interrupted turn may be `is_error: true`**★ (`terminal_reason` = `aborted_streaming`/`aborted_tools` — `claude-agent-sdk-python` `types.py` · SDK issue #429). Our translator emits `Error` when `is_error` (`claude/mod.rs:944-966`, **main cross-check ✓**) → an error shows on the screen, and the **stop after error of turn observation turns on, so the pending input does not go out.** Severity high.
3. An interrupt sent before `system/init` **gets the same acknowledgement response yet interrupts nothing** (SDK issue #429 — CLI 2.1.241, the main agent cross-checked the issue body ✓). Whether it is the same on 2.1.280 is unknown. → Do not read the acknowledgement response as 「멈췄다」 (it stopped). Severity medium.
4. On interrupt, **a truncated `assistant` message may arrive before the `result`** (SDK issue #338) — how to show the truncated end on the screen is missing. Severity medium.

**Tally:** 1~4 **lower the claude interrupt recommendation of §2 to 「스파이크 선행 · 결과 분류 규칙 필수」 (spike first · result classification rules required), conditional** — handed over as the exit condition of the F2 spike in the TRD (`docs/process/S21-chat-ux/trd.md`). The review did not touch the recommendations of the other three features (passing ≠ proof — a likely ceiling).

## 7. Limits · untested

- Not run: the Radix + `use-stick-to-bottom` wheel mismatch · `scrollTop` preservation across `display:none` · whether a replay runaway is bundled into one commit · the fate of pending input after an interrupt in claude stream-json · whether the resume record has `stream_event` · whether thinking deltas flow in our mode.
- Not found: scroll behavior of ChatGPT·Claude.ai web (abstain) · tool grouping in Copilot·Windsurf (abstain) · Zed's interrupt key (abstain).

---

## 8. Additional survey — how a codex 「거부」 (declined) tool is shown (light · 2026-09-27)

| Item | Value |
|---|---|
| **Status** | light 1 pass — 1 collection · main grounding spot check (1 upstream source place) · no adversarial review (light) |
| **Why** | The ★user confirmation★ 「`declined` 표시」 (the declined display) in TRD edition 3 §4-7 — the user said 「지금도 빨간 박스가 뜨지 않나 · 다른 곳은 어떻게 하나 보고 제안하라」 (doesn't a red box appear even now · look at how others do it and propose) |
| **Decision** | ★User 2026-09-27 = show separately (B)★ — orange 「거부됨」 (declined) · a one-line reason inside the row · summary 「거부 N」 (declined N) (separate from errors). Applied to TRD edition 4 §4-7 |

### 8-1. Our app now

- Our path **rejects every server request from codex (approval requests included) with JSON-RPC `METHOD_NOT_FOUND`** and never returns a success (`crates/engram-dashboard-agent/src/backend/codex/transport.rs:3054-3087` · each incoming request at `:3629` — the collector's close reading, likely). We launch codex with `approvalPolicy: on-request` · `sandbox: workspace-write` (`backend/codex/mod.rs:132-133`, main cross-check ✓) — writes outside the working folder and network access ask for approval before running.
- ★**When we reject, codex ends that tool as `failed`, not `declined`**★ — it turns the client error into `ReviewDecision::denied("approval request failed")` and closes the command item with `status:"failed"` · `exitCode:null` · `aggregatedOutput:null` (openai/codex `codex-rs/app-server/src/bespoke_event_handling.rs:2026-2032` · `:1468-1483`, main `41f9084` — **main cross-check ✓**). The item has no reason field. `declined` comes only from an explicit Decline/Cancel (we do not send one) · a guardian review · a network policy denial. A file-change approval rejection is also `denied("approval request failed")`, but that item's final state is **unknown** (★edition 5 correction — it closes as `declined` · see 「5판 정정」 (edition 5 correction) ① in §8-3 below · the fact in TRD edition 5 §4-7★).
- The installed build is codex-cli 0.156.1 — whether it behaves the same as upstream main is **unconfirmed** (edition 5: for the command side, the tag `rust-v0.156.1` source read shows the same — likely · not run · fact in TRD edition 5 §4-7). There is no measured fixture containing an approval exchange (the existing measurements were taken with `approvalPolicy:"never"`).
- ★**Nothing shows on the screen now**★ (likely — not measured): the rejection path emits no event to the screen, and the tool-end notification is dropped by the translator (`backend/codex/decoder.rs:262`). The red warning box (`StructuredTextView.tsx:484-490`) is drawn only for a codex `error` notification (`decoder.rs:747-764`) or a control-queue overflow where the rejection reply could not be sent. The only trace left = the text codex writes afterwards.

### 8-2. Peers

| Peer | Display | Color | Reason | Counted as a failure? |
|---|---|---|---|---|
| codex TUI (`codex-rs/tui/src/history_cell/approvals.rs:160-240`) | a separate line 「✗ You did not approve codex to run …」 · a review denial 「Request denied …」 | red ✗ | the wording is the reason | no group summary (likely) |
| Claude Code (issue anthropics/claude-code#10954) | under the tool line `⎿ User rejected update to <path>` | unknown | wording | unknown |
| t3code (`packages/client-runtime/src/work-log/presentation.ts:154-155,422-427`) | 「Declined …」 | the same as the failure mark | none | the row is a failure · excluded from the 「최근 도구 실패」 (recent tool failure) mark |
| Zed (`crates/agent_ui/src/conversation_view/thread_view.rs:8145-8147`) | Rejected = Failed = Canceled | red X | none | failure |
| vibe-kanban (`packages/ui/src/components/ToolStatusDot.tsx:16-19`) | `denied` (has a reason field) | red dot | the field exists | merged into errors |
| paseo (`tool-call-mapper-utils.ts:3-10`) | `rejected`/`denied` → `failed` | failure | none | merged |
| cline CLI (`apps/cli/src/tui/utils/tool-errors.ts:98-106`) | 「… call was skipped before execution.」 | **warning** (not an error) | detail kept | not an error |

(The collector's close reading · gh — likely. Cursor abstain.) The split = **red like a failure** (Zed · vibe-kanban · paseo) vs **shown separately** (t3code's wording · cline's warning tone).

- **Correction (2026-09-27 · TRD edition 5 — main reading · not run · likely):** paseo's failure words are `rejected`/`denied`, not `declined`, and every unknown non-empty status goes to 「실행 중」 (running) (`packages/server/src/server/agent/providers/tool-call-mapper-utils.ts:3-10` · `:22-36` · paseo `bbe3f17`) — codex `declined` will be drawn there as running (reading only). t3code (`5378f87f9`) writes 「Declined to …」 but counts `declined` as a failure (`packages/client-runtime/src/work-log/presentation.ts:154-155` · `:418-427`). Neither of these two peers separates 「실행 안 됨」 (not executed) from 「실패」 (failed).

### 8-3. Options and decision

- A merge into errors — it is what codex sends, so no extra work · something blocked by permission looks the same as a real failure.
- **B show separately (decided)** — orange 「거부됨」 (declined) · a one-line reason inside the row · summary 「거부 N」 (declined N). Since codex sends `failed`, **our path remembers the item it rejected** and emits its end as `Declined` (a plain `declined` status also stays `Declined`) (★edition 5 = our rejection is `Refused` · only a `declined` that is not in memory is `Declined` — the reason differs by origin · the `Declined` reason 「실행되지 않음」 (not executed) (user 2026-09-27) · TRD §4-7 ②-2★). Every rejection in our app is because of our own policy (no approval window), so red would send people hunting for a bug that does not exist.
- Still to confirm: B5 measurement — if 0.156.1 already sends `declined` for our rejection, the memory device is removed · the final state of the file-change path.
  - ★**Edition 5 correction (2026-09-27 · source read `rust-v0.156.1` · not run · likely)**★ — the lines this correction replaces = the two items of 「남은 확인」 (still to confirm) above (「0.156.1 이 우리 거절에 이미 `declined` 를 보내면 기억 장치는 뺀다」 · 「파일 변경 경로의 최종 상태」) and the 「그 item 의 최종 상태는 **모른다**」 in §8-1 — all three are stale. ① When we reject a file change, it closes as `declined`, not `failed` (`core/src/tools/handlers/apply_patch.rs:595, 608-623` · `core/src/tools/events.rs:333-343`) ② So the memory device cannot be dropped either way — it is always built to separate the reasons of our rejection from a vendor `declined` (the outcomes split into `Refused` / `Declined`). Canonical source = TRD edition 5 §4-7 fact · ②-2.
