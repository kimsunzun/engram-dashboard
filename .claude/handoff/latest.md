# Handoff: S38 merged; next = prune comments in U1 (leaf crates) and U2 (agent crate), most important first

## Read first

- S38 is done and on master (`204bec50` S38 merge, `2558d4da` follow-up merge). The user decided to also prune the comments of the U1/U2 crates (translated in S38 but never pruned): 「그냥 나머지도 전체 진행하되 핸드오프 하는게 좋겠지? 중요한 순서대로 하면됨」. This was S38's declared follow-up (TRD §2: "Pruning U1–U2 comments is a follow-up if the U3–U5 result is good").
- Cost is the user's main concern. Binding rules from the user: at most 2 workers at once; work in chunks and give feedback between them; reviews are targeted, not full; no Codex check for comment-only work; workers never spawn sub-agents/forks (put that line in every brief). Do not ask the user to avoid the keyboard during QA.
- Estimate given to the user: ~28k comment lines (U1 ~11.8k + agent ~16.5k), expected −35~45%; about 45 on a scale where this session's whole S38 work (translation+prune+review+QA, worker cache reads ~330M) = 100. Not measured.
- Engram feedback rows of this session are recorded and committed (rule: no push) — nothing to do there.

## One-line state · next first action

- State: nothing running. wt2 on `v0.5.0/chore/english-internal-text` (merged). `core.hooksPath` is now the relative `scripts/git-hooks` (user-approved), so qa mutation runs in the work tree. All S38 worktrees removed.
- Next first action: open `docs/handbook/branch-commit.md` and cut a new branch from master for this follow-up (suggested `v0.5.0/chore/prune-comments-leaf-agent`, commit label `S38` as its declared follow-up — confirm against the handbook's numbering rule), then create worktree(s) with `git worktree add` (never Agent `isolation: "worktree"`) and start chunk 1 of the agent crate.

## Plan (most important first)

Order = most read / largest first:
1. `crates/engram-dashboard-agent` — 16,545 `//` comment lines (largest, read most). Watch: 37 ts-rs bindings in `crates/engram-dashboard-agent/bindings/` + `commands.schema.json` — `///` docs on ts-export types and on command declarations feed generated files and LLM-facing schema; leave those untouched (the U4 prune broke 11 bindings this way and had to restore them).
2. `crates/engram-dashboard-transport` — 3,520.
3. `crates/engram-dashboard-messaging` — 3,079.
4. `crates/engram-dashboard-platform` — 1,402.
5. `crates/engram-dashboard-protocol` — 1,058 (wire contract; keep contract docs).
6. `crates/engram-dashboard-command` — 1,039.
7. `crates/engram-dashboard-cli` — 1,037.
8. `crates/engram-dashboard-base` — 667.
Skip files with <15 comment lines. Build each crate's queue largest-first.

Per crate: Sonnet prune worker (`worker-scout-sonnet`) in chunks of ~12–15 files → main samples 2 before→after per file and gives feedback → continue; hand the worker over via its progress file at ~400–500k context. Then one targeted review (`worker-reviewer`, `.s38-review/light-review-brief.md`) per crate group. After every chunk: comment-stripped token compare + `cargo check -p <crate> --tests`; at the end `cargo fmt -p <crate>` then `node scripts/check-hangul.mjs` (fmt can move a trailing `hangul-ok` off its literal) and the crate's tests per `.claude/skill-bindings/qa.md` 「모듈별 명령」. Finally `/engram-qa` (Rust source row → full regression; lanes per the table) → push → CI green → `--no-ff` merge (handbook procedure) — the user authorised merging when done (「다 되면 알아서 머지해」, given for S38; re-confirm only if anything is off).

Briefs to reuse (git-excluded, in wt2 `.s38-review/`): `prune-brief.md` (has the calibrated "Standard": rewrite blocks line by line, cut origins/dates/review labels/restatements, keep rules/why/external facts/do-not-revive/contracts, ★ on load-bearing lines, KEEP `ADR-NNNN`, `TRD S<n> §…`, `FIX <n>`), `light-review-brief.md` (risky deleted comments with English+Korean keywords on word boundaries, changed strings; no forks). Update the brief headers for the new branch/worktree paths and add "run `cargo fmt -p` and report `--check`".

## Also pending (small)

- tracking T-114 (Esc-key test) says "fix"; user decided to leave the test as is (「esc 테스트는 그냥 놔두고, esc만 안누르면 되는거잖아」) → in this follow-up's branch, remove T-114 or change it to "kept as is — QA skips it if a key was pressed".

## Repo state (measured 2026-11-18 16:48)

- wt2: branch `v0.5.0/chore/english-internal-text`, HEAD `c9f0818e` (previous handoff commit, not pushed; 1 ahead of origin) — this handoff's commit comes after this text. Tracked tree clean. master `2558d4da` = origin/master.

## Verification

- S38 QA (tree `4b9bdad9`): full regression PASS, claude/codex/shell real-daemon lanes PASS, Job lane known failure only, mutation PASS, GUI smoke/AC3/AC4 PASS. CI green on `4b9bdad9` and `042b480d`. Master CI for `204bec50`/`2558d4da` not read.
- Nothing of the follow-up is started or verified.

## Failed approaches (do not repeat)

- "Prune while translating" got translation only; "when unsure, keep" made prune workers timid — use the calibrated Standard and an example block from the start.
- Long single workers grew to 500–800k context (one cost 56M cache reads) — hand over earlier.
- Cutting `(TRD …)` tails conflicted with the project binding → restored 166 refs. Read bindings before calibrating.
- Shortening `///` on ts-rs types changed generated bindings — leave them.
- A reviewer worker spawned forks past the cap — every brief says "no sub-agents or forks".
- `git cherry-pick -n` leaves sequencer state → `git cherry-pick --quit`.
- Orphan Codex broker processes (`app-server-broker.mjs … --cwd <worktree>`) can lock a worktree folder; the user had to `taskkill /F` them (auto mode refused it for main).

## Resume command

`git -C I:/Engram/apps/engram-dashboard-wt2 log --oneline -2 master && git -C I:/Engram/apps/engram-dashboard-wt2 worktree list`

## Decisions

User decisions (this part of the session):
- U1/U2 prune · 상황: ~28k comment lines, estimate ~45 of this session's work, Claude recommended not now or agent-only · 선택지 skip ★ / agent only / all · 고른 것 「그냥 나머지도 전체 진행하되 핸드오프 하는게 좋겠지? 중요한 순서대로 하면됨」 · 갈림 · 조건: 「중요한 순서대로」.
- (묻지 않음) Esc test · 고른 것 「esc 테스트는 그냥 놔두고, esc만 안누르면 되는거잖아」 · —.
- (묻지 않음) worktree cleanup and hooks path · 고른 것 「워크트리 정리는 그냥 하면 되는거 아님? git 훅경로도」 · — (done: worktrees recycled after the user killed two Codex brokers; hooksPath set relative).
- Earlier S38 decisions (worker cap 2, chunked feedback, reduced reviews, keep TRD refs for now, Korean HTML reports stay) — see `history/20261118-155133-s38-merged-to-master.md`.

Claude decisions: order = agent first, then transport, messaging, platform, protocol, command, cli, base (size × read frequency); skip files <15 comment lines; leave ts-rs/schema `///` docs; ★ not restored where already gone.

## References

- `history/20261118-155133-s38-merged-to-master.md` (full S38 close-out).
- wt2 `.s38-review/`: `prune-brief.md`, `light-review-brief.md`, `review-fix-brief.md`.
- `docs/handbook/branch-commit.md`, `.claude/skill-bindings/qa.md`, `~/.claude/skills/engram-code-conventions/references/conventions.md` §주석.
