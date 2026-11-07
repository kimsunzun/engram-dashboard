# codex mid-turn input — when peer implementations push it in and what they settle (2026-09-26)

- **Status:** medium (3 collection tracks · 3 main grounding spot checks · 5 FIX items from the cross-family adversarial review applied)
- **Method:** collectors read each project's source directly at the commit in the table below · main checked 3 quotes against the original source (marked 「메인 대조」 in the table) · no tests were run
- **Date:** 2026-09-26
- **Confidence legend:** certain (two or more independent primary sources, or direct source reading + main cross-check) · likely (one primary source or partial support) · uncertain (inference · secondary source) · unknown (not read, or not decidable from source)
- **Prior survey:** `docs/research/mid-turn-input-display-2026-09-25.md` (display shape survey). This document digs into just its **delivery timing · settlement · failure handling**, from source.

## 0. Conclusion (first)

1. **There are three shapes.** ① Immediate steer — Codex CLI TUI (Enter) · paseo default. (Zed codex-acp is 「즉시 제출(core 처리 미확인 · core 0.137.0 고정)」, so we do not classify it as steer.) ② Hold until a tool-completed event — t3code queue mode (it goes out only after the approval/question wait and the temporary send gates are cleared). ③ Hold until the end of the turn — vibe-kanban · paseo queue mode (but if a permission request is waiting, queue turns into interrupt) · Codex TUI (Tab). — certain
2. **No place that emits an empty follow-up turn for input that codex only recorded and did not answer exists among the ones we checked (vendor TUI · t3code · vibe-kanban) — paseo and Zed are unchecked.** The vendor TUI also accepts that loss. — uncertain (paseo and Zed not investigated)
3. **No place measures the boundary or sets up a timing race.** The vendor relies on core folding it in at the head of the loop, and t3code only looks at whether a new `tool.completed` is visible. — certain
4. **No feature to cancel an item already handed over exists among the ones we checked (vendor TUI · t3code · vibe-kanban) — paseo and Zed are unchecked.** The ✕ (undo) exists only while the client still holds the item. — uncertain (paseo cancellation · Zed core routing not investigated)
5. **Rejection handling has two branches** — fall back to a new turn (vendor TUI · paseo) or hold at the front and wait for user action (t3code). — certain
6. **Our decision:** user decision 2026-09-26 — codex is cut down to the t3code shape (push in at the tool-completed event). The old design (boundary decision · settlement · empty follow-up turn · answer span) is withdrawn.

## 1. Comparison table

| Item | Codex CLI TUI | t3code | paseo | vibe-kanban | Zed codex-acp |
|---|---|---|---|---|---|
| Commit read | `openai/codex` `rust-v0.156.1` (`b412ff32`) | `5378f87f9` | `bbe3f17` | `78580443` | `296069e` |
| codex connection | app-server | app-server | app-server | CLI child process | codex-core in-process |
| Sending mid-turn | Enter = immediate `turn/steer` · Tab = local wait | Setting queue (default)/steer — `turn/start` either way | Setting interrupt/steer/queue, default steer | 1 waiting item per session | Immediate `Op::UserInput` submit (core handling unconfirmed · core pinned at 0.137.0) |
| When it is handed over | steer = immediately · Tab = when idle | Becomes due when the latest `tool.completed` id changes or the turn ends — it actually goes out only after the approval/question wait and the temporary send gates are cleared | steer = immediately · queue = after the agent stops (interrupt if a permission request is waiting) | As a follow-up after the run ends | Immediately (no busy check) |
| Settlement criterion | `userMessage` echo matching `clientId` (fallback = text + image count) | The id appears in the server read-model | Not investigated | Not applicable | Unknown |
| Input recorded without an answer | No follow-up turn | No follow-up turn | Not found | Not applicable | Unknown |
| Rejection | Clear the turn id and `turn/start` · review/compact is resent first on the next idle | Held at the front, no automatic retry | interrupt + new turn (if there is no turn in progress, `inactive` · if the ownership check fails it appears to throw — unread) · a queue failure waits again at the front | Discarded if Failed/Killed | Not applicable |
| Cancel | Esc = interrupt, then merge the waiting steers into one turn and resubmit | Delete → input box · Stop = the whole queue to the input box | Not investigated | DELETE → editor | `Op::Interrupt` once (whole thread) |
| Size (approx.) | 1.3–1.8k lines (uncertain) | 450 lines (likely) | 450 lines (likely) | 300 lines | 150 lines |

## 2. Notes per project

### 2-1. Codex CLI TUI (`openai/codex`@`b412ff32`, tag `rust-v0.156.1`)

- The router picks `turn/steer` if the active turn id is cached, otherwise `turn/start` (`codex@b412ff32 tui/src/app/thread_routing.rs:736-760`). Tab is a local wait until idle (`tui/src/bottom_pane/chat_composer.rs:3510-3518`). — certain
- **The TUI measures nothing.** Core drains the pending input only at the head of each loop iteration, after the sampling request and its tools have finished (`core/src/session/turn.rs:423-445, 552-567`). — certain
- It keeps a `PendingSteer` list and removes the earlier item not on the steer response but when the **committed `userMessage` echo** matches by `clientId` (fallback = compare text + image count) (`tui/src/chatwidget.rs:1374-1400`). — certain
- Input left over when a task ends is only recorded, without sampling (`core/src/tasks/mod.rs:643-683`). The TUI does not start a follow-up turn for that input. — likely
- On receiving `no active turn to steer`, it clears the cached turn id and sends with `turn/start` (`thread_routing.rs:783-792` — main cross-check). For review/compact, `ActiveTurnNotSteerable` is put in the rejection queue and resent first on the next idle (`tui/src/chatwidget/input_restore.rs:184-210, 288-305` · idle auto-send = `tui/src/chatwidget/input_flow.rs:234-265`). — certain
- With waiting steers, Esc = after interrupt, since the server dropped them, they are **merged into one and resubmitted as a new turn** (`input_restore.rs:311-378`). — certain · An item that has already been steered cannot be cancelled in normal mode. — likely

