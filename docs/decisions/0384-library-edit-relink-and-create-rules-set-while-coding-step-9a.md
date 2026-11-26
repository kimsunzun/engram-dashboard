# ADR-0384: Library edit, relink and create rules set while coding step 9a

- 상태: 확정 (2026-11-19, 근거: S25 step-9a U2–U4 code reviews (deep, Claude lead + risk lens + codex) and QA)
- 관련: ADR-0330 (relink) · ADR-0335 (presets first, then agents) · ADR-0338 (draft + save) · ADR-0339 (duplicates keep the first place) · ADR-0358 (whole-file rewrite; the writer keeps tags it cannot model) · ADR-0365 (link admission) · ADR-0366 (per-item writes under ticket and generation) · ADR-0377 (root generation inside the change) · ADR-0379 (refusal codes) · ADR-0380 (profile-edit port cell) · ADR-0381 (persist only after success) · step-9a TRD `docs/process/S25-preset-library/trd-library-edit.md` §3.3–§3.6 · parent TRD `docs/process/S25-preset-library/trd.md` §1-7 (`preset.check` answer) · Amends ADR-0380 (relink cell check before any write)

## 맥락
The step-9a TRD fixed the edit, agent-preset, relink and create bodies at the level of decisions and boundaries. Coding U2–U4 and the code reviews of each unit settled details the TRD left open or worded differently. Step 10a declares these bodies as commands and maps their answers; the rules below are not visible from the code alone as decisions, so 10a and later work must not undo them by accident.

## 결정
1. **Answers describe the committed state.** Every edit, agent-preset edit, relink and create builds its answer inside its own `change`, from what it just committed; folder stats and link probes run after the lock; nothing after the change can refuse. A refusal means nothing was applied.
2. **Every library edit takes an optional root generation** (passed to `change_at`), as do the agent-preset edits, relink and create.
3. **`preset.check` answer = `{folders, conflicts, problems, save_refuses}`** (parent TRD §1-7). A draft mismatch is a `Broken` problem row with `via: [preset]`, `args {path, list}` and no line; `save_refuses` is true iff any draft mismatch exists. Draft rows carry no line (never `line: 0`).
4. **`check` judges what `save` would write**: it keeps the line problems of every file entry the save keeps — the folder list when the draft omits `folders`, and an item's tag in `presets` / `settings` that the writer keeps from the original node (ADR-0358).
5. **`preset.add` and `preset.setBackend` share the partial answer `{applied, skipped: [{item, reason}], view}`.** A folder `add` skips an already-listed folder before judging it.
6. **Agent-preset edits on a backend with no settings check (e.g. the shell) are applied**; the view answers `takes_presets: false`. CLAUDE.md 「최소한만 막기」 — such an attach neither breaks a run nor opens a hole; launch ignores it.
7. **`detach` drops an attached entry that does not parse by its exact text**; such a spelling that matches no attached entry refuses the whole call `item-path-invalid` (so a repeated raw detach refuses once the entry is gone). `default-preset-detach` refuses only while the Default slot is on.
8. **Relink:**
   - the profile-edit cell is checked inside the change before any file is written whenever an agent attaches a pair, so `profile-edit-unavailable` is always a refusal with nothing applied (this amends ADR-0380 decision 3's 「`stopped: {code}` with no `path`」 for that case; a port store error is still `stopped` with no path, or the refusal when nothing was written);
   - `stopped` carries the refusal itself and `skipped` the full reason;
   - a reference is re-pointed only in the list `to` goes into; entries in other lists stay;
   - a re-point that would make a preset reach itself is skipped per file with `preset-cycle` `{path, target}`, as add and save refuse it;
   - a pair counts as `fixed` only after an actual re-point for it (in a preset file or the agent rewrite); an agent that attached the pair but was deleted by another request before the rewrite still counts — no reference to the old path is left either way (user, 2026-11-19);
   - part (b) runs only when the catalog holds neither an item nor a folder at `from`; each candidate passes link admission and is spelled as the catalog spells it;
   - an IO failure stops the walk (TRD rule kept); rerunning recovers the rest once the failure is cleared.
9. **Create:**
   - `LibraryStore::create_new` creates a file without ever replacing one (`AlreadyExists` ⇒ `name-exists`) and never removes a file by name after a failed write (it answers `library-write-failed` naming the leftover); `exists` is only a fast path that answers "present" when a stat succeeds;
   - the new item is built from the bytes written, classified as a scan would, with no read-back;
   - `newFolder` drops catalog items and folder entries at or below its path once `create_dir` has succeeded.

## 거부한 대안
- **A separate `refused` list in the check answer** (the first U2 shape) — the parent TRD (user-approved) puts mismatches in the problem list and asks for a folder list, and without `save_refuses` a caller could not tell a refusing save from kept problems. Basis: code review (U2 rounds 1–2).
- **Refuse agent-preset edits on a backend with no settings check** — blocks something that breaks nothing, against 「최소한만 막기」; launch already ignores such presets (TRD B13). Basis: code (`launch_check` → `NoopLaunchKits`).
- **`detach` accepts only spellings that parse** — an unparsable stored entry (it survives profile loading and shows as a broken row) could then never be removed by any command. Basis: code review (U3 round 1).
- **Create by `exists` then `write_atomic`** — the replacing rename, with its busy retry, can overwrite a file another program created in the window and lose its content. Basis: code (`base::file` `replace_with`), raised by all three reviewers.
- **Remove the partial file after a failed create write** — removal by name can delete a file another program put there in between. Basis: code review (U4 round 2).
- **Read the new file back to enter it into the catalog** — a read miss (e.g. a scanner holding the file) answered success with no catalog entry. Basis: code review (U4 round 1).
- **Re-point every list that names `from`** — with a stale catalog this writes a reference of the wrong kind into another list and reports it fixed. Basis: code review (U4 round 1).
- **Report `profile-edit-unavailable` as `stopped` after files were written** — a partial apply for a wiring bug that can be checked first; checking before any write keeps 「a refusal applied nothing」. Basis: code review (U4 round 1).
- **Treat an IO failure that never clears (a read-only file, a file over the read cap) as a per-file skip** — needs a new way to tell lasting IO errors from passing ones; the stop keeps agents from pointing ahead of preset files (ADR-0335 order). Basis: code review (U4 rounds 1–2).

## 근거
- U2 review: 3 rounds (round 3 PASS); U3: 2 rounds (round 2 PASS); U4: 3 rounds. QA (full regression, claude and codex lanes, mutation) PASS for U2 and U3; U4 lands with its own QA.
- Each rule has a guarding test in the asset-library `edit.rs` · `agent_edit.rs` · `relink.rs` · `create.rs` · `store.rs` tests.

## 영향 / 불변식
- Step 10a maps these answers as they are (`CheckAnswer`, the partial answer, `RelinkAnswer`, the agent-preset view); it must not rebuild an answer after the change or re-add a read-back.
- `LibraryStore` implementors, test fakes included, must make `create_new` exclusive and never replace.
- A change to the relink list filter, the `fixed` rule or the cycle skip needs this ADR amended.
