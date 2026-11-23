# ADR-0418: Step 9b U5: emptying - empty folders as fallback rows, mount checks on absence, read-only folder links removed as links

- 상태: 확정 (2026-11-24, 근거: main decisions during step-9b U5 coding, the read-only link removal within the user's decision of 2026-11-24 (「읽기전용 따지지 말고 그냥 지워」) · code review deep (lead, risk axis, Codex) PASS 2026-11-24)
- 관련: Amends ADR-0409 (UD1 rows of a bundle without a readable record) · ADR-0409 (decisions 9, 11 and 15) · ADR-0408 · ADR-0365 · ADR-0326 (decision 6) · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` §3.2 · §3.8 · §3.10

## 맥락
Step-9b U5 built emptying and permanent delete (`LibraryService::empty_trash`, `LibraryStore::remove_tree`). The review (three rounds) found that a bundle without a readable record listed only files, so an empty folder left by a deleted folder kept the bundle forever; that a vanished mount read as an empty trash and answered rows gone; and that the removal retry ran without re-checking the way. A measurement found std refuses to remove a read-only junction with `remove_dir` but removes the link itself with `remove_dir_all`.

## 결정
1. **A bundle without a readable record shows its files and its empty folders as rows** (amends ADR-0409 UD1's "shows its files"), for the view, restore and emptying alike. An empty-folder row is removed empty-only (`remove_dir`): content written into it after the judgement stays and the row answers `remaining`.
2. **A failed fallback walk is not an absence.** Only a bundle that is gone or foreign (checked on its own) answers `not-found`; any other walk failure leaves the rows `remaining {library-read-failed}`, untouched.
3. **Absence is judged against the mount.** A check that reads an absence stats the mount folder first — by its target when the mount folder is a link (ADR-0365 allows that one link) — so a mount gone mid-call answers `remaining {library-unavailable}`, never gone or not-found.
4. **The way is re-checked on every attempt.** `remove_tree(root, path)` takes the mount folder; after a refused first attempt every name from the mount folder down is checked again without following before `make_removable` and before each retry; a changed way stops with `path-through-link`.
5. **A refused folder link is removed with std `remove_dir_all`** when it still reads as a link (re-checked without following): measured on Windows to remove a read-only junction itself with its target and attributes intact; no attribute is changed, so ADR-0409 decision 11 ("a link is never entered or changed") holds.
6. **Rows answered before the removals are judged again after them**: a `path-through-link` or `inside-root` row that the same call's removals took away answers gone.

## 거부한 대안
- **Files only as fallback rows** — an empty folder of a deleted folder keeps a recordless bundle in the view forever, contrary to TRD §3.2. (review lead and Codex round 1; test)
- **Remove an empty-folder row with the whole-tree removal** — content an outside process wrote into it after the judgement would go, though no view listed it. (review risk round 2; test)
- **Clear the read-only attribute on the link** — changes a link (ADR-0409 decision 11) where std's `remove_dir_all` already removes it as itself. (measured by the U5 coder; std's Windows implementation opens the path with `FILE_FLAG_OPEN_REPARSE_POINT` and deletes the link — Codex round 2)
- **Trust the first mount stat for the whole call** — a pulled volume makes later bundles not-found and rows gone. (review risk round 1; tests for a plain and a linked mount folder)

## 근거
- Code review deep of U5, rounds 1–3 (lead, risk axis, Codex), PASS 2026-11-24; asset-library 896 passed / 2 ignored; mutations of the per-row re-check, the retry re-checks, the linked-mount check and the empty-only removal each turn a test red (coder).

## 영향 / 불변식
- `store.rs` (`LibraryStore::remove_tree`, `remove_entry`, `remove_dir`), `trash.rs` (`empty_trash`, `empty_bundle`, `remove_row`, `judge_in_files`, `mount_gone`, `fallback_rows`), `ops.rs` (restore of an empty-folder row).
- 10b declares `library.emptyTrash` over `empty_trash` (shell-only).
- Open (`docs/tracking.md` T-131): a Unix 0555 folder inside a row, and a read-only empty-folder row (`remove_dir` refused, no `make_removable` on that path).
- Accepted: a name swapped for a link between a check and the call (ADR-0409); a linked mount folder on a volume without readable identity answers `library-unavailable` whenever a check reads an absence.
