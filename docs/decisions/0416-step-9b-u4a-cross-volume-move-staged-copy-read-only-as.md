# ADR-0416: Step 9b U4a: cross-volume move - staged copy read only as listed, stagings kept or trashed and always named

- 상태: 확정 (2026-11-24, 근거: main decisions during step-9b U4a coding · code review deep (lead, risk axis, Codex) PASS 2026-11-24)
- 관련: ADR-0409 (decisions 8 and 12–14, M2, M3, M13, M14) · ADR-0415 · ADR-0414 · ADR-0365 · ADR-0400 (decision 25) · ADR-0377 · ADR-0266 · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` §3.2 · §3.5

## 맥락
Step-9b U4a built the cross-volume move: a volume probe per folder pair, a staged copy before the ticket, a compare in the hold, then the staging placed and the original trashed. The TRD (§3.5) fixed the outline. The review (four rounds) found that a copy reading files by path could read through a link swapped in after the listing (also above the item), that a no-follow open bypasses file-system filters, that a refused hold or a vanished source could hide or trash the only complete copy, and that the answered staging path did not survive a mount change.

## 결정
1. **The volume probe runs once per (item folder, destination folder) pair, then both mounts' `.trash` are checked** for an item that crosses (TRD §3.5 orders the trash check first; a same-volume move needs neither trash). A stale `root_generation` with a crossing item is refused before copying. Probe and staging folders take short fixed temp bases (`engram-probe`, `engram-staging`), so a long item name still fits.
2. **A copy reads each file only as listed.** Every name from the mount folder down is re-checked without following before the item stat, each listing, each tag, each open (busy retries included) and each folder's metadata; the file is opened without following (platform `fs::open_read_no_follow`) and the handle's kind, size, modified time and identity must match the listed tag. A non-link reparse point (a filter supplies its bytes — cloud placeholders, deduplication) is reopened normally and read only when both handles have the same identity; a link is never read. A mismatch fails the copy at that entry (copy) or answers `source-changed` (re-hash in the hold).
3. **Folder modified times are set through platform `fs::set_folder_modified`** (std cannot open a folder for that on Windows); M13's order kept. Copying times and permissions is strict for now (a failure fails the item, nothing lost) — open, see below.
4. **A staging is never lost and always named.** A failed copy names the entry it stopped at (`not_copied`, ADR-0400 decision 25.4) and goes to the destination mount's `.trash` as a staging row. Every kept staging is answered by its virtual path as made and its disk path (`staging_disk`), which survives a mount change. A source gone after the copy keeps the staging in place (`source-changed`), also when the whole call is refused. A whole-call refusal inside the hold (a stale `root_generation` — the hold compares it itself once anything was copied — or the `SKILL.md` judgement) first trashes every staging whose mount is still registered at the folder it was copied into and names them all (`args.stagings`). A name taken during the copy under skip answers `Skip::TakenDuringCopy` with the staging.
5. **A merge across volumes is refused** (`crosses-volumes`) for every `onConflict` — TRD §3.5 merges entry by entry; open, see below.
6. **An overwrite whose final rename fails but whose displaced entry is put back sends the staging to the destination `.trash`** (TRD silent). A file changed during the copy fails the copy (`library-write-failed`, staging trashed), not `source-changed`.

## 거부한 대안
- **Open by path, following** (round-1 code) — a file or a folder above the item swapped for a link after the listing is read through. (review Codex rounds 1–2; tests swapping a file, a folder inside and a folder above the item mid-listing and mid-copy)
- **No-follow open only** — `FILE_FLAG_OPEN_REPARSE_POINT` opens every reparse point as itself and bypasses its filter, so a placeholder could be read raw with matching size, time and identity. (review lead and risk round 2; measured: WOF compression hides its reparse point and reads correctly either way; cloud and dedup not measurable here — 약함 for those cases, the reopen keeps the link guarantee either way)
- **Answer the staging by its virtual path only** — after a relocate, rename or removal of the mount it points elsewhere or nowhere. (review Codex round 1; test resolves the returned location)
- **Trash every staging on a refusal or a vanished source** — the staging may be the only complete copy, and staging rows are not restorable. (review Codex round 1, risk round 2)
- **Item-name temp names** (`<name>.tmp<pid>.<n>`) — an item within ~12 units of the 255 limit fails at the probe, even inside one mount. (review risk / lead round 1; test with a 250-unit name)

## 근거
- Code review deep of U4a, rounds 1–4 (lead, risk axis, Codex), PASS 2026-11-24; asset-library 847 passed / 2 ignored, platform 152 / 5; the ignored two-volume test run for real (C: ↔ I:); WOF measurement by the coder (`compact /c /exe:xpress4k`, tag 0, bytes equal).

## 영향 / 불변식
- `ops.rs` (`relocate`, `probe_volumes`, `refuse_crossing`, `stage`, `move_in`, `move_staged`, `verify`, `refuse_staged`, `collide`, `kept`), `store.rs` (`LibraryStore::list_tree` / `copy_tree` / `content_hash`, `way_holds`, `open_as_listed`), platform `fs.rs` (`open_read_no_follow`, `opened_identity`), `fs/times.rs` (`set_folder_modified`).
- U4b (copy) reuses the probe, the staging copy and `Trashing::put_staging` unchanged; 10b carries `staging`, `staging_disk`, `staging_row`, `staging_kept`, `not_copied`, `stagings`, `Skip::TakenDuringCopy`.
- Open (user decision pending, `docs/tracking.md`): the cross-volume merge refusal; strict copying of times and permissions (shares that refuse chmod or folder times would fail every cross-volume move; Windows copies the whole attribute word).
- Accepted: every move between folders probes (one temp folder per pair); a folder swapped between its check and the open on a volume without file identity; the reopen branch is reached by no test here.
