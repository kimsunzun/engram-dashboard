# Handoff: S25 step 8 landed (U1 launch waits · U2 launch scope · U3 real kits), C1 closed — next = step 9a start check; three report items still unanswered by the user

## Know first
- **New process (user 2026-11-15, skills applied):** engram-implement / engram-review TRD-lifecycle rules — TRD = decisions and boundaries; reference docs (`docs/reference/architecture.md` + `docs/reference/structure/<topic>.md`, crate `lib.rs` headers) fixed in the same commit as the code; freeze the TRD in the last piece's commit (`Implemented`), hand-overs once in the receiving step's TRD; step-log is frozen (do not write it); new topic numbers = max `S<n>` in folder names / commit titles + 1, claimed by committing the new folder README; PRD/TRD/structure HTML only on request. Brief: `I:\Engram\agents\skill-factory\doc\skill-briefs\implement-trd-lifecycle-results.md`. The user wants every rule friction recorded in engram-implement `feedback.md` as it happens (「사용해보고 쭉 피드백으로 올리고」).
- **TRD procedure lines written before the new rules lose to the flow** (step 8's TRD said "confirm §6 with the user before coding"; the flow says autonomous = announce and proceed, ask only unconfirmed user-visible behaviour). Recorded as feedback (Engram `fb9f541`).
- The auto-mode classifier blocked two QA steps this session ("Undermining Oversight" — a ci.yml step-name listing; the ts-rs drift snapshot). Do not run a denied command on a worker's behalf; fill the gap with independent evidence and say so (done for U2/U3, see Verification).
- `src-tauri/bindings/*.ts` line-ending noise — never stage. Untracked `.claude/explain/` is not mine (user's background session) — leave it.
- Environment: `py` not `python`; heredoc + backslashes in inline Python broke twice (bell char from `\a`, `\E`) — write scripts to the scratchpad or use the Edit tool for paths with backslashes.

## One-line state · next first action
Step 8 is done and pushed; CI green on U1/U2/U3. **Next: step 9a** (`S25: feat(asset-library,daemon): 루트 · 첫 시작 기본 루트 · 편집 · 새로 만들기 · 다시 연결` — parent TRD `docs/process/S25-preset-library/trd.md` item 9a). Run `/engram-implement` §0: decide whether the parent item is enough or 9a needs its own TRD file, and check its review state (parent TRD status still says "Draft — awaiting re-review (2026-10-31)"; no review-pass line for 9a found — unverified, check). Read the step-8 hand-over in the 9a item (`DaemonWiring.library`). Before that, ask the user the open report items below (one at a time — the user asked 「하나씩 물어봐」).

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `c4d521d2` (+ this handoff commit); upstream: `dfcdd4e8` pushed, `c4d521d2` (research note) not yet pushed at save time; vs origin/master 0 behind.
- This session's commits: `1aa13b94` `83f34159` (T-110), `c3dd6f8f` U1, `a3515170` U2, `4180d9d1` U3 (+ TRD freeze, README, T-111, hand-overs), `00c76321` ADR-0360 ← ADR-0369 link, `dfcdd4e8` ADR-0361 note, `c4d521d2` codex yield research + C1 decision. User's own `5b79b821` (process bindings) was pushed with them.
- Engram repo: `5a6bcb3` `1874c32` (engram-qa feedback), `fb9f541` (engram-implement feedback — step 8 vs 7/17 comparison + TRD-procedure friction).
- CI: `c3dd6f8f` green, `a3515170` green, `4180d9d1` green; later commits are docs-only.
- Uncommitted: only the bindings noise and `.claude/explain/` (not mine).
- `.review-s25-d9/` (git-excluded) holds the step-8 briefs, diffs and commit messages (`s8-u1-*`, `s8-u2-*`, `s8-u3-*`).

## Verification state
- U1: code review deep (Claude lead + Claude timer/seat lens + codex gpt-6.1-sol xhigh) PASS, one 권장 round; QA: workspace 6214, frontend 2807 + tsc, gates 33/33, claude lane, mutations 6/6, GUI release (two claude agents launched and answered). **Missed** the CI backend step `Resolve and assert pinned toolchain` — the U1 commit message overstates "full regression"; main ran the toolchain assert later (PASS); feedback `5a6bcb3`.
- U2: review full PASS (no must); QA PARTIAL → filled: workspace 6217, mutations 3/3, GUI (claude answered, codex launched); the ci.yml step cross-check was classifier-blocked → settled from U1's step list on unchanged ci.yml + main's toolchain assert.
- U3: review deep — round 1 codex FIX (L3 did not verify JSON priming delivery) + 3 권장 fixed, round 2 two 권장 fixed, final PASS from all; QA: workspace 6231, gates, toolchain assert, ci.yml compared step by step, claude + codex lanes incl. `launch_kits_live` L1/L2/L4/L3, mutations 8/8, GUI release 4 scenes (claude/codex × JSON/terminal) with launch folders created and removed, L10 PASS. ts-rs drift step classifier-blocked → not applicable (no changed file has a ts-rs export); CI ran it green.
- Not verified: L5 compaction survival, L7 `DISABLE_AUTOUPDATER` via settings env, codex terminal mode's tool/yield behaviour, the two codex config keys at runtime.
- Rerun: `cargo test --workspace --locked -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md` (now incl. `launch_kits_live`).

## Failed approaches (do-not)
- Asking the user to confirm every TRD §6 item when ADRs already decided them — user: 「일단 물어볼거 하나씩 물어봐」.
- Claiming codex's tool waits 10 s by default — wrong; the model chose 10 s, the JS `exec` default is 30 s; corrected in `docs/research/codex-tool-yield-limits-2026-11-16.md`.
- Inline `py - <<'EOF'` with Windows paths containing `\a` / `\E` — corrupted text; use files or Edit.

## Decisions
User (2026-11-15/16):
- 새 절차 적용 범위 · 상황: skills changed mid-topic, step 8 TRD already reviewed · 선택지 step 8부터 새 절차 ★ / 다음 주제부터 · 고른 것 「step 8부터 새 절차로 진행해」 · 같음 · 조건: 「이미 리뷰 중인 step 8 TRD(trd-daemon-wiring.md)는 그대로 이어서 하면 되고, 「이대로 리뷰로 갈까요?」는 다시 묻지 않아도 돼」 · 「TRD 리뷰 지적은 내가 정한 걸 바꾸거나 내 선택이 필요한 것만 나한테 올리고」 · 「step 8이 끝나면 step 7·17과 비교해서 … feedback.md에 적어」.
- 실행 대기 표시(ADR-0181 d5 미구현) · 상황: launch waits grow to 88 s, a row spinner exists from 0 s on the double-click path only · 선택지 A 나중(tracking) ★ / B step 8에 넣기 · 고른 것 「뭐가됐건 기다리면 뭔가 기다리는 사인이 있어야되는데 이건 관용으로 확인하고 맞추면 될듯?」 then 「일단 나중에 UI할때 함께하고」 · 같음 · → tracking T-110 (with the convention check).
- C1 codex 셸 도구 대기 · 상황: codex yields after the model's chosen time (shell tool capped 30 s, JS tool uncapped), the command keeps running, the model polls · 선택지 priming 한 줄 ★(초기) / 그대로 두기 / CLI 대기 줄이기 · 고른 것 「ㅇㅇ 이대로 닫아」 (no change) · 갈림 (main's final recommendation was "no change" after the user's 「구지 지시문에 하드코딩 해야됨? 공용룰같은 느낌인데?」) · 근거: 「공용룰같은 느낌」 · → `docs/research/codex-tool-yield-limits-2026-11-16.md`.
- 작업 지속 · (묻지 않음) 야간 자율 진행 · 선택지 없음 · 「나 자러 갈테니깐 일 진행 가능함?」.
Main (2026-11-15/16): strict decode of an unknown deadline-class word kept (ADR-0272 d6), pinned by a test (TRD B18); 권장 fixed vs not per the new rule (see Open); K11 corrected + T-111 filed (a stopping daemon releases its lock before launch bodies finish); TRD K12 = child R18 now reachable (accepted earlier by the user, B29/R18); hand-overs written to parent items 9a (`DaemonWiring.library`) and 10a (R4/R4b live GUI re-run); folder README created; QA PARTIALs filled by independent evidence instead of running the classifier-denied commands.

Open (ask the user, one at a time):
1. Unfixed 권장 — 「이 중 고칠 것이 있나요?」: U1 CLI e2e ordinary branch proves "did not wait", not "the 10 s window" (window pinned by unit tests); U2 per-launch-kind test omits create (covered by the ad-hoc spawn test) and the moot test repeats its neighbour's setup; U3 tripwire hand-copies the backend list, `launch_kits_live` uses a `dunce` dev-dependency where `StdFolderOps.canonical` would do.
2. QA finding: the old priming provider still reads `builtin\instructions\priming.md` on every claude spawn (content unused on real kits); with no `builtin\` it logs an extra misleading WARN beside the `builtin-unavailable` notice. Recorded in step-8 TRD §3; not in tracking — ask whether to track/fix.
3. Leftover real-CLI sessions from QA (claude ~5 under `~/.claude/projects/`, codex ~7 threads under `~/.codex`) — ids in the U3 commit message / QA report; deleting is the user's call.
Candidate: tracking T-89 (codex resume first-turn report) — L6 saw `true` on resume with codex 0.167.0; may be closable (check its exact claim first).

## Improvements
- Recorded: engram-qa `5a6bcb3` (qa-verify backend block misses the toolchain assert — U1 overstated), `1874c32` (mutation copy outside the repo misses `builtin\`); engram-implement `fb9f541` (step-8 comparison row 기록 + TRD-procedure-vs-flow row 미반영).
- Report only (not recorded): the classifier blocking QA read-only steps (environment, not an instruction).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -4 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## References
- Step 8 TRD (frozen): `docs/process/S25-preset-library/trd-daemon-wiring.md` — §8 B18, §10 K11/K12.
- Parent TRD `docs/process/S25-preset-library/trd.md` — items 9a, 10a (step-8 hand-overs); folder `README.md`.
- `docs/research/codex-tool-yield-limits-2026-11-16.md`; tracking T-110, T-111.
- Bindings: `.claude/skill-bindings/implement.md` (new rules), `qa.md` (lanes now include `launch_kits_live` and asset-library/builtin/daemon lib.rs triggers).
