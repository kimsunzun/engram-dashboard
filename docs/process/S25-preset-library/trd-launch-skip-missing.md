# TRD — T-112: a launch skips missing items and an unusable library root, with a notice (ADR-0374)

| Item | Value |
|---|---|
| **Status** | **Draft 1 · 2026-11-20** · canonical ADR: ADR-0374 (relies on ADR-0361 · ADR-0341 · ADR-0363 · ADR-0357 / ADR-0364 · ADR-0370; step-21 context ADR-0385 · ADR-0386) |
| **Parent** | S25 topic ([`trd.md`](trd.md) — §1-3 resolution, §1-5 agent launch). No parent-TRD item of its own: T-112 sits between step 10a and step 21 (`docs/tracking.md` §T-112). What/why = PRD [`prd.md`](prd.md) E4 reasons ① and ④ as amended by ADR-0374. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Main flow 4 · 「Problem rows are the block」 · 「④ stands alone」 · Where to look) · `docs/reference/structure/agent-backend.md` (claude argv row: one `--add-dir` per folder entry *that exists*) · asset-library `resolve.rs` and `launch.rs` module headers · agent `launch.rs` `BlockReason` doc and `notice_text` doc. `architecture.md` and the crate `lib.rs` heads: none. |
| **Line-pointer basis** | Code at `290cc33b`. Code is cited by file + symbol; line numbers are hints. |
| **Not a source** | Frozen child TRDs (`trd-library-commands.md` · `trd-library-edit.md` · `trd-daemon-wiring.md` · `trd-asset-library.md`) are read only to find why something is so. Code wins over any document. |

## 0. Scope and PRD relation

**In.**
- A launch no longer refuses for reason ① when the item is **missing**: an instruction, skill, settings or preset file that is not in the library, and a folder entry whose folder is not there. The item is left out of the launch materials.
- A launch no longer refuses for reason ④ (a library preset attached, the root unset or unusable). Every library item the agent reaches is treated as missing; built-ins still resolve (ADR-0374 decision 2).
- Each skipped item is named in the launch reply notices, in the daemon log and to the launched agent (claude or codex) (ADR-0374 decision 3).
- The help texts that say otherwise are fixed (§6).

**Out.** Reason ② (exists but broken), ③ (conflict), the command-line and working-folder checks — they keep blocking (§3.1, §11 b1). The screens (the client stage, steps 11–13). Several roots (step 21 — §7). PRD text edits (main annotates PRD E4 / R1 addendum from ADR-0374).

**Settings are included** — no per-kind exception (ADR-0374 decision 1; the accepted risk is in §12 R1).

## 1. What the code has today (counted at `290cc33b`)

The problem rows are agent `launch::BlockReason` rows. One list feeds the problem list (`agent.presets`, `preset.get`, `preset.check`), the future tree ⛔ (`LibraryService::resolve_agents`, no production caller yet) and the launch block.

Production users of `BlockReason::Missing`:
1. asset-library `resolve.rs` `Walk::look` — a library item the lookup does not hold (`not-found` `{path}`).
2. `resolve.rs` `Resolution::attach_folder_status` — a folder entry whose status is not `Ok` (`folder-not-found` `{path}` or `{path, why}`). **The same row is used for `Missing`, `NotADirectory` and `Unreadable(why)`** (OS error, `timeout`, `not checked`, failed worker, an unanswered listing or read).
3. `resolve.rs` `Resolution::blocked` — orders ① ② ③ ④.
4. asset-library `lib.rs` `LibraryService::mark_links` — turns a `not-found` row under a link into ② `path-through-link`.
5. asset-library `edit.rs` (the `preset.check` body) — line-0 normalisation, reads every kind.
6. agent `launch.rs` `LaunchBlocked` `Display` and `to_args` — formatters.
7. agent `library_commands.rs` `problem_row` — the `ProblemRow` mapping.

Production users of `BlockReason::LibraryUnavailable`: `resolve.rs` `resolve` (the ④ early return) and `blocked`; asset-library `launch.rs` `LaunchKitSource::launch_check` (the ④ early return — "block ④ is the whole answer"); `edit.rs`; agent `launch.rs` formatters; `problem_row`.

