# TRD — S25 step 9a: root, first-start root, edits, create, relink (`S25: feat(asset-library,daemon): 루트 · 첫 시작 기본 루트 · 편집 · 새로 만들기 · 다시 연결`)

| Item | Value |
|---|---|
| **Status** | **Draft 1 (2026-11-16)** — not reviewed. Decisions and boundaries only; reconciled once at landing, then frozen. |
| **Canonical ADRs** | ADR-0311 (crate edges · ports · the daemon only wires) · ADR-0312 (one root · first-start default root) · ADR-0324 + ADR-0366 (library lock · per-item writes · exclusive root change) · ADR-0338 (draft + save · apply-now bus edits) · ADR-0358 (whole-file preset rewrite) · ADR-0341 (folder entries) · ADR-0329 (backend tag on preset items) · ADR-0330 (relink) · ADR-0335 (plan → execute → fix references) · ADR-0339 (same path in one list keeps the first place) · ADR-0315 + ADR-0362 (`LibPath`) · ADR-0365 (links skipped) · ADR-0291 (data-file rules) · ADR-0264 d4 / ADR-0271 (a file name used by one store belongs to that store) · ADR-0314 (the daemon owns the assets) |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 step 9a with 「ADR-0341 in 9a」 and the 「Handed over by step 7」 / 「Handed over by step 8」 paragraphs; §1-1 rows 1–2; §1-4 (assembly order, `config.rs` · `edit.rs` · `relink.rs` rows, flows ② ③, 「Root · default path」); §1-5 profile-edit port paragraph; §1-7 command rows and 「Common args · answer shapes」; §4 rows 「asset-library edit · reference · relink」 and 「daemon wiring」. |
| **Reference files to fix** | `docs/reference/architecture.md` (asset-library module entry — the daemon ↔ library wiring sentence) · asset-library `src/lib.rs` header + `LibraryValues` doc (U1 also corrects two pre-existing errors: the header says every folder operation goes through `folder::BoundedIo`, but `prepare_in`, the launch sweep / release and `Builtins::new` use std `fs`; `gate.rs` `Gate::change` doc names a profile-edit port the crate does not hold until U3) · daemon `src/data_dir.rs` module header · daemon `lib.rs` `DaemonWiring` doc (if it changes) · `docs/reference/structure/asset-library.md` (exists before 9a — written from the current code in its own docs commit) — the current-behaviour reference 9a's coders and reviewers read; **each unit updates it in the same commit as its code** whenever the unit changes or adds a contract it describes. |
| **Line-pointer basis** | `ca508879`. Code is cited by file + symbol; line numbers are hints. |

## 0. Scope

**Lands**:
- One library root, persisted, with a first-start default root.
- The daemon identity `daemon_id`.
- The root generation on edits.
- Stable `rev`.
- Every library / preset / agent-preset command **body**, as inherent `LibraryService` methods returning crate-local types (B2):
  - reads: `status` · `folders` · `read` · the `preset.get` view · `preset.check` · `previewFolder` · the `agent.presets` view
  - root: `set_root`
  - preset edits: `preset.add` · `remove` · `reorder` · `setBackend` · `save`
  - agent-preset edits: `attach` · `detach` · `reorder`
  - files: `newFolder` · `newFile` · `relink`
- The profile-edit port cell, filled by the daemon.

**Not here**:
- Command declarations (agent `declare_commands!`), `LibraryCommandHost` and its impl, CLI reachability, `engram help library`, `agent.new {presets}` — all step 10a. **Nothing new is callable by the CLI or an LLM after 9a**; the parent's 「verify by CLI」 (G20) starts at 10a.
- File operations, trash and restore (9b).
- Screens (11–13).

**Hand-overs received** (moved here from the parent item):
- **Step 7**:
  - `preset.save` carries origins — an item whose spelling changed must keep its `ItemOrigin`, or its map's other keys are dropped. This is the `PresetLists` caller contract in `format/write.rs`.
  - `relink.rs` leaves folder entries out (`refs` never sees a `FolderRef`).
  - `setRoot` and the first-start default root call `LibraryService::change_root` (ADR-0366 d3).
- **Step 8**:
  - The daemon keeps the plugged service as `DaemonWiring.library`.
  - `LibraryValues.root` is `None` until this step.
  - `daemon_id` becomes a value the service carries.
  - The profile-edit port (the manager) is filled late into the service before `command_table_slot.set` (parent §1-4 assembly order ② ④ ⑥).
- **ADR-0341 in 9a**:
  - The folder edits of `preset.add` · `remove` · `reorder`.
  - `preset.get` returns folders normalized and deduplicated.
  - `preset.save` · `check` take `folders?`.
  - `previewFolder` answers the normalized path and the `add` verdict.

