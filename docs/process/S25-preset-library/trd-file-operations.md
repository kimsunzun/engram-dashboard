# TRD — step 9b: file operations, copy, trash view, restore, emptying, permanent delete

| Item | Value |
|---|---|
| **Status** | **Draft 3 · 2026-11-23** — not reviewed (draft 2 applied ADR-0408: no OS Recycle Bin, no pilot; draft 3 records UD1–UD3 as decided). Decisions and boundaries only. Canonical ADRs: ADR-0400 decisions 23–25 (trash per mount, git exclude, cross-mount moves), ADR-0339 (overwrite, `yes` / `detachRefs`), ADR-0335 (plan → execute → fix references), ADR-0408 (emptying and permanent delete remove directly with `std::fs`, no OS Recycle Bin), ADR-0320 · ADR-0322 · ADR-0326 as amended by ADR-0408 (trash, human-only emptying). Relies on ADR-0324 · ADR-0358 · ADR-0365 · ADR-0377 · ADR-0396 · ADR-0405 · ADR-0407. All user decisions are taken (§6). |
| **Parent** | [`trd.md`](trd.md) §5-3 item 9b and its step-21 hand-over (absorbed here; the item shrinks to a status line + link when this TRD is reviewed). Runs after step 21, before 10b. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Modules: `ops.rs` · `trash.rs`; Contracts: the trash lock and its order, emptying outside the library lock and its removal never following a link, the bundle record (UD1), cross-volume copy outside the hold, the no-replace rename; Where to look). Asset-library `lib.rs` · `store.rs` · `gate.rs` headers; platform `fs.rs` header; base `file.rs` (the public temp name). `CLAUDE.md` 「핵심 불변식」 lock order (library lock → trash lock — ADR-0324 「영향」 candidate) is main's. |
| **Line-pointer basis** | Code at `25e10274`. Code is cited by file + symbol. |
| **Not a source** | Frozen child TRDs are read only to find why something is so. The parent TRD's §1-4 `store.rs` / `ops.rs` rows, §1-6 OS-trash row (withdrawn by ADR-0408) and §1-7 file-operation rows predate mounts; where they disagree with ADR-0400, ADR-0408 or this TRD, this TRD and the ADRs win. Code wins over any document. |

## 0. Scope

