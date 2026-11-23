# TRD — step 9b: file operations, copy, trash view, restore, emptying, permanent delete

| Item | Value |
|---|---|
| **Status** | **Draft 4 · 2026-11-23** — not yet passed review (draft 2 applied ADR-0408; draft 3 recorded UD1–UD3; draft 4 applies review findings). Decisions and boundaries only. Canonical ADRs: ADR-0400 decisions 23–25 (trash per mount, git exclude, cross-mount moves), ADR-0339 (overwrite, `yes` / `detachRefs`), ADR-0335 (plan → execute → fix references), ADR-0408 (emptying and permanent delete remove directly with `std::fs`, no OS Recycle Bin), ADR-0320 · ADR-0322 · ADR-0326 as amended by ADR-0408 (trash, human-only emptying). Relies on ADR-0324 · ADR-0358 · ADR-0365 · ADR-0377 · ADR-0396 · ADR-0405 · ADR-0407. User decisions taken (§6) except one held item pending the user (§6). |
| **Parent** | [`trd.md`](trd.md) §5-3 item 9b and its step-21 hand-over (absorbed here; the item shrinks to a status line + link when this TRD is reviewed). Runs after step 21, before 10b. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Modules: `ops.rs` · `trash.rs`; Contracts: the trash lock and its order, emptying outside the library lock and its removal never following a link, what `.trash` holds (bundle form, record, the no-follow checks from the mount folder down), the in-hold re-classification after a kind change, cross-volume copy outside the hold and the volume probe, the no-replace rename; Where to look). Asset-library `lib.rs` · `store.rs` · `gate.rs` headers; platform `fs.rs` header; base `file.rs` (the public temp name). `CLAUDE.md` 「핵심 불변식」 lock order (library lock → trash lock — ADR-0324 「영향」 candidate) is main's. |
| **Line-pointer basis** | Code at `25e10274`. Code is cited by file + symbol. |
| **Not a source** | Frozen child TRDs are read only to find why something is so. The parent TRD's §1-4 `store.rs` / `ops.rs` rows, §1-6 OS-trash row (withdrawn by ADR-0408) and §1-7 file-operation rows predate mounts; where they disagree with ADR-0400, ADR-0408 or this TRD, this TRD and the ADRs win. Code wins over any document. |

## 0. Scope

