# TRD — T-112: a launch skips missing items and an absent library root, with a notice (ADR-0374)

| Item | Value |
|---|---|
| **Status** | **Draft 5 · 2026-11-20 — reviewed deep, PASS** (§14). Decisions and boundaries only. The user's rule 「없음 → 건너뜀 / 있는데 못 읽음·깨짐 → 막음」 applies to every kind, the library root included (§11 a2). Canonical ADR: ADR-0374, narrowed by that rule (main records the amendment — §13) (relies on ADR-0361 · ADR-0341 · ADR-0363 · ADR-0357 / ADR-0364 · ADR-0370 · ADR-0215; step-21 context ADR-0385 · ADR-0386) |
| **Parent** | S25 topic ([`trd.md`](trd.md) — §1-3 resolution, §1-5 agent launch). No parent-TRD item of its own: T-112 sits between step 10a and step 21 (`docs/tracking.md` §T-112). What/why = PRD [`prd.md`](prd.md) E4 reasons ① and ④ as amended by ADR-0374 and the user's rule of 2026-11-20. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (the root paragraph 「an agent with a library preset attached is blocked with ④」 · Main flow 4 · 「Problem rows are the block」 · 「④ stands alone」 · Where to look) · `docs/reference/structure/agent-backend.md` (claude argv row: one `--add-dir` per folder entry the launch did not skip) · asset-library `resolve.rs`, `launch.rs` and `folder.rs` module headers · agent `launch.rs` `BlockReason` doc and `notice_text` doc · platform `fs.rs` (the new share probe). `architecture.md` and the crate `lib.rs` heads: none. |
| **Line-pointer basis** | Code at `290cc33b`. Code is cited by file + symbol; line numbers are hints. |
| **Not a source** | Frozen child TRDs (`trd-library-commands.md` · `trd-library-edit.md` · `trd-daemon-wiring.md` · `trd-asset-library.md`) are read only to find why something is so. Code wins over any document. |

## 0. Scope and PRD relation

**The rule (user, 2026-11-20).** For a preset's items (instruction · skill · settings · preset), a folder entry itself and the library root: absent → the launch skips it with a notice; exists but cannot be read, did not answer, or is broken → the launch is blocked. Contents **below** a folder entry follow ADR-0363: what cannot be read there is skipped and named in a notice (§3.1).

