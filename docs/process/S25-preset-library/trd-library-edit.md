# TRD — S25 step 9a: root, first-start root, edits, create, relink (`S25: feat(asset-library,daemon): 루트 · 첫 시작 기본 루트 · 편집 · 새로 만들기 · 다시 연결`)

| Item | Value |
|---|---|
| **Status** | **Implemented 2026-11-19** — U1 `7e764daa` · U1b `8b0ad3e3` · U2 `2fc21c12` · U3 `6aae4b76` · U4 `bda7d849`; coding-time rules = ADR-0384 (it partly amends ADR-0380 and supersedes §3.6 「`stopped: {code}` with no `path`」 for `profile-edit-unavailable`); unfixed review findings = tracking T-117. Body frozen at draft 8. — **Draft 8 (2026-11-18) — U-D3 confirmed by the user (option (a)); supersedes the 2026-10-17 no-pre-scan rule.** Decisions and boundaries only; reconciled once at landing, then frozen. |
| **Canonical ADRs** | ADR-0311 (crate edges · ports · the daemon only wires) · ADR-0312 (one root · first-start default root) · ADR-0324 + ADR-0366 (library lock · per-item writes · exclusive root change) · ADR-0338 (draft + save · apply-now bus edits) · ADR-0358 (whole-file preset rewrite) · ADR-0341 (folder entries) · ADR-0329 (backend tag on preset items) · ADR-0330 (relink) · ADR-0335 (plan → execute → fix references) · ADR-0339 (same path in one list keeps the first place) · ADR-0315 + ADR-0362 (`LibPath`) · ADR-0365 (links skipped) · ADR-0291 (data-file rules) · ADR-0264 d4 / ADR-0271 (a file name used by one store belongs to that store) · ADR-0314 (the daemon owns the assets) |
| **Parent TRD** | [`trd.md`](trd.md) §5-2 step 9a with 「ADR-0341 in 9a」 and the 「Handed over by step 7」 / 「Handed over by step 8」 paragraphs; §1-1 rows 1–2; §1-4 (assembly order, `config.rs` · `edit.rs` · `relink.rs` rows, flows ② ③, 「Root · default path」); §1-5 profile-edit port paragraph; §1-7 command rows and 「Common args · answer shapes」; §4 rows 「asset-library edit · reference · relink」 and 「daemon wiring」. |
| **Reference files to fix** | `docs/reference/architecture.md` (asset-library module entry — the daemon ↔ library wiring sentence) · asset-library `src/lib.rs` header + `LibraryValues` doc (U1 also corrects two pre-existing errors: the header says every folder operation goes through `folder::BoundedIo`, but `prepare_in`, the launch sweep / release and `Builtins::new` use std `fs`; `gate.rs` `Gate::change` doc names a profile-edit port the crate does not hold until U3) · daemon `src/data_dir.rs` module header · daemon `lib.rs` `DaemonWiring` doc (if it changes) · U1b: asset-library `lib.rs` header 「The root」, the `FirstStart` / `first_start` / `change_root` docs, `gate.rs` header 「First scan」, the `BoundedIo::spawner` doc · `docs/reference/structure/asset-library.md` (exists before 9a — written from the current code in its own docs commit) — the current-behaviour reference 9a's coders and reviewers read; **each unit updates it in the same commit as its code** whenever the unit changes or adds a contract it describes. |
| **Line-pointer basis** | `7e764daa` (U1 landed). Code is cited by file + symbol; line numbers are hints. |

## 0. Scope

**Lands**:
- One library root setting, persisted, whose default value is the data-root-based place (created at first start — U-D1).
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
  - `setRoot` and the first-start default root call `LibraryService::change_root` (ADR-0366 d3). `setRoot` still does; the first start no longer walks — it stats and publishes the root, and the background boot scan walks it (U-D3, §3.2).
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
  - Calling the first-start step, and then starting the background boot scan (U-D3, §3.2).
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
  - A refusal is a crate-local value holding a **class** (`Conflict` · `NotFound` · `InvalidArgument` · `Unavailable` · `Internal`) plus a `Reason {code, args, line}` (agent `launch::Reason`). 10a maps the class onto the bus code and passes the `Reason` as args. One more variant passes a port store error through unchanged: it holds the agent `StoreError`, and 10a answers it with agent `store_command_error` + `store_error_reason(PROFILES_FILE, …)` as every other registry-writing command does — class, code and retry mode (`RetryMode::AfterCondition` for an unreadable registry) are the agent's, not 9a's.
  - **Class → bus code is fixed here (B11)**: `Conflict` → `CONFLICT`, `NotFound` → `NOT_FOUND`, `InvalidArgument` → `INVALID_ARGUMENT`, `Unavailable` → `CONFLICT` (command `ErrorCode` has no unavailable code; precedent: agent `commands.rs` maps `PtyError::LaunchBlocked` to `CONFLICT`), `Internal` → `INTERNAL`. Rule: **every 9a refusal that applied nothing to the library or the registry maps onto a code daemon `command_delivery::retains_the_id` does not retain** (`INVALID_ARGUMENT` · `NOT_FOUND` · `CONFLICT`), so a retry with the same request id is not answered `ALREADY_APPLIED`. One leftover does not count as applied: folders a `set_root {create}` made (wholly or partly) before it refused `library-root-unusable` — the create step itself failing, or `change_root` refusing. A `library-config-write-failed` from `set_root` comes **after** the new root was published (§3.1, U-D2), so it did apply something (the live root); it still takes `Unavailable` → `CONFLICT`, which does not retain the id — a retaining code would answer a same-id retry `ALREADY_APPLIED` and never retry the save the caller was told failed, while a re-run republishes the same root and writes again. There are **two** exceptions, both `INTERNAL`: a port store write IO failure (passed through from agent `store_command_error`) and an empty or dead port cell (`profile-edit-unavailable`, a wiring bug) (§5).
  - Per §4 of `docs/reference/llm-command-conventions.md`, answers carry the post-change state: every preset edit answers the `preset.get` view, every agent-preset edit the `agent.presets` view.

## 2. Data shapes

**Persisted — `<data>\daemon\state\library.json`** (base data-file rules, ADR-0291, following agent `persistence/mod.rs`):
- `{"schema_version": 1, "root": "default" | "<absolute normalized path>"}` — the library root is **one setting whose default value is the data-root-based default place** (U-D1). `"default"` = the setting holds its default value; it is resolved on every start to `default_root` from the data-root origin. Any other value is an absolute path the user set. `set_root` to a path whose `path_key` equals this run's `default_root` stores `"default"` (the same as never changed). Deleting or re-creating the folder never changes the setting. `"default"` can never be read as a path (it is not absolute).
- `Spec`: `version_key = "schema_version"`, `current = 1`, a small cap (64 KiB). The shape check refuses a document whose `root` is neither `"default"` nor an absolute path string.
- Normalization = platform `normalize_folder`.
- Load outcomes:

| Load result | Root state | Writes this run |
|---|---|---|
| Missing (never written) | first-start default (§3.2) | yes |
| Usable | that root (the default value resolves to `default_root` of this run) | yes — re-`judge` before each write |
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
  - `default_root: PathBuf` — `DataLayout::default_library_root()`; what the setting's default value resolves to this run, and `suggested_new`.
  - `daemon_id: String`.
  - `root_generation_base: u64` — the per-run base of the root generation (§2 「In memory」), drawn by the daemon from its existing random source (`getrandom`) and passed as a value like `daemon_id` (ADR-0311 d5); asset-library adds no randomness dependency.
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
- The root generation (`Numbers.root`) starts each run at a **random per-run base** — not the clock, not 0 — that the daemon draws and passes in (`LibraryValues.root_generation_base`). It is drawn below 2^52, so the base plus the run's increments stays below 2^53 and a JSON client reads every value exactly. A generation a client kept from an earlier run is therefore not accepted. Only equality is meaningful: generations are never compared by order, within a run or across runs (B6).

**Answer types** (crate-local; names are code detail):