Test-only users: fixtures in agent `commands.rs`, `manager/prelaunch.rs`, daemon `connection_core.rs` (fake kits — the manager does not read the kind), and tests in asset-library `resolve.rs`, `launch.rs`, `lib.rs`, `edit.rs`, `host.rs`, `agent_edit.rs`.

**Only `launch_check` turns rows into a refusal.** So the change sits in `launch_check` and in two `Resolution` helpers; the row variants and the read entrances stay.

Other facts the design relies on:
- `launch_check` B30 gate: the advisory phase runs only when `resolution.problems.is_empty()`.
- `LaunchPlan.folders` is every resolved folder, whatever its status (today a non-`Ok` folder always blocked, so it never mattered). claude turns each into `--add-dir` (`backend/claude/launch.rs`).
- Notices already travel: `LaunchPlan.notices` → manager `prelaunch::settle` logs each (`info`, "launch notice") → `PlanNotices::against` (once per agent, ADR-0341 decision 5) → `agent.spawn` reply rows `{code, args, line?, text}` (`launch_notice_row`, `notice_text`) and wire `AgentEvent::Spawned.notices`. ADR-0363's `folder-listing-skipped` uses exactly this path.
- The only channel that reaches a launched agent is the merged instructions (`merge_instructions` → `prepare_in` writes `instructions.md`): claude `--append-system-prompt-file`; codex JSON `developerInstructions`; codex terminal `-c developer_instructions='…'`. It exists only in a real plan (ADR-0370).
- codex terminal: the merged text goes on the command line through `cmdline::priming_value`, which refuses `' " % \` and control characters, and `cmdline::fits` bounds the length. `SettingsCheck::command_line_fits` answers both (`codex-priming-unsafe-char` · `command-line-too-long`); claude answers `Ok` always.
- Default log level is `warn`; a release daemon does not read `RUST_LOG` (`logging-conventions.md`). The existing "launch notice" line is `info`, so it is not in a release log file.

## 2. Module boundaries

| Where | Change |
|---|---|
| agent `launch.rs` | Two notice-code constants and their own wording (§3.3). `BlockReason` doc: `Missing` and `LibraryUnavailable` are problem rows a launch never refuses on (ADR-0374). A small pure helper telling whether a code reports a launch skip (§3.5). No new type, no field change. |
| agent `manager/prelaunch.rs` | `settle` logs the two skip codes at `warn` (§3.5). Nothing else. |
| asset-library `resolve.rs` | `Resolution::blocked` = only the rows a launch refuses on. New `Resolution::skipped` = the rows a launch leaves out. `resolve` = the ④ guard + an expansion step that the launch can call without the guard (§3.2). Row shapes unchanged. |
| asset-library `launch.rs` | `launch_check` (skip, the ④ path, folders filter, B30 gate, notices, the agent note), `prepare_in` (writes the note), `Stamp` (carries the note). |
| daemon | `prompts/engram-help.md` (§6). `control/help.rs` test doc and asserts. A live scene in `tests/launch_kits_live.rs`. No wiring change. |
| backend `claude` / `codex` | **No change.** Folders are filtered before they reach the plan; the note travels inside the merged instructions. |
| shell, frontend | **No change.** Notices pass through as generic rows (`src-tauri/src/commands/layout.rs` `SpawnedAgent.notices`); the webview does not show notices yet (step 11). |

Backend knowledge stays in port answers: the fallback of the agent note (§3.4) asks only `command_line_fits`; no backend word is named in asset-library.

## 3. Behaviour

### 3.1 What a launch skips and what still blocks

A launch **skips** a row when it is:
- `Missing` with `not-found` — a library item (attached preset, nested preset, instruction, skill, settings) not found. Rows that `mark_links` turned into ② `path-through-link` are not skipped (it exists behind a link).
- `Missing` with `folder-not-found` **whose folder status is `Missing` or `NotADirectory`** (the folder is not there).

A launch **still refuses** on:
- ② every `Broken` row: YAML syntax, `library-read-failed`, `wrong-kind`, `preset-cycle`, `builtin-in-library-preset`, `item-path-invalid` (an unparsed attached spelling), settings refusals, skill format and name, `folder-path-cli-unsafe`, `path-through-link`, `command-line:`, `working-folder:`.
- ③ every `Conflict` row.
- A folder entry that exists but cannot be read, or whose check gave no answer in time (`Unreadable(why)` — OS error, `timeout`, `not checked`, failed worker, unanswered listing or read). Still the `Missing` / `folder-not-found` `{path, why}` row (ADR-0363 decision 3 · PRD R1 addendum). User decided — §11 U1 (§11 b).

The split reads the folder's recorded `FolderStatus` (`ResolvedFolder.status`), never the row's `args`. `Resolution::blocked` and `Resolution::skipped` partition `problems`; the order inside each keeps today's rule (kind, then expansion order).

Why ② ③ and the command-line checks stay (b1): each describes something that is there but wrong. Running would run with content other than what the file says (a half-read preset, a refused settings value), pick an undefined winner between two skills, or fail at spawn (a command line that does not fit, a working folder the backend refuses). ADR-0374 decision 4 left them out; `CLAUDE.md` 「최소한만 막기」 keeps blocking what breaks execution.

### 3.2 An unusable library root (old reason ④)

When a library preset is attached and the root is not usable (`RootState` not `Ok`):
- The launch does not read the overlay and does not use the catalog (ADR-0361's reason stands: a stale or empty catalog cannot be trusted).
- It expands the agent's presets over a lookup that answers every built-in as today and **every library item as missing** (`UnusableRootLookup` — the name says what step 21 parameterises, §7). So `engram:/default` and every `engram:/…` reference resolve; each attached library preset becomes a `not-found` row; a library item a built-in preset reaches (grammar allows it, nothing shipped does) is missing too.
- `resolve` is split into the ④ guard and the expansion step (`pub(crate)`); the launch calls the expansion step directly. The read entrances keep calling `resolve`, so the problem list and `agent.presets` still show the one ④ row (ADR-0361 decision 1 for screens stands).
- `mark_links` is not run (there is no usable root to probe).
- After that the launch is a normal launch: folder phase, advisory phase (B30 now reads blocking rows only), command-line and working-folder checks. Unparsed attached spellings come back as ② rows and block, as they do with a usable root.
- The library `not-found` rows of this launch are reported by one notice for the root (§3.3), not one notice each — ADR-0361's 「줄 하나만」 carried over to the notice.

### 3.3 Notices (caller)

Two new codes in agent `launch.rs`, each with its own wording arm in `own_wording` (ADR-0364 「영향」: a code that reports a failure gets its own arm). Facts only — no intended-behavior sentence, no fix (ADR-0357 · ADR-0364). Values are escaped (`escape_controls`). Draft wording (main decided — §11 U2):

| Code | `args` | `line` | Text |
|---|---|---|---|
| `missing-item-skipped` | `{item, via}` — `item` = the row's item (library path or `folder:<path>`), `via` = the row's chain | the row's line (the referencing preset's line), when it has one | `Notice: not found, left out of this launch: <item> (listed in <last of via>, line <n>)` — without `via` it ends at `<item>`; without a line it drops `, line <n>` |
| `library-unavailable-skipped` | `{message, items}` — `message` = the ④ reason today put in the block row (`no library root is set`, `missing`, `not a folder`, the stat error, the first-scan reason); `items` = the library items left out, in expansion order | none | `Notice: the library folder cannot be used (<message>). Left out of this launch: <items, joined by ", ">` — an empty list ends after the first sentence |

- One `missing-item-skipped` per skipped row; one `library-unavailable-skipped` per launch with an unusable root. The skip notices come first in `LaunchPlan.notices`, then the resolution's notices, then the folder and same-name notices.
- Malformed `args` fall back to the generic wording (ADR-0364 decision 2 — accepted, as for the other six).
- **Once per agent** (ADR-0341 decision 5): the new codes join the notice fingerprint like every other code; an unchanged skip set is not shown again in the reply. The log and the agent note are on every launch. Main decided — §11 U3.
- `agent.presets` does **not** gain the skip notices: its `problems` rows already show each missing item; the notices are what a launch did.

### 3.4 Telling the launched agent

Channel: the merged instructions — the one channel that reaches claude and codex in every mode (§1). No new channel.

- `launch_check` builds one **launch note** from the skip set and appends it as the last part of the merged instructions (after the priming and every source). It is part of the text the check measures, so it cannot turn a skip into a block:
  1. **Named form** — every skipped item with the preset that lists it, and the root sentence when §3.2 applied.
  2. If `command_line_fits` refuses the merged text with the named form, the **count form** — one fixed ASCII sentence with the number of skipped items, no path.
  3. If it still refuses, **no note**; the check is then judged on the text without a note (today's text), and the decision is logged at `warn` once.
  claude always takes 1 (`command_line_fits` is `Ok`). codex JSON takes 1 (instructions are not on the line). codex terminal takes 1 when the names hold no `' " % \` and fit; a folder path (backslashes) gives 2.