**In.**
- A launch no longer refuses for reason ① (missing): an instruction, skill, settings or preset file that is not in the library, and a folder entry whose folder is not there. The item is left out of the launch materials.
- A launch no longer refuses for reason ④ when the library root is **absent** (unset, missing, or a path that is not a folder): every library item the agent reaches is treated as missing; built-ins still resolve.
- A launch **is** refused when the root exists but cannot be read or did not answer (today's ④ row, unchanged), and when a folder entry exists but cannot be read or did not answer (a new ② code — §3.1).
- "Absent" is decided so a network share that does not answer is never absent (§3.1 share probe).
- `library.status` reports an unreadable root apart from a missing one (§3.6).
- Each skipped item is named in the launch reply notices, in the daemon log and, where the instruction channel allows it, to the launched agent (claude or codex) (ADR-0374 decision 3; §3.4).
- The help texts and docs that say otherwise are fixed (§6).

**Out.** Reason ② (exists but broken), ③ (conflict), the command-line and working-folder checks — they keep blocking (§3.1, §11 b1). The screens (the client stage, steps 11–13). Several roots (step 21 — §7). PRD and ADR text edits (main — §13).

**Settings are included** — no per-kind exception (ADR-0374 decision 1; the accepted risk is in §12 R1).

## 1. What the code has today, and every caller this task touches (counted at `290cc33b`)

The problem rows are agent `launch::BlockReason` rows. One list feeds the problem list (`agent.presets`, `preset.get`, `preset.check`), the future tree ⛔ (`LibraryService::resolve_agents`, no production caller yet) and the launch block. **Only `launch_check` turns rows into a refusal.**

Search: `rg` over `crates/`, `src-tauri/src`, `prompts` for each symbol below. `src-tauri/src` and `prompts` hold none of them except `prompts/engram-help.md` (texts — §6).

### 1.1 Production callers

| Symbol (file) | Caller | Disposition |
|---|---|---|
| `BlockReason::Missing` producers | `resolve.rs` `Walk::look` (`not-found`) | unchanged; the row is now skipped at launch (U2) |
| | `resolve.rs` `Resolution::attach_folder_status` (`folder-not-found` for `Missing` / `NotADirectory` / `Unreadable(why)`) | **U2**: `Unreadable(why)` becomes ② `folder-unreadable`; `folder-not-found` keeps `{path}` only |
| `BlockReason::Missing` readers | `lib.rs` `LibraryService::mark_links` | **U2**: a probe error becomes ② `library-read-failed` (§3.1) |
| | `resolve.rs` `Resolution::blocked` | **U2**: launch meaning; new `skipped` |
| | `edit.rs` `preset.check` body (line-0 normalisation) | unchanged (reads every kind) |
| | agent `launch.rs` `LaunchBlocked` `Display` · `to_args` | unchanged (no new kind) |
| | agent `library_commands.rs` `problem_row` | unchanged |
| `BlockReason::LibraryUnavailable` | `resolve.rs` `resolve` (④ guard) | **U2**: split into the guard + the expansion step |
| | `launch.rs` `launch_check` (④ early return) | **U2**: only for `RootState::Unreadable` |
| | `edit.rs`, agent `launch.rs` formatters, `problem_row` | unchanged |
| `RootState` (`resolve.rs`) | `resolve.rs` `resolve` guard | **U2**: `Unavailable` → `Absent` / `Unreadable`; `Resolution` records an absent root |
| | `lib.rs` `LibraryService::root_state` | **U2** (§3.2: typed stat; a failed first scan stats the root) |
| | `agent_edit.rs` `RootSeen::state` | **U2** (same rule as `root_state`; `RootSeen.stat` becomes typed) |
| | `launch.rs` `launch_check` (overlay only when `Ok`) | U2: also picks the absent / unreadable path |
| | `edit.rs` (`RootState::Ok` for `preset.check` drafts), `lib.rs` `resolve_over` (`library == false` → `Ok`) | unchanged |
| `stat_root` (`lib.rs`) | `root_state` | **U2**: answers `FolderStatus` (typed) |
| | `LaunchKitSource::stat_folder` (`launch.rs`, `agent.setWorkspace`) | U2: maps the typed answer to today's strings (`missing`, `not a folder`, the why, `STAT_TIMED_OUT`) — outward unchanged |
| | `stat_new_root` (`change_root` / `set_root` / `first_start`) | U2: maps to today's strings — outward unchanged (`a_first_start_whose_root_stat_is_refused_…` asserts `why.contains("denied")`, stays green) |
| | `status` (`library.status`) | **U2**: `Unreadable` → `RootStatus::Unreadable { why }` (§3.6) |
| | `agent_edit.rs` `LibraryService::agent_edit` (`RootSeen.stat`) | **U2**: stores the typed answer; stats whenever a root is set (also when the first scan failed) |
| `RootStatus` (`lib.rs`) | `status`; `host.rs` `status_ok` (words); `library.setRoot` answer (`LibrarySetRootOk` from `LibraryStatusOk`) | **U2**: new variant `Unreadable { why }`, word `unreadable` |
| `FolderStatus` producers `folder.rs` `status_of` / `StdFolderOps::stat` | `BoundedIo::stat_all` (required phase) | **U2**: unchanged themselves; `stat_all` adds the share probe as a second step of the required phase (§3.1) |
| | `IoAnswer::Canonical` mapping (advisory canonicalize) | unchanged (advisory; never a skip decision) |
| `folder_verdicts` (`edit.rs`) | `preset.add` folders (`edit.rs` ~572), `preset.previewFolder` (`edit.rs` ~910) | **U2**: `Unreadable` → refusal code `folder-unreadable` `{why}`; absent keeps `folder-not-found` `{why: missing \| not a folder}` |
| `status_why` (`edit.rs`) | `folder_verdicts`; `preview_folder` (when listing fails) | U2: moves to `folder.rs` as `FolderStatus::why`, shared by `stat_root`'s string callers; both edit callers convert, `preview_folder` keeps its `unlisted` diagnostic |
| `attach_folder_status` | `lib.rs` `resolve_over`, `settle_resolution`; `launch.rs` `folder_phase` (twice) | U2 (the row change above) |
| `FOLDER_NOT_FOUND` | `resolve.rs` `attach_folder_status`, `edit.rs` `folder_verdicts` | U2 (absent only); new `FOLDER_UNREADABLE` beside it |
| `launch_check`, `Stamp`, `prepare_in` (`launch.rs`) | manager through `LaunchKitSource::check_in` / `prepare_in` | U2 (internal; trait unchanged) |
| `FolderReads::read` (`launch.rs`) | the port's `folder_load_set` at launch | U2: unreadable files below a folder entry are named (§3.1) |
| `prelaunch::settle` (agent) | the one accepted check of a launch | **U1**: `warn` for the skip codes |
| `instructions_append` (new, agent `settings_check.rs`) | asset-library `launch_check` | **U1** (port) · U2 (use) |
| notice codes (new, agent `launch.rs`) | asset-library `launch_check`; agent `own_wording`, the level helper | U1 (codes, wording) · U2 (emitted) |
| platform `fs::share_root` (new, pure) | `folder.rs` `BoundedIo::stat_all` (the library root reaches it through `stat_root` → `stat_all`) | **U2** |
| `library.status` words | agent `commands.rs` `LibraryStatusOk` doc; `prompts/engram-help.md` | **U3** (texts; bindings regenerated) |

### 1.2 Tests that assert a changed symbol

Go red in **U2** and are rewritten there (so U2 ends green):
- asset-library `lib.rs` `a_library_preset_without_a_usable_root_is_block_four_and_the_root_is_stat_ed_only_for_one` — its half after `fail_next_scan` asserts "no root stat" (`stats_of("/lib") == 0`); under §3.2 a failed first scan stats the root once. Rewrite: expect one stat (`Ok` → `Unreadable(join why)`), and add a missing root after a failed scan → `Absent`, ④ in `skipped()`.
- daemon `src/lib.rs` `the_production_manager_launches_through_the_kept_library_service` — real wiring; asserts `[LibraryUnavailable]` without a root and a `Missing` `P.yaml` row after `change_root`.
- asset-library `launch.rs`: `a_check_blocks_on_missing_broken_and_conflicting_items_in_that_order`; `block_four_is_the_whole_answer_and_asks_nothing_else` (a removed root and an unset root — both now skip); `a_blocked_check_runs_no_advisory_io_and_judges_the_working_folder_as_spelled`; `folder_io_runs_with_the_library_lock_released` (expects `[("missing","not-found")]`); `a_create_with_a_missing_folder_entry_is_blocked_before_the_add` (R4); `a_folder_entry_added_between_the_checks_blocks_after_the_add` (R4b); `an_unreachable_folder_blocks_the_launch_with_one_line`; `a_held_file_read_in_a_folder_entry_blocks_it_and_the_check_ends_inside_the_budget`; `a_read_whose_worker_failed_blocks_its_folder_like_one_that_did_not_answer`.
- asset-library `resolve.rs`: `the_block_is_the_problem_list_ordered_by_kind`; `a_folder_that_is_not_ok_is_a_missing_line_on_every_backend_and_mode` (its `Unreadable` case); the ④ tests that build `RootState::Unavailable` (~1579–1619).
- asset-library `lib.rs`: `resolve_agents_spends_one_read_budget_on_every_held_folder_and_stats_each_once` (expects `Missing` `folder-not-found` `why: timeout`); `a_status_answers_the_daemon_id_root_setting_state_config_suggestion_and_generation` (「every stat answer but Ok is missing, with why」 for `denied` → now `Unreadable`).
- asset-library `host.rs` `every_status_word_lands_in_its_own_field` (`Missing { why: "timed out" }` → word `missing`; now `Unreadable` → `unreadable`, plus a `missing` case).
- asset-library `edit.rs` folder-add test at ~3154 (`locked`, `why: denied` → `folder-unreadable`).

Stay green (reason): `lib.rs` `a_configured_root_that_vanished_is_missing_not_recreated_and_the_setting_is_unchanged` and the `Missing { why: "missing" }` cases (~3812, ~3843) — a missing root is still `missing`; `lib.rs` `the_folder_stat_is_bounded_by_one_read_budget_and_names_a_timeout` — `stat_folder` keeps its strings; `edit.rs` ~3151 / ~3152 / ~3502 — absent folders keep `folder-not-found` `why: missing` / `not a folder`; `host.rs` folder views (they carry `FolderStatus` words, not rows); daemon `src/lib.rs` status test (~1422–1469, `RootStatus::Ok`); fake-kit fixtures in agent `commands.rs`, `manager/prelaunch.rs`, `tests/activation.rs`, daemon `connection_core.rs`, `tests/control_agent.rs` (the manager does not read the kind); the `blocked()` tests over other ② rows.

New tests are in §9.

### 1.3 Other facts the design relies on

- `launch_check` B30 gate: the advisory phase runs only when `resolution.problems.is_empty()`.
- `launch_check` asks `root_state(io, root, Ok(()), library_attached)` — it never passes a first-scan failure. The read entrances pass `first_scan_join()`, which re-runs a failed scan on every read; with a configured root that does not exist, `FsStore::scan` fails at the root's metadata, and today `root_state` / `RootSeen::state` answer `Unavailable(join why)` before any stat.
- `stat_root` today flattens the root's `FolderStatus` into `Result<(), String>` (`missing`, `not a folder`, the why); `stat_folder`, `stat_new_root`, `status` and `RootSeen` take that string.
- `folder.rs` `status_of` maps `ErrorKind::NotFound` to `Missing`. **Measured (main, 2026-11-20, rustc 1.95, Windows 11):** `std::fs::metadata` on `\\nohost-zz9-engram\share\x`, `\\localhost\noshare_zz9\x` and `Q:\nofolder` all answer `NotFound` (raw OS error 3) in microseconds — so today a down share reads as absent.
- `LaunchPlan.folders` is every resolved folder, whatever its status. claude turns each into `--add-dir`.
- Notices already travel: `LaunchPlan.notices` → `prelaunch::settle` logs each (`info`, "launch notice") → `PlanNotices::against` (once per agent, ADR-0341 decision 5) → `agent.spawn` reply rows `{code, args, line?, text}` and wire `AgentEvent::Spawned.notices`. ADR-0363's `folder-listing-skipped` uses exactly this path.
- The check runs up to three times per launch (`check_before_create`, then up to `PRELAUNCH_ATTEMPTS = 2`); `settle` and `prepare_in` run once.
- At launch, a file below a folder entry that the port cannot read for a reason other than no answer (`FolderReads::read` `Err(_)`) becomes a port `unsure` row `UnsureWhy::Unreadable`; `launch_check` drops `unsure` — it is neither blocked nor named. Only codex lists folders at launch (`folder_listing_at_launch`; claude answers `false`).
- The only channel that reaches a launched agent is the merged instructions: claude `--append-system-prompt-file` (appends); codex JSON `developerInstructions` and terminal `-c developer_instructions='…'` (replace the user's own `developer_instructions` — ADR-0215). codex treats a text as absent when `text.trim().is_empty()` (`backend/codex/launch.rs` `instructions_text`). It exists only in a real plan (ADR-0370).
- codex terminal: `cmdline::priming_value` refuses `' " % \` and control characters; `cmdline::fits` bounds the length; `SettingsCheck::command_line_fits` answers both. claude answers `Ok` always.
- Default log level is `warn`; a release daemon does not read `RUST_LOG`. The existing "launch notice" line is `info`.

## 2. Module boundaries

| Where | Change |
|---|---|
| platform `fs.rs` | New `share_root(path: &Path) -> Option<PathBuf>` — pure, no stat (only the drive-type query): Windows answers the volume root (`\\server\share\` for a UNC path or `\\?\UNC\…`; `Z:\` for a drive letter whose drive type is remote — `GetDriveTypeW` → `DRIVE_REMOTE`); a local drive letter and other systems answer `None`. |
| agent `launch.rs` | Two notice-code constants and their own wording (§3.3). `BlockReason` doc: a launch never refuses on a `Missing` row; it refuses on a `LibraryUnavailable` row only when the root exists but cannot be read or did not answer; `Missing` = absent, `Broken` = exists but cannot be used. A pure helper telling whether a code reports a launch skip (§3.5). No new type, no `Display` change. |
| agent `backend/settings_check.rs` | Port answer `instructions_append(mode) -> bool` — Engram's instructions are added to what the CLI loads (`true`) or replace a value the user set (`false`). Default `false`; claude overrides with `true`. (§3.4) |
| agent `manager/prelaunch.rs` | `settle` logs the two skip codes at `warn` (§3.5). |
| agent `commands.rs` (+ regenerated `bindings/`) | Doc comments, the `launch_blocked_reason` message, the `library.status` word list (§6). |
| asset-library `folder.rs` | `BoundedIo::stat_all`: after the required stats, for each `Missing` answer whose path has a `share_root`, ask `Stat(share root)` as an ordinary key on the same `LaunchIo` (de-duplicated per launch, same required budget) and combine: root `Ok` → `Missing` stands; root anything else → `Unreadable("share not reachable")`. Advisory phases do not probe. `StdFolderOps::stat`, `list` and `status_of` are unchanged (a folder that vanishes between its stat and its listing is not probed — accepted). `FolderStatus::why` (moved from `edit.rs` `status_why`). |
| asset-library `resolve.rs` | `RootState::Unavailable` → `Absent(why)` / `Unreadable(why)`; `Resolution` records an absent root; `attach_folder_status` gives `folder-unreadable` (new `FOLDER_UNREADABLE`); `blocked` / `skipped`; `resolve` = ④ guard + expansion step. |
| asset-library `lib.rs` | `stat_root` answers `FolderStatus`; `root_state` per §3.2; `status` gives `RootStatus::Unreadable`; `mark_links` probe error → ②; `stat_new_root` maps to today's string. |
| asset-library `agent_edit.rs` | `RootSeen.stat: Option<FolderStatus>`, taken whenever a root is set; `RootSeen::state` = `root_state`'s rule. `AgentPresetsView.problems` doc. |
| asset-library `edit.rs` | `folder_verdicts`: `folder-unreadable` for an unreadable folder; `FolderRefusal` doc. |
| asset-library `launch.rs` | `launch_check` (skip, absent-root path, unreadable-root block, folders filter, B30 gate, notices, unreadable files below a folder entry, the agent note), `prepare_in` (writes the note, logs a dropped note once), `Stamp`, `stat_folder` mapping, `folder_phase` doc. |
| asset-library `host.rs` | `status_ok`: word `unreadable`. |
| daemon | `prompts/engram-help.md` (§6); `control/help.rs` test doc and asserts; `src/lib.rs` wiring test (U2); live scene in `tests/launch_kits_live.rs`. No wiring change. |
| backend `claude` / `codex` launch translation | **No change.** Only claude's `settings.rs` gains the port answer. |
| shell, frontend | **No change.** Notices and status words pass through as strings; the webview shows neither yet (steps 11–13). |

Backend knowledge stays in port answers; OS knowledge stays in `platform` (`CLAUDE.md` 플랫폼 중립).

## 3. Behaviour

### 3.1 What a launch skips and what still blocks

**The split is by kind:** a launch skips exactly the `Missing` rows (and, at the read entrances, a `LibraryUnavailable` row that stands for an absent or unset root); everything else blocks.

`Missing` (absent — skipped):
- `not-found` — a library item (attached preset, nested preset, instruction, skill, settings) not found.
- `folder-not-found` `{path}` — a folder entry whose folder is not there: `FolderStatus::Missing` or `NotADirectory`. Consequence (main decided, H2): a folder entry on a local drive letter that is not present (unplugged disk, unmapped `subst`) is absent and skipped.

**Share probe (decides "absent" for folder entries and the library root).** A `NotFound` stat is `Missing` only when the volume is there. `BoundedIo::stat_all` runs the probe as a second step of the required phase: for each `Missing` answer on a path with a platform `share_root` (a UNC path, or a drive letter mapped to a network location), it asks `Stat(<share root>)` as an ordinary key on the same `LaunchIo`. The probe step runs over every `Missing` answer the required phase returns, including one already cached in `LaunchIo` by an earlier advisory ask of the same path — an unprobed advisory answer never stands for a required one. If the root does not stat as a folder, the item's answer becomes `Unreadable("share not reachable")` and it blocks (「네트워크 끊김 → 막기」). One key per volume root per launch, so absent entries on one share share one probe answer (less load); each absent entry still costs its own stat, then the probe — two round trips unless the root was already asked in the launch. Advisory asks (project-marker stats in `native_roots`) do not probe: there `Missing` and `Unreadable` both mean "no marker". A slow share is bounded by the same required budget (`timeout` → `Unreadable`). The library root gets the probe through the same path: `stat_root` already asks `stat_all` with `Need::Required`, so no separate stat is written in `stat_root`. Not covered: a local-drive path that reaches a share through a directory link (the drive letter is local, so `share_root` is `None`) — R10.

Everything else blocks:
- ② every `Broken` row: YAML syntax, `library-read-failed`, `wrong-kind`, `preset-cycle`, `builtin-in-library-preset`, `item-path-invalid`, settings refusals, skill format and name, `folder-path-cli-unsafe`, `path-through-link`, `command-line:`, `working-folder:`, and the new **`folder-unreadable`**.
- ③ every `Conflict` row.
- `LibraryUnavailable` when the root exists but cannot be read or did not answer (§3.2).

**`folder-unreadable` (new ② code, user H3).** A folder entry whose `FolderStatus` is `Unreadable(why)` — it exists but cannot be read, its stat, listing or a read gave no answer in time, or its share is not reachable — is a `Broken` row with `item` = `folder:<path>` (ADR-0361 item form), code `folder-unreadable`, `args` `{path, why}`, `line` = the entry's line. `folder-not-found` now always has `args` `{path}`. The row is the same at the read entrances and at the launch, so a screen (step 13) shows it as "cannot be read", apart from "not found". **The row's user-facing wording is short and fixed (by code); it names no cause.** The OS error or `timeout` stays in `args.why` for the log and an LLM caller.

**A link probe that fails.** `mark_links` turns a `not-found` row under a link into ② `path-through-link`. When the probe itself errors (`first_link` → `Err`), the row becomes ② `library-read-failed` `{path, why}` (the existing code, `launch.rs` `LIBRARY_READ_FAILED`) instead of staying `not-found`: the path could not be checked, so it is not known to be absent — except `NotADirectory` (a path under a file), which counts as absent like `edit.rs` `admit` (`NotFound | NotADirectory`). The same at the read entrances, and the same rule in `edit.rs` `PresetBodies::mark_row_link` (the per-line `PresetRow.state` of `preset.get` / `preset.check`), so a line's state and the answer's `problems` agree.

**Contents below a folder entry (ADR-0363).** A subfolder or entry that cannot be read is already skipped and named (`folder-listing-skipped`). A *file* the port cannot read for another reason than no answer (codex `unsure` `UnsureWhy::Unreadable`) now joins the same notice: its folder-relative path is added to that folder's `folder-listing-skipped` `paths` (one notice per folder, listing skips first). No new code. A read with no answer still makes the folder `folder-unreadable` (as today it blocked).

`LibraryUnavailable` in the split: at the read entrances the single ④ row is in `skipped()` when the resolution recorded an absent or unset root, and in `blocked()` when the root is unreadable. `launch_check` meets it only in the unreadable case (§3.2).

`blocked()` and `skipped()` together are exactly `problems`; the order inside each keeps today's rule. No row's `args` are read for the split.

`path-through-link` stays ② (b1): the path goes through a link inside the library, which the library never follows (ADR-0365); whether the link's target exists is not checked (`FsStore::first_link` stops at the first link component).

Why ② ③ and the command-line checks stay (b1): each describes something that is there but wrong, or a spawn that would fail. This is the user's rule (「있는데 못 읽음·깨짐 → 막음」) and `CLAUDE.md` 「최소한만 막기」.

### 3.2 The library root (old reason ④)

**The root state, one rule for every producer** (`lib.rs` `root_state` and `agent_edit.rs` `RootSeen::state`; `stat_root` answers a `FolderStatus`, so no string is matched):
- no root → `Unset`;
- the root's stat (taken when a library preset is attached; `RootSeen` takes it whenever a root is set) → `Missing` / `NotADirectory` → `Absent(why)`; `Unreadable(why)` → `Unreadable(why)`;
- the first scan failed (`joined` is `Err`, read entrances only) → the root is stat'ed: `Missing` / `NotADirectory` → `Absent`; `Ok` or `Unreadable` → `Unreadable(join why)`;
- otherwise `Ok`.

A configured root that does not exist is therefore `Absent` both at the read entrances and at the launch. A root that exists but whose first scan failed is `Unreadable` at the read entrances, while the launch (which does not join the first scan) stats it `Ok` and reads the overlay — handed to step 13 (§7).

When a library preset is attached and the root is not `Ok`, the launch looks at which:

**Absent — `Unset` or `Absent(why)`** (unset; missing; the root path is a file — "not usable as a folder", the same as a folder entry):
- No overlay read, no catalog use (ADR-0361's reason stands).
- Expansion over `AbsentRootLookup`: built-ins as today, **every library item missing**. `engram:/default` and every `engram:/…` reference resolve; each attached library preset becomes a `not-found` row.
- `resolve` = the ④ guard + the expansion step (`pub(crate)`); the launch calls the expansion step directly. The read entrances keep calling `resolve`, so screens still show the one ④ row (ADR-0361 decision 1 for screens); it sits in `skipped()`.
- No `mark_links`. Then a normal launch: folder phase, advisory phase (B30 reads `blocked()` only), command-line and working-folder checks. Unparsed attached spellings are ② rows and block.
- One `library-unavailable-skipped` notice for the root (§3.3), not one per item.

**Exists but unreadable — `Unreadable(why)`** (OS error, `timeout`, `STAT_UNANSWERED`, share not reachable) — user H1:
- **Blocked with today's ④ row unchanged**: `LibraryUnavailable` `{message: why}`, the whole answer — no expansion, no folder IO, no command-line or working-folder row (ADR-0361 decision 1 for this case; today's early return). No new shape.
- The row's user-facing wording stays short and fixed; `args.message` carries the cause.

### 3.3 Notices (caller)

Two new codes in agent `launch.rs`, each with its own wording arm in `own_wording` (ADR-0364). Facts only — no intended-behavior sentence, no fix (ADR-0357 · ADR-0364). Values escaped. Wording (main decided — §11 b):

| Code | `args` | `line` | Text |
|---|---|---|---|
| `missing-item-skipped` | `{item, via}` — the row's item (library path or `folder:<path>`) and chain | the row's line, when it has one | library item: `Notice: not found, left out of this launch: <item> (listed in <last of via>, line <n>)`; folder item: `Notice: folder not found or not a folder, left out of this launch: <path> (listed in …)`. Empty `via`: `(attached to this agent)`. No line → `, line <n>` dropped. |
| `library-unavailable-skipped` | `{message, items}` — `message` = `no library root is set`, `missing` or `not a folder`; `items` = library items left out, in expansion order | none | `Notice: the library folder cannot be used (<message>). Left out of this launch: <items, joined by ", ">` — an empty list ends after the first sentence |

- One `missing-item-skipped` per skipped row; one `library-unavailable-skipped` per launch with an absent root. Skip notices first, then the resolution's notices, then the folder and same-name notices.
- Malformed `args` fall back to the generic wording (ADR-0364 decision 2).
- **Once per agent** (ADR-0341 decision 5; §11 b U3). The log and the agent note are on every launch.
- `agent.presets` does not gain the skip notices: its `problems` rows already show each missing item.
- Unreadable files below a folder entry use the existing `folder-listing-skipped` (§3.1).

### 3.4 Telling the launched agent

Channel: the merged instructions (§1.3). No new channel.

- `launch_check` builds one **launch note** from the skip set (the two skip codes) and appends it as the last part of the merged instructions. It is part of the measured text, so it cannot turn a skip into a block. Forms, in order:
  1. **Named form** — the skipped items, and the root sentence when the absent-root path applied.
  2. If `command_line_fits` refuses the merged text with it, the **count form** — one fixed ASCII sentence, no path.
  3. If it still refuses, **no note**; the check is judged on the text without a note.
- **No note opens a replacing channel by itself.** "Engram has no instruction text" uses codex's own blankness: the merged text without the note is `trim()`-empty. Then a backend with `instructions_append(mode) == false` (codex) gets no note — `developer_instructions` / `developerInstructions` would replace the user's `~/.codex/config.toml` value (ADR-0215). The reply notice and the log still carry the skip. claude (`instructions_append == true`) keeps the note-only case.
- claude: always the named form. codex JSON: named form when Engram has instruction text, else none. codex terminal: named form when the names hold no `' " % \` and fit (a folder path gives the count form), else none when Engram has no instruction text.
- Note text (main decided — §11 b):
  - named: `Engram launch note: these items your presets reference were not found and were left out of this launch:` then one line per item — `- <item> (listed in <preset>)`, `- <item> (attached to this agent)`, or `- folder <path> (not found or not a folder; listed in <preset>)`.
  - absent root: `The library folder could not be used (<message>), so these library items were left out: <items>`. Library items appear only here; the line list is written only when it has a row.
  - count: `Engram launch note: 1 item your presets reference was not found and was left out of this launch.` / `… <n> items … were not found and were left out of this launch.`
- `prepare_in` writes the same note from the `Stamp` (asset-library private; `LaunchPlan` gains no field). A claude plan whose only instruction part is the note writes and carries `instructions.md`.
- **Dropped note, logged once.** When both forms are refused but the text without a note fits, the check records it in the `Stamp`; `prepare_in` (once per launch) logs it at `warn` (§3.5). When the text without a note is refused too, the launch blocks on the command-line row as today and nothing extra is logged.
- The note changes the instructions hash, so codex JSON re-applies its instructions on resume when the skip set changes — intended. A codex agent never gets a note-only text, so no codex JSON thread is left with a stale note-only text (§12 R6 for the remaining stale case).
- An exception to ADR-0337 decision 3 / ADR-0357 decision 2, made by ADR-0374 decision 3, for the two skip codes only.

### 3.5 Daemon log

`prelaunch::settle` logs every plan notice once per launch (the accepted check). The two skip codes at **`warn`**, every other code at `info` as today:

```
tracing::warn!(agent = %id, code = %notice.code, args = %notice.args, line = ?notice.line, "launch item skipped");
```

The level choice is a pure agent `launch.rs` helper over agent-owned codes. The dropped note is one `warn` in `prepare_in`: `tracing::warn!(agent = %id, epoch, code = %refusal_code, "launch note dropped; command line cannot carry it")`. A blocked launch keeps today's `info` line (`prelaunch::launch_blocked`); the log may carry the cause — only the user-facing row wording stays free of causes.

### 3.6 What does not change, and what changes outside the launch

- Problem rows, kinds and codes — except `folder-unreadable` (②) and a link-probe error (② `library-read-failed`); `ProblemRow` shape; the ④ row on screens. Screens still show a missing line as 「찾을 수 없음」 with relink.
- Attach and create rules (`agent.attachPreset`, `agent.new {presets}` — `library-unavailable` when no first scan was applied, ADR-0386 decision 5): unchanged.
- **Folder refusals of edits** (`preset.add`, `preset.previewFolder` — `folder_verdicts`, main decided): an absent folder keeps `folder-not-found` with `why` `missing` / `not a folder`; an unreadable or unanswered folder (share not reachable included) answers `folder-unreadable` with `why`.
- **`library.status`** (main decided): `state` = `unset` | `ok` | `missing` | `unreadable` (+ `state_why`). `missing` = the root is absent (missing or not a folder); `unreadable` = it exists but cannot be read or did not answer. The same split the launch uses. `library.setRoot` answers the same fields.
- `agent.setWorkspace` (`stat_folder`) and `library.setRoot`'s refusal (`stat_new_root`) keep their strings; a share that is not reachable now says `share not reachable` instead of `missing`. `share not reachable` becomes an agent-crate constant beside `STAT_TIMED_OUT`, and agent `commands.rs` `workspace_refusal` marks it clearable (`AfterCondition`) like a timeout (ADR-0386 d14).
- `LaunchBlocked` shape. A launch reply never carries a `missing` row, and carries `library_unavailable` only for an unreadable root.
- Boot restore: an agent whose only problems are skipped rows now starts. An agent whose library root or folder entry is on a share that is not up yet is still refused, as today.

## 4. Data shapes

- **Wire**: no new type. Two new notice codes (§3.3). New problem-row and folder-refusal code `folder-unreadable` (`args` / `why` as §3.1, §3.6); `folder-not-found` rows lose their `why`. New `library.status` / `library.setRoot` `state` word `unreadable`. `LaunchBlocked.to_args` keeps its kinds. ts-rs doc comments change (`ProblemRow`, `LaunchNoticeRow`, `AgentPresetsView`, `PreviewRefusalRow`, `LibraryStatusOk` / `LibrarySetRootOk` doc) — agent bindings regenerated; shapes unchanged.
- **Persisted**: the profile's `notice_fingerprint` may hold the new codes — no migration.
- **In memory**: `Stamp` gains the note text and the dropped-note fact. `RootState` `Absent` / `Unreadable`; `Resolution` gains the absent-root mark; `RootStatus::Unreadable { why }`; `RootSeen.stat: Option<FolderStatus>`; `stat_root` → `FolderStatus`.

## 5. Concurrency

Nothing new. The split and the note are pure computation inside the existing check; the share probe is one more ordinary key in the existing required phase (`BoundedIo::stat_all`). A missing item that appears between check and prepare is not carried; a carried item that disappears is `Stale` → `LaunchRaced`, and the retry skips it. The absent-root path takes the same read locks; the unreadable-root path is today's early return. `RootSeen` stats outside every lock, as today.

## 6. Texts that promise blocking or need the new words (found and listed)

| Text | Today | Fix |
|---|---|---|
| `prompts/engram-help.md` agent screen, `agent.spawn` | "notices in a success reply are notices about things left out on purpose" | A missing preset item or an unset or missing library folder does not stop the start; it is left out and named in `notices` (once per agent). |
| `prompts/engram-help.md` library screen, `library.status` | "(root · state · config)" | Name the state words: `unset` · `ok` · `missing` · `unreadable`. |
| `prompts/engram-help.md` library screen, `agent.presets` | "… problems and notices a start would meet" | A start leaves out missing items; a folder entry or library folder it cannot read stops it; no word 「block」 (help test). |
| `prompts/engram-help.md` library screen, problems paragraph | "A missing or broken item shows as a row …" | Add: a start leaves a missing item out and names it in `notices`; a folder entry it cannot read is broken (`folder-unreadable`) and stops it. |
| agent `commands.rs` `agent.spawn` doc | kinds incl. `missing`, "Fix the listed items" | Kinds `broken` · `conflict` · `library_unavailable` (a library folder that cannot be read). |
| agent `commands.rs` `launch_blocked_reason` | "(a missing or broken item, a conflict, or no library folder)" | "(a broken or unreadable item, a conflict, or a library folder that cannot be read)". |
| agent `commands.rs` `ProblemRow` doc | "What would block or break a launch …" | A launch leaves out the `missing` rows and refuses on the others (the rows of `args.reasons`). |
| agent `commands.rs` `LaunchNoticeRow` doc | "left out on purpose; not an error, nothing to fix" | Left out — on purpose, or because it was not found. |
| agent `commands.rs` `AgentPresetsView` `problems` / `notices` docs, `agent.presets` summary | "a launch would meet" · "on purpose" | As above. |
| agent `commands.rs` `PreviewRefusalRow` doc (+ `bindings/PreviewRefusalRow.ts`) | codes `… folder-not-found`; "For `folder-not-found`: missing, not a folder, the OS error or a timeout" | Add `folder-unreadable`; `folder-not-found` = missing or not a folder; `folder-unreadable` = the OS error, a timeout, a share not reachable. |
| agent `commands.rs` `library.status` doc | `state` = `unset` \| `ok` \| `missing` | add `unreadable`. |
| asset-library `edit.rs` `FolderRefusal` doc | "For `folder-not-found`: missing · not a folder · the OS error · a timeout" | As `PreviewRefusalRow`. |
| asset-library `agent_edit.rs` `AgentPresetsView.problems` | "the rows that would block a launch" | As above. |
| asset-library `resolve.rs` `FOLDER_NOT_FOUND` doc, `attach_folder_status` doc ("a ① line for every folder that is not `Ok` … one that exists but cannot be read is ① too") | | Absent → ①; unreadable → ② `folder-unreadable`. |
| asset-library `launch.rs` `folder_phase` doc ("… whose reads gets no answer is a ① line") | | ② `folder-unreadable`; unreadable files below are named (§3.1). |
| asset-library `folder.rs` `StdFolderOps` doc ("an unreadable folder entry blocks"), `status_of` | | Share probe. |
| `lib.rs` `RootStatus` doc ("anything but `Ok` — missing, not a folder, unreadable, timed out") | | Split. |
| daemon `control/help.rs` test doc | tense | Tense; assert the agent screen names left-out items. |
| `resolve.rs` header, `Resolution::blocked` / `resolve` docs; `launch.rs` header; agent `launch.rs` `BlockReason` / `notice_text` docs | | Rewrite (§2). |
| `builtin/instructions/priming.md` | no blocking claim | None. |
| `docs/reference/structure/asset-library.md`, `agent-backend.md` | | Header line. |
| PRD E4 / E4 addendum / R1 addendum / check line | | Main (§13). |

## 7. Interplay with step 21 (several roots — ADR-0385)

- The absent-root rule is "items of an absent root are missing" (`AbsentRootLookup`). Step 21 makes it per root: `Missing` for items of absent roots, one `library-unavailable-skipped` per absent root with an added `root` field, and a `LibraryUnavailable` block for a root that exists but cannot be read.
- Built-ins already resolve on the absent-root path.
- **Step 21 inherits**: the rule per root, the split by kind, `RootState` `Absent` / `Unreadable` and its one producer rule, the typed `stat_root`, the share probe, `RootStatus::Unreadable`, the two codes, `folder-unreadable`, the agent note rules, the `warn` logs, the read entrances' one-row ④.
- Nothing T-112 builds is to be removed by step 21.
- **Handed to the client stage (steps 11–13)**:
  - tree ⛔ 「지금 실행하면 막힐까」: an agent whose only problems are skipped rows has rows but no ⛔ (an absent-root ④ read resolution hides other rows).
  - `folder-unreadable` gets its short fixed wording ("cannot be read", no cause).
  - The ④ row has one code for absent and unreadable roots; a screen tells them apart with `library.status` (`missing` / `unreadable`).
  - A root that exists but whose first scan failed: the read entrances show ④ (blocked), while a launch stats the root `Ok` and runs (§3.2). Step 13 decides how a screen shows that.

## 8. Convention check — "a referenced file is missing → skip and warn"

- **git** skips a missing `include.path` file silently and dies on one it cannot access (`config.c` `handle_path_include` → `wrapper.c` `access_or_die`, where `access_error_is_ok` accepts only a missing-file error).
- **OpenSSH** skips an `Include` that matches no file and goes on, with a `debug` line (`readconf.c`, `GLOB_NOMATCH`).
- **codex** refuses to start when the one file named by `model_instructions_file` cannot be read (`codex-rs/core/src/config/mod.rs` `try_read_non_empty_file`).
- **Claude Code**'s memory docs (`code.claude.com/docs/en/memory`) do not say what a missing `@import` does — not confirmed.

Include-style references skip a missing file (git, OpenSSH); a single named file fails (codex). git's split (missing → skip, cannot access → fail) is the user's rule.

## 9. Test plan

QA rows (`.claude/skill-bindings/qa.md`): 「Rust 소스」 → 전체 회귀 (ts-rs docs change; binding sync gate). Lanes: asset-library `src/**` is on both lane paths → **both lanes**; daemon `src/control/**` adds the claude lane only; `platform/src/shell.rs` is a lane path but `fs.rs` is not. `manager/prelaunch.rs` → 「에이전트 수명」 row → GUI check on.

| What | Checked by | Unit |
|---|---|---|
| Wording and fallback of the two codes | agent `launch.rs` golden tests | U1 |
| `warn` for the skip codes, `info` for the rest | agent unit test over the level helper | U1 |
| `instructions_append`: claude `true`, codex and default `false` | agent backend unit tests | U1 |
| `share_root`: UNC path and `\\?\UNC\` → its `\\server\share\`; local drive letter, absent drive letter, relative path → `None`; on Windows | platform unit tests (pure) | U2 |
| `stat_all` probe: a `Missing` entry under a fake share whose root does not answer → `Unreadable("share not reachable")`; whose root is `Ok` → `Missing`; two entries on one share → one root stat; advisory phase → no probe; advisory then required ask of the same path → the required answer is probed | `folder.rs` tests with a fake `FolderOps` and a `share_root` seam | U2 |
| `StdFolderOps::stat` unchanged: a missing folder on a present drive → `Missing` | `folder.rs` tests (existing) | U2 |
| Split by kind; ④ row placement by the absent mark; parts equal `problems` | `resolve.rs` tests; rewrite `the_block_is_the_problem_list_ordered_by_kind` (one missing and one unreadable folder beside ② and ③) | U2 |
| Folder rows: absent → `folder-not-found` `{path}`; unreadable → `folder-unreadable` `{path, why}` | rewrite `a_folder_that_is_not_ok_is_a_missing_line_on_every_backend_and_mode`; `lib.rs` held-folder budget test expects `folder-unreadable` `why: timeout` | U2 |
| `mark_links` probe error → ② `library-read-failed` | `lib.rs` test with a store whose `first_link` errors | U2 |
| `mark_row_link` follows the same rule: probe error → the line is not `not-found`; `NotADirectory` → absent (both entrances agree) | `edit.rs` test with a store whose `first_link` errors / answers `NotADirectory` | U2 |
| `agent.setWorkspace` refusal with `share not reachable` is clearable (`AfterCondition`) | agent `commands.rs` `workspace_refusal` test | U1 |
| Root state rule: read entrance with a configured root that does not exist → `Absent`, ④ in `skipped()`; a root that exists but whose first scan failed → `Unreadable`, ④ in `blocked()`; `RootSeen::state` equals `root_state` for both (agent-preset edit answer = `agent.presets`, ADR-0384) | new `lib.rs` and `agent_edit.rs` tests; `resolve.rs` ④ tests move to `Absent` | U2 |
| `library.status`: denied → `unreadable` + why; missing → `missing` | `lib.rs` status test; `host.rs` `every_status_word_lands_in_its_own_field` with both words | U2 |
| `stat_folder` and `stat_new_root` keep their strings | `the_folder_stat_is_bounded_by_one_read_budget_and_names_a_timeout` unchanged; one `set_root` refusal test | U2 |
| Folder refusals: `denied` → `folder-unreadable` (`edit.rs` ~3154); missing / not a folder unchanged (~3151, ~3152, ~3502); `preset.previewFolder` the same | `edit.rs` tests | U2 |
| Missing items of each kind: launch, not in materials, one `missing-item-skipped` each | asset-library `launch.rs` tests (claude, codex JSON) | U2 |
| `LaunchPlan.folders` drops exactly the skipped folders | `launch.rs` test | U2 |
| Missing + broken / conflict → refusal holds only the blocking rows | rewrite `a_check_blocks_on_missing_broken_and_conflicting_items_in_that_order` | U2 |
| Absent root (unset, missing, a file): built-ins only, one `library-unavailable-skipped`, no catalog read, no `mark_links`; unparsed spelling still blocks | new tests from `block_four_is_the_whole_answer_and_asks_nothing_else`'s removed-root and unset halves | U2 |
| Unreadable root (held stat → `timeout`; access error): `[LibraryUnavailable]` alone, no folder IO, no advisory phase | `block_four_is_the_whole_answer_and_asks_nothing_else` kept for this case | U2 |
| Unreadable folder entry → one `folder-unreadable` row | `an_unreachable_folder_blocks_the_launch_with_one_line`, `a_held_file_read_in_a_folder_entry_blocks_it_and_the_check_ends_inside_the_budget`, `a_read_whose_worker_failed_blocks_its_folder_like_one_that_did_not_answer` | U2 |
| An unreadable file below a codex folder entry (not a timeout) → named in that folder's `folder-listing-skipped`, launch proceeds | new `launch.rs` test | U2 |
| B30 with skip-only rows; lock rule; R4 / R4b with a held stat (`folder-unreadable`) | rewrites of `a_blocked_check_runs_no_advisory_io_…`, `folder_io_runs_with_the_library_lock_released`, R4, R4b | U2 |
| Production wiring | daemon `src/lib.rs` `the_production_manager_launches_through_the_kept_library_service`: no root → not `LaunchBlocked` (implementer confirms the failure is the missing working folder, nothing spawns); after `change_root` to a root with a broken `P.yaml` → `Broken` `P.yaml`, no `LibraryUnavailable` | U2 |
| Agent note: named / count (singular, plural) / none; never blocks | `launch.rs` tests; `builtin.rs` priming-fits test extended | U2 |
| claude note-only plan writes `instructions.md`; codex with no Engram text gets no note — including default preset **on** with a whitespace-only priming (codex blankness) | `launch.rs` tests | U2 |
| Dropped-note `warn` once per launch, only when the text without a note fits | manager-level test over `create_and_activate` counting the event | U2 |
| Help screens | daemon `control/help.rs` tests | U3 |
| Live, claude lane: missing instruction + missing folder → starts, model repeats the note | new scene L5 in `tests/launch_kits_live.rs` | U3 |
| Live, codex lane: JSON developer text holds the named note; terminal command line the count form | extend `codex_l3_…` or add `codex_l5_…` | U3 |
| Live, CLI end to end inside a spawned agent: `engram agent.spawn --target <agent with a missing item>` → `missing-item-skipped`, none on a second run, `warn` in the daemon log file, the agent can say what was left out | manual QA | QA |
| Live, share probe: a folder entry under `\\<unreachable host>\share\x` blocks with `folder-unreadable`; if feasible, a mapped drive letter whose target is down (`net use`) | manual QA on Windows; result reported (§12 R9) | QA |
| GUI: double-click an agent with a missing item → starts, no block window | qa-verify 「GUI 실측」 | QA |

## 10. Implementation units

Sequential (U2 uses U1's constants and port answer; U3 edits texts and tests that read U2's behaviour). Build and tests green at the end of each.

- **U1 — agent**: `launch.rs` codes, wording, level helper, docs; `backend/settings_check.rs` `instructions_append` and claude's override; `manager/prelaunch.rs` `settle` level; the `share not reachable` constant beside `STAT_TIMED_OUT` and `commands.rs` `workspace_refusal` marking it clearable (+ its test).
- **U2 — platform + asset-library (+ the daemon wiring test)**: platform `fs::share_root`; `folder.rs` (`BoundedIo::stat_all` share probe, `FolderStatus::why`); `resolve.rs` (`RootState`, absent mark, `folder-unreadable`, `blocked` · `skipped`, expansion step, `AbsentRootLookup`); `lib.rs` (typed `stat_root`, `root_state`, `status` / `RootStatus::Unreadable`, `mark_links`, `stat_new_root`); `agent_edit.rs` (`RootSeen`); `edit.rs` (`folder_verdicts`, `preview_folder`, `PresetBodies::mark_row_link`); `host.rs` (`status_ok`); `launch.rs` (`launch_check`, `prepare_in`, `Stamp`, `stat_folder`, unreadable files below a folder entry); `builtin.rs` test; module headers. Every test §1.2 lists as red, **including daemon `src/lib.rs` `the_production_manager_launches_through_the_kept_library_service`**, is rewritten here.
- **U3 — texts and live**: `prompts/engram-help.md`; agent `commands.rs` docs (incl. `PreviewRefusalRow`, `library.status`) and `launch_blocked_reason`, regenerated agent bindings; asset-library `agent_edit.rs` / `edit.rs` / `resolve.rs` / `launch.rs` / `folder.rs` / `lib.rs` doc comments not already rewritten in U2; `control/help.rs` tests; reference files; live scenes.

## 11. Decisions

### (a) Decided by the user — ADR-0374 (closed)
- Decision 1: a missing item does not block; settings included, no per-kind exception.
- Decision 2: an unusable root does not block; its items are treated as missing; built-ins resolve. **Narrowed by H1 below** (only an absent root skips).
- Decision 3: each skipped item named in the reply notices, the daemon log and to the launched agent.
- Decision 4: only "missing" changes; ②, ③ and the command-line checks, the notice code and wording, and how the agent is told are this TRD's.
- Decision 5: a separate task after step 9a; current blocks stay until it lands.

### (a2) Decided by the user — 2026-11-20
- **The rule** — for a preset's items, a folder entry itself and the library root: absent → skip with a notice; exists but cannot be read, did not answer, or is broken → block. 「ㅇㅇ 같은 기준으로해.」 (on main's table 「없음 → 건너뜀 / 있는데 못 읽음·깨짐 → 막음, 라이브러리 폴더도 같은 기준」). Contents below a folder entry stay with ADR-0363 (skip and name — §11 b).
- U1 — a folder entry that exists but cannot be read, or did not answer, **keeps blocking**: 「ㅇㅇ 막아.」
- H1 — a library root that is set but cannot be read or did not answer **blocks** (today's ④ row); only an unset, missing or not-a-folder root skips. 「ㅇㅇ 같은 기준으로해.」
- H3 — the unreadable folder-entry row is **its own class** (② `folder-unreadable`), with no detailed cause in its user-facing text. 「ㅇㅇ 별도로 구분해. 너무 디테일하게 원인을 쓰지는 말고.」

### (b) Main decided (details the user leaves to main — reported)
- U2 — wording of the two notices (§3.3) and the agent note (§3.4); facts only.
- U3 — skip notices once per agent (ADR-0341 decision 5). Rejected: every launch (the user rejected repeating notices — 「알면서 쓰는사람도 있을텐데 그 메세지 실행하면서 계속 보면 안되잖아」).
- b1 — ②, ③, the command-line and working-folder checks keep blocking.
- b2 — the agent note falls back named → count → none. Rejected: tell a codex terminal agent nothing.
- b3 — a note never opens a replacing instruction channel by itself (codex blankness: `trim()`-empty).
- H2 — a folder entry on a local drive letter that is not present → absent → skipped.
- b4 — a network path whose share does not answer is **not** absent (share probe → `Unreadable` → block), after main's measurement that Windows answers `NotFound` for it (「네트워크 끊김 → 막기」).
- b5 — `library.status` gains the word `unreadable` (`RootStatus::Unreadable { why }`).
- b6 — edit folder refusals (`preset.add`, `preset.previewFolder`) answer `folder-unreadable` for an unreadable folder.
- b7 — a link probe that errors is ② `library-read-failed`, not a skipped `not-found`.
- b8 — a file below a folder entry that cannot be read (not a timeout) is skipped and named in `folder-listing-skipped` (ADR-0363), no new code.

### (c) Internal choices (author decides, reported)
- c1 — split by kind: skip every `Missing`; at the read entrances also the ④ row of an absent or unset root; block everything else. No `FolderStatus` or `args` read in the split.
- c2 — the absent-root path calls the expansion step over `AbsentRootLookup`; `resolve` keeps the guard; an unreadable root keeps today's early return.
- c3 — library rows of an absent root fold into one `library-unavailable-skipped`.
- c4 — one `missing-item-skipped` per row.
- c5 — the note rides in the `Stamp`; `LaunchPlan` gains no field.
- c6 — skip `warn` in `prelaunch::settle`; dropped-note `warn` in `prepare_in`.
- c7 — `agent.presets` does not gain skip notices.
- c8 — `LaunchPlan.folders` drops exactly the folders of skipped `folder-not-found` rows (`item` == `folder_item(&f.path)`); a `None`-status folder stays.
- c9 — the unreadable-root block reuses the ④ row; the unreadable folder entry takes `Broken` + one new code; `Display` unchanged.
- c10 — `instructions_append` defaults to `false`; claude overrides.
- c11 — `RootState::Unavailable` → `Absent` / `Unreadable`; a root path that is a file is absent.
- c12 — `stat_root` answers `FolderStatus` (no new type); `FolderStatus::why` gives the strings its other callers keep; `RootSeen.stat` holds the typed answer.
- c13 — the share probe = a pure platform function `fs::share_root` + a second step in `BoundedIo::stat_all`'s required phase asking the share root as an ordinary key (the library root through `stat_root` → `stat_all` — no separate stat); answer `Unreadable("share not reachable")`; `StdFolderOps::stat`, `list` and `status_of` unchanged.

## 12. Risks

- **R1 (accepted, ADR-0374)** — a missing settings file that carried a permission or sandbox limit lets the agent run without it. The notice, the `warn` line and the agent note are the only signals.
- **R2** — a typo in a preset path now launches; the problem list still shows 「찾을 수 없음」; the reply names it once; the agent is told where the channel allows it.
- **R3** — a removed library folder with many items gives many notices in one reply. Accepted.
- **R4** — file names from the library enter the agent's instructions; escaped, framed as names; same exposure as the source headers.
- **R5** — the codex terminal agent usually gets the count only (b2).
- **R6** — a codex agent with no Engram instruction text is not told by the note (b3). Also accepted: a resumed codex JSON thread keeps instructions whose file was since deleted — the text no longer carries it, and with no Engram text at all there is no note and `instructions_changed` stays `false`; the same class as removing an instruction by editing the preset.
- **R7** — boot restore before a network share is up: an agent whose root or folder entry sits on it is refused (H1, U1, b4), as today; a later start succeeds. An unplugged local disk is skipped (H2).
- **R8** — tree ⛔ meaning and the screen form of the first-scan difference are left to step 13 (§7).
- **R9 (unconfirmed)** — what a disconnected mapped network drive letter (`Z:`) answers to a stat (`NotFound` like a UNC path, or error 53 / 67 / 1222), and what a real, existing but offline host answers. The design probes a remote drive letter's root like a UNC share, so either way it is not absent; the QA live check (§9) records what Windows answers, and whether `GetDriveTypeW` (called by `share_root` on the caller's thread, outside a bounded worker) returns without contacting the server for a disconnected mapped drive.
- **R10 (accepted, rare)** — a local-drive path that reaches a network share through a directory link is not probed (the drive letter is local); if the share is down and Windows answers `NotFound`, the item reads as absent and is skipped. Canonicalising every path to find its real volume costs a resolve per item; not done.

## 13. Input claims corrected and records for main (code at `290cc33b`)

- ADR-0374 「영향」 lists agent `backend/claude` and `backend/codex` as touched; their launch translations need no change (claude's `settings.rs` gains one port answer). The real call sites are in §1.1, including `platform` (share probe) and `lib.rs` / `agent_edit.rs` / `host.rs` / `edit.rs`.
- **ADR to record (main):** the user's rule narrows **ADR-0374 decision 2** (only an absent root skips) and changes **ADR-0361**'s "not usable" definition for the launch (the whole-answer ④ block covers only the unreadable root; the read entrances keep one ④ row for both).
- **ADR to record (main):** H3 amends **PRD R1 addendum**, **PRD E4 addendum** and **ADR-0341 decision 6**'s folder exception (unreadable folder entry → ② `folder-unreadable`).
- **ADR to record (main):** b4 (a share that does not answer is unreadable, not absent), b5 (`library.status` `unreadable`), b6 (edit folder refusals), b7 (link-probe error), b8 (unreadable files below a folder entry join `folder-listing-skipped` — extends ADR-0363 decision 2 from listing failures to read failures).
- **ADR-0363 decision 3** — the unreadable half holds (as ②); the missing half goes stale.
- **ADR-0361 「영향」 bullet 2** holds for the unreadable-root block only.
- ADR-0337 decision 3 / ADR-0357 decision 2: ADR-0374 decision 3 makes an exception for the skip codes.
- ADR-0341 decision 5: the per-launch log is `info` today, never in a release log.
- `lib.rs` `RootStatus` doc says "missing" covers unreadable and timed out — true today, false after b5.
- agent `launch.rs` `notice_text` doc says the wording is Korean; the code is English.

## 14. Review record

- 2026-11-20 · deep (lead · risk-axis lens · Codex) · PASS · draft 5 (this commit).