## 1. Boundaries and ownership

- **Crate edges (ADR-0311)**:
  - asset-library depends on exactly agent · platform · command · base (CI gate 2), calls no daemon internals and has no `use tauri`.
  - The daemon only builds the service, plugs ports and passes values.
  - Nothing in 9a adds a crate edge.
- **Daemon owns**:
  - `identity.json` (name and format, in a daemon module — not `DataLayout`, ADR-0264 d4).
  - Where the data root came from.
  - The default-root place (`DataLayout` computes it).
  - Calling the first-start step.
  - Filling the port cell.
- **asset-library owns**:
  - `library.json` (name and format; joined onto the state folder the daemon hands in — parent §1-4 「What the daemon passes」).
  - The root state, every body listed in §0, `rev`, and the codes of §5.
  - The root generation lives in the gate's `Numbers.root` (parent §1-4 `config.rs` row), not in the config.
- **agent owns**:
  - `ProfileEditPort` and its `AgentManager` impl — unchanged by 9a.
  - The command vocabulary. 10a maps 9a's types onto it.
  - asset-library never mutates `ProfileRegistry` (M18); it reads it directly (M5).
- **`DataLayout` stays computation only** (`data_dir.rs` header). It gains the data-root origin and a pure `default_library_root()`; it creates nothing (B22).
- **No wire or ts-rs change**:
  - 9a touches no protocol type and no command declaration, so the binding drift gate does not fire.
  - The 9a types are Rust-only until 10a maps them.
- **Answer and refusal shape at the 10a seam**:
  - Bodies take an `AgentId`, not a name (the agent name is resolved in `make_table` — parent §1-5).
  - A refusal is a crate-local value holding a **class** (`Conflict` · `NotFound` · `InvalidArgument` · `Unavailable` · `Internal`) plus a `Reason {code, args, line}` (agent `launch::Reason`). 10a maps the class onto the bus code and passes the `Reason` as args.
  - Per §4 of `docs/reference/llm-command-conventions.md`, answers carry the post-change state: every preset edit answers the `preset.get` view, every agent-preset edit the `agent.presets` view.

## 2. Data shapes

**Persisted — `<data>\daemon\state\library.json`** (base data-file rules, ADR-0291, following agent `persistence/mod.rs`):
- `{"schema_version": 1, "root": "<absolute normalized path>"}`.
- `Spec`: `version_key = "schema_version"`, `current = 1`, a small cap (64 KiB). The shape check refuses a document whose `root` is not an absolute path string.
- Normalization = platform `normalize_folder`.
- Load outcomes:

| Load result | Root state | Writes this run |
|---|---|---|
| Missing (never written) | first-start default (§3.2) | yes |
| Usable | that root | yes — re-`judge` before each write |
| Corrupt | `unset` | allowed: `CopyAsideFirst`, i.e. `<name>.corrupt` |
| Newer / read failed | `unset` | **no** — `set_root` refused with `library-config-read-only` (ADR-0291 R17, as the profile store does) |

**Persisted — `<data>\daemon\state\identity.json`** (daemon; same rules):
- `{"schema_version": 1, "daemon_id": "<uuid v4, lowercase hyphenated>"}`.

| Load result | `daemon_id` this run |
|---|---|
| Missing | generate one and write it |
| Corrupt | copy aside, generate, write |
| Newer / read failed | a fresh id for this run only, not written, plus a `warn` (B3) |

**Values the daemon passes** (`LibraryValues`):
- `root: Option<PathBuf>` is replaced by:
  - `config: LibraryConfig` — the loaded `library.json` outcome plus the state folder it lives in.
  - `default_root: PathBuf` — `DataLayout::default_library_root()`; also `suggested_new`.
  - `daemon_id: String`.
- The config is read before `new` by an asset-library function the daemon calls (`config.rs`), so `LibraryService::new` keeps doing no disk work beyond the built-ins (parent §1-4 assembly order ②).
- `build_library_service` (daemon `lib.rs`) and the harness bin `roundtrip_smoke.rs` pass the same values.

**Data-root origin** (daemon):
- The branch `data_dir::default_data_dir` already takes (`ENGRAM_DATA_DIR` · debug walk-up · release `<exe>\data`) is returned with the path by a sibling function, and `DataLayout` carries it.
- `default_data_dir()` keeps its path rule. The shell's copy (`src-tauri/src/discovery/layout.rs`) is unaffected.
- `default_library_root()`:
  - release default ⇒ `<data root's parent>\library`
  - otherwise ⇒ `<data root>\library`
  - never guessed from a directory name (parent L236 · ADR-0312 d5)

