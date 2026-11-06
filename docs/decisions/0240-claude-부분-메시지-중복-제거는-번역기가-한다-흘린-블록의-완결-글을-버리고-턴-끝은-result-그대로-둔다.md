# ADR-0240: The translator removes claude partial-message duplicates — it discards the completed text of a streamed block and leaves the turn end as the result as it is

- 상태: 확정 (2026-09-27, basis: TRD 5th edition §5 · §9 + TRD 5th-edition review FIX applied (TRD §12) · before implementation · code unchanged)
- 관련: TRD `docs/process/S21-chat-ux/trd.md`(§5-1 ~ §5-5 · §6 · §7 B1 · §9 F4 · §10 0240) · research `docs/research/chat-ux-four-features-2026-09-27.md`(§4 — measured vendor format §4-2 · peers §4-3 · options §4-4) · ADR-0238(the truncated `assistant` line right before an interrupt · the turn-open gate also opens on a streamed `TextDelta`) · ADR-0044(decision 6 — partial-delta streaming follow-up) · ADR-0079(the route that reads and rewinds the resumption record `.jsonl` — where decision 5 and B1's resumption stop condition stand) · ADR-0045(stream-json · cleanup is in the backend) · ADR-0004(backend knowledge isolation) · ADR-0127(turn observation · 30-minute fail-open) · step-log S21 · Amends ADR-0044 (decision 6, partial-delta follow-up)

## 맥락
To make text flow character by character in claude JSON, the spawn arguments must include `--include-partial-messages` (today the StreamJson branch `claude/mod.rs:184-194` does not have it). Then the vendor streams text deltas as `stream_event`, and **separately from that** sends one completed `assistant` message for every non-empty block, before that block's `content_block_stop` (research §4-2 — measured + official docs = certain).

The `TextDelta` branch of the frontend accumulator only appends to the last text item and has no duplicate removal (`structuredAccumulator.ts:130-141`). The streamed deltas and the completed body gather into the same text item, so unless someone discards the completed body, **every claude answer becomes two copies**. The TRD wrote this rule as load-bearing and raised it as an ADR candidate (end of TRD §5-5).

## 결정
Notation — **[고름]** = an internal implementation with no user-visible effect, so the TRD picked it.

1. **Spawn argument** — in the StreamJson branch, `--include-partial-messages` after `--verbose`. The terminal branch is unchanged.
2. **Duplicate removal is the translator's job — it discards the completed text of a streamed block.** `ClaudeStreamDecoder` gets one field `PartialMessage { id: Option<String>, open: Option<u64>, streamed: BTreeSet<u64> }` [고름 — the same lifetime as the line buffer (one incarnation)]. `consume_line` takes an argument `partial: Option<&mut PartialMessage>` — `Some` for live, `None` for resumption.

   | Line | Rule |
   |---|---|
   | `stream_event` + `parent_tool_use_id` is not null | Discarded entirely (including `message_start` — so a sub-agent flow does not overwrite the parent state) |
   | `message_start` | Sets the state fresh as `{ id: message.id, open: None, streamed: {} }` — also for the new message that folded input opens in the middle of a turn |
   | `content_block_start{index}` | `open = Some(index)` |
   | `text_delta` of `content_block_delta` | If there is an `id` and the text is not empty, `TextDelta{text, turn_id: None, message_id: id}` · `index` goes into `streamed` · if there is no `id` (a late delta that came without `message_start`), it is discarded |
   | Other deltas (`thinking_delta` · `input_json_delta` · `signature_delta`) | Discarded — thoughts and tools come from the completed line as today |
   | `content_block_stop{index}` | If `open == Some(index)`, `open = None` |
   | `message_delta` · `message_stop` · an unknown `event.type` | Emits nothing — ★`MessageDone`/`TurnEnd`/`Structured` forbidden★ |
   | Completed `assistant` line | If `message.id == partial.id`, and `open = Some(k)`, and `streamed ∋ k`, **only the `text` blocks** of that line are not emitted. The other blocks and every other case (an un-streamed block · id mismatch · no `partial`) are as today |
   | `result` | Clears the state after today's translation |

   [고름] We take 「(메시지 id, 블록 번호)」 (message id, block number) as the open block — a completed line has no block number, but the contract says 「그 블록의 `content_block_stop` 보다 먼저」 (before that block's content_block_stop), so the block that is open at the moment the completed line arrives is that line's block. An empty block (stops without a completed line) takes no slot. If the vendor changes the order (completed after the stop), `open` is empty and **we emit the full text as today — we overlap rather than lose** (we err to the visible side).
3. **The turn end is the single `result` line as it is.** We do not emit stream side lines (`message_delta` · `message_stop` etc.) as `Structured`.
4. **By default we do not merge** (the same as codex — research §4-4). ★But if a numeric threshold is crossed, we merge inside B1★: if the number of streamed deltas in one turn of the B1 sampling's long answer (≥ 2,000 characters) **exceeds 10 % of `REPLAY_MAX_EVENTS` (4096 — `output_core.rs:1291`) (≈ 410)**, then inside the event list that one `decode()` call returns, adjacent `TextDelta` with the same `message_id` are joined into one (backend only · seq is assigned at emit time, so no holes appear). The decision is **based on real pump chunks** — because adjacent merging shrinks only when several lines come in one pump read (not based on fixture chunks · TRD §12 round-1 low 3). The decision number goes into the B1 return.
5. **The resumption record (transcript) skips `stream_event` lines entirely** [고름] — today's `_ => {}` (`:1003`). Whether or not the record has those lines, the completed `assistant` line emits the full text, so it is one copy.
6. **The wire type and the frontend do not change** — streamed text also uses the same `TextDelta` vocabulary.

## 거부한 대안
- **Replacement** (a new wire variant + replay replacement — vibe-kanban) — 「새 선 변형 + replay 가 교체까지 되감아야 함」 (a new wire variant + replay would have to rewind through the replacement) (research §4-4) · 「주소 붙은 화면 항목이 필요」 (an addressed screen item is needed) (research §4-3).
- **Tail comparison** (paseo) — 「상태가 더 많고 중복 모서리」 (more state and duplicate edges) (research §4-4 · TRD §10). The peer comparison is in research §4-3.
- **Unconditional merging** — we build it only when the numeric threshold (decision 4) is crossed.
- **Move `message_stop` to the turn end** — a turn would end at every tool call (research §4-4 trap · TRD §5-3).
- **Set up fresh translator state for every resumption record** (the way the instruction said) [고름] — decision 5 is simpler and does not rely on the record's shape (TRD §5-4).

## 근거
- **Vendor contract (certain)** — one completed message for every non-empty block, before that block's `content_block_stop` (research §4-2 — claude 2.1.280 measured + official docs).
- **Ring cap** — `REPLAY_MAX_EVENTS = 4096` (`output_core.rs:1291`). The 10 % threshold is the number TRD §9 set.
- **Review** — low 3 of the TRD round-1 light re-check (merging is based on real pump chunks) was reflected in decision 4 (TRD §12).
- ★**Verification status**★ — B1 implemented · measured (2026-09-27). Tests = 1–14 of TRD §5-5 · new fixture `backend/claude/fixtures/partial_stream_p1.jsonl` (it came up with our real spawn arguments · `MAX_THINKING_TOKENS=8000` · no control channel · no `extra_args`). Sampling = claude CLI 2.1.280 · model claude-opus-5-5 (the CLI default — the spawn has no `--model`). Measurements (one sampling session + one resumption of that session):
  - **Resumption** — in the record of the session with the flag on (`~/.claude/projects/<slug>/<sid>.jsonl`) there are **0** `stream_event` lines, and the completed `assistant` text blocks remain with the same character count as the live completed text. A resumption started with the same arguments plus `--resume <sid>` continued with the **same session id** and correctly recalled a word from the earlier conversation when asked.
  - **Idle intervals** — in the five intervals after each `result` before the next input was written (32.0 s · 6.0 s · 6.0 s · 6.0 s · 16.4 s from closing stdin to the end) and in the interval after the resumption turn, the top-level `stream_event` lines are **all 0** (each interval had only one `command_lifecycle completed` line). The vendor premise of 「턴 끝 뒤 진행 없음」 (no progress after the turn end) (impact section) held in this sampling.
  - **Ring pressure** (streamed text deltas / characters per turn) — T1 3 / 17 · T2 9 / 70 · T3 15 / 62 · **T4 (long answer) 519 / 2,428** · T5 2 / 16 · resumption turn 1 / 9. The 519 of T4 exceeded the threshold 410 and built the merging of decision 4. But in chunks simulated with the same 4096 B reads as the real pump, merging gave **519 → 519** (no reduction) — each delta came in its own read (delta interval about 50–80 ms). The simulation cut the pipe chunks of the sampling tool (Node) into 4096 B pieces and is not our Rust pump itself. **Main accepted this ring pressure as a follow-up** (codex is in the same position today) — one long answer uses about 13 % of `REPLAY_MAX_EVENTS`.

## 영향 / 불변식
- **If duplicate removal is missing, every claude answer is two copies** — the frontend has no removal. A regression test measures whether the first streamed delta and the completed body are one copy.
- **No progress after the turn end** — a streamed delta is a `TextDelta`, so it is a progress signal. It comes out only **inside** a turn: `result` clears the state, and a late delta that came without `message_start` is discarded, so there is no way to turn 「턴 중」 back on after the turn end. If stream side lines were emitted as `Structured`, the claude turn classifier would count them all as progress (`:538-545`) and it would become a 30-minute blockage path after the turn end (CLAUDE.md 「대기 입력 상태」 last paragraph).
  - ★This invariant relies on one vendor premise — **the vendor does not emit a top-level (`parent_tool_use_id` null) `stream_event` outside a turn**★. If a `message_start` + text delta comes while idle, the translator takes it as a new message and emits `TextDelta`, and 「턴 중」 (and the turn-open gate of ADR-0238) turns on, which becomes a 30-minute blockage path. The translator cannot prevent it (it has the same shape as the new `message_start` of folded input in the middle of a turn). The B1 sampling confirms it.
- ★**Conditions under which B1 stops and raises to main**★ (TRD §5-4):
  - Resumption — if no completed `assistant` text line remains in the record of the session with the flag on (only partial lines are left), the answer on the resumed screen is empty.
  - Idle intervals — if the top-level `stream_event` lines are not 0 after each `result` before the next input (including one interval of ≥ 30 seconds).
- **Leaving the existing fixtures unmodified is the evidence** — the event sequences of all existing fixtures without partial lines must be byte-for-byte the same. If an existing test has to be fixed, the design is wrong (11 of TRD §5-5).
- **Ring pressure** — with token-level `TextDelta` the ring fills faster, and the earlier history of a long conversation is pushed out of replay sooner (the threshold of decision 4 measures this).
- **Render cost** — `setItems` (`RichSlot.tsx:215`) + the scroll-follow RO run on every delta. codex already takes the same path today (TRD §9).
- ★**Three known limits**★ (the same as TRD §9 F4):
  - (a) **If the completed `assistant` line comes after that block's `content_block_stop`, the text comes out twice** — this is when the vendor contract (TRD §5-2) is violated. It is intended behavior (「잃기보다 겹친다」, we overlap rather than lose): a completed line has no block number, so which block it belongs to can be known only from the open block, and we rejected tail comparison (rejected alternatives). The test `a_text_block_that_was_not_streamed_falls_back_to_the_completed_text` (the completed-after-stop branch) pins this behavior.
  - (b) **Streamed text cannot be taken back** — if the CLI retries a failed stream or falls back to a non-streaming request, the front part that the failed attempt streamed stays on the screen. The vendor behavior was not checked.
  - (c) **`/compact` · auto compact was not sampled with the flag on** — if compact emits a top-level `stream_event`, the summary can flow into the chat. Unchecked — we check it in GUI QA.
- **Document update** — fix `claude/mod.rs:762-767` (「decoder 자신의 상태 = 줄 재조립뿐」) with this field.
- **PTY unchanged** — the terminal arguments and `TerminalSlot` stay as they are.
