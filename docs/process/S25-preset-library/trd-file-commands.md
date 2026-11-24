# TRD — S25 step 10b: file-operation and trash commands (`S25: feat(agent,daemon): 파일 조작 · 휴지통 명령 선언(2판 몫)`)

| Item | Value |
|---|---|
| **Status** | **Draft 2 (2026-11-24)** — review round 1 (deep, FIX) applied; decisions and boundaries only. User decisions taken 2026-11-24 (§10, D7 · D8 as recommended). D8 needs an ADR at the 10b commit amending ADR-0369 decision 1 and 10a's B17; the same ADR amends ADR-0322 decision 1's wording (§10). Canonical ADRs: ADR-0322 decisions 1–3 (emptying is human-only, through one shell-only bus command) · ADR-0326 decision 6 (empty-all = what the view listed) · ADR-0408 (emptying removes directly, gone / remaining per entry) · ADR-0409 (9b rules, UD1–UD4, M1–M15) · ADR-0414 · ADR-0415 · ADR-0416 · ADR-0417 · ADR-0418 (9b coding-time rules — 10b maps their answers as they are) · ADR-0339 (`yes` / `detachRefs`) · ADR-0405 decision 1 (a digest pins what the caller saw) · ADR-0155 (declaration next to the producer) · ADR-0157 (one entrance check) · ADR-0307 (LLM command rules) · ADR-0369 (deadline class) · ADR-0379 (refusal class → bus code) · ADR-0384 (answers mapped as built) · ADR-0386 (10a surface rules this step extends). |
| **Parent TRD** | [`trd.md`](trd.md) §5-3 item 10b · §1-7 (the `library.move` … `library.emptyTrash` rows, 「Only people do this」) · §1-8 (no new wire variant) · §6-5 2 (the screen route, M6). |
| **Step-9b TRD** | [`trd-file-operations.md`](trd-file-operations.md) §0 「Out」 (the line between 9b and 10b) · §3 · §4 「Service answers (10b maps them 1:1 onto the wire)」 and its placeholder codes. |
| **Pattern** | [`trd-library-commands.md`](trd-library-commands.md) (step 10a, Implemented): the port, the host mapping, the refusal function, snake_case fields (its B5), `#[since(9)]`, help topic `library`. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Wiring: the file-command methods of `impl LibraryCommandHost for LibraryService`; the shell-only command) · `docs/reference/architecture.md` (daemon: `call_daemon_command` refuses shell-only names off the socket; control discovery omits them) · asset-library `src/lib.rs` header 「Commands」 paragraph · `ops.rs` / `trash.rs` docs that say `library.trash` (→ `library.listTrash`, D1) · agent `commands.rs` header and the v9 catalog comment · daemon `control/commands.rs` header (a third check beside arguments and the lease) and `control/catalog.rs` header (the list omits shell-only names) · `ops.rs` docs that still spell the arguments `onConflict` / `detachRefs` (→ `on_conflict` / `detach_refs` where they name the wire) · `docs/reference/llm-command-conventions.md` §5 (the `#[deadline(Launch)]` check line: not only launches — D8) · `prompts/engram-help.md`. |
| **Line-pointer basis** | Code at `9f166061` (9b U5 committed — step 9b Implemented; its coding-time rules ADR-0418). Code is cited by file + symbol. |
| **Not a source** | Frozen TRDs (`trd-library-commands.md`, `trd-library-mounts.md` …) are read only to find why something is so. Where the parent §1-7 rows disagree with ADR-0408, ADR-0409, the 9b TRD or the code, those win (§12). |

## 0. Scope

**In** — every name an LLM or the shell calls for the 9b service, on the daemon command bus (CLAUDE.md 「LLM-우선 제어」 · ADR-0307):