**In memory**:
- `ItemContent::Preset` gains `rev` (B7), computed from the bytes the item was made from — scan, late merge, edit or create — so `preset.get` answers from the catalog with no disk read (flow ①).
- The profile-edit cell: `OnceLock<Weak<dyn ProfileEditPort>>` (B17).

**Answer types** (crate-local; names are code detail):

| Body | Answer |
|---|---|
| status | `{daemon_id, root, state: unset \| ok \| missing (+ why), suggested_new, root_generation}` |
| `preset.get` view | per-list rows with state · conflict rows · problem list (incl. inner presets with their `via`) · `folders` (normalized, deduplicated, with status) · `rev` · count of referencing agents |
| `agent.presets` view | parent §1-7 row, incl. `workspace_lock` through the port and agent `baseRev` |
| multi-add | `{added, skipped: [{item, reason}]}` |
| relink | `{fixed: [(old, new)], more: n}` |
| create | the new path |
| `folders` · `read` · `previewFolder` | per the parent §1-7 rows |

## 3. Behaviour

### 3.1 Root, status, identity

- **`status`**:
  - `state` = `unset` (no root) · `ok` · `missing` (the root stat answers anything but `Ok` — missing, not a folder, unreadable, timeout — with `why`).
  - The stat runs on one read budget (`BoundedIo::begin_read`, 5 s), outside the lock.
  - `root_generation` = `Numbers.root`.
- **`set_root {path, create?}`** (B5): see the sequence below.
- **Root change effects**:
  - Agent attachments and preset lines are untouched; the same paths are looked up in the new root (Δf).
  - The old root's `.trash` stays where it is.
  - `relink` never crosses roots.

`set_root` sequence:
1. Normalize `path` and require it absolute (else `InvalidArgument` · `item-path-invalid`).
2. Take the service's root-change mutex.
3. If `create`, create the folder and its ancestors.
4. Write `library.json` (`judge` → atomic write).
5. `change_root`.

`change_root` refuses in two ways, publishing nothing: the stat does not answer `Ok`, or the walk fails at the root. On refusal, the previous `library.json` is restored — the old content written back, or our just-written file removed when none existed — and the refusal is answered (`Unavailable` · `library-root-unusable` with `why`). If the restore itself fails, the answer carries both errors and the file's root wins at the next start (risk K3). A `library.json` write failure answers an error with nothing changed.

### 3.2 First start (B4)

- When `library.json` is **Missing**, the daemon (production wiring only — the test server builds no service) calls one asset-library method once, after building the service and before `command_table_slot.set`.
- The method creates `default_root`, writes `library.json`, then `change_root` — the walk of an empty folder.
- If creation, the write or `change_root` fails: state `unset`, nothing written (a just-written file is removed as in §3.1), the daemon starts, and the next start tries again.
- A root that was written and later vanished is `missing` and is **never** re-created (ADR-0312 d5 · ADR-0366 d3).

### 3.3 Preset edits (`preset.add` · `remove` · `reorder` · `setBackend` · `save`)

**One pipeline**:
1. First-scan entrance (`first_scan_join`); an unset or unusable root → `Unavailable` · `library-unavailable`.
2. Folder entries to add are stat'ed **before** the change, outside every lock (no stat under the library lock — parent §1-3).
3. `change` with the optional root generation, checked inside the same write-lock take (B6).
4. Inside the change:
   - Read the file's current bytes through the store (library cap). A missing file or parent → `NotFound` · `not-found` (no re-creation — M76-REVISED-2, M65).
   - Parse.
   - Apply the operation to `PresetLists`, keeping every kept item's origin.
   - `format::rewrite_lists`.
   - `store.write_atomic`.
   - Apply the item made from the written bytes to the catalog (`Catalog::apply_items`, seq drawn inside the lock).
5. Answer the `preset.get` view.

**Writer refusals**: `preset-write-*` and the reader's `kind-unknown` · `wrong-kind` (`WriteError::to_reason`) answer `InvalidArgument` with the file untouched.

**M11**: whenever the change read bytes that differ from the catalog item, it applies them to the catalog **also when it answers a refusal**, so the same `CONFLICT` does not repeat after an outside edit.

**Per operation**:

