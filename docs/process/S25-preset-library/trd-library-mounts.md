# TRD — step 21: the library is one virtual root of named mounts (ADR-0396)

| Item | Value |
|---|---|
| **Status** | **Draft 1 · 2026-11-21 — not reviewed.** Decisions and boundaries only. Canonical ADR: ADR-0396 (amends ADR-0385 decisions 1–2 and ADR-0312 decisions 2 and 6). Relies on ADR-0315 · ADR-0320 · ADR-0330 · ADR-0335 · ADR-0339 · ADR-0365 · ADR-0375 · ADR-0376 · ADR-0377 · ADR-0381 · ADR-0382 · ADR-0386 · ADR-0387. Seven user decisions are open (§7); the design below follows the ★ options and names the sections that change if the user picks otherwise. |
| **Parent** | S25 topic ([`trd.md`](trd.md) §5-2 item 21 — its body, the 2026-11-19 user decision on the built-in, and the hand-overs from step 10a and T-112 are absorbed here; the item shrinks to a status line + link when this TRD is reviewed). Runs after 10a and T-112, before 9b. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (What it is · Modules (+ `mounts.rs`) · Wiring (`LibraryStart`) · the root paragraph → a mounts paragraph · Main flow 1, 4, 7 · Contracts: lock order, generations, tickets, links, ④ stands alone, absent/unreadable · Where to look). Also the asset-library `lib.rs` header, `config.rs` / `gate.rs` / `path.rs` / `store.rs` / `resolve.rs` / `launch.rs` headers, agent `commands.rs` library docs. `architecture.md`: the asset-library line only. `CLAUDE.md` 「모듈」 asset-library line (「한 루트 폴더 아래」) is main's. |
| **Line-pointer basis** | Code at `727590ae`. Code is cited by file + symbol. |
| **Not a source** | Frozen child TRDs (`trd-library-commands.md` · `trd-library-edit.md` · `trd-launch-skip-missing.md` · `trd-asset-library.md`) are read only to find why something is so. Code wins over any document. |

## 0. Scope

**In.**
- The virtual root: no folder of its own, only named mounts at its top, each mapped to a real folder. A reference is `<mount>/a/b`; the first segment is replaced by the mount's folder only at the disk boundary (ADR-0396 decisions 1–2).
- `library.json` holds the mount list (schema 2) and reads schema 1 as one mount.
- Mount management commands; `library.status` answers the mounts; `library.setRoot` retires (§7 UD3).
- Mount add / remove / rename / relocate policies, overlap refusal, the first start, the one-time move of paths already written (§7 UD1).
- The built-in as a read-only mount `engram` that `list`, `read` and `folders` treat like any mount (§7 UD5 — the 2026-11-19 user decision in item 21), plus the two 10a hand-overs (§3.4).
- T-112's absent / unreadable rule made per mount (§3.3).
- The rules 9b inherits for moves, copies and trash across mounts (§3.6).

**Out.** The move / copy / delete / trash / restore / empty commands themselves (step 9b). Screens (steps 11–13). Per-PC data keyed by the mount id (favorites, T-72). Export / import (T-73).

## 1. Facts the design relies on (checked at `727590ae`)

