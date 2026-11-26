# ADR-0417: Step 9b U4b: copy - staged like a move across volumes, own folder judged once with a safe default, no merge

- 상태: 확정 (2026-11-24, 근거: main decisions during step-9b U4b coding · code review full (lead, Codex) PASS 2026-11-24)
- 관련: ADR-0416 · ADR-0415 · ADR-0409 (decisions 3, 12 and 14, M3, M14) · ADR-0400 (decision 20) · ADR-0365 · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` §3.6 · UD3

## 맥락
Step-9b U4b built copy (`LibraryService::copy_items`) on U4a's staging machinery. The TRD (§3.6) fixed the outline: every copy stages, no reference fix, ` - Copy` naming into the source's own folder (UD3), the built-in mount as a source. The review (four rounds) found that deciding "its own folder" by spelling, or by an identity that cannot be read, lets an `overwrite` copy trash the source itself, and that the decision must be tied to the generation the hold compares.

## 결정
1. **Every copy — one inside a mount too — stages as a move across volumes does** (ADR-0416's staging, reads and naming of stagings), without a volume probe, a source compare or a reference fix; the copy enters the catalog by a folder-scoped walk after the hold. The change number is raised as by every hold that applies something.
2. **A copy onto a same-name folder is refused `copy-merge-refused`** on every volume (TRD §3.1 merges) — open, the same decision as ADR-0416 decision 5 (`docs/tracking.md` T-127).
3. **"Its own folder" is judged once before the ticket and carried into the hold**: by path key, else the two folders' identities, else their canonical paths (by path key), else — nothing answering — taken as its own folder, so the copy takes a ` - Copy` name and never overwrites. The root generation the hold compares is read before this judgement, so a mount change after it fails the item `target-mount-changed` (staging kept and named). The hold does no disk IO on the source mount.
4. **A copy into its own folder takes the next free ` - Copy` / ` - Copy (n)` name whatever `onConflict` says** (UD3; Explorer).
5. **From the built-in mount** only its items (each copied under its file's name, e.g. `default.yaml`) and its virtual folder `engram/settings` (a folder holding its item's file); every built-in path is admitted from the built-in folder down without following before any stat; the destination folder is checked (missing or a file `not-found` naming it, a link `path-through-link`) before any copy.

## 거부한 대안
- **Own folder by spelling only** — another spelling of the same folder (8.3 short name, case) plus `overwrite` trashes the source and puts the copy in its place. (review lead round 1; 8.3 alias test)
- **Identity error read as "different folder"** — on a volume without file identity the same overwrite happens. (review Codex round 2; test with no identity and a failing canonical lookup)
- **Judge own folder inside the hold** — disk IO on the source mount under the write lock without judging its recorded state (ADR-0400 decision 6). (review lead round 2)
- **Judge before reading the generation** — a mount relocated in between leaves a stale answer the hold trusts. (review Codex round 3; test relocating between the judgement and the staging)

## 근거
- Code review full of U4b, rounds 1–4 (lead, Codex), PASS 2026-11-24; asset-library 866 passed / 2 ignored.

## 영향 / 불변식
- `ops.rs` (`copy_items`, `copyable`, `refuse_copying`, `into_own_folder`, `admit_builtin`, `destination_folder`, `copy_source` / `copy_from`, `copy_in`, `copy_staged`, `place_copy_named`, `copy_name`), `lib.rs` (`builtin_folder`).
- 10b maps `CopyAnswer` (`Copied { to, trashed }`) and the staging fields of ADR-0416.
- Accepted: with neither identity nor canonical path answering, a copy into a really different folder takes a ` - Copy` name even under `skip` / none.