### 2-2. t3code (`t3code@5378f87f9`)

- The setting `followUpBehavior` = queue (default)/steer. For codex it does **not use** `turn/steer` and always sends `turn/start` even mid-turn — code comment "Codex accepts follow-ups while the current turn is still running" (`CodexSessionRuntime.ts:2470-2487` — main cross-check). — certain
- Queue mode is held by the client. For each item it notes the latest `tool.completed` activity id at the time of entry, and it becomes due when the latest id differs or the turn is no longer running (`queuedMessageStore.ts:196-205` — main cross-check). It sends one per boundary (`ChatView.tsx:8603-8644`). **Becoming due is only one of the send conditions** — if the approval/question wait or a temporary send gate (environment offline · settings not loaded · checkpoint rewind · message loading · no machine selected) remains, even a due item does not go out (`ChatView.tsx:8614-8634`). — certain
- An item is taken out by `take()` just before sending, and the optimistic row is cleared when its id appears in the server read-model — there is no cross-check against the codex echo. — likely
- There is no follow-up handling for input recorded without an answer. — likely
- Failure = held at the front until user action, no automatic retry. Delete = returned to the input box, Stop = the whole queue is poured into the input box before the interrupt. — certain

### 2-3. paseo (`paseo@bbe3f17`)

- `sendBehavior` interrupt/steer/queue, default steer (`app/src/hooks/use-settings/storage.ts:29,121,194`). — certain
- steer = an immediate `turn/steer` carrying `clientUserMessageId` (`codex-app-server-agent.ts:4174-4218`). A definite rejection (`no active turn to steer` · not steerable · turn id mismatch) falls back to interrupt + a new turn (`agent-manager.ts:2435-2474`). But that substitute path appears to return `inactive` if there is no turn in progress and the expected turn id equals the active turn, and to throw if it hits the turn ownership check (`agent-manager.ts:2462-2477` — the body of the check function is unread, uncertain). — certain
- queue mode = hold until the agent stops and send only the head one (`host-runtime.ts:2164-2210`). A failed waiting send waits again at the front. — certain
- **Exception:** if a permission request is waiting, the queue behavior turns into interrupt — because the turn is stopped until the permission response, and making it wait would leave the message isolated (`app/src/composer/index.tsx:1479-1493` · `composer/input/state.ts:8-12`). — certain
- We did not find handling of input recorded without an answer. — uncertain

### 2-4. vibe-kanban (`vibe-kanban@78580443`)

- It does not use `turn/steer` or `turn/interrupt` at all. — certain
- 1 waiting item per session (replaced if put in again), sent as a follow-up after the run ends and discarded if the run is Failed/Killed (`container.rs:619-666`, `queued_message.rs:33-71`). Cancel = DELETE → to the editor. — certain

### 2-5. Zed codex-acp (`codex-acp@296069e`)

- It links codex-core in-process rather than using the app-server. A second `session/prompt` is submitted immediately as `Op::UserInput` without a busy check (`thread.rs:3173-3281` · submit point `3263-3279`). — certain · That line shows only the **submit**, and we do not know how core routes it. So we classify it as 「즉시 제출(core 처리 미확인)」 rather than 「즉시 steer」.
- It pins codex core at the tag `rust-v0.137.0` (`Cargo.toml:24-35`) — its version differs from the TUI grounds (`rust-v0.156.1`), so we do not carry the vendor core's behavior over to it as is. — certain
- Cancel = `Op::Interrupt` once on the whole thread. — certain

## 3. Limits

- The codex VS Code extension and app are closed source, so we did not read them.
- Paths not traced: the paseo store reconcile · the path by which t3code takes `item/completed` in as `tool.completed` · the codex abort path.
- No tests were run — every decision is from reading the source.
- Main cross-checks are only 3 (`thread_routing.rs:783-792` · `CodexSessionRuntime.ts:2470-2487` · `queuedMessageStore.ts:196-205`). The adversarial review's re-check is in section 4 below.
- We did not investigate paseo's handling and cancellation of input recorded without an answer, or Zed's core routing — conclusions 2 and 4 are narrowed to that scope.

## 4. Adversarial review result

The cross-family adversarial review re-checked 10 quotes against the original sources — 8 supported · 2 partially supported. 5 FIX items applied.

1. paseo queue mode turns into interrupt when a permission request is waiting — we attached an exception to 「멈출 때까지 붙든다」 (conclusion 1 · comparison table · 2-3).
2. t3code's new `tool.completed` (or the end of the turn) only makes a waiting item **due**, and the approval/question wait and the temporary send gates still block sending — we narrowed the timing statement (conclusion 1 · comparison table · 2-2).
3. The substitute path of the paseo steer rejection has an `inactive` return and a turn ownership check branch — we added a caveat (comparison table · 2-3 · the check function body unread).
4. We narrowed conclusions 2 and 4 to the checked scope (vendor TUI · t3code · vibe-kanban) and lowered the confidence to uncertain — paseo and Zed are unchecked.
5. For Zed codex-acp only the submit was confirmed and it pins core at `rust-v0.137.0` — we changed the 「즉시 steer」 classification to 「즉시 제출(core 처리 미확인 · core 0.137.0 고정)」 (conclusion 1 · comparison table · 2-5).

In addition, we added the idle auto-send quote (`input_flow.rs:234-265`) to the grounds for the vendor TUI's resend of a rejected steer.
