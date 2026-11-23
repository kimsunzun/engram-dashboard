# Handoff: S25 step 21 done (U4b, U5, CI green); step 9b TRD reviewed + ADR-0408/0409 written — next: 9b coding from U1

## Know first
- The user wants short, plain answers: big picture first, the flow only, one question at a time, concrete examples (what triggers → what is left → what a retry / restart does) before options. No design-doc labels; say what happens. Feedback rows this session: Engram `77eff59` (failure-path questions need a concrete scenario), `ee34332` (engram-adr template/script fixed to Korean tokens).
- The session ran in code mode (REPL). Subagent cap = 5 running at once. Replace a worker past ~500k context.
- Codex relay calls move to the background after 180 s; the result is recovered from the run's output file (works). A Codex re-check via the same thread is unreliable — start a new Codex task with the earlier findings in the prompt.
- `src-tauri/bindings/*.ts` line-ending noise and untracked `.claude/explain/` are not ours — never stage.
- `core.hooksPath` points into another worktree, so QA mutation runs in an isolated copy (cold build).

## One-line state · next first action
Server stage: step 21 (library mounts) is **Implemented** and pushed (U4b `013eec57`, U5 + freeze `25e10274`, CI green on both). Step 9b (file operations, per-mount trash, emptying by direct removal) has a **reviewed TRD** (`docs/process/S25-preset-library/trd-file-operations.md`, draft 7, deep PASS) and its ADRs (ADR-0408 emptying = std::fs removal, no OS Recycle Bin; ADR-0409 the 9b rules). No 9b code yet.
**Next first action:** run `/engram-implement` for step 9b starting at U1 (platform `fs::rename_no_replace` + `fs::make_removable`, TRD §9 / §3.10). Write a fresh 9b coder brief (model on `.review-s25-d9/s21-code/coder-brief.md`; the TRD + ADR-0400 d23–25, 0405, 0408, 0409 are the design). Units are sequential: U1 → U2 (trash core, delete, restore, view, exclude, trash lock, Change hold with raise) → U3 (one-volume move/rename) → U4 (cross-volume move, copy) → U5 (emptying, permanent delete) → U6 (texts, references). Review strength: deep (data loss / file system). Before U3: a short manual comparison of the Explorer conflict rules on this Windows (TRD R6).
Then 10b (commands), then **22 hook components** (new parent-TRD item, user decision), then the server-stage acceptance stop.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `864fd26f`, 0 ahead / 0 behind origin at save time (this handoff commit will be 1 ahead until pushed).
- Working tree clean except `.claude/explain/` (not ours) and the `src-tauri/bindings` noise.
- This session's commits: ADR-0407 (`01a281cb`…`abc3e8d1`), link 0405→0407 `07bb2475`, U4b `013eec57`, T-122 closed `be81cbaf`, U5 + step-21 freeze `25e10274`, ADR-0408 (`a07528ac`, `93f6a596`, `0fdb4318`), 9b TRD drafts 3–7 (`87e59d66`…`f94540e1`), ADR-0409 (`34776fc6`, `afa6511d`), TRD link `864fd26f`.
- Engram core repo (`I:/Engram`): feedback `77eff59` (global-feedback) and `ee34332` (engram-adr feedback). Not pushed by main.