- LLM and shell: `library.move` · `library.rename` · `library.copy` · `library.delete` · `library.listTrash` (the trash view — the parent's placeholder `library.trash`, D1) · `library.restore` (6).
- Shell only: `library.emptyTrash` (empty-all and permanent delete — ADR-0322 decisions 1–3 · ADR-0408 decision 4): declared beside the others, refused by the daemon when it arrives off the socket, absent from the control route's command list, `engram help` and the priming (§4).
- Seven new `LibraryCommandHost` methods, their `host.rs` mapping, the daemon's `NoLibrary` answers, regenerated agent bindings and `commands.schema.json`, the `engram help library` additions.
- One CLI change (D7): `engram`'s `run_invoke` posts a name the catalog does not list to `/control/call` anyway, so the daemon decides what is reachable and answers it — its `unknown_name` still answers a typo, and the shell-only refusal reaches the agent (§4.2).
- The deadline class of the long-running file commands (D8, §2).

**Out** — screens (trash view, modals, drag — client stage); the webview round trip, how the screen sends `library.emptyTrash`, and the shell's reply wait for these commands (step 11 / 13 — hand-over §4.4); any service behaviour change (9b and its pending decisions, §7); a progress record (G38); automatic emptying (ADR-0322 decision 8); the large-answer file hand-off (ADR-0307 §2-4, undecided); no new wire message, `PROTOCOL_VERSION` unchanged (parent §1-8).

**Catalog** — stays 9 (the same unreleased generation as 10a and step 21); the v9 comment gains one line.

## 1. Boundaries and ownership

- **agent owns the vocabulary** — the seven names, their args and answer rows go into the one `declare_commands!` block of `commands.rs` (one block per module); the handlers and port methods into `library_commands.rs`, as 10a. `make_table` keeps its signature.
- **The port speaks agent types.** Each new `LibraryCommandHost` method takes the spellings the caller typed (paths as strings, `on_conflict` as the declared word, trash addresses as declared rows) and answers declared rows or a `PortRefusal`. Agents cross the port by id only; the handler names them from one `AgentCommandHost::roster` read and drops an id the roster no longer holds (10a B18). The `CONFLICT` plan's agents already carry names from the service and pass through untouched.
- **asset-library maps, never re-runs** — `host.rs` calls the one 9b method per command and converts its answer field by field; no second body call, no read-back (ADR-0384). Refusals keep their class and code 1:1 (`LibraryRefusal` → `PortRefusal`).
- **The daemon only wires — plus one gate.** `NoLibrary` answers every new name `CONFLICT` `library-unavailable` (as 10a). `call_daemon_command` (`control/commands.rs`) gains the shell-only check (§4.2); the control route's command list omits shell-only names — the filter sits where `mcp_server.rs` hands the table's declarations to `catalog::handle_list`. No other daemon file changes.
- **The CLI changes in one place** — `run_invoke` no longer stops locally at a name its catalog read does not list: it posts the call and prints the daemon's answer (D7, §4.2). Every name is still reached by the general form `engram <name> --flag value` over `/control/call`; the local argument screening stays for listed names.
- **No shell or frontend change** in 10b.

## 2. Command surface (wire)

Field names are snake_case (10a B5). Every declaration is `#[since(9)]`. The five LLM writes take an optional `root_generation` (RG — ADR-0377); `library.emptyTrash` takes none (its service has no generation check). All six writes are `Effect::Write`; `library.listTrash` is `Read`.

**Deadline class (D8).** `library.move` · `rename` · `copy` · `delete` · `restore` and `library.emptyTrash` are declared `#[deadline(Launch)]`; `library.listTrash` stays `Ordinary` (its reads are bounded by the 5 s IO budget). What that buys, per path, as the code does it:

- **CLI → `/control/call`** (every agent): `catalog::handle_call` runs the daemon's own names on the spot through `locals.run` (caller `None`) — **no delivery seat, no reply bound, no request-id guard**; the CLI makes a new request id per run. The class only lengthens the CLI's silence window to 94 s (`LAUNCH_TIMEOUT` = `LAUNCH_RELAY_CALLER_WAIT_SECS`, chosen by `call_silence_window`). Past it the CLI prints `CONNECT_FAILED` `read failed: …`, while the command keeps running on the blocking pool to its end.
- **Shell socket** (the screen, `library.emptyTrash`): stage 1 opens a seat with the launch reply bound (85 s) and keeps the request id. The shell's own wait is the shared 30 s today: its `reply_deadline_of` gives the long wait only to `Spawn` · `SpawnByCwd` · `SpawnProfile` (hand-over §4.4 — no shell caller in 10b).
- `Effect::Write` keeps the id only on the socket's seats; on the CLI path a repeat is a new call (§11 R1).

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
- `TrashRowArg` / `TrashAddressRow` = `{mount, bundle, path}` — `path` is the row's path inside its mount, as the view gives it (ADR-0409 M5: emptying takes no per-mount argument — the mount travels in every address, here for restore too).

### 2.2 Per-item rows

One flat row per given path or trash row, in the order given (ADR-0409 per-item results): `status` = `done` | `skipped` | `conflict` | `failed`, then the fields of that status. A non-`done` row carries `code` · `args` — the `Reason` as built (its `line` is always empty for these operations, so it is not declared on these rows), so the staging fields of ADR-0416 decision 4 (`staging` · `staging_disk` · `staging_row` · `staging_kept` · `not_copied`) reach the caller inside `args` unchanged.

| Row | Key | `done` fields |
|---|---|---|
| `MoveItemRow` | `path` | `to` · `trashed: [TrashAddressRow]` (an overwrite's displaced entries) · `original?: TrashAddressRow` (a cross-volume original) · `left: [PathReasonRow]` (a merge's entries left, ADR-0415) |
| `CopyItemRow` | `path` | `to` · `trashed: [TrashAddressRow]` (0 or 1 — a list as on move, D13) |
| `DeleteItemRow` | `path` | `trashed: TrashAddressRow` |
| `RestoreItemRow` | `mount` · `bundle` · `path` | `to` (the virtual path it is back at) |

`skipped` codes (the four `Skip` variants — D3): `inside-given-folder` `{folder}` (a folder above it was given in the same call — P5) · `in-place` `{}` (a move into its own folder, a rename to its own spelling) · `name-exists` `{path}` (taken, `on_conflict` `Skip`) · `name-taken-during-copy` `{path, staging…}` (`Skip::TakenDuringCopy`).

### 2.3 Trash view and emptying rows

- `MountTrashRow` = `mount` · `problem?: {code, args}` (the mount is named, not failed: `trash-unusable` `{mount, why}` or `library-unavailable` `{mount, message}`) · `bundles: [{bundle, deleted_at, source, why?, trouble?, rows: [TrashViewRow]}]` by id. `deleted_at` = RFC 3339 UTC from the id (`2026-11-23T08:15:00Z`). `source` = `record` | `no_record` | `record_unreadable` (`why`) | `unchecked` (no rows; `trouble` says why).
- `TrashViewRow` = `path` · `original` (the virtual path a restore returns it to — `MountTrash::original`) · `kind` = `root` | `found` | `other` · `state` = `present` (with `entry` = `file` | `folder` | `link`) | `through_link` | `failed` (`why`) · `staging` · `intended?` (virtual path) · `restorable` (D11) = kind is not `other`, not staging, state `present`, **and `original` passes restore's own admission** (`admits_inside`, as `ops.rs` `restorable` judges it before anything else) — so a fallback row with a dot name (`.DS_Store`, `.hidden/file.md` in a bundle without a readable record) is not restorable. Since ADR-0418 decision 1 a recordless bundle also lists its empty folders as `found` rows; they follow the same rule.
- `EmptiedRow` = `mount` · `bundle` · `path` · `outcome` = `gone` | `remaining` | `not_found` · `code?` · `args?` — every row that is not `gone` carries its reason (`remaining`: why it stays, the OS error code a diagnostic inside `args` — ADR-0408 decision 2; `not_found`: the `not-found` reason, which names the foreign bundle or the path that is no row). `EmptiedBundleRow` = `mount` · `bundle` · `outcome` (same words) · `untouched?: {code, args}`.

### 2.4 Refusals of the whole call

The 10a refusal function maps every port refusal (ADR-0379; `CommandError.args` = `{code, args, line?}`). The whole-call refusals are exactly the ones the 9b method docs list — 10b adds only `shell-only` (§4.2):

- the five LLM writes: `library-unavailable` (no first scan applied — `Unavailable` → `CONFLICT`, retry after condition) · `root-generation-stale` (`CONFLICT`);
- `move` / `copy`: `to_folder` not a library path, or a dot or temp name in it → `INVALID_ARGUMENT` `item-path-invalid`; in the built-in → `CONFLICT` `mount-read-only`; `rename`: `name` not one valid name → `INVALID_ARGUMENT` `item-path-invalid`;
- `delete` → `CONFLICT` `delete-unconfirmed`; `move` / `rename` → `CONFLICT` `skill-unconfirmed`, `INVALID_ARGUMENT` `detach-refs-invalid` (§3); with agents to change while the agent store is read-only → the store's refusal; the port cell empty → `INTERNAL` `profile-edit-unavailable` (keeps the id — ADR-0379 decision 5);
- `emptyTrash` refuses only one way: neither `bundles` nor `rows` → `INVALID_ARGUMENT` `nothing-to-empty` (M86 — whoever calls); everything else answers per row and per bundle.

**Final codes (D4).** The 9b placeholders are final as built: `into-itself` · `kind-mismatch` · `links-inside` · `source-changed` · `target-mount-changed` · `crosses-volumes` · `copy-merge-refused` · `trash-unusable` · `trash-not-reserved` · `path-through-link` · `not-restorable` · `inside-root` · `record-unreadable` · `no-replace-unsupported` · `delete-unconfirmed` · `skill-unconfirmed` · `detach-refs-invalid` · `nothing-to-empty`; reused `item-path-invalid` · `name-exists` · `not-found` · `virtual-root-read-only` · `mount-read-only` · `library-unavailable` · `library-read-failed` · `library-write-failed` · `root-generation-stale` · `profile-edit-unavailable`. New in 10b: three skip codes (`inside-given-folder` · `in-place` · `name-taken-during-copy` — the fourth skip, `name-exists`, reuses the existing code; §2.2) and `shell-only` (§4.2). `other` is a view row kind, not a code (§12 C7).

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

- **The gate.** `call_daemon_command` checks `SHELL_ONLY` right after `contains` and before the argument check: a listed name with `caller == None` is refused and the host is never called; with `Some(conn)` it runs. One place, as the parent's 「a name check in `call_daemon_command`'s `caller`」 (parent §1-11a finding 16) — a handler cannot see the caller.
- **Why `caller == None` is the key, and what keeps it true** (a comment at the check says this). Today exactly two paths reach `call_daemon_command`: `catalog::handle_call` (`/control/call`) and `control/agent.rs` (`/control/agent`, which only builds `agent.<verb>` names) pass `None`; socket stage 1 (`command_delivery`, `DaemonLocalCommands::run`) passes the sending connection — only the shell connects there with the daemon token. The one other door that holds a connection number for a connectionless caller, `CommandBus::invoke` (a synthetic `ConnId` counted down from `u64::MAX`), never runs the daemon's own names (it refuses them before stage 1). Main's default: keep `Option<ConnId>` (no explicit caller kind) and pin it with a test per door — `/control/call` refused, `/control/agent` cannot name it, the socket runs it, `CommandBus::invoke` with `library.emptyTrash` does not run it — so a new door that passes a connection number for a non-shell caller turns a test red.
- **The refusal (D7 (b)).** Bus code `UNSUPPORTED` (retry `Never`, does not keep the id) with `args.code` = `shell-only` and a message: emptying the trash and deleting for good are done by the user in the library screen. `catalog::refused` adds the 「list the command catalog」 hint only to `INVALID_ARGUMENT` and `UNKNOWN_COMMAND`, so this text carries none (a test asserts it).
- **The CLI reaches it.** Today `run_invoke` fetches `/control/commands` first and, for a name the list does not hold, prints its own `UNKNOWN_COMMAND` without calling — the refusal above would never reach an agent. U2 changes `run_invoke` to post an unlisted name to `/control/call` anyway (no local argument screening — there is no schema; the ordinary 10 s silence window) and print the daemon's answer: a real typo still answers the daemon's `unknown_name` with the catalog hint; `library.emptyTrash` answers `shell-only`. CLI test: an unlisted name is posted and the daemon's code is printed.
- **The list.** The control route's list (`/control/commands`, built by `catalog::handle_list` from the declarations `mcp_server.rs` hands it — what `engram commands [<name>]` reads) omits `SHELL_ONLY` names. WS `ListCommands` keeps them (the shell's discovery).