- Draft text (main decided — §11 U2):
  - named: `Engram launch note: these items your presets reference were not found and were left out of this launch:` then one line per item `- <item> (listed in <preset>)`, then, for §3.2, `The library folder could not be used (<message>), so these library items were left out: <items>`.
  - count: `Engram launch note: <n> items your presets reference were not found and were left out of this launch.`
- `prepare_in` writes the same note: the check stores the chosen note text in the plan's `Stamp` (asset-library private, opaque to the manager — no `LaunchPlan` field change), and `prepare_in` passes it to the one merge. A plan whose only instruction part is the note still writes `instructions.md`.
- The note changes the instructions hash, so codex JSON re-applies its instructions on resume when the skip set changes (the existing `instructions_changed` path) — intended.
- This is an exception to ADR-0337 decision 3 / ADR-0357 decision 2 ("nothing sent to the started agent"), made by ADR-0374 decision 3, for these two codes only. The other notices still reach only the caller.

### 3.5 Daemon log

`prelaunch::settle` already logs every plan notice once per launch (after the accepted check, not on the discarded pre-create or raced checks). It logs the two skip codes at **`warn`** — abnormal but falling back safely (`logging-conventions.md` levels) — and every other code at `info` as today:

```
tracing::warn!(agent = %id, code = %notice.code, args = %notice.args, line = ?notice.line, "launch item skipped");
```