| Operation | Rules |
|---|---|
| `add {items \| folders, after?}` | **Items**: partial apply (Δ7). Skipped with a reason: the save mismatches below; a library path not in the catalog (`not-found`, B12); already in the list (`already-present`, by path key; built-ins by id). Missing `after` ⇒ append (M14). Optional `backend` only on instruction and skill items (G29 · M20). **Folders**: normalize; refuse relative · cmd-unsafe (`path_is_cli_safe`) · missing · not a folder · unreadable · stat timeout — **all or nothing** across the call, naming each bad path; an already-present folder (by normalized `path_key`) is skipped, not refused. `items` and `folders` never mix in one call. |
| `remove {items \| folders}` | Idempotent, no edition check (M24-8). A folder matches by normalized `path_key` and drops every spelling. |
| `reorder {section, order, baseRev}` | `baseRev` ≠ the current `rev` ⇒ `Conflict` · `rev-mismatch` `{rev}`. `order` must be a permutation of the section as `preset.get` shows it (folders: deduplicated and normalized) — else `InvalidArgument` · `reorder-order-mismatch`. The folders section writes the normalized, deduplicated list. |
| `setBackend {items, backend?}` | Instruction and skill items only (`backend-tag-not-allowed` otherwise). A word the registry does not know ⇒ `backend-tag-unknown`. Omitted = common. |
| `save {instructions, skills, settings, presets, folders?, baseRev?}` | Wholesale replace. A supplied `baseRev` that differs ⇒ `Conflict` · `rev-mismatch`. **All or nothing** — any mismatch refuses the whole save with `InvalidArgument` · `preset-save-refused` `{items: [{list, path, code}]}`, file untouched. The mismatches: `preset-cycle` · `wrong-kind` · `backend-tag-not-allowed` · `backend-tag-unknown` · `item-path-invalid` · `builtin-in-library-preset` · `item-duplicate` (one target twice in a list, regardless of tag — ADR-0339) · `folder-path-relative` · two folders equal after normalization. A missing path or a missing folder is legal content (M78-REVISED); a cmd-unsafe folder is legal content here (it blocks at launch). `folders` omitted = unchanged. Fields outside the lists are never touched. |

**`preset.check {…, folders?}`**: the same validation and the same resolution function over the draft — the catalog lookup overlaid with the draft preset. No write; file, catalog and `rev` unchanged. The answer has the `preset.get` view's conflict and problem shape.

### 3.4 Read bodies (B2)

| Body | Rule |
|---|---|
| `preset.get` | `resolve_preset` plus the catalog item's `rev`, the normalized and deduplicated folder list (first place kept, as resolution does), and the referencing-agent count (`refs::referrers_of`). The read never rewrites the file. |
| `folders` | The folder tree from `Catalog::folders` plus skill items. |
| `read {path}` | Item detail per parent §1-7, incl. the absolute path, direct and indirect references, state, and the instruction excerpt cap. |
| `previewFolder {folder, backend?, mode?}` | Read only, bounded. Returns: the normalized path; the `add` verdict (the same checks and codes as a folder `add`); per backend and mode, what would load — through the same port answers `launch.rs` uses (`folder_load_set` · `native_skill_roots` · `root_skills`). |
| `agent.presets {agent}` | `resolve_agent` plus `workspace_lock` through the port (cell required) and agent `baseRev` (B15). |

Every body except `previewFolder` passes the first-scan entrance first.

### 3.5 Agent-preset bodies (`attach` · `detach` · `reorder`)

- **Order**: validate against the catalog, then write through the port **inside one `change`** — the port's last-write-wins contract relies on all writers queueing under the library write lock (`profile_edit.rs` `set_attachments` doc).
- **What an agent takes**: presets only. A library preset path, or `engram:/default` meaning the 「기본」 slot (`Attachments.default_preset`) (B16).
- **`attach {presets, after?}`**: partial apply. Skips: not a preset (`wrong-kind`), not in the catalog (`not-found`), already attached (`already-present`), not a reference (`item-path-invalid`).
- **`detach {presets, yes?}`**: removing `engram:/default` without `yes` ⇒ `Conflict` · `default-preset-detach`.
- **`reorder {order, baseRev}`**: a `baseRev` mismatch ⇒ `Conflict` · `rev-mismatch`.
- **Unknown agent**: a port `NotFound` ⇒ `NotFound` · `not-found`.
- **Port unavailable**: an empty or dead cell ⇒ `Internal` · `profile-edit-unavailable` — never a silent Noop.
- **Answer**: the `agent.presets` view.

### 3.6 Relink and create

**`relink {from, to}`** (ADR-0330):
- Both must be `LibPath`s (else `item-path-invalid`).
- `to` must be in the catalog (else `NotFound` · `not-found`).
- `to`'s kind must equal the kind of the list(s) that reference `from` (else `InvalidArgument` · `relink-kind-mismatch` `{expected, found}`).
- `from`'s own presence is not checked (no added condition).
- Inside one `change`:
  - (a) Every reference to `from` (path key) becomes `to`.
  - (b) For every other target not in the catalog whose parent equals `from`'s parent: if `parent(to)/<its name>` is in the catalog with the kind its referencing list needs, it is fixed too and counted in `more`.
  - Preset files are rewritten one at a time (pipeline of §3.3, origins kept, a duplicate keeps its first place — ADR-0339).
  - Then agents go through `rewrite_preset_paths(renames)` once (ADR-0335 order).
