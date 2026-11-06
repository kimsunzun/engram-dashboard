# Mid-turn input — M15 daemon measurement (reaction latency of the codex pathway hand-over) (2026-09-26)

- **Status:** measurement complete. The result of measuring the **M15** row of §3-3 of `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` (L161 — tool end `max(①) + max(리더 밀림) + max(②) < 6 ms`), taken as the task. ★This document did not edit the TRD body★ — adding a link from the M15 row to this document is for the next revision.
- ★**Follow-up (2026-09-26 simplification — ADR-0235): the hand-over policy this measurement turned on has been removed.**★ `HAND_OVER_POLICY = AtEarliestBoundary` (including holding during the answer interval) is gone from the code, and codex now hands over the held text at two times — when a tool item of the running turn ends, and, at turn end, the moment the id of the new turn opened by the first of the remaining texts becomes known (the rest is steered into that turn — ADR-0235 decision 5). The instrumentation target also changed — `engram::codex_handover` is gone and `engram::codex_steer` (one line per event · a `phase` field) replaces it. So the `RUST_LOG=…engram::codex_handover=debug` in §1 and §4 turns nothing on in the current code. The values are left as the record of product commit `4ba00b5`.
- **Consumers:** TRD §3-3 M15 row · implementation plan `.claude/handoff/attachments/20260926-midturn-impl/plan.md` §5. The answer-end value is compared with the M16 window.
- **Preceding report:** [`mid-turn-m16-measurements-2026-09-26.md`](mid-turn-m16-measurements-2026-09-26.md) (answer-end window — narrowest window 9.4 ms). Description of the measurement tools = `.claude/handoff/attachments/20260926-midturn-impl/m15/README.md`. Not repeated here.
- **Date measured:** 2026-09-26 (KST 16:18–16:31 · log times are UTC 07:18–07:31)
- **Target:** product commit `4ba00b5` (P2g — after `lag_us` was changed to a definition that subtracts the `read()` wait time) · **daemon = release profile** (`cargo build --release -p engram-dashboard-daemon`) · codex = `codex-cli 0.156.1` (confirmed by the `cli_version` of the rollout in all four runs) · model `gpt-6-sol` · effort `high` (the codex default setting on this PC — ★differs from M16's `gpt-6-luna`/`low`★). Product settings as they are: approval `on-request` · sandbox `workspace-write`.
- **Harness · raw log location:** under `.claude/handoff/attachments/20260926-midturn-impl/m15/` (inside the repository · tracked). The OS user name was replaced with `<user>`.
  - Extracted instrumentation lines (`--extract`): `runs/m15-{T,H,F,F2}-daemon-*.trace.log`
  - Per-run parser output: `runs/m15-{T,H,F,F2}-parse.{txt,json}` · combined: `runs/m15-combined.json`
  - Driver event record (wall-clock ms — send time · ToolCall · TurnEnd · QueuedInput): `runs/m15-{T,H,F,F2}-driver-events.jsonl`
  - Harness: `harness/m15b.mjs` (driver) · `m15combine.mjs` (combine — reuses the parser's `analyze`/`stats`/`LIMIT_US` as they are) · `m15floodcheck.mjs` (flood window cross-check) · `m15summ.mjs` · `flood.js` (line printer)
  - ★The OS user name was replaced with `<user>`★. The instrumentation lines contain no paths to begin with.
  - The full original daemon logs were not put in the repository because the header line holds an absolute path — the extracts above are all of the instrumentation lines.
- **Confidence legend:** certain (a value printed directly in the log · repeated observation) · likely (repeated observation + source reading fit together) · uncertain (inference · single shot)

## 0. Conclusion (first)

1. **Green (PASS) — tool-end reaction latency sum = 2275 µs < 6000 µs.** `max(①) 783 + max(리더 밀림) 1405 + max(②) 87`. The margin is 3.7 ms (38 % of the limit). This is the value with the four logs combined, and of the per-log verdicts, the two that can be decided are PASS (F2 = 1486 µs · F = 131 µs). The parser warning (fewer than 20 samples) is fully resolved in the combination. **Certain** (log values as they are · parser formula as it is).
2. **The dominant term = reader lag 1405 µs — one tool end in a turn where 3 parallel tools poured out output at the same time** (run T `seq=74`). The tool-end lag of sequential tools is p95 631 µs, and its maximum values are the end lines after a 6000-line flood. Under the P2g definition (an upper bound on the time the reader was busy outside `read()`), it is **time actually spent busy**, not an instrumentation defect. However, since it is an upper bound, the real wait is at or below this (§6). **Likely.**
3. **The maximum of ①, 783 µs, is mostly line decoding** — `handle_us` 769 µs (from the moment the reader picks up the line → the moment it raises the signal). It is the time to decode one `item/completed` line (about 340 KB) carrying a 6000-line `aggregatedOutput`, and the share left until the writer wakes is tens of µs. For a tool end with small output, ① p50 is 100 µs. **Certain** (both fields are printed on the same line).
4. **② is small and even** — turn id hop 23 cases 49–87 µs · steers handed over immediately during an output flood 28 cases 29–72 µs (combined 79 cases p95 70 µs). **Certain.**
5. **Answer-end reaction sum = 221 µs** (① 59 + lag 75 + ② 87). It is more than 40 times smaller than M16's narrowest answer-end window of 9.4 ms — daemon-side latency is no obstacle to holding the answer interval. It is a comparison value, not a verdict. **Certain** (the value) · the conclusion that it always fits in the answer interval window is **likely** (M16 was a vendor-only harness).
6. **All handed-over input was accepted in the same turn** — 23/23 turn id hops got an answer as `OK<n> TWO`, and 28/28 steers during the flood were listed by number in the answer at the end of that turn (7/7 turns such as `1.1, 1.2, 1.3, 1.4`). Dropped input 0 (`Dropped` 0 · errors 0). **Certain.**

## 1. Method

**Daemon:** built the release daemon in an isolated worktree (`4ba00b5`, detached), and started it directly each run with a **new isolated data folder** (`scripts/launch-detached.ps1 -EnvVars 'RUST_LOG=warn,engram::codex_handover=debug','ENGRAM_DATA_DIR=<격리 폴더>'`). One run = one daemon = one codex agent, so each log has one `seq` space (the parser's duplicate `seq` warnings 0). ★The app (client shell) was not started★ — the driver below connects directly to the daemon.

**Driver (`m15b.mjs`):** sends the same WS frames as the front-end `WsTransport`/`ProtocolClient` directly to the daemon — with the token in `daemon.json`, `Auth` → `Hello` → `CreateProfile{backend:'codex', output_format:'StreamJson'}` → `SpawnProfile` → `Subscribe` → input via `WriteStdin` (the same raw text as the chat box `RichSlot`). With no Origin header it enters the 「네이티브/하네스 허용」 ("native/harness allowed") branch of net `ws.rs`. The condition for considering a turn finished = `TurnEnd` received + outstanding input (a `Queued` not followed by `Delivered`/`Dropped`) 0 + 6 seconds quiet.

**Four runs (all on the same PC · same codex settings):**

| Run | Purpose | Turns | Operation per turn |
|---|---|---|---|
| T | tool end · parallel tools | 7 | turns 1–6: tell it to run the four `node flood.js 1000/3000/5000/2000` separately, in order · turn 7: tell it to run the three `node flood.js 3000 20 100` **in parallel** |
| H | turn id hop (`turn/start` response → steer the held item) | 23 | send text A (`Reply with just the word OK<n>.`) and text B (`Also append the word TWO …`) **back to back** — B arrives before the turn id, is held, and is handed over on the turn id signal |
| F2 | steer during an output flood | 7 | tell it to run `node flood.js 6000 40 100` (6000 lines · pours out over about 2.4 seconds) **as is**, and type one text each at +700/+1100/+1500/+1900 ms after its `ToolCall` |
| F | (auxiliary) steer during a running tool — not a flood | 7 | same as F2, but the prompt was 「출력을 보이지 말라」 (do not show the output) → the model **wrapped the command and swallowed the output** as `$x = node flood.js … 2>&1; $x \| Select-Object -Last 1` (7/7). Used only as steer samples typed while the tool was running |

**Flood window cross-check (`m15floodcheck.mjs`, F2):** compared the wall-clock the driver sent with the `recv_ms` of the daemon log's `tool_end` lines against the same PC clock. The first output delta time was obtained only in turn 1 through the decoder debug line — `engram_dashboard_agent::backend::codex::decoder=debug` was turned on additionally only for the F2 daemon, and that line is printed **only the first time for each name**, so it is 8 lines in the whole log (measurement disturbance negligible — likely).

**Verdict formula:** exactly the parser's (`parse-handover-trace.mjs`, the commit `4ba00b5` version). The combination summed the raw values of the four logs and recounted with the same `stats()`·`LIMIT_US`. The per-log parser output and the pairing check (unpaired lines 0 · format errors 0) were run separately for each run.

## 2. Results

### 2-1. Required sample counts

| Requirement | Needed | Obtained | Basis |
|---|---|---|---|
| Tool end (including output of a thousand lines or more) | ≥ 20 | **39** (T 25 · F2 7 · F 7) — of which those where output of a thousand lines or more actually flowed = **32** (T 24 sequential + 1 parallel · F2 7) | `phase="wake" signal="tool_end"` 39 lines · merged wake-ups 0 |
| Explicit parallel tool attempt | ≥ 1 | **1 — it actually ran in parallel** | T turn 7: three `ToolCall` started within 75 ms · that turn's `tool_end` signal was **1** (it is raised only when the running set empties) · ended 1.36 s after the first start (if sequential it would exceed 3 seconds, with at least 0.6 s of output each + startup) |
| Turn id hop steer | ≥ 20 | **23** (H) | `phase="steer" hop="signal" signal="turn_id"` 23 lines · B 23/23 `Queued` → `Delivered` · answer 23/23 `OK<n> TWO` |
| Steer during an output flood | ≥ 20 | **28** (F2) + auxiliary 28 (F) | F2: all 28 sent at `ToolCall` +700–1901 ms · that turn's `tool_end` was `ToolCall` +3043–3119 ms · every steer was ≥ 1143 ms ahead of the tool end · turn 1's first output delta = `ToolCall` +568 ms |

### 2-2. Per-term values (four logs combined · µs · release)

| Term | Signal | n | min | p50 | p95 | max |
|---|---|---|---|---|---|---|
| reader lag `lag_us` | tool_end | 39 | 14 | 113 | 631 | **1405** |
| ① signal → wake `wake_us` | tool_end | 39 | 33 | 100 | 598 | **783** |
| (inside ①) decoding `handle_us` | tool_end | 39 | 24 | 85 | 578 | 769 |
| reader lag | answer_end | 81 | 13 | 35 | 48 | 75 |
| ① | answer_end | 81 | 20 | 27 | 37 | 59 |
| reader lag | token_usage | 106 | 84 | 110 | 1724 | 2722 |
| ① | token_usage | 106 | 18 | 27 | 37 | 62 |
| reader lag | turn_id | 44 | 30 | 49 | 78 | 118 |
| ① | turn_id | 44 | 25 | 36 | 64 | 160 |
| ② turn id hop `steer_us` | — | 23 | 49 | 57 | 70 | 87 |
| ② handed over immediately (announce) | — | 56 | 29 | 50 | 61 | 72 |
| ② combined (for the verdict) | — | 79 | 29 | 52 | 70 | **87** |
| whole turn id hop `hop_us` | — | 23 | 77 | 96 | 129 | 221 |

- Vendor emit → reader pick-up (`emitted_ms`→`recv_ms`, cross-process wall-clock · ms · for reference): tool end 0–4 ms · answer end 0–1 ms. Not used in the verdict.
- ★The large lag of `token_usage` (1.5–2.7 ms) is the end of sampling **right after** the high-output tool end of F2★ (all 6 cases over 1 ms are F2, and they were printed 5–7 ms after that turn's tool-end signal). In between, the reader took in and decoded a completion line of about 340 KB with 4 KiB reads, so it appears that time spent busy without a short read piled up (likely). `token_usage` is not an M15 verdict term.

### 2-3. Per-run parser verdicts

| Run | Instrumentation lines | Tool-end verdict (log alone) | Answer-end sum |
|---|---|---|---|
| T | 156 (signal 78 / wake 78 / steer 0) | INCOMPLETE — ② samples 0 (no steer was typed) | INCOMPLETE (same reason) |
| H | 253 (115 / 115 / 23) | INCOMPLETE — tool end 0 | 184 µs |
| F | 98 (35 / 35 / 28) | PASS 131 µs | 195 µs |
| F2 | 112 (42 / 42 / 28) | PASS 1486 µs (dominant = ①) | 174 µs |
| **Combined** | 619 | **PASS 2275 µs (dominant = reader lag)** | **221 µs** |

In all four logs, format errors 0 · unpaired signal·wake·signal-hop steer 0 · duplicate `seq` 0.

## 3. Verdict (TRD M15 row)

- **Tool end: green.** `783 + 1405 + 87 = 2275 µs < 6000 µs`. The maxima of the three terms came from **different events** (① = F2 `seq=32` · lag = T `seq=74` · ② = H `seq=65`), so it is a conservative sum, set larger than the real latency of any single event. **Certain.**
- **Nature of the dominant term:** a tool-end line that arrived while the reader was processing, without pause, the output poured out by three parallel tools at once. Only busy time was counted, per the P2g definition, so it is real load. There is no trace of an instrumentation defect (counting idle gaps) — the `turn_id` lag that passed between turns is at most 118 µs (under the old definition in phase A it was 101 seconds). **Likely** (it is an upper bound, so the real pipe wait is at or below this. There is a known error toward the larger side: the first line after idle includes the time to process the tail of the previous chunk — `DrainMark` doc).
- **Answer end (comparison value): 221 µs** — fits comfortably inside M16's narrowest window of 9.4 ms.

## 4. Reproduction

1. Isolated worktree: `git worktree add --detach <폴더> 4ba00b5` → inside it `scripts/run-detached.ps1 -Command "cargo build --release -p engram-dashboard-daemon"` (`__EXIT=0`, 2 min 8 s).
2. For each run: a new data folder → `launch-detached.ps1 -Exe '<워크트리>/target/release/engram-dashboard-daemon.exe' -WorkDir '<codex 작업 폴더>' -EnvVars 'RUST_LOG=warn,engram::codex_handover=debug','ENGRAM_DATA_DIR=<데이터 폴더>'` (call it with `-Command` from Git Bash · paths with slashes).
3. `node m15b.mjs <데이터 폴더> <codex 작업 폴더> <tools|hops|flood> <턴 수> <사건.jsonl>` — put `flood.js` in the work folder. The order in this draft: `tools 7` → `hops 23` → `flood 7`. Between runs, stop the daemon (after checking its own PID · `ExecutablePath`, `taskkill /T /F`) and start a new one.
4. Per log `node parse-handover-trace.mjs <로그> [--json] [--extract …]` → `node m15combine.mjs <parser.mjs> T=<로그> H=<로그> F=<로그> F2=<로그>`.
5. Flood window: `node m15floodcheck.mjs <F2 사건.jsonl> <F2 로그>`.

## 5. Cleanup · cost

- 44 codex turns (T 7 · H 23 · F 7 · F2 7), `gpt-6-sol`/`high`. No usage limit or rate limit was hit (`Failed` turns 0 · error events 0). Run time about 13 minutes in total.
- What was started was only 4 daemons (one per run) and their codex children, and all were cleaned up by their own PID. The isolated worktree and data folders were also cleaned up — the extracts were kept.

## 6. Limits · untested

- **Not one steer was woken directly by a tool-end signal** (`hop="signal" signal="tool_end"` 0). Every text typed into a running turn was handed over immediately (announce), and none was held until the tool end and then handed over. So the verdict's ② was filled with the turn id hop and the immediate hand-over. It keeps the design of measuring and adding three terms separately (README 「한 번에 도는 경로가 없어서」), but **there is no value measuring the one path tool end → steer from end to end.** Certain.
- **That the flood was actually flowing at the steer time** was seen directly only in turn 1 (first delta +568 ms). Turns 2–7 are an inference carried over from the same command and the same tool time (3043–3119 ms). Likely.
- **`lag_us` is an upper bound.** The real pipe wait was not measured separately to compare. The premise 「Windows 파이프 읽기는 그 순간 있는 만큼 돌려준다」 ("a Windows pipe read returns what is there at that moment") (`DrainMark` doc) was also not confirmed separately on this pathway. Uncertain.
- **The model differs from M16** (`gpt-6-sol`/`high` versus `gpt-6-luna`/`low`). The three legs measured are time inside the daemon and are not tied to the model (likely), but whether to use parallel tools and whether to wrap the command is model behavior.
- **The input path differs from the app** — instead of the shell (Tauri), a driver connected to the daemon over the same wire. All legs measured are inside the daemon codex pathway, so we judge there is no effect (likely). **Whether the app attaches to a directly started daemon was not confirmed this time either.**
- **PC load was not controlled** — codex processes of other sessions were running on the same PC. It is a single measurement session.
- Among the instrumentation lines, `wake` and `steer` are printed in a batch after the steer has been fully written (`Traced`). The line order and time in the log are not the write order, and pairs were built only by `seq`.

## 7. Where it departed from the preceding documents · observations about the procedure

- **The procedure draft of the M15 README (start the app and attach it to the directly started daemon · debug build) was not used.** A driver that attaches to the daemon directly over WS needs none of shell build · npm · vite port · identifier conflict. The verdict was also made on release — a debug build sets ① large (phase A debug: high-output tool end `handle_us` 2134 µs · this release: the maximum of the same class 769 µs).
- **If told 「출력을 보이지 말라」 ("do not show the output"), the model swallows the output into a variable** (F 7/7) — it does not become a flood sample. To get a flood, write 「그대로 · 리다이렉트·파이프·변수 없이」 ("as is · without redirect·pipe·variable") (F2 7/7 ran the original text as it was).
- The README's requirement 「보낸 직후 턴 id 가 오기 전에 글을 친다」 ("type the text right after sending, before the turn id arrives") was reproduced 23/23 with two `WriteStdin` frames sent back to back.
- The M15 phase A log (commit `df1ab8d`, debug) uses the old definition of `lag_us`, so it was not used in the verdict (same as the warning at the head of the README).

## 8. Summary table

| Item | Value | Confidence |
|---|---|---|
| Tool-end reaction sum | 2275 µs < 6000 µs → **green** | certain |
| Dominant term | reader lag 1405 µs (tool end of the parallel-tool turn) — an upper bound on the time actually spent busy | likely |
| max ① (tool end) | 783 µs — of which decoding 769 µs (the 6000-line completion line) | certain |
| max ② | 87 µs (turn id hop) · immediate hand-over 72 µs | certain |
| Answer-end sum (M16 comparison) | 221 µs (M16 minimum window 9.4 ms) | certain (value) |
| Samples | tool end 39 · turn id hop 23 · steer during flood 28 (+auxiliary 28) | certain |
| Parallel tools | explicit attempt once → actually parallel (tool-end signal 1) | certain |
| Acceptance of handed-over input | 51/51 got an answer in the same turn · dropped 0 | certain |
