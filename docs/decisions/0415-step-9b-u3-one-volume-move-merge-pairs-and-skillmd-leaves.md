# ADR-0415: Step 9b U3: one-volume move - merge pairs and SKILL.md leaves, ordered mapping, the shared trash path

- 상태: 확정 (2026-11-24, 근거: user decision 2026-11-24 (decision 2) · main decisions during step-9b U3 coding · code review deep (lead, risk axis, Codex) PASS 2026-11-24)
- 관련: ADR-0409 (decisions 2, 4 and 10, M3, M7, M8) · ADR-0339 (decision 1, M108, M110) · ADR-0414 (decisions 1, 4 and 5) · ADR-0365 · ADR-0335 · ADR-0358 · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` §3.4 · §3.5

## 맥락
Step-9b U3 built move and rename on one volume (`move_items` · `rename`): one no-replace rename per item, same-name folders merging entry by entry, `onConflict`, an overwrite's displaced entry into the trash, the catalog and the references following in the same hold. The TRD left open what a merge's emptied source folder maps to, how a merge that takes a skill's `SKILL.md` is judged, and how several items of one call compose. The deep review (four rounds) found that a prefix mapping of an emptied folder re-points references to entries that never moved, that a plan made before the disk step can be outrun by earlier items of the same call, and that two displacements of one path collide on one trash row.

## 결정
1. **A merge's emptied source folder maps only itself.** It becomes an exact pair `S → D` (the key itself, never a prefix) only when S's `SKILL.md` became D's (moved there plainly or by overwrite); a keep-both `SKILL (2).md` makes no pair. A plain folder emptied by a merge maps nowhere. What the catalog still holds under S is removed, never re-keyed. The pair is pushed even when the empty-only removal of S then fails (the failure is a `left` row; the range is removed only when S went).
2. **A merge that would take a referenced skill folder's `SKILL.md` while the folder keeps an entry asks** (user decision 2026-11-24): the same `skill-unconfirmed` refusal, `yes` / `detachRefs` and digest as a lone `SKILL.md` moved out (ADR-0339 M110), planned from no-follow listings and stats at the call and again in the hold (only for an item the catalog holds a skill at or below). A planned leave is detached only when its folder is no skill after the disk step and did not become an exact pair's old path.
3. **The disk step never goes beyond the confirmed plan.** A merge places a skill folder's `SKILL.md` last; when an entry stayed behind (skipped, failed, invalid name, a sub-merge that kept something) and that leave was not in the hold's plan, the `SKILL.md` stays and the item's `left` names it `skill-unconfirmed` with `{path}` — unless it would stay anyway (a taken name under skip, a kind mismatch under overwrite), which answers its own reason. A new call plans it afresh.
4. **Pairs compose in move order.** References and agents are mapped through every pair in order, each pair applied to what the earlier ones produced, as the catalog re-keys them. The answer's mapping rows carry `exact` and are applied in order. An item whose source is, or lies within, a path an earlier item of the call displaced or whose whole pair produced it fails `not-found`.
5. **The skill re-read is narrowed to the skill at a pair's own new path** (TRD §3.4 says "or of a folder above it"): a skill's fields depend only on its own folder's name (`folder`, the name fallback), so skills strictly below a moved folder need no re-read. Every re-read admits the path first (no link — ADR-0365).
6. **One trash path for delete and move** (`Trashing`: `put` · `take_back` · `finish`): the trash lock and the bundle id at the first `put`, `.trash` ensured once per mount, a path already a root of every bundle of the mount goes into a new bundle (a second displacement of one path keeps both versions). A delete now publishes every mount's record after all its trash moves and retries a failed bundle with the next item of that mount. The relink-style reference walk (`walk_refs`) is shared by `detach_refs` and `follow_refs`.
7. **An overwrite's `.trash` is checked twice**: without following before the ticket, and again inside the hold before the call's first rename; unusable fails the item `trash-unusable` with nothing of it moved.
8. **Git trees are found per rewritten or skipped file's folder**, after admitting that folder (a link: no tree), so a nested repository is named and no probe follows a link.

## 거부한 대안
- **Prefix mapping `S → D` for an emptied merge folder** — re-points kept references to entries deleted from or never moved out of S onto D, and re-keys stale catalog items over D's. (review risk round 1; test `a_merged_skill_folder_maps_only_itself_and_its_stale_catalog_range_leaves`)
- **Trust the call-time / in-hold plan alone** — an earlier item of the same call, or an entry whose rename fails after `SKILL.md` moved, makes the disk step leave a referenced skill without its `SKILL.md` unasked, or turns a confirmed detach into dropping references to an intact skill. (review round 2, Codex and risk; tests for both shapes and the failing rename)
- **First matching pair only** — a later item that moves a path an earlier pair produced leaves references and agents on a path that is gone. (review risk round 2; test `references_and_agents_follow_every_pair_in_move_order`)
- **Re-read every skill at or below a moved folder** (the TRD's wording) — more reads under the write lock and more link surface for fields that cannot change. (code: the skill's fields)
- **One bundle per mount for every displacement** — the second `store.trash` of one path fails `library-write-failed` after the first item already answered done with its content in the trash. (review risk and Codex round 1)
- **Check `.trash` only at the first displacement** — a merge's free entries move before a broken `.trash` is found (TRD §3.5: refuse before any disk step). (review Codex rounds 1–2)

## 근거
- Code review deep of U3, rounds 1–4 (lead, risk axis, Codex), PASS 2026-11-24; asset-library 805 passed / 1 ignored, each decision above with a test and a mutation check by the coder.
- User decision 2026-11-24 for decision 2 (「그냥 넣어 … 어쨋든 ㄱㄱ」).

## 영향 / 불변식
- `ops.rs` (`relocate`, `move_in`, `merge`, `stays_anyway`, `plan_leaves` / `plan_merge`, `Pair` / `Moves`, `mapped`, `agent_moves`, `follow_moves`, `reread_file`, `Trashing`, `walk_refs`, `working_trees`, `delete_in`), `rename.rs` (`git_tops`, `GitTree`).
- Later units: U4 pushes verified cross-volume pairs through the same `Moves` / `follow_moves` / `follow_refs` and puts its originals and staging rows through `Trashing`; 10b carries the mapping's `exact` and order on the wire.
- Open (user decision pending, `docs/tracking.md`): a keep-both merge of a referenced skill into a same-name skill that empties the source asks nothing (`SKILL.md` becomes `SKILL (2).md`, the source folder goes).
- Accepted: the merge-leave plan and the `.trash` pre-check before the ticket go through the store, not `BoundedIo`; a `.trash` broken by another process during the hold itself is found at the displacement.