Identifiers and values are fields; the message is short English. The level choice comes from a pure agent `launch.rs` helper over the agent-owned code constants (no backend knowledge in the manager). The note-dropped case of §3.4 step 3 is one `warn` in `launch_check`: `"launch note dropped; command line cannot carry it"` with `agent` and the refusal code.

### 3.6 What does not change

- Problem rows, their kinds and codes; `ProblemRow`; `preset.get` / `preset.check` / `agent.presets` answers; the ④ row on screens. Screens still show a missing line as 「찾을 수 없음」 with relink (ADR-0374 「영향」).
- Attach and create rules (`agent.attachPreset`, `agent.new {presets}` — `library-unavailable` refusal when no first scan was applied, ADR-0386 decision 5): those are edits, not launches.
- `LaunchBlocked` shape. A launch reply never carries `missing` or `library_unavailable` rows any more; the kinds stay in the shape because the problem list uses them.
- Boot restore: an agent whose only problems are missing items now starts instead of `RestoreOutcome::Failed`.

## 4. Data shapes

- **Wire**: no new type, no binding regeneration. Two new notice codes inside the existing notice row; their `args` as §3.3. `LaunchBlocked.to_args` keeps its kinds.
- **Persisted**: the profile's `notice_fingerprint` may now hold the two codes (canonical `code` + `args`, ADR-0341) — no migration; an old fingerprint simply differs once. Nothing else persisted.
- **In memory**: `Stamp` (inside `LaunchPlan.stamp`, asset-library only) gains the chosen note text.

## 5. Concurrency

Nothing new. The split and the note are pure computation inside the existing check. A missing item that appears between check and prepare is simply not carried (the next launch carries it). A carried item that disappears between check and prepare is `Stale` → `LaunchRaced` as today; the retry then skips it. The ④ path takes the same read locks as before and no write lock.

## 6. Texts that promise blocking (found and listed)

10a was told its texts must stay true (ADR-0386 decision 11). Checked in code:

