# ADR-0405: Mount rename after step-21 U4b: yes pins the plan digest, unread references need confirmation, a lost mount stops and is recorded, the repeat must be the same rename

- 상태: 확정 (2026-11-23, 근거: user 2026-11-23 「ㅇㅇ A로 해.」 (yes pins what the caller saw — after conventions Terraform 「Saved plan is stale」, git `--force-with-lease`, LSP versioned edits) · 「그대로둬.」 (a stopped rename does not reserve `to`) · 「ㅇㅇ 쭉 진행해」 after agreeing B (unread references need confirmation) · step-21 U4b code review deep, rounds 1–4) · 부분 폐기 by ADR-0407 (decision 3 the repeat after a lost-mount stop asks before it finishes)
- 관련: Amends ADR-0400 (decisions 12 and 14 rename confirmation, unread files, stop rules and the rename repeat) · ADR-0402 (U4a amendments) · ADR-0339 (`yes` model) · ADR-0358 (whole-file rewrite) · ADR-0335 (relink stop rule) · ADR-0381 (repeat after a failed save) · ADR-0396 (decision 3 references in unreadable mounts stay broken) · step-21 TRD `docs/process/S25-preset-library/trd-library-mounts.md` · Amended by ADR-0407 (decision 3 the repeat after a lost-mount stop asks before it finishes)

## 맥락
Coding and reviewing step-21 unit U4b (`library.renameMount`) found that ADR-0400 decision 12's confirmation token, `root_generation`, moves only with mount changes: a file that came to reference the old name after the plan the caller saw was rewritten whole (comments and quoting lost, possibly in another git repository) without being shown. A rename whose only references sat in a mount that could not be read ran at once, leaving them broken with no chance to wait for the mount. A mount whose folder vanished during the rename was skipped as `not-found`, so the rename finished with references left old; and a mount lost since its last walk was never recorded, so the same call stopped again on every repeat. Decision 14's repeat rule accepted any failed save that left the old name in the saved list, so an unrelated removal could turn a rename into a rewrite towards the wrong mount.

## 결정
1. **`yes` pins what the caller saw (amends ADR-0400 decision 12).** A rename that needs confirmation answers `mount-rename-unconfirmed {plan, root_generation, plan_digest}`; `plan_digest` is a hash of the plan's sides — files to rewrite, files left unread, mounts never walked. The confirming call must carry `yes`, that `root_generation` and that `plan_digest`; if its own plan differs, nothing is applied and it answers `mount-rename-unconfirmed` with the fresh plan, and the same comparison runs again inside the rename's hold. A stale `root_generation` still answers `root-generation-stale` with a fresh plan.
2. **References left unread need the same confirmation (amends ADR-0400 decision 12).** A rename that would leave any reference un-rewritten — a referencing file in a mount that cannot be read, or a mount never walked this run, whose references are unknown — asks first even when nothing readable is rewritten. The caller (an AI) can wait for the mount and call again, or ask the user and confirm. A rename with nothing to rewrite and nothing left unread runs without confirmation.
3. **A mount lost during the rename stops it (amends ADR-0400 decision 12).** A file that reads as not found inside the hold makes the rename stat that file's mount folder once: gone → the walk stops before the agents and the registration (ADR-0335's rule), the old name stays; present → the file is skipped as deleted outside Engram. A lost mount is recorded unreadable — before the plan (the mounts that hold files to rewrite are stat'ed fresh) and after such a stop — so the repeat names it among the unread mounts and finishes.
4. **The rename repeat must be the same rename (amends ADR-0400 decision 14).** A rename of an unregistered `X` to a live `Y` counts as the repeat of a rename whose save failed only when the saved entry for `X` is at `Y`'s folder (a rename keeps its folder); otherwise `mount-not-found`. A rename to the name the mount already has (also a case-only repeat) saves the list again.
5. **A stopped rename does not reserve `to`.** Between a stop and the finishing call, `addMount to` is accepted; references already written resolve into that mount, and `addMount`'s `already_pointing` shows them.

## 거부한 대안
- **Keep `root_generation` as the only token (ADR-0400 decision 12 as written)** — a file edited or pulled outside Engram after the plan was rewritten whole without being shown. 사용자 직접 기각 (convention: Terraform, git `--force-with-lease`, LSP).
- **Lock the files between the plan and `yes`** — Engram cannot lock edits made outside it (editor, git pull), which is the main case, and a lock needs a timeout state. 약함 — 사용자에게 설명, 사용자는 A를 고름.
- **Run at once when only unread references remain, reporting the skips after** — the caller loses the chance to wait for the mount. 사용자 직접 기각.
- **Reserve `to` while a rename is stopped** — one more stored state for a rare case that `already_pointing` already reveals. 사용자 직접 기각.
- **Skip files of a mount that vanished during the hold** — the rename finished with references left old, and the repeat answered `mount-not-found`. Basis: review round 1 (risk axis).
- **Record nothing after a lost-mount stop** — every repeat stopped at the same file under the write lock. Basis: review round 2 (lead, risk axis must).
- **Accept any failed save naming `X` as the rename repeat** — `removeMount A` then `renameMount A→B` rewrote A's references into an unrelated `B`. Basis: review round 1 (risk axis).

## 근거
- User decisions 2026-11-23 quoted in 상태; conventions named there.
- U4b deep code review (lead, user-data-integrity axis, Codex), rounds 1–4; QA full regression PASS on the round-3 tree (re-run for this ADR's tree before the commit).

## 영향 / 불변식
- `rename.rs`: `confirmation_digest` (three sides), `RenamePlan::needs_confirmation`, `rename_in`'s in-hold digest comparison, `mount_gone`, the pre-plan observe, `Gate::record_unusable`; `config.rs` `unsaved_rename` returns the saved place.
- Wire: `LibraryRenameMountArgs.plan_digest`; `unread` entries `{mount, state, why, paths}` (`paths: null` = never walked); refusal args carry `plan_digest`.
- A held or never-walked mount makes every rename ask while it stays so.
- A change that lets a confirmed rename rewrite or leave unread a file outside the confirmed digest breaks decision 1.