**In (9b builds the service, platform and base parts).**
- Service operations on `LibraryService`: delete (→ the item's own mount `.trash`), move, rename, copy, trash view, restore, emptying and permanent delete, with plan / confirmation, per-item results and `onConflict` (Explorer rules, P5).
- Moves and copies across mounts (ADR-0400 decision 25, step-21 TRD §3.6 including 「9b must also」).
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
- **Agent port.** `ProfileEditPort::rewrite_preset_paths` and `remove_preset_paths` both exist (agent `manager/profile_edit.rs`).
- **Platform.** `fs` has `file_identity`, `path_key` / `same_path`, `link_dir` / `unlink_dir`, `share_root`, `retry_busy` — no no-replace rename. The `windows` 0.58 union already has `Win32_Storage_FileSystem` (`MoveFileExW`), so the rename needs no new feature. No unix `libc` dependency (`libc` 0.2.186 is in `Cargo.lock`). `link_dir` makes a junction on Windows and a symbolic link elsewhere — the link-safety tests use it, so they need no OS `cfg`.
- **std removal.** `fs::remove_dir_all` does not follow links — it removes a symbolic link (on Windows any name-surrogate reparse point, junctions included) as an entry and does not enter it, including when the path given is itself a link (std docs; the behaviour since the Rust 1.58.1 fix). It stops at the first entry it cannot remove. This TRD does not rely on that claim alone: §8 tests it on both OSes.
- **Mounts.** `mounts::overlap` judges canonical paths + identity through a `FolderProbe`; `MountPath` is the store's only path type; `moved_to` maps a leading-names prefix.

## 2. Module boundaries

```
asset-library
  ops.rs     NEW  delete · move · rename · copy · restore: plan → execute → fix references (ADR-0335)
  trash.rs   NEW  bundle ids, bundle record (UD1), trash view, emptying, the trash lock, the git exclude line
  store.rs        + no-replace rename, create-new copy keeping mode/mtime, tree listing for the compare, restore move,
                    remove_tree (direct removal for emptying — a link is removed as a link)
  gate.rs         + a Change-ticket hold with a raise-before-agents closure; the trash lock (leaf)
  rename.rs       unchanged; its plan/digest pattern is reused, not its code paths
platform::fs      + rename_no_replace(from, to)
base::file        + a public temp-name maker (the shape is base's)
```

- The store stays mount-unaware (`MountPath` in, `MountPath` out); the cross-mount logic lives in `ops.rs` above two store calls on two mount folders.
- No OS `cfg` outside platform: the cross-volume test is `ErrorKind::CrossesDevices` from `rename_no_replace`; mode bits and modified time go through std (`set_permissions`, `File::set_times`).
- `.trash` is outside the catalog; the trash view and emptying read the disk through `BoundedIo` (never under the library lock).

## 3. Behaviour

### 3.1 Common to every operation
- **Entrance.** Every operation waits at the first-scan entrance before its ticket (ADR-0324 decision 6); takes an optional `root_generation` checked inside its write lock (ADR-0377); refuses a one-segment path `virtual-root-read-only` and anything in the built-in `mount-read-only` (copy *from* the built-in is allowed — P6 · M14); refuses a path under `.trash` (ADR-0320 「영향」 — `admits` already does); passes `admits` + `first_link` before any read or write (ADR-0365); refuses a path in an unusable mount `library-unavailable {mount}` per item.
- **One hold.** The disk step that changes library paths, the catalog update and the reference fix run under one `Change` ticket and one write-lock hold. The change number is raised inside the hold **before** the agents pass (ADR-0400 decisions 18 and 25), as the rename primitive's `raise`. A `Change` ticket, not `Root`: no mount registration moves.
- **Per-item results** (P5 · M26): each given path answers `done` · `skipped` · `conflict` · `failed {code, args}`; what finished stays finished. A name collision without `onConflict` answers `conflict` for that item only and overwrites nothing. Parent and child given together = only the parent (P5).
- **`onConflict`** (move, copy) = `overwrite` · `skip` · `keepBoth` (spelling is 10b's). Same-name folders merge; inside a merge each colliding file follows `onConflict`. `keepBoth` = 「(2)」 numbering (R17). Overwrite = the existing entry goes to `.trash` then the rename (ADR-0339 decision 1 — references follow the place, no choice).
- **Stop rules** (ADR-0335, as relink and the rename primitive): a per-file refusal skips that file; a read or write failure while fixing references stops before the agents; nothing is journaled; the leftovers are fixed by relink (moves) or by repeating the delete (detach). Leftover base temps are not cleaned (G40).
- **`yes` and `detachRefs`** exactly as §1-7 「공통 인자」 and ADR-0339 (M108 · M110): delete with references, `SKILL.md` rename, `SKILL.md` moved out → `CONFLICT` with the references and the two forms; `detachRefs` without `yes` = no `yes`; `detachRefs` on an operation with no item of those shapes = `INVALID_ARGUMENT`. `yes` + `detachRefs` also carries the `plan_digest` from the `CONFLICT` (UD2); plain `yes` does not.

### 3.2 Delete
- Plan: the items, the references to them (a folder sums everything inside — P7; a lone `SKILL.md` counts its skill folder's references — ADR-0339). Referenced and no `yes` → one `CONFLICT` for the whole call.
- Hold: one new bundle id for the operation; under the trash lock, each item renames (no-replace) into `<its mount>\.trash\<bundle>\<MountPath>`; the catalog range leaves; with `detachRefs`: raise → preset files drop the lines of every deleted path (counted at execution — M105; a preset trashed in the same operation is not rewritten) → `remove_preset_paths` with the deleted preset paths only (none for `SKILL.md` cases). Without `detachRefs` the references stay and read 「찾을 수 없음」 until restored (P3).
- Items in several mounts → each goes into its own mount's `.trash` under the same bundle id.
- The bundle record (UD1) is written after the moves, inside the hold.

### 3.3 Move and rename
- **Rename** = a move to a new name in the same folder. A taken name refuses `name-exists` (no `onConflict` on rename — §1-7). A case-only rename is allowed (Explorer).
- **Refusals before any disk step:** the target inside the source (judged on canonical path and identity, as `mounts::overlap`) `move-into-itself`; a file onto a folder of the same name or the reverse under `overwrite` → that item `conflict` with `kind-mismatch` (§7 M8).
- **Same volume (same mount, or two mounts on one volume):** `rename_no_replace`; `AlreadyExists` = conflict; inside the hold with the catalog re-key and the reference walk (§3.6 step 8).
- **Different volumes** (`rename_no_replace` answers `CrossesDevices`): refuse `links-inside {links}` when any link is in the source, nothing moved (ADR-0365). Otherwise, **before the ticket and outside every lock**, copy every entry (dot entries and non-library files included) into a base temp name in the target folder, create-new only, keeping mode bits and modified time. Then the hold: re-list the source and compare entry by entry with what was copied (tags; an added or removed entry counts) and compare the root generation with the one noted before the copy → any change stops the item and keeps both (`source-changed` / `target-mount-changed`, with the finished copy's path); else no-replace rename temp → final name, original → its own mount's `.trash` (same bundle), catalog re-key, reference walk. A failed copy goes whole into the **target** mount's `.trash` (never removed by name); the original stays.
- **Overwriting move:** copy to a temp name in the target (same-volume moves skip the copy); the target's existing entry → target mount's `.trash`; no-replace rename of the copy (or the source) into place; if that last rename fails, the trashed entry is renamed back and the temp name reported.
- **Merge** (a same-name folder): files move one by one by the same rules; the mapping table holds each moved leaf and each wholly moved folder; a source folder left empty is removed with an empty-only remove (§7 M7); one left with skipped files stays.
- **References:** old → new mapping (leading names, as `moved_to`) rewrites every preset file in every readable mount (whole files — ADR-0358) and then the agents through `rewrite_preset_paths`; every touched list keeps the first place of each path (ADR-0339). Files in an unreadable mount are skipped with `library-unavailable` and a never-walked mount is one skipped row (as relink). The answer carries the mapping, the rewritten files, the skipped ones and the git working trees they are in.
- **`SKILL.md` leaving its skill folder** (also into a subfolder — M110) follows §3.1 `yes` / `detachRefs`; with `detachRefs` the move runs, then the preset lines pointing at that skill folder drop (no agent step).

### 3.4 Copy
- Same entry rule and link refusal as a cross-volume move (steps 3–4), for every copy including one inside a mount; no reference fix; `onConflict` as move; pasting into the source's own folder names the copy with the copy suffix (UD3) and numbers on (Explorer).
- From the built-in: items only (`file_of`); `engram/settings` copies as a folder of its item; `engram` itself is a one-segment path (refused).
- The copy lands under a base temp name outside every lock; the hold does only the no-replace rename into place; the new range enters the catalog through a folder-scoped walk after the hold (the answer waits for it, §7 M3).

### 3.5 Trash view and restore
- **View** (`library.trash` material): for each usable mount, the bundles in its `.trash`: bundle id, deleted time (from the id), and rows = the deleted items (UD1). Rows carry `{mount, bundle, path}` and the original virtual path. One combined view across mounts (one 「🗑 휴지통」 row — R18; Windows Recycle Bin also shows every drive's items in one view). An unreadable mount is named, not failed. Read through `BoundedIo`, no library lock, no ticket.
- **Restore** (G11 · M26): each row renames (no-replace) from the bundle back to the same `MountPath` in the same mount (the bundle lives in the mount folder, so a mount rename does not break it — ADR-0400 decision 23); missing parent folders are created (G11 — the stated exception to ADR-0335 decision 7); a taken name = that row `conflict`, and a repeat with the keep-both choice restores as 「(2)」; never overwrites. Under the trash lock and a `Change` hold; the catalog gains the restored range by a folder-scoped walk after the hold. References resolve again only when the row returns to its place (P3). A restored bundle with nothing left is removed (§7 M6).

### 3.6 Emptying and permanent delete
- Human only: the service method exists in 9b; only the shell-only route reaches it (10b, M6).
- **Addressing:** empty-all = the `{mount, bundle}` pairs the trash view listed (ADR-0326 decision 6 — a bundle created after the listing stays); permanent delete = rows `{mount, bundle, path}`. Neither given → refused (M86). No separate per-mount argument (§7 M5).
- **Lock rules (ADR-0324 decision 9 / ADR-0322 decision 7, per mount):** no library lock, no ticket; the mount list and folders are read once in a brief read lock and used to the end even if a mount is relocated or removed meanwhile; each mount is stat'ed through `BoundedIo` before its bundles (one that does not answer leaves its bundles remaining with `library-unavailable`, untouched); the bundle list is taken under the trash lock after the waiting count is 0; per bundle: wait for 0 → take the trash lock → remove → release.
- **Removal (ADR-0408):** `LibraryStore::remove_tree` — a file is removed, a link (file or folder, junction included) is removed as a link without entering it, a folder is removed with its contents (`remove_dir_all`, which removes the links inside as links). Empty-all removes each bundle folder whole; when that stops part-way (an open file, denied access), the rows still present are removed one by one, so one held file keeps only its own row. Permanent delete removes the given rows. A transient lock gets the store's short busy retry. Nothing goes to the OS Recycle Bin.
- **Results:** per bundle (empty-all) or row: **gone** (absent after the call) · **remaining** `{code: os error}` (still present — it stays in `.trash` and keeps its row in the view; a folder row may be partly emptied) · `NOT_FOUND` (absent before the call — ADR-0408 decision 2). Judged by existence before and after, not by the error alone.
- **Never through a link.** Only paths inside `<mount>\.trash\<bundle>` are removed, each re-derived from the mount folder read at the start and checked with `MountPath` (no `..`, never the `.trash` folder itself or the mount folder); a bundle folder that is itself a link is removed as a link. The link-safety rows of §8 hold this.

### 3.7 `.git/info/exclude`
- Found by walking up from the mount folder for a `.git` folder or file (`gitdir:` → the common dir via `commondir`); line `/<mount path from the repo top>/.trash/`, gitignore metacharacters escaped (`* ? [ # !`, trailing space) — ADR-0400 decision 24. Written when a delete first creates a mount's `.trash` and checked again whenever a bundle is created in it (cheap; repairs a removed line). Appended through `write_atomic` of the whole file when the exact line is absent. A failure logs `warn` and the delete goes on (the line is cosmetic for git status — M22).

### 3.8 Platform functions
- **`fs::rename_no_replace(from, to)`** — Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` / `MOVEFILE_COPY_ALLOWED` (files and folders; another volume answers `ERROR_NOT_SAME_DEVICE`, which std maps to `CrossesDevices` — confirmed by U1's tests); Linux `renameat2(RENAME_NOREPLACE)`, macOS `renamex_np(RENAME_EXCL)` (§7 M11). Busy retries through `retry_busy`.
- No Recycle Bin function (ADR-0408 decision 3).
- Gates: new per-OS code stays inside platform; `libc` (if M11 holds) joins the platform gate 5 hand list in the same commit.

## 4. Data shapes

- **Persisted:** `library.json` and `agents.json` unchanged. New: `<mount>\.trash\<bundle>\` folders keeping each item's `MountPath` (ADR-0320 · ADR-0400 decision 23) and one dot-named record per bundle listing the deleted roots (format §7 M6). Bundle id = `<UTC basic time>-<short id>`, one per operation, the same id in every mount the operation touched.
- **Service answers** (10b maps them 1:1 onto the wire): per-item results (§3.1); the old → new mapping; removed paths and changed presets / agents (ADR-0339 · M103); for a cross-volume stop, the finished copy's path; the trash view rows; emptying results per bundle or row (gone / remaining / `NOT_FOUND`) with the OS error code of a remaining entry as a diagnostic field (ADR-0408 decision 2).
- **New codes** (names are placeholders, final in 10b): `move-into-itself` · `kind-mismatch` · `links-inside {links}` · `source-changed` · `target-mount-changed` · `delete-unconfirmed` / `skill-unconfirmed` (CONFLICT, two forms) · `detach-refs-invalid` (INVALID_ARGUMENT); reused: `name-exists` · `not-found` · `virtual-root-read-only` · `mount-read-only` · `library-unavailable` · `root-generation-stale`.

## 5. Concurrency

- **Lock order:** mount-change mutex → library lock → trash lock → turn mutex (the trash lock and the turn mutex are leaves; the trash lock is taken only under the write lock or by emptying holding nothing else). Across crates unchanged: library lock before the registry's.
- **One trash lock for every mount** (§7 M4) with the waiting count and hand-over of ADR-0324 decision 9; one operation's trash step (all its moves into any mount's `.trash`) raises the count once and lowers it once, so emptying never lists a half-filled bundle.
- **Cross-volume copies run before the ticket**, outside every lock, into names the scan skips; the hold re-checks source and generation. Same-volume moves do the rename inside the hold (one `MoveFileExW`).
- **`agent.new` race:** the move's and the detach's hold raise the change number before their agents pass, so an `agent.new` judged before them judges again (ADR-0400 decision 18).
- **Not promised:** a cross-volume compare re-lists the source under the write lock, so a large folder holds launches' brief read locks for that listing (R2). Emptying a bundle holds a waiting delete or restore for that bundle's removal (ADR-0324 「영향」); a share that stops answering in the middle of a removal holds it with no time limit (R4).

## 6. User decisions

**All taken.** UD1–UD3 decided 2026-11-23 as recommended — user 「ㅇㅇ 넘어가.」 to 「추천대로 정하고 리뷰로 넘어갈까요?」. ADR-0408 (direct removal, no OS Recycle Bin) decided the same day — user 「그냥 표준함수 한줄로 지우기해.」.

| # | Decision | Source | Rejected |
|---|---|---|---|
| UD1 | A trash-view row is the deleted item — a deleted folder is one row. One record file per bundle inside `.trash` names the deleted roots (§7 M6); a bundle whose record is missing (a crash between the moves and the record) shows its files as rows (R7). Convention: Windows Recycle Bin (`$I` record + `$R` content per item), freedesktop.org trash (`info/*.trashinfo` + `files/`) — from knowledge, not re-fetched. Replaces parent §6-5 1 「no separate record file」. | user 2026-11-23 (above) | (b) no record, rows = the files of a bundle |
| UD2 | Only `yes` + `detachRefs` pins what the caller saw: the `CONFLICT` of a delete / `SKILL.md` rename / `SKILL.md` move-out carries a `plan_digest` of the referencing preset files and agents; the detach form must carry it; a different plan answers the `CONFLICT` again, nothing written (ADR-0405 decision 1 style). Plain `yes` (keep) carries no digest. | user 2026-11-23 (above) | (b) every `yes` pins · (c) no pin |
| UD3 | Copying into the source's own folder names the copy ` - Copy` (`review - Copy.md`, then ` - Copy (2)`), English as English Windows Explorer; the daemon does not read `ui.language` (ADR-0372 English-original). Replaces PRD R17 / parent §1-7 「- 복사본」. | user 2026-11-23 (above) | (b) the UI language |

Each item checked against the PRD and ADRs first. Already decided and not asked again: trash per mount and the cross-mount move rules (ADR-0400 decisions 23 and 25); a move or rename fixes references without asking, also across mounts (PRD E9 · §1-7 · ADR-0400 decision 25 step 8); emptying human only, modal wording (ADR-0322 · ADR-0326 · G50); emptying and permanent delete remove directly, no OS Recycle Bin, gone / remaining per entry (ADR-0408); empty-all = the listed bundles (ADR-0326 decision 6); restore never overwrites, 「(2)」 on request (G11 · M26); delete keeps references by default, `detachRefs` shape (ADR-0339); no automatic emptying (G11); leftover temps stay (G40).

## 7. Main decides (reported)

- **M1** `Change` ticket for every 9b operation; a sibling of `change_at` that hands the body a `raise` closure (change number only — the root generation keeps its meaning, ADR-0377).
- **M2** Cross-volume copies (move and copy) run before the ticket into a base temp name in the target folder; base exposes the temp-name maker; the hold does the compare, the generation check and the renames.
- **M3** Catalog: a same-volume move and a verified cross-volume move re-key the range; a copy or restore enters its new range by a folder-scoped walk after the hold, before the answer.
- **M4** One trash lock for all mounts (amends ADR-0324 decision 9's 「the root」).
- **M5** No per-mount argument for emptying; every address carries its mount.
- **M6** Bundle record (UD1): a dot-named JSON file in the bundle folder, written atomically after the moves; listed roots only; deleted time from the id. A bundle emptied of rows by restore is removed (record, then empty folders, empty-only removes); emptying removes the record with its bundle.
- **M7** After a merge, a source folder is removed only when empty (empty-only remove, never by name over content).
- **M8** File vs folder of the same name under `overwrite` = that item `conflict` `kind-mismatch` (Explorer refuses mixing them — to confirm in the manual comparison before U3).
- **M9** The removal is a store method (`LibraryStore::remove_tree`), so emptying is tested on `MemStore` (results, locks, hand-over) and the real-disk behaviour once on `FsStore` (§8).
- **M10** A row removed part-way is **remaining** with the first error's OS code; the view shows what is left of it. No retry loop beyond the store's busy retry.
- **M11** Unix no-replace rename through `libc` (already locked); the alternative (hard link for files, check-then-rename for folders) is racy for folders.
- **M12** The exclude line is always anchored (`/.trash/` at the repo top too — ADR-0320 decision 3 wrote `.trash/`; anchoring avoids matching a `.trash` elsewhere in the repo).
- **M13** Copies keep Unix mode bits / the Windows read-only attribute (set after `sync_all`) and the modified time (Explorer keeps it).
- **M14** A failed cross-volume copy, and a partial one, goes into the target mount's `.trash` under the operation's bundle id.
- **M15** The trash view and restore read `.trash` through `BoundedIo` with `IO_LIMIT`; a mount that does not answer is named.

## 8. Test plan

qa table row: 「Rust 소스 · Cargo.toml · Cargo.lock」 → full regression, GUI off. No claude / codex lane path is touched (the daemon control route is 10b's). No new lane: emptying touches only temp folders the tests create (nothing reaches the OS Recycle Bin — ADR-0408). Mutation checks per qa 「변이 확인」 on the marked rows (†).

| What | How | Unit |
|---|---|---|
| `rename_no_replace`: file and folder; a taken name `AlreadyExists` and nothing replaced †; case-only rename; another volume `CrossesDevices` (ignored test, needs a second volume) | platform tests | U1 |
| Delete: into the item's own mount `.trash` keeping `MountPath`; two mounts one bundle id; mount top / built-in / under `.trash` refused; references kept → 「찾을 수 없음」; `detachRefs` drops lines over every deleted path and passes only preset paths to the port; `SKILL.md` cases never call the port; parent §4 row 「파일 조작 · 휴지통」 delete cases | `ops.rs` tests, `MemStore` + fake port | U2 |
| Bundle record and view: folder = one row (UD1); missing record → file rows; combined view across mounts; unreadable mount named | `trash.rs` tests | U2 |
| Restore: same mount and path, parents created, conflict per row, keep-both 「(2)」, never overwrites †; after a mount rename returns into the renamed mount | `ops.rs` tests | U2 |
| Exclude: repo top, subfolder (anchored), worktree `gitdir:` → common dir, submodule, metacharacters escaped, idempotent, failure does not fail the delete | `trash.rs` tests on temp folders | U2 |
| `detachRefs` digest (UD2): a preset that came to reference the item after the `CONFLICT` → fresh `CONFLICT`, nothing written † | `ops.rs` tests | U2 |
| Move / rename same volume: Explorer rules (merge, into itself refused, parent+child, `onConflict` three values, conflict without it), overwrite → old to `.trash`, references follow, first place kept; cross-mount same volume; references in an unreadable mount skipped; `agent.new` judged before a move is judged again (registry-seam ordering test) † | `ops.rs` / `gate.rs` tests | U3 |
| Interrupted move (failure injection after the disk step / after k of N presets): unfixed lines read 「찾을 수 없음」, one relink fixes them, nothing journaled, temps left | `ops.rs` tests | U3 |
| Cross-volume move: links refused naming them, nothing moved †; source edited during the copy → keeps both with the copy path †; mount relocated during the copy → keeps both; copy failure → partial copy in the target's `.trash`, original in place; overwriting move whose last rename fails → entry restored, temp named; mode bits and mtime kept | `ops.rs` tests with a store seam answering `CrossesDevices` + one ignored two-volume test | U4 |
| Copy: entry rule, no reference fix, `- Copy` naming (UD3), from the built-in, never overwrites † | `ops.rs` tests | U4 |
| Emptying: outside the library lock (list / launch check / `preset.add` finish while a fake removal is held); per-bundle hand-over; one operation = one trash step; listed bundles only; neither given → refused; a bundle stopped part-way → rows removed one by one, the held row **remaining** with its code, the others gone †; absent before → `NOT_FOUND` †; mount relocated mid-emptying keeps the copied folder; a mount not answering its stat → its bundles remaining, untouched (parent §4 row, per mount) | `trash.rs` tests on `MemStore` with removal failure injection | U5 |
| **Link safety (ADR-0365 · ADR-0408)** — on a real temp disk, an outside folder `keep\` holding a file, and a link to it made with platform `link_dir` (a junction on Windows, a symbolic link on Unix — no `cfg` in the test): (1) the link as a row inside a bundle; (2) the link nested two folders down inside a deleted folder; (3) the bundle folder itself replaced by a link to `keep\`. Empty-all and permanent delete each leave `keep\` and its file untouched, the link gone, the rest of the bundle gone †. Also on (1)–(3) for the trash view (lists the link, never its target) and restore (moves the link, not the target). CI runs it on Windows only (every test job is `windows-latest`; the only Ubuntu job publishes releases), so the junction side is gated in CI. The same test exercises a symbolic link when the suite runs on Unix, but no runner does that today (§11 open item) | `store.rs` / `trash.rs` tests on `FsStore` | U5 |
| Link-safety mutation: replacing `remove_tree`'s link branch with a following walk (`read_dir` + recurse) must turn the link-safety row red | qa 「변이 확인」 on that row | U5 |

## 9. Implementation units

Sequential — `ops.rs`, `trash.rs`, `store.rs`, `gate.rs` and the test fixtures overlap from U2 on. Build and tests green at the end of each. Sizes include tests (about half).

- **U1 — platform no-replace rename** (~250–350): `rename_no_replace` (Windows `MoveFileExW` without the replace flag; Linux `renameat2`, macOS `renamex_np` through `libc` — M11), platform gate 5 list. Its tests settle what the dropped pilot used to: another volume answers `CrossesDevices` (ignored test, needs a second volume), a case-only rename succeeds, a taken file or folder name answers `AlreadyExists` with nothing replaced.
- **U2 — trash core, delete, restore** (~1,300–1,700): base temp-name maker; store no-replace use (`trash` and `rename` stop replacing); the trash lock; bundle ids and record; delete with keep / `detachRefs` / digest; view; restore; exclude line.
- **U3 — move and rename on one volume** (~1,400–1,900): Explorer rules, overwrite, merge, `SKILL.md` cases, reference walk across mounts, the `raise` closure, `agent.new` ordering test. Before coding it, a short manual comparison on this Windows (PRD risk line 「compare with the real Windows」): folder merge, keep-both name, copy suffix, file-vs-folder collision — differences go to §6 / §7.
- **U4 — cross-volume move and copy** (~1,000–1,400): create-new tree copy keeping mode and mtime, link refusal, compare, keep-both stops, partial copies to `.trash`, overwriting move's restore-back; copy (all cases) and the built-in source.
- **U5 — emptying and permanent delete** (~400–600): `LibraryStore::remove_tree` (`FsStore` and `MemStore`), outside the library lock, the mount stat before its bundles, hand-over, whole-bundle removal then per-row on a stop, gone / remaining / `NOT_FOUND` results, the link-safety tests on the real disk.
- **U6 — texts and references** (~200): headers, `asset-library.md`.

## 10. Risks

- **R2 (as R3 of step 21)** A cross-volume compare lists the source under the write lock; a large folder delays launches' brief read locks.
- **R3** A move rewrites preset files in other mounts, possibly other git repositories, whole and without asking (decided — E9); the answer names those files and repositories.
- **R4** Emptying cannot be undone from Engram and reaches no OS Recycle Bin (ADR-0408). A folder row that stops part-way is left partly emptied and shown as remaining. A mount share that answers its stat and then stops mid-removal holds the trash lock with no time limit (std removal is not bounded).
- **R5** A bundle restored after a mount rename keeps its `old/…` references (accepted — ADR-0400 decision 23).
- **R6** The Explorer rules were written from memory (PRD risk line); the manual comparison before U3 may change §3.3.
- **R7** A crash between a delete's moves and its record leaves a bundle shown as file rows (UD1 fallback).
- **R8 (accepted, noted for main)** The Unix symbolic-link side of the link-safety test is not run in CI (Windows-only runners); it rests on std's documented behaviour until a Unix run exists.

## 11. Records for main

- **Documents vs code:**
  - Parent §1-4 `store.rs` row lists `copy` (base `copy_atomic`) for P6 — ADR-0400 decision 25 forbids it (drops permissions, replaces). `LibraryStore::copy` has no production caller; 9b does not use it.
  - `LibraryStore::trash` and `rename` use std `fs::rename` (replaces a file) — §3.6's no-replace rule needs them changed (U2).
  - ADR-0324 decision 9's trash lock and waiting count are not in the code (`gate.rs`).
  - Parent §1-7 `library.copy` names the built-in source `engram:/…` — now `engram/…` (ADR-0400 decision 20).
  - Parent §1-6's OS-trash row (IFileOperation, STA thread, `Win32_UI_Shell`, pilot) and the parent §6-2 / §1-11a pilot items are withdrawn by ADR-0408; they stay as history.
  - Base `temp_path` is private; §3.6's 「base temp-name shape first」 needs a public maker (U2).
- **ADRs to write after review:**
  - New ADR: UD1–UD3 and the §3 / §7 rules (implements ADR-0400 decisions 23–25, cites ADR-0408) — UD1 one record per bundle (format M6), UD2 the detach-form `plan_digest`, UD3 the ` - Copy` suffix; plus M1–M15.
  - Amendments: ADR-0322 decision 7 and ADR-0326 decision 6 (addresses carry the mount; the copied-aside mount folders replace 「the root」 and 「setRoot」); ADR-0324 decision 9 (one trash lock over every mount's `.trash`); ADR-0320 decision 3 (always anchored — M12) and decision 4 (a bundle holds one record naming its deleted roots — UD1); ADR-0339 decision 2 (the LLM detach form carries `plan_digest` — UD2).
  - Already done, nothing more: ADR-0322 decisions 4 and 6 and ADR-0326 decisions 1–3 and 5 (amended by ADR-0408); ADR-0322 decision 4's 「the bundle whole, then the remaining entries one by one」 stays as §3.6's removal order.
  - Not ADRs (text fixes at the next doc pass): parent §6-5 1 「no separate record file」 (UD1); PRD R17 and parent §1-7 copy row 「- 복사본」 → ` - Copy` (UD3); PRD R48 / parent §1-7 `yes` four cases gain the detach-form digest (UD2).
- **Accepted risk R8:** no Unix runner exercises the symbolic-link side of the link-safety test; adding one (or recording a local Unix run) is main's call.
- **Parent TRD:** item 9b → status line + link; §1-4 `store.rs` / `ops.rs` rows and §1-7 copy row get a pointer here.

## 12. Review record

- Not reviewed.
