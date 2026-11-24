# Handoff: S25 — step 9b done; step 10b U1 + U2 committed — next: 10b U3 (docs / headers), then the server-stage end

## Know first
- **Autonomous run on the user's word** (2026-11-24 「ㅇㅇ 쭉쭉 진행해. 자율모드로. UI 전까지하고 막히면 어쩔 수 없고.」 then 「ㅇㅇ 리뷰 구현 QA 까지 쭉 진행해.」 for 10b). Rule change asked by the user (「자율모드에서는 사용자 승인없이 진행 가능한걸로 변경해야될듯? 피드백 올려」) is filed as engram-implement feedback (Engram `24ce00c`), NOT applied to the rules yet — until it is, TRD approval and *Stop* points still require the user; this session had explicit go-ahead for 10b only.
- Codex via CLI (not the plugin): first review `codex exec --sandbox read-only --skip-git-repo-check -c model_reasoning_effort="xhigh" "<brief>" < /dev/null`; re-reviews `codex exec resume -c model_reasoning_effort="xhigh" -c sandbox_mode="read-only" --skip-git-repo-check <session id> "<brief>" < /dev/null` (id from the output's `session id:`), run from REPL `$` with a short `.timeout()`.
- **Claude reviewer / coder agent ids do not survive a session** (SendMessage → "No transcript found"); a new session spawns fresh workers and hands them the previous round's findings files. Codex threads DO resume by id.
- Step notes and briefs: 9b `.review-s25-d9/s9b-code/` (agents.md, unit briefs, round files, commit messages); 10b `.review-s25-10b/` (agents.md, u1/u2/u3-unit.md, review and QA briefs, results). Both folders are in `.git/info/exclude`.
- QA briefs: build from `.review-s25-d9/s9b-code/u3-qa-brief.md` pattern, BUT check the TRD §8 per-unit GUI on/off — the 10b U2 brief wrongly said GUI off and shell scene C2 had to be run as a supplement. **U3's TRD row runs CLI scene C1** on the qa 「GUI 실측」 isolated daemon (needs U1–U3).
- Not ours: `src-tauri/bindings/*.ts` line-ending noise, `.claude/explain/`.

## One-line state · next first action
9b (file operations) is **Implemented** (U3, U4a, U4b, U5 committed; TRD `trd-file-operations.md` marked; ADR-0415 – ADR-0418). 10b (commands) TRD reviewed deep PASS (`trd-file-commands.md`, ADR-0419); **U1** (six LLM commands) and **U2** (shell-only `library.emptyTrash`, gate, CLI change) committed and pushed after review + QA PASS.
**Next:** 10b **U3** — brief ready at `.review-s25-10b/u3-unit.md` (root help line, priming pointer, headers, reference files of the TRD's 「Reference files to fix」 row, the U1/U2 batched doc items, and the one-word `library.renameMount` stop-prefix fix in asset-library `host.rs` `rename_ok`). Spawn a fresh worker-senior coder with it (base HEAD `00ca7336`); review light/full (docs + one fix); QA with **CLI scene C1** (TRD §8). Then mark 10b TRD `Implemented` + parent `trd.md` item 10b status, then step 22 (hook components) needs its TRD — and the end-of-server-stage CLI pass with the user (parent TRD §5-3).

## Repo state (measured at save)
- Branch `v0.5.0/feat/preset-library`, HEAD `00ca7336`, 0 ahead / 0 behind origin (this handoff commit makes it 1 ahead).
- Uncommitted: none of ours (only `.claude/explain/` untracked and the bindings line-ending noise).

## Verification state
- Committed and gated: 9b U3 `ad6dbe00`, U4a `7468d90b`, U4b `b093beb6`, U5 `9f166061` (9b Implemented); 10b U1 (commit after `003c75fb`), U2 `00ca7336`. Each: code review (deep / full) PASS, QA full regression + claude/codex lanes + mutation PASS; U4a ran the two-volume test (C: ↔ I:); 10b U2 ran shell scene C2 on an isolated daemon + app.
- Not run yet: 10b CLI scene C1 (U3's QA), the server-stage end CLI pass. Clippy is not installed for the pinned toolchain (never run).
- Rerun: `cargo test --workspace -- --test-threads=4` · full QA per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- SendMessage to a previous session's agent id → `No transcript found for agent ID` — spawn fresh.
- A QA brief copied without checking the TRD's GUI line skipped a required live scene (10b U2 C2).
- Telling a coder a helper "already exists" without checking (10b/U4a no-follow open) — the coder had to add one; verify such premises first.

## Decisions
User:
- 둘 다 유지 이름 · 상황: TRD R17 「(2)」 · (2) ★ / - Copy · 「ㅇㅇ (2)로해」 · 같음.
- 합치기 뒤 스킬 폴더가 SKILL.md를 잃는 경우 · 상황: 참조가 「찾을 수 없음」 · 확인 받기 ★ / 그대로 / 막기 · 「그냥 넣어. 근데 llm이 인자 없이 호출하지는 않을것 같은데 어쨋든 ㄱㄱ」 · 같음 (ADR-0415 decision 2).
- U1 미결 권장(읽기 전용 폴더 쓰기 열기 거부 시 링크 재확인 생략) · 상황: 휴지통 비우기 때만, 드묾 · 그대로 ★ / 고치기 · 「ㅇㅇ 읽기전용 따지지 말고 그냥 지워」 · 같음 · 근거: read-only entries are removed without asking (main applied it also to read-only junctions — ADR-0418 decision 5).
- 10b TRD를 추천안으로 리뷰→구현→QA 진행 · 상황: 프로젝트 규칙상 TRD 승인 필요 · D7 (b) 거절하며 안내 ★ / (a) 숨김; D8 (b) 긴 대기 ★ / (a) 기본 대기 · 「ㅇㅇ 리뷰 구현 QA 까지 쭉 진행해.」 · 같음.
- (묻지 않음) 자율 모드 승인 규칙 · 「자율모드에서는 사용자 승인없이 진행 가능한걸로 변경해야될듯? 피드백 올려」 → filed, not applied.
- (묻지 않음) 핸드오프 시점 · 「아 그리고 다 끝나고 끊을 수 있는 시점 오면 핸드오프하자.」 · 「현재 어디까지 진행됐음? 끊기 좋은 타이밍에 핸드오프 하면 도리듯」.
Main: U4 split into U4a / U4b; U6 texts folded into the units' doc passes; read-only junction removed via std `remove_dir_all` (no attribute change); D7 kept by making the CLI post unlisted names (not re-asked — the user's choice unchanged); shell-side long wait handed to the client stage; ADR-0415 – ADR-0419.
Open (ask the user — all committed with the current behaviour as "decision pending", see `docs/tracking.md`): **T-125** keep-both merge that empties a referenced skill asks nothing (asked once, no answer); **T-127** merge across volumes / copy onto a same-name folder refused (Explorer merges); **T-128** strict time/permission copy (recommend best-effort); **T-131** Unix 0555 folder in a row and a read-only empty-folder row cannot be emptied. Recorded rare items: T-126, T-129, T-130, T-132, T-133 (overlapping repeat after the CLI's wait).
- Idea raised by the user (not filed): an operator / external-terminal token so a plain terminal Claude can drive the daemon via `engram` (today the CLI needs an engram-spawned agent's `ENGRAM_TOKEN`).

## References
- TRDs: `docs/process/S25-preset-library/trd-file-operations.md` (Implemented), `trd-file-commands.md` (reviewed; §8 C1, §9 U3, 「Reference files to fix」); parent `trd.md` §5-3 (10b item has 「Re-review needed」 — satisfied by the child TRD review; update its status at the U3 commit), item 22, server-stage end.
- ADR-0415 – ADR-0419; `docs/tracking.md` T-125 – T-133.
- Feedback filed this session (Engram): `4aa84f2` (workers across sessions), `680c4b4` (qa lanes.tsv / env-var test), `24ce00c` (autonomous approval rule).
