# ADR-0363: An unreadable folder below a folder entry is skipped and named in a notice; an unreadable folder entry itself still blocks

- 상태: 확정 (2026-11-13, 근거: user decision 2026-11-12 (S25 step 7 P6 hold, after a convention survey) · landed `72effa1d` (P6 — the listing and the notice code) · `d445f03e` (P9 — the launch reply carries the notice))
- 관련: Amends ADR-0341 (decision 6, an unreadable folder below a folder entry) · ADR-0360 (launch folder checks) · ADR-0357 (notice wording) · child TRD `docs/process/S25-preset-library/trd-asset-library.md` §3-8 (folder listing, U2 · B22)

## 맥락
ADR-0341 decision 6 blocks a launch when a folder entry is missing or unreadable. To read a folder entry the codex way (instructions and skills — ADR-0341 decision 3), Engram lists the folder below the entry, down several levels (child TRD §3-8). Step 7 P6 met a case the TRD did not cover: the folder entry opens, but a folder (or an entry) below it cannot be read — access denied on one subfolder, an entry whose type cannot be read. Read literally, "unreadable folder" in decision 6 could block the whole launch for one unreadable subfolder of a large rules folder.
Main first proposed skipping, then proposed blocking after the lead reviewer pointed at ADR-0341 decision 6 and the new state skipping adds. The user asked for the convention first (「읽을 수 없는 하위 폴더는 관행을 한번 살펴봐.」). Survey: codex skill discovery, ripgrep, find, git and walkdir all skip an unreadable subfolder and warn.

## 결정
1. **Below a folder entry, a folder whose read fails (at open or part-way — the entries already read are kept) or an entry whose type cannot be read is left out of the listing, and the walk goes on.** The listing records the skipped paths (folder-relative, in walk order — `FolderListing.skipped`); the daemon log names the first one and the count.
2. **The launch reply names what was skipped** with the notice `folder-listing-skipped`, `args` `{folder, paths}`. Its sentence names the first three paths and counts the rest (「외 n개」); the wording is its own (ADR-0364). It does not block.
3. **The folder entry itself still blocks when it cannot be read** — at open or part-way through its own top level — and when it is missing: ADR-0341 decision 6 stands for the entry. Only folders and entries below it are skipped.

## 거부한 대안
- **Block the launch when any folder below a folder entry cannot be read** (main's revised recommendation, following ADR-0341 decision 6 read literally) — one unreadable subfolder of a large shared rules folder would stop every launch of every agent using it; the convention survey found no tool that stops (codex skill discovery, ripgrep, find, git, walkdir all skip and warn). User: 「ㅇㅇ 저희는 건너띄고 하자.」 약함 — 사용자 직접 기각.
- **Skip without telling the caller** (the generic walkers' default — a warning in their own output only) — the caller would not know that part of the folder was not read. User: 「ㅇㅇ 건너띄되 어떤게 건너띄웠는데 알림은 해줘야됨.」 약함 — 사용자 직접 기각.

## 근거
- User decision 2026-11-12, after the convention survey: 「ㅇㅇ 저희는 건너띄고 하자.」 + 「ㅇㅇ 건너띄되 어떤게 건너띄웠는데 알림은 해줘야됨.」
- Code: asset-library `folder.rs` `StdFolderOps::list` and its `walk` (a failure at the top fails the whole listing as `Unreadable`; a failure below is recorded in `skipped`), `launch.rs` (the `folder-listing-skipped` notice from `FolderListing.skipped`); agent `launch.rs` `FOLDER_LISTING_SKIPPED` and its wording; the P9 test `listing_notices_name_a_cut_and_skipped_entries_and_do_not_block`.

## 영향 / 불변식
- The top of a folder entry and the folders below it take different paths: a read failure at the top is ① (ADR-0341 decision 6), below it is a skip plus a notice. A change that turns a failure below the top into a listing failure brings back the whole-launch block this ADR removed.
- An entry whose type cannot be read counts toward the 10,000-entry listing cap, so the walk stays bounded.
- A skipped subfolder may hold files the backend would have loaded (an `AGENTS.md` lower down, a skill folder); the launch carries what was read, and the notice is the caller's only signal.
- Notices follow ADR-0341 decision 5 (once per agent; a changed set shows again).
