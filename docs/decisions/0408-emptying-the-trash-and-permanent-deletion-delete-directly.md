# ADR-0408: Emptying the trash and permanent deletion delete directly with the standard file functions, no OS Recycle Bin (amends ADR-0326 and ADR-0322)

- 상태: 확정 (2026-11-23, 근거: user 2026-11-23 「그냥 표준함수 한줄로 지우기해.」 after the step-9b TRD draft 1 showed that handing emptied entries to the OS Recycle Bin needs a COM call on its own thread and a pilot before building · the user's reading of the model: Engram keeps one trash of its own (the per-mount `.trash`), and emptying is throwing that trash away (「어차피 os로 간게 뭐 특별히 하는게 아니고 휴지통 버리기잖아」) · convention check 2026-11-23: VS Code, Obsidian, Joplin and JetBrains each keep one trash, none two)
- 관련: Amends ADR-0326 (decisions 1, 2, 3 and 5 hand over to the OS trash) · Amends ADR-0322 (decisions 4 and 6 send to the OS trash through IFileOperation) · ADR-0320 (delete = a per-mount `.trash` bundle — per mount since ADR-0400) · ADR-0365 (links are never followed) · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md`

## 맥락
ADR-0322 and ADR-0326 made emptying the trash and permanent deletion hand the entries to the OS Recycle Bin of the daemon PC (`IFileOperation` on a dedicated STA thread, one `PerformOperations` per entry) and let the OS decide. The step-9b TRD draft 1 turned that into a platform function plus a pilot that must prove, before anything is built, that the call runs under the detached daemon, opens no window, does not hang and leaves open files in place. Items reaching that call were already deleted once and kept in Engram's own `.trash`, where people and agents can list and restore them.

## 결정
1. Emptying the trash and permanent deletion remove the entries directly with the standard file functions (Rust `std::fs` — a file or a link is removed, a folder is removed with its contents; a link inside is removed as a link, never followed — ADR-0365). The OS Recycle Bin is not used.
2. Result per entry: **gone** or **remaining**, with the OS error code for a remaining entry (diagnostics only). What could not be removed (an open file, denied access) stays in `.trash` and is reported for that entry; a path absent before the call is `NOT_FOUND`.
3. There is no platform Recycle Bin function and no Recycle Bin pilot.
4. Unchanged: emptying and permanent deletion are human-only through the shell-only bus command `library.emptyTrash {items?}` (ADR-0322 decisions 1–3); the confirmation modal 「영구 삭제」 · 「항목 N개를 삭제하시겠습니까?」 (ADR-0326 decision 4 — it already says delete, not recycle); empty-all removes only the bundles the trash view showed (ADR-0326 decision 6); no automatic emptying (ADR-0322 decision 8).

## 거부한 대안
- **Hand the emptied entries to the OS Recycle Bin (ADR-0326 as written)** — a COM call on its own STA thread, per-entry result plumbing and a pilot on the daemon's detached launch, for entries already kept once in Engram's own trash; the OS bin is also absent on network shares, which step 21 made mountable. 사용자 직접 기각.
- **Delete straight to the OS Recycle Bin, no Engram trash** — an agent cannot reliably undo its own delete (no portable programmatic restore from the OS bin; on a network share the delete is permanent). Explained to the user, who kept Engram's own trash.

## 근거
- User decision 2026-11-23 quoted in 상태.
- Convention check 2026-11-23 (light): VS Code (OS trash, asks before a permanent delete when it fails), Obsidian (user picks the system trash, the vault `.trash` or permanent), Joplin (app trash), JetBrains (Local History) — one trash each.

## 영향 / 불변식
- Step 9b: no Recycle Bin unit; emptying is a per-bundle / per-entry removal under the trash rules of ADR-0322 decision 7 and the step-9b TRD.
- A removal must not follow a link out of `.trash` (ADR-0365); the step-9b TRD names how this is checked on Windows (junctions) and Unix (symlinks).
- Emptying cannot be undone from Engram, and nothing reaches the OS Recycle Bin.
