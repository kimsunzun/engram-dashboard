# TRD — S25 step 10b: file-operation and trash commands (`S25: feat(agent,daemon): 파일 조작 · 휴지통 명령 선언(2판 몫)`)

| Item | Value |
|---|---|
| **Status** | **Draft 1 (2026-11-24)** — decisions and boundaries only; not reviewed. Two forks wait for the user (§10, D7 · D8). Canonical ADRs: ADR-0322 decisions 1–3 (emptying is human-only, through one shell-only bus command) · ADR-0326 decision 6 (empty-all = what the view listed) · ADR-0408 (emptying removes directly, gone / remaining per entry) · ADR-0409 (9b rules, UD1–UD4, M1–M15) · ADR-0414 · ADR-0415 · ADR-0416 · ADR-0417 (9b coding-time rules — 10b maps their answers as they are) · ADR-0339 (`yes` / `detachRefs`) · ADR-0405 decision 1 (a digest pins what the caller saw) · ADR-0155 (declaration next to the producer) · ADR-0157 (one entrance check) · ADR-0307 (LLM command rules) · ADR-0369 (deadline class) · ADR-0379 (refusal class → bus code) · ADR-0384 (answers mapped as built) · ADR-0386 (10a surface rules this step extends). |
| **Parent TRD** | [`trd.md`](trd.md) §5-3 item 10b · §1-7 (the `library.move` … `library.emptyTrash` rows, 「Only people do this」) · §1-8 (no new wire variant) · §6-5 2 (the screen route, M6). |
| **Step-9b TRD** | [`trd-file-operations.md`](trd-file-operations.md) §0 「Out」 (the line between 9b and 10b) · §3 · §4 「Service answers (10b maps them 1:1 onto the wire)」 and its placeholder codes. |
| **Pattern** | [`trd-library-commands.md`](trd-library-commands.md) (step 10a, Implemented): the port, the host mapping, the refusal function, snake_case fields (its B5), `#[since(9)]`, help topic `library`. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Wiring: the file-command methods of `impl LibraryCommandHost for LibraryService`; the shell-only command) · `docs/reference/architecture.md` (daemon: `call_daemon_command` refuses shell-only names off the socket; control discovery omits them) · asset-library `src/lib.rs` header 「Commands」 paragraph · `ops.rs` / `trash.rs` docs that say `library.trash` (→ `library.listTrash`, D1) · agent `commands.rs` header and the v9 catalog comment · daemon `control/commands.rs` header (a third check beside arguments and the lease) and `control/catalog.rs` header (the list omits shell-only names) · `prompts/engram-help.md`. |
| **Line-pointer basis** | Code at `c58ea68d` plus the uncommitted 9b U5 revision in the working tree (`LibraryService::empty_trash`, `trash.rs`). **U5 is being revised now: `empty_trash`'s signature and answer may still change**; U2 below starts after U5 is committed. Code is cited by file + symbol. |
| **Not a source** | Frozen TRDs (`trd-library-commands.md`, `trd-library-mounts.md` …) are read only to find why something is so. Where the parent §1-7 rows disagree with ADR-0408, ADR-0409, the 9b TRD or the code, those win (§12). |

## 0. Scope

**In** — every name an LLM or the shell calls for the 9b service, on the daemon command bus (CLAUDE.md 「LLM-우선 제어」 · ADR-0307):

