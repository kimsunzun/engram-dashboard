# TRD — T-112: a launch skips missing items and an absent library root, with a notice (ADR-0374)

| Item | Value |
|---|---|
| **Status** | **Draft 3 · 2026-11-20** — the user's rule 「없음 → 건너뜀 / 있는데 못 읽음·깨짐 → 막음」 applied to every kind, the library root included (§11 a2). Canonical ADR: ADR-0374, narrowed by that rule (main records the amendment — §13) (relies on ADR-0361 · ADR-0341 · ADR-0363 · ADR-0357 / ADR-0364 · ADR-0370 · ADR-0215; step-21 context ADR-0385 · ADR-0386) |
| **Parent** | S25 topic ([`trd.md`](trd.md) — §1-3 resolution, §1-5 agent launch). No parent-TRD item of its own: T-112 sits between step 10a and step 21 (`docs/tracking.md` §T-112). What/why = PRD [`prd.md`](prd.md) E4 reasons ① and ④ as amended by ADR-0374 and the user's rule of 2026-11-20. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (the root paragraph 「an agent with a library preset attached is blocked with ④」 · Main flow 4 · 「Problem rows are the block」 · 「④ stands alone」 · Where to look) · `docs/reference/structure/agent-backend.md` (claude argv row: one `--add-dir` per folder entry the launch did not skip) · asset-library `resolve.rs` and `launch.rs` module headers · agent `launch.rs` `BlockReason` doc and `notice_text` doc. `architecture.md` and the crate `lib.rs` heads: none. |
| **Line-pointer basis** | Code at `290cc33b`. Code is cited by file + symbol; line numbers are hints. |
| **Not a source** | Frozen child TRDs (`trd-library-commands.md` · `trd-library-edit.md` · `trd-daemon-wiring.md` · `trd-asset-library.md`) are read only to find why something is so. Code wins over any document. |

## 0. Scope and PRD relation

**The rule (user, 2026-11-20 — every kind: instruction · skill · settings · preset · folder entry · the library root):** absent → the launch skips it with a notice; exists but cannot be read, did not answer, or is broken → the launch is blocked.

