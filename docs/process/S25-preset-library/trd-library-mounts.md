# TRD — step 21: the library is one virtual root of named mounts (ADR-0396)

| Item | Value |
|---|---|
| **Status** | **Draft 2 · 2026-11-21 — not reviewed** (draft 1 reviewed round 1: FIX; this draft applies the findings and ADR-0398). Decisions and boundaries only. Canonical ADR: ADR-0396 (amends ADR-0385 decisions 1–2 and ADR-0312 decisions 2 and 6). Relies on ADR-0315 · ADR-0320 · ADR-0324 · ADR-0330 · ADR-0335 · ADR-0339 · ADR-0358 · ADR-0361 · ADR-0365 · ADR-0366 · ADR-0375 · ADR-0376 · ADR-0377 · ADR-0381 · ADR-0382 · ADR-0386 · ADR-0387 · ADR-0398. All user decisions are taken (§7). |
| **Parent** | S25 topic ([`trd.md`](trd.md) §5-2 item 21 — its body, the 2026-11-19 user decision on the built-in, and the hand-overs from step 10a and T-112 are absorbed here; the item shrinks to a status line + link when this TRD is reviewed). Runs after 10a and T-112, before 9b. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (What it is · Modules (+ `mounts.rs`) · Wiring (`LibraryStart`, the data root) · the root paragraph → a mounts paragraph · Main flow 1, 4, 7 · Contracts: lock order, generations, tickets, links, ④ stands alone, absent/unreadable · Where to look). Also the asset-library `lib.rs` header, `config.rs` / `gate.rs` / `path.rs` / `store.rs` / `resolve.rs` / `launch.rs` headers, agent `commands.rs` library docs. `architecture.md`: the asset-library line only. `CLAUDE.md` 「모듈」 asset-library line (「한 루트 폴더 아래」) is main's. |
| **Line-pointer basis** | Code at `727590ae`. Code is cited by file + symbol. |
| **Not a source** | Frozen child TRDs (`trd-library-commands.md` · `trd-library-edit.md` · `trd-launch-skip-missing.md` · `trd-asset-library.md`) are read only to find why something is so. Code wins over any document. |

## 0. Scope

**In.**
- The virtual root: no folder of its own, only named mounts at its top, each mapped to a real folder. A reference is `<mount>/a/b`; the first segment is replaced by the mount's folder only at the disk boundary (ADR-0396 decisions 1–2).
- `library.json` holds the mount list (schema 2). **Not shipped — no migration (ADR-0398):** no reader for schema 1, no rewrite of paths already written (§3.5).
- Mount management commands; `library.status` answers the mounts; `library.setRoot` retires (§7 UD3).
- Mount add / remove / rename / relocate policies, overlap refusal, the first start.
- The built-in as a read-only mount `engram` that `list`, `read` and `folders` treat like any mount (§7 UD5 — the 2026-11-19 user decision in item 21), plus the two 10a hand-overs (§3.4).
- T-112's absent / unreadable rule made per mount (§3.3).
- The rules 9b inherits for moves, copies and trash across mounts (§3.6).

**Out.** The move / copy / delete / trash / restore / empty commands themselves (step 9b). Screens (steps 11–13). A mount identity and per-PC data keyed by it (favorites, T-72). Export / import (T-73).

## 1. Facts the design relies on (checked at `727590ae`)