| Text | Today | Fix |
|---|---|---|
| `prompts/engram-help.md` agent screen, `agent.spawn` | "notices in a success reply are notices about things left out on purpose" | **False after T-112.** Say: a missing preset item or an unusable library folder does not stop the start; it is left out and named in `notices` (once per agent). "If the launch is blocked …" stays. |
| `prompts/engram-help.md` library screen, `agent.presets` | "the folders, conflicts, problems and notices a start would meet" | Make it say a start leaves out the `missing` rows and blocks on the others; no word 「block」 on the library screen (the help test asserts it) — e.g. "a start leaves missing items out". |
| `prompts/engram-help.md` library screen, the problems paragraph | "A missing or broken item shows as a row in problems …" | True. Add one sentence: a start leaves a missing item out and names it in `notices`. |
| `builtin/instructions/priming.md` | no blocking claim (only "If an engram command is refused … tell the owner") | None. No priming change (the codex terminal line budget, ADR-0386 「영향」). |
| daemon `control/help.rs` test `the_library_screen_teaches_every_library_command_and_the_edit_rules` doc | "that stops being true when launches skip missing items" | Tense only; add an assert that the agent screen names left-out missing items. |
| asset-library `resolve.rs` header ("the problem list, the tree ⛔ and the launch block come from the same rows"), `Resolution::blocked` doc, `resolve` doc | one list = block | Rows are one list; the launch refuses on `blocked()` only. |
| asset-library `launch.rs` header ("Block ④ is the whole answer — nothing else is checked") | ④ blocks | §3.2. |
| agent `launch.rs` `BlockReason` doc ("One reason that blocks a launch"), `LibraryUnavailable` doc, `notice_text` doc (also says "Korean" while the wording is English) | | Rewrite. |
| `docs/reference/structure/asset-library.md` Main flow 4, 「Problem rows are the block」, 「④ stands alone」 | | Rewrite (header line). |
| `docs/reference/structure/agent-backend.md` claude argv row | "one `--add-dir <folder>` per folder entry" | "per folder entry that exists". |
| PRD E4 (four reasons, "④ no library folder"), R1 addendum, check line "for each of the four block reasons" | | Not this task — main annotates from ADR-0374. |

## 7. Interplay with step 21 (several roots — ADR-0385)

- Nothing here assumes one root beyond what the code already does. The unusable-root rule is written as "items of an unusable root are missing" (`UnusableRootLookup`); with one root that is every library item. Step 21 makes it per root: a lookup that answers `Missing` for items of the roots that are unusable, and one `library-unavailable-skipped` notice per unusable root with an added `root` field in `args` (an added field, not a reshape).
- The built-in as an always-present read-only root (step 21) needs nothing from T-112: built-ins already resolve in the unusable-root path.
- **Step 21 inherits**: the skip / block split (`blocked` · `skipped`), the two codes and their wording, the agent note and its fallback, the `warn` log, and the read entrances' one-row ④ (step 21 decides whether a screen shows one row per unusable root).
- Nothing T-112 builds is to be removed by step 21.
- **Handed to the client stage (steps 11–13)**: tree ⛔ is 「지금 실행하면 막힐까」 (PRD E4). With `blocked()` now meaning the launch block, an agent whose only problems are missing items has rows but no ⛔. Step 13 confirms that with the user (or adds a lighter mark).

## 8. Convention check — "a referenced file is missing → skip and warn"

Light check, sources read directly:
- **git** skips a missing `include.path` file silently and dies on one it cannot access (`config.c` `handle_path_include` → `wrapper.c` `access_or_die`, where `access_error_is_ok` accepts only a missing-file error).
- **OpenSSH** skips an `Include` that matches no file and goes on, with a `debug` line (`readconf.c`, `GLOB_NOMATCH` → "include … matched no files").
- **codex** refuses to start when the one file named by `model_instructions_file` cannot be read (`codex-rs/core/src/config/mod.rs` `try_read_non_empty_file` returns the error).
- **Claude Code**'s memory docs (`code.claude.com/docs/en/memory`, 「Import additional files」) do not say what a missing `@import` does — not confirmed.

Reading: include-style references (a list that pulls in files) skip a missing file — git, OpenSSH; a single file the tool is explicitly told to use fails — codex. A preset's items are include-style. git's split (missing → skip, unreadable → fail) matches U1 option A. The skip-and-tell precedent inside Engram is ADR-0363 (codex skill discovery, ripgrep, find, git, walkdir).

## 9. Test plan