**In.**
- A launch no longer refuses for reason ① (missing): an instruction, skill, settings or preset file that is not in the library, and a folder entry whose folder is not there. The item is left out of the launch materials.
- A launch no longer refuses for reason ④ when the library root is **absent** (unset, missing, or a path that is not a folder): every library item the agent reaches is treated as missing; built-ins still resolve.
- A launch **is** refused when the root exists but cannot be read or did not answer (today's ④ row, unchanged), and when a folder entry exists but cannot be read or did not answer (a new ② code — §3.1).
- Each skipped item is named in the launch reply notices, in the daemon log and, where the instruction channel allows it, to the launched agent (claude or codex) (ADR-0374 decision 3; §3.4).
- The help texts and docs that say otherwise are fixed (§6).

**Out.** Reason ② (exists but broken), ③ (conflict), the command-line and working-folder checks — they keep blocking (§3.1, §11 b1). The screens (the client stage, steps 11–13). Several roots (step 21 — §7). PRD and ADR text edits (main — §13).

**Settings are included** — no per-kind exception (ADR-0374 decision 1; the accepted risk is in §12 R1).

## 1. What the code has today (counted at `290cc33b`)

The problem rows are agent `launch::BlockReason` rows. One list feeds the problem list (`agent.presets`, `preset.get`, `preset.check`), the future tree ⛔ (`LibraryService::resolve_agents`, no production caller yet) and the launch block.

Production users of `BlockReason::Missing`:
1. asset-library `resolve.rs` `Walk::look` — a library item the lookup does not hold (`not-found` `{path}`).
2. `resolve.rs` `Resolution::attach_folder_status` — a folder entry whose status is not `Ok` (`folder-not-found` `{path}` or `{path, why}`). **The same row is used for `Missing`, `NotADirectory` and `Unreadable(why)`** (OS error, `timeout`, `not checked`, failed worker, an unanswered listing or read). A folder key the statuses lack keeps `status: None` and gets no row.
3. `resolve.rs` `Resolution::blocked` — orders ① ② ③ ④.
4. asset-library `lib.rs` `LibraryService::mark_links` — turns a `not-found` row whose path goes through a link inside the library into ② `path-through-link`.
5. asset-library `edit.rs` (the `preset.check` body) — line-0 normalisation, reads every kind.
6. agent `launch.rs` `LaunchBlocked` `Display` and `to_args` — formatters.
7. agent `library_commands.rs` `problem_row` — the `ProblemRow` mapping.

Production users of `BlockReason::LibraryUnavailable`: `resolve.rs` `resolve` (the ④ early return) and `blocked`; asset-library `launch.rs` `LaunchKitSource::launch_check` (the ④ early return — "block ④ is the whole answer"); `edit.rs`; agent `launch.rs` formatters; `problem_row`.

Tests that run the **real** resolution or launch check and assert a ① or ④ row or refusal that this task changes (they go red when U2 lands):
- daemon `src/lib.rs` `the_production_manager_launches_through_the_kept_library_service` — real production wiring; asserts `LaunchBlocked` `[LibraryUnavailable]` without a root, then a `Missing` `P.yaml` row after `change_root`. Under U2 neither refusal happens (the later failure is not `LaunchBlocked`).
- asset-library `launch.rs`: `a_check_blocks_on_missing_broken_and_conflicting_items_in_that_order`, `block_four_is_the_whole_answer_and_asks_nothing_else` (a removed root and an unset root — both now skip), `a_blocked_check_runs_no_advisory_io_and_judges_the_working_folder_as_spelled`, `folder_io_runs_with_the_library_lock_released` (expects `[("missing","not-found")]`), `a_create_with_a_missing_folder_entry_is_blocked_before_the_add` (ADR-0348 R4), `a_folder_entry_added_between_the_checks_blocks_after_the_add` (R4b); and, for the new folder code, `an_unreachable_folder_blocks_the_launch_with_one_line`, `a_held_file_read_in_a_folder_entry_blocks_it_and_the_check_ends_inside_the_budget`, `a_read_whose_worker_failed_blocks_its_folder_like_one_that_did_not_answer` (they expect a `Missing` `folder-not-found` row).
- asset-library `resolve.rs`: `the_block_is_the_problem_list_ordered_by_kind` (two `Missing` folders expected inside `blocked()`), `a_folder_that_is_not_ok_is_a_missing_line_on_every_backend_and_mode` (its `Unreadable` case).
- asset-library `lib.rs` `resolve_agents_spends_one_read_budget_on_every_held_folder_and_stats_each_once` (expects `Missing` `folder-not-found` with `why: timeout`).
- Tests that build `RootState::Unavailable` (asset-library `edit.rs` imports `RootState`; `resolve.rs` tests) — renamed variants (c11).

Tests with **fake** kits or fixtures (the manager does not read the kind — stay green): agent `commands.rs`, `manager/prelaunch.rs`, `tests/activation.rs`; daemon `connection_core.rs`, `tests/control_agent.rs`. `host.rs` folder views (they carry `FolderStatus` words, not rows), and the `blocked()` tests over other ② rows stay green.

**Only `launch_check` turns rows into a refusal.** So the launch change sits in `launch_check` and in two `Resolution` helpers; the row variants stay. The one row change visible to the read entrances is the unreadable folder entry (§3.1).

Other facts the design relies on:
- `launch_check` B30 gate: the advisory phase runs only when `resolution.problems.is_empty()`.
- `launch_check` asks `root_state(io, root, Ok(()), library_attached)` — it never passes a first-scan failure. `stat_root` maps the root's `FolderStatus`: `Missing` → `missing`, `NotADirectory` → `not a folder`, `Unreadable(why)` → the OS error, `timeout` or `STAT_UNANSWERED`. All of them are today one `RootState::Unavailable(why)`.
- `LaunchPlan.folders` is every resolved folder, whatever its status (today a non-`Ok` folder always blocked, so it never mattered). claude turns each into `--add-dir` (`backend/claude/launch.rs`).
- Notices already travel: `LaunchPlan.notices` → manager `prelaunch::settle` logs each (`info`, "launch notice") → `PlanNotices::against` (once per agent, ADR-0341 decision 5) → `agent.spawn` reply rows `{code, args, line?, text}` (`launch_notice_row`, `notice_text`) and wire `AgentEvent::Spawned.notices`. ADR-0363's `folder-listing-skipped` uses exactly this path.
- The check runs up to three times per launch (`check_before_create` for a create, then up to `PRELAUNCH_ATTEMPTS = 2` in `prelaunch`); `settle` runs once, on the accepted check; `prepare_in` runs once.
- The only channel that reaches a launched agent is the merged instructions (`merge_instructions` → `prepare_in` writes `instructions.md`): claude `--append-system-prompt-file` (appends to what claude loads); codex JSON `developerInstructions` and codex terminal `-c developer_instructions='…'` (replace the user's own `developer_instructions` from `~/.codex/config.toml` — ADR-0215). It exists only in a real plan (ADR-0370).
- codex terminal: the merged text goes on the command line through `cmdline::priming_value`, which refuses `' " % \` and control characters, and `cmdline::fits` bounds the length. `SettingsCheck::command_line_fits` answers both (`codex-priming-unsafe-char` · `command-line-too-long`); claude answers `Ok` always.
- Default log level is `warn`; a release daemon does not read `RUST_LOG` (`logging-conventions.md`). The existing "launch notice" line is `info`, so it is not in a release log file.

## 2. Module boundaries

| Where | Change |
|---|---|
| agent `launch.rs` | Two notice-code constants and their own wording (§3.3). `BlockReason` doc: a launch never refuses on a `Missing` row; it refuses on a `LibraryUnavailable` row only when the root exists but cannot be read or did not answer; `Missing` = absent, `Broken` = exists but cannot be used (ADR-0374 and the user's rule). A small pure helper telling whether a code reports a launch skip (§3.5). No new type, no field change, no `Display` change. |
| agent `backend/settings_check.rs` | One port answer `instructions_append(mode) -> bool` — whether Engram's instructions are added to what the CLI already loads (`true`) or replace an instruction value the user set (`false`). Default `false`; claude overrides with `true`. codex keeps the default. (§3.4) |
| agent `manager/prelaunch.rs` | `settle` logs the two skip codes at `warn` (§3.5). Nothing else. |
| agent `commands.rs` (+ regenerated `bindings/`) | Doc comments and the `launch_blocked_reason` message (§6). |
| asset-library `resolve.rs` | `RootState::Unavailable` splits into `Absent(why)` and `Unreadable(why)`. `Resolution` records whether its ④ row stands for an absent or unset root. `attach_folder_status`: an `Unreadable` folder becomes the ② row `folder-unreadable` (new code). `Resolution::blocked` = the rows a launch refuses on; new `Resolution::skipped` = the rows it leaves out. `resolve` = the ④ guard + an expansion step the launch can call without the guard (§3.2). |
| asset-library `lib.rs` | `root_state` / `stat_root` answer `Absent` or `Unreadable` (a first scan that could not be applied is `Unreadable`). |
| asset-library `launch.rs` | `launch_check` (skip, the absent-root path, the unreadable-root block, folders filter, B30 gate, notices, the agent note), `prepare_in` (writes the note, logs a dropped note once), `Stamp` (carries the note and the dropped fact). |
| asset-library `agent_edit.rs` | `AgentPresetsView.problems` doc (§6). |
| daemon | `prompts/engram-help.md` (§6). `control/help.rs` test doc and asserts. `src/lib.rs` wiring test (§9, in U2). A live scene in `tests/launch_kits_live.rs`. No wiring change. |
| backend `claude` / `codex` launch translation | **No change.** Folders are filtered before they reach the plan; the note travels inside the merged instructions. Only claude's `settings.rs` gains the one port answer above. |
| shell, frontend | **No change.** Notices pass through as generic rows (`src-tauri/src/commands/layout.rs` `SpawnedAgent.notices`); the webview does not show notices or rows yet (steps 11–13). |

Backend knowledge stays in port answers: the agent note asks only `command_line_fits` and `instructions_append`; no backend word is named in asset-library.

## 3. Behaviour

### 3.1 What a launch skips and what still blocks

**The split is by kind:** a launch skips exactly the `Missing` rows (and, at the read entrances, a `LibraryUnavailable` row that stands for an absent or unset root); everything else blocks.

`Missing` (absent — skipped):
- `not-found` — a library item (attached preset, nested preset, instruction, skill, settings) not found.
- `folder-not-found` `{path}` — a folder entry whose folder is not there: `FolderStatus::Missing` or `NotADirectory` (a file at that path is not a folder). Consequence (main decided, H2): a folder entry on a drive letter that is not present (unplugged disk, unmapped `subst`) is answered `NotFound` by Windows, so it is absent and skipped; a network share that does not answer is `Unreadable` and blocks.

Everything else blocks:
- ② every `Broken` row: YAML syntax, `library-read-failed`, `wrong-kind`, `preset-cycle`, `builtin-in-library-preset`, `item-path-invalid` (an unparsed attached spelling), settings refusals, skill format and name, `folder-path-cli-unsafe`, `path-through-link`, `command-line:`, `working-folder:`, and the new **`folder-unreadable`** below.
- ③ every `Conflict` row.
- `LibraryUnavailable` when the root exists but cannot be read or did not answer (§3.2).

**`folder-unreadable` (new ② code, user H3).** A folder entry whose `FolderStatus` is `Unreadable(why)` — it exists but cannot be read, or its stat, listing or a read gave no answer in time (OS error, `timeout`, `not checked`, failed worker, unanswered listing or read) — is a `Broken` row with `item` = `folder:<path>` (ADR-0361 item form), code `folder-unreadable`, `args` `{path, why}`, `line` = the entry's line. Before, it was `Missing` `folder-not-found` with `why`; `folder-not-found` now always has `args` `{path}`. The row is the same at the read entrances and at the launch, so a screen (step 13) shows it as "cannot be read", apart from "not found". **The row's user-facing wording is short and fixed (by code); it names no cause.** The OS error or `timeout` stays in `args.why` for the log and an LLM caller.

`LibraryUnavailable` in the split: at the read entrances, the single ④ row is in `skipped()` when the resolution recorded an absent or unset root, and in `blocked()` when the root exists but is unreadable. `launch_check` meets it only in the unreadable case (§3.2).

`blocked()` and `skipped()` together are exactly `problems`; the order inside each keeps today's rule (kind, then expansion order). No row's `args` are read for the split.

`path-through-link` stays ② (b1): the path goes through a link inside the library, which the library never follows (ADR-0365). The row says why it is not loaded; whether the link's target exists is not checked (`store.rs` `FsStore::first_link` stops at the first link component).

Why ② ③ and the command-line checks stay (b1): each describes something that is there but wrong. Running would run with content other than what the file says (a half-read preset, a refused settings value), pick an undefined winner between two skills, or fail at spawn (a command line that does not fit, a working folder the backend refuses). This is the user's rule (「있는데 못 읽음·깨짐 → 막음」) and `CLAUDE.md` 「최소한만 막기」.

### 3.2 The library root (old reason ④)

When a library preset is attached and the root is not `Ok`, the launch looks at which:

**Absent — `RootState::Unset` or `Absent(why)`** (unset; the root folder is missing; the root path is a file — `NotADirectory`, "not usable as a folder", the same as a folder entry):
- The launch does not read the overlay and does not use the catalog (ADR-0361's reason stands: a stale or empty catalog cannot be trusted).
- It expands the agent's presets over a lookup that answers every built-in as today and **every library item as missing** (`AbsentRootLookup` — the name says what step 21 parameterises, §7). So `engram:/default` and every `engram:/…` reference resolve; each attached library preset becomes a `not-found` row; a library item a built-in preset reaches (grammar allows it, nothing shipped does) is missing too.
- `resolve` is split into the ④ guard and the expansion step (`pub(crate)`); the launch calls the expansion step directly. The read entrances keep calling `resolve`, so the problem list and `agent.presets` still show the one ④ row (ADR-0361 decision 1 for screens stands); that row sits in `skipped()`.
- `mark_links` is not run (there is no usable root to probe).
- After that the launch is a normal launch: folder phase, advisory phase (B30 now reads `blocked()` only), command-line and working-folder checks. Unparsed attached spellings come back as ② rows and block, as they do with a usable root.
- The library `not-found` rows of this launch are reported by one notice for the root (§3.3), not one notice each — ADR-0361's 「줄 하나만」 carried over to the notice.

**Exists but unreadable — `RootState::Unreadable(why)`** (the root stat answered an OS error, `timeout`, or `STAT_UNANSWERED`) — user H1:
- The launch is **blocked with today's ④ row unchanged**: `LibraryUnavailable` `{message: why}`, the whole answer — no expansion, no folder IO, no command-line or working-folder row (ADR-0361 decision 1 as it stands for this case; today's early return in `launch_check`). Chosen because it adds no shape: the kind, code and `args` already exist, and the reply is the one line ADR-0361 chose.
- The row's user-facing wording stays short and fixed; `args.message` carries the cause for the log and an LLM caller.

A first scan that could not be applied is `Unreadable` at the read entrances (the root exists, its walk failed); the launch never sees it (§1).

### 3.3 Notices (caller)

Two new codes in agent `launch.rs`, each with its own wording arm in `own_wording` (ADR-0364 「영향」: a code that reports a failure gets its own arm). Facts only — no intended-behavior sentence, no fix (ADR-0357 · ADR-0364). Values are escaped (`escape_controls`). Wording (main decided — §11 b):

| Code | `args` | `line` | Text |
|---|---|---|---|
| `missing-item-skipped` | `{item, via}` — `item` = the row's item (library path or `folder:<path>`), `via` = the row's chain | the row's line (the referencing preset's line), when it has one | library item: `Notice: not found, left out of this launch: <item> (listed in <last of via>, line <n>)`; folder item: `Notice: folder not found or not a folder, left out of this launch: <path> (listed in …)` (the `folder:` tag dropped). Empty `via` (an attached preset itself): `(attached to this agent)`. Without a line, `, line <n>` is dropped. |
| `library-unavailable-skipped` | `{message, items}` — `message` = the absent root's state: `no library root is set`, `missing` or `not a folder`; `items` = the library items left out, in expansion order | none | `Notice: the library folder cannot be used (<message>). Left out of this launch: <items, joined by ", ">` — an empty list ends after the first sentence |

- One `missing-item-skipped` per skipped row; one `library-unavailable-skipped` per launch with an absent root. The skip notices come first in `LaunchPlan.notices`, then the resolution's notices, then the folder and same-name notices.
- Malformed `args` fall back to the generic wording (ADR-0364 decision 2 — accepted, as for the other six).
- **Once per agent** (ADR-0341 decision 5; §11 b U3): the new codes join the notice fingerprint like every other code; an unchanged skip set is not shown again in the reply. The log and the agent note are on every launch.
- `agent.presets` does **not** gain the skip notices: its `problems` rows already show each missing item; the notices are what a launch did.

### 3.4 Telling the launched agent

Channel: the merged instructions — the one channel that reaches claude and codex (§1). No new channel.

- `launch_check` builds one **launch note** from the skip set and appends it as the last part of the merged instructions (after the priming and every source). It is part of the text the check measures, so it cannot turn a skip into a block. Forms, tried in order:
  1. **Named form** — the skipped items, and the root sentence when the absent-root path of §3.2 applied.
  2. If `command_line_fits` refuses the merged text with the named form, the **count form** — one fixed ASCII sentence with the number of skipped items, no path.
  3. If it still refuses, **no note**; the check is judged on the text without a note (today's text).
- **A note never opens the instruction channel by itself on a backend where Engram's instructions replace the user's own** (`instructions_append(mode) == false` — codex). When the merged text without the note is empty (no priming, no source), such a backend gets no note: `developer_instructions` / `developerInstructions` would replace the user's `~/.codex/config.toml` value (ADR-0215). The reply notice and the log still carry the skip. claude (`instructions_append == true`) keeps the note-only case: `--append-system-prompt-file` appends.
- Which form each gets: claude always the named form. codex JSON the named form when Engram has instruction text, else none. codex terminal the named form when the names hold no `' " % \` and fit (a folder path, with backslashes, gives the count form), else none when Engram has no instruction text.
- Note text (main decided — §11 b):
  - named, items from usable places: `Engram launch note: these items your presets reference were not found and were left out of this launch:` then one line per item — `- <item> (listed in <preset>)`, or `- <item> (attached to this agent)` for an attached preset itself; a folder item as `- folder <path> (not found or not a folder; listed in <preset>)`.
  - named, absent root: the sentence `The library folder could not be used (<message>), so these library items were left out: <items>`. The library items are only in this sentence. The line list above is written only when it has at least one row (no header over an empty list).
  - count: `Engram launch note: 1 item your presets reference was not found and was left out of this launch.` / `… <n> items … were not found and were left out of this launch.`
- `prepare_in` writes the same note: the check stores the chosen note text in the plan's `Stamp` (asset-library private, opaque to the manager — no `LaunchPlan` field change), and `prepare_in` passes it to the one merge. A claude plan whose only instruction part is the note still writes and carries `instructions.md`.
- **Dropped note, logged once.** When the named and count forms are refused but the text without a note fits (the note alone caused the refusal), the check records that in the `Stamp`. `prepare_in` — once per launch, after the accepted check — logs it at `warn` (§3.5). When the text without a note is refused too, the launch blocks on the command-line row as today and nothing extra is logged. The ignored checks (`check_before_create`, a raced prelaunch attempt) log nothing.
- The note changes the instructions hash, so codex JSON re-applies its instructions on resume when the skip set changes (the existing `instructions_changed` path) — intended. Because a codex agent never gets a note-only text, a codex JSON thread is never left with a stale note when the skip set empties (with no text, `instructions_text` is `None` and `instructions_changed` stays `false`).
- This is an exception to ADR-0337 decision 3 / ADR-0357 decision 2 ("nothing sent to the started agent"), made by ADR-0374 decision 3, for these two codes only. The other notices still reach only the caller.

### 3.5 Daemon log

`prelaunch::settle` already logs every plan notice once per launch (after the accepted check, not on the discarded pre-create or raced checks). It logs the two skip codes at **`warn`** — abnormal but falling back safely (`logging-conventions.md` levels) — and every other code at `info` as today:

```
tracing::warn!(agent = %id, code = %notice.code, args = %notice.args, line = ?notice.line, "launch item skipped");
```

Identifiers and values are fields; the message is short English. The level choice comes from a pure agent `launch.rs` helper over the agent-owned code constants (no backend knowledge in the manager).

The dropped note (§3.4) is one `warn` in asset-library `prepare_in`, once per launch: `tracing::warn!(agent = %id, epoch, code = %refusal_code, "launch note dropped; command line cannot carry it")`.

A blocked launch (an unreadable root or folder entry included) keeps today's `info` line in `prelaunch::launch_blocked`; the log may carry the cause (`args.why` / `message`) — only the user-facing row wording is kept free of causes (§3.1).

### 3.6 What does not change

- Problem rows, their kinds and codes — except the unreadable folder entry (`folder-unreadable`, ②, §3.1); `ProblemRow` shape; the ④ row on screens. Screens still show a missing line as 「찾을 수 없음」 with relink (ADR-0374 「영향」).
- Attach and create rules (`agent.attachPreset`, `agent.new {presets}` — `library-unavailable` refusal when no first scan was applied, ADR-0386 decision 5): those are edits, not launches. The `preset.add` folder refusal (`edit.rs`, an edit answer) follows the same rule (main decided, 2026-11-20): an unreadable or unanswered folder answers `folder-unreadable` `{path, why}`; an absent folder keeps `folder-not-found` `{path}`. Its `edit.rs` tests that assert the old code for an unreadable folder change in U2.
- `LaunchBlocked` shape. A launch reply never carries a `missing` row, and carries a `library_unavailable` row only for a root that exists but cannot be read or did not answer; both kinds stay in the shape because the problem list uses them.
- Boot restore: an agent whose only problems are skipped rows now starts instead of `RestoreOutcome::Failed`. An agent with a library preset whose root does not answer yet (a network share not up at boot) is still refused, as today.

## 4. Data shapes

- **Wire**: no new type. Two new notice codes inside the existing notice row; their `args` as §3.3. One new problem-row code `folder-unreadable` (`kind: broken`, `args` `{path, why}`); `folder-not-found` loses its `why`. `LaunchBlocked.to_args` keeps its kinds. The doc comments of ts-rs-exported types change (`ProblemRow`, `LaunchNoticeRow`, `AgentPresetsView` — §6), so the agent bindings (`crates/engram-dashboard-agent/bindings/*.ts`) are regenerated; their shapes do not change.
- **Persisted**: the profile's `notice_fingerprint` may now hold the two codes (canonical `code` + `args`, ADR-0341) — no migration; an old fingerprint simply differs once. Nothing else persisted.
- **In memory**: `Stamp` (inside `LaunchPlan.stamp`, asset-library only) gains the chosen note text and the dropped-note fact. `RootState` gains `Absent` / `Unreadable` in place of `Unavailable`; `Resolution` gains the absent-root mark.

## 5. Concurrency

Nothing new. The split and the note are pure computation inside the existing check. A missing item that appears between check and prepare is simply not carried (the next launch carries it). A carried item that disappears between check and prepare is `Stale` → `LaunchRaced` as today; the retry then skips it. The absent-root path takes the same read locks as before and no write lock; the unreadable-root path is today's early return.

## 6. Texts that promise blocking (found and listed)

10a was told its texts must stay true (ADR-0386 decision 11). Checked in code:

| Text | Today | Fix |
|---|---|---|
| `prompts/engram-help.md` agent screen, `agent.spawn` | "notices in a success reply are notices about things left out on purpose" | **False after T-112.** Say: a missing preset item or an unset or missing library folder does not stop the start; it is left out and named in `notices` (once per agent). "If the launch is blocked …" stays. |
| `prompts/engram-help.md` library screen, `agent.presets` | "the folders, conflicts, problems and notices a start would meet" | Say a start leaves out the missing items; a folder entry or library folder it cannot read stops it; no word 「block」 on the library screen (the help test asserts it). |
| `prompts/engram-help.md` library screen, the problems paragraph | "A missing or broken item shows as a row in problems (kind missing · broken · library_unavailable) …" | True. Add one sentence: a start leaves a missing item out and names it in `notices`; a folder entry it cannot read shows as broken (`folder-unreadable`) and stops it. |
| agent `commands.rs` `agent.spawn` declaration doc (in the registration help JSON) | kinds `missing` · `broken` · `conflict` · `library_unavailable`, "Fix the listed items" | Kinds `broken` · `conflict` · `library_unavailable` (a library folder that cannot be read); no `missing`. |
| agent `commands.rs` `launch_blocked_reason` | "(a missing or broken item, a conflict, or no library folder)" | "(a broken or unreadable item, a conflict, or a library folder that cannot be read)". |
| agent `commands.rs` `ProblemRow` doc | "What would block or break a launch … the row `agent.spawn` puts in `args.reasons`" | A problem of the preset; a launch leaves out the `missing` rows and refuses on the others, which are the rows of `args.reasons`. |
| agent `commands.rs` `LaunchNoticeRow` doc | "something the launch leaves out on purpose; not an error, nothing to fix" | Something the launch left out — on purpose, or because it was not found. |
| agent `commands.rs` `AgentPresetsView` `problems` / `notices` docs and the `agent.presets` summary | "Every other problem a launch would meet" · "What a launch would leave out on purpose" · "problems and notices a launch would meet" | Problems as rows (a launch skips the missing ones); notices = what a launch leaves out on purpose (skip notices come only from a launch). |
| asset-library `agent_edit.rs` `AgentPresetsView.problems` | "the rows that would block a launch" | Same sense as above. |
| asset-library `resolve.rs` `FOLDER_NOT_FOUND` doc ("missing, not a folder or could not be checked … plus `why`") | | Missing or not a folder, `{path}`; the new `FOLDER_UNREADABLE` doc carries the `why` list. |
| `builtin/instructions/priming.md` | no blocking claim (only "If an engram command is refused … tell the owner") | None. No priming change (the codex terminal line budget, ADR-0386 「영향」). |
| daemon `control/help.rs` test `the_library_screen_teaches_every_library_command_and_the_edit_rules` doc | "that stops being true when launches skip missing items" | Tense only; add an assert that the agent screen names left-out missing items. |
| asset-library `resolve.rs` header ("the problem list, the tree ⛔ and the launch block come from the same rows"), `Resolution::blocked` doc, `resolve` doc | one list = block | Rows are one list; the launch refuses on `blocked()` only. |
| asset-library `launch.rs` header ("Block ④ is the whole answer — nothing else is checked") | ④ blocks | §3.2 (absent root skips; unreadable root is still the whole answer). |
| agent `launch.rs` `BlockReason` doc ("One reason that blocks a launch"), `Missing` / `LibraryUnavailable` docs, `notice_text` doc (also says "Korean" while the wording is English) | | Rewrite (§2). |
| `docs/reference/structure/asset-library.md` root paragraph ("an agent with a library preset attached is blocked with ④"), Main flow 4, 「Problem rows are the block」, 「④ stands alone」 | | Rewrite (header line). |
| `docs/reference/structure/agent-backend.md` claude argv row | "one `--add-dir <folder>` per folder entry" | "per folder entry the launch did not skip". |
| PRD E4 (four reasons, "④ no library folder"; E4 addendum: an unreadable folder or a share that does not answer is ①), R1 addendum (an unreadable folder entry shows 「폴더를 찾을 수 없음」 and blocks with ①), check line "for each of the four block reasons" | | Not this task — main (§13). |

## 7. Interplay with step 21 (several roots — ADR-0385)

- Nothing here assumes one root beyond what the code already does. The absent-root rule is written as "items of an absent root are missing" (`AbsentRootLookup`); with one root that is every library item. Step 21 makes it per root: a lookup that answers `Missing` for items of the absent roots, one `library-unavailable-skipped` notice per absent root with an added `root` field in `args` (an added field, not a reshape), and a `LibraryUnavailable` block for a root that exists but cannot be read.
- The built-in as an always-present read-only root (step 21) needs nothing from T-112: built-ins already resolve in the absent-root path.
- **Step 21 inherits**: the rule (absent → skip, unreadable → block, per root), the split by kind (`blocked` · `skipped`), `RootState` `Absent` / `Unreadable`, the two codes and their wording, `folder-unreadable`, the agent note with its fallback and the `instructions_append` rule, the `warn` logs, and the read entrances' one-row ④ (step 21 decides whether a screen shows one row per root).
- Nothing T-112 builds is to be removed by step 21.
- **Handed to the client stage (steps 11–13)**: tree ⛔ is 「지금 실행하면 막힐까」 (PRD E4). With `blocked()` now meaning the launch block, an agent whose only problems are skipped rows has rows but no ⛔ (an absent-root ④ read resolution hides other rows, so its `blocked()` is empty even when an unparsed spelling would block a launch). Step 13 confirms that with the user (or adds a lighter mark). Step 13 also gives `folder-unreadable` its short fixed wording ("cannot be read", no cause). The ④ row has one code for an absent and an unreadable root; a screen that must tell them apart reads `library.status` (it already reports the root's state).

## 8. Convention check — "a referenced file is missing → skip and warn"

Light check, sources read directly:
- **git** skips a missing `include.path` file silently and dies on one it cannot access (`config.c` `handle_path_include` → `wrapper.c` `access_or_die`, where `access_error_is_ok` accepts only a missing-file error).
- **OpenSSH** skips an `Include` that matches no file and goes on, with a `debug` line (`readconf.c`, `GLOB_NOMATCH` → "include … matched no files").
- **codex** refuses to start when the one file named by `model_instructions_file` cannot be read (`codex-rs/core/src/config/mod.rs` `try_read_non_empty_file` returns the error).
- **Claude Code**'s memory docs (`code.claude.com/docs/en/memory`, 「Import additional files」) do not say what a missing `@import` does — not confirmed.

Reading: include-style references (a list that pulls in files) skip a missing file — git, OpenSSH; a single file the tool is explicitly told to use fails — codex. A preset's items are include-style. git's split (missing → skip, cannot access → fail) is the user's rule. The skip-and-tell precedent inside Engram is ADR-0363 (codex skill discovery, ripgrep, find, git, walkdir).

## 9. Test plan

QA table rows (`.claude/skill-bindings/qa.md`): 「Rust 소스」 → 전체 회귀 (ts-rs doc comments change, so the binding sync gate runs there too). Lanes: asset-library `src/**` is on both the claude and the codex lane paths, so **both lanes** run; daemon `src/control/**` adds the claude lane only. `manager/prelaunch.rs` is on the launch path (「에이전트 수명」 row → GUI check on).

| What | Checked by | Unit |
|---|---|---|
| Wording and fallback of the two codes (library item, folder item, empty `via`, no line) | agent `launch.rs` golden tests beside the six existing arms | U1 |
| `warn` for the skip codes, `info` for the rest | agent unit test over the level helper | U1 |
| `instructions_append`: claude `true`, codex and the default `false` | agent backend unit tests | U1 |
| Split by kind: every `Missing` row in `skipped()`; every `Broken` (incl. `folder-unreadable`, `path-through-link`) and `Conflict` in `blocked()`; the ④ row in `skipped()` for an absent or unset root and in `blocked()` for an unreadable one; the two parts equal `problems` | `resolve.rs` unit tests; rewrite `the_block_is_the_problem_list_ordered_by_kind` with one missing folder (skipped) and one unreadable folder (blocked, `folder-unreadable`) beside the ② and ③ rows | U2 |
| Folder rows: `Missing` / `NotADirectory` → `Missing` `folder-not-found` `{path}`; `Unreadable(why)` → `Broken` `folder-unreadable` `{path, why}` on every backend and mode | rewrite `a_folder_that_is_not_ok_is_a_missing_line_on_every_backend_and_mode` | U2 |
| Read entrances: the held-folder budget test expects `folder-unreadable` with `why: timeout` | `lib.rs` `resolve_agents_spends_one_read_budget_on_every_held_folder_and_stats_each_once` | U2 |
| A missing instruction, skill, settings, preset, nested preset and folder: the plan launches, the item is not in instructions / skills / raw settings / `folders`, one `missing-item-skipped` each with `via` and `line` | asset-library `launch.rs` tests (claude and codex JSON worlds) | U2 |
| `LaunchPlan.folders` drops exactly the folders of the skipped rows | `launch.rs` test with one missing and one `Ok` folder | U2 |
| Missing + broken → refusal holds only the broken row; missing + conflict likewise | rewrite `a_check_blocks_on_missing_broken_and_conflicting_items_in_that_order` | U2 |
| Absent root (unset, missing, a file at the root path): plan launches with built-ins only, one `library-unavailable-skipped` naming the attached library presets, no catalog read, no `mark_links`; an unparsed attached spelling still blocks | new `launch.rs` tests (from the removed-root and unset halves of `block_four_is_the_whole_answer_and_asks_nothing_else`) | U2 |
| Unreadable root (stat held past the budget → `timeout`; an access error): `LaunchBlocked` `[LibraryUnavailable]` alone, no folder IO, no advisory phase, no library read | `block_four_is_the_whole_answer_and_asks_nothing_else` kept for this case with a held root stat | U2 |
| Unreadable folder entry blocks with one `folder-unreadable` row | `an_unreachable_folder_blocks_the_launch_with_one_line`, `a_held_file_read_in_a_folder_entry_blocks_it_and_the_check_ends_inside_the_budget`, `a_read_whose_worker_failed_blocks_its_folder_like_one_that_did_not_answer` — expected row `Broken` `folder-unreadable` | U2 |
| B30: with only skipped rows the advisory phase runs | rewrite `a_blocked_check_runs_no_advisory_io_and_judges_the_working_folder_as_spelled` to block with a ② row; a new test for skip-only | U2 |
| Lock rule: folder IO and the link probe still run with the library lock released | `folder_io_runs_with_the_library_lock_released` — the first check now passes with a `missing-item-skipped` notice (the link probe still runs before the split) | U2 |
| R4 / R4b keep their purpose with a cause that still blocks | `a_create_with_a_missing_folder_entry_is_blocked_before_the_add` and `a_folder_entry_added_between_the_checks_blocks_after_the_add`: the folder entry's stat is held past the budget (`w.ops.hold` → `Unreadable(timeout)`); the expected row becomes `Broken` `folder-unreadable` (`blocked_folder` matches it); rename to say "unreadable" | U2 |
| Production wiring still reaches the kept service | daemon `src/lib.rs` `the_production_manager_launches_through_the_kept_library_service`: without a root the refusal is not `LaunchBlocked` (P.yaml skipped; the implementer confirms the failure comes from the missing working folder and nothing spawns); after `change_root` to a root holding a `P.yaml` with broken YAML, `LaunchBlocked` holds a `Broken` `P.yaml` row and no `LibraryUnavailable` | U2 (same unit, so U2 ends green) |
| Agent note: named form in `instructions.md` and the hash; codex terminal with a folder path → count form; count form singular / plural; a priming near the limit → no note and no block | `launch.rs` tests over `prepare_in` and the fallback; `builtin.rs` `the_shipped_priming_fits_the_codex_terminal_command_line` extended with the longest count sentence | U2 |
| A claude plan whose only instruction part is the note writes and carries `instructions.md`; a codex plan with no Engram instruction text gets no note and no instruction channel (`instructions_file` `None`) | `launch.rs` tests (claude and codex worlds with the default preset off) | U2 |
| The dropped-note `warn` is emitted once per launch, only when the text without a note fits | a manager-level test over `create_and_activate` (pre-create check + prelaunch) counting the event (tracing test subscriber or a counter seam the implementer picks) = 1; a text refused without the note → 0 | U2 |
| Help screens | daemon `control/help.rs` tests | U3 |
| Live, claude lane: an agent with a missing instruction and a missing folder starts; the model repeats the launch note | new scene L5 in `crates/engram-dashboard-daemon/tests/launch_kits_live.rs` (`claude_l5_…`, one turn) | U3 |
| Live, codex lane: JSON — the rollout developer text holds the named note (`rollout_developer_texts`); terminal — the command line holds the count form (`command_line_holding`) | extend `codex_l3_…` or add `codex_l5_…` | U3 |
| Live, end to end through the CLI: inside a spawned agent (the `engram` CLI needs its token) run `engram agent.spawn --target <agent with a missing preset item>`; the reply has `missing-item-skipped`, a second run has no notice, the daemon log file has the `warn` line, and the launched agent can say what was left out | manual QA step, reported with the reply text | QA |
| GUI: double-click an agent with a missing item in the agent tree — it starts, no block window | qa-verify 「GUI 실측」 | QA |

## 10. Implementation units

Sequential (U2 uses U1's constants and port answer; U3 edits help and tests that read U2's behaviour). Build and tests green at the end of each.

- **U1 — agent**: `launch.rs` two constants, two wording arms, the level helper, `BlockReason` / `notice_text` docs; `backend/settings_check.rs` `instructions_append` (default `false`) and claude's override; `manager/prelaunch.rs` `settle` level. Tests: §9 U1 rows.
- **U2 — asset-library (+ the daemon wiring test)**: `resolve.rs` (`RootState` `Absent` / `Unreadable`, the absent-root mark, `folder-unreadable`, `blocked` · `skipped` by kind, the expansion step without the ④ guard, `AbsentRootLookup`), `lib.rs` (`root_state` / `stat_root`), `launch.rs` (`launch_check` skip, absent-root path, unreadable-root block, `folders` filter, B30 gate, notices, the note with its fallback and the `instructions_append` rule, `Stamp`, `prepare_in` and its dropped-note log), `builtin.rs` test; module headers. Updates every test §1 lists as going red, **including daemon `src/lib.rs` `the_production_manager_launches_through_the_kept_library_service`**, so the unit ends green. Tests: §9 U2 rows.
- **U3 — texts and live**: `prompts/engram-help.md`; agent `commands.rs` docs and `launch_blocked_reason`, regenerated agent bindings; asset-library `agent_edit.rs` doc; `control/help.rs` tests; reference files (header line); live scenes in `launch_kits_live.rs`.

## 11. Decisions

### (a) Decided by the user — ADR-0374 (closed)
- Decision 1: a missing item does not block; settings included, no per-kind exception.
- Decision 2: an unusable root does not block; its items are treated as missing; built-ins resolve. **Narrowed by H1 below** (only an absent root skips).
- Decision 3: each skipped item named in the reply notices, the daemon log and to the launched agent.
- Decision 4: only "missing" changes; ②, ③ and the command-line checks, the notice code and wording, and how the agent is told are this TRD's.
- Decision 5: a separate task after step 9a; current blocks stay until it lands.

### (a2) Decided by the user — 2026-11-20
- **The rule** — for every kind (instruction · skill · settings · preset · folder entry · the library root): absent → skip with a notice; exists but cannot be read, did not answer, or is broken → block. 「ㅇㅇ 같은 기준으로해.」 (on main's table 「없음 → 건너뜀 / 있는데 못 읽음·깨짐 → 막음, 라이브러리 폴더도 같은 기준」).
- U1 — a folder entry that exists but cannot be read, or whose check did not answer in time, **keeps blocking** (option A — recommended): 「ㅇㅇ 막아.」 Context: a network share down or access denied refuses the launch, as today (ADR-0363 decision 3); a missing folder entry is skipped (ADR-0374); an unreadable folder below an entry is already skipped with a notice (ADR-0363).
- H1 — a library root that is set but cannot be read or did not answer **blocks** the launch of an agent with a library preset attached (today's ④ row); only an unset, missing or not-a-folder root skips (§3.2). Under the rule above: 「ㅇㅇ 같은 기준으로해.」
- H3 — the unreadable or unanswered folder-entry row is **its own class**, apart from "not found" (§3.1: ② `folder-unreadable`); its user-facing text names no detailed cause. 「ㅇㅇ 별도로 구분해. 너무 디테일하게 원인을 쓰지는 말고.」

### (b) Main decided (details the user leaves to main — reported)
- U2 — the wording of the two notices (§3.3) and of the agent note (§3.4) is as drafted; facts only (ADR-0357).
- U3 — the skip notices in the reply are **once per agent** (ADR-0341 decision 5 as it stands; a changed skip set shows again). Rejected: every launch — adds a per-code exemption in `PlanNotices::against`, and the user already rejected repeating notices (「알면서 쓰는사람도 있을텐데 그 메세지 실행하면서 계속 보면 안되잖아」). The log and the agent note are on every launch.
- b1 — ②, ③, the command-line and working-folder checks keep blocking (§3.1): they are there but wrong, or the spawn would fail; ADR-0374 decision 4 did not cover them, and the user's rule blocks them.
- b2 — the agent note falls back named → count → none (§3.4). Rejected: tell a codex terminal agent nothing (as for the other terminal notices, ADR-0337 decision 3) — the count is cheap and the note can never turn a skip into a block.
- b3 — a note never opens a replacing instruction channel by itself (codex with no Engram instruction text gets no note; §3.4): it would replace the user's own `developer_instructions` (ADR-0215). claude keeps the note-only case.
- H2 — a folder entry on a drive letter that is not present (unplugged disk, unmapped `subst`): Windows answers `NotFound` → absent → **skipped** (follows from the rule; §3.1). A down UNC share answers `Unreadable` → blocks.

### (c) Internal choices (author decides, reported)
- c1 — the split is by kind over the rows: skip every `Missing`; at the read entrances also the ④ row when the resolution recorded an absent or unset root; block everything else. No `FolderStatus` or `args` reading in the split.
- c2 — the launch's absent-root path calls the expansion step without the ④ guard over `AbsentRootLookup`; `resolve` keeps the guard for the read entrances; an unreadable root keeps today's early return.
- c3 — library rows of an absent root fold into one `library-unavailable-skipped` notice; no per-item notices for them.
- c4 — one `missing-item-skipped` per row (the problem-row grain), not one aggregated notice.
- c5 — the note is the last part of the merged instructions and rides in the asset-library `Stamp`; `LaunchPlan` gains no field.
- c6 — the skip `warn` is emitted in `prelaunch::settle` (once per launch), chosen by an agent helper over agent-owned codes; the dropped-note `warn` in `prepare_in` (once per launch) from the `Stamp`.
- c7 — `agent.presets` does not gain skip notices.
- c8 — `LaunchPlan.folders` drops exactly the folders of the skipped `folder-not-found` rows (matched by `item` == `folder_item(&f.path)`); a folder whose status is unknown (`None`) has no row, stays as today and can never vanish silently.
- c9 — the unreadable-root block reuses today's ④ `LibraryUnavailable` row (no new shape); the unreadable folder entry takes the existing `Broken` kind with one new code `folder-unreadable` (least new shape for a row that had no class of its own). `LaunchBlocked` `Display` needs no change (its `Broken` arm prints the code).
- c10 — `instructions_append` is a port answer with a default of `false` (the safe side: a note alone never opens the channel), overridden by claude.
- c11 — `RootState::Unavailable(why)` splits into `Absent(why)` (`missing`, `not a folder`) and `Unreadable(why)` (OS error, `timeout`, `STAT_UNANSWERED`, a first scan that could not be applied). A root path that is a file is absent ("not usable as a folder"), as for a folder entry.

## 12. Risks

- **R1 (accepted, ADR-0374)** — a missing settings file that carried a permission or sandbox limit lets the agent run without it. The notice, the `warn` line and the agent note are the only signals.
- **R2** — a typo in a preset path now launches. The problem list still shows 「찾을 수 없음」; the reply names it once; the agent is told every launch where the channel allows it.
- **R3** — a removed library folder with many items gives many notices in one reply. Accepted (each is one line; the root case is one notice).
- **R4** — file names from the library enter the agent's instructions. They are escaped and framed as a list of names; same exposure as the source headers `<!-- path -->` today.
- **R5** — the codex terminal agent usually gets the count only (b2).
- **R6** — a codex agent with no Engram instruction text (default preset off, no library instructions) is not told by the note (b3); the reply notice and the `warn` line are its signals. This also keeps a codex JSON thread from holding a stale note on resume.
- **R7** — boot restore before a network share is up: an agent whose library root or folder entry sits on that share is refused (H1, U1), as today; a later start succeeds. A disk that is unplugged (drive letter absent) is skipped instead (H2).
- **R8** — tree ⛔ meaning for agents whose only problems are skipped rows, and the wording of `folder-unreadable`, are left to step 13 (§7).

## 13. Input claims corrected and records for main (brief and ADRs → code at `290cc33b`)

- ADR-0374 「영향」 lists agent `backend/claude` and `backend/codex` as touched. Their launch translations need no change: folders are filtered in asset-library and the note travels in the merged instructions. Only claude's `settings.rs` gains one port answer (b3).
- The real call sites add asset-library `lib.rs` `mark_links` (must run before the split) and `root_state` / `stat_root` (c11), and agent `manager/prelaunch.rs` `settle` (the log line); `edit.rs` and `library_commands.rs` read the variants but need no change.
- **ADR to record (main):** the user's rule (§11 a2) narrows **ADR-0374 decision 2** — only an absent root (unset, missing, not a folder) skips; a root that exists but cannot be read or did not answer blocks. It also changes **ADR-0361**'s "not usable" definition for the launch (decision 1: the whole-answer ④ block now covers only the unreadable root; the read entrances keep one ④ row for both).
- **ADR to record (main):** H3 amends **PRD R1 addendum** (an unreadable folder entry shows 「폴더를 찾을 수 없음」 and blocks with ①), **PRD E4 addendum** (a share that does not answer counts as unreadable ①) and **ADR-0341 decision 6**'s folder exception: the unreadable folder entry is now ② `folder-unreadable`, shown as "cannot be read".
- **ADR-0363 decision 3** — "the folder entry itself still blocks when it cannot be read … and when it is missing: ADR-0341 decision 6 stands for the entry": the unreadable half still holds (as ②); the missing half goes stale (a missing entry is skipped) — main.
- **ADR-0361 「영향」 bullet 2** ("`check_in` must not run folder IO or judge the command line or the working folder for a ④ resolution") still holds for the unreadable-root block; it no longer describes an absent root, which the launch handles as a normal launch (§3.2) — main.
- ADR-0337 decision 3 and ADR-0357 decision 2 say nothing is sent to the started agent; ADR-0374 decision 3 makes an exception for the skip codes. The ADR record should say so (main).
- ADR-0341 decision 5 says the daemon log records omissions on every launch; it does, but at `info`, which a release log never holds (default `warn`, no `RUST_LOG`).
- The `preset.add` folder refusal (`edit.rs`) answered an unreadable folder as `folder-not-found` with `why`; it now answers `folder-unreadable` (§3.6 — main decided: the user's rule covers every surface).
- agent `launch.rs` `notice_text` doc says the wording is Korean; the code is English.

## 14. Review record

_(no review recorded in this file)_