- LLM and shell: `library.move` · `library.rename` · `library.copy` · `library.delete` · `library.listTrash` (the trash view — the parent's placeholder `library.trash`, D1) · `library.restore` (6).
- Shell only: `library.emptyTrash` (empty-all and permanent delete — ADR-0322 decisions 1–3 · ADR-0408 decision 4): declared beside the others, refused by the daemon when it arrives off the socket, absent from the control route's command list, `engram help` and the priming (§4).
- Seven new `LibraryCommandHost` methods, their `host.rs` mapping, the daemon's `NoLibrary` answers, regenerated agent bindings and `commands.schema.json`, the `engram help library` additions.

**Out** — screens (trash view, modals, drag — client stage); the webview round trip and how the screen sends `library.emptyTrash` (step 11 / 13 — hand-over §4.4); any service behaviour change (9b and its pending decisions, §7); a progress record (G38); automatic emptying (ADR-0322 decision 8); the large-answer file hand-off (ADR-0307 §2-4, undecided); no new wire message, `PROTOCOL_VERSION` unchanged (parent §1-8).

**Catalog** — stays 9 (the same unreleased generation as 10a and step 21); the v9 comment gains one line.

## 1. Boundaries and ownership

- **agent owns the vocabulary** — the seven names, their args and answer rows go into the one `declare_commands!` block of `commands.rs` (one block per module); the handlers and port methods into `library_commands.rs`, as 10a. `make_table` keeps its signature.
- **The port speaks agent types.** Each new `LibraryCommandHost` method takes the spellings the caller typed (paths as strings, `on_conflict` as the declared word, trash addresses as declared rows) and answers declared rows or a `PortRefusal`. Agents cross the port by id only; the handler names them from one `AgentCommandHost::roster` read and drops an id the roster no longer holds (10a B18). The `CONFLICT` plan's agents already carry names from the service and pass through untouched.
- **asset-library maps, never re-runs** — `host.rs` calls the one 9b method per command and converts its answer field by field; no second body call, no read-back (ADR-0384). Refusals keep their class and code 1:1 (`LibraryRefusal` → `PortRefusal`).
- **The daemon only wires — plus one gate.** `NoLibrary` answers every new name `CONFLICT` `library-unavailable` (as 10a). `call_daemon_command` gains the shell-only check (§4); the control route's command list omits shell-only names. No other daemon file changes.
- **The CLI does not change** — every name is reached by the general form `engram <name> --flag value` over `/control/call`.
- **No shell or frontend change** in 10b.

## 2. Command surface (wire)

Field names are snake_case (10a B5). Every declaration is `#[since(9)]`; every write takes an optional `root_generation` (RG — ADR-0377). Deadline class: D8. All six writes are `Effect::Write` (the delivery seat keeps the request id against a double apply); `library.listTrash` is `Read`.

### 2.1 Commands

| Command | Body | Args | Ok |
|---|---|---|---|
| `library.move` | `move_items` | `paths: [string]` · `to_folder` · `on_conflict?` · `yes?` · `detach_refs?` · `plan_digest?` · RG | `MoveOk` |
| `library.rename` | `rename` | `path` · `name` · `yes?` · `detach_refs?` · `plan_digest?` · RG | `MoveOk` (one item) |
| `library.copy` | `copy_items` | `paths` · `to_folder` · `on_conflict?` · RG | `items: [CopyItemRow]` |
| `library.delete` | `delete` | `paths` · `yes?` · `detach_refs?` · `plan_digest?` · RG | `items: [DeleteItemRow]` · `detached` · `written` · `skipped: [PathReasonRow]` · `agents: [AgentRefRow]` · `stopped?: RelinkStopRow` |
| `library.listTrash` | `trash_view` | — | `mounts: [MountTrashRow]` |
| `library.restore` | `restore` | `rows: [TrashRowArg]` · `keep_both?` · RG | `items: [RestoreItemRow]` |
| `library.emptyTrash` (shell only) | `empty_trash` | `bundles?: [{mount, bundle, rows: [string]}]` · `rows?: [TrashRowArg]` | `rows: [EmptiedRow]` · `bundles: [EmptiedBundleRow]` |

- `on_conflict` = a macro enum `Overwrite` | `Skip` | `KeepBoth` (typed words are case-folded to it, as `kinds`); omitted = that item's conflict (M26). `yes` / `detach_refs` default false. `keep_both` default false (a taken name is that row's conflict; with it the row returns as `name (2)` … — G11).
- `MoveOk` = `items: [MoveItemRow]` · `mapping: [{from, to, exact}]` in move order (ADR-0415 decision 4 — apply in order; `exact` = only `from` itself maps) · `detached` · `written` · `skipped: [PathReasonRow]` · `trees: [{mount, git: bool|null, top: string|null}]` (the git working trees the written and skipped files lie in, once per `(mount, tree)`; `git` null = could not be told, as `library.renameMount`'s `git`) · `agents: [AgentRefRow]` · `stopped?: RelinkStopRow`.
- `TrashRowArg` / `TrashAddressRow` = `{mount, bundle, path}` — `path` is the row's path inside its mount, as the view gives it (ADR-0409 M5: every address carries its mount).

### 2.2 Per-item rows

One flat row per given path or trash row, in the order given (ADR-0409 per-item results): `status` = `done` | `skipped` | `conflict` | `failed`, then the fields of that status. A non-`done` row carries `code` · `args` · `line?` — the `Reason` as built, so the staging fields of ADR-0416 decision 4 (`staging` · `staging_disk` · `staging_row` · `staging_kept` · `not_copied`) reach the caller inside `args` unchanged.

| Row | Key | `done` fields |
|---|---|---|
| `MoveItemRow` | `path` | `to` · `trashed: [TrashAddressRow]` (an overwrite's displaced entries) · `original?: TrashAddressRow` (a cross-volume original) · `left: [PathReasonRow]` (a merge's entries left, ADR-0415) |
| `CopyItemRow` | `path` | `to` · `trashed: [TrashAddressRow]` (0 or 1 — a list as on move, D13) |
| `DeleteItemRow` | `path` | `trashed: TrashAddressRow` |
| `RestoreItemRow` | `mount` · `bundle` · `path` | `to` (the virtual path it is back at) |

`skipped` codes (the four `Skip` variants — D3): `inside-given-folder` `{folder}` (a folder above it was given in the same call — P5) · `in-place` `{}` (a move into its own folder, a rename to its own spelling) · `name-exists` `{path}` (taken, `on_conflict` `Skip`) · `name-taken-during-copy` `{path, staging…}` (`Skip::TakenDuringCopy`).

### 2.3 Trash view and emptying rows

- `MountTrashRow` = `mount` · `problem?: {code, args}` (the mount is named, not failed: `trash-unusable` `{mount, why}` or `library-unavailable` `{mount, message}`) · `bundles: [{bundle, deleted_at, source, why?, trouble?, rows: [TrashViewRow]}]` by id. `deleted_at` = RFC 3339 UTC from the id (`2026-11-23T08:15:00Z`). `source` = `record` | `no_record` | `record_unreadable` (`why`) | `unchecked` (no rows; `trouble` says why).
- `TrashViewRow` = `path` · `original` (the virtual path a restore returns it to — `MountTrash::original`) · `kind` = `root` | `found` | `other` · `state` = `present` (with `entry` = `file` | `folder` | `link`) | `through_link` | `failed` (`why`) · `staging` · `intended?` (virtual path) · `restorable` = kind is not `other`, not staging, state `present` (a derived flag so the LLM does not try a restore that answers `not-restorable` — D11).
- `EmptiedRow` = `mount` · `bundle` · `path` · `outcome` = `gone` | `remaining` | `not_found` · `code?` · `args?` (a remaining row's reason; the OS error code is a diagnostic inside `args` — ADR-0408 decision 2). `EmptiedBundleRow` = `mount` · `bundle` · `outcome` (same words) · `untouched?: {code, args}`.

### 2.4 Refusals of the whole call

The 10a refusal function maps every port refusal (ADR-0379; `CommandError.args` = `{code, args, line?}`). The whole-call refusals are exactly the ones the 9b method docs list — nothing is added by 10b except `shell-only` (§4, if D7 (b)):

- every write: `library-unavailable` (no first scan applied — `Unavailable` → `CONFLICT`, retry after condition) · `root-generation-stale` (`CONFLICT`);
- `move` / `copy`: `to_folder` not a library path, or a dot or temp name in it → `INVALID_ARGUMENT` `item-path-invalid`; in the built-in → `CONFLICT` `mount-read-only`; `rename`: `name` not one valid name → `INVALID_ARGUMENT` `item-path-invalid`;
- `delete` → `CONFLICT` `delete-unconfirmed`; `move` / `rename` → `CONFLICT` `skill-unconfirmed`, `INVALID_ARGUMENT` `detach-refs-invalid` (§3); with agents to change while the agent store is read-only → the store's refusal; the port cell empty → `INTERNAL` `profile-edit-unavailable` (keeps the id — ADR-0379 decision 5);
- `emptyTrash`: neither `bundles` nor `rows` → `INVALID_ARGUMENT` `nothing-to-empty` (M86 — whoever calls).

**Final codes (D4).** The 9b placeholders are final as built: `into-itself` · `kind-mismatch` · `links-inside` · `source-changed` · `target-mount-changed` · `crosses-volumes` · `copy-merge-refused` · `trash-unusable` · `trash-not-reserved` · `path-through-link` · `not-restorable` · `inside-root` · `record-unreadable` · `no-replace-unsupported` · `delete-unconfirmed` · `skill-unconfirmed` · `detach-refs-invalid` · `nothing-to-empty`; reused `name-exists` · `not-found` · `virtual-root-read-only` · `mount-read-only` · `library-unavailable` · `library-read-failed` · `library-write-failed` · `root-generation-stale`. New in 10b: the four skip codes (§2.2) and `shell-only` (D7 (b)). `other` is a view row kind, not a code (§12 C7).

## 3. Confirmation flow over the wire (UD2)

1. A delete of referenced items, a rename or move that takes a referenced skill's lone `SKILL.md` out of its folder, or a merge that would leave a referenced skill folder without its `SKILL.md` (ADR-0415 decision 2) answers `CONFLICT` `delete-unconfirmed` / `skill-unconfirmed`; nothing is applied. `args.args` = `plan` (`presets` · `agents: [{id, name}]`) · `plan_digest` · `forms` — `keep` `{yes: true}` and `detach` `{yes: true, detach_refs: true, plan_digest}`. **The forms are spelled with the wire names**: `DeletePlan::to_json` writes `detachRefs` today and changes to `detach_refs` in U1 (one spelling at the source — D5; a test feeds `forms.detach` back as the arguments of the same command and it passes the entrance check).
2. The caller repeats the same call with `--yes true` (references kept — they read as not found until a restore) or `--yes true --detach_refs true --plan_digest <d>` (references removed). Plain `yes` carries no digest (UD2). `detach_refs` without `yes` is no `yes` (ADR-0339 M108). `detach_refs` on a move or rename with no such item → `INVALID_ARGUMENT` `detach-refs-invalid`.
3. A plan that changed — at the call, or inside the hold before any disk change (ADR-0405 decision 1 · ADR-0409 decision 2) — answers the same `CONFLICT` with the fresh plan and digest, nothing written. The delete digest covers the items too (ADR-0414 decision 3), so a digest for `[A]` does not confirm `[A, B]`.
4. A whole-call refusal met inside the hold after cross-volume copies also names the stagings (`args.args.stagings` — ADR-0416 decision 4).
5. The screen uses the same three steps (its modal shows `plan` and sends the chosen form) — there is no second confirmation path. Moves and renames otherwise ask nothing (UD4).

## 4. `library.emptyTrash` — shell only

### 4.1 Where it is declared

In the same `declare_commands!` block as the other library commands, so the shell finds it in WS `ListCommands`, in the bindings and in `commands.schema.json`, and the socket's stage 1 runs it. A new list `SHELL_ONLY` in agent `commands.rs` names it — the same pattern as `INPUT_AFFECTING` (a const list of declared `Write` names that the daemon's one entrance checks; a test pins that every listed name is declared and `Write`). No second table and no second `declare_commands!` block (one block per module; a second table would need its own catalog and still need the entrance check). Main's default (D6); this reads the parent's 「not in the agent command table」 as 「not in the list agents see」 (§12 C4).

### 4.2 How the control route refuses it

- `call_daemon_command` checks `SHELL_ONLY` right after `contains` and before the argument check: a listed name with `caller == None` (the two connectionless entrances, `/control/call` and `/control/agent` — the CLI and so every agent) is refused and the host is never called; with `Some(conn)` (the socket — only the shell connects there) it runs. One place, as the parent's 「a name check in `call_daemon_command`'s `caller`」 (parent §1-11a finding 16) — a handler cannot see the caller.
- The control route's list (`/control/commands`, `catalog::handle_list`, which `engram commands [<name>]` reads) is built without `SHELL_ONLY` names. WS `ListCommands` keeps them (the shell's discovery).
- The refusal: D7 decides between (a) the unknown-name answer and (b) an explicit, non-retaining refusal with code `shell-only` and a message saying emptying the trash and deleting for good are done by the user in the library screen.

