# Handoff: S25 steps 17 · 18 landed; step 8 child TRD reviewed (deep, PASS) and committed — next = step 8 start question to the user (§6 user-visible list + plan), then coding U1 → U2 → U3

## Know first
- **New document rule (user 2026-11-13, first applied in step 8):** a TRD holds decisions and boundaries only; when the step lands, fix only the lines that differ from what shipped, mark "Implemented + commits", then it is frozen. No 「as landed」 rewrites, no copies into the parent TRD (link only); hand-overs once, in the receiving step; step-log one line; `docs/reference/*` is updated in the same commit as the change that makes it false. A fundamental design problem found mid-step = revise the whole TRD and re-review. Research: `docs/research/design-doc-lifecycle-after-implementation-2026-11-13.md`. Skill changes are recorded (not yet applied) in engram-implement `feedback.md` (Engram `e29b9ac`, `8482816` — hand-over spec for another agent).
- **Untracked file not mine:** `docs/research/living-reference-dual-audience-structure-2026-11-14.md` appeared during this session — presumably the user's background session (the S25-TRD → `docs/reference/` catch-up the user said they would run elsewhere). Do not touch, stage or delete it.
- `src-tauri/bindings/*.ts` (~45) line-ending noise — never stage.
- Environment: `python` is a no-op Windows Store stub — use `py`. Docs here are mixed LF/CRLF; edit with py scripts that preserve line endings. Heredocs with complex quoting failed once in Bash — write the py script to the scratchpad and run it.
- Claude subagents hit `API Error: 403 … authentication_failed` several times today; user: 「중간중간 끊기니깐 그냥 끊기는대로 그대로 이어서 하면됨」 — just retry (SendMessage, then a fresh agent).
- User's review-fix criterion still applies: a finding is fixed only if the fix adds no conditions/states/branches; otherwise record it. Explain user-visible questions with a concrete example first (the user asked 「뭔소리야? 무슨초?」 when C2 was framed in internal terms).

## One-line state · next first action
Step 8 TRD `docs/process/S25-preset-library/trd-daemon-wiring.md` (draft 5) passed PRD+TRD review and is committed (`d18a2dc3`). **Next: `/engram-implement` §1 start question — show the user §6 (10 user-visible changes, each with its example) and the plan (U1 launch waits → U2 launch scope → U3 real kits; one coder per unit in sequence; code review per unit — U1/U3 deep (timeouts, lifetime, new daemon dependency), U2 at least full; QA full regression + claude/codex lanes for U3 + real CLI lane checks §5), get confirmation, then code.** The coder brief must point at the TRD sections, not copy them.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `d18a2dc3` (+ this handoff commit); upstream 0/0 before this commit; vs origin/master 0 behind / 231 ahead.
- This session's commits: `4caa7701` `a72508ed` `fc57c999` (ADR-0368), `06e65d0e` step 17, `887459fd` step 18, `2734644c` step-17 doc pass, `c1e4b532` design-doc lifecycle research + decision, `f7a5dc67` `cb6f40bf` (ADR-0369), `d18a2dc3` step 8 child TRD + timeout research. Engram repo: `f20e020` (engram-review feedback), `e29b9ac` `8482816` (engram-implement feedback).
- CI: `06e65d0e` green, `2734644c` green; later commits are docs-only (path filter excludes `docs/**`) — no CI run expected.
- Uncommitted: only the bindings noise and the untracked research file above (not mine).
- `.review-s25-d9/` (git-excluded) holds this session's briefs: `s17-*`, `s17d-*`, `s8-trd-brief.md`, `s8-trd-review-*.md`, `s8-trd-draft3.md` / `draft4.md`, `s8-trd-r3.diff` / `r4.diff`.