### 4.3 What is accepted (unchanged — parent §6-5 2, G21)

The guarantee is 「not listed, refused off the socket」. An agent is the same OS user and could in theory reach the WS with the portfile token, or delete `.trash` with its own tools; no device is added.

### 4.4 Hand-over to the client stage (steps 11 · 12a · 13)

- The screen sends `library.emptyTrash` only from a human-gesture handler after the 「영구 삭제」 modal (ADR-0326 decision 4); empty-all sends the `bundles` with exactly the rows the view listed (ADR-0326 decision 6).
- **No webview `__engramCmd` entry and no shell bus command may wrap `library.emptyTrash`** — an agent can call shell commands through `/control/call` relaying (ADR-0160), so a wrapper would reopen the control route. The step-11 / 12a TRDs carry a test that no registered shell or webview command reaches it.
- **The shell's wait for launch-class envelopes.** `reply_deadline_of` (`src-tauri/src/daemon_client/connection.rs`) gives the long wait only to `Spawn` · `SpawnByCwd` · `SpawnProfile` and treats every bus envelope as ordinary (30 s) — its doc says the first Engram caller of a launch-class envelope must add the classification (ADR-0369 「영향」). The first screen caller of `library.move` · `rename` · `copy` · `delete` · `restore` · `emptyTrash` adds it, with a slow-envelope test; until then a screen call past 30 s times out on the shell side while the daemon finishes.