### 4.3 What is accepted (unchanged — parent §6-5 2, G21)

The guarantee is 「not listed, refused off the socket」. An agent is the same OS user and could in theory reach the WS with the portfile token, or delete `.trash` with its own tools; no device is added.

### 4.4 Hand-over to the client stage (steps 11 · 12a · 13)

- The screen sends `library.emptyTrash` only from a human-gesture handler after the 「영구 삭제」 modal (ADR-0326 decision 4); empty-all sends the `bundles` with exactly the rows the view listed (ADR-0326 decision 6).
- **No webview `__engramCmd` entry and no shell bus command may wrap `library.emptyTrash`** — an agent can call shell commands through `/control/call` relaying (ADR-0160), so a wrapper would reopen the control route. The step-11 / 12a TRDs carry a test that no registered shell or webview command reaches it.

### 4.5 Tests

`call_daemon_command` with `caller None` → the refusal and a counting fake host sees no call; with `Some` → the host runs; through the real `/control/call` route → the refusal; `/control/commands` omits the name, WS `ListCommands` lists it; the rendered `engram help` sections and `builtin/instructions/priming.md` do not contain `emptyTrash` (a text guard). Mutation: removing the check, or the list filter, turns a test red.

## 5. Bindings and generation

- New declared types are exported by agent `tests/ts_export.rs`; `crates/engram-dashboard-agent/bindings/` and `commands.schema.json` are regenerated in each unit that changes declarations (CI ts-rs sync gate). Expected ~7 commands × args/ok + ~12 row types.
- Catalog stays 9 (same unreleased generation); the v9 comment adds 「the file-operation and trash commands (step 10b) land in it too; `library.emptyTrash` is shell-only」.
- No protocol crate change, no `src-tauri/bindings` change.