| Body | Answer |
|---|---|
| status | `{daemon_id, root, root_setting: default \| explicit (absent when unreadable), state: unset \| ok \| missing (+ why), config: ok \| read-only (+ why) \| corrupt \| unsaved (+ why), suggested_new, root_generation}` |
| `preset.get` view | per-list rows with state · conflict rows · problem list (incl. inner presets with their `via`) · `folders` (normalized, deduplicated, with status) · `rev` · count of referencing agents |
| `agent.presets` view | parent §1-7 row, incl. `workspace_lock` through the port and agent `baseRev` |
| multi-add | `{added, skipped: [{item, reason}]}` |
| relink | `{fixed: [(old, new)], more: n, written: [path], skipped: [{path, code}], stopped?: {path?, code}}` (§3.6) |
| create | the new path |
| `folders` · `read` · `previewFolder` | per the parent §1-7 rows |

## 3. Behaviour

### 3.1 Root, status, identity

- **`status`**:
  - `state` = `unset` (no root) · `ok` · `missing` (the root stat answers anything but `Ok` — missing, not a folder, unreadable, timeout — with `why`).
  - `root_setting` = `default` (the setting holds its default value — `library.json` says `"default"` or is not written) · `explicit` (a path the user set); absent when `library.json` could not be read (`config` = `read-only` or `corrupt`) — except while `config` is `unsaved`, when it describes the live root. After a failed first start (create or the root stat failed): `root_setting: default`, `state: unset`, no `root`; the next start retries (§3.2).
  - `config` = the `library.json` state **now**: `ok` (usable, or not written yet) · `read-only` with `why` (newer or unreadable at load — `set_root` is refused for the rest of the run) · `corrupt` (to be copied aside to `<name>.corrupt` before the next write) · `unsaved` with `why` (this run published a root — `set_root` or the first start — whose `library.json` write failed: the live root may differ from the file's setting — a re-set of the same root whose save fails is `unsaved` with both equal — and the next start follows the file that is left (§3.1 restart rule); it clears at the next successful write). After a successful `set_root` over a corrupt or missing file it is `ok`. When two states apply, the live state wins: a `set_root` over a corrupt file whose save fails is `unsaved`, not `corrupt`. While `unsaved`, `root` and `root_setting` describe the live root. A read-only or unsaved run is visible here, not only in the `set_root` answer.
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
3. If `library.json` is read-only this run (newer or unreadable at load), refuse `Unavailable` · `library-config-read-only` — before anything is created.
4. If `create`, create the folder and its ancestors. A create failure ⇒ `Unavailable` · `library-root-unusable` `{path, why}`; folders partly created may remain; nothing published or written.
5. `change_root` (stat + walk).
6. Only after a successful publish, write `library.json` (`judge` first, then the atomic write): `"default"` when `path_key(path)` equals this run's `default_root`, else the absolute path.

**Persist only after success (U-D2)** — there is no write-then-undo:
- `change_root` refuses in two ways, publishing nothing: the stat does not answer `Ok`, or the walk fails at the root. Then nothing is written (the file keeps its setting) and the answer is `Unavailable` · `library-root-unusable` `{path, why}`. Folders step 4 created may stay behind; they do not count as applied (§1).
- A `library.json` write failure after a successful publish (a `judge` refusal or the atomic write failing): the new root stays live for this run and the file keeps its old setting (or stays unwritten). The answer is `Unavailable` · `library-config-write-failed` `{why}`; `why` (and 10a's message) says the change is live but not saved and the next start follows the file that is left (`Reason.line` is a file line number, not this text). `status` shows the live root with `config: unsaved` (§3.1 `status`). Nothing is undone. The class maps onto the non-retaining `CONFLICT` although the root was applied (§1).
- **Restart rule after a failed save**: the next start loads whatever `library.json` is left and follows the §2 load table — Missing ⇒ the first start runs again (§3.2); Usable ⇒ that setting; Corrupt ⇒ `unset` (copied aside at the next write); Newer / read failed ⇒ `unset`, read-only for that run. No automatic retry is promised beyond that table.
- **Crash between the publish and the write**: the caller gets no answer; the next start follows the restart rule above.

### 3.2 First start and the boot scan (B4 · B23 · U-D3)

- When `library.json` is **Missing**, the daemon (production wiring only — the test server builds no service) calls one asset-library method once, after building the service and before `command_table_slot.set`.
- The method creates `default_root`, then checks that it is usable with the same root stat `change_root` uses (exists, is a folder, answers within the one read budget), then publishes it as the root **without walking** (the no-walk publish below), then — only after that publish — writes `library.json` with the setting at its default value (`"root": "default"`, U-D1 — not the absolute path). The background boot scan below walks it (U-D3). The create is std `create_dir_all` (no time limit); the stat runs on the one read budget. 「Persist only after success」 (U-D2) here means after the root stat succeeded and the root was published.
- If creation or the root stat fails: nothing is published or written, the root stays `unset`, the daemon starts, and the next start tries again.
- **The no-walk publish** is a new `gate.rs` primitive. Contract: it draws a **Root ticket** (exclusive, FIFO — an unticketed publish could let a walk already running apply the old root's results after it); precondition: no root published yet; inside the write lock it sets the root and raises `Numbers.root`; it leaves the first scan **un-raised** and preferably is not counted as a pending first scan, so the first scan is a real walk. `change_root` with an empty build must **not** be used: it marks the first scan raised (ADR-0366 d3), so nothing would ever walk the folder — which may already hold files (a first start re-run after a failed save).
- A walk failure afterwards (the first scan, or any later scan) is the same as a root that fails later: `library.json` stays written, `status` answers the root stat (`missing` with `why` when the folder became unusable; a root that stats `Ok` but cannot be listed gives `state: ok` while readers answer `library-unavailable` — as a normal start does at `7e764daa`), and readers answer through the first-scan entrance rules. **Before the first scan is raised**, a failed or vanished root makes the entrance answer `library-unavailable` (`Gate::first_scan_join` returns the failed scan; the next reader walks again); only **after** it is raised does a root that vanished answer per-file `not-found` (§3.3 step 1, §5). It is never a reason to unwrite the file.
- If the write fails after a successful publish (`judge` refusal or atomic write): the default root is live for this run (`config: unsaved`), a `warn` is logged, and the next start follows the §3.1 restart rule by the file that is left. When no file was written (Missing), the first start runs again — it is idempotent: the folder already exists, so it stats it, publishes it and writes, and the first scan walks it. When the pre-write `judge` found a file that appeared meanwhile, the next start loads that file instead — e.g. a newer file published the default root for this run and makes the next start `unset` and read-only.
- A crash between the publish and the write: the next start follows the same restart rule (no answer is owed — the first start has no caller).
- A configured root that later vanished is `missing` and is **never** re-created (ADR-0312 d5 · ADR-0366 d3) — the same rule whether the setting holds its default value or an explicit path: the default value resolves to this start's `default_root`, and if that folder does not exist the state is `missing` (only a Missing `library.json` triggers the first-start step). The setting is unchanged by the folder vanishing or by someone re-creating it.
- Because the default value is resolved from the data-root origin on every start, moving the distribution (or data) folder moves the library with it; a copy of the folder uses its own `library`.
- **Boot scan (U-D3) — the daemon never walks the library before it is reachable.** On **every** start (not only the first), inside `build_daemon_wiring` right after the library service is built and the first-start step has run, whatever its outcome (production wiring only) — so before `command_table_slot.set` and before `daemon.json` — the daemon calls one asset-library method that starts the first scan of the published root as a background job on the service's spawner, and returns at once. Its body **joins the first scan** (`first_scan_join`), not `refresh(None)` (which would draw a second full walk behind a reader's); its receiver is `self: &Arc<Self>` so the job is `'static`. Nothing on the boot path before `daemon.json` waits for the walk. Other boot disk work stays as at `7e764daa` — `config::load`, `Builtins::new`, and at first start the create and the root stat. With no published root (unset) the job is not started. If the spawn fails, a `warn` is logged and the first reader starts the scan at the entrance, as before.
- Requests that need the catalog wait at the first-scan entrance until the first scan ends (unchanged — they attach to the walking scan, ADR-0324 d6 · d7); a failed first scan answers them as it does today. **Launches are not blocked during the boot scan**: the launch check skips the first-scan entrance and reads the files its profile reaches fresh (the overlay — `launch.rs`); only the per-item applies take the write lock, briefly.

### 3.3 Preset edits (`preset.add` · `remove` · `reorder` · `setBackend` · `save`)

**One pipeline**:
1. First-scan entrance (`first_scan_join`) plus **no published root → `Unavailable` · `library-unavailable`** (root unset, or the entrance failed). No root stat is added here. **Before the first scan is raised**, a root that vanished or cannot be walked fails the entrance's scan ⇒ `library-unavailable`. **After it is raised**, a root that vanished is not detected at this step — the edit's read answers per-file `not-found` (step 4) and `status` says `missing` (`Gate::first_scan_join` returns `Ok` once raised, whatever happened to the folder since).
2. Folder entries to add are stat'ed **before** the change, outside every lock (no stat under the library lock — parent §1-3).
3. `change` with the optional root generation, checked inside the same write-lock take (B6).
4. Inside the change:
   - **Link admission (ADR-0365 「영향」)** before any read: the path must pass `store::admits` (else `InvalidArgument` · `item-path-invalid`) and `LibraryStore::first_link` must answer `Ok(None)` (a link at or above the file ⇒ `InvalidArgument` · `path-through-link` `{path}`, the existing `PATH_THROUGH_LINK`; a `first_link` IO error ⇒ `Unavailable` · `library-read-failed`). `FsStore::read` and `write_atomic` follow links, so without this an edit could read or write outside the root. `first_link` is a store call on a library path like the read itself, not a folder-entry stat (`BoundedIo`), so the parent's no-stat-under-the-lock rule does not cover it.
   - Read the file's current bytes through the store (library cap). A missing file or parent → `NotFound` · `not-found` (no re-creation — M76-REVISED-2, M65). Any other read error (lock, permission, over the cap) → `Unavailable` · `library-read-failed` `{path, why}`.
   - Parse.
   - `reorder` / `save` with `baseRev`: compare it with the `rev` of **the bytes just read** (not the catalog's).
   - Apply the operation to `PresetLists`, keeping every kept item's origin.
   - **Nothing changed ⇒ no write**: if the resulting `PresetLists` equals the parsed one (remove of an absent item, an add where every item is skipped, `setBackend` to the same tag, reorder to the same order, a save of the same lists), skip `rewrite_lists` and `write_atomic`; `rev` is unchanged.
   - `format::rewrite_lists`.
   - `store.write_atomic`. A failure → `Unavailable` · `library-write-failed` `{path, why}`; base `write_atomic` leaves the target unchanged on failure, so nothing was applied.
   - Apply the item made from the written bytes to the catalog (`Catalog::apply_items`, seq drawn inside the lock).
5. Answer the `preset.get` view.

**Writer refusals**: `preset-write-*` (`WriteError::NumberOutOfRange` · `RoundTrip`), the reader's `kind-unknown` · `wrong-kind` · `preset-multi-document`, and the reader codes `WriteError::Parse` passes through (`yaml-syntax` — `format/mod.rs`; `field-invalid` and the other `format/read.rs` codes) — all via `WriteError::to_reason` — answer `InvalidArgument` with the file untouched.

**M11**: whenever the change read bytes that differ from the catalog item, it applies them to the catalog **also when it answers a refusal**, so the same `CONFLICT` does not repeat after an outside edit.

**Per operation**:

| Operation | Rules |
|---|---|
| `add {items \| folders, after?}` | **Items**: partial apply (Δ7). Skipped with a reason: the save mismatches below; a library path not in the catalog (`not-found`, B12); already in the list (`already-present`, by path key; built-ins by id). Missing `after` ⇒ append (M14). Optional `backend` only on instruction and skill items (G29 · M20). **Folders**: normalize; refuse relative · cmd-unsafe (`path_is_cli_safe`) · missing · not a folder · unreadable · stat timeout — **all or nothing** across the call, naming each bad path; an already-present folder (by normalized `path_key`) is skipped, not refused. `items` and `folders` never mix in one call. |
| `remove {items \| folders}` | Idempotent, no edition check (M24-8). A folder matches by normalized `path_key` and drops every spelling. |
| `reorder {section, order, baseRev}` | `baseRev` ≠ the `rev` of the bytes read inside the change ⇒ `Conflict` · `rev-mismatch` `{rev}`. `order` must be a permutation of the section as `preset.get` shows it (folders: deduplicated and normalized) — else `InvalidArgument` · `reorder-order-mismatch`. The folders section writes the normalized, deduplicated list. |
| `setBackend {items, backend?}` | Partial apply, like `add`: each item is applied or skipped with a reason — an item not in the preset is skipped with `not-found`; a non-instruction, non-skill item with `backend-tag-not-allowed`. A `backend` word the registry does not know refuses the whole call (`InvalidArgument` · `backend-tag-unknown`, nothing applied) — it is never a per-item skip here. Omitted = common. |
| `save {instructions, skills, settings, presets, folders?, baseRev?}` | Wholesale replace. A supplied `baseRev` that differs from the `rev` of the bytes read inside the change ⇒ `Conflict` · `rev-mismatch`. **All or nothing** — any mismatch refuses the whole save with `InvalidArgument` · `preset-save-refused` `{items: [{list, path, code}]}`, file untouched. The mismatches: `preset-cycle` · `wrong-kind` · `backend-tag-not-allowed` · `backend-tag-unknown` · `item-path-invalid` · `builtin-in-library-preset` · `item-duplicate` (one target twice in a list, regardless of tag — ADR-0339) · `folder-path-relative` · two folders equal after normalization. A missing path or a missing folder is legal content (M78-REVISED); a cmd-unsafe folder is legal content here (it blocks at launch). `folders` omitted = unchanged. Fields outside the lists are never touched. |

**`preset.check {…, folders?}`**: the same validation and the same resolution function over the draft — the catalog lookup overlaid with the draft preset. No write; file, catalog and `rev` unchanged. The answer has the `preset.get` view's conflict and problem shape. It passes the first-scan entrance; with no published root a draft whose items include library paths answers `Unavailable` · `library-unavailable` (as `preset.get`), and a built-in-only draft answers.

### 3.4 Read bodies (B2)

| Body | Rule |
|---|---|
| `preset.get` | `resolve_preset` plus the catalog item's `rev`, the normalized and deduplicated folder list (first place kept, as resolution does), and the referencing-agent count (`refs::referrers_of`). The read never rewrites the file. A library preset path with no published root ⇒ `Unavailable` · `library-unavailable`; `engram:/default` needs no root and still answers its view. |
| `folders` | The folder tree from `Catalog::folders` plus skill items. |
| `read {path}` | Item detail per parent §1-7, incl. the absolute path, direct and indirect references, state, and the instruction excerpt cap. |
| `previewFolder {folder, backend?, mode?}` | Read only, bounded. Returns: the normalized path; the `add` verdict (the same checks and codes as a folder `add`); per backend and mode, what would load — through the same port answers `launch.rs` uses (`folder_load_set` · `native_skill_roots` · `root_skills`). |
| `agent.presets {agent}` | `resolve_agent` plus `workspace_lock` through the port (cell required) and agent `baseRev` (B15). |

**First-scan entrance, read bodies**: `preset.get` (a library preset path), `preset.check` (§3.3), `folders`, `read` and `agent.presets` pass the first-scan entrance first; `folders`, `read`, a library-path `preset.get` and a `preset.check` whose draft holds library paths answer `library-unavailable` with no published root, while `agent.presets` answers its view with the rows `resolve_agent` gives under that root state. `previewFolder`, `status`, `preset.get` of `engram:/default` and a built-in-only `preset.check` do not need it (none reads the library catalog). The edit and file bodies are covered in §3.3, §3.5 and §3.6.

### 3.5 Agent-preset bodies (`attach` · `detach` · `reorder`)

- **Order**: validate against the catalog, then write through the port **inside one `change`** — the port's last-write-wins contract relies on all writers queueing under the library write lock (`profile_edit.rs` `set_attachments` doc).
- **What an agent takes**: presets only. A library preset path, or `engram:/default` meaning the 「기본」 slot (`Attachments.default_preset`) (B16).
- **`attach {presets, after?}`**: partial apply. Skips: not a preset (`wrong-kind`), not in the catalog (`not-found`), already attached (`already-present`), not a reference (`item-path-invalid`).
- **`detach {presets, yes?}`**: removing `engram:/default` without `yes` ⇒ `Conflict` · `default-preset-detach`.
- **`reorder {order, baseRev}`**: `baseRev` is compared with agent `baseRev` computed from the registry read **inside the change**; a mismatch ⇒ `Conflict` · `rev-mismatch`.
- **First-scan entrance and an unset root**: an `attach` naming any library preset (it validates against the catalog) passes the first-scan entrance; with no published root it answers `Unavailable` · `library-unavailable`, nothing applied. `detach`, `reorder` (it validates nothing against the catalog — only the order and `baseRev`), and an `attach` of `engram:/default` only, need no catalog and work with no root.
- **Unknown agent**: a port `NotFound` ⇒ `NotFound` · `not-found`.
- **Port store failure**: `ProfileEditError::Store` (nothing changed — the port's doc) is passed through unchanged (§1 pass-through variant) and answered as agent `store_command_error` answers every registry-writing command (ADR-0291 d6 R17): a store read-only this run or a newer file ⇒ `CONFLICT` · never; an unreadable file at the pre-write re-judge ⇒ `CONFLICT` · after-condition; a write IO failure ⇒ `INTERNAL` (§5).
- **Port unavailable**: an empty or dead cell ⇒ `Internal` · `profile-edit-unavailable` — never a silent Noop.
- **Answer**: the `agent.presets` view.

### 3.6 Relink and create

**`relink {from, to}`** (ADR-0330):
- Passes the first-scan entrance; no published root ⇒ `Unavailable` · `library-unavailable`.
- Both must be `LibPath`s (else `item-path-invalid`).
- `to` must be in the catalog (else `NotFound` · `not-found`).
- `to`'s kind must equal the kind of the list(s) that reference `from` (else `InvalidArgument` · `relink-kind-mismatch` `{expected, found}`).
- `from`'s own presence is not a precondition (no added condition), but it decides part (b).
- Inside one `change`:
  - **Link admission** (§3.3 step 4, ADR-0365): `to` passes `admits` and `first_link` before anything is written (refusal = nothing applied); each preset file the relink rewrites passes them before its read.
  - (a) Every reference to `from` (path key) becomes `to`.
  - (b) **Only when `from` is missing from the catalog** (ADR-0330 d1–d2 and PRD R19 frame part (b) as finishing the recovery of a missing line — 「other not-found entries」): for every other target not in the catalog whose parent equals `from`'s parent, if `parent(to)/<its name>` is in the catalog with the kind its referencing list needs, it is fixed too and counted in `more`. A present `from` re-points only references to `from` itself.
  - Preset files are rewritten one at a time (pipeline of §3.3, origins kept, a duplicate keeps its first place — ADR-0339).
  - Then agents go through `rewrite_preset_paths(renames)` once (ADR-0335 order).
  - **Per-file refusals are skipped**: a refusal that would repeat on a retry — a writer refusal, `path-through-link` / `item-path-invalid` on the file, the file gone from disk (`not-found`) — skips that file, records it in `skipped: [{path, code}]`, and the walk goes on; the agent rewrite still runs.
  - **IO failures stop**: `library-read-failed` or `library-write-failed` on a preset file stops the walk and the agent rewrite does not run; `stopped: {path, code}`. A failure of the agent rewrite itself (the port's pass-through store error or `profile-edit-unavailable`) gives `stopped: {code}` with no `path`.
  - **Answer**: `fixed` = the pairs whose every referencing preset file was written (and, when agents reference them, whose agent rewrite succeeded); `more` counts the part-(b) pairs among them; `written` = the preset files written; `skipped`; `stopped` when the walk stopped. When a stop comes before anything was written or rewritten, it is answered as that failure's refusal instead (nothing applied, §1). Otherwise it is an answer, not a refusal, because something was applied. The rest is recovered by running the same relink again (ADR-0330 · ADR-0335: no journal).
- Folder entries are never touched.

**`newFolder {parent?, name}`** · **`newFile {folder?, name, kind}`**:
- Pass the first-scan entrance; no published root ⇒ `Unavailable` · `library-unavailable`.
- `name` is one `LibPath` segment. Extension per kind: markdown kinds take `.md`; settings / preset take `.yaml` (`.yml` accepted as already present). The kind's extension is appended unless the name already ends with it; a name ending in the other kind's extension (`.yaml` / `.yml` for a markdown kind, `.md` for settings / preset) ⇒ `InvalidArgument` · `item-path-invalid`.
- Templates: markdown = empty; settings / preset = `kind: <kind>`, `version: 1` and an empty `description: ` line (as `builtin.rs` test `DEFAULT_PRESET` spells it) — `format/read.rs` `version_of` refuses a file without `version`, and `parse_preset` / `parse_settings` call it, so a template without it would not parse.
- **Link admission** (ADR-0365): the new path passes `admits` (a dot or temp name ⇒ `item-path-invalid`) and the parent folder passes `first_link` (a link ⇒ `path-through-link`) before anything is created.
- Refusals: an existing name ⇒ `Conflict` · `name-exists`; a missing parent ⇒ `NotFound` · `not-found`; a create or write IO failure ⇒ `Unavailable` · `library-write-failed`.
- The new item enters the catalog in the same `change`.

## 4. Concurrency

- **Root changes**:
  - `change_root` is exclusive from its ticket to its publish. Edits queued behind it run against the new root.
  - A supplied root generation is compared inside the edit's own write-lock take (B6), so an edit can never write into a root it did not ask for.
  - `set_root` calls and the first start are serialized by one service mutex taken across `change_root` (for the first start: the root stat and the publish without a walk) and the `library.json` write that follows it. Only `set_root` and the first-start step take it, and they take nothing else under it except the gate (no cycle).
- **Boot scan (U-D3)**: the background first scan is a full scan like any other — it draws a ticket in FIFO order and takes the write lock only to apply (ADR-0324 · ADR-0366 d1). No lock is held across the boot: the daemon only spawns the job and goes on, and the job does not hold the root-change mutex. A `set_root` or edit that arrives during the walk queues behind its ticket, and readers attach to it at the entrance.
- **Edits**:
  - Each is one `change`: ticket, write lock, disk read and write inside, per-item `apply_items`, change generation raised before release (ADR-0366 d1 writer 5). This satisfies `LibraryService::change`'s rule that the disk effect is finished inside.
  - Readers wait out one file write for a preset edit; for a relink, every preset file it rewrites plus the registry save; for an agent-preset edit, the registry save (K2). The library is local by assumption (`lib.rs` header).
  - Folder stats and `previewFolder` listings run outside every lock.
- **Lock order**: library lock → manager / registry locks, never the reverse. Port calls happen inside `change`. The manager calls `check_in` holding none of its locks (`manager/prelaunch.rs` header), and the port impl never calls back into the library.
- **Outside edits**: an outside editor changing the file between the read and the write inside one change loses to the change (last write wins). This is the same as the notepad rule (G53).
- **Launch merges**: a late launch merge drawn before an edit is discarded by the raised change generation (ADR-0324 d5).

## 5. Codes (main's choice — §8 B8)

Pattern: kebab-case, a const beside the code that refuses (as `resolve.rs` · `format/read.rs`), with the visibility its users need (the existing `format/read.rs` ones are `pub(crate)` or private; `resolve.rs` ones are `pub`), each with its `args` in the doc comment. Existing codes are reused where they fit.

**Class rule (B11, §1)**: every refusal below applied nothing to the library or the registry (the `set_root {create}` leftover folders of §1 aside), so its class maps onto a bus code that does not retain the request id (`Unavailable` → `CONFLICT`). One refusal did apply something and still maps non-retaining: `library-config-write-failed` after a publish (the live root) — so a same-id retry re-runs `set_root` and retries the save instead of being answered `ALREADY_APPLIED` (§1). There are **two** exceptions, both `INTERNAL`: the empty or dead port cell (a wiring bug) and the port store write IO failure (passed through from agent `store_command_error`).

| Case | Class | Code | args |
|---|---|---|---|
| Supplied root generation stale | Conflict | `root-generation-stale` (new) | `{given, current}` |
| No published root (unset, or the first-scan entrance failed) for a body that needs the catalog (§3.3 step 1 · §3.4 · §3.5 · §3.6). Before the first scan is raised this includes a root that vanished or cannot be walked; a root that vanished **after the first scan is raised** is **not** this row — the file answers `not-found` and `status` says `missing` | Unavailable | `library-unavailable` (existing) | `{message}` |
| Path at or below a link (edit, relink `to`, create parent — ADR-0365; on a preset file a relink rewrites it is a per-file skip instead, §3.6) | InvalidArgument | `path-through-link` (existing, `PATH_THROUGH_LINK`) | `{path}` |
| Edit / relink read: IO error other than not-found (lock, permission, over the cap), or a `first_link` IO error | Unavailable | `library-read-failed` (existing, `launch.rs` `LIBRARY_READ_FAILED`) | `{path, why}` |
| `write_atomic` (or create's `create_dir`) failure — base leaves the target unchanged | Unavailable | `library-write-failed` (new) | `{path, why}` |
| `set_root` target unusable — the `create` step failed, or `change_root` refused (stat or walk); folders partly created may remain; nothing published, nothing written | Unavailable | `library-root-unusable` (new) | `{path, why}` |
| `library.json` read-only this run | Unavailable | `library-config-read-only` (new) | `{why}` |
| `library.json` write failure (`judge` or atomic write) after `set_root` published the new root — **published but not saved**: the root stays live this run, the file keeps its setting, and the next start follows the file that is left by the §2 load table (§3.1 restart rule). The first start logs it instead (§3.2) | Unavailable (applied the live root; still non-retaining — class rule above) | `library-config-write-failed` (new) | `{why}` |
| Port `ProfileEditError::Store` | passed through — agent `store_command_error` decides: read-only or newer ⇒ `CONFLICT` · never; unreadable ⇒ `CONFLICT` · after-condition; write IO ⇒ `INTERNAL` (exception: retains the id although nothing changed) | no 9a code — the agent's answer, message from `store_error_reason(PROFILES_FILE, …)` (ADR-0291 d6) | the agent's |
| Relink per-file refusal (writer refusal, link, file gone) | — (skipped, the walk goes on) | the file's own code in `skipped[].code` | `{path, code}` (§3.6) |
| Relink stopped by an IO failure or the agent rewrite, after something was applied | — (an answer) | the failure's own code in `stopped.code` | `{path?, code}` (§3.6) |
| `rev` / agent `baseRev` mismatch | Conflict | `rev-mismatch` (new) | `{rev}` (current) |
| Save mismatches | InvalidArgument | `preset-save-refused` (new) | `{items: [{list, path, code}]}` — inner codes existing + `item-duplicate` (new) |
| Folder `add` refused | InvalidArgument | `preset-folders-refused` (new) | `{folders: [{path, code, why?}]}` — inner: `folder-not-found` (existing; missing · not a folder · unreadable · timeout, with `why`) · `folder-path-cli-unsafe` (existing) · `folder-path-relative` (existing) |
| Skip reasons (multi-add, attach) | — | `not-found` · `wrong-kind` · `item-path-invalid` · `backend-tag-not-allowed` · `backend-tag-unknown` · `preset-cycle` (existing) · `already-present` (new) | per the inner code |
| Skip reasons (`setBackend`) | — | `not-found` · `backend-tag-not-allowed` (existing) | per the inner code |
| `setBackend` with a `backend` word the registry does not know | InvalidArgument | `backend-tag-unknown` (existing) | `{backend}` |
| `reorder` order not a permutation | InvalidArgument | `reorder-order-mismatch` (new) | `{expected}` |
| Detach 「기본」 without `yes` | Conflict | `default-preset-detach` (new) | `{agent}` |
| Create: name exists | Conflict | `name-exists` (new) | `{path}` |
| Relink: kind differs | InvalidArgument | `relink-kind-mismatch` (new) | `{from, to, expected, found}` |
| Port cell empty or dead | Internal (exception: retains the id) | `profile-edit-unavailable` (new) | `{}` |
| Writer refusals | InvalidArgument | `preset-write-mismatch` · `preset-multi-document` · `preset-write-number-range` (existing), and the reader codes `WriteError::Parse` passes through — `yaml-syntax` (`format/mod.rs`), `field-invalid` · `kind-unknown` · `wrong-kind` and the other `format/read.rs` codes (existing) | `WriteError::to_reason` |

## 6. User-visible changes

- **First start creates the default library folder**: `<dist>\library` (release), `<repo>\.engram-dev\library` (debug) or `<ENGRAM_DATA_DIR>\library`. Decided — G35 · ADR-0312 d5. G39 accepts that a distribution rebuild deletes `<dist>\library`.
- **Moving the distribution folder keeps the library** (U-D1): the library root is one setting whose default value is the data-root-based default place; while the user has not changed it, it resolves from the data root on every start, so after a move the library is found in the moved folder and a copy of the folder uses its own `library`. A root the user sets elsewhere stays at its absolute path; setting it back to the default place is the same as never changing it.
- **The daemon comes up without waiting for the library walk** (U-D3): the first scan runs in the background on every start; a request that needs the catalog waits for it to end; a launch does not wait. An 「indexing…」 cue on the screens is screen work later (step 13), not 9a.
- **Two new state files**: `daemon\state\library.json` and `daemon\state\identity.json`.
- **Launches**: an agent whose profile attaches a library preset gets a real resolution instead of block ④. No create entrance attaches one before 10a, so this is reachable only through a hand-edited profile file. The block ① / ④ wording here (and in K1) becomes a notice when the separate 「launch does not block on missing items」 task lands (user 2026-11-17, its own ADR — not in 9a): it covers both a missing item (block ①) and an unusable library root with library presets attached (block ④ — the user's own example was removing the library folder).
- **No command, CLI or screen change.**

## 7. Implementation units (each green on its own, in order)

| Unit | Content | Files | Hands to the next | Size (rough, incl. tests) |
|---|---|---|---|---|
| **U1** root · status · identity (+ two catalog reads) | §2 persisted shapes incl. the root setting's default value (U-D1), its resolution against `default_root` on every start, and `set_root` to the default place storing the default; `LibraryConfig` load; `LibraryValues` replacement, migrating **every** constructor (below); `status` (incl. `root_setting` and `config`); `set_root` with the mutex, `library.json` written only after the publish (U-D2); first start (publishes, then writes the default value); the root-generation check on `change` (B6); the refusal type (class + `Reason`, plus the store pass-through variant, §1) — `set_root` and the root-checked `change` already answer with it; the data-root origin + `DataLayout::default_library_root`; `identity.json`; wiring; the `folders` and `read` bodies (read-only over the catalog, no overlap with U2's module) | asset-library `config.rs` (new), `lib.rs`; daemon `data_dir.rs`, `lib.rs` (+ identity module). `LibraryValues` construction sites today (`rg -n "LibraryValues \{" crates`): daemon `src/lib.rs` `build_library_service`, daemon `tests/launch_kits_live.rs`, asset-library `src/launch.rs` (test fixture), asset-library `src/lib.rs` (test); plus daemon `src/bin/roundtrip_smoke.rs`, which builds through `build_library_service` and changes with its signature | `status`, `set_root`, a root-generation-checked `change`, the refusal type; `LibraryValues {config, default_root, daemon_id, root_generation_base}` | ~1.6k |
| **U1b** first start without a walk + boot scan (U-D3) | the first start's order (create → root stat → no-walk publish → write `"default"`) and the `gate.rs` no-walk publish primitive (§3.2); the boot-scan method (joins `first_scan_join`, receiver `self: &Arc<Self>`) started on every start inside `build_daemon_wiring` after the first start, whatever its outcome; a spawn failure falls back to the reader-started scan | asset-library `gate.rs` (the no-walk publish), `lib.rs` (the first-start method, the boot-scan method); daemon `lib.rs` (`build_daemon_wiring` calls the boot-scan method after the first start). Reference updates: asset-library `lib.rs` header 「The root」, `FirstStart` / `first_start` / `change_root` docs, `gate.rs` header 「First scan」, `BoundedIo::spawner` doc, `docs/reference/structure/asset-library.md` | the first start no longer walks; the boot scan | ~0.4k |
| **U2** preset view + edits | `rev` on the preset item (catalog read paths); the `preset.get` view; the §3.3 pipeline incl. link admission and the no-change skip; `add` · `remove` · `reorder` · `setBackend` · `save` · `check`; `previewFolder` (shares the folder-add verdict) | asset-library `edit.rs` (new; a view module if it grows), `catalog.rs`, `lib.rs`; adding `rev` to `ItemContent::Preset` also touches every `Preset { doc }` pattern or constructor without `..` — today `builtin.rs`, `catalog.rs`, `gate.rs`, `launch.rs`, `lib.rs`, `refs.rs`, `resolve.rs` (check with `rg -n "ItemContent::Preset" crates/engram-dashboard-asset-library/src`) | the edit pipeline helper (admission → read latest → `rewrite_lists` with origins → write unless unchanged → `apply_items`), the `preset.get` view | ~2.2k |
| **U3** port cell + agent-preset bodies | the cell + its daemon fill; `agent.presets` view + agent `baseRev`; `attach` · `detach` · `reorder` | asset-library `lib.rs` (+ an agent-edit module); daemon `lib.rs` | the filled port | ~0.9k |
| **U4** relink + create | `relink` (incl. admission and the part-way stop); `newFolder` · `newFile` (incl. admission) | asset-library `relink.rs`, `create.rs` (new), `lib.rs`, `catalog.rs` — `Catalog` has no folder-insert primitive today (`folders` is written only by `apply_scan`), so `newFolder` needs one | — | ~1.3k |

- Order: **U1 → U1b → U2 → U3 → U4**. U1b is small and U2 builds on its first-start code being settled (both touch asset-library `lib.rs`). U1 and U3 both touch daemon `lib.rs` and asset-library `lib.rs`; U4 uses U2's pipeline and U3's cell — strictly sequential.
- Each unit fixes the reference files it makes wrong in its own commit:
  - U1: `data_dir.rs` header, `LibraryValues` doc, `lib.rs` header
  - U3: `architecture.md` sentence, `DaemonWiring` doc if changed
  - every unit: `docs/reference/structure/asset-library.md` for the contracts it changes or adds (U1 root config incl. the root setting's default value · status · identity values · refusal shape; U2 edits · `rev` · link admission on edits; U3 port cell · agent-preset bodies; U4 relink · create)
- ADRs for decisions later work relies on (§8 (b) B3 · B6 · B7 · B8 · B17, and §8 (c) U-D1 — the library root is one setting whose default value is the data-root-based place, which refines ADR-0312 d5's 「default root」) are written by main after the PRD+TRD review, before coding (implement binding). §8 (c) U-D2 (persist only after success) is amended by main into the ADRs that state the old order.

## 8. Decisions

**(a) Already decided — cite only.**
- One root, with a first-start default root at the place computed from the data-root origin; uncreatable ⇒ `unset` (ADR-0312 d1 · d5, G35, M24-2).
- `set_root` goes through `change_root` (ADR-0366 d3). The first start no longer walks: it publishes after the root stat, and the background boot scan walks (U-D3, B23).
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
- **B4** First start = create → root stat → no-walk publish → write `"default"`; the background boot scan walks it (U-D2 · U-D3). An uncreatable first-start root or a failed root stat ⇒ `unset`, `library.json` not written, the daemon starts, the next start retries; a failed write after the publish (or a crash before it) ⇒ the default root is live this run, and the next start follows the §2 load table by the file that is left — no file ⇒ the (idempotent) first start runs again; a walk failure after the publish leaves the file written (a root that fails later, not an unwrite); a vanished written root stays `missing`. — matches ADR-0312 d5 / M24-2 and does not freeze a failed attempt into the file.
- **B5** `set_root` = normalize + absolute check → mutex → read-only check → create (if asked) → `change_root` → only after a successful publish, write `library.json` (the default value when the path is the default place, else the absolute path). A refused `change_root` writes nothing; a write failure after the publish answers `library-config-write-failed` with the new root live and the file unchanged — no undo (U-D2). One mutex serializes `set_root` and the first start. — the file only ever records a root that was actually published (the user: 「성공해야 적는게맞지」); the mutex prevents two calls from interleaving publish and file order.
- **B6** The root generation enters edits as an optional argument checked inside the edit's own write-lock take (a root-checked sibling of `LibraryService::change`); stale ⇒ `root-generation-stale`. Each run starts it at a random per-run base below 2^52 (not the clock, not 0), drawn by the daemon from its existing random source and passed in `LibraryValues` like `daemon_id` (ADR-0311 d5) — asset-library adds no randomness dependency; compared only for equality (§2 「In memory」). — a check before the ticket could pass and then write after a queued root change; a per-run random base keeps a generation from an earlier run from matching.
- **B7** `rev` = lowercase-hex SHA-256 of the exact file bytes (BOM included), kept on the catalog preset item. — stable across restarts (M21); `sha2` is already a dependency; `preset.get` stays a memory read.
- **B8** The code strings of §5. — kebab-case beside the refusing code, reusing existing codes; the parent left them to implementation; the user delegated command details and wording to main (`llm-command-conventions.md` §1 header, 2026-10-26).
- **B9** T-106 P8 is **not** fixed in 9a; it stays in tracking and is risk K1. — the root state needs a stat that must stay outside the library lock (parent §1-3), so root and catalog cannot be read in one take. The narrow fix (note `Numbers.root`, recompute when it moved) adds a retry state, against the user's 2026-11-12 criterion; the effect is one transient read answer.
- **B10** Four sequential units, U1 → U4 (§7). — file overlap in both `lib.rs` files; U4 needs U2 and U3.
- **B11** Refusals are `{class, Reason}` and 10a maps the class onto the bus code; the mapping is fixed here (§1): `Unavailable` → `CONFLICT`, and every refusal that applied nothing maps onto a code `command_delivery::retains_the_id` does not retain; `library-config-write-failed` after a publish (U-D2) applied the live root but is non-retaining too, so a same-id retry retries the save. — keeps the vocabulary in agent (parent §1-5) while 9a fixes codes, args and the retry meaning; `ErrorCode` has no unavailable code and agent `commands.rs` already maps `LaunchBlocked` to `CONFLICT`. Two exceptions, both `INTERNAL`: a port store write IO failure (the store error is passed through and `store_command_error` maps it, keeping the agent's retry modes) and an empty or dead port cell.
- **B12** A multi-add skips a library path that is not in the catalog (`not-found`) and an already-present item (`already-present`), like `attach`. — a typo guard matching the folder `add`; `save` keeps missing paths legal because it replaces a draft that may hold 「찾을 수 없음」 lines.
- **B13** Edit disk IO runs inside the change's write lock; folder stats run before it, outside. — the `change` contract requires the disk effect inside; no stat under the library lock.
- **B14** An edit that read new bytes applies them to the catalog even when refusing (M11).
- **B15** Agent `baseRev` = lowercase-hex SHA-256 of the canonical JSON `{"default": <bool>, "presets": [<paths in order>]}`. — stable, order-sensitive, read straight from the registry.
- **B16** `engram:/default` in agent-preset commands names the 「기본」 slot (`Attachments.default_preset`). — the profile stores it as a bool; this spells it as the reference users and LLMs already see.
- **B17** The port cell is `OnceLock<Weak<dyn ProfileEditPort>>`, set once in `build_daemon_wiring_with_store` right after `Arc::new(manager)`; an empty or dead cell ⇒ `profile-edit-unavailable`. — no `Arc` cycle; filled before `command_table_slot.set` (parent §1-4 ⑥); no silent Noop.
- **B18** The daemon calls an asset-library `config.rs` load before `LibraryService::new`, and calls the first-start method after building the service and then the boot-scan method (B23), in production wiring only. — `new` stays free of disk work beyond the built-ins (parent §1-4 ②); the test server builds no service.
- **B19** `relink` checks `to` (present, same kind) but does not require `from` to be missing; part (b) runs only when `from` is missing from the catalog. — no added condition; ADR-0330 d1–d2 and PRD R19 frame part (b) as finishing the recovery of a missing line, so a present `from` re-points only references to `from` itself.
- **B20** Create appends the kind's extension unless present, refuses a name ending in the other kind's extension, and refuses an existing name; the settings / preset template carries `version: 1`. — one call creates a usable file; no overwrite path outside 9b's Explorer rules.
- **B21** `status.state = missing` covers every non-`Ok` root stat, with `why`. — the parent defines three states; `why` keeps the cause.
- **B22** The data-root origin comes from the branch `default_data_dir` already takes (a sibling function returning it); `DataLayout` carries it and computes `default_library_root` without creating anything. — no guessing from names (parent L236); the shell copy keeps its path-only rule.
- **B23** The daemon never walks the library before it is reachable (U-D3): every start begins the first scan of the published root as a background job on the service's spawner, inside `build_daemon_wiring` right after the service is built and the first-start step has run (before `command_table_slot.set` and `daemon.json`); the job joins `first_scan_join` (not `refresh(None)`); the boot path before `daemon.json` does not wait for it; catalog requests wait at the first-scan entrance as before, launches do not (the launch check skips the entrance); a spawn failure falls back to the reader-started scan with a `warn`. `set_root` is unchanged (a user request answered after its walk). — follows the convention of IDEs and language servers that come up first and index in the background (U-D3); the entrance already makes early readers wait, so no new waiting rule is needed.

**(c) User decisions.**
- **U-D1** (user 2026-11-17, 「A,B라고하면 당연히 B지」 and 「그냥 동일룰이라잖아. 기본 라이브러리라는 개념은 없어. 단지 디폴트 경로가 그거일 뿐이지」) **The library root is one setting whose default value is the data-root-based default place** (`DataLayout::default_library_root`). There is no separate 「default library」 concept. While the user has not changed it, the setting holds its default value (persisted as `"root": "default"`) and is resolved on every start from the data-root origin, so moving the distribution keeps the library and a copy uses its own `library`. Deleting the folder does not change the setting, nor does re-creating it at the configured place; a configured place missing at start is `missing` and is not re-created (ADR-0312 d5, unchanged). `set_root` with a path whose `path_key` equals this run's default place stores the default (the same as never changed); any other path is stored absolute. Applied in §2, §3.1, §3.2, §6, §7 U1, §9 U1.
- **U-D2** (user 2026-11-17, after the review PASS: 「아니 성공해야 적는게맞지. 왜 먼저적어?」) **`library.json` is written only after the root change succeeded** — for `set_root` and the first start alike. No write-then-undo: a refused `change_root` writes nothing; a write failure after the publish leaves the new root live for the run, the file at its old setting, and answers `library-config-write-failed` (published but not saved). Replaces draft 3's write → `change_root` → restore order (and its risk K3). Applied in §1, §3.1, §3.2, §4, §5, §7 U1, §8 (b) B4 · B5 · B11, §9 U1, §10.
- **U-D3** (user 2026-11-18, after U1 landed: 「ㅇㅇ 백그라운드로 훑는게 좋은데 일단 서버쪽 작업하고 UI 쪽은 나중에 작업하면 되려나」; light convention check the same day — JetBrains IDEs and rust-analyzer come up first and index in the background, requests during indexing are not blocked by startup, a progress cue sits in a status bar, and a startup-blocking loading screen is rare — Obsidian's 「Loading cache」 is the exception users complain about) **The daemon never walks the library before it is reachable.** A background job starts the first scan of the configured root on every start; catalog requests wait at the first-scan entrance until it ends. The first start = create → root stat → publish without a walk → write `"default"`; U-D2's 「persist only after success」 now means after the root stat succeeded and the root was published, and a later walk failure is a root that fails later, not a reason to unwrite. `set_root` is unchanged. The 「indexing…」 cue is screen work later (step 13). Applied in §0, §1, §3.2, §4, §6, §7 U1b, §8 (a) · (b) B4 · B18 · B23, §9 U1b, §10 K8 · K9.
  - **Confirmed (user 2026-11-18, answering the draft-7 hold — option (a)):** 「백그라운드로 해. 데이터가 필요할때 훓는건 백그라운드 없을때 얘기이고.」 The 2026-10-17 rule 「데이터가 필요할때 훓어야지」 was about a world without a background scan. U-D3 **supersedes** that rule — PRD §3 R19 「the daemon does not pre-scan at startup」, parent `trd.md` §1-4 (around :206), cited by ADR-0335. Main amends PRD R19, the parent TRD lines (:171, :206, :952) and ADR-0381 / 0375 / 0366 / 0335 through a new ADR.
  - **Launches are not blocked during the boot scan** (asked next by the user; main answered from the code): the launch check skips the first-scan entrance and reads the files its profile reaches fresh (its overlay — `launch.rs`); only the per-item applies take the write lock, briefly.
- Recorded outside 9a (affects only the reading of §6 「Launches」 and K1): the user decided 2026-11-17 that a launch no longer blocks on missing items — it covers both a missing item (block ①: missing instructions, skills and settings are skipped) and an unusable library root with library presets attached (block ④ — the user's own example was removing the library folder); what is skipped is named in a notice to the caller and in the log. A separate task with its own ADR; nothing in 9a changes for it.

Nothing is open — see §12.

## 9. Test plan

All units: 전체 회귀 (qa binding row 「Rust 소스」). `crates/engram-dashboard-asset-library/src/**` and daemon `src/lib.rs` are in the **claude and codex lane** path lists, so every unit also runs both lanes (cost — announce per qa binding 「비용 있는 검사」). No GUI scene: no front or shell change.

| Unit | Automated (new) | Mutations QA must hit |
|---|---|---|
| U1 | `library.json` load outcomes (missing · usable · corrupt · newer · read failure) and write policies; first start creates, publishes, then writes; an uncreatable place ⇒ `unset`, no file, a second start retries; a first-start write failure after the publish ⇒ the default root live this run, `config: unsaved`, and the next start follows the file that is left (no file ⇒ the first start runs again and writes; a newer file found by the pre-write `judge` ⇒ `unset`, read-only); a configured root that vanished ⇒ `missing`, not re-created, the setting unchanged (default value and explicit path alike); the first start writes the default value, not a path; the default value resolves to the new place after the data folder moves (a `library.json` carried to another data root answers that root's `default_root`), while an explicit root stays the absolute path; `set_root` to a path whose `path_key` equals the default place stores the default value; `status` `root_setting` (incl. after a failed first start) and `config` (ok · read-only with `why` · corrupt; `ok` after a successful `set_root` over a corrupt or missing file); `set_root` on a read-only config refuses before creating anything; default place per origin (env · debug · release — `ENGRAM_DATA_DIR` isolation never touches the real folder); `set_root` order: a refused `change_root` (stat and walk alike) writes nothing and the file keeps its setting; a write failure after the publish answers `library-config-write-failed` with the new root live, the file unchanged and `status` `config: unsaved` (also over a corrupt file — `unsaved` wins), and the next start (a service rebuilt over the same files) follows the file that is left; `unsaved` clears after a later successful `set_root`; a create failure answers `library-root-unusable` with nothing published or written; failure injection for `library.json` writes goes through a write seam in `config.rs`; concurrent `set_root` calls serialize; a stale root generation is refused inside the lock (an edit queued behind a root change); a generation kept from an earlier run (a rebuilt service) is refused; gate tests that asserted absolute root generations (`== 0`, `== 1`) assert relative to the base; `identity.json` cases; `status` fields; one daemon wiring test over a temp `DataLayout`; `folders` / `read` rows | `library.json` written before the root change; the mutex removed; the generation check moved before the ticket; the per-run generation base made a constant; the default place guessed from a name; the first start writing the absolute path instead of the default value; `set_root` to the default place storing it absolute |
| U1b | the first start with **files already in the default place** creates, stats and publishes without a walk and writes `"default"`; the walk is parked with the existing `TestStore` hold (`hold_at(At::Start, …)` / `wait_held`) and the first start is asserted to have returned; the first scan then fills the catalog — observed without a catalog reader (`numbers().swap_ins`, `with_read`, `TestStore::scans()`), since a reader would run the first scan itself; a reader that arrives before the scan ends waits at the entrance and then sees the items; a walk failure after a published first start leaves `library.json` written, the root published, readers answering `library-unavailable` until a scan succeeds and `status` answering the root stat; a failed root stat at first start writes nothing and leaves `unset`; with the walk parked, the boot-scan method returns; the boot scan also starts on a normal start (a usable `library.json`); a spawn failure leaves the scan to the first reader — run on a normal start or with a spawner that fails only the boot job (the service spawner also runs the `BoundedIo` stat workers); a daemon source-order guard like `the_boot_sweeps_before_the_plug`: `build_daemon_wiring` calls no catalog reader (`first_scan_join`, `refresh`, `list`, `folders`, `read`, `resolve_*`) and the boot-scan call comes after the first start regardless of its outcome (`build_daemon_wiring` hard-codes `FsStore`, `StdFolderOps`, `ThreadSpawn`, so it has no store seam; `FolderOps` serves the root stat, not the walk — the walk is `LibraryStore::scan` / `read`). The production-wiring daemon tests may leave a detached boot-scan thread — they wait on `numbers().swap_ins` or note it | the no-walk publish raises the first scan; the first start walking on the boot path; the boot scan joined (waited for) before `build_daemon_wiring` returns, the boot scan started only at first start, the boot scan drawing `refresh(None)`; `library.json` unwritten on a later walk failure |
| U2 | the parent §4 row 「asset-library edit · reference · relink」 for edits: idempotent `add` / `remove`; `reorder` `rev-mismatch`; save all-or-nothing with every mismatch code; missing path legal; moved target `not-found` and no re-creation; origins kept (an item's other keys survive a respelling); `description` untouched; writer refusals leave the file untouched; M11 under a refusal; `rev` equal across a service rebuild; `check` writes nothing; an edit that changes nothing writes nothing and keeps `rev` (remove of an absent item, all-skipped add, same tag, same order); `setBackend` partial apply with `not-found` skips; link admission — an edit through a link or on a dot / temp name is refused before any read and the file outside the root is untouched; a read error other than not-found and a `write_atomic` failure answer their non-retaining codes — with the catalog unchanged when the bytes read equal the catalog item, and with the read bytes applied (M11) when they differ; folder add all-or-nothing, matching by `path_key`, normalized storage; `preset.get` folder dedup; `previewFolder` verdict = the `add` verdict | M11 dropped on refusal; a missing-target write re-creates the file; origins dropped; a folder stat moved under the lock; admission removed; the no-change skip removed |
| U3 | attach / detach / reorder only through the port (a fake port asserts the calls and that each call comes from inside a change); 「기본」 detach without `yes`; agent `baseRev` order sensitivity; an empty cell ⇒ `profile-edit-unavailable`; a port `Store` error is passed through and answered as `store_command_error` answers it (unreadable keeps `after-condition`); with no root, `detach`, `reorder` and a 「기본」-only `attach` work while a library-preset `attach` answers `library-unavailable`; the daemon fills the cell before the table is set (source-order guard) | the registry mutated directly; the cell check folded into a Noop |
| U4 | relink (a) + (b) with same-kind-only, `more` counted, presets then agents, duplicate first place, folder entries untouched, no cross-root (parent §4 row); part (b) only when `from` is missing; a per-file refusal (writer refusal, link, file gone) is skipped and the walk and the agent rewrite go on; an IO failure stops the walk, skips the agent rewrite and answers `fixed` (only pairs whose every referencing file was written) + `written` + `skipped` + `stopped`; a stop in the agent rewrite gives `stopped` with no `path`; link admission on `to` and on each rewritten preset file; create templates (each settings / preset template parses), extensions incl. the other kind's extension refused, `name-exists`, link admission on the parent, the catalog insert (file and folder) | kind check removed; agents rewritten before presets; admission removed; part (b) run with a present `from`; a per-file refusal stopping the walk; an IO stop still running the agent rewrite |

## 10. Risks

| # | Risk | Caught by |
|---|---|---|
| K1 | T-106 P8: a read computation pairs the root state from before a `set_root` with the catalog after it — one transient wrong answer (e.g. block ④ for an agent whose new root is fine). Reachable from 9a on. The block-④ wording becomes a notice when the separate 「launch does not block on missing items」 task lands — that decision covers block ① (a missing item) and block ④ (an unusable root with library presets attached) (user 2026-11-17, §8 (c)). | accepted (B9); tracking T-106 stays |
| K2 | The library write lock is held across disk IO: a preset edit's one file read and one write; a relink's every preset file it rewrites plus the registry save; an agent-preset edit's registry save — on a slow disk, readers and launch checks wait. | the library is local by assumption (`lib.rs` header); not measured |
| K4 | A run-only `daemon_id` (unreadable or newer `identity.json`) orphans the client's drafts for that run. | `warn` at boot; drafts are a client-stage feature |
| K5 | The app's WMI-started dev daemon (no `ENGRAM_DATA_DIR`) creates `<repo>\.engram-dev\library` on first start. | intended (debug default place); inside the ignored dev folder |
| K6 | The lane cost of every unit (claude + codex). | qa binding announcement rule |
| K7 | A published-but-unsaved root for one run (U-D2): after a `library.json` write failure — or a crash between the publish and the write (no answer reaches the caller) — the live root may differ from the file's setting, and the next start follows the §2 load table by the file that is left (Missing ⇒ the first start again; Usable ⇒ that setting; Corrupt or Newer / read failed ⇒ `unset`, the latter read-only). | the `library-config-write-failed` answer says so; `status` `config: unsaved`; U1 tests with write-failure injection |
| K8 | A large library makes the first catalog request wait for the first walk — the same wait as before U-D3, now usually hidden because the walk starts in the background at boot. A request that comes right after boot still waits at the entrance. | U1b test with the walk parked (`TestStore` hold — the first start and the boot-scan method return before the walk ends); the 「indexing…」 cue is screen work later (step 13) |
| K9 | An uncontained panic in the walk / parse code ends the daemon at **every start** (ADR-0292 · ADR-0293: an uncontained panic is logged, then aborts) — before U-D3 it happened only at the first catalog reader. | the walk / parse code is not in the containment list; existing scan tests; accepted with U-D3 |

K3 (draft 3's 「rollback of `library.json` fails」) is gone: U-D2 removed the write-then-undo order.

## 11. Input claims corrected (parent `trd.md` → code / ADR at `ca508879`)

- §2-4 item 4 (single-preset tag conflicts) is listed **open** → closed by the user 2026-11-11 (U5 (a), recorded with step 7). The parent's §2-4 should mark it closed at freeze.
- §1-4 assembly order cites daemon `lib.rs` line numbers (`:609 → :651`, `:660`, `:666`, `:772`) → today the MCP start is before `build_daemon_wiring`, then `command_table_slot.set`, then the accept loop. The order holds; the numbers are stale.
- ADR-0338 d6 names `yaml-edit` as the writer → superseded by ADR-0358 (README records it). ADR-0338 d3 / d4 lack `folders?` → added by ADR-0341, which the README status of ADR-0338 does not record.
- L236 「`DataLayout` also holds where the root came from」 → `DataLayout` only computes paths and today does not know the origin (`default_data_dir` returns a bare path). The origin is returned by a sibling of `default_data_dir` and carried by `DataLayout`; folder creation stays outside `DataLayout` (B22).
- §1-7 create row (`library.newFile`, around parent line 376) gives the settings / preset template as `kind` + an empty `description: ` line → `format/read.rs` `version_of` refuses a file with no `version` (`parse_preset` · `parse_settings` call it), so that template would not parse. 9a's template adds `version: 1` (§3.6).
- 9a row 「The four block reasons all stand here」 → all four already exist (agent `launch.rs` `BlockReason`; ④ since step 7). 9a keeps them true through edits and adds none.

## 12. Open — user

None. Every user-visible choice in 9a is decided by an ADR or a G-number (§8 (a)), by the user (§8 (c) U-D1 · U-D2 · U-D3), or is a command detail the user delegated to main (`llm-command-conventions.md` §1 header).

## 13. Review record

PRD+TRD review 2026-11-17 · full (Claude lead + codex) · PASS on `43f64c4f` (round 1 FIX on `702463f9`). Draft 3's changes (round-2 findings, U-D1 rewording) closed by main's diff check, not re-reviewed.

Draft 4 (U-D2 order) re-review 2026-11-17: Claude PASS, codex FIX (restart wording) — fixed in draft 5, closed by main's diff check.

Draft 6 (U-D3) re-review 2026-11-18: Claude FIX, codex FIX — wording and test fixes applied in draft 7; U-D3 itself on hold for the user (see §12).

Draft 8 — the user's answer to the U-D3 hold applied (no design change beyond removing option (b)); closed by main's diff check.
