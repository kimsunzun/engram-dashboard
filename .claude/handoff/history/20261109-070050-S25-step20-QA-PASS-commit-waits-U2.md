# Handoff: S25 step 20 — P1+P2+P3 coded, reviewed (all PASS), QA PASS incl. live run; code commit waits on the user's U2 answer

## Know first
- **All step-20 code is UNCOMMITTED in the working tree** (fingerprint `HEAD=1d44d4b7 TREE=2a4501c794b0598ed2148f083f4bd3a5af05b0da`; QA re-checked it unchanged at the end). Gates passed (review PASS ×3 pieces + QA PASS). It is held only because of the U2 review hold — do not commit before the user answers U2. When committing, stage by piece (P1 agent / P2 protocol+daemon / P3 shell) or as one commit, then compare `git write-tree` with the QA TREE.
- **U2 (pending with the user):** does codex `agent.spawnInto` need a longer deadline than the 7 s bus seat? New data from QA R8: healthy codex creates took 0.71–1.19 s, including right after a daemon restart (738 ms on the bus right after restart); the ≈9.7 s case in `usage_probe.rs:38-39` was NOT reproduced. Silent fake: bus TIMEOUT at 7524 ms, row marked at 10101 ms. Options: (a) longer spawn-verb deadline (seat + CLI), (b) accept (main's recommendation, now with measured data). Also tied to U2: on a slow success or a link drop after `Spawned`, the notice set is recorded as shown while the caller gets TIMEOUT/CONFLICT (no live effect until step 8).
- **Drive C: is nearly full** (9.5 GB free, 98%). The QA executor filled it during a cold mutation build, then used `cargo clean --target-dir` on its OWN two QA target folders (real delete, not the recycle command — the recycle bin is on C: and would not free space). Tell the user; file a safety note (deletion rule bypassed under disk-full).
- Line-ending noise in `src-tauri/bindings/*.ts` — never stage.
- No push (user: 「푸쉬없이」).

## One-line state · next first action
Step 20 implemented and verified. **Next:** (1) ask the user U2 with the R8 data (and the C: drive note, and the open translation question for `prompts/engram-help.md`); (2) on the answer, commit the step (pieces), then the doc pass; (3) then TRD §5-2 step 5.

## Doc pass still to do (after commit)
- Parent `docs/process/S25-preset-library/trd.md` §5-2: add a step-20 entry; "absorbed into step 20" lines at 10a and in the step-11 hand-over; update the `agent.spawn` row (~:390) and the "until 10a" parenthesis (~:529); copy the step-8 hand-over item (remove the debug hook `ENGRAM_QA_BLOCK_CWD` / `ENGRAM_QA_BLOCK_AFTER_ADD_CWD` in `crates/engram-dashboard-daemon/src/lib.rs` `qa_block`, re-run R4/R4b with a real trigger) and step-11 items (webview senders `deliver_notices: true`; WS activation paths Delivered; daemon pre-add block wording vs bus `agent.spawn`).
- `trd-create-body.md`: §3-6 drift (`SpawnRefusal` has `notices` + `live_agent_id`; `launch_blocked` is `Option<Map<String, Value>>`); R7d expected glyph is `"circle"` (title 더블클릭으로 활성화) not `"square"`; §5 claim "codex rows appear only once the handshake settles" is wrong — the row appears within ~100 ms, then turns blocked; record QA result + R8 table; WS cwd is now spelling-normalized (intended, ADR-0348 decision 2).
- `docs/tracking.md`: delete T-76 row + section (closed by step 20, R7c proves teardown).
- `docs/process/step-log.md`: S25 step 20 entry.
- `docs/reference/structure/session-path-ownership.md:673-675` stale `create_and_start` description.

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- HEAD `1d44d4b7` + this handoff commit; ahead of origin by ~209 (nothing pushed this session). Uncommitted: step 20 code (P1+P2+P3, ~32 files incl. tests, bindings `AgentCommand.ts`/`AgentEvent.ts`, `commands.schema.json`, `prompts/engram-help.md`).
- origin/master moved ahead again (7+ commits at an earlier check) — merge before integrating S25.

## Verification state (QA PASS — tree 2a4501c7)
- Full regression (reused, mechanically confirmed): backend 25 steps rc=0, `cargo test --workspace` 5584 passed / 0 failed; bindings sync PASS; frontend vitest 2762 + tsc; gates 34/34 (re-run by QA).
- codex lane PASS (usage_probe_smoke 2, backend_contract --ignored 11; lanes.tsv updated); claude lane n/a.
- Mutation spot (QA, isolated): WS `created_agent_id: Some→None` → red; shell `args.agent_id` insert removed → red.
- Live run R1–R8 all PASS (R4/R4b via debug hook; R5a/R5b CLI; R6 before/after on daemon from 1d44d4b7; R7c error 224 ms / silent 10089 ms with fake pid torn down vs base leaving it alive; R7d kill → no mark; R7b TIMEOUT 7524 ms; R8 timings above). Evidence files went to the recycle bin with `$QA_TMP`.
- Rerun: `cargo test --workspace -- --test-threads=4`; procedure `.claude/skill-bindings/qa-verify.md` 「전체 회귀」.

## Decisions (unchanged from the previous handoff — see history `20261109-021434-…`), plus
- Main: batched comment pass delegated to the P3 coder instead of main-direct (≈12 places over 8 files incl. a test rename) — reason told to the user.
- Main: P3 contract fixes adopted (row shape = bus `agent.spawn` with `line: null`; placement failure carries `args.agent_id`; not-placed clause; help recovery; contract lines first under the CLI 400-char cut). Deferred: daemon pre-add block wording (step 11); link-drop notice loss (U2 family).

## Improvements to file
- QA executor bypassed the recycle rule (`cargo clean`) under disk-full — global feedback: QA briefs should check free disk before cold isolated builds / reuse the shared target dir for mutation copies.
- Translation kit: cross-batch term sheet step (from earlier).
- engram-review: test-procedure lines need code-checking like design lines (R7c `--cd` filter).

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v src-tauri/bindings | head -40`