## 6. Help and priming

- **`engram help library`** (`prompts/engram-help.md`, English, one line per paragraph, `{tool}`): a files block — `library.move` · `rename` · `copy` · `delete` · `listTrash` · `restore`, one line each; one paragraph on per-item results (`status`, the codes in `code`/`args`, a staging named in `args`); one paragraph on the confirmation (§3: read the plan, repeat with `--yes true`, or `--yes true --detach_refs true --plan_digest <d>`; a changed plan answers again); one line on after a timeout (read `library.list` / `library.listTrash` before repeating — a repeated move answers `not-found`, a repeated copy with `KeepBoth` makes a second copy). Under D7 (b) one sentence: 「Emptying the trash and deleting for good are done by the user in the library screen.」 — no command name. Under D7 (a) nothing.
- **Root help line and the priming pointer line** gain 「move · copy · delete · restore」; the priming's codex command-line margin is checked (10a R5 — 735 of 8191 characters at 10a).
- **Command help** (the declarations' doc comments, shown by `engram commands <name>`) is English CLI text. 10b adds **no UI text**: no `src/i18n/en.ts` / `ko.ts` / `ui_text.rs` change, so ADR-0372 / ADR-0373 do not apply here. The trash view's and modals' words are the client stage's, written English-first with the Korean key then.

## 7. Pending user decisions on 9b behaviour (`docs/tracking.md`)

10b maps the current behaviour; each answer below changes at most what codes appear, not a declared shape:

| Item | Current | If the user flips it | Wire effect |
|---|---|---|---|
| T-125 keep-both merge of a referenced skill that empties the source | no ask | asks (`skill-unconfirmed`) | none — an existing code and shape gains one more trigger |
| T-127 merge across volumes · copy onto a same-name folder | refused `crosses-volumes` / `copy-merge-refused` | merged entry by entry | those two codes are no longer answered; a merged copy needs `left` rows on `CopyItemRow` — an additive field, added with that change (not pre-added — D15) |
| T-128 strict times and permissions on copies | strict (item fails) | best effort, logged | none if only logged; a per-item warning field would be additive |
| T-131 read-only junction row cannot be emptied | `remaining` | removed | none — the row answers `gone` |

## 8. Test plan

`.claude/skill-bindings/qa.md` 「돌릴 것 표」: **Rust 소스** (every unit) → full regression; **claude · codex lanes** — U1–U2 touch `crates/engram-dashboard-asset-library/src/**` (both lanes) and U2 `crates/engram-dashboard-daemon/src/control/**` (claude) → both lanes; `prompts/engram-help.md` = 「그 밖」 → full regression, and 「문서 · 바인딩 diff 가 명령을 바꿨으면 바뀐 명령만 문자 그대로 한 번」 → scene C1. GUI: off for U1 and U3; **on for U2 by choice** (scene C2 measures the socket half of the gate). Mutation checks per qa 「변이 확인」 on the rows marked †.

| What | How | Unit |
|---|---|---|
| Declarations | agent `tests/command_declarations.rs` (names, effects, `since(9)`, catalog 9, deadline class per D8, `SHELL_ONLY` ⊂ declared `Write` names); `tests/ts_export.rs` + regenerated bindings and schema (CI sync gate); `lint_spec` over every new spec | U1 · U2 |
| Handler mapping | `library_commands.rs` tests over a fake `LibraryCommandHost`: each verb passes its args as typed (`on_conflict` word, `yes` / `detach_refs` / `plan_digest`, RG, trash addresses); agent names filled from the roster, an unknown id dropped; every refusal class → its code with `args` untouched (plan, forms, stagings) | U1 · U2 |
| Impl mapping | `host.rs` tests over the existing test world: each answer equals the 9b answer field by field (no second body call); every `Outcome` × `Skip` variant → its row and skip code †; the staging fields arrive in `args`; `forms.detach` fed back as args passes the entrance check † ; view nesting, `deleted_at`, `original`, `restorable`; `NoLibrary` answers | U1 · U2 |
| Shell-only gate | §4.5 (control route refused, host not called †; socket runs; list filter †; help / priming text guard) | U2 |
| Help · priming | daemon `control/help.rs` tests (`library` renders, root line); CLI `help_file_section`; the priming tests (asset-library `the_shipped_priming_fits_the_codex_terminal_command_line`, daemon `the_priming_pointer_names_help_entries_that_actually_render`, CLI `the_priming_pointer_lines_parse_as_help_requests`) | U3 |
| CLI scene C1 | on the qa 「GUI 실측」 isolated daemon, from an agent's `engram`: delete a referenced item → `delete-unconfirmed` with forms → `--yes true` → `listTrash` → `restore` (taken name → `conflict` → `--keep_both true`) → `move` with `mapping` and `trees` → `rename` → `copy` into its own folder (` - Copy`) → move a referenced `SKILL.md` out → `skill-unconfirmed` → detach form; `engram library.emptyTrash …` refused (D7 answer) and absent from `engram commands`; time a large cross-volume copy against the wait chosen in D8 | U3 (after U1 · U2) |
| Shell scene C2 | same daemon: `invoke('forward_daemon_command', …)` (qa 「셸 invoke」) sends `library.emptyTrash` with the `bundles` of a fresh `listTrash` → rows `gone`, bundle folders gone; a row not listed survives | U2 |

## 9. Implementation units

Sequential — U1 and U2 both touch agent `commands.rs` / `library_commands.rs`, asset-library `host.rs` and daemon `control/commands.rs`; a new trait method breaks every implementor, so each unit carries its port methods, the `host.rs` mapping, `NoLibrary` and the test doubles together. Build and tests green at the end of each. Sizes include tests (about half).

| Unit | Content | Lines |
|---|---|---|
| U1 | The six LLM-callable commands end to end: declarations and rows, port methods, handlers, `host.rs` mapping, `DeletePlan::to_json` forms spelling (§3), `NoLibrary`, fakes, bindings and schema, tests. | ~1,600–2,100 |
| U2 | `library.emptyTrash` and the gate: declaration, `SHELL_ONLY`, port method and mapping, `NoLibrary`, `call_daemon_command` check (D7), control-list filter, tests, bindings. **Starts after the 9b U5 revision is committed** (`empty_trash` signature). | ~500–700 |
| U3 | Help, root line, priming pointer, headers and reference files (the 「Reference files to fix」 row). | ~250 |

## 10. Decisions

### Needs user decision

- **D7 — How the control route answers `library.emptyTrash`.** Example: the user tells an agent 「휴지통 비워 줘」 and the agent guesses `engram library.emptyTrash`. (a) **Hidden**: the same `UNKNOWN_COMMAND` answer as a name that does not exist, and `engram help` says nothing about emptying — the agent cannot tell the user where to do it. (b) **Explicit** ★: a non-retaining refusal `shell-only` whose message says emptying the trash and deleting for good are done by the user in the library screen, plus that one sentence in `engram help library` (no command name) — the agent can answer the user in one turn; the name stays out of every list. Convention: APIs split — GitHub's REST API answers 404 for what the caller may not see (hiding existence), Kubernetes RBAC answers 403 Forbidden with the reason (from knowledge, not re-fetched); for a guard against mistakes rather than an attacker, the explicit form fits. Either way the name is absent from `engram commands`, `engram help` command lines and the priming (parent §1-7).
- **D8 — How long a caller waits for a big move or copy.** Example: an agent moves a 2 GB folder to a mount on another drive; the copy takes 40 s. (a) Today's ordinary waits (bus 7 s / CLI 10 s): the caller gets `OUTCOME_UNKNOWN` after 10 s while the move finishes; the help tells it to read `library.list` before repeating. (b) ★ The long wait the launch commands already use (85 s reply bound / 88 s caller wait — ADR-0369) for the six writing file commands; past that, the same `OUTCOME_UNKNOWN` rule. Convention: trash-cli (`trash-put`, `trash-restore`, `trash-empty`) and GNOME `gio` (`gio move`, `gio copy`, `gio trash`) block until the operation ends and have no timeout option (their man page and README, checked 2026-11-24); Windows Explorer and VS Code show progress and wait (from knowledge). Mechanism under (b) is main's: reuse the existing `Launch` deadline class as is (its schema word stays `Launch`) rather than a new class word.

### Main's default (reported)

- **D1** Names: `library.move` · `rename` · `copy` · `delete` · `restore` · `emptyTrash` as the parent; the view = **`library.listTrash`** (replaces the placeholder `library.trash`, which reads as a verb — `gio trash` moves to the trash). Convention: trash-cli `trash-list`, `gio trash --list`; this bus's `agent.listQueuedInputs`. One `emptyTrash` for empty-all and permanent delete (ADR-0408 decision 4), not trash-cli's separate `trash-empty` / `trash-rm`.
- **D2** Args: snake_case (10a B5); `to_folder` (the parent's `toFolder` — a folder, not the new path; coreutils `cp` / `mv` call it `--target-directory`); `rename`'s new name is `name` (as `agent.rename` and `library.newFolder`); `on_conflict` words `Overwrite` · `Skip` · `KeepBoth` (Windows Explorer's replace / skip / keep both, macOS Finder's Replace / Keep Both — from knowledge); restore and emptying address trash rows as `rows` (the parent's `items`); restore's choice is `keep_both` (it never overwrites — G11), not `on_conflict`.
- **D3** Per-item rows flat with `status` (§2.2); skip reasons as codes `inside-given-folder` · `in-place` · `name-exists` · `name-taken-during-copy`.
- **D4** Final codes = as built (§2.4); no rename (renaming touches the 9b tests and ADR texts for no LLM gain).
- **D5** The `CONFLICT` forms carry the wire spelling, fixed at the source (`DeletePlan::to_json`), not rewritten in `host.rs`.
- **D6** `library.emptyTrash` in the shared block + `SHELL_ONLY` list, checked in `call_daemon_command` by `caller == None`, omitted from the control list (§4).
- **D9** Catalog 9 kept; bindings regenerated per unit.
- **D10** Help additions as §6; no UI text, so no Korean key in 10b.
- **D11** The view nests mount → bundle → rows like the service and the screen's grouping (§6-5 2); `deleted_at` RFC 3339 UTC; `restorable` derived as §2.3.
- **D12** Agents in answers named by the handler from one roster read (10a B18).
- **D13** `CopyItemRow.trashed` is a list (0 or 1), as `MoveItemRow.trashed` (T-130 item 6 — the crate types differ).
- **D14** A text guard keeps `emptyTrash` out of `prompts/engram-help.md` and the priming.
- **D15** Nothing is pre-added for the pending T-items (§7); their fields are additive in the same catalog.

## 11. Risks

- **R1** Time against the seat (D8). Under (a) every large cross-volume move or copy answers `OUTCOME_UNKNOWN`; under (b) beyond 85 s the same. A repeated move answers `not-found` per item (safe); a repeated copy with `KeepBoth` makes a second copy, with no `on_conflict` it answers `conflict` (safe). Measured in C1.
- **R2** The shell-only guarantee is 「not listed, refused off the socket」 (accepted — G21); a future shell or webview command wrapping `library.emptyTrash` would reopen it (hand-over §4.4).
- **R3** Answer size: a delete or move of many items, a long `mapping`, a trash of many bundles — all printed into the agent's context (ADR-0307 §2-4 hand-off undecided).
- **R4** `empty_trash` is under revision (9b U5); U2's mapping follows its committed shape — if the answer changes, §2.3's emptying rows change with it before U2.
- **R5** Declarations and bindings grow (~7 commands, ~40 generated files); review cost. Mitigated by the unit split.
- **R6** The priming pointer line grows by a few words (codex command-line limit — 10a R5).
- **R7** The forms spelling change (§3) changes asset-library test expectations written for `detachRefs`.

## 12. Input claims corrected (parent `trd.md`, 9b TRD, brief → code at `c58ea68d` + working tree)

- **C1** Parent §1-7 / §5-3 `library.emptyTrash {items?}` — the service takes `bundles` (each with the rows the view listed) and `rows`; neither → `nothing-to-empty` (M86).
- **C2** Parent §1-7 `emptyTrash` row 「hands over to the daemon PC's OS trash and the OS decides」 and 「there is one root, so no root arg (G36)」 — withdrawn by ADR-0408 (direct removal) and by mounts (every address carries its mount — ADR-0409 M5).
- **C3** Parent §1-7 `library.restore {items}` and `library.trash` — `rows` and `library.listTrash` (D1 · D2); `library.move {…, toFolder, …}` · `rename {…, newName, …}` · `detachRefs` placeholders → `to_folder` · `name` · `detach_refs`.
- **C4** Parent 「not in the agent command table」 — the declaration must sit in the daemon table `make_table` builds (the socket's stage 1 runs only that table); what it is absent from is the control route's list, `engram help` and the priming (§4.1).
- **C5** `DeletePlan::to_json` spells the detach form `detachRefs`; the wire argument is `detach_refs` — fixed in U1 (§3).
- **C6** 9b TRD §0 places the refusal 「at `call_daemon_command`'s `caller`」 — consistent with §4.2.
- **C7** 9b TRD §4 lists `other` among codes — in the code it is a view row kind (`RowKind::Other`); its list also predates three codes added while coding: `crosses-volumes`, `copy-merge-refused` (ADR-0416 · ADR-0417) and `nothing-to-empty` (U5).
- **C8** ADR-0416 「10b carries `staging`, `staging_disk`, `staging_row`, `staging_kept`, `not_copied`, `stagings`」 — they are keys inside reason `args`, not answer fields; 10b carries `args` as is (§2.2) — only `Skip::TakenDuringCopy` needs a row (`name-taken-during-copy`).
- **C9** Parent §1-7 copy row 「- 복사본」 and `engram:/…` — already corrected by the 9b TRD §11 (UD3 ` - Copy`, `engram/…`); repeated here because the 10b help text uses the corrected forms.

## 13. Review record

(empty)