QA table rows (`.claude/skill-bindings/qa.md`): 「Rust 소스」 → 전체 회귀; asset-library `src/**` and daemon `src/control/**` are on the **claude and codex lane** paths → both lanes; `manager/prelaunch.rs` is on the launch path (「에이전트 수명」 row → GUI check on).

| What | Checked by |
|---|---|
| Split: `not-found` and a `Missing` / `NotADirectory` folder are in `skipped()`; an `Unreadable` folder (each `why`), `path-through-link`, every ② and ③ are in `blocked()` | `resolve.rs` unit tests |
| A missing instruction, skill, settings, preset, nested preset and folder: the plan launches, the item is not in instructions / skills / raw settings / `folders`, one `missing-item-skipped` each with `via` and `line` | asset-library `launch.rs` tests (claude and codex JSON worlds) |
| Missing + broken → refusal holds only the broken row; missing + conflict likewise | rewrite `a_check_blocks_on_missing_broken_and_conflicting_items_in_that_order` |
| Unusable root (unset, missing, not a folder, first scan failed): plan launches with built-ins only, one `library-unavailable-skipped` naming the attached library presets, no catalog read, no `mark_links`; an unparsed attached spelling still blocks | replace `block_four_is_the_whole_answer_and_asks_nothing_else` |
| Read entrances still give the one ④ row and `not-found` rows | existing `lib.rs` / `host.rs` / `agent_edit.rs` tests stay green unchanged |
| B30: with only skipped rows the advisory phase runs | rewrite `a_blocked_check_runs_no_advisory_io_and_judges_the_working_folder_as_spelled` to block with a ② row; a new test for skip-only |
| Agent note: named form in `instructions.md` and the hash; codex terminal with a folder path → count form; a priming near the limit → no note and no block | `launch.rs` tests over `prepare_in` and the fallback; `builtin.rs` `the_shipped_priming_fits_the_codex_terminal_command_line` extended with the count sentence |
| Wording and fallback of the two codes | agent `launch.rs` golden tests beside the six existing arms |
| `warn` for the skip codes, `info` for the rest | agent unit test over the level helper |
| Help screens | daemon `control/help.rs` tests |
| Live, claude lane: an agent with a missing instruction and a missing folder starts; the model repeats the launch note | new scene L5 in `crates/engram-dashboard-daemon/tests/launch_kits_live.rs` (`claude_l5_…`, one turn) |
| Live, codex lane: JSON — the rollout developer text holds the named note (`rollout_developer_texts`); terminal — the command line holds the count form (`command_line_holding`) | extend `codex_l3_…` or add `codex_l5_…` |
| Live, end to end through the CLI: inside a spawned agent (the `engram` CLI needs its token) run `engram agent.spawn --target <agent with a missing preset item>`; the reply has `missing-item-skipped`, a second run has no notice, the daemon log file has the `warn` line, and the launched agent can say what was left out | manual QA step, reported with the reply text |
| GUI: double-click an agent with a missing item in the agent tree — it starts, no block window | qa-verify 「GUI 실측」 |

## 10. Implementation units

Sequential (U2 uses U1's constants; U3 edits help and tests that read U2's behaviour). Build and tests green at the end of each.

- **U1 — agent**: `launch.rs` two constants, two wording arms, the level helper, `BlockReason` / `notice_text` docs; `manager/prelaunch.rs` `settle` level. Tests: goldens, level helper.
- **U2 — asset-library**: `resolve.rs` (`blocked` · `skipped`, the expansion step without the ④ guard, `UnusableRootLookup`), `launch.rs` (`launch_check` skip and ④ path, `folders` filter, B30 gate, notices, the note and its fallback, `Stamp`, `prepare_in`), `builtin.rs` test; module headers. Tests in §9.
- **U3 — texts and live**: `prompts/engram-help.md`, `control/help.rs` tests, reference files (header line), live scenes in `launch_kits_live.rs`.

## 11. Decisions

### (a) Decided by the user — ADR-0374 (closed)
- Decision 1: a missing item does not block; settings included, no per-kind exception.
- Decision 2: an unusable root does not block; its items are treated as missing; built-ins resolve.
- Decision 3: each skipped item named in the reply notices, the daemon log and to the launched agent.
- Decision 4: only "missing" changes; ②, ③ and the command-line checks, the notice code and wording, and how the agent is told are this TRD's.
- Decision 5: a separate task after step 9a; current blocks stay until it lands.