## Verification state
- Step 17 (`06e65d0e`): code review full (Claude + codex gpt-6.1-sol high) 2 rounds PASS; QA PASS on the commit tree — workspace 6189, frontend 2807 + tsc, gates 33/33, claude lane 10/10, codex lane 16/16, release assembly harness (real script, build steps stubbed) OK + 2 injected faults caught, mutation 5/5. **Not verified:** a real claude spawn receiving the moved priming path — carried to step 8 §5.
- Step 18 + step-17 doc pass: `/engram-review doc` full 2 rounds PASS (one Claude reviewer needed 4 attempts because of 403s); comment-only check 0/0.
- Step 8 TRD: review deep (Claude lead + Claude wait-chain lens + codex) — round 1 FIX (2 must) + hold C2; rounds 2–3 one must each (webview `SpawnByCwd` via `forward_to_daemon`; codex's relay open-wait must withdrawn on the B17 evidence); final all PASS. No code exists yet for step 8.
- Rerun: `cargo test --workspace --locked -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- `python - <<EOF` silently did nothing (Windows stub printed "Python") — main reported a fix that had not been applied; a reviewer caught it. Use `py` and check the file after editing.
- Main looked for `scripts/cite-check.mjs` in the repo; it lives in the engram-review skill folder (`<skill>/scripts/`). Recorded as feedback (`f20e020`).
- Bundling "freeze vs maintain" with "copy vs link" in one option table — codex refuted it; keep them as separate choices.

## Decisions
User (2026-11-13/14):
- CLAUDE.md asset-library line · 상황: step 18 needed user confirmation (M1), the exact line was shown · 선택지 add the shown line / 안 함 · 고른 것 「음? 진행하면 되는거 아냐?」 then 「일단 쭉 진행하고」 · 같음 → committed in `887459fd`.
- 문서 수명 · 상황: TRDs carried design + current behaviour + diary, the doc pass cost as much as the code review · 선택지 A reconcile-then-freeze ★ / B freeze at approval / C living TRD · 고른 것 「생각해보니 그게 맞는것 같은데?」 · 같음 · 조건: 「어쨋든 왠만하면 안고치는데 근본적으로 문제있으면 trd 자체를 전체적으로 한번 갱신한다는거아냐」 · 「일단 이거 피드백으로 올리고」 · 「피드백에 잘 남겨봐. 다른 에이전트가 인계해서 스킬 수정하게.」 → research report status, feedback rows.
- 기존 TRD → reference 따라잡기 · (묻지 않음) 시점 · 선택지 없음 · 요지: the user will run it later in a background session (「다른곳에 백그라운드로 할건데」).
- 셸 실행 대기 (C2) · 상황: a launch may take up to 85 s, the shell's one shared timeout is 30 s, ADR-0181 recorded the user's one-shared-value choice · 선택지 A all requests 88 s / B launch-only deadline ★ · 고른 것 「B를 하면 될것같은데」 · 같음 · 조건: 「아니 연속성을 하려면 기준점 + 추가시간? 그런식으로 되야되지 않냐는거지 관행이 이게 아니면 그냥 하나하나 계산해서 하고」 · 「어쨋든 ㄱㄱ」 → ADR-0369 (derived chain: base `LAUNCH_REPLY_BOUND_SECS` + margin 3 s → 88 / 91 / 94; others stay 30 s).
Main (2026-11-13/14): builtin lookup on the install-root anchor + priming moved into `builtin\` (ADR-0368 — internal, reported); manifest guard tests; B14 (shell classification of launch-class socket envelopes) deferred — no sender, user's no-extra-branch rule; shell restates base/margin pinned by test (ADR-0360 d6) instead of a wire field; B16 class read via `CommandRoster::class_of` before `open`; B17 open wait 30 s then reply W1; B15 CLI reads the class from the catalog (static list removed); K9 ("shell gave up, launch still finished" → CONFLICT) recorded, not mapped.
Open: none blocking. TRD §8 (c) C1 (codex shell-tool timeout < 94 s → one priming line) is conditional on a measurement in §5.

## Improvements
- Recorded: engram-review `feedback.md` (`f20e020`) — cite-check/row-diff path; engram-implement `feedback.md` (`e29b9ac`, `8482816`) — TRD lifecycle rules with exact places to change.
- Not recorded (report only): the step-8 TRD author suggested listing `LibraryValues` / `LibraryService::new` in full in briefs; the doc-fix worker suggested saying what to do with multi-name diagram rows.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings`

## References
- Step 8 TRD `docs/process/S25-preset-library/trd-daemon-wiring.md` — §6 (user-visible list), §7 (units), §8 (decisions B1–B17, C1–C2), §9 (tests), §10 (risks K1–K11).
- ADR-0369 (launch waits), ADR-0368 (builtin lookup), ADR-0360, ADR-0181, ADR-0155.
- Research: `docs/research/long-request-timeout-conventions-2026-11-14.md`, `docs/research/design-doc-lifecycle-after-implementation-2026-11-13.md`.
- Parent TRD `docs/process/S25-preset-library/trd.md` step 8 row (links the child TRD); child TRD `trd-asset-library.md` §12 step-8 entries.