- Folder entries are never touched.

**`newFolder {parent?, name}`** · **`newFile {folder?, name, kind}`**:
- `name` is one `LibPath` segment. `.md` (markdown) or `.yaml` (settings, preset) is appended unless the name already ends with `.md` or with `.yaml` / `.yml`.
- Templates: markdown = empty; settings / preset = `kind: <kind>` + an empty `description: ` line.
- Refusals: an existing name ⇒ `Conflict` · `name-exists`; a missing parent ⇒ `NotFound` · `not-found`; a dot or temp name ⇒ `item-path-invalid`.
- The new item enters the catalog in the same `change`.

## 4. Concurrency

- **Root changes**:
  - `change_root` is exclusive from its ticket to its publish. Edits queued behind it run against the new root.
  - A supplied root generation is compared inside the edit's own write-lock take (B6), so an edit can never write into a root it did not ask for.
  - `set_root` calls are serialized by one service mutex taken across the config write, `change_root` and the rollback. Only `set_root` and the first-start step take it, and they take nothing else under it except the gate (no cycle).
- **Edits**:
  - Each is one `change`: ticket, write lock, disk read and write inside, per-item `apply_items`, change generation raised before release (ADR-0366 d1 writer 5). This satisfies `LibraryService::change`'s rule that the disk effect is finished inside.
  - Readers wait out one file write. The library is local by assumption (`lib.rs` header).
  - Folder stats and `previewFolder` listings run outside every lock.
- **Lock order**: library lock → manager / registry locks, never the reverse. Port calls happen inside `change`. The manager calls `check_in` holding none of its locks (`manager/prelaunch.rs` header), and the port impl never calls back into the library.
- **Outside edits**: an outside editor changing the file between the read and the write inside one change loses to the change (last write wins). This is the same as the notepad rule (G53).
- **Launch merges**: a late launch merge drawn before an edit is discarded by the raised change generation (ADR-0324 d5).

## 5. Codes (main's choice — §8 B8)

Pattern: kebab-case, `pub const` beside the code that refuses (as `resolve.rs` · `format/read.rs`), each with its `args` in the doc comment. Existing codes are reused where they fit.

| Case | Class | Code | args |
|---|---|---|---|
| Supplied root generation stale | Conflict | `root-generation-stale` (new) | `{given, current}` |
| Root unset or unusable for an edit or read | Unavailable | `library-unavailable` (existing) | `{message}` |
| `set_root` target unusable after create | Unavailable | `library-root-unusable` (new) | `{path, why}` |
| `library.json` read-only this run | Unavailable | `library-config-read-only` (new) | `{why}` |
| `rev` / agent `baseRev` mismatch | Conflict | `rev-mismatch` (new) | `{rev}` (current) |
| Save mismatches | InvalidArgument | `preset-save-refused` (new) | `{items: [{list, path, code}]}` — inner codes existing + `item-duplicate` (new) |
| Folder `add` refused | InvalidArgument | `preset-folders-refused` (new) | `{folders: [{path, code, why?}]}` — inner: `folder-not-found` (existing; missing · not a folder · unreadable · timeout, with `why`) · `folder-path-cli-unsafe` (existing) · `folder-path-relative` (existing) |
| Skip reasons (multi-add, attach) | — | `not-found` · `wrong-kind` · `item-path-invalid` · `backend-tag-not-allowed` · `backend-tag-unknown` · `preset-cycle` (existing) · `already-present` (new) | per the inner code |
| `reorder` order not a permutation | InvalidArgument | `reorder-order-mismatch` (new) | `{expected}` |
| Detach 「기본」 without `yes` | Conflict | `default-preset-detach` (new) | `{agent}` |
| Create: name exists | Conflict | `name-exists` (new) | `{path}` |
| Relink: kind differs | InvalidArgument | `relink-kind-mismatch` (new) | `{from, to, expected, found}` |
| Port cell empty or dead | Internal | `profile-edit-unavailable` (new) | `{}` |
| Writer refusals | InvalidArgument | `preset-write-mismatch` · `preset-multi-document` · `preset-write-number-range` (existing) | `WriteError::to_reason` |

## 6. User-visible changes

