# TRD — S25 step 10a: library, preset and agent-preset commands (`S25: feat(agent,daemon): 라이브러리 · 프리셋 · 에이전트 프리셋 명령 선언 + 포트(1판 몫)`)

| Item | Value |
|---|---|
| **Status** | **Draft 1 (2026-11-19)** — decisions and boundaries only; awaiting PRD+TRD review. Canonical ADRs: ADR-0155 (declaration next to the producer) · ADR-0307 (LLM command rules) · ADR-0311 (crate edges, the daemon only wires) · ADR-0379 (refusal class → bus code) · ADR-0380 (profile-edit port cell) · ADR-0384 (9a coding-time rules — 10a maps the answers as they are) · ADR-0341 (folder entries, `preset.previewFolder`) · ADR-0358 (whole-file rewrite — the R5 sentence) · ADR-0367 (backend words are strings) · ADR-0377 (root generation inside the change) · ADR-0378 (`rev` · `baseRev`) · ADR-0381 (`library-config-write-failed` is non-retaining). |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 item 10a (with its 「ADR-0341 in 10a」, 「Absorbed into step 20」 and the step-17 / step-8 / step-9a hand-over paragraphs) · §1-5 (profile-edit port, `agent.setWorkspace`, `agent.duplicate`, 「Place and ownership of the command vocabulary」) · §1-7 (daemon command table, 「Common args · answer shapes」, 「Help」, 「CLI」) · §1-8 (wire · generation) · §6-2 (help-topic question). |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Wiring: the `LibraryCommandHost` impl and the daemon table; drop 「Not in the crate yet」; Where to look) · `docs/reference/architecture.md` (agent entry: library / preset / agent-preset declarations; asset-library entry: implements the library command port) · asset-library `src/lib.rs` header (「Refusals」 and a new 「Commands」 paragraph) · agent `commands.rs` header (a third port beside `AgentCommandHost` · `UsageCommandHost`) · daemon `control/commands.rs` header (the table now also plugs the library service) · `prompts/engram-help.md` header (six required sections, not five) · asset-library `Cargo.toml` comment on the `command` dependency (B1). |
| **Line-pointer basis** | HEAD `4a945160`. Code is cited by file + symbol; line numbers are hints. |
| **Not a source** | Frozen TRDs (`trd-library-edit.md` · `trd-create-body.md` (status 「Landed」) · `trd-daemon-wiring.md` · `trd-asset-library.md`) are read only to find why something is so. |

## 0. Scope

**In** — declare every edition-1 library, preset and agent-preset command on the daemon command bus so the `engram` CLI (and so the LLM) and later the screen call the same thing (CLAUDE.md 「LLM-우선 제어」 · ADR-0055 · ADR-0307):

- `library.status` · `setRoot` · `folders` · `refresh` · `list` · `read` · `newFolder` · `newFile` · `relink` (9)
- `preset.get` · `check` · `add` · `remove` · `reorder` · `setBackend` · `save` · `previewFolder` (8 — `previewFolder` from ADR-0341 decision 7)
- `agent.presets` · `attachPreset` · `detachPreset` · `reorderPresets` (4) · `agent.setWorkspace` · `agent.duplicate` (2, bodies in the manager) · `agent.new` gains `presets` (changed)

plus: the port `LibraryCommandHost` and its impl, the daemon wiring (production · test server · smoke bins), regenerated agent bindings, the `engram help library` topic, and the hand-over items (§4.6).

**Out** — edition-2 commands (`library.move` · `rename` · `copy` · `delete` · `trash` · `restore` · `emptyTrash` — step 10b and later); the tree ⛔ read (client step); the webview round trip `agentClient.callCommand` (client step); any shell-only gate (`library.emptyTrash` is 10b — nothing in 10a is shell-only); ADR-0374 (launch skips missing items — tracking T-112, its own task); no change to any 9a body's rules (ADR-0384).

**Already done elsewhere** — the `agent.spawn --cwd` pre-create check and the `agent.spawn` summary rewrite (step 20, `0badd007`); the catalog bump to 9 (step 1 — §8 A2).

## 1. Boundaries and ownership

