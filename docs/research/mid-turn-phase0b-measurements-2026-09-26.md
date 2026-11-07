# Mid-turn input — Phase 0b vendor measurements (M9 · M10 · M13) (2026-09-26)

- **Status:** Measurements complete · not yet reflected in the TRD. These are the results of measuring the three (M9 · M10 · M13) that `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` §3-3 left as 「구현을 막는 측정」 (measurements that block implementation), together with each stop condition. ★This document did not edit the TRD body★ — rewriting is for the next revision (no orphans: a link from TRD §3-3 to this document is needed).
- **Consumers:** TRD §3-3 (measurement table) · §5-5 (codex dispositions — debt · echo · `-32600`) · §9 P0 (stop-condition decisions).
- **Preceding report:** [`mid-turn-phase0-measurements-2026-09-25.md`](mid-turn-phase0-measurements-2026-09-25.md) — the spawn arguments and wire format (§1) were reused as is and are not rewritten here.
- **Measured on:** 2026-09-26 (KST · log timestamps are UTC 2026-09-25T15:55–16:02)
- **Target versions:**
  - codex = `codex-cli 0.156.1` · model `gpt-6-luna` · effort `low`. ★**Newer** than the TRD's reading baseline (0.154.0)★ — the codex results and source line numbers below are all 0.156.1.
  - claude = haiku, via `claude_harness.js`. ★The CLI version string was not left in this measurement record★ (Phase 0 was `2.1.280`).
- **Purpose:** the three implementation-blocking measurements of TRD §3-3 and their stop conditions — M13 red → stop before P3 · M9/M10 red → stop before P2.
- **Harness and raw log location (inside the repository · tracked):** `.claude/handoff/attachments/20260925-midturn-phase0/`
  - Harness: `claude_harness.js` (scenario M13) · `codex_harness.js` (M9 · M10) · summarizer: `m10codex.js`
  - Raw logs: `logs/claude-M13-compact-2026-09-25T15-55-09-512Z.jsonl` · `logs/claude-M13-cost-2026-09-25T15-57-33-238Z.jsonl` · `logs/codex-M9-2026-09-25T16-01-30-909Z.jsonl` · `logs/codex-M10-2026-09-25T16-01-58-313Z.jsonl`
  - The wrapper logs `run-*.log` are caught by the `*.log` gitignore and are not tracked.
- **Confidence legend:** certain (a fact printed directly in the raw logs, observed repeatedly) · likely (observed once or twice + consistent with a code reading) · uncertain (inference · single shot · cannot be decided from the outside)

## 0. Conclusion (first)

1. **All three are green — not one stop condition was hit.**
   - **M13 (claude slash command mid-turn):** `/compact` 3/3 · `/cost` 3/3. When written mid-turn it is `queued`, and then **it is not folded at a tool boundary but held until the running turn ends**, and only then newly `started`. There was never a `refused`·`cancelled`·`discarded`. → no P3 stop.
   - **M9 (codex empty-input `turn/start`):** `input: []` was accepted 4/4. When there was a message that had not got a real answer, the empty turn answered **that one only** (3/3). → no P2 stop.
   - **M10 (codex turn-end 「기록만」 (record-only)):** hit 3 of 4 times. The record-only input was carried, `clientId` and all, in the `thread/items/list` called immediately from the `turn/completed` handler (3/3). → no P2 stop.
2. ★**Two surprises that bear on the design**★
   - **M9 control:** when there was **nothing** that had not got an answer, opening an empty turn made the model **say the previous answer once more** (`READY-7`). It is not double delivery, but the user sees a duplicate answer → the debt that opens an empty turn must be set up **only when that item really got no answer**.
   - **M10 late failure:** `-32600 "no active turn to steer"` came **0.1 ms before** `turn/completed` (even before `thread/status/changed` idle). ★Do not assume `-32600` always comes after `turn/completed`★ — in Phase 0 M6 it came after.
