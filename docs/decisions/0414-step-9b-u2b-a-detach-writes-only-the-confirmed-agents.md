# ADR-0414: Step 9b U2b: a detach writes only the confirmed agents; restore reads the trash inside its hold; the delete digest covers the items; spelling and kind judged in the hold

- 상태: 확정 (2026-11-23, 근거: main decisions during step-9b U2b coding · code review deep (lead, risk axis, Codex) PASS 2026-11-23)
- 관련: Amends ADR-0409 (M15 restore reads of the trash) · Amends ADR-0339 (M101 the delete agents pass over the whole registry) · ADR-0409 (decision 2, UD2) · ADR-0400 (decision 18) · ADR-0405 (decision 1) · ADR-0365 · ADR-0412 · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` §3.3 · §3.7

## 맥락
Step-9b U2b built delete (keep / `detachRefs` with the UD2 digest), the trash view and restore. The code review found that a detach could change what the caller never confirmed: a preset inside a target that failed to move, and an agent created by `agent.new` between the in-hold judgement and the raise. The raise cannot move before the disk step: a late merge reads a moved change number as "the disk change is done". Restore also had to read `.trash` under its write-lock hold, which ADR-0409 M15 had placed behind `BoundedIo`.

## 결정
1. **A detach rewrites only what the in-hold judgement confirmed.** Preset files: those the in-hold plan listed that still name a path that went (a preset inside a target that stayed is never rewritten). Agents: the agents the in-hold plan listed, each written once with `ProfileEditPort::set_attachments` (the remaining order and `default_preset` kept), after the raise. An agent created meanwhile keeps its attachment, which then reads "not found". This replaces ADR-0339 M101's whole-registry removal for a delete; `remove_preset_paths` has no production caller. A store error part-way leaves the earlier agents written and the answer says which (no journal — ADR-0335).
2. **The raise stays after the disk step and the preset rewrites, before the agents** (ADR-0400 decision 18 unchanged).
3. **The delete digest covers the items** as well as the referencing preset files and agents (one line per target path key), so a digest answered for `[A]` does not confirm `[A, B]`.
4. **Kinds are judged inside the hold.** Whether a target is a lone `SKILL.md` (kind `SkillFile` in the catalog, never a folder spelled so) is read again from the hold's catalog before the references are judged.
5. **Spelling.** A delete acts on the catalog's spelling only when that spelling passes the no-follow link admission and the store's identity says it is the same disk entry as the given one; else the given spelling (admitted first, so a link is refused before any identity probe). A restore puts a row back under the record's (or the bundle file's) spelling.
6. **Restore reads `.trash` inside its hold through the store** (record, fallback rows once per call, stats), like every write's in-hold disk step; only the trash view goes through `BoundedIo` (`BoundedIo::run_all`). A share that stops answering during a restore holds the library write lock, as during a delete. This narrows ADR-0409 M15 to the view.
7. **The view names held (overlapping) mounts** `library-unavailable` and never reads them; a bundle whose check fails with an IO error is listed with the error, not hidden, and its restore answers `library-read-failed`.

## 거부한 대안
- **Raise before the in-hold judgement** (closes the `agent.new` window by refusing the insert) — a scan that reads the old disk after the early raise would merge after the disk change, the late-merge rule (ADR-0324 · ADR-0366) assumes a moved number means the change is on disk. (code: `gate.rs` late merge)
- **Keep the whole-registry removal** — removes attachments of agents the caller never saw, which UD2 rejected. (review: lead, risk, Codex)
- **A new port method taking agent ids** (one roster save, all-or-nothing) — an agent-crate change (trait, manager, registry, five test doubles) for deletes that rarely touch many agents. (code)
- **Restore's reads through `BoundedIo`** — the restore's own renames and folder creation must run in the hold and are unbounded anyway, and `BoundedIo` has no writes; bounding only the reads would not keep a stalled share off the write lock. (code; Codex re-judged its finding as record-only)

## 근거
- Code review deep of U2b, rounds 1–4 (lead, risk axis, Codex), PASS 2026-11-23; each item above has a test (partial-failure detach, agent inserted after the judgement, item lines in the digest, a SKILL.md restored before the hold, a case-variant delete through a parent turned link, a held mount in the view, a failed bundle check).

## 영향 / 불변식
- `ops.rs` (`delete`, `delete_in`, `detach_refs`, `same_entry_spelling`, `check_target`, `restore`, `put_back`), `trash.rs` (`trash_view`, `RecordRead::Failed`), agent `profile_edit.rs` (`remove_preset_paths` unused in production).
- Later units: a move's reference walk follows the same rule — rewrite only what the in-hold judgement saw; raise after the disk step.
- Accepted: an agent created during a detach keeps a dangling attachment; a restore on a stalled share holds the write lock.