## Verification state
- Step 21 U4b: review deep rounds 1–6 (round 6 PASS, lead fresh) · QA full regression PASS on tree 5503d9d7 (workspace 6680, frontend 2884, gates 34/34, mutation 2/2) · CI green.
- Step 21 U5: review full (lead PASS, Codex FIX on help wording → fixed, main diff-checked) · QA PASS on tree 00c80aed: workspace 6681, frontend 2884, gates 34/34, claude lane 14/14 and codex lane 19/19 (incl. the new L6 scenes — the step-21 lanes deferred by user agreement), live scenes over the daemon bus (not the `engram` CLI layer — covered by cli tests only), GUI start from a non-default mount · CI green. Mutation n/a (no production line changed).
- Step 9b: documents only — nothing built or tested. TRD review deep (lead, data-loss axis, Codex) 4 rounds → PASS on `9cad4164` (draft 6) + lead batched in draft 7.
- Rerun: `cargo test -p engram-dashboard-asset-library` · `cargo test -p engram-dashboard-agent -- --test-threads=4` · `cargo test -p engram-dashboard-daemon -- --test-threads=4` · full QA per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- A stop answer carrying a digest of "what is left" (computed after the hold) — admits files edited after the lock (U4b round 5); replaced by ADR-0407.
- `edit()` with an anchor that matches twice fails atomically (nothing written) — pass `{all: true}` or a longer anchor.
- An interrupted `git commit` leaves `…/.git/worktrees/engram-dashboard-wt1/index.lock`: check no `git.exe` runs, then `bash ~/.claude/references/recycle.sh <lock>`.
- The ADR lint reads "amended by ADR-NNNN" anywhere in the 관련 line as a link — avoid that phrase in free text there.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings/ && gh run list --branch v0.5.0/feat/preset-library --limit 1`

## Decisions
User:
- 멈춘 이름 변경의 다시 실행 · 상황: 확인된 renameMount가 파일 쓰기 실패로 중간에 멈춤, 멈춘 응답의 남은-목록 지문이 잠금 밖에서 계산돼 안 본 파일을 고칠 수 있음(리뷰 5라운드 must) · (a) 멈춘 응답에 지문 없음, 다시 실행은 남은 목록을 다시 확인 ★ / (b) 지문을 잠금 안에서 계산 · 「ㅇㅇ 어쨋든 old new 폴더 바뀌는게 최종이니깐 아직 안된게 가시적으로 보이고 중지됐으면 다시 실행 가능하다는거잖아. 그거 자체가 히스토리니깐. 진행 ㄱㄱ」 · 같음 · 근거: 관례(Terraform stale plan, VS Code·IntelliJ 재실행) → ADR-0407.
- 멈춘 뒤 옛 토큰이 통과하는 드문 경우(T-122) · 상황: 쓴 파일을 전부 old로 되돌리면 남은 목록이 승인한 목록과 같아짐, 승인 범위 밖은 안 건드림 · 그대로 ★ / 멈추면 확인 번호 올림 · 「그대로 둬.」 · 같음 → ADR-0407 거부한 대안, T-122 closed.
- U5 리뷰가 남긴 작은 것들 · 상황: 기록만 한 4건 · 고칠지 물음 · 요지: 「고칠거 이해가 안가긴하는데」 → 기록만(T-121) · 미상.
- 다음 순서 · 상황: 남은 서버 = 9b·10b, 훅은 계획 밖 · 9b 먼저 ★ / 훅 먼저 · 「ㅇㅇ 그럼 9b 먼저해.」 · 같음.
- (묻지 않음) 훅 컴포넌트 추가 · 상황: 지금 claude 훅은 설정 원문 안에만, 스크립트는 라이브러리 밖 · 없음 · 「없는것보다는 있는게 좋겠지? 유저가 하드코딩하기 딱 좋은게 훅인데」 · — · 조건: 「기존 인프라에 그대로 끼고 우리쪽은 거치고 클로드 코덱스쪽으로 에러를 넘기는거라서 많이 수정은 안한다고 생각되는데 어쨋든 쭉 진행하면 될듯」 → parent TRD item 22 (after 10b).
- 휴지통 모델 · 상황: 사용자가 "이중 휴지통" 느낌, 관례는 한 겹(VS Code·Obsidian·Joplin·JetBrains), LLM 되살리기·공유 폴더 때문에 자기 휴지통 필요 · (b) 우리 .trash 한 겹, 비우기 = 진짜 삭제 ★ / (a) 두 겹 / (c) OS 휴지통만 · 「…그냥 자기 휴지통 아님? 어차피 os로 간게 뭐 특별히 하는게 아니고 휴지통 버리기잖아.」 then 「그냥 표준함수 한줄로 지우기해.」 · 같음 → ADR-0408.
- 9b UD1–UD3 · 상황: TRD 초안 사용자 결정 3건 · 추천대로(지운 단위 한 줄+기록 파일 / detachRefs만 지문 / " - Copy") ★ · 「ㅇㅇ 넘어가.」 · 같음 → ADR-0409.
- 9b UD4 옮길 때 참조 고침 확인 · 상황: 다른 마운트·저장소 프리셋을 통째로 다시 씀(주석 손실 — 모든 프리셋 편집에서 이미 받아들임, ADR-0358) · (a) 확인 없음 ★ / (b) renameMount처럼 확인 · 「ㅇㅇ 그냥 주석 없애. 애초에 주석을 쓰질 말든가. 사람이 쓸일은 없으니.」 · 같음 → ADR-0409.
- ADR 한글 칸 이름 · 상황: 스킬 템플릿·스크립트가 한국어 토큰 고정, 옛 파일명 338개 한글 · 앞으로 영어 / 그대로 · 「일단 그대로 두고 해당 영문은 피드백으로 올려.」 · — → engram-adr feedback `ee34332`; 옛 파일명은 그대로(추천).
Main (reported): re-review by the raiser only (a finding both raised → Claude only); U4b round-6 wording fixed by main; U5 help musts fixed by the coder and closed by main's diff check; 9b TRD fixes by the drafter, hash verification = hash while copying + racy-only re-hash in the hold (synthesis of Codex r2 and lead r3); T-121 U4b/U5 additions recorded; CLAUDE.md module line updated to mounts.
Open: none blocking.

## References
- 9b: `docs/process/S25-preset-library/trd-file-operations.md` (§3 rules, §8 tests, §9 units, §11 ADR list), ADR-0408, ADR-0409, ADR-0400 d23–25, step-21 TRD §3.6; parent TRD items 9b / 10b / 22.
- Briefs (git-excluded): `.review-s25-d9/s9b/` (TRD draft brief, review brief, round diffs, ADR draft), `.review-s25-d9/s21-code/` (step-21 coder brief, handovers, QA briefs).
- `docs/tracking.md` T-121 (step-21 recorded items), T-122 (closed).