- **agent owns the vocabulary** — names, args, answer row types, all in the one `declare_commands!` block of `commands.rs` (one block per module — `declare_commands!` doc; `CommandTable::new` takes one declared slice). The handlers live in a new module `library_commands.rs` beside it (the port trait, the verb functions, the refusal → `CommandError` mapping); `make_table` gains a third port argument (B2).
- **The port `LibraryCommandHost`** (agent, name from parent §1-5) — one typed method per library / preset / agent-preset command. Its methods take parsed values (an `AgentId`, never a name — the handler resolves `target` with `commands::resolve`, the same resolver as `agent.spawn`) and answer the declared ok types or a port refusal type agent defines (B1). It also has the read-locked judgement `agent.new {presets}` needs (§4.1).
- **asset-library implements it** — `impl LibraryCommandHost for LibraryService` in a new module `host.rs`: parses spellings with the crate's own helpers (`edit::lib_path` · `edit::parse_ref` · `AssetRef::parse`), calls the 9a body, converts the crate-local answer into the declared row types and `LibraryRefusal` 1:1 into the port refusal. It never calls a second body to rebuild an answer and never reads a file back (ADR-0384 「영향」).
- **The manager** — gains the `agent.setWorkspace` and `agent.duplicate` bodies (parent §1-5); `AgentCommandHost` gains one method for each. No library involvement.
- **The daemon only wires** — `make_daemon_table` takes `Arc<dyn LibraryCommandHost>`: production passes `DaemonWiring.library` (today destructured as `library: _` in the accept-loop function); assemblies without a service (the test server, `saturation_pilot`) pass `NoLibrary` (B8); `roundtrip_smoke` passes its real service and plugs the profile-edit port (§4.6).
- **The CLI does not change** — every new name is reached by the general form `engram <name> --flag value` over `/control/call` (the agent family route `engram agent <verb>` has the fixed list `CLI_AGENT_VERBS`, so the help teaches full names, as it already does for `agent.new`). MCP exposes only the mail tools; commands reach an LLM through the CLI.
- **The help screen** — daemon `control/help.rs` gains `HelpTopic::Library` (section `library`, required); `prompts/engram-help.md` gains the root line and the section. The CLI sends the word as is (the daemon owns the word table).
- **No wire change** — no new wire message, `PROTOCOL_VERSION` unchanged (parent §1-8). No shell or frontend change.

## 2. Command surface (wire)

Field names are snake_case like every existing declaration (`input_id` · `output_format` · `as_of_seq`); the parent's camelCase spellings (`baseRev`, `toFolder`) are placeholders (B5). `root_generation` (optional, `u64`) = the value `library.status` answers; every edit below marked RG carries it (A7). Every new declaration is `#[since(9)]`; none is `#[deadline(Launch)]` (B17).

### 2.1 Library

| Command | Effect | Body | Args | Ok |
|---|---|---|---|---|
| `library.status` | Read | `LibraryService::status` | — | `daemon_id` · `root?` · `root_setting?` (`default`\|`explicit`) · `state` (`unset`\|`ok`\|`missing`) + `why?` · `config` (`ok`\|`read_only`\|`corrupt`\|`unsaved`) + `why?` · `suggested_new` · `root_generation` |
| `library.setRoot` | Write | `set_root` | `path` · `create?` | the status after the change (same row as `library.status`) |
| `library.folders` | Read | `folders` | — | `folders: [{path, name, skill}]` (flat, by path — no recursive type in the macro) |
| `library.refresh` | Read (B7) | `refresh` | `folder?` | `changed` · `problems: [{path, code, args, line?}]` · `no_kind: [path]` · `skipped_links: [path]` · `errors: [{path?, why}]` — the report as is; a refresh never refuses (B19) |
| `library.list` | Read | `list` (+ B11) | `folder?` · `query?` · `kinds?` · `builtins?` | `folder?` · `rows: [ListRow]` · `folders: [{path, name}]` · `builtins: [ListRow]` |
| `library.read` | Read | `read` | `path` | `row: ListRow` · `absolute` · `presets_direct` · `presets_indirect` · `agents_direct: [{id, name}]` · `agents_indirect: [{id, name}]` (B18) · `excerpt?` · `settings_fields` |
| `library.newFolder` | Write | `new_folder` | `parent?` · `name` · RG | `path` |
| `library.newFile` | Write | `new_file` | `folder?` · `name` · `kind` (`Markdown`\|`Settings`\|`Preset`) · RG | `path` |
| `library.relink` | Write | `relink` | `from` · `to` · RG | `fixed: [{from, to}]` · `more` · `written: [path]` · `skipped: [{path, code, args, line?}]` · `stopped?: {path?, code, args, message}` (B9) |

`ListRow` = `path` · `kind` (word — B4) · `name` · `state?: ReasonRow` · `references` · `description`. `ReasonRow` = `{code, args, line?}`.

### 2.2 Preset