- **One root everywhere.** `gate.rs` `State { catalog, root: Option<PathBuf> }`; one `Numbers.root` generation; `LibraryService` `set_root` · `first_start` · `default_place` · `start_boot_scan` · `status`; `config.rs` `library.json` `{"schema_version":1,"root":"default"|<abs>}` (ADR-0375). Daemon `DataLayout::default_library_root` = `<dist>\library` (release) or `<data>\library`.
- **The disk join already takes the root.** `LibPath::to_path(root)` · `from_disk(root, …)` · `key(root)` — about 30 call sites in 8 files (`catalog` · `create` · `edit` · `launch` · `lib` · `relink` · `resolve` · `store`). `LibraryStore` methods take `root` + a disk path, `trash(root, path, bundle)` included, so the store can stay mount-unaware. `catalog::key` keys the catalog by the spelling through platform `path_key` (case-folded on Windows).
- **Built-in.** `AssetRef { Library(LibPath), BuiltIn(BuiltinId) }`; `BuiltinId` is a closed set of 3 read once from `builtin\`. `engram:/` appears in 161 Rust lines over 21 files and 6 lines of `builtin/` and `prompts/`. `edit.rs` `lib_path` refuses a built-in (`item-path-invalid`); `library.list` already answers `engram:/default` in `builtins` when asked (`builtins: true`). Agents carry the built-in preset as the `default_preset` bool, not as a path.
- **Agent attachments** are `profile.rs` `AttachedPreset { path: String }`; `ProfileEditPort::rewrite_preset_paths` / `remove_preset_paths` match whole paths by `same_path` (no prefix match — a caller expands a folder per attached item).
- **Reference index.** `Catalog::presets_listing(key)` answers the presets listing a target, missing targets included; relink rewrites files through `LibraryService::rewrite_in` and agents through the port, presets first, no journal (ADR-0335).
- **Launch** (T-112): `resolve.rs` `RootState { Unset, Absent, Unreadable, Ok }` with one producer `RootState::of`; `launch.rs` `Skips`; agent `launch.rs` `LIBRARY_UNAVAILABLE_SKIPPED` (args `message`, `items`); ④ `BlockReason::LibraryUnavailable`.
- **Platform** has `fs::file_identity` (volume + file id, follows links, `Unsupported` on volumes without ids) — but `FileIdentity` has private fields and no serialized form, so it cannot be persisted today. Also `same_path` · `path_key` · `normalize_folder` · `share_root`.
- **The crate holds no random source and reads no clock** (`LibraryValues` doc) — ids must come from a counter or the daemon.
- **Nothing here has shipped.** The asset-library crate is absent at tag `v0.4.0`, so `library.json` v1 and command catalog v9 exist only in development data folders; `catalog_version` stays 9 (the same unreleased generation — add a comment line).
- **Boot restore is off** (daemon `run()` item 9), so nothing launches before the boot scan unless a user starts it. The profile-edit port is plugged inside `build_daemon_wiring_with_store`, before `first_start`.
- Prefix-like tags in use: `folder:` (agent `launch.rs` `FOLDER_ITEM_PREFIX`) and `engram:/`. `LibPath` refuses `:`, so neither collides with a mount name.

## 2. The model and module boundaries

```
virtual root (no folder)                      library.json (schema 2)
├── engram   → <dist>\builtin   read-only     mounts: [{id, name, path}]   path = "default" | <abs>
├── library  → <dist>\library   (default)     next_id, migration
└── company  → D:\git\company-presets
reference "company/skills/review" → D:\git\company-presets\skills\review
```

- **New module `mounts.rs`** (asset-library): the mount table (`Mounts`), name rules, the overlap check, and the one join: a `LibPath` → `(mount, disk path)`; a disk path under a mount → its `LibPath`. Every `to_path` / `from_disk` / `key(root)` caller goes through it. `LibPath` itself stays a plain validated path (no mount type, no new syntax — ADR-0396 decision 2); a path of one segment names a mount.
- **`gate.rs`** `State.root` → `State.mounts`; one write lock and one generation for the whole tree (§8 c1). The first-scan state is kept per mount (§5).
- **`store.rs` stays per folder.** The service resolves the mount first and passes the mount folder as `root`; `trash` therefore lands in that mount's `.trash` with the path relative to the mount.
- **`config.rs`** reads schema 1 and 2, writes 2 (§4).
- **`builtin.rs`** keeps reading the three files once; the `engram` mount is a view over it, not a scanned folder (§3.4).
- **Rename walk** = one function shared by the mount rename, the migration and 9b's move fix-up: rewrite every reference whose leading names match a mapping, in the preset files of every readable mount, then agent attachments once (expanded per attached path, since the port matches whole paths). It reuses `rewrite_in` and `rewrite_preset_paths`.
- **agent** gains the new command declarations and the `mount` notice arg; **daemon** passes the built-in folder as before and `NoLibrary` answers the new commands `library-unavailable`. **platform** is unchanged (the re-add rule of §8 c2 is deferred to T-72).
- No `cfg` outside platform; overlap and identity checks use platform functions.

## 3. Behaviour

### 3.1 The virtual root
- **Reads.** `library.list` with no `folder` lists the mounts as folders (each with its state); `library.folders` includes each mount as a top folder; `library.read` of a one-segment path answers the mount folder's row. With no mount registered the virtual root still reads: an empty list, never `library-unavailable` (this closes the item-21 complaint that `library.list` with no root refuses).
- **Nothing is created at the top.** `library.newFolder` without `parent` and `library.newFile` without `folder` refuse `INVALID_ARGUMENT` `virtual-root-read-only`. A one-segment target of any write (and, in 9b, of a move, rename or delete) is refused the same way — a mount's top is changed only by the mount commands, and Engram never deletes a mount's real folder.
- **A path whose first segment names no mount** reads as absent (not found), never as a path under some default mount.
- **Name comparison** of mount names = platform `path_key` (the catalog rule), so `Company/x` and `company/x` are the same reference on Windows.

### 3.2 Mount lifecycle (user-accepted handling 2026-11-21, plus ★ choices)
- **Add** (`library.addMount {name, path, create?}`): name checked (UD2); path absolute and normalized; created first with `create`; refused when it is the same folder as a mount, inside one, or contains one (`mount-overlaps` `{with}`), or overlaps the daemon data folder's other parts, the launch folder or the built-in folder. Same-folder and nesting are judged by path key **and** by `file_identity` of the folder and its ancestors, so a junction, a mapped drive or a second spelling of the same folder is caught; a volume without ids falls back to the path rule. The folder must stat as a folder (as `set_root` today). Then a mount ticket: the new mount is walked and its items applied; other mounts' items stay.
- **Remove** (`library.removeMount {mount}`): drops the registration only — no file is touched, ever. Its items leave the catalog; references to them become absent and follow ADR-0387 (launches skip with one notice per mount). Running agents are unaffected (their materials are already prepared; folders stay on disk). The answer names how many preset lines in other mounts and how many agents now point at the removed mount (UD8: no confirmation).
- **Rename** (`library.renameMount {mount, to, yes?}`): the rename walk rewrites `old/…` → `to/…` in the preset files of every readable mount (the renamed mount's own included) and in agent attachments, **then** renames the registration — so a stop midway leaves the old name registered and running the same rename again finishes it (relink's no-journal rule, ADR-0335). Without `yes`, when any file would be rewritten, it applies nothing and answers the plan: files per mount, and which mounts are git working trees (UD7). Files in a mount that cannot be read are reported as skipped; their references stay broken and are fixed by hand (ADR-0396 decision 3). A case-only rename is allowed.
- **Relocate** (`library.relocateMount {mount, path, create?}`): re-points a mount at another folder under the same name (the old `setRoot` for one mount; the Lightroom 「Find Missing Folder」 shape) — references do not change. Same overlap rules as add; the mount is re-walked.
- **Read-only / unsaved config** keeps today's rules for every mount command: a read-only `library.json` refuses `library-config-read-only` before anything; a failed save after the change answers `library-config-write-failed`, the change stays live and `config` says `unsaved` (ADR-0381).
- Each mount change raises the one generation, so an edit carrying an older `root_generation` is refused `root-generation-stale` (ADR-0377), whichever mount it touches.

### 3.3 Launch and read entrances per mount (T-112 hand-over)
- `RootState` is computed **per mount** with the same producer `RootState::of` (stat + that mount's first-scan answer). A reference resolves through its mount's state:
  - mount not registered, or registered and absent (missing / not a folder / absent local drive) → its items are `Missing`; a launch skips them with **one `library-unavailable-skipped` per mount**, args gaining `mount` (the name) — `message` says 「not registered」 or the stat answer;
  - mount exists but cannot be read, did not answer, or its share is not reachable → ④ `LibraryUnavailable` with `mount` in the reason args; a launch that reaches it is refused, and — as today for the root — it does no folder IO and no command-line check;
  - `Ok` → as today.
- At the read entrances one ④ row per unusable mount stands for that mount's items (ADR-0361 / ADR-0387 per mount); other mounts expand normally. Built-ins never block (unchanged).
- **Inherited unchanged from T-112:** the split by row kind (`Resolution::skipped` / `blocked`), `RootState` `Absent` / `Unreadable` and its one rule, the typed `stat_root`, the share probe (`BoundedIo::stat_all` + platform `fs::share_root`), `RootStatus::Unreadable` (now per mount), both skip codes, `folder-unreadable`, the launch note rules, the `warn` logs, notices once per agent. Nothing T-112 built is removed.

### 3.4 The built-in (★ UD5 B)
- The built-in is the reserved read-only mount `engram`, always present (even with no user mount), backed by `builtin.rs`. References are spelled `engram/default`, `engram/priming`, `engram/settings/default`; `AssetRef::parse` maps a first segment `engram` to `BuiltIn` and refuses any other id under it (`UnknownBuiltin`). The old spelling `engram:/…` is rewritten by the migration (§3.5) and then refused with a clue (`item-path-invalid`, `why` names the new spelling).
- `library.list {folder: "engram"}`, `library.read engram/default` and `library.folders` work like any mount; writes into it refuse `mount-read-only`. `library.list`'s `builtins` flag and answer field are removed (the folder replaces them).
- Special handling kept: read once, an unreadable built-in becomes an empty stand-in + `builtin-unavailable` (ADR-0361), the `default_preset` bool, no `.trash`, never renamed, removed or relocated (`mount-read-only`).
- **10a hand-over 1** — `preset.get engram/default` keeps the `builtin-unavailable` notice when the built-in folder is unusable (`edit.rs` `complete_view` today keeps only resolution problems).
- **10a hand-over 2** — `PresetSection` answer words become lowercase (`presets` · `instructions` · `skills` · `settings` · `folders`), the same words as the YAML keys and the refusal arg `list` (`format/read.rs` `key`). `FolderLoadKindWord` stays as it is: no lowercase counterpart of its words was found in refusal args (§12).
- If the user picks UD5 A instead, §3.4 shrinks to: `engram:/` kept; `library.read engram:/…` and `library.list {folder: "engram:"}` accepted as a special case; `engram` still reserved as a mount name; both 10a fixes as above.

### 3.5 The paths already written (★ UD1 a)
- Schema 1 `library.json` reads as one mount (UD1 name, `path` kept, `"default"` stays the word) and records `migration: {mount, state: pending}` in schema 2 at the first save.
- **Agent attachments** are rewritten in `first_start` (registry only, no library IO — the port is plugged before it): `p` → `<mount>/p`.
- **Preset files** are rewritten by the boot-scan job right after that mount's first walk, through the rename walk with the mapping 「prefix `<mount>/`」 (and `engram:/x` → `engram/x` under UD5 B). When the pass ends without a stop, `migration` is cleared; a stop leaves it pending and the next start runs it again.
- **Idempotent by rule:** a path whose first segment already is the migrated mount's name, or `engram` followed by a built-in id, is left as is. So the migrated mount's name is chosen not to equal any top-level entry of the old root (`library`, else `library-2`, …), and legacy paths that would still be ambiguous (`engram/default` · `engram/priming` · `engram/settings/default` as library paths) are logged at `warn` and left for relink.
- A launch started by a user during that first boot scan can skip a not-yet-rewritten nested reference once, with a notice (§11 R4).
- v1 never shipped: the v1 reader and the migration can be deleted once no v1 file is left (record a tracking item).

### 3.6 What step 9b inherits (rules decided here; 9b builds the commands)
- **Trash per mount:** delete = rename into `<mount folder>\.trash\<bundle>\<path from the mount>` (ADR-0320 per mount); a bundle keeps the mount-relative path, so a later mount rename does not break restore; restore returns into the same mount. `.git/info/exclude` per mount that is a git working tree. Emptying covers every mount's `.trash` (9b decides the per-mount argument). A removed mount's `.trash` is left on disk, unreachable from Engram. The built-in has none.
- **Move across mounts:** a rename when both folders are on one volume; otherwise copy, and only after the copy completed move the original into **its own mount's** `.trash` as one bundle; on any failure the original stays where it was and the partial copy is removed (best effort, reported if it cannot be). Telling the two apart: try the rename and treat `ErrorKind::CrossesDevices` as 「copy」 (9b confirms what Windows answers). References are fixed by the shared rename walk after the move.
- **Copy across mounts** = the same copy path (P6), no reference fix.
- One-segment paths (mount tops) are never moved, renamed or deleted by 9b.
- 9b keeps the single generation and the mount ticket rules of §5.

## 4. Data shapes

- **Persisted — `library.json` schema 2** (base data-file rules, ADR-0291 · ADR-0376): `schema_version: 2`, `mounts: [{id, name, path}]` (`path` = `"default"` or an absolute normalized folder; order = the order shown), `next_id` (the counter ids come from), `migration` (only while pending). A schema-1 file is read by a v1 reader (R3: an earlier edition is read only with a reader). An older build meeting schema 2 treats it as newer: no mounts, mount commands read-only (R2) — accepted, nothing shipped. The built-in mount is not stored.
- **Persisted — agents.json**: shape unchanged; attachment strings change once (§3.5). `notice_fingerprint` sees the new `mount` arg, so each skip notice shows once more after the upgrade.
- **Wire — `library.status`** (★ UD3): `{daemon_id, mounts: [MountRow], config, config_why, suggested_new, root_generation}`, `MountRow = {name, path, default: bool, builtin: bool, state, state_why}`; `root`, `root_setting`, `state`, `state_why` leave the top level. The mount id is not on the wire.
- **Wire — new commands:** `library.addMount {name, path, create?}` · `library.removeMount {mount}` · `library.renameMount {mount, to, yes?}` · `library.relocateMount {mount, path, create?}`; each answers the status after the change (rename and remove also their report: `fixed`, `written`, `skipped`, `stopped` like relink; remove: `presets`, `agents` counts). `library.setRoot` is removed. Refusal codes: `mount-name-invalid` · `mount-name-taken` (INVALID_ARGUMENT / CONFLICT), `mount-overlaps {with}` · `mount-read-only` · `mount-rename-unconfirmed {plan}` (CONFLICT), `mount-not-found` (NOT_FOUND), `virtual-root-read-only` (INVALID_ARGUMENT); today's `library-config-*` and `item-path-invalid` reused.
- **Wire — notices and rows:** `library-unavailable-skipped` args gain `mount`; ④ reason args gain `mount`; `PresetSection` words lowercase; `library.list` loses `builtins`.
- **Bindings:** agent `bindings/*.ts` and `commands.schema.json` regenerated (status, the four commands, `PresetSection`, list args/answer). `catalog_version` stays 9 with a comment line (§1).

## 5. Concurrency

- **One library lock, one generation** for the whole tree (§8 c1). Lock order unchanged: root-change mutex (renamed mount-change; every mount command and the first start take it) → library lock → turn mutex. Across crates unchanged: library lock before the manager's and the registry's.
- **A mount change is a root ticket** (exclusive from ticket to publish). Add / relocate walk the one mount outside the lock and apply it under the write lock after dropping that mount's range; remove drops the range; rename re-keys it. Other mounts' items are not rebuilt.
- **First scan per mount.** The boot scan walks every mount; a reader waits only for the mounts its range touches (the virtual-root list waits for none; a `query` waits for all). A mount whose walk failed answers `library-unavailable` with `mount` for its range, and its next reader or a `refresh` of it walks again — today's single-root rule per mount. A dead share delays only its own mount.
- **Rename walk** runs inside one `change_at` write lock (presets, then agents), as relink.
- Launch checks still skip the first-scan entrance and stat each mount they reach in the required phase (one stat per mount per launch, cached in `LaunchIo`).

## 6. Texts to change

| Text | Change |
|---|---|
| `prompts/engram-help.md` `## library` | Paths start with a mount name; the built-in is `engram/…`; the four mount commands; `library.status` answers mounts; `setRoot` gone; nothing at the top; skip notices name the mount. |
| agent `commands.rs` library docs | Status shape, the four commands, `list` / `newFolder` / `newFile` unspecified folder = refused at the top, `ProblemRow` / notice docs (`mount`). |
| asset-library headers | `lib.rs` (the root → mounts), `config.rs`, `gate.rs`, `path.rs` (`engram/…`), `store.rs` (root = a mount folder), `resolve.rs` / `launch.rs` (per mount). |
| `builtin/` presets, tests, priming | `engram:/` → `engram/` (UD5 B); priming has no `engram:/` today. |
| `docs/reference/structure/asset-library.md` | Header line. |

## 7. User decisions (open)

**UD1 — today's single root and the paths already written.**
- Name of the mount it becomes: ★ (a) the folder's own disk name (the default place → `library`), adjusted to avoid a top-level entry of the same name; (b) a fixed word (`main`); (c) asked once through the CLI. Precedent: VS Code multi-root names a workspace folder after its disk folder and lets a `name` override it (`docs/research/multi-library-reference-paths-survey-2026-10-15.md` — `name` property, collector report). The name can be renamed later (§3.2).
- Paths already written: ★ (a) rewrite once — attachments at the first start, preset files after that mount's first walk, idempotent by rule (§3.5); cost ≈ the rename walk with a 「prefix」 mapping + a pending marker, removable later because v1 never shipped. (b) No rewrite — every written reference shows 「not found · relink」 and launches skip those items until the user (or an LLM) relinks folder by folder (`library.relink` part (b)); zero code, but every agent with library presets loses them until then. (c) Keep reading an unprefixed path as that mount — **a second meaning for every path**: every resolve, key and comparison must try two readings, a mount name equal to an old top folder is ambiguous forever, and it is the 「unprefixed = base」 rule ADR-0396 rejected. Precedent for a bulk rewrite on renaming a root: not found — looked at Unreal, VS Code, Claude Code marketplaces and Obsidian in the survey above (Unreal leaves redirectors instead); our own relink (ADR-0335) is the model.

**UD2 — mount name rules.** ★ (a) The rules of one `LibPath` segment (Unicode allowed; no `/ \ :` or control character, no `.`/`..`, no name ending in `.` or a space, no Windows device name — ADR-0362), plus: no leading `.` (dot entries are hidden and `.trash` lives in a mount), reserved `engram`, unique by platform `path_key` (case-insensitive on Windows), at most 64 characters. (b) ASCII lowercase identifiers `[a-z0-9-_]` — no case question across OSes, but the user's own examples are Korean (`업무/…`, ADR-0315 `스킬/리뷰/review`). Whether the name must equal anything on disk: settled no by ADR-0396 「영향」 (the folder's place and disk name are free). `folder` need not be reserved — `folder:` carries a `:` that no `LibPath` accepts. Precedent: Unreal mount names and Claude Code marketplace names are one-name-one-source (survey above, conclusion 1 and table row ①); case-insensitive uniqueness follows our catalog rule (ADR-0325).

**UD3 — mount management commands.** ★ (a) `library.status` answers every mount (one query per area — `llm-command-conventions.md` §2) + four writes `addMount` · `removeMount` · `renameMount` · `relocateMount`, each answering the status after the change; `setRoot` removed. (b) Keep `setRoot` as 「relocate the first mount」 beside the new commands — **two commands change a mount's folder**, so help, tests and the first-mount notion stay forever. (c) One `library.setMounts {mounts}` whole-list write — one call, but a rename hidden in a list diff cannot carry the rewrite plan or `yes`. Precedent: Claude Code registers sources with `claude plugin marketplace add` / remove (`docs/research/library-multi-root-display-survey-2026-10-14.md` conclusion 5); Lightroom re-points a missing folder with 「Find Missing Folder」 (same survey, conclusion 6) — the relocate command.

**UD4 — first start.** ★ (a) Register the default place `<dist>\library` (created) as one mount named `library` — today's behaviour plus a name (ADR-0312 decision 5, user G35 「최초 디폴트 등록해둬」). (b) Register nothing; the virtual root starts empty and the user adds a mount. (c) (a) plus a shipped sample mount — no sample library exists yet (ADR-0385 맥락). Precedent: apps that own the repository propose a default place — Calibre's 「Calibre Library」, Obsidian 「Create new vault」; VS Code, which only shows folders, proposes none (`docs/research/library-empty-state-first-run-survey-2026-10-15.md` conclusions 2–3).

**UD5 — the built-in `engram:/`.** ★ (B) A reserved read-only mount `engram`, spelled `engram/…`, always present; `engram:/` rewritten by UD1's pass then refused with a clue (§3.4). This is the 2026-11-19 user decision (「the built-in becomes one read-only root among the roots」) carried into the mount model, and gives one reference form (ADR-0396 decision 2). Cost: ~170 spelling lines, mostly tests; `engram` reserved. (A) Keep `engram:/` and special-case `list` / `read` — **two reference forms**: every parser, comparison and help text keeps both, and the tree needs a fake node for `engram:`. Precedent: Unreal's engine content is a mount point (`/Engine/`) referenced in the same form as project and plugin mounts (`multi-library-reference-paths-survey` conclusion 1). Both options carry the two 10a fixes (§3.4).

**UD6 — which folders may be mounts.** ★ (a) Any absolute folder (a git clone anywhere, a share, `~/.claude/skills`), refused only for overlap (§3.2). **This reverses ADR-0312 decision 3** (user G37 「외부에 두지말고 engram 안으로」 — outside content is copied in, not used in place): a mounted outside folder is used in place, and Engram writes into it (preset edits, its `.trash`, its `.git/info/exclude`). (b) Only folders under one chosen parent place — keeps decision 3, but contradicts ADR-0396 decision 1's own example (「a git clone anywhere on disk」) and the reason for mounts (repositories in different places). Precedent: Claude Code (`--add-dir`, personal and project skill folders), codex, Blender and Ableton register folders anywhere; none forces one parent; Obsidian keeps one root per vault (`docs/research/multi-root-asset-library-practice-2026-10-30.md` conclusion 1).

**UD7 — renaming a mount when files change.** ★ (a) Without `yes` it applies nothing and answers the plan (files per mount, which mounts are git working trees); with `yes` it runs. (b) Run at once and only warn in the answer — the LLM caller learns about the other repositories after they changed. Precedent: none found for a confirmation on a root rename (looked at Unreal, VS Code, Claude Code, Obsidian — `multi-library-reference-paths-survey`); our own `yes` on a rename that detaches references (ADR-0339) is the model.

**UD8 — removing a mount that is referenced.** ★ (a) No confirmation: nothing on disk changes and adding it back under the same name restores every reference; the answer counts the preset lines and agents now pointing at it. (b) `yes` when anything references it. Precedent: Claude Code removes a marketplace's plugins with the marketplace and marks dropped plugins 「not found」 (`multi-root-asset-library-practice` table) — whether it asks first is not recorded; a confirmation precedent was not found (also looked at Blender's library list and Lightroom in the same surveys).

**UD9 — a read-only flag per user mount** (e.g. a company repository nobody should edit from Engram). ★ (a) Not now — only the built-in is read-only; add a flag when asked. (b) Add it now. Precedent: not found for a per-library read-only flag — Blender's Link / Append is an import method, not a library property (`multi-root-asset-library-practice`).

## 8. Main's choices and author's internal choices (reported)

- **c1 (main)** — One library lock and one generation for the whole virtual tree; any mount change raises it. Reason: no lock-order question, and the `root_generation?` argument shape keeps working. Cost accepted: a mount change makes queued edits in every mount stale (retry).
- **c2 (main)** — A mount has a stable internal id besides its name; paths and screens use the name, so later per-PC data (T-72) survives a rename. Ids come from `next_id` in `library.json` (the crate has no random source). Re-registering the same folder gets a new id — the re-add rule (same folder → same id) is deferred to T-72: it needs a second place holding mount state (`retired`, with a persisted identity) and a serialized `FileIdentity` in platform (none today), nothing uses the id before T-72, and a deleted-and-recloned repository gets a new identity anyway.
- **c3 (main)** — One catalog keyed by the virtual path (`catalog::key` of `<mount>/…`).
- **c4** — The store stays per folder; mounts are resolved in the service (`mounts.rs`), so `LibraryStore` and its fakes do not change shape.
- **c5** — A mount folder may itself be a link (junction or symbolic link), as the one root may be today (ADR-0365 per mount); links inside a mount are still never followed. Overlap is judged on the link target.
- **c6** — An unregistered mount name reads as an absent mount: one skip notice per name, not one `missing-item-skipped` per item.
- **c7** — Add and relocate require an existing folder (or `create`), as `set_root` does; a mount may later go absent.
- **c8** — A mount may not overlap the daemon data folder's other parts, the launch folder or the built-in folder.
- **c9** — Logs: one `info` line per mount change (`mount`, `path`), `warn` for a migration path left ambiguous and for a rename that skipped files (`docs/reference/logging-conventions.md`).

## 9. Test plan

QA rows (`.claude/skill-bindings/qa.md`): 「Rust 소스」 → 전체 회귀 (bindings change → sync gate). Lanes: asset-library `src/**`, `builtin/**` and daemon `src/lib.rs` → **both** claude and codex lanes; daemon `control/**` (help) → claude lane. GUI: 「에이전트 수명」 row is not touched (manager unchanged) and the frontend reads no library type → **off**, except one start scene below.

| What | Checked by | Unit |
|---|---|---|
| Name rules (UD2), reserved `engram`, case-insensitive uniqueness | `mounts.rs` unit tests | U1 |
| Overlap: same folder by two spellings, a junction to a mount, nested both ways, data/launch/built-in folders; FAT-like `Unsupported` falls back to the path rule | `mounts.rs` tests with a fake identity seam + one real-junction test | U1 |
| `library.json` v1 → one mount; v2 round trip; newer / corrupt / unreadable keep ADR-0376 | `config.rs` tests | U1 |
| Join: `<mount>/a` ↔ disk path; one-segment path = the mount; unknown mount → absent | `mounts.rs` / `path.rs` tests | U2 |
| Virtual root reads: list with zero mounts answers empty; folders include mounts; create at the top refused | `lib.rs` / `create.rs` tests | U2 |
| Scan per mount: a failing mount does not fail the others; its range answers `library-unavailable` with `mount`; refresh of it re-walks | `gate.rs` / `lib.rs` tests | U2 |
| Cross-mount reference resolves; relink across mounts | `resolve.rs` / `relink.rs` tests | U2 |
| Per-mount launch rule: absent and unregistered mount → skipped + one notice with `mount`; unreadable mount → ④ with `mount`, no folder IO; another mount's items unaffected | asset-library `launch.rs` tests; agent `launch.rs` notice golden with `mount` | U3 |
| Built-in mount: `list engram`, `read engram/default`, write refused, `engram:/` refused with clue; `preset.get` keeps `builtin-unavailable` | `lib.rs` / `edit.rs` / `path.rs` tests | U3 |
| Migration: attachments and preset files prefixed once; second run changes nothing; stop leaves `pending` and a rerun finishes; ambiguous legacy paths left + `warn` | `lib.rs` migration tests over a v1 fixture | U3 |
| Mount commands: add / remove / rename (plan without `yes`, run with `yes`, stop + rerun) / relocate; config read-only and unsaved; stale generation after a mount change | `lib.rs` / `host.rs` tests; agent `commands.rs` schema tests | U4 |
| Rename walk rewrites the renamed mount's own files and other mounts' files and agents; unreadable mount's files reported skipped | `relink.rs` (shared walk) tests | U4 |
| Daemon wiring: production manager launches through a two-mount service; `NoLibrary` answers the new commands | daemon `src/lib.rs` tests | U4 |
| Help screens name the mount commands | daemon `control/help.rs` tests | U5 |
| Live, claude lane: a preset in mount A with a skill in mount B reaches the agent | new scene in `tests/launch_kits_live.rs` (`claude_l6_…`) | U5 |
| Live, codex lane: the same over codex JSON | `codex_l6_…` | U5 |
| Live CLI scenes (manual QA): upgrade a copy of a v1 data folder → `library.status` shows `library`, attachments and presets rewritten; `addMount` a git clone in a temp folder; `removeMount` → spawn answers one `library-unavailable-skipped` with `mount`, none on a second spawn; add back → spawn clean; `renameMount` without and with `yes`; a mount under `\\<unreachable>\share` → spawn refused | `engram library.*` / `engram agent.spawn` in a spawned agent | QA |
| GUI: double-click start of an agent whose preset lives in a non-default mount | qa-verify 「GUI 실측」 | QA |

## 10. Implementation units

Sequential — `lib.rs`, `path.rs`, `gate.rs` and the test fixtures overlap in every unit. Build and tests green at the end of each. Sizes include tests (about half); the bulk of U2 is test fixtures re-spelled under a test mount.

- **U1 — mount model and persistence** (~600 lines): `mounts.rs` (name rules, overlap with the identity seam, join), `config.rs` schema 2 + v1 reader, `gate.rs` `State.mounts` holding exactly the one migrated mount; the service still behaves as one root. Platform: none.
- **U2 — virtual paths through the service** (~1,500–2,500): every `to_path` / `from_disk` / `key(root)` caller through `mounts.rs`; scan and first scan per mount; catalog keyed by virtual path; virtual-root reads and the top-level refusal; edits, create, relink across mounts; test fixtures moved under a test mount.
- **U3 — launch per mount, built-in mount, migration** (~800–1,200): `resolve.rs` / `launch.rs` / agent `launch.rs` (`mount` arg); `path.rs` `engram/…` and the ~170 spelling sites; `builtin/` files; the 10a fixes; the migration (first start + boot-scan job) and the rename walk it uses.
- **U4 — mount commands** (~800–1,000): agent declarations (status reshaped, four commands, `setRoot` removed, `PresetSection` lowercase, `list` without `builtins`), `host.rs`, daemon `NoLibrary`, rename / remove reports, bindings regenerated.
- **U5 — texts, references, live** (~300): `prompts/engram-help.md`, doc comments not already rewritten, `control/help.rs` tests, reference files, live scenes.

## 11. Risks

- **R1** — Test churn: 558 asset-library tests spell root-relative paths; U2 moves them under a test mount. Mechanical; a helper keeps the diff readable.
- **R2 (accepted, c1)** — Any mount change makes edits queued in other mounts stale; the caller retries with the new `root_generation`.
- **R3** — A rename rewrites files in other git repositories; UD7's plan answer is the only guard. Uncommitted changes in those repositories are rewritten too.
- **R4 (accepted, rare)** — During the first boot scan of the upgraded version, a user-started launch can skip a nested reference not yet rewritten, once, with a notice. Boot restore is off.
- **R5** — Legacy paths the migration cannot place (§3.5) stay broken and are logged; fixed by relink.
- **R6** — Under UD6 (a), mounting a folder a backend also reads natively (`~/.claude/skills`) can load a skill twice (natively and through a preset), and Engram's `.trash` appears inside it. Whether claude skips a dot folder there is unconfirmed.
- **R7** — A cross-drive move whose partial copy cannot be removed leaves a stray copy in the target mount (9b reports it).
- **R8** — On a case-sensitive OS, mount names follow that OS's `path_key`, so shared presets must match case exactly across OSes.
- **R9** — `file_identity` answers `Unsupported` on some volumes (FAT roots); overlap then falls back to path comparison, which a junction can defeat.

## 12. Records for main

- **Item 21's user decision (2026-11-19) predates ADR-0396**; carried as UD5 B. The parent item's 「the built-in becomes one read-only root among the roots」 maps to a mount, not a root.
- **10a hand-over 2 names `FolderLoadKindWord`** — no lowercase counterpart of its words was found in refusal args (searched asset-library). Unconfirmed what clash 10a meant; left unchanged unless main names it.
- **Input claims checked:** `gate.rs` `NO_ROOT`, `edit.rs` `lib_path` refusing a built-in, `RootState`, `Skips`, `TRASH_DIR` / `LibraryStore::trash` with no production caller, no `.git/info/exclude` code, no watcher — all confirmed. Not in the brief: `library.list` already has `builtins: true`; the library never shipped (no asset-library crate at `v0.4.0`); the crate holds no random source.
- **ADRs to write after review (implement flow §0-1):** a new ADR for the user decisions UD1–UD9 and §3.2 / §3.6 rules (implements ADR-0396 decision 4); amendments — ADR-0375 (the setting is a mount list), ADR-0381 (per mount command), ADR-0382 (first scan per mount; migration in the boot-scan job), ADR-0387 decision 2 (per mount, `mount` arg), ADR-0320 decisions 1–2 (trash in each mount; mount tops cannot be deleted), ADR-0365 (each mount folder may be a link), ADR-0312 decision 3 (if UD6 a) and decision 5 (first start registers a mount) and its 「영향」 「two root management commands」, ADR-0315 decision 3 (if UD5 B), ADR-0386 (`setRoot` retired, `list` loses `builtins`).
- **Parent TRD:** item 21's title 「several library roots」 → 「library mounts」; body → status line + link; 9b's item gains a pointer to §3.6.
- **Tracking:** delete the v1 reader and migration once no v1 `library.json` is left (v1 never shipped).

## 13. Review record

