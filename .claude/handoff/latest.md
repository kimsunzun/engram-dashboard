# Handoff: S25 step 9b — U1, U2a, U2b committed; U3 fixed after review round 1 (uncommitted) — next: U3 re-review

## Know first
- **Codex is called through the CLI now, not the plugin** (user 2026-11-24 「ㅇㅇ 지시서를 cli 로 되돌리면 되긴하는데 skill-factory 에 내용 전달하고 너는 하던거 계속하면 될듯」). The instruction files still say plugin until skill-factory changes them (intake Engram `0dce62c`, `agents/skill-factory/doc/insights/2026-11-24-codex-cli-return-intake.md` — commands there). First review = `codex exec --sandbox read-only --skip-git-repo-check -c model_reasoning_effort="xhigh" "<brief>" < /dev/null`; re-reviews = `codex exec resume -c model_reasoning_effort="xhigh" -c sandbox_mode="read-only" --skip-git-repo-check <session id> "<brief>" < /dev/null` (session id from the output's `session id:` line). Run it from REPL `$` with a short `.timeout()` so it goes to the background; a completion notification arrives. A plugin-made thread cannot be resumed by the CLI (`already has an active writer`) — U3's round-1 Codex review ran through the plugin, so the U3 re-review is a fresh CLI thread; resume from round 3 on.
- The user wants Codex re-reviews continued in one thread, not a new one each round (user 「토큰폭팔」, 「몇번째 얘기하는데」). A handoff note must not override that.
- Ask with a concrete example first; never point the user at "above" — restate the question in full (user 2026-11-24 「내가 위에라고 하면 다 뒤져서 찾아야됨?」).
- Main's step notes: `.review-s25-d9/s9b-code/agents.md`; briefs, round files, diffs in the same folder. Replace a coder past ~500k (two U3 coders so far). Code mode (REPL).
- Not ours: `src-tauri/bindings/*.ts` line-ending noise, `.claude/explain/`.

## One-line state · next first action
9b: U1 `f5a8c4fc`, U2a `c4e0f5b8`, U2b `5d71e2bf` committed and pushed (deep review + QA PASS each; ADR-0412, ADR-0414). U3 (one-volume move / rename) built, deep-reviewed (round 1: all three FIX), and fixed by a fresh coder (13 items incl. the user's item 13) — **uncommitted, re-review not done**.
**Next:** U3 round-2 re-review on snapshot `71a96023c3670fd00c0f2b6c1205f81fdf925543` (incremental diff `.review-s25-d9/s9b-code/u3-r2.diff` from `6bcc407e`; fix list `u3-round1.md` items 1–13; coder report summarized in `handover-u3.md` 「Round 1」): lead (`ab466ac38068ab4ca`) checks 1,2,3,6,9,11,12 and the shared-code refactor (delete must stay identical); risk (`aff5ef38093ebb76d`) checks 3,4,5,7,8,13 (last lens round); Codex (fresh CLI thread) checks 1,2,6,7,10. Then batched wording (ops.rs anchor ~:5064, `asset-library.md` 「Folder IO is bounded」), ADR for U3 decisions (below), QA (both lanes — asset-library path), commit. Then U4 (cross-volume move, copy), U5 (emptying), U6 (texts). After 9b: 10b commands → 22 hook components → server-stage stop → client UI.

## Repo state (measured at save)
- Branch v0.5.0/feat/preset-library, HEAD `5d71e2bf`, 0 ahead / 0 behind origin (this handoff commit makes it 1 ahead).
- Uncommitted (U3): `crates/engram-dashboard-asset-library/src/{catalog,lib,mounts,ops,rename}.rs`, `docs/reference/structure/asset-library.md`.

## Verification state
- U3: **gate not passed** (round-2 re-review and QA not run) — do not trust or commit this diff yet. Coder ran: asset-library 793 passed / 1 ignored (+13, 0 lost), agent and daemon green, workspace build, fmt clean; spot mutations on items 1, 2, 5, 7, 13.
- Coder notes for the re-review: `reclassify` lists its folder without a link check (shared with delete, unchanged); a destination whose last name is a link answers `path-through-link` instead of `onConflict`; a keep-both merge that renames `SKILL.md` to `SKILL (2).md` and then empties the folder removes the skill without asking (outside item 13 as written); delete now publishes records after all trash moves and retries a failed bundle per item; log wording changed.
- Rerun: `cargo test -p engram-dashboard-asset-library` · agent / daemon `-- --test-threads=4` · full QA per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- Codex plugin `--resume` cannot pick a thread by id (upstream #230/#700, PRs #475/#719 open); `codex exec resume <id>` on a plugin thread fails: `thread-store conflict: thread … already has an active writer`.
- Codex retried right at its reset time still answered `You've hit your usage limit … try again at 12:40 AM` — wait a few minutes past the reset.
- QA's `cargo test --workspace` stops at the first failing binary; add a `--no-fail-fast` run when it stops early (U2b QA did).

## Decisions
User:
- 둘 다 유지 이름 · 상황: TRD R17 「(2)」, 이 PC 탐색기 엔진 플래그는 " - Copy" · (2) ★ / - Copy · 「ㅇㅇ (2)로해」 · 같음.
- Codex 호출 방식 · 상황: 플러그인은 ID resume 불가(#230), CLI resume 실측 성공, 2026-09-16 플러그인 전환 이유(가시성·알림)는 REPL에서 해소 · CLI로 되돌림 ★ / 플러그인 유지 · 「ㅇㅇ 지시서를 cli 로 되돌리면 되긴하는데 skill-factory 에 내용 전달하고 너는 하던거 계속하면 될듯」 · 같음 · 조건: 지시서 수정은 skill-factory.
- 합치기 뒤 스킬 폴더가 SKILL.md를 잃는 경우 · 상황: 참조가 「찾을 수 없음」이 됨, SKILL.md 단독 이동은 이미 확인함(ADR-0339 M110) · 확인 받기(A) ★ / 그대로(B) / 막기(C) · 「그냥 넣어. 근데 llm이 인자 없이 호출하지는 않을것 같은데 어쨋든 ㄱㄱ」 · 같음.
- (묻지 않음) 핸드오프 시점 · 「핸드오프 지점 됐는데 적당할때 핸드오프 진행하자.」
Main: U2 split; T-123, T-124; ADR-0412, ADR-0414 (detach writes only confirmed presets/agents; restore reads in hold; digest covers items; spelling/kind in hold); U3 keep-both naming in one function; U3 fixes as in `u3-round1.md`. **ADR to write at the U3 commit:** item 13 (merge → skill-unconfirmed), merge pair rules (exact S→D only when S's SKILL.md became D's), skill re-read narrowed to the pair's own path (TRD §3.4 difference), second displacement → second bundle, `Trashing` / `walk_refs` shared by delete and move.
Open (ask the user): U1 unfixed 권장 — a folder write-open denial in `make_removable` skips the link re-check (rare; few lines to fix).

## References
- TRD `docs/process/S25-preset-library/trd-file-operations.md` (§3.5, §3.6 for U4, §3.8 for U5, §8, §9); ADR-0409, 0412, 0414, 0339, 0335, 0400 d23–25.
- `.review-s25-d9/s9b-code/`: `coder-brief.md`, `u3-unit.md`, `u3-round1.md`, `u3-{lead,risk,codex}-r1.md`, `handover-u1/u2a/u2b/u3.md`, `u3-explorer-probe-result.md`, commit messages `u2a-/u2b-commit-msg.txt` (pattern).