- **One root everywhere.** `gate.rs` `State { catalog, root: Option<PathBuf> }`; one `Numbers.root` generation; `LibraryService` `set_root` · `first_start` · `default_place` · `start_boot_scan` · `status`; `config.rs` `library.json` `{"schema_version":1,"root":"default"|<abs>}` (ADR-0375). Daemon `DataLayout::default_library_root` = `<dist>\library` (release) or `<data>\library`. `LibraryValues` carries `default_root`, `launch_root` and `builtin_dir`, but no data root.
- **The disk join.** `LibPath::to_path(root)` · `from_disk(root, …)` · `key(root)` have about 16 production call sites in 7 files — 7 of them in `store.rs` (its scan builds `LibPath`s from disk with `from_disk`, `trash` joins with `to_path`), none in `relink.rs`. The wider work is the root threaded through the service: the `Gate::change` closure's `Option<&Path>`, `resolve.rs` `CatalogLookup.root`, the launch overlay's root. `catalog::key` keys the catalog by the spelling through platform `path_key` (case-folded on Windows). `LibraryStore` methods take `root` + a disk path.
- **Built-in.** `AssetRef { Library(LibPath), BuiltIn(BuiltinId) }`; `BuiltinId` is a closed set of 3, read once; `builtin.rs` `file_of` maps them to `presets/default.yaml` · `instructions/priming.md` · `settings/default.yaml` under `builtin\`. `engram:/` appears in 161 Rust lines over 21 files and 6 lines of `builtin/` and `prompts/`. `edit.rs` `lib_path` refuses a built-in (`item-path-invalid`); `library.list` answers `engram:/default` in `builtins` when asked. Commands carry the built-in preset as the profile's `default_preset` bool, but `AttachedPreset.path` may also hold `engram:/…` (`profile.rs` doc; existing tests seed it).
- **Agent attachments** are `profile.rs` `AttachedPreset { path: String }`; `ProfileEditPort::rewrite_preset_paths` / `remove_preset_paths` match whole paths by `same_path` (a caller expands a folder per attached item).
- **Reference index.** `Catalog::presets_listing(key)` answers the presets listing a target, missing targets included; relink rewrites files through `LibraryService::rewrite_in` and agents through the port, presets first, no journal (ADR-0335).
- **Tickets.** `gate.rs` `may_enter`: a `Scan` or `Root` ticket enters only at the head of one FIFO line; a `Change` enters only when every earlier ticket is an entered `Change`. So `change_at` called while holding a `Root` ticket never enters. `scan_until_applied` holds its ticket across `build` → `FsStore::scan` (std `fs::metadata` + `WalkDir`, unbounded).
- **`agent.new {presets}`** judges under one library read lock and calls `create_agent` after releasing it (agent `commands.rs`; ADR-0386 decision 5: released before the name-allocation gate).
- **Launch** (T-112): `resolve.rs` `RootState { Unset, Absent, Unreadable, Ok }` with one producer `RootState::of`; `launch.rs` `Skips`; agent `launch.rs` `LIBRARY_UNAVAILABLE_SKIPPED` (args `message`, `items`); ④ `BlockReason::LibraryUnavailable`. An unparsed spelling still blocks a launch.
- **Platform and IO.** `fs::file_identity` (volume + file id, follows links, `Unsupported` on volumes without ids); `same_path` · `path_key` · `normalize_folder` · `share_root`. `BoundedIo` bounds stat, list and `canonical` (`folder.rs`).
- **Save failure and retry.** `config.rs` `fail_writes` is the test seam for a failed `library.json` write; agent `library_commands.rs` `RETRY_AFTER_CONDITION_TAIL` tells the caller to 「clear the cause, then repeat the same call」 (ADR-0381).
- **Nothing here has shipped.** The asset-library crate is absent at tag `v0.4.0`; `library.json` and command catalog v9 exist only in development data folders. `catalog_version` stays 9 (the same unreleased generation — add a comment line). The crate holds no random source; it does read the clock (`SystemTime::now` in `lib.rs` `build_scan_reading`, `create.rs`, `edit.rs`).
- `PresetSection` is also an argument (`preset.reorder --section`); `ProblemRow.kind` is a host-checked `String` (precedent for words outside a Rust enum's spelling).
- Prefix-like tags in use: `folder:` (agent `launch.rs` `FOLDER_ITEM_PREFIX`) and `engram:/`. `LibPath` refuses `:`, so neither collides with a mount name.

## 2. The model and module boundaries

```
virtual root (no folder)                               library.json (schema 2)
├── engram   (read-only, view over builtin\)           mounts: [{name, path}]   path = "default" | <abs>
│   ├── default            → builtin\presets\default.yaml
│   ├── priming            → builtin\instructions\priming.md
│   └── settings/default   → builtin\settings\default.yaml
├── library  → <dist>\library   (first start)
└── company  → D:\git\company-presets
reference "company/skills/review" → D:\git\company-presets\skills\review
```

- **New module `mounts.rs`** (asset-library): the mount table (`Mounts`), name rules, the overlap check, and the only converter between the two path types:
  - `LibPath` — a virtual path from the virtual root (`<mount>/…`); what the catalog, references, the wire and the resolver use.
  - `MountPath` — a path relative to one mount's folder; the only path type the store takes and returns (`ScanListing`, `first_link`, `trash`, `from_disk`).
  A missed prefix or strip does not compile. `LibPath` itself stays a plain validated path (no new syntax — ADR-0396 decision 2); a path of one segment names a mount.
- **`gate.rs`** `State.root` → `State.mounts`; one write lock and one generation for the whole tree (§8 c1); the first-scan answer is kept per mount (§5).
- **`store.rs`** stays per folder: it receives a mount folder as `root` and `MountPath`s; it knows no mount names.
- **`config.rs`** reads and writes schema 2 only.
- **`builtin.rs`** keeps reading the three files once; the `engram` mount is a view over it (its join is `file_of`), not a scanned folder (§3.4).
- **Rename primitive** (`gate.rs` + `relink.rs`): one `Root`-ticket operation that, in one write-lock hold, rewrites references by a leading-names mapping in the preset files of every readable mount, then agent attachments, then re-keys the catalog range, renames the registration and raises the generation (§3.2, §5). The reference-rewriting part is the walk 9b's move fix-up reuses (inside its own change).
- **agent** gains the new command declarations, the `mount` arg and the `PresetSection` word change; **daemon** passes the data root and the built-in folder, and `NoLibrary` answers the new commands `library-unavailable`. **platform** is unchanged.
- No `cfg` outside platform; overlap and identity checks use platform functions.

## 3. Behaviour

### 3.1 The virtual root
- **Reads.** `library.list` with no `folder` lists the mounts as folders, each row with its `state` / `state_why`; `library.folders` includes each mount as a top folder with the same fields. With no mount registered the virtual root still reads (only `engram`) — never `library-unavailable`. `library.read` of a one-segment path answers `not-found`, as for any folder today (folders are not items); mount states are in `library.status` and the virtual-root `list`.
- **Whole-tree reads with an unusable mount answer partially.** `library.list {query}` without `folder` and `library.folders` answer from every usable mount and name each unusable one with its state (§4) instead of refusing — Windows Explorer shows an unavailable drive beside working ones. A read whose range is inside one unusable mount refuses `library-unavailable` with `mount`.
- **Nothing is created at the top.** `library.newFolder` without `parent` and `library.newFile` without `folder` refuse `INVALID_ARGUMENT` `virtual-root-read-only`. A one-segment target of any write (and, in 9b, of a move, rename or delete) is refused the same way — a mount's top is changed only by the mount commands, and Engram never deletes a mount's real folder.
- **A path whose first segment names no mount** reads as absent (not found), never as a path under some default mount.
- **Name comparison** of mount names = platform `path_key` (the catalog rule), so `Company/x` and `company/x` are one reference on Windows. `engram` is reserved case-insensitively on every OS; `AssetRef::parse` and the mount lookup use that same rule, so `Engram/x` means the built-in on Linux too.

### 3.2 Mount lifecycle
- **Add** (`library.addMount {name, path, create?}`): `name` is required — no name is derived from the folder (a derived name could be empty for `D:\`, start with `.`, be reserved, too long or a device name). Name checked (UD2); path absolute and normalized; created first with `create`; the folder must stat as a folder through `BoundedIo` (as `set_root` today). Refused `mount-overlaps {with}` when it is the same folder as a mount, inside one or containing one, or overlaps the data root's other parts (`daemon\`, `shell\`), the launch folder or the built-in folder (§8 c8). **Overlap compares canonical final paths** (links and junctions resolved — `BoundedIo` `canonical`) of both sides and their ancestors by path key, and also `file_identity` of those folders when the volume gives one. Then a `Root` ticket: the new mount is walked and its range applied; other mounts' items stay. The answer also counts references that already point at that name (presets, agents) — they now resolve into the new folder.
- **Remove** (`library.removeMount {mount}`): drops the registration only — no file is touched, ever. Its range leaves the catalog; references to it become absent and follow ADR-0387 (launches skip with one notice per mount). Running agents are unaffected (their materials are prepared; folders stay on disk). The answer counts the preset lines in other mounts and the agents now pointing at it (UD8: no confirmation).
- **Rename** (`library.renameMount {mount, to, yes?, root_generation?}`):
  - Refused `mount-unusable {mount, state}` when the mount itself is absent or unreadable — its own references could not be rewritten.
  - The plan is built only after every mount's first scan was applied (the call waits at the first-scan entrance like `query`). It lists the files to rewrite per mount, which mounts are git working trees (§3.6 rule), the mounts it could not read (absent, unreadable, not yet walked — their references stay broken and are fixed by hand, ADR-0396 decision 3), the agents it would change, and that files are rewritten whole (ADR-0358: comments and quoting are not kept). It also counts references already pointing at `to`.
  - Without `yes`, when any file would be rewritten, nothing is applied: `CONFLICT` `mount-rename-unconfirmed {plan, root_generation}`. With `yes` the call should carry that `root_generation`; a stale one refuses with a fresh plan (`root-generation-stale` + `plan`).
  - With `yes` (or nothing to rewrite) it runs the rename primitive (§2): presets → agents → re-key → registration → generation, in one write-lock hold under a `Root` ticket, then saves. A read or write failure stops the walk before the agents and the registration (relink's rule, ADR-0335): files already written point at `to`, the old name stays registered, and the same call finishes it. A case-only rename is allowed.
- **Relocate** (`library.relocateMount {mount, path, create?}`): re-points a mount at another folder under the same name (the old `setRoot` for one mount; the Lightroom 「Find Missing Folder」 shape) — references do not change. Same overlap rules as add, excluding the mount itself (a move into its own subfolder or parent is still judged against every other mount, and the data / launch / built-in folders); the mount is re-walked.
- **A repeated call after a failed save succeeds** (ADR-0381 decision 3, `RETRY_AFTER_CONDITION_TAIL`). The mount change is published before `library.json` is saved; when the save fails (`library-config-write-failed`, `config: unsaved`) the caller repeats the same call, and a call whose effect is already live answers success and saves again:
  - `addMount` with the same name and the same folder live → success + save;
  - `removeMount` of a name not registered while the setting is `unsaved` → success + save (otherwise `mount-not-found`, so a mistyped name is still refused);
  - `renameMount X→Y` with `X` not registered and `Y` live → runs the reference walk again (idempotent — only remaining `X/…` references change; under the `yes` rule), then saves;
  - `relocateMount` to the folder already live → success + save.
  Help says: after a timeout, read `library.status` before repeating.
- **Read-only config** keeps today's rule for every mount command: a read-only `library.json` refuses `library-config-read-only` before anything.
- Each mount change raises the one generation, so an edit carrying an older `root_generation` is refused `root-generation-stale` (ADR-0377), whichever mount it touches. **`agent.new {presets}`** re-checks the judged generation at its registry write, so a rename between the judgement and `create_agent` cannot store `old/…` unseen: nothing is created and the call answers `root-generation-stale`. Mechanism for the implementer: hold the library read lock across the registry write (the lock order library → registry allows it) or compare inside it; ADR-0386 decision 5 releases the lock before the name-allocation gate — re-check its reason first.
- **At load and at each boot** the saved mounts are checked for overlap again (a `"default"` place that moved with the data folder, a mount folder later replaced by a junction, a reassigned drive letter): a mount overlapping an earlier one in the list is kept registered but not walked; its state is `unreadable` with `why` naming the other mount, so a launch reaching it is refused (ADR-0387: exists but broken → block) until the user relocates or removes it.

### 3.3 Launch and read entrances per mount (T-112 hand-over)
- `RootState` is computed **per mount** with the same producer `RootState::of` (that mount's stat + its first-scan answer). A reference resolves through its mount's state:
  - mount not registered, or registered and absent (missing / not a folder / absent local drive) → its items are `Missing`; a launch skips them with **one `library-unavailable-skipped` per mount**, args gaining `mount` (the name) — `message` says 「not registered」 or the stat answer;
  - mount exists but cannot be read, did not answer, its share is not reachable, or it overlaps another mount → ④ `LibraryUnavailable` with `mount` in the reason args; a launch that reaches it is refused, and — as today for the root — it does no folder IO and no command-line check;
  - `Ok` → as today.
- At the read entrances one ④ row per unusable mount stands for that mount's items (ADR-0361 / ADR-0387 per mount); other mounts expand normally. Built-ins never block (unchanged).
- **`agent.attachPreset` and `agent.new {presets}` keep the attach rule per mount** (user O3, ADR-0386 decision 5): a named library preset whose mount has no applied first scan refuses the whole call `library-unavailable` with `mount`, nothing applied. They are not made partial (§12).
- **Inherited unchanged from T-112:** the split by row kind (`Resolution::skipped` / `blocked`), `RootState` `Absent` / `Unreadable` and its one rule, the typed `stat_root`, the share probe (`BoundedIo::stat_all` + platform `fs::share_root`), `RootStatus::Unreadable` (now per mount), both skip codes, `folder-unreadable`, the launch note rules, the `warn` logs, notices once per agent. Nothing T-112 built is removed.

### 3.4 The built-in (UD5 B)
- The built-in is the reserved read-only mount `engram`, always present (even with no user mount), backed by `builtin.rs`. References are spelled `engram/default`, `engram/priming`, `engram/settings/default`; `AssetRef::parse` maps a first segment `engram` (case-insensitive) to `BuiltIn` and refuses any other name under it (`UnknownBuiltin`). The old spelling `engram:/…` is refused with a clue (`item-path-invalid`, `why` names the new spelling).
- **Its virtual layout** (§2): `list engram` answers the items `default` (preset) and `priming` (instruction) and the virtual folder `settings`; `list engram/settings` answers `default` (settings). The join is `file_of`; `library.read engram/default` answers `absolute` = the real file (`<builtin>\presets\default.yaml`). `library.folders` includes `engram` and `engram/settings`. Writes into it refuse `mount-read-only`. `library.list`'s `builtins` flag and answer field are removed (the folder replaces them).
- Special handling kept: read once, an unreadable built-in becomes an empty stand-in + `builtin-unavailable` (ADR-0361), the `default_preset` bool, no `.trash`, never renamed, removed or relocated (`mount-read-only`).
- **10a hand-over 1** — `preset.get engram/default` keeps the `builtin-unavailable` notice when the built-in folder is unusable (`edit.rs` `complete_view` today keeps only resolution problems).
- **10a hand-over 2** — `PresetSection` words become lowercase (`presets` · `instructions` · `skills` · `settings` · `folders`), the same words as the YAML keys and the refusal arg `list` (`format/read.rs` `key`) — in answers and in the `preset.reorder --section` argument. The wire field becomes a host-checked `String` (the `ProblemRow.kind` precedent); a word outside the five refuses `INVALID_ARGUMENT`. `FolderLoadKindWord` stays as it is: no lowercase counterpart of its words was found in refusal args (§12).

### 3.5 Data written before mounts — not shipped, no migration (ADR-0398)
- A schema-1 `library.json` is an earlier edition with no reader, so the base data-file rules treat it as corrupt (copied aside before the next write): no mounts until the developer runs `addMount`, or deletes `library.json` so the first start registers `library` again.
- Paths already written in preset files and agent attachments are not rewritten: a root-relative path now names an unregistered mount and reads as absent; an attached `engram:/…` is an unparsed spelling and blocks that agent's launch until it is detached. Development data may be dropped or fixed by hand (relink, detach).

### 3.6 What step 9b inherits (rules decided here; 9b builds the commands)
- **Trash per mount:** delete = rename into `<mount folder>\.trash\<bundle>\<MountPath>` (ADR-0320 per mount); a bundle keeps the mount-relative path, so a later mount rename does not break restore; restore returns into the same mount. Emptying covers every mount's `.trash` (9b decides the per-mount argument). A removed mount's `.trash` is left on disk, unreachable from Engram. The built-in has none.
- **Git working tree** (for `.git/info/exclude` and UD7's plan): walk up from the mount folder through its parents for a `.git` entry — a folder, or a file (a worktree or submodule, whose `gitdir:` leads to the common dir); write `info/exclude` in the common dir.
- **Move across mounts:**
  1. Refused when the target is inside the source (judged by canonical path and identity, as overlap).
  2. On one volume it is a rename, and the rename must not replace (std `fs::rename` replaces an existing file — `store.rs` `rename`); a taken name is a conflict.
  3. On different volumes: copy, only to names that did not exist (`create_new` semantics). Telling the two apart: try the rename and treat `ErrorKind::CrossesDevices` as 「copy」 (9b confirms what Windows answers).
  4. What is copied: files and plain folders as a scan sees them — dot entries are not copied (they are not library content; `.trash` included), links are not followed or copied (ADR-0365 — reported as skipped), non-library files inside a skill folder are copied (the folder is the item).
  5. Before the original goes to trash: compare the original's tag / `rev` with the one read for the copy — a change saved during the copy stops the step and keeps both; re-check the generation — a target mount removed or relocated meanwhile stops the step and keeps both.
  6. Only then the original goes into **its own mount's** `.trash` as one bundle. On any failure the original stays, and a partial copy is never removed by name: it goes into the **target** mount's `.trash` (ADR-0335's no-delete-by-name rule, store `create_new`).
  7. References are fixed by the shared rewrite walk after the move.
- **Copy across mounts** = steps 3–4, no reference fix.
- One-segment paths (mount tops) are never moved, renamed or deleted by 9b. 9b keeps the single generation and the ticket rules of §5.

## 4. Data shapes

- **Persisted — `library.json` schema 2** (base data-file rules, ADR-0291 · ADR-0376): `{"schema_version": 2, "mounts": [{"name", "path"}]}`, `path` = `"default"` or an absolute normalized folder; list order = the order shown. Schema 1 has no reader (§3.5). An older build meeting schema 2 treats it as newer (R2). The built-in mount is not stored. No mount id (§8 c2).
- **Persisted — agents.json**: unchanged. `notice_fingerprint` sees the new `mount` arg, so each skip notice shows once more.
- **Wire — `library.status`** (UD3): `{daemon_id, mounts: [MountRow], config, config_why, suggested_new, root_generation}`, `MountRow = {name, path, default: bool, builtin: bool, state, state_why}`; `root`, `root_setting`, `state`, `state_why` leave the top level.
- **Wire — new commands:** `library.addMount {name, path, create?}` · `library.removeMount {mount}` · `library.renameMount {mount, to, yes?, root_generation?}` · `library.relocateMount {mount, path, create?}`; each answers the status after the change, plus: add — `already_pointing {presets, agents}`; remove — `now_absent {presets, agents}`; rename — `fixed`, `written`, `skipped`, `stopped` (as relink) and `already_pointing`. `library.setRoot` is removed. Refusal codes: `mount-name-invalid` (INVALID_ARGUMENT) · `mount-name-taken` · `mount-overlaps {with}` · `mount-read-only` · `mount-unusable {mount, state}` · `mount-rename-unconfirmed {plan, root_generation}` (CONFLICT) · `mount-not-found` (NOT_FOUND) · `virtual-root-read-only` (INVALID_ARGUMENT); today's `library-config-*`, `root-generation-stale` and `item-path-invalid` reused.
- **Wire — reads:** at the virtual root, `library.list` folder rows (`ListFolderRow`) and `library.folders` mount nodes gain `state`, `state_why` (set on mount rows only); a whole-tree `query` answer gains `unavailable: [{mount, state, why}]`; `library.list` loses `builtins`; `library.read` of a one-segment path answers `not-found`.
- **Wire — notices and rows:** `library-unavailable-skipped` args gain `mount`; ④ reason args and `library-unavailable` refusal args gain `mount`; `PresetSection` → lowercase host-checked `String` in answers and in `preset.reorder` args.
- **Bindings:** agent `bindings/*.ts` and `commands.schema.json` regenerated (status, the four commands, list and folders rows, `PresetSection`). `catalog_version` stays 9 with a comment line (§1).

## 5. Concurrency

- **One library lock, one generation** for the whole tree (§8 c1). Lock order unchanged: root-change mutex (now the mount-change mutex; every mount command and the first start take it) → library lock → turn mutex. Across crates unchanged: library lock before the manager's and the registry's.
- **A mount change is a `Root` ticket** (exclusive from ticket to publish). Add / relocate walk the one mount, then apply it under the write lock after dropping that mount's range; remove drops the range. **Rename is one `Root`-ticket primitive** that does presets → agents → re-key → registration → generation in one write-lock hold (it cannot call `change_at`: a `Change` never enters behind a held `Root` ticket — `may_enter`). So no `agent.attachPreset`, `preset.add` or `agent.new` can write an `old/…` path between the walk and the re-key: an edit queued behind the rename meets the new generation (stale if it carried the old one) and the new name. The save follows the hold.
- **Bounded stat before any walk ticket.** Every scan — the boot scan, a refresh, add, relocate — stats each mount it would walk through `BoundedIo` (the T-112 share probe included) **before** it draws its ticket. A mount that is absent, unreadable or does not answer is not walked; it takes that state (ADR-0387). So a dead share does not hold the ticket line.
- **What is not promised:** a mount that answers its stat but walks slowly (a reachable slow share, a huge folder) still holds the one FIFO line for its walk, as today's one root does — later edits, other mounts' scans and mount commands wait behind it (§11 R3). Readers wait at the first-scan entrance for the whole first scan, not per mount.
- Launch checks still skip the first-scan entrance and stat each mount they reach in the required phase (one stat per mount per launch, cached in `LaunchIo`).

## 6. Texts to change

| Text | Change |
|---|---|
| `prompts/engram-help.md` `## library` | Paths start with a mount name; the built-in is `engram/…`; the four mount commands (`name` required for add; rename's plan and `yes` + `root_generation`); after a timeout read `library.status` before repeating; `library.status` answers mounts; `setRoot` gone; nothing at the top; partial whole-tree answers; skip notices name the mount; `--section <presets\|instructions\|skills\|settings\|folders>` (today `--section <Presets\|…>`). |
| agent `commands.rs` library docs | Status shape, the four commands, `list` / `newFolder` / `newFile` unspecified folder = refused at the top, `read` of a mount name, `ProblemRow` / notice docs (`mount`), `PresetSection` words. |
| asset-library headers | `lib.rs` (the root → mounts), `config.rs`, `gate.rs` (rename primitive, bounded stat before tickets), `path.rs` (`engram/…`, `MountPath`), `store.rs` (root = a mount folder, `MountPath`), `resolve.rs` / `launch.rs` (per mount). |
| `builtin/` presets, tests | `engram:/` → `engram/` (UD5 B); priming has no `engram:/` today. |
| `docs/reference/structure/asset-library.md` | Header line. |

## 7. User decisions

**Decided 2026-11-21: every ★ below (UD2 a · UD3 a · UD4 a · UD5 B · UD6 a · UD7 a · UD8 a · UD9 a)** — user 「ㅇㅇ 진행해.」 to 「추천대로 전부 갈까요?」. UD6 a reverses ADR-0312 decision 3. UD1 was then replaced by ADR-0398.

**UD1 — today's single root and the paths already written.** Not shipped — no migration (ADR-0398, user 2026-11-21). The first-start mount is named `library` (fixed). §3.5 says what happens to development data.

**UD2 — mount name rules.** ★ (a) The rules of one `LibPath` segment (Unicode allowed; no `/ \ :` or control character, no `.`/`..`, no name ending in `.` or a space, no Windows device name — ADR-0362), plus: no leading `.` (dot entries are hidden and `.trash` lives in a mount), reserved `engram` (case-insensitive on every OS), unique by platform `path_key`, at most 64 characters. (b) ASCII lowercase identifiers `[a-z0-9-_]` — no case question across OSes, but the user's own examples are Korean (`업무/…`, ADR-0315 `스킬/리뷰/review`). Whether the name must equal anything on disk: settled no by ADR-0396 「영향」. `folder` need not be reserved — `folder:` carries a `:` that no `LibPath` accepts. Precedent: Unreal mount names and Claude Code marketplace names are one-name-one-source (`docs/research/multi-library-reference-paths-survey-2026-10-15.md` conclusion 1 and row ①); case-insensitive uniqueness follows our catalog rule (ADR-0325).

**UD3 — mount management commands.** ★ (a) `library.status` answers every mount (one query per area — `llm-command-conventions.md` §2) + four writes `addMount` · `removeMount` · `renameMount` · `relocateMount`, each answering the status after the change; `setRoot` removed. (b) Keep `setRoot` as 「relocate the first mount」 beside them — two commands change a mount's folder. (c) One `library.setMounts {mounts}` whole-list write — a rename hidden in a list diff cannot carry the plan or `yes`. Precedent: Claude Code `claude plugin marketplace add` / remove (`docs/research/library-multi-root-display-survey-2026-10-14.md` conclusion 5); Lightroom 「Find Missing Folder」 (same survey, conclusion 6) — the relocate command.

**UD4 — first start.** ★ (a) Register the default place `<dist>\library` (created) as one mount named `library` (ADR-0312 decision 5, user G35 「최초 디폴트 등록해둬」). (b) Register nothing. (c) (a) plus a shipped sample mount — none exists yet. Precedent: Calibre 「Calibre Library」, Obsidian 「Create new vault」; VS Code proposes none (`docs/research/library-empty-state-first-run-survey-2026-10-15.md` conclusions 2–3).

**UD5 — the built-in `engram:/`.** ★ (B) A reserved read-only mount `engram`, spelled `engram/…`, always present; `engram:/` refused with a clue (§3.4) — the 2026-11-19 user decision carried into the mount model; one reference form (ADR-0396 decision 2). (A) Keep `engram:/` and special-case `list` / `read` — two reference forms everywhere. Precedent: Unreal's `/Engine/` mount (`multi-library-reference-paths-survey` conclusion 1).

**UD6 — which folders may be mounts.** ★ (a) Any absolute folder (a git clone anywhere, a share, `~/.claude/skills`), refused only for overlap (§3.2). **This reverses ADR-0312 decision 3** (user G37 — outside content is copied in): a mounted outside folder is used in place, and Engram writes into it (preset edits, its `.trash`, its `.git/info/exclude`). (b) Only folders under one parent — contradicts ADR-0396 decision 1's own example. Precedent: Claude Code, codex, Blender and Ableton register folders anywhere; Obsidian keeps one root per vault (`docs/research/multi-root-asset-library-practice-2026-10-30.md` conclusion 1).

**UD7 — renaming a mount when files change.** ★ (a) Without `yes` it applies nothing and answers the plan (§3.2); with `yes` it runs. (b) Run and only warn. Precedent: none found for a confirmation on a root rename (looked at Unreal, VS Code, Claude Code, Obsidian — `multi-library-reference-paths-survey`); our own `yes` in ADR-0339 is the model.

**UD8 — removing a referenced mount.** ★ (a) No confirmation; the answer counts what now points at it; adding it back restores every reference. (b) `yes` when referenced. Precedent: Claude Code removes a marketplace's plugins and marks dropped plugins 「not found」 (`multi-root-asset-library-practice` table); whether it asks first is not recorded, and no confirmation precedent was found (also looked at Blender and Lightroom in the same surveys).

**UD9 — a read-only flag per user mount.** ★ (a) Not now. (b) Now. Precedent: not found — Blender's Link / Append is an import method, not a library property (`multi-root-asset-library-practice`).

## 8. Main's choices and author's internal choices (reported)

- **c1 (main)** — One library lock and one generation for the whole virtual tree; any mount change raises it. Reason: no lock-order question, and the `root_generation?` argument shape keeps working. Cost accepted: a mount change makes queued edits in every mount stale (retry).
- **c2 (main)** — No mount id now. T-72 decides a mount's identity and how durable it must be (a counter in `library.json` could be issued twice across a run whose save failed — ADR-0381 decision 2), and the re-add rule.
- **c3 (main)** — One catalog keyed by the virtual path (`catalog::key` of `<mount>/…`).
- **c4** — Two path types: `LibPath` (virtual) and `MountPath` (relative to a mount, the store's only type); `mounts.rs` is the only converter, so the store and its fakes stay mount-unaware and a missed prefix or strip does not compile.
- **c5** — A mount folder may itself be a link (junction or symbolic link), as the one root may be today (ADR-0365 per mount); links inside a mount are still never followed. Overlap is judged on canonical final paths.
- **c6** — An unregistered mount name reads as an absent mount: one skip notice per name, not one `missing-item-skipped` per item.
- **c7** — Add and relocate require an existing folder (or `create`), as `set_root` does; a mount may later go absent.
- **c8** — A mount may not overlap the data root's `daemon\` and `shell\` parts, the launch folder or the built-in folder. `LibraryValues` gains `data_root` (daemon `DataLayout` root) for this check; the default place inside it stays allowed.
- **c9** — Logs: one `info` line per mount change (`mount`, `path`); `warn` for a rename that skipped files and for a mount found overlapping at load (`docs/reference/logging-conventions.md`).

## 9. Test plan

QA rows (`.claude/skill-bindings/qa.md`): 「Rust 소스」 → 전체 회귀 (bindings change → sync gate). Lanes: asset-library `src/**`, `builtin/**` and daemon `src/lib.rs` → **both** claude and codex lanes; daemon `control/**` (help) → claude lane. GUI: the manager is unchanged and the frontend reads no library type → **off**, except one start scene below.

| What | Checked by | Unit |
|---|---|---|
| Name rules (UD2), reserved `engram` case-insensitive on every OS, uniqueness by path key | `mounts.rs` unit tests | U1 |
| Overlap: two spellings of one folder, a junction to a mount, nested both ways, data / launch / built-in folders, relocate into its own subfolder; `Unsupported` identity falls back to canonical paths | `mounts.rs` tests with fake `canonical` / identity seams + one real-junction test | U1 |
| `library.json` schema 2 round trip; schema 1 is corrupt; newer / unreadable keep ADR-0376 | `config.rs` tests | U1 |
| Join: `LibPath` ↔ `MountPath`; one-segment path = the mount; unknown mount → absent; built-in layout via `file_of` | `mounts.rs` / `path.rs` tests | U2 |
| Virtual root reads: zero user mounts answers `engram` only; mount rows carry state; create at the top refused; `read` of a mount name `not-found` | `lib.rs` / `create.rs` tests | U2 |
| Scans stat mounts with `BoundedIo` before the ticket: a held stat does not hold the line (a queued edit enters); a failing mount does not fail the others; partial whole-tree `query` and `folders` | `gate.rs` / `lib.rs` tests | U2 |
| Cross-mount reference resolves; relink across mounts | `resolve.rs` / `relink.rs` tests | U2 |
| Per-mount launch rule: absent and unregistered mount → skipped + one notice with `mount`; unreadable or overlapping mount → ④ with `mount`, no folder IO; other mounts unaffected; attach / new keep the whole-call refusal per mount | asset-library `launch.rs` / `agent_edit.rs` tests; agent `launch.rs` notice golden with `mount` | U3 |
| Built-in mount: `list engram`, `list engram/settings`, `read engram/default` `absolute`, write refused, `engram:/` refused with clue, `Engram/default` is the built-in; `preset.get` keeps `builtin-unavailable` | `lib.rs` / `edit.rs` / `path.rs` tests | U3 |
| `PresetSection` lowercase in answers and `preset.reorder` args; an unknown word refused | `host.rs` / agent `commands.rs` tests | U3 |
| Mount commands: add / remove / relocate; add and rename count `already_pointing` | `lib.rs` / `host.rs` tests; agent `commands.rs` schema tests | U4 |
| Rename: refused for an absent / unreadable mount; plan lists unread mounts, git trees (`.git` folder and file, from a subfolder of a repo), whole-file rewrite; `yes` with a stale `root_generation` refuses with a fresh plan; a write failure stops before agents and registration and the same call finishes | `lib.rs` / `relink.rs` tests | U4 |
| Rename atomic: an `attachPreset` / `preset.add` of `old/…` queued during a rename meets the new name or a stale generation; `agent.new` judged before a rename is refused stale, nothing created | `gate.rs` / `agent_edit.rs` tests; agent manager-level test | U4 |
| Repeat after a failed save, every mount command (`fail_writes`): the repeat answers success and saves; the file then holds the live mounts | `lib.rs` tests | U4 |
| Overlap found at load → that mount not walked, `unreadable` + why, launch refused | `lib.rs` test | U4 |
| Daemon wiring: production manager launches through a two-mount service; `data_root` passed; `NoLibrary` answers the new commands | daemon `src/lib.rs` tests | U4 |
| Help screens name the mount commands and the lowercase `--section` words | daemon `control/help.rs` tests | U5 |
| Live, claude lane: a preset in mount A with a skill in mount B reaches the agent | new scene in `tests/launch_kits_live.rs` (`claude_l6_…`) | U5 |
| Live, codex lane: the same over codex JSON | `codex_l6_…` | U5 |
| Live CLI scenes (manual QA, fresh data folder): `library.status` shows `library` and `engram`; `addMount` a git clone in a temp folder; `removeMount` → spawn answers one `library-unavailable-skipped` with `mount`, none on a second spawn; add back → spawn clean; `renameMount` without and with `yes`; a mount under `\\<unreachable>\share` → not walked, spawn refused, other commands not delayed | `engram library.*` / `engram agent.spawn` in a spawned agent | QA |
| GUI: double-click start of an agent whose preset lives in a non-default mount | qa-verify 「GUI 실측」 | QA |

## 10. Implementation units

Sequential — `lib.rs`, `path.rs`, `gate.rs` and the test fixtures overlap in every unit. Build and tests green at the end of each. Sizes include tests (about half); the bulk of U2 is test fixtures re-spelled under a test mount.

- **U1 — mount model and persistence** (~450 lines): `mounts.rs` (name rules, overlap over `canonical` + identity, the `LibPath` / `MountPath` converter), `config.rs` schema 2, `LibraryValues.data_root`, `gate.rs` `State.mounts` holding the one first-start mount; the service still behaves as one root.
- **U2 — virtual paths through the service** (~1,500–2,500): `MountPath` through the store; the threaded root (`Gate::change` closure, `CatalogLookup`, launch overlay) through `mounts.rs`; scans with the bounded stat before the ticket and the first-scan answer per mount; catalog keyed by virtual path; virtual-root and partial whole-tree reads; the top-level refusal; edits, create, relink across mounts; test fixtures moved under a test mount.
- **U3 — launch per mount and the built-in mount** (~700–1,000): `resolve.rs` / `launch.rs` / `agent_edit.rs` / agent `launch.rs` (`mount` arg); `path.rs` `engram/…` and the ~170 spelling sites; `builtin/` files; the 10a fixes (`PresetSection` as a host-checked `String`).
- **U4 — mount commands** (~1,000–1,300): agent declarations (status reshaped, four commands, `setRoot` removed, list and folders rows, `list` without `builtins`), `host.rs`, the rename primitive and its plan, repeat-after-failed-save, the `agent.new` generation re-check, the load-time overlap check, daemon wiring and `NoLibrary`, bindings regenerated.
- **U5 — texts, references, live** (~300): `prompts/engram-help.md`, doc comments not already rewritten, `control/help.rs` tests, reference files, live scenes.

## 11. Risks

- **R1** — Test churn: 558 asset-library tests spell root-relative paths; U2 moves them under a test mount. Mechanical; a helper keeps the diff readable.
- **R2 (accepted, c1)** — Any mount change makes edits queued in other mounts stale; the caller retries with the new `root_generation`.
- **R3 (accepted, as today)** — A mount that answers its stat but walks slowly holds the one ticket line for its walk; later edits, other mounts' scans and mount commands wait behind it, possibly past the CLI wait (ADR-0369). Only a dead mount is kept off the line (§5).
- **R4** — A rename rewrites files in other git repositories, whole (comments and quoting lost — ADR-0358); UD7's plan is the only guard. Uncommitted changes in those repositories are rewritten too.
- **R5** — Overlap detection can be escaped where neither canonical paths nor identities agree: SMB servers that give no file id, and one share under several host spellings (`\\nas` · `\\10.0.0.5` · a mapped `Z:`).
- **R6** — Under UD6 (a), mounting a folder a backend also reads natively (`~/.claude/skills`) can load a skill twice (natively and through a preset), and Engram's `.trash` appears inside it. Whether claude skips a dot folder there is unconfirmed.
- **R8** — On a case-sensitive OS, user mount names follow that OS's `path_key`, so shared presets must match case exactly across OSes (`engram` is the one exception).
- **R9** — Development data written before mounts is not carried (§3.5, ADR-0398).

## 12. Records for main

- **Round-1 finding not applied as written:** R-partial asked the multi-mount attach / `agent.new` judgement to answer partially. Their whole-call refusal when no first scan was applied is a user decision (O3, ADR-0386 decision 5); §3.3 keeps it per mount and only the reads answer partially. Changing it needs the user.
- **Item 21's user decision (2026-11-19) predates ADR-0396**; carried as UD5 B. The parent item's 「the built-in becomes one read-only root among the roots」 maps to a mount, not a root.
- **10a hand-over 2 names `FolderLoadKindWord`** — no lowercase counterpart of its words was found in refusal args (searched asset-library). Unconfirmed what clash 10a meant; left unchanged unless main names it.
- **`agent.new` mechanism** — ADR-0386 decision 5 releases the library read lock before the name-allocation gate without saying why; the implementer re-checks before holding it across the registry write (§3.2).
- **Input claims checked:** `gate.rs` `NO_ROOT`, `edit.rs` `lib_path` refusing a built-in, `RootState`, `Skips`, `TRASH_DIR` / `LibraryStore::trash` with no production caller, no `.git/info/exclude` code, no watcher — all confirmed. Corrected in draft 2: the crate does read the clock; the join has about 16 production sites in 7 files, most of the work being the threaded root.
- **ADRs to write after review (implement flow §0-1):** a new ADR for UD2–UD9 and the §3.2 / §3.5 / §3.6 rules (implements ADR-0396 decision 4); amendments —
  - ADR-0375 (the setting is a mount list) · ADR-0381 (per mount command; repeat after a failed save) · ADR-0382 (first-scan answer per mount; bounded stat before the walk ticket);
  - ADR-0324 decisions 4 and 6 and ADR-0366 decision 3 (tickets: the rename primitive, the stat before the ticket);
  - ADR-0361 decision 1 (+「영향」) and decision 3, ADR-0387 decision 2 (per mount, `mount` arg, an overlapping mount blocks);
  - ADR-0320 decisions 1–2 (trash in each mount; mount tops cannot be deleted) and decision 4 (a cross-mount move or overwrite touches two `.trash`);
  - ADR-0312 decision 3 (UD6 a), decision 5 (first start registers mount `library`), decision 7 (+「영향」 — trash, exclude and the built-in per mount) and its 「영향」 「two root management commands」;
  - ADR-0315 decision 3 (UD5 B) · ADR-0365 (each mount folder may be a link) · ADR-0330 (relink across mounts) · ADR-0386 decision 13 and the last 「영향」 bullet (`setRoot` retired, `list` loses `builtins`).
- **Parent TRD:** item 21's title 「several library roots」 → 「library mounts」; body → status line + link; 9b's item gains a pointer to §3.6.

## 13. Review record

