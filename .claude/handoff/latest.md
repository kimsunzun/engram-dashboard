# Handoff: S38 (internal text → English) merged to master — done; only small leftovers

## Read first

- S38 is finished and on master: merge `204bec50` (S38) + follow-up merge `2558d4da` (qa known-failure text, T-114). Other sessions can sync from master now (the user tells them; do not message other sessions).
- Cost was the user's main complaint this session. Rules the user set for multi-agent work: at most 2 workers at once (「다음에는 2개로 운영해봐」), work in chunks with feedback (「끊고 피드백하고」), no Codex check for translation, reviews reduced to targeted ones (「리뷰를 좀 줄여」). Workers must not spawn sub-agents/forks.
- Do not ask the user to stop using the keyboard during QA (「내가 작업하는데 키를 누르는걸 막으면 안되지」).

## One-line state · next first action

- State: nothing in progress. Master = `2558d4da` (pushed; its CI was started by the push and not watched). Topic branch `v0.5.0/chore/english-internal-text` = `042b480d`, pushed, fully merged.
- Next first action (suggestion only — wait for the user): pick from the leftovers below; none blocks anything.

## Leftovers (none started)

1. Two stale worktrees `I:/Engram/apps/s38-wt/u1`, `u2b` (+ scratch notes in `s38-wt/`): recycle failed (0x20 sharing violation, later 0x7C; two `node.exe` with `s38-wt` in their command line, PIDs 31928/38496 at the time, owner unknown — not killed). User command: `! bash ~/.claude/references/recycle.sh /i/Engram/apps/s38-wt && git -C /i/Engram/apps/engram-dashboard-wt2 worktree prune`. Unit branches `s38-u1`..`s38-u6`, `s38-u2a/b` kept (merged content).
2. T-114 (docs/tracking.md): platform test `escape_reads_up_when_nobody_holds_it` reads the live Esc key state — fix as its own topic.
3. `core.hooksPath` is an absolute path into the main checkout's `scripts/git-hooks`; the qa binding's mutation check compares the literal `scripts/git-hooks`, so mutation falls back to the isolated copy. Decide: set the relative path, or loosen the binding check (binding = instruction file → user approval).
4. U1–U2 comment pruning (user: 「A로 해보고 괜찮으면 B도 정리하자」) — not decided; it is expensive (~20k comment lines).
5. TRD references in comments: keep for now; decide removal when TRDs move to references (user).
6. Engram feedback rows committed, not pushed (rule): 52e360c (per-mode agent cap), e0db641 (code-conventions: prune existing comments), daaf878 (rm -f), 236e860 (reviewer forks past cap), df8bf06 (no rustfmt rule). 6d82f4e is from the previous session.

## Repo state (measured 2026-11-18 15:51)

- wt2: branch `v0.5.0/chore/english-internal-text`, HEAD `042b480d`, 0/0 vs origin, tracked tree clean (untracked = `.claude/handoff/**` attachments and an old root `bindings/` folder from 2026-10-20, not S38). This handoff's commit comes after this text and is not pushed.
- master `2558d4da` = origin/master.

## Verification

- QA (engram-qa, third run, tree `4b9bdad9`): full regression PASS (backend 5512 passed, gates 32/32 incl. `cargo fmt --check` and `Hangul check`, frontend 2928 + tsc); claude, codex, shell real-daemon lanes PASS (`lanes.tsv` updated to `4b9bdad9`); real-process Job lane = known failure `ignored_daemon_kill_cleans_pty_child` only; mutation PASS (isolated copy); GUI smoke, AC3 (logs English, 0 Hangul lines) and AC4 (band en / ko head + English cause / live switch) PASS. `escape_reads_up_when_nobody_holds_it` skipped by user decision.
- CI green on `4b9bdad9` and `042b480d`. Master CI for `204bec50` / `2558d4da` started, result not read.
- Not verified: master CI results; the qa binding change in `042b480d` was checked only by `launcherWiring.test.ts` (6 passed) + CI, not a local full regression.
- Known quality gap (accepted for cost): reviews were targeted; translation meaning/term slips in comments likely remain outside reviewed hunks (part-A full review found 25 in two files).