| Command | Effect | Body | Args | Ok |
|---|---|---|---|---|
| `preset.get` | Read | `preset_get` | `path` (library path or `engram:/…`) | `PresetView`: `path` · `rev` · `description?` · `presets`/`instructions`/`skills`/`settings: [{path, backend?, line, state?}]` · `folders: [FolderRow]` · `conflicts: [ProblemRow]` · `problems: [ProblemRow]` · `agents` |
| `preset.check` | Read | `preset_check` | `path` · `presets` · `instructions` · `skills` · `settings` · `folders?` | `save_refuses` · `folders` · `conflicts` · `problems` (A5) |
| `preset.add` | Write | `preset_add` | `path` · exactly one of `items: [string]` (+ `backend?` for the call — B6) or `folders: [string]` · `after?` · RG | `{applied, skipped: [{item, code, args, line?}], view: PresetView}` (A6) |
| `preset.remove` | Write | `preset_remove` | `path` · exactly one of `items` or `folders` · RG | `PresetView` |
| `preset.reorder` | Write | `preset_reorder` | `path` · `section` (`presets`\|`instructions`\|`skills`\|`settings`\|`folders`) · `order` · `base_rev` · RG | `PresetView` |
| `preset.setBackend` | Write | `preset_set_backend` | `path` · `items` · `backend?` (omitted = common) · RG | partial answer as `add` |
| `preset.save` | Write | `preset_save` | `path` · `presets` · `instructions` · `skills` · `settings` (each `[{path, backend?}]`) · `folders?` · `base_rev?` · RG | `PresetView` |
| `preset.previewFolder` | Read | `preview_folder` | `folder` · `backend?` · `mode?` (`Terminal`\|`StreamJson`) | `path?` · `refused?: {path, code, why?}` · `loads: [{backend, mode, predicted, set, unread?}]` · `truncated` · `skipped` · `unlisted?` — `set` = `Json` (the agent port's `FolderLoadSet` spread, B4) |

`FolderRow` = `{path, line?, status?}` (`status` word `ok`\|`missing`\|`not_a_directory`\|`unreadable` + `why?`). Backend tags and `backend` args are strings checked by the registry, never the `AgentBackend` enum (A9).

### 2.3 Agent

| Command | Effect | Body | Args | Ok |
|---|---|---|---|---|
| `agent.presets` | Read | `agent_presets` | `target` | `AgentPresetsView`: `agent_id` · `name` · `default_preset` · `presets` · `cwd` · `workspace_lock?` (`running`\|`preparing`\|`conversed`) · `backend` · `mode` · `base_rev` · `takes_presets` · `folders` · `conflicts` · `problems` · `notices: [LaunchNoticeRow]` |
| `agent.attachPreset` | Write | `attach_preset` | `target` · `presets` · `after?` · RG | `{applied, skipped, view: AgentPresetsView}` |
| `agent.detachPreset` | Write | `detach_preset` | `target` · `presets` · `yes?` · RG | `AgentPresetsView` |
| `agent.reorderPresets` | Write | `reorder_presets` | `target` · `order` · `base_rev` · RG | `AgentPresetsView` |
| `agent.new` (changed) | Write | `verb_new` + §4.1 | + `presets?` | + `skipped: [{item, code, args, line?}]` |
| `agent.setWorkspace` | Write | manager (§4.2) | `target` · `cwd` | `agent_id` · `name` · `cwd` |
| `agent.duplicate` | Write | manager (§4.3) | `target` | `agent_id` · `name` · `state` |

`ProblemRow` = the row `agent.spawn` puts in `args.reasons` (agent `LaunchBlocked::to_args`): `kind` (`missing`\|`broken`\|`conflict`\|`library_unavailable`) · `item?` · `via` · `items?: [{item, via}]` · `code` · `args` · `line?` — declared typed with `kind: String`, so the words match the launch block exactly (B4). `LaunchNoticeRow` is the existing declared row.

## 3. Refusal → command error

One function in `library_commands.rs` maps the port refusal (ADR-0379):

| Refusal | Bus code | Retained id? |
|---|---|---|
| `Conflict` | `CONFLICT` | no |
| `NotFound` | `NOT_FOUND` | no |
| `InvalidArgument` | `INVALID_ARGUMENT` | no |
| `Unavailable` | `CONFLICT` | no (incl. `library-config-write-failed` — ADR-0381) |
| `Internal` (`profile-edit-unavailable`) | `INTERNAL` | yes (ADR-0379 decision 5) |
| `Store(StoreError)` | agent `store_command_error` + `store_error_reason(PROFILES_FILE, …)` | as the agent decides (write IO = `INTERNAL`) |

- **Shape** — `CommandError.args` = `{code, args, line?}` of the `Reason` (the same keys as one `args.reasons` row, agent `launch::put_reason`); the message is one readable English line naming the command and the code (B3). A Store refusal carries no args, as every other registry-writing command.
- **Argument errors the handler raises itself** (blank values, `items` and `folders` both or neither, an unparsable `target`) are `INVALID_ARGUMENT` without args, like `reject_blanks`; target resolution failures are the resolver's (`NOT_FOUND` / `CONFLICT` with candidates).
- **Codes per command** — exactly the ones the 9a bodies document (no new refusal codes from 10a except B9 and B12):
  - every library read: `library-unavailable`; `read` · `preset.get`: `not-found`; `preset.get`: the reader's code, `wrong-kind`;
  - `setRoot`: `item-path-invalid` (relative) · `library-config-read-only` · `library-root-unusable` · `library-config-write-failed`;
  - every preset edit (`edit.rs` header): `library-unavailable` · `root-generation-stale` · `item-path-invalid` · `wrong-kind` · `path-through-link` · `not-found` · `library-read-failed` · reader/writer codes (`WriteError::to_reason`) · `library-write-failed`; plus `reorder`: `reorder-order-mismatch` · `rev-mismatch`; `save`: `preset-save-refused` · `rev-mismatch`; folder `add`: `preset-folders-refused`; `setBackend`: `backend-tag-unknown`;
  - `previewFolder`: `backend-tag-unknown`; `check`: `library-unavailable`;
  - agent-preset bodies (`agent_edit.rs` header): `profile-edit-unavailable` · `root-generation-stale` · `not-found` `{agent}` · Store; `attach`: `library-unavailable`; `detach`: `item-path-invalid` · `default-preset-detach`; `reorder`: `reorder-order-mismatch` · `rev-mismatch`;
  - `relink`: `item-path-invalid` · `library-unavailable` · `root-generation-stale` · `not-found` · `path-through-link` · `library-read-failed` · `relink-kind-mismatch` · `profile-edit-unavailable` · a stop before any write = its own refusal;
  - `newFolder` · `newFile`: `item-path-invalid` · `library-unavailable` · `root-generation-stale` · `path-through-link` · `library-read-failed` · `name-exists` · `not-found` · `library-write-failed`.
- **Not refusals** — skips (`skipped[]`), a relink `stopped`, a refresh `errors[]`: answers (ADR-0379 decision 6, ADR-0384 decision 8).
- **Advertised errors** — each declaration lists the codes its refusals can map onto (`errors [NOT_FOUND, CONFLICT]` or a subset); `INVALID_ARGUMENT` · `INTERNAL` come from the table.

## 4. Behaviour

### 4.1 `agent.new {presets}`

- `verb_new` builds the new profile (`new_profile`), then — when `presets` is given — asks the library port to judge it: one read lock over the catalog, the same pure judgement `attach` uses (`agent_edit::attach` over a `CatalogLookup`, the profile outside the registry) → accepted list + skips (G41 partial apply). The profile is created with the accepted list already in `presets` (`create_agent`, one registry write) — no profile-edit-port step after the create (B10; the port doc already says `agent.new` writes a new id outside the port).
- Naming a library preset while no first scan could be applied: refuse the whole call `CONFLICT` `library-unavailable`, nothing created — the attach rule (§12 O3).
- No backend policy change; `llm_creation_refusal` and the store refusals stay as they are. The answer adds `skipped`.

### 4.2 `agent.setWorkspace` (manager body — parent §1-5)

- Normalize the spelling (`normalize_spelling`, as `new_profile`); stat the folder outside every manager lock (std `fs` — B12); missing or not a folder ⇒ refusal `workspace-not-found` `{cwd, why}` (`INVALID_ARGUMENT`).
- Take the agent's `SpawnReservation`; held ⇒ `CONFLICT` `workspace-locked` `{reason: preparing}`. Running or a persisted session id ⇒ `CONFLICT` `workspace-locked` `{reason: running | conversed}` — the same decision function `ProfileEditPort::workspace_lock` answers (so `agent.presets` and this command cannot disagree — parent §1-5).
- One `ProfileRegistry` mutate writes `cwd` (filling an empty `display_name` with the current name first — 「`display_name` pinned」); the reservation drops on return. A store refusal answers as every registry-writing command (`store_command_error`).
- The reservation table records who holds it (launch · working-folder change) so a launch that loses to a working-folder change answers `CONFLICT` 「the working folder is being changed — try again」 instead of today's silent `SpawnOutcome::Moot(None)`; a create verb's after-create activation that loses keeps the step-20 `INTERNAL` + `args.agent_id` path (parent §1-5 · (ga) 1 ②).
- Notifies `RosterChanged` (the tree shows `cwd` — parent §1-8 「confirm at implementation」, B13).

### 4.3 `agent.duplicate` (manager body — parent §1-5 · B13)

- The new name is issued under the name-allocation gate by the existing rule (`<name>(2)` · `(3)` …, no space — `assign_new_name`) and pinned as `display_name`.
- Reading the original and inserting the copy run inside one registry lock, so the copy queues with `rewrite_preset_paths` / `remove_preset_paths` on that lock (a copy cannot slip past a relink).
- Carried: `default_preset` · `presets` · `command` (backend, mode, `extra_args`) · `cwd` · `env`. Not carried: backend session id and history · running state · last-applied instructions hash · notice fingerprint (ADR-0341 decision 5) · last failure. Where the copy sits in the tree (`parent_id`) is the original's parent (§12 O4).
- Store refusal as every registry-writing command; notifies `RosterChanged`. Answer = new id · name · `sleeping`.

### 4.4 Wiring

- `make_table(host, notify, usage, library)`; every new name is inserted with `blocking_handler` (bodies wait on the first-scan entrance and on bounded folder IO).
- Production: `DaemonWiring.library` reaches `make_daemon_table` (the accept loop no longer drops it). Test server and `saturation_pilot`: `NoLibrary`. `roundtrip_smoke`: its own service (§4.6).

### 4.5 Help and priming

- `engram help` root gains one line: `{tool} help library  Library. Instruction · skill · settings · preset files and attaching presets to agents`.
- `## library` section (written in the help file's rules: one line per paragraph, `{tool}`, no emphasis): what the library is; the commands of §2 with their main flags (full names); the settings and preset file formats with an example that carries `description:` and the sentence 「write a one-line description when you create or edit a settings or preset file」 (B12); the folder entry form (`folders:` · absolute paths · not a library asset — ADR-0341); `root_generation` and `base_rev` in one line each; the R5 sentence — an edit through Engram rewrites the whole preset file, hand-written comments and layout are not kept (ADR-0358); how to read `args` on a refusal (`code` · `args`).
- `## agent` gains `agent.new --presets`, `agent.setWorkspace`, `agent.duplicate` and a one-line pointer to `help library` for the agent-preset commands (B15).
- The priming (`builtin/instructions/priming.md`) gains the `engram help library` line in its topic list and the R5 sentence. Scope and language: §12 O1 · O2 (the whole priming is translated to English in its existing format). No launch-wait line (ADR-0371). The shipped priming must still pass the codex terminal command-line check (asset-library `builtin.rs` `the_shipped_priming_fits_the_codex_terminal_command_line`).
- Neither text promises that a missing item blocks a launch (ADR-0374 lands separately — T-112); they say a missing item shows as a problem row.

### 4.6 Hand-over items

| Item | From | 10a |
|---|---|---|
| R5 sentence in the priming and in `help library` | step 17 (child TRD `trd-asset-library.md` §11 R5 · §12) | §4.5 |
| `roundtrip_smoke` plugs the profile-edit port | step 9a | `roundtrip_smoke` calls `LibraryService::plug_profile_edit(Arc::downgrade(&manager))` before `make_daemon_table` and passes the service as the table's library port (the production order — ADR-0380) |
| Map 9a answers as they are; `profile-edit-unavailable` is always a refusal | step 9a · ADR-0384 decision 8 | §3 · §8 A4 · A8 |
| Re-judge T-117's root stat on every agent-preset edit | step 9a · tracking T-117 | kept as is (B16); the line in T-117 is closed at the freeze |
| R4 / R4b live GUI re-run | step 8 (`trd-daemon-wiring.md` B10 · L9) | QA scene G1 (§9) — R4 is reachable live only on the wake path (§11 C5); R4b stays manager-level |
| `agent.new {presets}` validation before `create_agent` | step 20 (`trd-create-body.md` 「Stays in step 10a」) | §4.1 |

## 5. Concurrency

- Library handlers run on the blocking pool (`call_daemon_command` contract). Lock order is the asset-library one (library lock → manager / registry locks, never the reverse — `gate.rs` header, agent `profile_edit.rs`): a handler holds no manager lock when it calls the port.
- `agent.new {presets}`: the judgement's library read lock is released before `create_agent` takes the name-allocation gate and the registry lock — no nesting. Accepted race: a relink or a later delete between the judgement and the create leaves the new agent pointing at the old path, which then shows as missing (relinkable) — the same state a delete after the create gives (§10 R3).
- `agent.setWorkspace`: stat outside every lock; reservation (leaf `spawning` mutex, briefly) → registry mutate → release; no library lock.
- `agent.duplicate`: name gate → one registry lock for read-original + insert; queues with the profile-edit port's rewrites on that lock.
- Agent-preset edits keep 9a's order (port cell → first-scan entrance → root stat outside every lock → one `change_at`).

## 6. User-visible changes

- 23 new commands and `agent.new --presets`, all reachable by `engram <name>` (G20). `engram commands` lists them; `engram help library` exists; `engram help` shows the new line.
- New agents can be created with presets, have their working folder changed and be duplicated from the CLI.
- An agent created with a library preset whose items are missing is blocked at launch (block ①) until T-112 lands — reachable now that a create entrance carries a library preset.
- A launch racing a working-folder change answers `CONFLICT` instead of nothing (§4.2).

## 7. Implementation units (sequential — every unit edits `commands.rs` and regenerates `crates/engram-dashboard-agent/bindings/`)

| Unit | What | Hands to the next |
|---|---|---|
| **U1** port + `library.*` | `LibraryCommandHost` with the 9 library methods and the port refusal type; the refusal mapping (§3); `make_table` / `make_daemon_table` take the port; `NoLibrary`; production wiring; asset-library `host.rs` for the 9; B11 list filters; declarations + bindings + schema; reference-doc wiring lines | the trait, the refusal type and its mapping, the shared rows (`ReasonRow` · `ProblemRow` · `ListRow`) |
| **U2** `preset.*` | the 8 preset methods + impl + declarations (`PresetView` · `FolderRow` · partial answer · `CheckAnswer` · preview rows) | `PresetView`, the partial-answer row |
| **U3** agent-preset + `agent.new {presets}` | the 4 agent-preset methods + impl (`AgentPresetsView`); the read-locked judgement and `verb_new`; `roundtrip_smoke` plug | `AgentPresetsView`, the judgement method |
| **U4** manager bodies | `agent.setWorkspace` · `agent.duplicate` bodies, `AgentCommandHost` methods (+ every fake host), the reservation holder, declarations | — |
| **U5** help · priming · docs | `HelpTopic::Library` + required section; `prompts/engram-help.md`; priming; reference docs; T-117 line | — |

Each unit is green on its own (full regression); U1 already answers through the real service in production.

## 8. Decisions

### 8 (a) — already decided

- **A1** Declared in the agent `declare_commands!` block; asset-library implements the vocabulary through `LibraryCommandHost`; the daemon plugs it with `blocking_handler` — parent §1-5 「Place and ownership」 · ADR-0155 decision 1 · ADR-0311.
- **A2** The catalog stays at 9 — parent §1-8 (bump once per branch in the first step that changes the declarations; step 1 did); `commands.rs` `catalog_version: 9`, master is still 8 and v9 is unreleased (latest tag `v0.4.0`).
- **A3** Class → code, the non-retaining rule, the two `INTERNAL` exceptions and the Store pass-through — ADR-0379 decisions 2–5 · ADR-0381 (`library-config-write-failed`).
- **A4** Answers are mapped as built inside the change; no rebuild after the change, no read-back — ADR-0384 decision 1 and 「영향」.
- **A5** `preset.check` = `{folders, conflicts, problems, save_refuses}` — ADR-0384 decision 3.
- **A6** `preset.add` · `setBackend` share `{applied, skipped, view}`; every preset edit answers the `preset.get` view and every agent-preset edit the `agent.presets` view — ADR-0384 decision 5 · ADR-0379 「영향」 · conventions §1-4.
- **A7** Root generation is optional on every library edit, agent-preset edit, relink and create; not on `setRoot` — M14 · M26 · ADR-0377 · ADR-0384 decision 2 · `set_root` signature.
- **A8** `profile-edit-unavailable` is always a refusal with nothing applied (supersedes step-9a TRD §3.6 「`stopped: {code}` with no `path`」) — ADR-0384 decision 8.
- **A9** Backend words are strings, checked through the registry — ADR-0367.
- **A10** Every operation is a bus command, CLI-callable by the general form; no new wire message — M21 · G20 · parent §1-7 「CLI」 · §1-8.
- **A11** No `agent.launchCheck` — G15 · M21.
- **A12** `agent.new {presets}`: partial apply, validated on the profile to be created before `create_agent`, answer = id + skipped — G41 · Δ7 · parent §1-7.
- **A13** `agent.setWorkspace` · `agent.duplicate` bodies in the manager; what duplicate carries and drops; the name rule — parent §1-5 · B13 · M12 · ADR-0341 decision 5.
- **A14** `agent.setWorkspace`: path must exist and be a folder; locked ⇒ `CONFLICT` with the reason; done under the agent's reservation; `display_name` pinned — parent §1-5 · R49 · ADR-0321.
- **A15** Help is grouped by topic — one `library` line at the root; `help library` carries the file formats, an example with `description:`, the description sentence (B12 · user 2026-10-19), the folder entry form (ADR-0341 in 10a) and the R5 sentence (ADR-0358 · child TRD R5).
- **A16** No 10a command touches an editor draft; the LLM cannot see, save or discard one — G52 · M93.
- **A17** Nothing in 10a is shell-only (`library.emptyTrash` is 10b — M6).
- **A18** The agent argument is resolved by the `agent.spawn` resolver and passed on as an id — parent §1-5 (flow review G17) · conventions §1-2.
- **A19** `preset.save` checks the edition only when `base_rev` is given; `preset.reorder` · `agent.reorderPresets` require it — G53 · M84 · M11 · ADR-0378.
- **A20** Detaching `engram:/default` needs `yes` while the slot is on; raw-spelling detach rules — Δ9 · ADR-0384 decision 7.
- **A21** The help topic contract question (parent §6-2) is answered by code: daemon `control/help.rs` already serves topics (`HelpTopic` · `mail` · `agent` · `window` · `settings`); `library` is one more variant and required section.

### 8 (b) — main decides

- **B1** The port speaks agent types: `LibraryCommandHost` returns the declared ok types or an agent-defined port refusal (`{class, reason}` | `Store(StoreError)`), asset-library converts `LibraryRefusal` into it 1:1, and agent maps it to `CommandError` (§3). — Follows the `LaunchKitSource` precedent (asset-library answers agent's `LaunchBlocked`; agent picks the bus code) and keeps bus codes out of asset-library (ADR-0379 rejected alternative 「asset-library answers bus codes itself」). Consequence: asset-library's `command` dependency stays test-only; its `Cargo.toml` comment is corrected. Alternative: the impl returns `CommandError` directly from `host.rs` (uses the dependency the comment reserved for 10a).
- **B2** Declarations in the existing `commands.rs` block; handlers, the trait and the mapping in a new `library_commands.rs`. — One block per module (macro rule) and one declared slice per table; a second module keeps `commands.rs` (5,900 lines) from growing by the verbs.
- **B3** `CommandError.args` = `{code, args, line?}`; message = `<command>: <code>` plus the key args in English. — Same keys as an `args.reasons` row, so one reader parses both.
- **B4** Rows are typed declarations (`ReasonRow` · `ProblemRow` · `ListRow` · `FolderRow`), with word fields as `String` where the words must equal existing JSON (`ProblemRow.kind`, `ListRow.kind`) and `Json` only for payloads whose keys depend on a code (`args`) or on the agent port (`previewFolder` `set`). — The pattern of `LaunchNoticeRow` and `StoreStatusRow` (word + optional fields); a macro enum would serialize `Missing`, not the launch block's `missing`.
- **B5** snake_case field names; the agent argument is `target` (with `presets`, `order`, `base_rev`, `root_generation`). — Every existing declaration is snake_case; every existing `agent.*` verb names the agent it acts on `target` (one word per kind — conventions §1-2); the parent's `{agent}` is a placeholder.
- **B6** `preset.add` · `remove` take `items: [string]` or `folders: [string]` (both or neither ⇒ `INVALID_ARGUMENT`); `add` items share one optional `backend`; `preset.save` lists are `[{path, backend?}]`. — The common LLM call is one backend per call (conventions §1-1 fewest calls); `save` carries a whole draft.
- **B7** `library.refresh` is `Read`. — It changes only the in-memory catalog and a rerun is harmless; `Write` would make a same-id retry answer `ALREADY_APPLIED` with no report (`command_delivery::retains_the_id`).
- **B8** `NoLibrary` port (daemon `control/commands.rs`, beside `NoUsageLimits`) answers every library command `CONFLICT` `library-unavailable` `{message: "no library service in this assembly"}`. — `NoUsageLimits` precedent; keeps one table shape in every assembly.
- **B9** A relink `stopped` carrying a Store error answers `code: "profile-store-refused"`, `args: {why}` (`why` = `store_error_reason`). — `stopped` must carry the refusal itself (ADR-0384 decision 8) and a store error has no reason code; kebab-case beside the code that answers (ADR-0379 decision 6).
- **B10** `agent.new {presets}` judges under one library read lock and writes the accepted list into the new profile; no port step after the create, so no `INTERNAL`-after-create path. — The port doc (`profile_edit.rs` `set_attachments`) already places `agent.new` outside the port; one registry write; the race is the delete race (§10 R3). Alternative: create bare, then `attach_preset` through the port (parent §1-7's 「race at the edit-port step」 → `INTERNAL` + id).
- **B11** `library.list` gains `query` (search below the folder), `kinds` (omitted or empty = all) and `builtins` (the one `engram:/default` row) in U1 as a small `list` extension. — The parent decided the shape (B1 · Δa · G2); declaring them now avoids changing the declaration in a later step. Alternative: declare only `folder?` now and add the three with the screen step (still catalog 9).
- **B12** `agent.setWorkspace` codes `workspace-not-found` `{cwd, why}` and `workspace-locked` `{reason}`; the folder stat uses std `fs` outside every lock; the reservation table becomes holder-aware. — Codes are main's (ADR-0379 B8 precedent); the manager has no bounded IO; the holder kind is what parent §1-5 asks for.
- **B13** `agent.setWorkspace` and `agent.duplicate` call `RosterChanged`. — The tree shows `cwd` and composition (parent §1-8 「the changed value flows along today's path for refreshing the agent list」).
- **B14** The unit split of §7. — Each group is green alone; shared files force order.
- **B15** Agent-preset commands are documented in `help library`; `help agent` gets the three agent-only additions and a pointer. — The help is grouped by topic (B12); attaching presets is library work.
- **B16** T-117's root stat stays (every agent-preset edit stats the root outside every lock, ≤ `IO_LIMIT` 5 s on an unresponsive share). — Routing adds no lock around it; the cost sits within the ordinary seat and only on a dead share; moving it needs the in-change order change T-117 describes.
- **B17** Every new command is `Ordinary`. — Only launches carry `Launch` (ADR-0369); the first-scan wait is measured in QA (§10 R1).
- **B18** `library.read` agents are `{id, name}`. — The LLM calls by name (conventions §4); the id disambiguates duplicates.
- **B19** `library.refresh` maps the report as is, including a total failure as one `errors` row with no path. — ADR-0384 「map as they are」; the body never refuses.

## 9. Test plan

`.claude/skill-bindings/qa.md` 「돌릴 것 표」 rows: **Rust source** (every unit) → full regression; **claude · codex lanes** — U1–U5 touch `crates/engram-dashboard-asset-library/src/**`, `crates/engram-dashboard-daemon/src/control/**` (claude), `crates/engram-dashboard-daemon/src/lib.rs` and U5 touches `builtin/**` → both lanes; `prompts/engram-help.md` = 「그 밖」 → full regression. Mutation check per unit (qa 「변이 확인」).

| What | How |
|---|---|
| Declarations | agent `tests/command_declarations.rs` (names, effects, `since(9)`, catalog 9); `tests/ts_export.rs` export list + regenerated `bindings/` and `commands.schema.json` (CI ts-rs sync gate); `lint_spec` over every new spec |
| Handler mapping | agent `library_commands.rs` tests over a fake `LibraryCommandHost`: each verb passes parsed args and the resolved id; every refusal class → its code and `args`; Store → `store_command_error`; `items`/`folders` one-of; partial answers carried unchanged |
| Impl mapping | asset-library `host.rs` tests over the existing test `World`: each answer equals the 9a body's answer field by field (no second body call); relink `stopped` carries the refusal; `NoLibrary` answers |
| `agent.new {presets}` | judgement = `attach`'s on an unregistered profile; skipped rows; created profile holds only accepted presets; O3 rule |
| Manager bodies | `agent.setWorkspace`: each lock reason, reservation held, not a folder, `display_name` pinned, launch losing the reservation answers `CONFLICT`; `agent.duplicate`: carried / dropped fields, `(2)` naming, queues with `rewrite_preset_paths` |
| Daemon | `control/commands.rs`: the table contains every new name; production assembly passes the real service (`DaemonWiring.library` used); `roundtrip_smoke` plugs the port before the table (source-order guard like the existing ADR-0380 one) |
| Help · priming | `control/help.rs`: `library` required and rendered, root lists it; CLI `help_file_section` tests; asset-library `the_shipped_priming_fits_the_codex_terminal_command_line` |
| CLI scene C1 (G20) | on the qa 「GUI 실측」 isolated daemon: `engram library.status` → `setRoot --create` → `newFolder` · `newFile --kind Preset` → `preset.add` (items, folders) → `preset.get` · `check` · `save --base_rev` → `relink` → `agent.new --presets` → `agent.presets` · `attachPreset` · `detachPreset` · `reorderPresets` → `agent.setWorkspace` · `agent.duplicate`; each refusal prints `code` + `args` and exits non-zero |
| GUI scene G1 (step-8 hand-over) | create an agent with `engram agent.new --presets` whose preset lists a missing folder entry; start it from the tree → blocked (block ①, nothing started, last failure shown); fix the folder → starts. Valid only while T-112 has not landed |

## 10. Risks

- **R1** A command that waits on the first-scan entrance (a large or slow library) can outlive the ordinary seat and answer `OUTCOME_UNKNOWN` while it keeps running. Measured in C1 on a cold daemon; an edit is safe to retry (a refusal applied nothing; a success holds the id).
- **R2** `commands.rs` and the bindings grow by ~23 declarations and ~50 generated files; review cost. Mitigated by B2 and the unit split.
- **R3** `agent.new {presets}` race with a relink or delete (§5) — the new agent can point at an old path; it shows as missing and is relinkable.
- **R4** The holder-aware reservation (§4.2) touches every launch's reservation take; a wrong branch would change the existing `Moot(None)` answer of a double launch. Pinned by the manager tests and the full regression.
- **R5** The priming is length-bounded by the codex terminal command line; growing it can block every codex terminal launch (O1).
- **R6** ADR-0374 (T-112) changes what a missing item does at launch; texts written now must stay true after it (§4.5) and G1 is valid only before it.
- **R7** Later independent roots (ADR-0312 decision 6 — an added root's paths get a name prefix; an unprefixed path = the current root) are not designed for and no root argument is added, but nothing here blocks them: every path argument and every path field in an answer is a plain string validated by the existing `LibPath` / `AssetRef` rules (a prefix would be a parser extension, not a shape change), omitted `folder` / `parent` means the current root, and root management stays `library.status` + `library.setRoot`. The one single-root-shaped answer is `library.status` (`root?` · `root_generation`) and the one `root_generation` argument: both read as 「the current root」, and per-root values would be added fields, not a change of these. `help library` should say 「the library root」 without claiming there can only ever be one.

## 11. Input claims corrected (parent `trd.md` and the brief → code at `4a945160`)

- **C1** §6-2 「whether `engram help` accepts a topic」 — it does (`HelpTopic` in daemon `control/help.rs`); no contract change, one variant (A21).
- **C2** 「The bodies of `agent.setWorkspace` · `agent.duplicate` stay in the manager」 — neither body exists in the manager yet (`profile.rs` says 「`agent.duplicate` does not exist yet」); 10a builds both (§4.2 · §4.3, U4).
- **C3** `library.list {folder?, query?, kinds?, builtins?}` — the body takes only `folder` and its `builtins` is always empty (B11).
- **C4** §1-7 `agent.new` 「a failure after creation (a race at the edit-port step) is `INTERNAL`」 — the port doc places `agent.new` outside the port; B10 removes that path.
- **C5** Step-8 hand-over 「re-run R4 / R4b live」 — after 10a no live create-and-start entrance carries a library preset (`agent.spawn --cwd` and wire `SpawnByCwd` take no presets; `agent.new` registers without starting), so the pre-create block R4 is reachable live only as a wake block (G1); R4b needs a race and stays manager-level.
- **C6** `docs/reference/structure/asset-library.md` names `LibraryCommandHost` as an impl to come; asset-library's `command` dependency comment says it is used once 10a lands — under B1 it stays test-only (comment corrected).
- **C7** The priming is user-held and Korean (ADR-0383 「Held by the user」), while CLAUDE.md 「언어」 says new text is English — O2.
- **C8** `trd-create-body.md` is 「Landed」 (frozen): used only for why `agent.new {presets}` stayed in 10a.

## 12. Closed — user (2026-11-19)

- **O1 What the priming carries for the library — (a).** The `engram help library` pointer line in the topic list + the R5 sentence + the one-line 「write a description」 rule, in the priming's existing format (user 「현재는 기존 포맷 맞춰서 진행해」). The full formats stay in `help library`.
- **O2 Language — (c) translate the whole priming to English now** (user 「영어로 하고 해당 문서도 나중에 개선하긴 해야되는데 어쨋든 현재는 기존 포맷 맞춰서 진행해」): a translation that keeps the existing structure and lines, plus the O1 lines in English. Improving the priming's content is later (tracking T-118). The priming leaves ADR-0383's held list for its language only.
- **O3 `agent.new --presets` with the library unavailable — (a)** refuse the whole call `CONFLICT` `library-unavailable`, nothing created (main's recommendation, taken on the user's 「진행해」; reopen if the user objects).
- **O4 Where `agent.duplicate` puts the copy — (a)** the same parent as the original (a sibling) (main's recommendation, taken on the user's 「진행해」; reopen if the user objects).

## 13. Review record

(empty)
