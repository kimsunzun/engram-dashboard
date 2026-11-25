# Handoff: S25 step 22 (hook components) — P3, P4 (+T-125, T-131), P5 committed locally; only the GUI check and integration remain

## Know first
- **Nothing pushed.** Branch is 13 commits ahead of origin (14 with this handoff). P3 (`3211b40d`), P4 (`1e52e5f2`), P5 (`2f383f4a`) are committed locally with "GUI check pending" in their messages — the user asked to run autonomously "until GUI" (「GUI 전까지는 자율모드로 쭉 진행해」) and to test everything together after P4/P5 (「p4 p5 끝나고 다 테스트해보고 정리하면 될듯」).
- **GUI check is the next gate** (P3 = codex transport row; scene H1 in TRD §5). The QA executor can run GUI 실측 itself (qa binding 「GUI 실측」 / qa-verify recipes) — the user does not have to watch; ask the user whether to run it now.
- TRD `trd-hook-components.md` is **not** marked Implemented yet (held until the GUI check passes); parent `trd.md` item 22 likewise.
- Base is behind origin/master (8 commits at last fetch) — merge master at integration (handbook `docs/handbook/branch-commit.md`).
- Not ours: `src-tauri/bindings/*.ts` line-ending noise, `.claude/explain/`.
- Codex weekly limit: unknown now; this session used ~8 codex review calls (xhigh/high).

## One-line state · next first action
Step 22 code is complete and gated except GUI: P3 (codex assembly + wiring, deep review + fix round, QA PASS minus GUI), P4 + T-125 + T-131 (deep review, 3 fix rounds, QA PASS — no GUI needed), P5 (help/docs, full review, QA PASS minus GUI). **Next:** report to the user (morning), ask the open items one at a time (list below), then run the GUI check (P3 transport + scene H1) via `/engram-qa`; on PASS: mark TRD Implemented (+ parent item 22, README), merge master, push, CI green, `--no-ff` integration per the handbook; then the end-of-server-stage CLI pass with the user; then the client stage (first slot = read-only structure viewer, T-135).

## Repo state (measured at save)
- Branch `v0.5.0/feat/preset-library`, HEAD `2f383f4a`, 13 ahead / 0 behind origin (this handoff commit makes 14). Uncommitted: none of ours (only the bindings noise and `.claude/explain/`).
- Commits this session: tracking decisions `8fb58025` `f0afd41c` `8d0d8651`; ADR-0423 `0390e559` `a86d6e39`; P3 `3211b40d`; ADR-0424/0425 `4597fd45` `173a1a9d` `bf0c981b` `78471fe7`; P4 `1e52e5f2`; P5 `2f383f4a`.
- Snapshot ref `refs/s25/p3b-wip` (old P3 WIP) can be deleted (`git update-ref -d refs/s25/p3b-wip`).

## Verification state
- P3: review deep (lead, risk = protocol/handshake, codex) → fix round 1 → all PASS. QA: workspace 7076, gates 34/34, frontend 2884, bindings no drift, codex lane incl. H-a..H-d PASS (0.171.0), claude lane PASS (2.1.330), mutation 3/3. **GUI not run.**
- P4/T-125/T-131: review deep (lead, risk = data integrity, codex) → 3 fix rounds → lead/risk PASS, codex PASS after a one-line test gate (closed by main diff check). QA PASS: workspace 7094, gates 34/34, both lanes, mutation 3 caught. **Unix branches compile-checked only (x86_64-unknown-linux-gnu), never run** (CI is Windows-only).
- P5: review full (lead PASS, codex FIX doc → fixed). QA: workspace 7095, both lanes, help/commands read literally, pin mutation caught. **GUI (scene H1) not run.**
- Rerun: `cargo test --workspace -- --test-threads=4`; full QA per `.claude/skill-bindings/qa.md`. Results: `.review-s25-22/p3-qa-result.md`, `p4-qa-result.md`, `p5-qa-result.md`.