## Failed approaches (do not repeat)

- Sonnet coders told to "prune while translating" translated only; prune needed separate chunked passes with calibration (example block + "rewrite blocks line by line"). "When unsure, keep" made them far too timid.
- Running one worker through a whole unit grew contexts to 500–800k (U4 prune worker 56M cache reads). Hand over at ~400–500k via the progress file.
- Full Opus review of a 20k-line unit diff: split into parts or it overflows; targeted review (risky deleted comments + changed strings) cost ~1/3.
- `cargo fmt` after translation can split a line and move a trailing `// hangul-ok` marker off the Korean literal → run `check-hangul.mjs` after every fmt.
- My prune calibration told workers to cut `(TRD …)` tails, conflicting with the project binding (keep existing TRD refs, `FIX <n>` anchors) → 166 refs had to be restored. Check project bindings before writing a calibration.
- `git cherry-pick -n` leaves sequencer state; run `git cherry-pick --quit` afterwards.

## Resume command

`git -C I:/Engram/apps/engram-dashboard-wt2 log --oneline -3 master && git -C I:/Engram/apps/engram-dashboard-wt2 worktree list`

## Decisions

User decisions (this session):
- (묻지 않음) drop Codex check for translation · 상황: translation is internal text, no behaviour change · 선택지 없음 · 고른 것 「번역에서 구지 codex 확인은 필요없어보여」 · — · 근거: 「실제 동작에 영향이 가는건 아니니」.
- (묻지 않음) worker cap · 상황: 3 workers burned usage fast · 선택지 없음 · 고른 것 「다음에는 2개로 운영해봐」 · —.
- (묻지 않음) chunked prune with feedback · 상황: prune workers stopped early and were timid · 선택지 없음 · 고른 것 「한꺼번에 하지 말고 끊고 피드백하고 끊고 피드백하고 하면됨」 · —.
- TRD refs in comments · 상황: my calibration contradicted the binding, 168 refs removed in U4 · 선택지 keep ★ / remove · 고른 것 「나중에 trd에서 레퍼런스로 옮길때 결정하는게 좋으려나?」 → keep now · 같음.
- review reduction · 상황: full Opus reviews ~35M per part, ~10 left · 선택지 targeted sample review ★ / keep full · 고른 것 「ㅇㅇ 리뷰를 좀 줄여」 · 같음.
- U5 prune long tail · 상황: 76 small files left · 선택지 stop ★ / continue · 고른 것 「그냥 진행해 작은 파일들이라는거잖아」 · 갈림.
- Korean HTML reports · 상황: pending since last session · 고른 것 「html 보고서는 내가 보는거잖아. claude.md에서 룰 적어놨잖아」 → keep Korean · 같음.
- (묻지 않음) Esc test · 상황: QA asked user not to press keys · 고른 것 「esc는 안건들게 아니면 그냥 빼고 하던가」 → skip in QA · —.
- (묻지 않음) merge · 고른 것 「다 되면 알아서 머지해」 · —; worktree cleanup 「그거 안쓰면 지워」 · —.
- (묻지 않음) feedback rows · 고른 것 「멀티,서브 에이전트 상한을 둬야될것같음. 모드에 따라 다르게」, 「기존 주석 정리해야됨」 · —.

Claude decisions: do not restore ★ markers dropped by the U3 prune (rule text kept; cost); U3 daemon accepted at ~20–35% comment shrink (dense contracts); skip files with <15 comment lines in prune; blame-ignore gets docs commits + U3/U4/U5 + glossary commit, not U1/U2/U6/logging rule; S38 follow-ups (qa known-failure text, T-114) committed on the S38 branch and merged again because `v0.5.0/chore/misc` is checked out in another worktree.

## References

- `docs/process/S38-english-internal-text/` (README, prd, trd — Implemented), `docs/decisions/0383-*.md`.
- `.s38-review/` (git-excluded): `prune-brief.md`, `light-review-brief.md`, `review-fix-brief.md`, `u9-notes.md`, `new-terms.md`.