- **First start creates the default library folder**: `<dist>\library` (release), `<repo>\.engram-dev\library` (debug) or `<ENGRAM_DATA_DIR>\library`. Decided — G35 · ADR-0312 d5. G39 accepts that a distribution rebuild deletes `<dist>\library`.
- **Two new state files**: `daemon\state\library.json` and `daemon\state\identity.json`.
- **Launches**: an agent whose profile attaches a library preset gets a real resolution instead of block ④. No create entrance attaches one before 10a, so this is reachable only through a hand-edited profile file.
- **No command, CLI or screen change.**

## 7. Implementation units (each green on its own, in order)

| Unit | Content | Files | Hands to the next | Size (rough, incl. tests) |
|---|---|---|---|---|
| **U1** root · status · identity (+ two catalog reads) | §2 persisted shapes; `LibraryConfig` load; `LibraryValues` replacement; `status`; `set_root` with mutex + rollback; first start; the root-generation check on `change` (B6); the data-root origin + `DataLayout::default_library_root`; `identity.json`; wiring; the `folders` and `read` bodies (read-only over the catalog, no overlap with U2's module) | asset-library `config.rs` (new), `lib.rs`; daemon `data_dir.rs`, `lib.rs` (+ identity module), `bin/roundtrip_smoke.rs` | `status`, `set_root`, a root-generation-checked `change`; `LibraryValues {config, default_root, daemon_id}` | ~1.6k |
| **U2** preset view + edits | `rev` on the preset item (catalog read paths); the `preset.get` view; the §3.3 pipeline; `add` · `remove` · `reorder` · `setBackend` · `save` · `check`; `previewFolder` (shares the folder-add verdict); the refusal type (class + `Reason`) | asset-library `edit.rs` (new; a view module if it grows), `catalog.rs`, `lib.rs` | the edit pipeline helper (read latest → `rewrite_lists` with origins → write → `apply_items`), the `preset.get` view, the refusal type | ~2.2k |
| **U3** port cell + agent-preset bodies | the cell + its daemon fill; `agent.presets` view + agent `baseRev`; `attach` · `detach` · `reorder` | asset-library `lib.rs` (+ an agent-edit module); daemon `lib.rs` | the filled port | ~0.9k |
| **U4** relink + create | `relink`; `newFolder` · `newFile` | asset-library `relink.rs`, `create.rs` (new), `lib.rs` | — | ~1.3k |

- U1 and U3 both touch daemon `lib.rs` and asset-library `lib.rs`; U4 uses U2's pipeline and U3's cell — strictly sequential.
- Each unit fixes the reference files it makes wrong in its own commit:
  - U1: `data_dir.rs` header, `LibraryValues` doc, `lib.rs` header
  - U3: `architecture.md` sentence, `DaemonWiring` doc if changed
  - every unit: `docs/reference/structure/asset-library.md` for the contracts it changes or adds (U1 root config · status · identity values; U2 edits · `rev` · refusal shape; U3 port cell · agent-preset bodies; U4 relink · create)
- ADRs for decisions later work relies on (§8 (b) B3 · B6 · B7 · B8 · B17) are written by main after the PRD+TRD review, before coding (implement binding).

## 8. Decisions

**(a) Already decided — cite only.**
- One root, with a first-start default root at the place computed from the data-root origin; uncreatable ⇒ `unset` (ADR-0312 d1 · d5, G35, M24-2).
- `set_root` and the first start go through `change_root` (ADR-0366 d3).
- Per-item catalog writes under ticket and generation (ADR-0366 d1).
- Draft + save; apply-now bus edits; `reorder` is the only edition-checked bus edit; `save`'s optional `baseRev` (ADR-0338 d4 · d9, M84).
- Save all or nothing; missing paths legal (M78 · M78-REVISED · M85).
- Whole-file rewrite (ADR-0358).
- Folder-entry rules (ADR-0341, parent 「ADR-0341 in 9a」).
- Relink = one pick, the rest by name, same kind (ADR-0330).
- References fixed presets-then-agents; duplicates keep the first place (ADR-0335 · ADR-0339).
- `rev` is a stable hash of the bytes (M21).
- The root generation is an optional argument (M14 · M26).
- Agent edits only through the port (M18).
- `daemon_id` owned by the daemon and carried as a value (ADR-0311 d5).

**(b) Main decides — recommendation + reason.**
- **B1** This step has its own TRD file; the parent item becomes a status line + link at the step's freeze. — the step's size (four units) and the decisions below do not fit a parent row.
- **B2** 9a builds every library, preset and agent-preset command body (incl. the reads `status` · `folders` · `read` · `preset.get` · `previewFolder` · `agent.presets`) as inherent `LibraryService` methods returning crate-local types; 10a only declares and maps. — before this, neither step owned the read bodies, and edits must answer the `preset.get` shape anyway; follows the step-7 `list` / `ListAnswer` precedent.
- **B3** `library.json` (asset-library) and `identity.json` (daemon) follow the base data-file rules (ADR-0291) as agent `persistence/mod.rs` does; a newer or unreadable `library.json` makes `set_root` read-only for the run; an unwritable or newer `identity.json` gives a run-only id + `warn`. — one rule for every data file; a run-only id is the least harm (it only orphans client drafts for that run).
- **B4** An uncreatable first-start root ⇒ `unset`, `library.json` not written, the daemon starts, the next start retries; a vanished written root stays `missing`. — matches ADR-0312 d5 / M24-2 and does not freeze a failed attempt into the file.
- **B5** `set_root` = create (if asked) → write `library.json` → `change_root`; a refusal restores the previous file; one mutex serializes `set_root`. — the persisted root is never one that was not asked for; the mutex prevents two calls from interleaving file and publish order.
- **B6** The root generation enters edits as an optional argument checked inside the edit's own write-lock take (a root-checked sibling of `LibraryService::change`); stale ⇒ `root-generation-stale`. — a check before the ticket could pass and then write after a queued root change.
- **B7** `rev` = lowercase-hex SHA-256 of the exact file bytes (BOM included), kept on the catalog preset item. — stable across restarts (M21); `sha2` is already a dependency; `preset.get` stays a memory read.
- **B8** The code strings of §5. — kebab-case beside the refusing code, reusing existing codes; the parent left them to implementation; the user delegated command details and wording to main (`llm-command-conventions.md` §1 header, 2026-10-26).
- **B9** T-106 P8 is **not** fixed in 9a; it stays in tracking and is risk K1. — the root state needs a stat that must stay outside the library lock (parent §1-3), so root and catalog cannot be read in one take. The narrow fix (note `Numbers.root`, recompute when it moved) adds a retry state, against the user's 2026-11-12 criterion; the effect is one transient read answer.
- **B10** Four sequential units, U1 → U4 (§7). — file overlap in both `lib.rs` files; U4 needs U2 and U3.
- **B11** Refusals are `{class, Reason}` and 10a maps the class onto the bus code. — keeps the vocabulary in agent (parent §1-5) while 9a fixes codes and args.
- **B12** A multi-add skips a library path that is not in the catalog (`not-found`) and an already-present item (`already-present`), like `attach`. — a typo guard matching the folder `add`; `save` keeps missing paths legal because it replaces a draft that may hold 「찾을 수 없음」 lines.
- **B13** Edit disk IO runs inside the change's write lock; folder stats run before it, outside. — the `change` contract requires the disk effect inside; no stat under the library lock.
- **B14** An edit that read new bytes applies them to the catalog even when refusing (M11).
- **B15** Agent `baseRev` = lowercase-hex SHA-256 of the canonical JSON `{"default": <bool>, "presets": [<paths in order>]}`. — stable, order-sensitive, read straight from the registry.
- **B16** `engram:/default` in agent-preset commands names the 「기본」 slot (`Attachments.default_preset`). — the profile stores it as a bool; this spells it as the reference users and LLMs already see.
- **B17** The port cell is `OnceLock<Weak<dyn ProfileEditPort>>`, set once in `build_daemon_wiring_with_store` right after `Arc::new(manager)`; an empty or dead cell ⇒ `profile-edit-unavailable`. — no `Arc` cycle; filled before `command_table_slot.set` (parent §1-4 ⑥); no silent Noop.
- **B18** The daemon calls an asset-library `config.rs` load before `LibraryService::new`, and calls the first-start method after building the service, in production wiring only. — `new` stays free of disk work beyond the built-ins (parent §1-4 ②); the test server builds no service.
- **B19** `relink` checks `to` (present, same kind) but not `from`. — no added condition; a present `from` only re-points references, and part (b) touches only missing targets.
- **B20** Create appends the kind's extension unless present; refuses an existing name. — one call creates a usable file; no overwrite path outside 9b's Explorer rules.
- **B21** `status.state = missing` covers every non-`Ok` root stat, with `why`. — the parent defines three states; `why` keeps the cause.
- **B22** The data-root origin comes from the branch `default_data_dir` already takes (a sibling function returning it); `DataLayout` carries it and computes `default_library_root` without creating anything. — no guessing from names (parent L236); the shell copy keeps its path-only rule.

**(c) User decisions.** None — see §12.

## 9. Test plan

All units: 전체 회귀 (qa binding row 「Rust 소스」). `crates/engram-dashboard-asset-library/src/**` and daemon `src/lib.rs` are in the **claude and codex lane** path lists, so every unit also runs both lanes (cost — announce per qa binding 「비용 있는 검사」). No GUI scene: no front or shell change.

| Unit | Automated (new) | Mutations QA must hit |
|---|---|---|
| U1 | `library.json` load outcomes (missing · usable · corrupt · newer · read failure) and write policies; first start creates, writes and publishes; an uncreatable place ⇒ `unset`, no file, a second start retries; a written-and-vanished root ⇒ `missing`, not re-created; default place per origin (env · debug · release — `ENGRAM_DATA_DIR` isolation never touches the real folder); `set_root` order, rollback on both `change_root` refusals, concurrent `set_root` calls serialize; a stale root generation is refused inside the lock (an edit queued behind a root change); `identity.json` cases; `status` fields; one daemon wiring test over a temp `DataLayout`; `folders` / `read` rows | rollback removed; the mutex removed; the generation check moved before the ticket; the default place guessed from a name |
| U2 | the parent §4 row 「asset-library edit · reference · relink」 for edits: idempotent `add` / `remove`; `reorder` `rev-mismatch`; save all-or-nothing with every mismatch code; missing path legal; moved target `not-found` and no re-creation; origins kept (an item's other keys survive a respelling); `description` untouched; writer refusals leave the file untouched; M11 under a refusal; `rev` equal across a service rebuild; `check` writes nothing; folder add all-or-nothing, matching by `path_key`, normalized storage; `preset.get` folder dedup; `previewFolder` verdict = the `add` verdict | M11 dropped on refusal; a missing-target write re-creates the file; origins dropped; a folder stat moved under the lock |
| U3 | attach / detach / reorder only through the port (a fake port asserts the calls and that each call comes from inside a change); 「기본」 detach without `yes`; agent `baseRev` order sensitivity; an empty cell ⇒ `profile-edit-unavailable`; the daemon fills the cell before the table is set (source-order guard) | the registry mutated directly; the cell check folded into a Noop |
| U4 | relink (a) + (b) with same-kind-only, `more` counted, presets then agents, duplicate first place, folder entries untouched, no cross-root (parent §4 row); create templates, extensions, `name-exists`, the catalog insert | kind check removed; agents rewritten before presets |

## 10. Risks

| # | Risk | Caught by |
|---|---|---|
| K1 | T-106 P8: a read computation pairs the root state from before a `set_root` with the catalog after it — one transient wrong answer (e.g. block ④ for an agent whose new root is fine). Reachable from 9a on. | accepted (B9); tracking T-106 stays |
| K2 | An edit holds the library write lock across one file read and one write — on a slow disk, readers and launch checks wait. | the library is local by assumption (`lib.rs` header); not measured |
| K3 | `set_root` rollback fails after a refused `change_root` — `library.json` names a root that is not live until restart. | the answer carries both errors; U1 test with write-failure injection |
| K4 | A run-only `daemon_id` (unreadable or newer `identity.json`) orphans the client's drafts for that run. | `warn` at boot; drafts are a client-stage feature |
| K5 | The app's WMI-started dev daemon (no `ENGRAM_DATA_DIR`) creates `<repo>\.engram-dev\library` on first start. | intended (debug default place); inside the ignored dev folder |
| K6 | The lane cost of every unit (claude + codex). | qa binding announcement rule |

## 11. Input claims corrected (parent `trd.md` → code / ADR at `ca508879`)

- §2-4 item 4 (single-preset tag conflicts) is listed **open** → closed by the user 2026-11-11 (U5 (a), recorded with step 7). The parent's §2-4 should mark it closed at freeze.
- §1-4 assembly order cites daemon `lib.rs` line numbers (`:609 → :651`, `:660`, `:666`, `:772`) → today the MCP start is before `build_daemon_wiring`, then `command_table_slot.set`, then the accept loop. The order holds; the numbers are stale.
- ADR-0338 d6 names `yaml-edit` as the writer → superseded by ADR-0358 (README records it). ADR-0338 d3 / d4 lack `folders?` → added by ADR-0341, which the README status of ADR-0338 does not record.
- L236 「`DataLayout` also holds where the root came from」 → `DataLayout` only computes paths and today does not know the origin (`default_data_dir` returns a bare path). The origin is returned by a sibling of `default_data_dir` and carried by `DataLayout`; folder creation stays outside `DataLayout` (B22).
- 9a row 「The four block reasons all stand here」 → all four already exist (agent `launch.rs` `BlockReason`; ④ since step 7). 9a keeps them true through edits and adds none.

## 12. Open — user

None. Every user-visible choice in 9a is decided by an ADR or a G-number (§8 (a)), or is a command detail the user delegated to main (`llm-command-conventions.md` §1 header).

## 13. Review record

(empty)