### (a2) Decided by the user — 2026-11-20
- U1 — a folder entry that exists but cannot be read, or whose check did not answer in time, **keeps blocking** (option A — recommended): 「ㅇㅇ 막아.」 Context: a network share down or access denied refuses the launch, as today (ADR-0363 decision 3); a missing folder entry is skipped (ADR-0374); an unreadable folder below an entry is already skipped with a notice (ADR-0363).

### (b) Main decided (details the user leaves to main — reported)
- U2 — the wording of the two notices (§3.3) and of the agent note (§3.4) is as drafted; facts only (ADR-0357).
- U3 — the skip notices in the reply are **once per agent** (ADR-0341 decision 5 as it stands; a changed skip set shows again). Rejected: every launch — adds a per-code exemption in `PlanNotices::against`, and the user already rejected repeating notices (「알면서 쓰는사람도 있을텐데 그 메세지 실행하면서 계속 보면 안되잖아」). The log and the agent note are on every launch.
- b1 — ②, ③, the command-line and working-folder checks keep blocking (§3.1): they are there but wrong, or the spawn would fail; ADR-0374 decision 4 did not cover them.
- b2 — the agent note falls back named → count → none (§3.4). Rejected: tell a codex terminal agent nothing (as for the other terminal notices, ADR-0337 decision 3) — the count is cheap and the note can never turn a skip into a block.

### (c) Internal choices (author decides, reported)
- c1 — the split is two `Resolution` methods over unchanged rows, decided from `FolderStatus`, not from `args`.
- c2 — the launch's unusable-root path calls the expansion step without the ④ guard over `UnusableRootLookup`; `resolve` keeps the guard for the read entrances.
- c3 — library rows of an unusable root fold into one `library-unavailable-skipped` notice; no per-item notices for them.
- c4 — one `missing-item-skipped` per row (the problem-row grain), not one aggregated notice.
- c5 — the note is the last part of the merged instructions and rides in the asset-library `Stamp`; `LaunchPlan` gains no field.
- c6 — the `warn` line is emitted in `prelaunch::settle` (once per launch), chosen by an agent helper over agent-owned codes.
- c7 — `agent.presets` does not gain skip notices.
- c8 — `LaunchPlan.folders` holds only folders whose status is `Ok`.

## 12. Risks

- **R1 (accepted, ADR-0374)** — a missing settings file that carried a permission or sandbox limit lets the agent run without it. The notice, the `warn` line and the agent note are the only signals.
- **R2** — a typo in a preset path now launches. The problem list still shows 「찾을 수 없음」; the reply names it once; the agent is told every launch.
- **R3** — a removed library folder with many items gives many notices in one reply. Accepted (each is one line; the root case is one notice).
- **R4** — file names from the library enter the agent's instructions. They are escaped and framed as a list of names; same exposure as the source headers `<!-- path -->` today.
- **R5** — the codex terminal agent usually gets the count only (b2).
- **R6** — tree ⛔ meaning for missing-only agents is left to step 13 (§7).

## 13. Input claims corrected (brief and ADR-0374 → code at `290cc33b`)

- ADR-0374 「영향」 lists agent `backend/claude` and `backend/codex` as touched. They need no change: folders are filtered in asset-library and the note travels in the merged instructions.
- The real call sites add asset-library `lib.rs` `mark_links` (must run before the split) and agent `manager/prelaunch.rs` `settle` (the log line); `edit.rs` and `library_commands.rs` read the variants but need no change.
- ADR-0337 decision 3 and ADR-0357 decision 2 say nothing is sent to the started agent; ADR-0374 decision 3 makes an exception for the skip codes. The ADR record should say so (main).
- ADR-0341 decision 5 says the daemon log records omissions on every launch; it does, but at `info`, which a release log never holds (default `warn`, no `RUST_LOG`).
- PRD R1 addendum / E4 addendum put an unreadable folder entry under ① with a missing one; after T-112 ① splits (§3.1).
- agent `launch.rs` `notice_text` doc says the wording is Korean; the code is English.

## 14. Review record

_(no review yet)_