## Open items for the user (ask one at a time, scene first, no jargon)
1. GUI check now? (QA runs it; codex transport + scene H1.)
2. T-136 (O7): codex hook verdicts of boot-restored agents are only logged at info (nothing written in release). Recommend warn.
3. T-137: (a) deleting a lone hooks.json now asks (recommend keep); (b) a SKILL.md arriving in a referenced hook folder turns it into a skill without asking (recommend record only).
4. Unfixed recommendations: P3 — silent hooks/list test wall-clock bound (cap + 3 s) may be tight on slow CI; handshake timeouts say "ms" vs turn timeouts "seconds"; lane H-a waits for roster removal not process exit (os error 32 seen once). P4 — keep-both clause in merge (~ops.rs 4534) has no guarding test; empty fresh plan answers skill-unconfirmed for a hook-only call (rare race); markers matched by exact name after a case-only rename (pre-existing); Unix code never run (suggest a Linux CI job for platform + asset-library).
5. ADR rejected alternatives marked 약함, never shown to the user: ADR-0421 (×2), ADR-0422 (×1), ADR-0423 ("ask hooks/list after history paging"), ADR-0424 ("a code per marker kind"), ADR-0425 ("list folders from the checked handle").
6. Records left by lanes (user's call to delete): codex threads in ~/.codex (ids in the QA result files), claude session records under ~/.claude/projects/.
- Still waiting from before: TRD O8 (detect codex review screen), T-109/T-134/T-135 direction items.

## Failed approaches (do-not)
- REPL: rg patterns containing backticks inside `$` templates get refused by the rm guard ("unterminated quote") — search file text in JS instead.
- Resuming a codex thread past 200k tokens is forbidden by the global rule — new call with previous findings + incremental diff (did so for P3 r2, P4 r3).
- `edit()` with leading-space-sensitive markdown list lines: ADR bodies use 3-space indents — copy exact text.

## Decisions
User:
- 다른 드라이브 이동 시 속성 복사·같은 이름 폴더 합치기(T-128, T-127) · 상황: USB·NAS로 옮기면 속성 설정 거부로 매번 실패, 원본은 남음 · A 내용만 옮기고 로그 ★ / B 지금처럼 실패 · 고른 것 「음? 뭔가 복잡하면 그냥 다 막아.」 · 갈림 · → kept strict/refused, archived.
- 둘 다 유지 합치기로 참조 스킬이 사라질 때(T-125) · 상황: SKILL.md가 SKILL (2).md가 되어 프리셋 참조가 깨짐, 묻지 않음 · 묻기 ★ / 그대로 · 「어쨋든 이대로 진행하던지」 · 같음 · → ADR-0424.
- 읽기전용 남은 항목 삭제(T-131) · 상황: Unix 쓰기금지 폴더·Windows 읽기전용 빈 폴더를 못 지움 · 지우게 고침 ★ / 그대로 · 「ㅇㅇ 읽기전용하고 관계없지. 그냥 지우면 될듯.」 · 같음 · → ADR-0425.
- (묻지 않음) 진행 방식 · 「그럼 GUI 전까지는 자율모드로 쭉 진행해.」 · 조건: 「p4 p5 끝나고 다 테스트해보고 정리하면 될듯.」 · 「다 끝나면 핸드오프해. 막히면 그냥 이세션에서 하는게 좋겠지?」
Main: P3b brief scope; hooks/list own 2 s cap (lead hold → main, ADR-0423); foreign = trusted AND enabled; one notice per why; P4+T-125+T-131 in one coder (shared ops.rs); lone hooks.json delete asks (held → T-137); "skill-unconfirmed wins" one code per call; detach filter judges where pairs put a folder; Unix chmod of unreadable folders via O_PATH + /proc on Linux, none elsewhere; shells in record-less bundles opened (single-row residue accepted); local commits with "GUI pending" instead of waiting (user's go-ahead); TRD Implemented held until GUI.
Open: list above.

## References
- TRD `docs/process/S25-preset-library/trd-hook-components.md`; ADR-0420…0425; tracking T-136, T-137.
- Step notes `.review-s25-22/` (git-excluded): `agents.md` (all agents, trees, verdicts), unit/fix briefs, handovers `handover-p3a/p3b/p4/p5.md`, review and QA results.
- Feedback filed this session: Engram `ad32101` (global: tracking items relayed in record terms), `92ae544` (engram-implement: tracking close rule).