3. **M10 — the echo of a record-only input comes before `turn/completed`.** `item/started userMessage`(clientId) is 4.5–4.8 ms ahead of `turn/completed` and carries the id of the turn that is ending (3/3). The basis that Phase 0 M6 failed to hit and left unconfirmed (「기록된 입력은 완료 알림 전에 에코된다」, recorded input is echoed before the completion notification) now stands. The probe also carries the same item — the decision stands on either the echo or the probe.

## 1. M13 — claude mid-turn `/compact` · `/cost`

**Method:** `claude_harness.js` scenario M13, sixth argument `slashCmd` (default `/compact`). Make turn A call Bash `sleep 12`, and about 2 s after that `tool_use` write the slash command as a user line (2 times mid-turn) · 1 time separately while idle. One run per command.

**Evidence — `/compact` mid-turn** (`logs/claude-M13-compact-…jsonl`):

```
tool_use(Bash sleep 12) → +~2 s 우리가 /compact 씀 → queued (+1 ms)
tool_result → A result/success → A completed
→ /compact started (A completed 1 ms 뒤)
→ system/status compacting → compact_result success (14–18 s)
→ system/init → compact_boundary
→ user "This session is being continued..."
→ isReplay "<local-command-stdout>Compacted</local-command-stdout>"  (우리가 안 보낸 uuid)
→ isReplay (우리 uuid) "<command-name>/compact</command-name>"
→ result success, result ""  → completed  (result 가 completed 0.6 ms 앞)
```

**Evidence — `/cost` mid-turn** (`logs/claude-M13-cost-…jsonl`): `queued` 13230.4 → held until A `completed` 27644.8 → `started` 27645.7 → `completed` 30014.9.

**Result:**

| Question | Result | Confidence |
|---|---|---|
| Is a mid-turn slash command accepted | **Yes** — `/compact` 2/2 · `/cost` 2/2 were `queued` | certain |
| Is it folded at a tool boundary | **No** — newly `started` within about 1 ms after the running turn `completed` | certain |
| While idle | `queued` → `started` within 1 ms · the tail is the same as mid-turn (`/cost` `completed` at ~1.3 s) | certain |
| `refused`·`cancelled`·`discarded` | **never** (6/6) | certain |
| End order | `result` (empty string) → `completed` 0.6 ms — same as the order M1 saw for a turn-opening command | certain |

- ★**`/compact` emits one more `isReplay` line with a uuid we did not send**★ — `<local-command-stdout>Compacted</local-command-stdout>`. If the echo match is done only against 「내가 보낸 uuid 」 (the uuid I sent), this line falls into an unknown line.
- ★**Surprise — `/compact` while idle echoed the 「Compacted」 stdout line of the preceding compact again**★ (uuid `6905adf2`). Do not count a duplicate echo as a new event.
- Cost: `/compact` run `total_cost_usd` ≈ $0.128.

**Stop condition (M13 red → stop before P3):** not hit.

## 2. M9 — codex empty-input `turn/start`

**Method:** `codex_harness.js` scenario M9 (control) · variant A runs on after each hit of the M10 scenario (the `m9summary` line of the log). What was sent = `turn/start {threadId, input: []}`, no `clientUserMessageId`.

**Result:**

| Question | Result | Confidence |
|---|---|---|
| Is an empty `input` accepted | **Yes** 4/4 — response ~50 ms `{"turn":{"id":…,"items":[],"itemsView":"notLoaded","status":"inProgress"}}` → `turn/started` → sampling → `turn/completed` completed | certain |
| Is a user item or echo created | **None** — neither a `userMessage` item nor an echo | certain |
| Variant A — when there is a message that has not got a real answer | the empty turn's only `agentMessage` answered **exactly that message** (PINEAPPLE2/3/4) 3/3 | certain |
| Variant B (control) — when there is nothing unanswered | the model **repeated the previous answer** (`READY-7`) — the user sees a duplicate answer | likely (once) |

