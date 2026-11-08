# ADR-0335: File operations = plan → execute → fix references, with order only and no progress record (Obsidian shape) — leftover temporary files are not cleaned up automatically either

- 상태: 제안 (2026-10-31, basis: user decisions G38 (2026-10-31 「일단 이건 이대로 두고 나중에 생각하자」) · G40 (2026-10-31 「남기고 알아서 치우라그래」) · P3 · P5 · G36 + main M4 · M26 · M36 · M56 (reported — the user was told 2026-10-31) · M65 (re-review r7 — reported) · research `docs/research/multi-file-rename-crash-safety-practice-2026-10-31.md` · `docs/research/atomic-write-temp-cleanup-practice-2026-10-31.md` · `docs/research/junction-recycle-bin-atomic-write-2026-10-31.md`) · 부분 폐기 by ADR-0339 (replaces Decision 5 and extends Decisions 1 and 6)
- 관련: ADR-0312 · ADR-0315 · ADR-0330 · ADR-0320 · ADR-0324 (edit ordering) · ADR-0336 · ADR-0265 Decision 4 · ADR-0291 (atomic write) · PRD R17 · R19 · TRD §1-4 flow ④ · `store.rs` · §1-11a finding 15 · §7 7 · T-74 · Amended by ADR-0339 (replaces Decision 5 and extends Decisions 1 and 6)

## 맥락
An in-app move or rename fixes one file together with the many presets and agents that point at it. What happens if it is interrupted midway?

## 결정
1. Move · rename · relink = plan → execute → fix references. Order = rename the file → fix the referencing presets one by one → agents. Each file write is atomic and **uses the already existing `base::file::write_atomic` as it is** (temporary `<이름>.tmp<pid>.<번호>` → sync → swap in — ADR-0265 Decision 4 · ADR-0291 · M56).
2. There is no progress record (journal). If interrupted midway, some references point to the old path and become 「찾을 수 없음」, and one relink fixes all references to that old path (R19 · ADR-0330).
3. **Leftover temporary files are not cleaned up automatically — neither at boot nor at scan** (G40). The library does not call `base::file::sweep_temps` (M56). The scan skips the base temporary-name shape — it does not appear in the list and is not a problem (G40 · M56). That decision calls the public base function `base::file::is_temp_name` (it tells by name alone) — we do not copy the shape (main 2026-10-31 — M65).
4. Explorer rules — only the overwritten file goes to `.trash` (P5). A move inside the root can go anywhere (ADR-0312). If names collide and `onConflict` is not given, only that entry fails and nothing is overwritten — the caller chooses (M26 · a name collision on restore has the same shape).
5. Delete does not fix references — place = identity (P3 · M4). Delete = `.trash` inside the root (ADR-0320).
6. Agent-side reference fixing = only the old → new mapping table is passed to the profile-edit port, and the manager fixes the matching attachments across the **current** whole registry while holding the registry lock once and returns the agents it changed. The plan does not pull the agent list in advance (the plan's registry read is only the `yes` decision). `agent.duplicate` also does the source read + insert within one hold, so the two queue — if the clone comes first, the copy is fixed; if the fix comes first, the copy carries the new path (M36 · re-review B3).
7. **A library write first checks that the parent folder exists** — if it does not, that entry is `NOT_FOUND`. `write_atomic` creates missing folders (ADR-0291), so we block it before that — a folder deleted from outside is not revived (G43) (main 2026-10-31 — M65).

## 거부한 대안
- **An operation record that writes down the intent + finishing it at boot** (the old `library-ops\` journal roll-forward) — user 「일단 이건 이대로 두고 나중에 생각하자」 (G38) · no such peer was found (research 2026-10-31).
- **Fixing references to the trash path on delete** (review #5) — user 「따로 처리를 해야됨?」 (2026-10-20 · P3). If restored, it resolves by path (P3 · M4).
- **A fixed identity per file (sidecar `.meta` · an id inside the file · NTFS file id) as a relink candidate** — user 「나중에 생각하자」 (G38 · T-74). Doing so reopens R19 「후보 제안 없음」 (G38).
- **Permanent deletion** — see the same alternative reason in ADR-0320 (「영구 삭제 + 센 확인 — 에이전트 호출엔 답할 사람이 없다」).
- **Outside cleanup + only name relink** (memo 68 — no in-app move or rename · user 2026-10-14) — the user reversed it on 2026-10-15 (memos 106 · 107 「이왕 하는 김에 다 해버려」 — put in in-app move, rename and delete). The recorded reason (memo 108): the reference is a path from the library root, so only Engram can fix references at the moment of a move or rename and report on deletion — without in-app moves, every reorganization relies on name relink. Place = edition 2 (P1 — main · delegated by the user 「알아서 해 순서적인 부분은」).
- **Sweeping leftover temporary files at boot** (a detail of G38 「Stray temp files swept at boot」) — it disagreed with the user's 2026-10-17 「데몬이 켜질 때 미리 훑지 않는다」 (TRD §1-11a finding 15) and the user removed cleanup itself with 「남기고 알아서 치우라그래」 (G40). In the sample, no one scans and cleans up at app start (research atomic-write-temp-cleanup).
- **Cleaning old temporary files at scan** (the second branch of the research — Syncthing 24 hours · git `tmp_obj_*` shape) — by the same user decision, we do not clean at scan either (G40 「not at boot, not at scan」).
- **A fixed name `<파일>.engram-tmp`** (the first shape of G40 — the next write of the same file overwrites it · the same model as VS Code `.vsctmp`) — it becomes a second rule beside the already existing `base::file::write_atomic` (M56 · research junction-recycle-bin). Cost: leftover debris is not overwritten by the next write (the pid differs) — dying mid-write is rare, so we accepted it (M56).
- **Pulling the agent list at plan time and fixing only that list** — a copy made by `agent.duplicate` in the meantime stays with the old path (M36 · re-review B3).

## 근거
- User decisions G38 · G40 · P3 · P5 · research (2026-10-31).
- Research atomic-write-temp-cleanup (light · no review): the two common branches = no cleanup (a fixed name so the next write overwrites it — VS Code · git `index.lock` · Rust crates) · lazily at scan (a recognizable prefix + an age threshold). Common ground = the same folder · an app marker in the name · the scanning side skips that name. On Windows, a named temporary file cannot be avoided (`O_TMPFILE` is Linux only).
- Research junction-recycle-bin (medium · codex review): `write_atomic` = certain (`file.rs:348`) · the shell settings save already uses it.

## 영향 / 불변식
- To revive a journal, reopen G38. The recovery path for an interrupted operation is the single relink.
- The common skeleton (plan → execute → fix references) is used only by move, rename and relink.
- Debris `<이름>.tmp<pid>.<번호>` may remain in the library folder — it is not seen in the list and deleting it is up to the user (G40). 「훑기가 base 임시 이름 꼴을 건너뛴다」 is an implementation premise (research pilot item).
- Durability is stated only up to temporary-file sync + a same-volume rename — directory sync on Windows is a no-op (research junction-recycle-bin).
- If the library is changed to call `sweep_temps`, reopen G40.
- A narrow gap remains where the folder is deleted between the parent check and the write — to close even that, add to base a shape that does not create folders (implementation judgment — TRD §1-4 `store.rs` · M65).
