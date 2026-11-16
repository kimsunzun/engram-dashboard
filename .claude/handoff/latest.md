# Handoff: old priming path removed (ADR-0370, CI green); step-9a TRD draft 1 + new asset-library reference doc committed — next = 9a PRD+TRD review with the reference doc folded in

## Know first
- **New process rules are live** (skill-factory, user-approved; binding cherry-pick `7a16f5eb`): finished (`Implemented`) TRDs are not read as sources — later steps read code (wins) · ADRs · reference docs (`docs/reference/architecture.md`, `docs/reference/structure/*.md`, crate `lib.rs` headers) · `docs/tracking.md`. Decisions later work must keep (user picks, rejected options, "no change", policies) go to an ADR by the step's last commit (immediately if closed outside a step); decisions still open at a step's end go to `tracking.md`. Freeze = add one `Implemented` line; do not align the TRD body to the code.
- **User standards set this session** (all recorded as feedback; apply them now):
  - Chat summaries split 「판단 필요」 (user-visible behaviour / policy / scope change) from 「보고만」 (main's internal choices, one line each). While a question to the user is pending, reply with only that question — no other reports attached (user: 「물어봐야되는것만 얘기해 뭔 정보들이 이렇게 많아」).
  - Do not ask before starting each step; ask only 「이대로 리뷰로 갈까요?」 after a TRD and real user decisions.
  - Reference docs = overall outline and flow + contracts not visible from one file + file/symbol pointers. No code content (types, field lists, code tables, step-by-step algorithms).
  - An existing topic without a reference doc that a step relies on/changes heavily gets its doc BEFORE the step (own docs commit); each piece then updates it in its code commit.
- `src-tauri/bindings/*.ts` line-ending noise — never stage. Untracked `.claude/explain/` is not mine — leave it.
- Auto-mode classifier denied a QA pre-launch check bundle again (session-lock check + `lanes.tsv` read). Do not run a denied command for a worker; fill the gap with independent evidence and say so.
- Environment: `py` not `python`; for paths with backslashes use files or the Edit tool, not inline heredoc Python. Full-repo `grep -r` hits `target/` binaries and takes minutes — scope greps to `crates docs scripts`.

## One-line state · next first action
Old priming path removal is landed and pushed (`ca508879`, CI green). Step 9a: TRD draft 1 `docs/process/S25-preset-library/trd-library-edit.md` (written by a worker from main's decisions D1–D10; main read only the outline, §6 and the reference line — **not reviewed, not fully read by main**) and the new reference `docs/reference/structure/asset-library.md` (92 lines, pre-9a code) are committed and pushed. **Next: run the 9a PRD+TRD review** (`/engram-review trd`, full — Claude lead + codex), with `asset-library.md` in the same review (reviewers check it against code; main fixes doc findings and closes them by diff, no re-review — user 「ㅇㅇ 한번 넣어봐」). Before spawning, main reads the TRD in full once. The user already approved going to review with the reference doc folded in (「ㅇㅇ 한번 넣어봐」 answering main's 「이대로 리뷰로 갈까요?」 + fold-in proposal) — do not ask again.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `702463f9` (+ this handoff commit); pushed — 0 ahead / 0 behind origin at save time (this handoff commit will be ahead until pushed).
- This session's commits: `5ed12c2d` `2d5de8db` `50a03054` `91f25c33` (priming TRD drafts → reviewed), `486ee5b2` (step-8 doc corrections ②–⑤), `fa8ff843` `37a393f8` (ADR-0370), `72427fd0` `6ba485d4` (ADR-0371 C1 closed), `0d341965` (10a → ADR-0371 pointer), `ef31d041` (ADR-0316 impact note), `4e868aba` (ADR grounds), `7a16f5eb` (binding cherry-pick of master `54916d0c`), `ca508879` (priming removal code + docs), `563dbd91` (asset-library reference), `702463f9` (9a TRD draft 1).
- CI: `ca508879` success. Later commits docs-only.
- Engram repo (not pushed — feedback commits only): `8a7329b`, `05ba675`, `ed6bcbe`, `81937b0`.
- Uncommitted: only bindings noise and `.claude/explain/` (not mine).
- `.review-s25-d9/` (git-excluded) holds this session's briefs and diffs: `opr-*` (priming removal: scout facts, review/coder/QA briefs, diffs), `s9a-scout.md` (9a scout report), `s9a-trd-brief.md` (main's 9a decisions D1–D10).