- **Source (0.156.1):** an empty input takes the idle branch of `start_or_steer` — `core/src/session/turn_input.rs:290-365`.
- ★**Design implication**★: an empty turn does not create double delivery, but a **wasted debt** creates a visible duplicate answer. The debt is set up only when 「그 항목이 정말 답을 못 받았다」 (that item really got no answer) has been confirmed (TRD §5-5).
- Logs: `logs/codex-M9-2026-09-25T16-01-30-909Z.jsonl` (B) · `logs/codex-M10-2026-09-25T16-01-58-313Z.jsonl` (A, the `m9summary` line).

**Stop condition (M9 red → stop before P2):** not hit.

## 3. M10 — codex turn-end 「기록만」 (record-only) branch

**Method:** a turn without tools (「reply with exactly: ACK-k」). Inside the **first** `thread/tokenUsage/updated` handler of that turn, a busy wait of d ms and then a steer. In the `turn/completed` handler, immediately `thread/items/list {limit:40, desc}` + a `limit:2` desc cursor walk (until the page containing the previous turn's id comes out). d is adaptive (starts at `M10_D0`=1.0 ms). Hit decision = the steer was accepted with the turn id · the answer is only ACK-k · no second sampling · the next empty turn answered PINEAPPLEk.

**Attempts:**

| d (ms) | steer time (relative to tokenUsage) | Result |
|---|---|---|
| 1.0 | +1.4 ms | late → `-32600 "no active turn to steer"` |
| 0.53 | +0.6 ms | **hit** |
| 0.41 | +0.5 ms | **hit** |
| 0.36 | +0.4 ms | **hit** |

- The early edge was **not seen** (stopped at 3 hits). The window is roughly within (0, 1.4) ms after tokenUsage is received — ★inference and uncertain★. In Phase 0 M6 T4 a steer sent at ~+0 ms went in as a follow-up sampling of the same turn.

**Result (per TRD question):**

| # | Question | Result | Confidence |
|---|---|---|---|
| ① | Is it carried, `clientId` and all, at probe time | **Yes** — the probe in the `turn/completed` handler came back 0.9–1.4 ms later and the `userMessage`+`clientId` that was steered was at index 0 (3/3) | certain |
| ② | Does an echo come · when | **Yes, before `turn/completed`** — `item/started userMessage`(clientId) 4.5–4.8 ms ahead of `turn/completed`, `item/completed` ~2.3 ms after · the steer success response ~2.4 ms after · it carries the id of the turn that is ending | certain |
| ③ | Does an echo come after the probe said 「없음」 (none) | not applicable — the probe never produced 「없음」 | — |
| ④ | Turn id · position | the ended turn's **own** id (same as the steer response) · desc index 0 (after the last `agentMessage`) · ascending within the turn = U, agentMessage, W | certain |
| ⑤ | `limit:2` page walk | page 0 = [W, agentMessage](찾음) (found) · page 1 = [U, the previous turn's agentMessage] → passed the turn in 2 pages · walk 1.9–3.5 ms | certain |

- **Record-only lengthens the turn end:** tokenUsage → `turn/completed` is 8.1–8.5 ms (3.3–3.6 ms without record-only).
- ★**It cannot be told from the `turn/completed` payload**★ — `itemsView "summary"` carries only `agentMessage`. A record-only item is visible only through the echo or the probe.
- ★**Surprise — the `-32600` of the late failure came 0.1 ms before `turn/completed`**★ (even before `thread/status/changed` idle). At the time `-32600` is received, the `turn/completed` of that turn may not have arrived yet.
- **Source (0.156.1):**
  - Last pending-input check — `core/src/session/turn.rs:555-567`
  - Where the window closes — `active_turn.task.take()` in `on_task_finished`, `core/src/tasks/mod.rs:640-650`
  - Recorded before `TurnComplete` — `take_pending_input_for_turn_state` `:655` · `run_hooks_and_record_inputs` `:673`
  - Steer rejected after the task becomes `None` — `turn_input.rs:634-638`
- Log: `logs/codex-M10-2026-09-25T16-01-58-313Z.jsonl`.

**Stop condition (M10 red → stop before P2):** not hit.

## 4. Reproduction

- **Scenarios:**
  - claude: `claude_harness.js` scenario `M13` + sixth argument `slashCmd` (default `/compact`) → log name `claude-M13-<cmd>-<stamp>.jsonl`.
  - codex: `codex_harness.js` scenario `M9` (control) · `M10` (hunt + the M9 variant A after every hit). Helper functions `spin`·`pick`·`emptyTurnStart`·`walkItems`. Environment variables `M10_MAX`(40) · `M10_HITS`(3) · `M10_D0`(1.0 ms).
  - Summary: `node m10codex.cjs <log> [verbose]`.
- ★**ESM trap**★ — the repository `package.json` has `"type":"module"`, so running `node <harness>.js` inside the repository dies with `require is not defined`. **Copy it outside the repository as `.cjs`** and run it there.
- **Running:** launch outside the shell tree with `scripts/run-detached.ps1`, and decide completion by the `__EXIT` marker (same as the preceding report).

## 5. Cleanup · cost

- **Processes:** no leftover processes on the codex side. None on the claude side either — the child of the `/compact` run (36388) ended on its own with 0 once stdin was closed, and for the `/cost` run the main session confirmed with `tasklist` that child PID 29624 had disappeared from the list after `__EXIT=0` was printed in the wrapper log (2026-09-26).
- **Traces left (new threads that may be thrown away):** 2 codex threads = `%USERPROFILE%\.codex\sessions\2026\09\26\`. Existing user sessions were not touched.
- **Cost:** codex 2 runs (M9 ~9 s · M10 ~30 s) · 9 turns · 4% of the 5-hour window. claude `/compact` run ≈ $0.128 (no record of the amount for the `/cost` run).

## 6. Limits

- **One session · one model · one version.** codex is one thread each on `0.156.1` · `gpt-6-luna`, claude is one haiku session each. It was not re-measured on the TRD reading baseline `0.154.0`.
- **The early edge of the M10 window was not seen** — it stopped at 3 hits, so for (0, 1.4) ms only the upper side is measured and the lower side is inference.
- **The settlement-deadline branch was not taken** — the probe always found the item in time, so the case of the deadline passing did not arise. 「탐침 없음 → 나중 에코」 (no probe result → later echo) (③) is likewise unobserved for the same reason.
- **The user's codex hooks settings were not inspected.** The pending-input hook runs on the recording path — a slow `UserPromptSubmit`·`Stop` hook only **widens** the window and does not change what happens inside the window (source reading · likely).
- The M9 control (duplicate answer) is a single observation.

## 7. Summary table

| M# | Result | Green/Red | Affected TRD sections |
|---|---|---|---|
| M13 | `/compact`·`/cost` accepted 3/3 each mid-turn · not folded at a tool boundary, held until turn end · rejection·cancel·discard 0 · `/compact` has one more line of another's uuid `isReplay` (while idle, a preceding compact line is re-echoed) | green | §3-3 · §9 P0 (no P3 stop) |
| M9 | `input: []` accepted 4/4 · no echo·item · when there is something unanswered it answers only that (3/3) · **when there is none, it repeats the previous answer** | green + design implication | §3-3 · §5-5 (debt only when really unanswered) · §9 P0 |
| M10 | hit 3/4 (window estimated (0, 1.4) ms) · `clientId` and all at index 0 in the probe · **echo 4.5–4.8 ms before `turn/completed`** · ended turn's id · `limit:2` 2 pages · **`-32600` may come before `turn/completed`** | green + a new fact | §3-3 · §5-5 (echo basis stands · remove the `-32600` order assumption) · §9 P0 (no P2 stop) |

> **Correction (2026-09-26, M16):** the vendor's last pending-input check is the task loop `core/src/tasks/regular.rs:120`, which runs `run_turn` again inside the same turn — the pointer to `core/src/session/turn.rs:555-567` in §3 above is inaccurate. Basis · measurement = [M16 report](mid-turn-m16-measurements-2026-09-26.md).