**In (9b builds the service, platform and base parts).**
- Service operations on `LibraryService`: delete (→ the item's own mount `.trash`), move, rename, copy, trash view, restore, emptying and permanent delete, with plan / confirmation, per-item results and `onConflict` (Explorer rules, P5).
- Moves and copies across mounts (ADR-0400 decision 25, step-21 TRD §3.6 including 「9b must also」).
- Re-classifying kinds inside the hold when an operation adds, removes or moves a `SKILL.md`.
- The per-mount `.git/info/exclude` line (M22, ADR-0400 decision 24).
- One platform function: a no-replace rename.
- Emptying and permanent delete as direct removal with `std::fs` through the store (ADR-0408) — a link inside `.trash` is removed as a link, never followed (ADR-0365).
- A public base temp-name maker (cross-volume copies land under base's temp shape first — ADR-0335 decision 3).
- The trash lock of ADR-0324 decision 9 (not in the code yet).

**Out (step 10b and later).** The command declarations (`library.move` · `rename` · `copy` · `delete` · `trash` · `restore` · `emptyTrash`), the `LibraryCommandHost` methods and their `host.rs` mapping, the daemon's control-route refusal of `library.emptyTrash` (M6, at `call_daemon_command`'s `caller`), `engram help`, bindings, live CLI scenes. **The line:** 9b ends at `LibraryService` methods returning crate types, tested at service level; 10b adds every name an LLM or the shell can call. Screens (trash view, modals, drag) are the client stage. Automatic emptying (G11), a changeable trash place (G31), a progress record (G38), cleaning leftover temp files (G40), and any OS Recycle Bin call or pilot (ADR-0408 decision 3) stay out.

## 1. Facts the design relies on (checked at `25e10274`)

- **Store.** `LibraryStore` has `rename` (std `fs::rename` — replaces an existing file), `copy` (base `copy_atomic` — replaces, drops permissions), `trash(root, path, bundle)` (checks the destination, then std rename — a file appearing in between is replaced), `create_new`, `TRASH_DIR` = `.trash`. None of `rename` / `copy` / `trash` has a production caller. `admits` refuses dot names and base temp names, so `.trash` and base temps are never walked.
- **Base.** `write_atomic` / `copy_atomic` write `<name>.tmp<pid>.<n>` through a private `temp_path`; `is_temp_name` is public; `replace_with` removes its own temp on failure.
- **Gate.** `Numbers { change, root, swap_ins }`; `Gate::rename_mount` is a `Root`-ticket, one-write-lock primitive with a `raise` closure that bumps root generation and change number before the agents pass; `LibraryService::change_at` (used by relink) checks `root_generation` inside its write lock (ADR-0377) and raises the change number at its end. **No trash lock exists.**
- **Rename skeleton.** `rename.rs`: plan under the mount-change mutex → confirmation (`yes` + `root_generation` + `plan_digest`, `confirmation_digest`) → the same digest judged again inside the hold → presets one at a time through `rewrite_in` → raise → agents through `rewrite_preset_paths` → re-key (`Catalog::rekey_range`). Stop rules = relink's (ADR-0335).
- **Agent port.** `ProfileEditPort` (agent `profile_edit.rs`) has `rewrite_preset_paths` and `remove_preset_paths`; `AgentManager` implements it in agent `manager/profile_edit.rs`.
- **Catalog.** `Catalog::rekey_range` moves items keeping their content, tag and kind — it does not re-classify. `moved_to` maps only a mount name; a path prefix inside a mount needs its own mapper over `catalog::key` / `ref_key` (path-key compare).
- **Edit pipeline.** `LibraryService::rewrite_in` takes the file's tag and bytes, rewrites, then writes; it does not re-check the tag right before the write (R11).
- **Platform.** `fs` has `file_identity`, `path_key` / `same_path`, `link_dir` / `unlink_dir`, `share_root`, `retry_busy` — no no-replace rename. The `windows` 0.58 union already has `Win32_Storage_FileSystem` (`MoveFileExW`), so the rename needs no new feature. No unix `libc` dependency (`libc` 0.2.186 is in `Cargo.lock`). `link_dir` makes a junction on Windows and a symbolic link elsewhere — the link-safety tests use it, so they need no OS `cfg`.
- **std removal.** `fs::remove_dir_all` does not follow links — it removes a symbolic link (on Windows any name-surrogate reparse point, junctions included) as an entry and does not enter it, including when the path given is itself a link (std docs; the behaviour since the Rust 1.58.1 fix). It stops at the first entry it cannot remove. On Unix it crosses into a bind or FUSE mount below the folder (R10). This TRD does not rely on the link claim alone: §8 tests it (CI on Windows only — R8).
- **Mounts.** `mounts::overlap` judges canonical paths + identity through a `FolderProbe`; `MountPath` is the store's only path type; `moved_to` maps a leading-names prefix.

## 2. Module boundaries

```
asset-library
  ops.rs     NEW  delete · move · rename · copy · restore: plan → execute → fix references (ADR-0335)
  trash.rs   NEW  what .trash holds (bundle form, record, no-follow checks), view, emptying, the trash lock, the exclude line
  store.rs        + no-replace rename, create-new tree copy keeping mode/mtime, tree listing for the compare, a no-follow
                    check of every name from the mount folder down, remove_tree (a link is removed as a link)
  gate.rs         + a Change-ticket hold with a raise-before-agents closure; the trash lock (leaf)
  rename.rs       unchanged; its plan/digest pattern is reused, not its code paths
platform::fs      + rename_no_replace(from, to)
base::file        + a public temp-name maker (the shape is base's)
```

- The store stays mount-unaware (`MountPath` in, `MountPath` out); the cross-mount logic lives in `ops.rs` above two store calls on two mount folders.
- No OS `cfg` outside platform: the cross-volume test is `ErrorKind::CrossesDevices` from `rename_no_replace`; mode bits and modified time go through std (`set_permissions`, `File::set_times`).
- `.trash` is outside the catalog; the trash view and emptying read the disk (never under the library lock).

## 3. Behaviour

### 3.1 Common to every operation
- **Entrance.** Every operation waits at the first-scan entrance before its ticket (ADR-0324 decision 6); takes an optional `root_generation` checked inside its write lock (ADR-0377); refuses a one-segment path `virtual-root-read-only` and anything in the built-in `mount-read-only` (copy *from* the built-in is allowed — P6 · M14); refuses a path under `.trash` (ADR-0320 「영향」 — `admits` already does); passes `admits` + `first_link` before any read or write (ADR-0365); refuses a path in an unusable mount `library-unavailable {mount}` per item.
- **One hold.** The disk step that changes library paths, the catalog update (including re-classification, §3.4) and the reference fix run under one `Change` ticket and one write-lock hold. The change number is raised inside the hold **before** the agents pass (ADR-0400 decisions 18 and 25), as the rename primitive's `raise`. A `Change` ticket, not `Root`: no mount registration moves.
- **A step that will touch `.trash`** raises the trash waiting count and waits until no bundle is being emptied **before** it draws its ticket, holding no lock; then ticket → write lock → trash lock, in ADR-0324 decision 9's order. Emptying starts no new bundle while the count is above 0, so the trash lock is free when the step reaches it, and a share stalling an emptying never holds the library write lock (§5).
- **Per-item results** (P5 · M26): each given path answers `done` · `skipped` · `conflict` · `failed {code, args}`; what finished stays finished. A name collision without `onConflict` answers `conflict` for that item only and overwrites nothing. Parent and child given together = only the parent (P5). A move or copy into the folder the item is already in (with no new name) answers `skipped` for a move; a copy names a copy (UD3).
- **`onConflict`** (move, copy) = `overwrite` · `skip` · `keepBoth` (spelling is 10b's). Same-name folders merge; inside a merge each colliding file follows `onConflict`. `keepBoth` = 「(2)」 numbering (R17). Overwrite = the existing entry goes to `.trash` then the rename (ADR-0339 decision 1 — references follow the place, no choice).
- **Stop rules** (ADR-0335, as relink and the rename primitive): a per-file refusal skips that file; a read or write failure while fixing references stops before the agents; nothing is journaled. Recovery (ADR-0335 decision 2 · ADR-0339 M104): after an interrupted move the lines not yet fixed read 「찾을 수 없음」 and one relink fixes them; after an interrupted detach the lines not yet removed are kept references — removed with ×, or by restoring the item to its place and deleting it again with `detachRefs`. Leftover base temps are not cleaned (G40).
- **`yes` and `detachRefs`** exactly as §1-7 「공통 인자」 and ADR-0339 (M108 · M110): delete with references, `SKILL.md` rename, `SKILL.md` moved out → `CONFLICT` with the references and the two forms; `detachRefs` without `yes` = no `yes`; `detachRefs` on an operation with no item of those shapes = `INVALID_ARGUMENT`. The `CONFLICT` carries a `plan_digest` (referencing preset files and agents); `yes` + `detachRefs` must carry it and plain `yes` does not (UD2). The digest is judged at the call and **again inside the write-lock hold before any disk change**; a different plan answers the `CONFLICT` with the fresh plan, nothing written (ADR-0405 decision 1).

### 3.2 What `.trash` holds, and never through a link
- **The trash folder.** `<mount>\.trash` is checked fresh, from the mount folder down without following anything (a `first_link`-style `symlink_metadata` walk), before every trash step, view, restore, permanent delete and emptying. Absent → a trash step creates it with `create_dir` (never `create_dir_all` through a link). A link or not a folder → **that mount's trash is unusable**: every trash step there refuses per item `trash-unusable {mount}` (a delete, an overwrite's displaced entry, a cross-volume original, a failed copy — the operation then keeps the item in place); the view names the mount; nothing is restored, emptied or permanently deleted there.
- **What a bundle is.** A direct child of `.trash` that is a real folder (not a link) and whose name parses exactly as the bundle-id form (§4). Anything else in `.trash` — another app's files (Obsidian keeps a vault-level `.trash`), a link, a loose file — is **foreign**: never listed, restored, emptied or permanently deleted. Emptying and permanent delete re-check the form of every address they receive; a foreign address answers `NOT_FOUND`.
- **Between the bundle and the row.** Every name from the bundle folder down to the row is checked without following. A link among them → that row is left untouched and answered `remaining {code: path-through-link}` (restore, permanent delete, emptying). A row that is itself a link is moved as a link (restore) or removed as a link (emptying, permanent delete).
- **Bundle ids.** One per operation. Created with `create_dir` of `.trash\<id>` under the trash lock; `AlreadyExists` → a new id. The same id is used in every mount the operation touches; when it is taken in a later mount, that mount gets a new id and the view shows two groups (rare, accepted).
- **The record (UD1).** One per bundle folder: a dot-named JSON file (§7 M6) created **create-new** inside the hold after that mount's moves of the operation, naming every root the operation put in that bundle — a deleted item, an overwrite's displaced entry, a cross-volume move's original, a failed or partial copy. A partial copy's row is its temp-named staging folder at its `MountPath`, marked `staging {intended}`: shown in the view, never restorable (`not-restorable`), removable by emptying and permanent delete. Roots are read **filtered by existence** (a restored or removed root drops out); the record is never rewritten.
- **Only what Engram made.** A staging folder or probe (§3.5) is trashed or removed only if this operation created it; a temp-name collision at creation draws a new temp name.

### 3.3 Delete
- Plan: the items, the references to them (a folder sums everything inside — P7; a lone `SKILL.md` counts its skill folder's references — ADR-0339). Referenced and no `yes` → one `CONFLICT` for the whole call.
- Hold: the bundle (§3.2); under the trash lock each item renames (no-replace) into `<its mount>\.trash\<bundle>\<MountPath>` (parents inside the bundle made with `create_dir`, no-follow); the record; the catalog range leaves and the parent is re-classified (§3.4); with `detachRefs`: raise → preset files drop the lines of every deleted path (counted at execution — M105; a preset trashed in the same operation is not rewritten) → `remove_preset_paths` with the deleted preset paths only (none for `SKILL.md` cases). Without `detachRefs` the references stay and read 「찾을 수 없음」 until restored (P3).
- Items in several mounts → each goes into its own mount's `.trash` (§3.2 ids).

### 3.4 Catalog after a kind change
- `rekey_range` keeps kinds, so it is not enough when an operation adds, removes or moves a `SKILL.md` (delete, rename to or from `SKILL.md`, a move into or out of a skill folder, a merge). Inside the same hold, every folder whose direct children changed (source parent, target parent) is re-classified from disk with the scan's classifier (`FileKind::of` and the skill-folder rule): the folder's skill status, the moved entries' kinds, and the derived rows (`recompute_derived`). One no-follow listing per affected folder, under the write lock.
- Moves map paths with a prefix mapper over the path key (a library path inside a mount, not `moved_to`, which maps only a mount name).

### 3.5 Move and rename
- **Rename** = a move to a new name in the same folder. A taken name refuses `name-exists` (no `onConflict` on rename — §1-7). A case-only rename is allowed (Explorer) and goes only through `rename_no_replace`.
- **Refusals before any disk step:** the target inside the source (judged on canonical path and identity, as `mounts::overlap`) `into-itself`; a file onto a folder of the same name or the reverse under `overwrite` → that item `conflict` with `kind-mismatch` (§7 M8).
- **Volume probe (before the ticket, no lock).** Create-new an empty folder under a base temp name beside the source, `rename_no_replace` it into the target folder, then remove it there empty-only. Success = same volume. `CrossesDevices` = different volumes (the probe is removed beside the source, empty-only). Any other error → that item `failed` with the code; a probe that cannot be removed stays as a leftover temp (G40). The probe can go stale (a mount relocated meanwhile): the hold's generation check stops it, and a same-volume rename answering `CrossesDevices` inside the hold answers `target-mount-changed`.
- **Same volume (same mount, or two mounts on one volume):** `rename_no_replace` inside the hold; `AlreadyExists` = conflict; catalog re-key + re-classification + reference walk (step-21 §3.6 step 8).
- **Different volumes:** refuse `links-inside {links}` when any link is in the source, nothing moved (ADR-0365). Otherwise, before the ticket and outside every lock, copy every entry (dot entries and non-library files included) into a staging folder under a base temp name in the target folder: create-new only, each file's tag taken **before** its bytes are read, mode bits and modified time kept (§7 M13). Then the hold: re-list the source and compare entry by entry with the tags noted at copy time (an added or removed entry counts); where the volume gives no file identity, compare content hashes taken during the copy with a fresh hash of the source; compare the root generation. Any change stops the item and keeps both (`source-changed` / `target-mount-changed`, with the staging path). Else no-replace rename staging → final name, original → its own mount's `.trash` (the trash row is in the answer), catalog re-key + re-classification, reference walk. If the original cannot be trashed, the copy stays in place and the answer carries both paths and the code. A failed or partial copy goes into the **target** mount's `.trash` as a staging row (§3.2); the original stays. If the final rename answers `AlreadyExists` (a name appeared during the copy), the item is `conflict` and the staging folder goes to the target's `.trash` the same way.
- **Overwriting move:** copy to a staging name in the target (same-volume moves skip the copy); the target's existing entry → target mount's `.trash`; no-replace rename of the copy (or the source) into place. If that rename fails, the trashed entry is renamed back; if that also fails, the answer carries the trash row and the staging name.
- **Merge** (a same-name folder): files move one by one by the same rules; the mapping holds each moved leaf and each wholly moved folder; a source folder left empty is removed empty-only (§7 M7); one left with skipped files stays.
- **References:** the old → new mapping rewrites every preset file in every readable mount (whole files — ADR-0358) and then the agents through `rewrite_preset_paths`; every touched list keeps the first place of each path (ADR-0339). Files in an unreadable mount are skipped with `library-unavailable` and a never-walked mount is one skipped row (as relink). The answer carries the mapping, the rewritten files, the skipped ones and the git working trees they are in.
- **`SKILL.md` leaving its skill folder** (also into a subfolder — M110) follows §3.1 `yes` / `detachRefs`; with `detachRefs` the move runs, then the preset lines pointing at that skill folder drop (no agent step).

### 3.6 Copy
- Same entry rule, link refusal and staging as a cross-volume move, for every copy including one inside a mount; no reference fix; `onConflict` as move. A copy into the source itself or any of its descendants refuses `into-itself` (canonical path and identity, as move). Into the source's own folder the copy is named ` - Copy`, then ` - Copy (2)` (UD3).
- From the built-in: items only (`file_of`); `engram/settings` copies as a folder of its item; `engram` itself is a one-segment path (refused).
- The hold does only the no-replace rename into place and the catalog entry of the new range, by a folder-scoped walk after the hold (the answer waits for it, §7 M3).

### 3.7 Trash view and restore
- **View** (`library.trash` material): for each mount whose trash is usable (§3.2), the bundles: id, deleted time (from the id), and rows = the record's existing roots (UD1; no record → the files found, R7). Rows carry `{mount, bundle, path}`, the original virtual path, and `staging` where so. One combined view across mounts (one 「🗑 휴지통」 row — R18). An unusable or unreadable mount is named, not failed. Read through `BoundedIo`, no library lock, no ticket — a view can see a bundle whose trash step is still running (§3.8 addresses by row).
- **Restore** (G11 · M26): each row renames (no-replace, §3.2 checks) from the bundle back to the same `MountPath` in the same mount (the bundle lives in the mount folder, so a mount rename does not break it — ADR-0400 decision 23); missing parent folders are created with `create_dir` (G11 — the stated exception to ADR-0335 decision 7); a taken name = that row `conflict`, and a repeat with the keep-both choice restores as 「(2)」; never overwrites; a staging row is `not-restorable`. Under the trash lock and a `Change` hold; the catalog gains the restored range by a folder-scoped walk after the hold, re-classified (§3.4). References resolve again only when the row returns to its place (P3). When no root of the bundle exists any more, the record and then the empty bundle folder are removed (empty-only).

### 3.8 Emptying and permanent delete
- Human only: the service method exists in 9b; only the shell-only route reaches it (10b, M6).
- **Addressing.** Empty-all = per bundle, the rows the view listed: `{mount, bundle, rows}` (ADR-0326 decision 6 — what the view did not list stays). Permanent delete = rows `{mount, bundle, path}`. Neither given → refused (M86). No separate per-mount argument (§7 M5). Every address is re-checked: bundle form, the trash folder, the no-follow path (§3.2).
- **Lock rules (ADR-0324 decision 9 / ADR-0322 decision 7, per mount):** no library lock, no ticket; the mount list and folders are read once in a brief read lock and used to the end even if a mount is relocated or removed meanwhile; each mount is stat'ed through `BoundedIo` before its bundles (one that does not answer leaves its bundles `remaining {library-unavailable}`, untouched); per bundle: wait for the waiting count 0 → take the trash lock → remove → release.
- **Removal order (ADR-0408).** Per bundle: the listed rows one by one (`LibraryStore::remove_tree` — a file removed; a link, file or folder, junction included, removed as a link without entering it; a folder removed with its contents through `remove_dir_all`, whose inner links are removed as links), then — only when no root of the bundle exists any more — the record, then the empty bundle folder (empty-only). There is no whole-bundle `remove_dir_all`. A row listed but not in the bundle's current roots is `NOT_FOUND`; a root the view did not list survives with the record. A read-only entry is removed (Explorer empties read-only files): when std refuses one, the store clears read-only on that non-link entry (checked without following) and retries once. A transient lock gets the store's short busy retry. Nothing goes to the OS Recycle Bin.
- **Results.** Per row: **gone** · **remaining** `{code}` (an OS error code, or `path-through-link`; it stays and keeps its row; a folder row may be partly emptied) · `NOT_FOUND` (absent before the call — ADR-0408 decision 2). Per bundle: **gone** when the bundle folder no longer exists, else **remaining**. Judged by existence before and after.

### 3.9 `.git/info/exclude`
- After the hold of every step that created a `.trash` (delete, overwrite, a cross-volume original, a failed copy), through `BoundedIo`: walk up from the mount folder for a `.git` folder or file (`gitdir:` → the common dir via `commondir`); line `/<mount path from the repo top>/.trash/`, gitignore metacharacters escaped (`* ? [ # !`, trailing space) — ADR-0400 decision 24 · M12. Checked again whenever a bundle is created there (repairs a removed line). The file's bytes are kept as they are; a newline is added first when the file does not end with one; the line is appended through `write_atomic` only when absent. Skipped with `warn` when the exclude file (or `info`) is a link, or its read fails other than `NotFound`. A failure never fails the step (M22).

### 3.10 Platform functions
- **`fs::rename_no_replace(from, to)`** — Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` / `MOVEFILE_COPY_ALLOWED` (files and folders; another volume answers `ERROR_NOT_SAME_DEVICE`, which std maps to `CrossesDevices` — confirmed by U1's tests); Linux `renameat2(RENAME_NOREPLACE)`, macOS `renamex_np(RENAME_EXCL)` (§7 M11). A file system that does not support it (`EINVAL` / `ENOTSUP`) answers `Unsupported` → that item `failed {no-replace-unsupported}`; never a fallback to an overwriting rename. Busy retries through `retry_busy`.
- No Recycle Bin function (ADR-0408 decision 3).
- Gates: new per-OS code stays inside platform; `libc` joins the platform gate 5 hand list in the same commit.

## 4. Data shapes

- **Persisted:** `library.json` and `agents.json` unchanged. New: `<mount>\.trash\<bundle>\` folders keeping each root's `MountPath` (ADR-0320 · ADR-0400 decision 23) and one dot-named record per bundle (§3.2, §7 M6). Bundle id = `<UTC basic time, second>-<short id>` (e.g. `20261123T081500Z-a1b2c3` — the exact pattern is the parse rule of §3.2), one per operation.
- **Service answers** (10b maps them 1:1 onto the wire): per-item results (§3.1); the old → new mapping; removed paths and changed presets / agents (ADR-0339 · M103); the trash rows of displaced entries and originals; for a cross-volume stop, the staging path; the trash view rows; emptying results per row and bundle (gone / remaining / `NOT_FOUND`) with the code of a remaining entry as a diagnostic field (ADR-0408 decision 2).
- **New codes** (names are placeholders, final in 10b): `into-itself` · `kind-mismatch` · `links-inside {links}` · `source-changed` · `target-mount-changed` · `trash-unusable {mount}` · `path-through-link` · `not-restorable` · `no-replace-unsupported` · `delete-unconfirmed` / `skill-unconfirmed` (CONFLICT, two forms, `plan_digest`) · `detach-refs-invalid` (INVALID_ARGUMENT); reused: `name-exists` · `not-found` · `virtual-root-read-only` · `mount-read-only` · `library-unavailable` · `root-generation-stale`.

## 5. Concurrency

- **Lock order:** mount-change mutex → library lock → trash lock → turn mutex (the trash lock and the turn mutex are leaves). Across crates unchanged: library lock before the registry's.
- **One trash lock for every mount** (§7 M4) with the waiting count and hand-over of ADR-0324 decision 9. A step that will touch `.trash` raises the count and waits for the trash lock to be free **before its ticket**, holding nothing (§3.1); it lowers the count after its last trash move. So emptying never lists a half-filled bundle for removal, never starts a bundle while a step waits, and a stalled emptying holds only steps that have not yet taken the library lock.
- **Before the ticket, outside every lock:** the volume probe and cross-volume copies, into names the scan skips; the hold re-checks the source and the generation. Same-volume moves do the rename inside the hold (one `MoveFileExW`).
- **`agent.new` race:** the move's and the detach's hold raise the change number before their agents pass, so an `agent.new` judged before them judges again (ADR-0400 decision 18).
- **Not promised:** the cross-volume compare (and, on a volume without identities, the source re-hash) and the §3.4 re-classification read the disk under the write lock (R2). A share that stops answering mid-removal holds emptying with no time limit (R4) — but not the library lock.

## 6. User decisions

**Taken.** UD1–UD3 decided 2026-11-23 as recommended — user 「ㅇㅇ 넘어가.」 to 「추천대로 정하고 리뷰로 넘어갈까요?」. ADR-0408 (direct removal, no OS Recycle Bin) decided the same day — user 「그냥 표준함수 한줄로 지우기해.」.

| # | Decision | Source | Rejected |
|---|---|---|---|
| UD1 | A trash-view row is the deleted item — a deleted folder is one row. One record file per bundle inside `.trash` names the roots (§3.2, §7 M6); a bundle whose record is missing (a crash between the moves and the record) shows its files as rows (R7). Convention: Windows Recycle Bin (`$I` record + `$R` content per item), freedesktop.org trash (`info/*.trashinfo` + `files/`) — from knowledge, not re-fetched. Replaces parent §6-5 1 「no separate record file」. | user 2026-11-23 (above) | (b) no record, rows = the files of a bundle |
| UD2 | Only `yes` + `detachRefs` pins what the caller saw: the `CONFLICT` of a delete / `SKILL.md` rename / `SKILL.md` move-out carries a `plan_digest` of the referencing preset files and agents; the detach form must carry it; a different plan (at the call or inside the hold) answers the `CONFLICT` again, nothing written (ADR-0405 decision 1 style). Plain `yes` (keep) carries no digest. | user 2026-11-23 (above) | (b) every `yes` pins · (c) no pin |
| UD3 | Copying into the source's own folder names the copy ` - Copy` (`review - Copy.md`, then ` - Copy (2)`), English as English Windows Explorer; the daemon does not read `ui.language` (ADR-0372 English-original). Replaces PRD R17 / parent §1-7 「- 복사본」. | user 2026-11-23 (above) | (b) the UI language |

**Pending user (held by main).** A move or rename (also across mounts) rewrites preset files in other mounts, possibly other git repositories, whole — comments and quoting dropped (ADR-0358) — without a plan confirmation, while `library.renameMount` asks first when files would be rewritten (step-21 UD7, ADR-0405). The current design is kept (PRD E9 · ADR-0400 decision 25 step 8) until the user decides.

Each item checked against the PRD and ADRs first. Already decided and not asked again: trash per mount and the cross-mount move rules (ADR-0400 decisions 23 and 25); emptying human only, modal wording (ADR-0322 · ADR-0326 · G50); emptying and permanent delete remove directly, no OS Recycle Bin, gone / remaining per entry (ADR-0408); empty-all = what the view listed (ADR-0326 decision 6); restore never overwrites, 「(2)」 on request (G11 · M26); delete keeps references by default, `detachRefs` shape (ADR-0339); no automatic emptying (G11); leftover temps stay (G40).

## 7. Main decides (reported)

- **M1** `Change` ticket for every 9b operation; a sibling of `change_at` that hands the body a `raise` closure (change number only — the root generation keeps its meaning, ADR-0377). Built in U2 (delete + detach is its first user).
- **M2** The volume probe and cross-volume copies (move and copy) run before the ticket into base temp names in the target folder; base exposes the temp-name maker; the hold does the compare, the generation check and the renames.
- **M3** Catalog: a same-volume move and a verified cross-volume move re-key the range, then re-classify (§3.4); a copy or restore enters its new range by a folder-scoped walk after the hold, before the answer.
- **M4** One trash lock for all mounts (amends ADR-0324 decision 9's 「the root」).
- **M5** No per-mount argument for emptying; every address carries its mount.
- **M6** Bundle record: a dot-named JSON file in the bundle folder (a dot name never collides with a root — roots are library paths, which never start with a dot), created create-new, roots with an optional `staging {intended}` mark; deleted time from the id. Never rewritten; roots read filtered by existence; removed only after every root is gone (restore or emptying), then the empty bundle folder.
- **M7** After a merge, a source folder is removed only when empty (empty-only remove, never by name over content).
- **M8** File vs folder of the same name under `overwrite` = that item `conflict` `kind-mismatch` (Explorer refuses mixing them — to confirm in the manual comparison before U3).
- **M9** The removal is a store method (`LibraryStore::remove_tree`), so emptying is tested on `MemStore` (results, locks, hand-over) and the real-disk behaviour on `FsStore` (§8).
- **M10** A row removed part-way is **remaining** with the first error's code; the view shows what is left of it. No retry loop beyond the busy retry and the one read-only retry.
- **M11** Unix no-replace rename through `libc` (already locked); no fallback (§3.10).
- **M12** The exclude line is always anchored (`/.trash/` at the repo top too — ADR-0320 decision 3 wrote `.trash/`; anchoring avoids matching a `.trash` elsewhere in the repo).
- **M13** Copies keep Unix mode bits / the Windows read-only attribute and the modified time (Explorer keeps it). Order per file: write → `sync_all` → `set_times` → permissions (read-only last); folders get theirs after their children.
- **M14** A failed or partial cross-volume copy, and a staging folder whose final rename hit a taken name, go into the target mount's `.trash` as staging rows (§3.2).
- **M15** The trash view and restore read `.trash` through `BoundedIo` with `IO_LIMIT`; a mount that does not answer is named.

## 8. Test plan

qa table row: 「Rust 소스 · Cargo.toml · Cargo.lock」 → full regression, GUI off. No claude / codex lane path is touched (the daemon control route is 10b's). No new lane: emptying touches only temp folders the tests create (ADR-0408). Mutation checks per qa 「변이 확인」 on the marked rows (†). Real-disk rows use `FsStore` on temp folders and platform `link_dir` (a junction on Windows, a symbolic link on Unix — no `cfg` in the test); CI runs them on Windows only (R8).

| What | How | Unit |
|---|---|---|
| `rename_no_replace`: file and folder; a taken name `AlreadyExists` and nothing replaced †; case-only rename; another volume `CrossesDevices` (ignored test, needs a second volume); `Unsupported` maps to `failed`, no overwriting fallback † | platform tests | U1 |
| Store trash step uses the no-replace rename: a file appearing at the destination is never replaced † | `store.rs` tests | U2 |
| Delete: into the item's own mount `.trash` keeping `MountPath`; two mounts one bundle id; id taken → new id; record create-new names the roots; mount top / built-in / under `.trash` refused; references kept → 「찾을 수 없음」; `detachRefs` drops lines over every deleted path and passes only preset paths to the port; `SKILL.md` cases never call the port; parent §4 row 「파일 조작 · 휴지통」 delete cases | `ops.rs` tests, `MemStore` + fake port | U2 |
| Delete + detach: `agent.new` judged before the hold is judged again (registry-seam ordering) †; UD2 digest — a preset that came to reference the item after the `CONFLICT` → fresh `CONFLICT` †, and an edit landing between the confirming call's plan and its hold → fresh `CONFLICT`, nothing written † | `ops.rs` / `gate.rs` tests | U2 |
| Kind change without a refresh: delete `SKILL.md` → folder no longer a skill; rename `x.md` → `SKILL.md` and back; move `SKILL.md` into / out of a folder; catalog reads right after the call † | `ops.rs` tests | U2 (delete) · U3 (rename, move) |
| Record and view: folder = one row; missing record → file rows; restored / removed roots filtered out; staging row shown, not restorable; combined view across mounts; unreadable or unusable mount named | `trash.rs` tests | U2 |
| Foreign content: another app's files, a loose file, a link and a badly named folder in `.trash` are absent from the view, refused by address, and survive empty-all and permanent delete † | `trash.rs` tests on `FsStore` | U2 |
| Restore: same mount and path, parents created, conflict per row, keep-both 「(2)」, never overwrites †; after a mount rename returns into the renamed mount; last root restored → record and bundle folder removed | `ops.rs` tests | U2 |
| Exclude: repo top, subfolder (anchored), worktree `gitdir:` → common dir, submodule, metacharacters escaped, idempotent, bytes kept and newline added, exclude file as a link skipped, read failure skipped, failure does not fail the step | `trash.rs` tests on temp folders | U2 |
| Move / rename same volume: Explorer rules (merge, into itself refused, parent+child, same-folder move `skipped`, `onConflict` three values, conflict without it), overwrite → old to `.trash` with its record, references follow (path-key prefix mapper), first place kept; cross-mount same volume; references in an unreadable mount skipped; `agent.new` judged before a move is judged again † | `ops.rs` / `gate.rs` tests | U3 |
| Interrupted move (failure injection after the disk step / after k of N presets): unfixed lines read 「찾을 수 없음」, one relink fixes them, nothing journaled, temps left | `ops.rs` tests | U3 |
| Volume probe: same volume → nothing left; `CrossesDevices` → cross-volume path; other error → item `failed`; probe removed empty-only | `ops.rs` tests with a store seam + one ignored two-volume test | U4 |
| Cross-volume move: links refused naming them, nothing moved †; source edited during the copy → keeps both with the staging path †; no-identity volume: same size and mtime but new content → keeps both †; mount relocated during the copy → keeps both; copy failure → staging row in the target's `.trash` with its record, original in place; final rename taken → `conflict`, staging trashed; overwriting move whose last rename fails → entry restored, or trash row + staging name answered | `ops.rs` tests with a store seam answering `CrossesDevices` | U4 |
| Create-new tree copy on a real disk: never overwrites an existing name †; a junction / symbolic link inside the source refused; read-only entries and modified times kept | `store.rs` tests on `FsStore` | U4 |
| Copy: entry rule, no reference fix, ` - Copy` naming (UD3), from the built-in, into its own descendant refused †, never overwrites † | `ops.rs` tests | U4 |
| Emptying on `MemStore`: outside the library lock (list / launch check / `preset.add` finish while a fake removal is held); per-bundle hand-over; a step waiting before its ticket holds no library lock while a removal is held; one operation = one trash step; neither given → refused; rows one by one, then record, then bundle folder; a held row **remaining**, the others gone †; after that failure a fresh view still shows the surviving folder row †; absent before → `NOT_FOUND` †; mount relocated mid-emptying keeps the copied folder; a mount not answering → its bundles remaining, untouched (parent §4 row, per mount) | `trash.rs` tests with removal failure injection | U5 |
| View vs empty-all: a two-item trash step held between its items, view, then the step finishes, then empty-all with the listed rows → the late row and the record survive † | `trash.rs` tests | U5 |
| **Link safety (ADR-0365 · ADR-0408)**, real disk — an outside folder `keep\` holding entries at the same relative paths as the bundle rows, so a following implementation deletes something: (1) a link as a row; (2) a link two folders down inside a deleted folder row; (3) the bundle folder replaced by a link to `keep\` (foreign → untouched); (4) `<mount>\.trash` itself a link to `keep\` (trash unusable: delete refuses, view names the mount, nothing restored or emptied); (5) a link between the bundle and a row (e.g. `bundle\a` → `keep\` with `keep\b\x.md` behind row `a\b\x.md`) → row `remaining {path-through-link}`. For each: empty-all, permanent delete, view and restore leave every entry of `keep\` in place †; links in rows are removed or moved as links. Read-only entries in a removed folder row are removed | `store.rs` / `trash.rs` tests on `FsStore` | U5 (2–4 checked by U2's view and restore too) |
| Link-safety mutations: `remove_tree`'s link branch replaced with a following walk; the ancestor check removed — each must turn the link-safety row red | qa 「변이 확인」 | U5 |

## 9. Implementation units

Sequential — `ops.rs`, `trash.rs`, `store.rs`, `gate.rs` and the test fixtures overlap from U2 on. Build and tests green at the end of each. Sizes include tests (about half).

| Unit | Content | Lines |
|---|---|---|
| U1 | Platform no-replace rename (Windows `MoveFileExW`; Linux `renameat2`, macOS `renamex_np` through `libc` — M11; `Unsupported` without fallback), platform gate 5 list. Its tests settle `CrossesDevices` across volumes (ignored, second volume), case-only rename, `AlreadyExists` with nothing replaced. | ~250–350 |
| U2 | Base temp-name maker; store no-replace use (`trash` and `rename` stop replacing) and the no-follow name check; the `Change` hold with `raise` (M1); the trash lock and the wait-before-ticket rule; §3.2 (trash folder check, bundle form, ids, record, staging ownership); delete with keep / `detachRefs` / digest judged at the call and in the hold; re-classification on delete; view; restore; exclude line. | ~1,700–2,200 |
| U3 | Move and rename on one volume: Explorer rules, overwrite, merge, `SKILL.md` cases and re-classification, the path-key prefix mapper, reference walk across mounts, `agent.new` ordering for moves. Before coding it, a short manual comparison on this Windows (PRD risk line): folder merge, keep-both name, copy suffix, file-vs-folder collision — differences go to §6 / §7. | ~1,400–1,900 |
| U4 | Volume probe; cross-volume move and copy: create-new tree copy keeping mode and mtime (M13), tags before bytes, hashes where no identity, link refusal, compare, keep-both stops, staging rows, overwriting move's restore-back; copy (all cases, into-itself) and the built-in source. | ~1,200–1,600 |
| U5 | Emptying and permanent delete: `LibraryStore::remove_tree` (`FsStore`, `MemStore`), outside the library lock, mount stat, hand-over, rows → record → bundle folder, address re-checks, read-only retry, results; the real-disk link-safety tests. | ~500–700 |
| U6 | Texts and references: headers, `asset-library.md`. | ~200 |

## 10. Risks

- **R2 (as R3 of step 21)** The cross-volume compare (with a re-hash on volumes without identities) and the §3.4 re-classification read the disk under the write lock; a large folder delays launches' brief read locks.
- **R3** A move rewrites preset files in other mounts, possibly other git repositories, whole and without asking (decided — E9; pending the user, §6); the answer names those files and repositories.
- **R4** Emptying cannot be undone from Engram and reaches no OS Recycle Bin (ADR-0408). A folder row that stops part-way is left partly emptied and shown as remaining. A share that answers its stat and then stops mid-removal holds emptying (and steps waiting before their tickets) with no time limit; it does not hold the library lock (§5).
- **R5** A bundle restored after a mount rename keeps its `old/…` references (accepted — ADR-0400 decision 23).
- **R6** The Explorer rules were written from memory (PRD risk line); the manual comparison before U3 may change §3.5.
- **R7** A crash between a step's moves and its record leaves a bundle shown as file rows (UD1 fallback).
- **R8 (accepted, noted for main)** The Unix symbolic-link side of the real-disk tests is not run in CI (Windows-only runners); it rests on std's documented behaviour until a Unix run exists.
- **R9 (accepted unless the user decides otherwise)** In a mount that is also another app's folder (an Obsidian vault with a vault-level `.trash`), Engram's bundles appear inside that app's own trash view, and that app may empty them. Engram never touches the app's own entries (§3.2).
- **R10 (accepted)** On Unix, removing a folder row crosses into a bind or FUSE mount below it (`remove_dir_all` follows no links but does not stop at mount points).
- **R11 (pre-existing)** `rewrite_in` does not re-check the file's tag right before its write, so an outside edit landing between its read and write is lost; every 9b reference walk inherits it. No tracking entry found — main to open one.
- **R12** The no-follow checks run just before each disk call; a name replaced by a link in the instant between check and call is not caught (local write access to the mount is assumed trusted).

## 11. Records for main

- **Documents vs code:**
  - Parent §1-4 `store.rs` row lists `copy` (base `copy_atomic`) for P6 — ADR-0400 decision 25 forbids it (drops permissions, replaces). `LibraryStore::copy` has no production caller; 9b does not use it.
  - `LibraryStore::trash` and `rename` use std `fs::rename` (replaces a file) — the no-replace rule changes them (U2).
  - ADR-0324 decision 9's trash lock and waiting count are not in the code (`gate.rs`).
  - Parent §1-7 `library.copy` names the built-in source `engram:/…` — now `engram/…` (ADR-0400 decision 20).
  - Parent §1-6's OS-trash row (IFileOperation, STA thread, `Win32_UI_Shell`, pilot) and the parent §6-2 / §1-11a pilot items are withdrawn by ADR-0408; they stay as history.
  - Base `temp_path` is private; the staging and the probe need a public maker (U2).
  - `rekey_range` keeps kinds (§3.4) and `moved_to` maps only mount names — 9b adds re-classification and a path prefix mapper.
- **ADRs to write after review:**
  - New ADR: UD1–UD3 and the §3 / §7 rules (implements ADR-0400 decisions 23–25, cites ADR-0408) — what `.trash` holds (bundle form, foreign content, the record, staging rows, no-follow checks from the mount folder down), the wait-before-ticket rule, the volume probe, re-classification, M1–M15.
  - Amendments: ADR-0322 decision 7 and ADR-0326 decision 6 (addresses carry the mount and, for empty-all, the listed rows; the copied-aside mount folders replace 「the root」 and 「setRoot」); ADR-0324 decision 9 (one trash lock over every mount's `.trash`; a step raises the waiting count and waits before its ticket); ADR-0320 decision 3 (always anchored — M12) and decision 4 (a bundle holds one record naming its roots — UD1); ADR-0339 decision 2 (the LLM detach form carries `plan_digest`, judged again in the hold — UD2).
  - Already done, nothing more: ADR-0322 decisions 4 and 6 and ADR-0326 decisions 1–3 and 5 (amended by ADR-0408). ADR-0322 decision 4's 「the bundle whole first」 is replaced by rows → record → folder (§3.8) — say so in the new ADR.
  - Not ADRs (text fixes at the next doc pass): parent §6-5 1 「no separate record file」 (UD1); PRD R17 and parent §1-7 copy row 「- 복사본」 → ` - Copy` (UD3); PRD R48 / parent §1-7 `yes` four cases gain the detach-form digest (UD2).
- **Accepted risks for main:** R8 (no Unix runner), R9 (bundles visible in another app's trash), R10 (Unix mounts inside a removed folder); R11 needs a tracking entry.
- **Pending user:** the §6 held item (rewrite without a plan confirmation on moves).
- **Parent TRD:** item 9b → status line + link; §1-4 `store.rs` / `ops.rs` rows and §1-7 copy row get a pointer here.

## 12. Review record

- Not yet passed.
