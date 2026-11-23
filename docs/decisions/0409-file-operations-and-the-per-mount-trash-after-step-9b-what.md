# ADR-0409: File operations and the per-mount trash after step 9b: what .trash holds, no-replace renames, the wait before the ticket, staged and hash-checked copies

- 상태: 확정 (2026-11-23, 근거: user 2026-11-23 「ㅇㅇ 넘어가.」 (UD1–UD3 as recommended) · 「ㅇㅇ 그냥 주석 없애. 애초에 주석을 쓰질 말든가. 사람이 쓸일은 없으니.」 (UD4) · step-9b TRD review deep (lead, data-loss axis, Codex) PASS 2026-11-23) · 부분 폐기 by ADR-0412 (M12 the exclude line write path) · 부분 폐기 by ADR-0414 (M15 restore reads of the trash) · 부분 폐기 by ADR-0418 (UD1 rows of a bundle without a readable record)
- 관련: Amends ADR-0320 (decisions 3 and 4 the exclude line and the bundle contents) · Amends ADR-0322 (decisions 4 and 7 the emptying order and the emptied root) · Amends ADR-0324 (decision 9 the trash lock and its waiting count) · Amends ADR-0326 (decision 6 empty-all by the bundle ids shown) · Amends ADR-0339 (decision 2 the LLM detach form) · ADR-0400 (decisions 23–25) · ADR-0405 · ADR-0408 · ADR-0358 · ADR-0365 · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` · Amended by ADR-0412 (M12 the exclude line write path) · Amended by ADR-0414 (M15 restore reads of the trash) · Amended by ADR-0418 (UD1 rows of a bundle without a readable record)

## 맥락
Step 9b builds delete, move, rename, copy, the trash view, restore, emptying and permanent delete on the per-mount trash of ADR-0400 decisions 23–25, with direct removal for emptying (ADR-0408). The step-9b TRD (`docs/process/S25-preset-library/trd-file-operations.md`, reviewed 2026-11-23) settled four user decisions and the rules later work leans on: what may live in a mount's `.trash` and how Engram touches it without following links, how a trash step queues with emptying, how a copy across volumes is verified, and how the catalog stays right after a kind change.

## 결정
**User decisions (2026-11-23).** UD1–UD3: user 「ㅇㅇ 넘어가.」 to 「추천대로 정하고 리뷰로 넘어갈까요?」. UD4: user 「ㅇㅇ 그냥 주석 없애. 애초에 주석을 쓰질 말든가. 사람이 쓸일은 없으니.」.
1. **UD1 — a trash-view row is the deleted item.** A deleted folder is one row. Each bundle holds one record naming its roots; a bundle without a readable record shows its files as rows.
2. **UD2 — only `yes` + `detachRefs` pins what the caller saw.** The `CONFLICT` of a delete, a `SKILL.md` rename or a `SKILL.md` move-out carries a `plan_digest` of the referencing preset files and agents. The detach form must carry it, and it is judged at the call and again inside the hold before any disk change. Plain `yes` carries none.
3. **UD3 — a copy into its source's own folder is named ` - Copy`**, then ` - Copy (2)`. The suffix is English and does not depend on `ui.language`.
4. **UD4 — a move or rename rewrites references without a plan confirmation**, also in other mounts and git repositories, with whole-file rewrites (ADR-0358). This differs from `library.renameMount` (ADR-0405).

**What `.trash` holds.**
5. `<mount>\.trash` is checked without following links, from the mount folder down, before every trash step, view, restore, permanent delete and emptying. If it is a link or not a folder, that mount's trash is unusable: trash steps there refuse per item, and nothing is restored or removed there.
6. A bundle is a direct, non-link child of `.trash` whose name parses exactly as the bundle id (`<UTC basic time>-<short id>`). Anything else is foreign and is never listed, restored, emptied or permanently deleted. Every received address is re-checked.
7. Bundle ids are created with `create_dir` under the trash lock; a taken id gets a new id. There is one id per operation, shared by every mount the operation touches.
8. **Record lifecycle.**
   - The record is a dot-named JSON file. It is written to a base temp name and published with a no-replace rename inside the hold, and never rewritten.
   - It names every root the operation put in the bundle: a deleted item, a displaced entry, a cross-volume original, a failed or partial copy (a staging row that is never restorable).
   - Roots are read filtered by a stat that tells `NotFound` from other errors.
   - When a root goes, the now-empty folders above it are pruned bottom-up, empty-only and never through a link. The record goes when no root remains and nothing else is in the bundle, then the bundle folder.
   - Leftover non-root entries are shown as `other` rows, which are removable but not restorable.
9. A link among the names between the bundle and a row leaves that row untouched and answers `remaining`. A row that is itself a link is moved or removed as a link.

**Disk steps.**
10. Every rename that must not replace goes through platform `fs::rename_no_replace`. On Windows that is `MoveFileExW` without the replace flag; on Linux `renameat2` and on macOS `renamex_np`, through `libc`. "Unsupported" fails the item, with no fallback to an overwriting rename.
11. Emptying and permanent delete remove the listed rows one by one, then prune the bundle shell. There is no whole-bundle removal. When std refuses a read-only entry, platform `fs::make_removable` runs once and the removal is retried. On Windows that is a no-follow walk that clears read-only and skips multi-link files; on Unix it does nothing.
12. A cross-volume move or any copy stages under a base temp name in the target folder, before the ticket and holding no lock. The copy is create-new, keeps mode and modified time, refuses a source containing links, and hashes every file while copying. Inside the hold, entry tags are compared and only files whose tag was racy at copy time are re-hashed. Any change keeps both.
13. A volume probe moves nothing: an empty temp folder is created beside the source and no-replace renamed into the target, once per folder pair. `CrossesDevices` selects the cross-volume path.

**Queueing.**
14. Every step that may touch `.trash` raises the trash waiting count after its probe and copy, just before its ticket. This covers every delete, restore and copy, an overwriting move, and a cross-volume move. The step then waits until no bundle is being emptied, holding no lock, and takes ticket → write lock → trash lock. A guard releases the count on every exit. A hold that did not raise the count never waits on the trash lock.
15. One trash lock covers every mount's `.trash`. Emptying holds no library lock and no ticket, reads the mount list and folders once, and removes per bundle under the trash lock.

**Main decisions (M1–M15).**
- M1: Every operation runs under a `Change` ticket, in a hold that raises the change number before its agents pass.
- M2 / M12: The volume probe and the copies run before the ticket, in base temp names.
- M3: A move re-keys its range and then re-classifies it; a copy or restore enters its range by a folder walk after the hold.
- M3: Inside the hold, every folder whose direct children changed is re-classified, and every skill whose folder name or location changed has its `SKILL.md` re-read with the scan's item builder.
- M5: Emptying has no per-mount argument. Empty-all addresses the rows the view listed, per bundle.
- M7: A merge removes a source folder only when it is empty.
- M8: A file and a folder of the same name under `overwrite` give `kind-mismatch`.
- M12: The exclude line is always anchored (`/<mount path>/.trash/`) and escaped. It is written after the hold through `BoundedIo`, serialized by a leaf mutex, keeps the file's bytes, and is skipped when the file is a link.
- M13: Order per copied file: write → `sync_all` → times → permissions; folders get theirs after their children.
- M14: Failed or partial staging goes to the target mount's `.trash`.
- M15: The view and restore read `.trash` through `BoundedIo`.

## 거부한 대안
- **UD1 (b) no record, rows = the files of a bundle**: a deleted folder of many files would show many rows. 사용자 직접 기각.
- **UD2 (b) every `yes` pins**: one more round trip on every referenced delete, when plain `yes` writes nothing outside the items. 사용자 직접 기각.
- **UD2 (c) no pin**: the detach form rewrites files the caller never saw (ADR-0405's harm). 사용자 직접 기각.
- **UD3 (b) the UI language for the suffix**: the caller must pass the language, and an LLM copy would differ from a screen copy. 사용자 직접 기각.
- **UD4 (b) confirm like `renameMount`**: one more round trip per move and a confirmation path as large as the rename's, for a comment loss already accepted on every preset edit. 사용자 직접 기각.
- **Whole-bundle `remove_dir_all` first**: a part-way failure removes the record while folder rows survive, so a fresh view loses them (review).
- **Waiting on the trash lock inside the write lock**: a share stalling an emptying would hold the library write lock and every launch read behind it (review).
- **Re-hashing every source file in the hold**: it bulk-reads the tree under the write lock. Comparing tags alone misses a same-size edit within the timestamp resolution; re-hashing only racy files closes that gap (review).
- **Tags alone to verify a copy**: same reason (review, Codex).
- **`rekey_range` alone after a kind change**: it keeps kinds, skill names and `Skill.folder` (review, Codex and lead).
- **Unix fallback to check-then-rename**: racy for folders, and it can overwrite (review).

## 근거
- Step-9b TRD reviewed 2026-11-23, deep (lead, data-loss axis, Codex), PASS on `9cad4164` (draft 6). Draft 7 adds the lead's batched items.
- User decisions quoted above. ADR-0408 (direct removal) was decided the same day.
- Conventions:
  - Windows Recycle Bin `$I`/`$R` and freedesktop.org `.trashinfo` keep one record per trashed item (from knowledge).
  - English Windows Explorer names a copy ` - Copy`.
  - Obsidian keeps a vault-level `.trash`, which is why foreign content in `.trash` must be tolerated.
- Code at `25e10274`:
  - `LibraryStore::rename` and `trash` replace through std `fs::rename`.
  - `Catalog::rekey_range` keeps kinds.
  - `LibraryStore::exists` answers `false` on every error.
  - No trash lock existed.

## 영향 / 불변식
- **Link safety.**
  - No trash step, view, restore, permanent delete or emptying follows a link.
  - `.trash`, the bundle and every name down to a row are checked without following, from the mount folder down.
  - A row that is a link is moved or removed as a link; `make_removable` never enters a link.
  - A change that lets any of these follow a link breaks this ADR and ADR-0365.
- **Bundle-id form.** Engram lists, restores or removes only bundles whose name parses as the id form and that are real folders. Other content in `.trash` is never touched.
- **No-replace.** No library or trash rename may replace an existing entry: no overwriting fallback, and the case-only rename goes through `rename_no_replace` too.
- **Wait before the ticket.** A step that may touch `.trash` raises the waiting count before its ticket and never waits on the trash lock while holding the library lock. The count is released on every exit.
- **Record lifecycle.**
  - The record is published by a no-replace rename and never rewritten.
  - It is removed only when no root remains and the bundle holds nothing else.
  - Rows are removed one by one before the shell is pruned.
  - Absence is a `NotFound` stat, never an error read as absence.
- **Racy re-hash.** A cross-volume copy is accepted only if every entry's tag is unchanged and every racy file's re-hash equals its copy-time hash.
- Accepted:
  - Unix bind/FUSE mounts inside a removed folder are crossed.
  - Engram bundles may show in another app's own `.trash` view.
  - The racy rule's blind spots are as the catalog's.
  - A name swapped for a link between a check and a call is not caught.