### 4.5 Tests

`call_daemon_command` with `caller None` → `UNSUPPORTED` `shell-only` and a counting fake host sees no call; with `Some` → the host runs; the per-door tests of §4.2 (the raw `/control/call` route refused, `CommandBus::invoke` does not run it); the refusal text carries no catalog hint; `/control/commands` omits the name, WS `ListCommands` lists it; the CLI posts an unlisted name and prints the daemon's answer; a text guard: the rendered `engram help` sections, `builtin/instructions/priming.md` and **every `help` blob `/control/commands` returns** contain no `emptyTrash`. Mutation: removing the check, the list filter, or the CLI's post of unlisted names turns a test red.

## 5. Bindings and generation

- New declared types are exported by agent `tests/ts_export.rs`; `crates/engram-dashboard-agent/bindings/` and `commands.schema.json` are regenerated in each unit that changes declarations (CI ts-rs sync gate). Expected ~7 commands × args/ok + ~12 row types.
- Catalog stays 9 (same unreleased generation); the v9 comment adds 「the file-operation and trash commands (step 10b) land in it too; `library.emptyTrash` is shell-only」.
- No protocol crate change, no `src-tauri/bindings` change.

## 6. Help and priming

- **`engram help library`** (`prompts/engram-help.md`, English, one line per paragraph, `{tool}`): a files block — `library.move` · `rename` · `copy` · `delete` · `listTrash` · `restore`, one line each; one paragraph on per-item results (`status`, the codes in `code`/`args`, a staging named in `args`); one paragraph on the confirmation (§3: read the plan, repeat with `--yes true`, or `--yes true --detach_refs true --plan_digest <d>`; a changed plan answers again); one paragraph on a reply that never comes: a `CONNECT_FAILED` `read failed` after a file command means the outcome is unknown and the command may still be running — do not repeat a move, copy, delete or restore; after a pause read the destination (`library.list`, `library.listTrash`) and tell the user about any staging that was kept (a temp-named folder only the user can remove). One sentence (D7 (b)): 「Emptying the trash and deleting for good are done by the user in the library screen.」 — no command name. **These lines land in U1**, with the commands, because the daemon test `the_library_screen_teaches_every_library_command_and_the_edit_rules` (`control/help.rs`) requires every declared `library.*` name on the library screen by its full name (count 24 today): U1 raises its count to 30 (six more `library.*` names — five writes and `listTrash`); U2 makes it exclude `SHELL_ONLY` names and assert they are absent from the screen, so 30 names stay required while 31 are declared.
- **Root help line and the priming pointer line** gain 「move · copy · delete · restore」; the priming's codex command-line margin is checked (10a R5 — 735 of 8191 characters at 10a).
- **Command help** (the declarations' doc comments, shown by `engram commands <name>`) is English CLI text. 10b adds **no UI text**: no `src/i18n/en.ts` / `ko.ts` / `ui_text.rs` change, so ADR-0372 / ADR-0373 do not apply here. The trash view's and modals' words are the client stage's, written English-first with the Korean key then.

## 7. Pending user decisions on 9b behaviour (`docs/tracking.md`)

10b maps the current behaviour; each answer below changes at most what codes appear, not a declared shape:

| Item | Current | If the user flips it | Wire effect |
|---|---|---|---|
| T-125 keep-both merge of a referenced skill that empties the source | no ask | asks (`skill-unconfirmed`) | none — an existing code and shape gains one more trigger |
| T-127 merge across volumes · copy onto a same-name folder | refused `crosses-volumes` / `copy-merge-refused` | merged entry by entry | those two codes are no longer answered; a merged copy needs `left` rows on `CopyItemRow` — an additive field, added with that change (not pre-added — D15) |
| T-128 strict times and permissions on copies | strict (item fails) | best effort, logged | none if only logged; a per-item warning field would be additive |
| T-131 what read-only entries emptying still cannot remove (the read-only junction row is resolved — ADR-0418 decision 5; left: a Unix 0555 folder inside a row, a read-only empty-folder row) | `remaining` | removed (write bit added / `make_removable` + one retry) | none — the row answers `gone` |

## 8. Test plan

`.claude/skill-bindings/qa.md` 「돌릴 것 표」: **Rust 소스** (every unit) → full regression; **claude · codex lanes** — U1–U2 touch `crates/engram-dashboard-asset-library/src/**` (both lanes) and U2 `crates/engram-dashboard-daemon/src/control/**` (claude) → both lanes; `prompts/engram-help.md` = 「그 밖」 → full regression, and 「문서 · 바인딩 diff 가 명령을 바꿨으면 바뀐 명령만 문자 그대로 한 번」 → scene C1. GUI: off for U1 and U3; **on for U2 by choice** (scene C2 measures the socket half of the gate). Mutation checks per qa 「변이 확인」 on the rows marked †.

| What | How | Unit |
|---|---|---|
| Declarations | agent `tests/command_declarations.rs` (names, effects, `since(9)`, catalog 9, `SHELL_ONLY` ⊂ declared `Write` names); the deadline pin `only_agent_spawn_is_declared_with_the_launch_deadline` (agent `commands.rs`) changes — renamed and widened to `agent.spawn` plus the six D8 names, `library.listTrash` not among them; `tests/ts_export.rs` + regenerated bindings and schema (CI sync gate); `lint_spec` over every new spec | U1 · U2 |
| Handler mapping | `library_commands.rs` tests over a fake `LibraryCommandHost`: each verb passes its args as typed (`on_conflict` word, `yes` / `detach_refs` / `plan_digest`, RG, trash addresses); agent names filled from the roster, an unknown id dropped; every refusal class → its code with `args` untouched (plan, forms, stagings) | U1 · U2 |
| Impl mapping | `host.rs` tests over the existing test world: each answer equals the 9b answer field by field (no second body call); every `Outcome` × `Skip` variant → its row and skip code †; the staging fields arrive in `args`; `forms.detach` fed back as args passes the entrance check † ; view nesting, `deleted_at`, `original`; `restorable` false for a fallback dot-name file row (`.DS_Store`) and a row under a dot-named folder (`.hidden/file.md`) †, true for a plain recordless file row; `EmptiedRow` carries the reason on `not_found` rows; `NoLibrary` answers | U1 · U2 |
| Shell-only gate | §4.5 (control route refused with `UNSUPPORTED` `shell-only`, host not called †; no catalog hint in the text; per-door tests incl. `CommandBus::invoke`; socket runs; list filter †; text guard over help, priming and `/control/commands` help blobs) | U2 |
| CLI | `engram` tests: an unlisted name is posted to `/control/call` and the daemon's code and text are printed †; a listed name still gets local argument screening | U2 |
| Help · priming | daemon `control/help.rs` `the_library_screen_teaches_every_library_command_and_the_edit_rules` (count 30 in U1; in U2 shell-only names excluded and asserted absent) and the root line; CLI `help_file_section`; the priming tests (asset-library `the_shipped_priming_fits_the_codex_terminal_command_line`, daemon `the_priming_pointer_names_help_entries_that_actually_render`, CLI `the_priming_pointer_lines_parse_as_help_requests`) | U1 (help lines) · U2 (exclusion) · U3 (root and priming lines) |
| CLI scene C1 | on the qa 「GUI 실측」 isolated daemon, from an agent's `engram`: delete a referenced item → `delete-unconfirmed` with forms → `--yes true` → `listTrash` → `restore` (taken name → `conflict` → `--keep_both true`) → `move` with `mapping` and `trees` → `rename` → `copy` into its own folder (` - Copy`) → move a referenced `SKILL.md` out → `skill-unconfirmed` → detach form; `engram library.emptyTrash …` answers `UNSUPPORTED` `shell-only` with the screen sentence and no catalog hint, and is absent from `engram commands`; a mistyped name still answers the daemon's unknown-name text; a large cross-volume copy: the CLI waits up to 94 s, and past it prints `CONNECT_FAILED` `read failed` while the copy finishes (read the destination after) | U3 (after U1 · U2) |
| Shell scene C2 | same daemon: `invoke('forward_daemon_command', …)` (qa 「셸 invoke」) sends `library.emptyTrash` with the `bundles` of a fresh `listTrash` → rows `gone`, bundle folders gone; a row not listed survives (a small trash — the shell's wait is the ordinary 30 s until the §4.4 hand-over) | U2 |

## 9. Implementation units

Sequential — U1 and U2 both touch agent `commands.rs` / `library_commands.rs`, asset-library `host.rs` and daemon `control/commands.rs`; a new trait method breaks every implementor, so each unit carries its port methods, the `host.rs` mapping, `NoLibrary` and the test doubles together. Build and tests green at the end of each. Sizes include tests (about half).

| Unit | Content | Lines |
|---|---|---|
| U1 | The six LLM-callable commands end to end: declarations and rows (the five writes `#[deadline(Launch)]` — D8; the pin test widened), port methods, handlers, `host.rs` mapping, `DeletePlan::to_json` forms spelling (§3), `NoLibrary`, fakes, bindings and schema, **the `engram help library` lines and paragraphs of §6** (so the help-screen test ends green), tests. | ~1,700–2,200 |
| U2 | `library.emptyTrash` and the gate: declaration (`#[deadline(Launch)]`), `SHELL_ONLY`, port method and mapping, `NoLibrary`, the `call_daemon_command` check with its comment and per-door tests, the `UNSUPPORTED` `shell-only` refusal, the control-list filter, the help-screen test's exclusion, the text guard, **the CLI's `run_invoke` change and its test** (`crates/engram-dashboard-cli`), bindings. | ~600–800 |
| U3 | Root help line, priming pointer, headers and reference files (the 「Reference files to fix」 row). | ~200 |

## 10. Decisions

### User decisions (2026-11-24)

- D7 and D8 below: the user took the recommended options (b) — 「ㅇㅇ 리뷰 구현 QA 까지 쭉 진행해.」 after being asked 「추천안(D7 · D8)대로 리뷰→구현→QA까지 진행할까요?」.


- **D7 — How the control route answers `library.emptyTrash`.** Example: the user tells an agent 「휴지통 비워 줘」 and the agent guesses `engram library.emptyTrash`; with the CLI change of §4.2 that call reaches the daemon. (a) **Hidden**: the same unknown-name answer as a name that does not exist, and `engram help` says nothing about emptying — the agent cannot tell the user where to do it. (b) **Explicit** ★ (taken): a non-retaining refusal — bus code `UNSUPPORTED`, `args.code` `shell-only`, no catalog hint — whose message says emptying the trash and deleting for good are done by the user in the library screen, plus that one sentence in `engram help library` (no command name) — the agent can answer the user in one turn; the name stays out of every list. Convention: APIs split — GitHub's REST API answers 404 for what the caller may not see (hiding existence), Kubernetes RBAC answers 403 Forbidden with the reason (from knowledge, not re-fetched); for a guard against mistakes rather than an attacker, the explicit form fits. Either way the name is absent from `engram commands`, `engram help` command lines and the priming (parent §1-7).
- **D8 — How long a caller waits for a big move or copy.** Example: an agent moves a 2 GB folder to a mount on another drive; the copy takes 40 s. (a) Today's ordinary wait: the agent's CLI gives up after 10 s with `CONNECT_FAILED` `read failed` while the move finishes. (b) ★ (taken) The long launch wait for `library.move` · `rename` · `copy` · `delete` · `restore` and `library.emptyTrash` (not `listTrash`): the agent's CLI waits 94 s, and past that prints the same `CONNECT_FAILED` `read failed` while the command finishes; the help (§6) tells the agent to treat that as outcome unknown, not repeat, read the destination after a pause and report kept stagings. On the CLI path there is no seat, reply bound or request-id guard either way (§2); on the shell socket the seat gets the 85 s launch bound, while the shell's own 30 s wait is widened only by the client stage (§4.4). *Draft 1 described (b) as an 85 s / 88 s wait ending in `OUTCOME_UNKNOWN`; that is the socket's seat, not the agent's path — corrected here after review round 1.* Convention: trash-cli (`trash-put`, `trash-restore`, `trash-empty`) and GNOME `gio` (`gio move`, `gio copy`, `gio trash`) block until the operation ends and have no timeout option (their man page and README, checked 2026-11-24); Windows Explorer and VS Code show progress and wait (from knowledge). Mechanism under (b) is main's: reuse the existing `Launch` deadline class as is (its schema word stays `Launch`) rather than a new class word.
- **ADR at the 10b commit.** D8 amends ADR-0369 decision 1 (launch requests = `Spawn*` and commands declared with the launch class — now also the long-running file commands, not only launches) and 10a's B17 (「every new command is `Ordinary`」); the same ADR amends ADR-0322 decision 1's wording — `library.emptyTrash` is declared in the agent crate's shared block (shell-only by the daemon's gate, not by a separate declaration), and `engram help library` carries one sentence saying emptying is the user's, without the name (D7 (b)).

### Main's default (reported)

- **D1** Names: `library.move` · `rename` · `copy` · `delete` · `restore` · `emptyTrash` as the parent; the view = **`library.listTrash`** (replaces the placeholder `library.trash`, which reads as a verb — `gio trash` moves to the trash). Convention: trash-cli `trash-list`, `gio trash --list`; this bus's `agent.listQueuedInputs`. One `emptyTrash` for empty-all and permanent delete (ADR-0408 decision 4), not trash-cli's separate `trash-empty` / `trash-rm`.
- **D2** Args: snake_case (10a B5); `to_folder` (the parent's `toFolder` — a folder, not the new path; coreutils `cp` / `mv` call it `--target-directory`); `rename`'s new name is `name` (as `agent.rename` and `library.newFolder`); `on_conflict` words `Overwrite` · `Skip` · `KeepBoth` (Windows Explorer's replace / skip / keep both, macOS Finder's Replace / Keep Both — from knowledge); restore and emptying address trash rows as `rows` (the parent's `items`); restore's choice is `keep_both` (it never overwrites — G11), not `on_conflict`.
- **D3** Per-item rows flat with `status` (§2.2); skip reasons as codes `inside-given-folder` · `in-place` · `name-exists` (reused) · `name-taken-during-copy`.
- **D4** Final codes = as built (§2.4); no rename (renaming touches the 9b tests and ADR texts for no LLM gain).
- **D5** The `CONFLICT` forms carry the wire spelling, fixed at the source (`DeletePlan::to_json`), not rewritten in `host.rs`.
- **D6** `library.emptyTrash` in the shared block + `SHELL_ONLY` list, checked in `call_daemon_command` by `caller == None`, omitted from the control list (§4). The caller stays `Option<ConnId>` (no explicit caller kind); the dependency is named in a comment and pinned by one test per door (§4.2).
- **D9** Catalog 9 kept; bindings regenerated per unit.
- **D10** Help additions as §6; no UI text, so no Korean key in 10b.
- **D11** The view nests mount → bundle → rows like the service and the screen's grouping (§6-5 2); `deleted_at` RFC 3339 UTC; `restorable` derived as §2.3.
- **D12** Agents in answers named by the handler from one roster read (10a B18).
- **D13** `CopyItemRow.trashed` is a list (0 or 1), as `MoveItemRow.trashed` (T-130 item 6 — the crate types differ).
- **D14** A text guard keeps `emptyTrash` out of `prompts/engram-help.md`, the priming and every `help` blob of `/control/commands`.
- **D16** The CLI posts a name its catalog read does not list to `/control/call` (D7 needs it; the daemon is the source of truth for reachability, as `run_invoke`'s doc already says for `callable:false`).
- **D15** Nothing is pre-added for the pending T-items (§7); their fields are additive in the same catalog.

## 11. Risks

- **R1** The agent's path has no guard against an overlapping repeat (D8). `/control/call` runs the daemon's own names on the spot — no seat, no request-id guard, a new id per CLI run — so an agent that repeats a move, copy, delete or restore while the first is still running (after `CONNECT_FAILED` `read failed` at 94 s, or 10 s for an ordinary command) starts a second operation. Across volumes that means a second staging copy; a first move then answering `source-changed` keeps a temp-named staging only the user can remove; a repeated copy with `KeepBoth` makes a second copy. After the first has ended a repeat is safe (a move answers `not-found` per item, a copy without `on_conflict` answers `conflict`). 10b narrows it only by the help text (§6). Closing it needs a seat for the daemon's write commands on the control route, or a service refusal of overlapping operations on the same paths — **outside 10b** (record for main: a tracking entry). Measured in C1.
- **R2** The shell-only guarantee is 「not listed, refused off the socket」 (accepted — G21); a future shell or webview command wrapping `library.emptyTrash` would reopen it (hand-over §4.4).
- **R3** The gate keys on `caller == None` (§4.2). A future door that hands `call_daemon_command` a connection number for a non-shell caller — as `CommandBus::invoke` already makes synthetic ones for connectionless callers, though it never runs the daemon's own names — would let `library.emptyTrash` through. Pinned by the comment at the check and the per-door tests.
- **R4** Answer size: a delete or move of many items, a long `mapping`, a trash of many bundles — all printed into the agent's context (ADR-0307 §2-4 hand-off undecided).
- **R5** Declarations and bindings grow (~7 commands, ~40 generated files); review cost. Mitigated by the unit split.
- **R6** The priming pointer line grows by a few words (codex command-line limit — 10a R5).
- **R7** The forms spelling change (§3) changes asset-library test expectations written for `detachRefs`.
- **R8** Until the client stage widens `reply_deadline_of` (§4.4), a screen `library.emptyTrash` of a large trash times out on the shell side at 30 s while the daemon finishes; the screen must re-read `listTrash` rather than resend.

## 12. Input claims corrected (parent `trd.md`, 9b TRD, brief → code at `9f166061`)

- **C1** Parent §1-7 / §5-3 `library.emptyTrash {items?}` — the service takes `bundles` (each with the rows the view listed) and `rows`; neither → `nothing-to-empty` (M86).
- **C2** Parent §1-7 `emptyTrash` row 「hands over to the daemon PC's OS trash and the OS decides」 and 「there is one root, so no root arg (G36)」 — withdrawn by ADR-0408 (direct removal) and by mounts (ADR-0409 M5: emptying takes no per-mount argument; every address carries its mount).
- **C3** Parent §1-7 `library.restore {items}` and `library.trash` — `rows` and `library.listTrash` (D1 · D2); `library.move {…, toFolder, …}` · `rename {…, newName, …}` · `detachRefs` placeholders → `to_folder` · `name` · `detach_refs`.
- **C4** Parent 「not in the agent command table」 — the declaration must sit in the daemon table `make_table` builds (the socket's stage 1 runs only that table); what it is absent from is the control route's list, `engram help` and the priming (§4.1). ADR-0322 decision 1's wording is amended to match by the ADR of §10 (D8).
- **C5** `DeletePlan::to_json` spells the detach form `detachRefs`; the wire argument is `detach_refs` — fixed in U1 (§3).
- **C6** 9b TRD §0 places the refusal 「at `call_daemon_command`'s `caller`」 — consistent with §4.2.
- **C7** 9b TRD §4 lists `other` among codes — in the code it is a view row kind (`RowKind::Other`); its list also predates three codes added while coding: `crosses-volumes`, `copy-merge-refused` (ADR-0416 · ADR-0417) and `nothing-to-empty` (U5).
- **C10** Draft 1 assumed the launch class gives every caller an 85 s reply bound and `OUTCOME_UNKNOWN`; on `/control/call` daemon names run without a seat, and the CLI's long wait is 94 s ending in `CONNECT_FAILED` `read failed` (§2). The shell treats every bus envelope as ordinary (`reply_deadline_of`).
- **C11** Draft 1 said the CLI needs no change; `run_invoke` refuses a name its catalog read does not list before posting, so a hidden name never reached the daemon (§4.2).
- **C12** T-131 as first recorded (a read-only junction row) is resolved by ADR-0418 decision 5; what stays open is a Unix 0555 folder inside a row and a read-only empty-folder row (§7).
- **C8** ADR-0416 「10b carries `staging`, `staging_disk`, `staging_row`, `staging_kept`, `not_copied`, `stagings`」 — they are keys inside reason `args`, not answer fields; 10b carries `args` as is (§2.2) — only `Skip::TakenDuringCopy` needs a row (`name-taken-during-copy`).
- **C9** Parent §1-7 copy row 「- 복사본」 and `engram:/…` — already corrected by the 9b TRD §11 (UD3 ` - Copy`, `engram/…`); repeated here because the 10b help text uses the corrected forms.

## 13. Review record

(empty)