## Verification state
- Priming removal (`ca508879`): TRD review full PASS; code review full — round 1 FIX, round 2 FIX (unbounded probe), round 3 FIX (`taskkill /T`), round 4 PASS. QA: workspace 6220 passed, gates 33/33, frontend 2807 + tsc, ts-rs no drift; claude + codex lanes PASS; mutations 2/2; GUI claude/codex × JSON/terminal answered; no `builtin\` → only `builtin-unavailable`, no priming WARN, no `*.settings.json`; `ENGRAM_PRIMING_FILE` ignored silently; roundtrip_smoke live B_SENT=true, SETUP-FAIL without priming / default preset, SKIPPED with claude off PATH. QA PARTIAL only for the classifier-denied pre-launch bundle; GUI ran anyway; main wrote `lanes.tsv` (claude 2.1.284, codex-cli 0.167.0, `7a16f5eb +uncommitted`).
- Not fixed (accepted): kill-path test proves cut-and-reaped, not tree-wide; installed claude exiting non-zero on `--version` → SKIPPED; `ProbeEnd::TimedOut.reaped` is read only by tests (likely a dead-code warning in the non-test build — unverified); `codex/mod.rs` ~172 long line; pre-existing stale comments: `control/mcp_server.rs` "two priming files", `control/mod.rs` ~114 "(codex·gemini …)", agent `types.rs` ~845 `accepts_mcp_config` field doc, `backend/claude/mod.rs` ~357 ADR-0106 note premise.
- 9a: nothing verified — TRD draft unreviewed; `asset-library.md` unreviewed (symbol existence checked by the writer's script).
- Rerun: `cargo test --workspace --locked -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- Attaching processing reports to a pending user question (user correction above).
- Asking per step before starting.
- Letting a reference doc carry code content (first `asset-library.md` was ~4,900 words; trimmed to 92 lines).
- Ending a timed-out child with `platform::process::kill_tree` (`taskkill /T`) — rejected by ADR-0291 「거부한 대안」 (g); use `platform::spawn` `TreeRoot` (ADR-0275).
- A coder's scripted edit with a wrong end marker deleted a test-module tail (restored from HEAD) — when checking a coder round, compare test-name sets against the previous snapshot.

## Decisions
User (2026-11-16):
- step-8 안 고친 권장 · 상황: test-only gaps, no behaviour impact · 선택지 고침 / 안 함 ★ · 고른 것 「그냥 문제없으면 수정하지마」 · 같음.
- 옛 priming 로더 · 상황: read the file every claude/codex spawn, extra WARN without builtin\ · 선택지 9a에서 같이 ★ / 지금 따로 / tracking · 고른 것 「기존건 지워」 (done now as its own task) · 갈림.
- 남은 QA CLI 세션 · 상황: ~12 sessions in ~/.claude, ~/.codex · 선택지 휴지통 ★ / 그대로 · 고른 것 요지: resume is per-folder anyway, claude auto-cleans, codex keeps; 「가이드로 하기가 애매」 → leave, no binding change · 갈림.
- TRD 요약 모양 · (묻지 않음) · 「실제로 물어봐야되는건 별도의 항목으로 두는거지? … 정보홍수 때문에」.
- priming 제거 범위 D2(설정 조각 포함) · 선택지 포함 ★ / priming만 · 고른 것 「ㅇㅇ 이대로 가면 되긴하는데」 · 같음.
- roundtrip_smoke 변형 D4 · 선택지 `--builtin-dir` ★ / tracked 파일 수정 / `--priming` 내부 복사 · 고른 것 「ㅇㅇ 별도로 복사해서해.」 · 같음.
- `ENGRAM_PRIMING_FILE` 경고 D5 · 선택지 경고 유지 ★ / 지움 · 고른 것 「음? v0.4.0 이전은 필요없는데? 그냥 지우면 될듯? 앞으로 나올일 없으면」 · 갈림 → ADR-0370.
- skill-factory 규칙 확정(바인딩 cherry-pick · step-8 문서 수정 · C1 ADR) · (묻지 않음) pasted message · 「ㅇㅇ 이어서 작업해」.
- 단계 시작 질문 · (묻지 않음) · 「스탭 하나하나당 물어보라고 되어있음?」 → do not ask.
- asset-library 레퍼런스 · 선택지 9a 마지막 커밋에서 생성 ★ / 나중에 따로 · 고른 것 「만들고 그거 기준으로 하면되는거아님? 결과적으로 좋은 방향으로 해.」 → created before 9a (`563dbd91`) · 갈림.
- 레퍼런스 내용 기준 · (묻지 않음) · 「구지 코드에 써져있는 내용은 안쓰고 전체 개략도? 전체적인 흐름만 볼 수 있게함녀 되는거 아님?」.
- 단독 레퍼런스 리뷰 · 선택지 9a TRD 리뷰에 포함, 문서 지적은 메인 diff 확인 ★ / 별도 doc full 리뷰 · 고른 것 「ㅇㅇ 한번 넣어봐. 개선해야될건 피드백 해보고.」 · 같음.
- 핸드오프 시점 · (묻지 않음) · 「핸드오프 하고 진행하는게 좋을듯? … 한 프로세스는 전부 체크해보게 되는거잖아.」
Main: priming-removal code-review holds closed as internal choices (probe timeout = SETUP-FAIL, probe after config gates, hermetic temp home in unit tests, single priming read via `builtin_priming_text`, drop the impossible codex guard, `LibraryHost` / `build_library_service` plain `pub`); D1 one ADR (0370) + ADR-0371 for C1; profile-edit port hand-over → 9a (not 10a); batched comment fixes delegated to a worker (flow says main — deviation, accepted); 9a draft decisions D1–D10 in `.review-s25-d9/s9a-trd-brief.md` (own TRD file; all command bodies incl. reads in 9a so 10a only declares; state files follow ADR-0291 base file rules; uncreatable first-start root → unset, nothing written, retry next start; setRoot = create → write library.json → change_root with restore on refusal; root generation checked inside the edit's lock take; rev = SHA-256 lowercase hex; T-106 P8 kept open as risk K1 — worker found the fix larger than one lock scope; 4 sequential units).
Open: 9a review; T-89 may be closable (L6 saw resume first-turn `true` on codex 0.167.0 — check its exact claim).

## Improvements
- Recorded (Engram, not pushed): `8a7329b` implement — TRD summary mixes user decisions with main's choices; `05ba675` global — pending question buried under other reports; `ed6bcbe` implement — reference docs for existing topics a step relies on + review of a standalone reference doc; `81937b0` implement — reference docs carry outline and cross-file contracts, not code content.
- To observe in the 9a review (user asked for feedback): how folding `asset-library.md` into the TRD review works; record frictions to engram-implement `feedback.md`.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -4 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## References
- 9a TRD draft: `docs/process/S25-preset-library/trd-library-edit.md`; main's decisions brief `.review-s25-d9/s9a-trd-brief.md`; scout `.review-s25-d9/s9a-scout.md`.
- Reference: `docs/reference/structure/asset-library.md` (new), `agent-backend.md`.
- Parent TRD `docs/process/S25-preset-library/trd.md` §5-2 item 9a (+ 「Handed over by step 8」), §1-4, §1-7.
- ADR-0370 (priming withdrawal), ADR-0371 (C1 closed); bindings `.claude/skill-bindings/implement.md`, `review.md`, `qa.md`.
