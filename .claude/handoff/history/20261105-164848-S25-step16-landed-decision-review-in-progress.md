# Handoff: S25 step 16 landed (P1–P4 + docs) and pushed (CI not checked) · next = step 5 (claude launch-material translation)

## Know first
- Step 5 is a 멈춤(뒤) step (stop and confirm with the user AFTER it lands). Step 16 was the 멈춤(앞) step — the user approved it (「쭉쭉 진행해」).
- The 26 ` M src-tauri/bindings/*.ts` files are pre-existing line-ending-only noise — never stage them.
- The user wants autonomous progress (「쭉 진행해 … 핸드오프하면 되고 하던거 끝나면」): finish a unit, save a handoff, continue.

## In progress with the user — review of decisions main made on the user's behalf (do this BEFORE step 5)
The user asked to go through, one at a time, what main decided while the user said 「알아서/쭉 진행해」. Explain each from basics (the user said main assumed too much context) and concisely. List:
1. Launch-block reasons not reaching two entrances (screen double-click via Tauri carrier; LLM via shell bus `agent.spawnInto`) — moved to step 11. **Open:** user agreed the human/screen path can wait; for `agent.spawnInto` (LLM path, shell-owned: creates a slot + asks the daemon `SpawnByCwd` + places the agent; folds the reply to text — per reviewer, main has not read the code) main proposed moving it to step 8 (when blocks first become real); the user asked whether it must be done now. Not decided — ask: now vs step 8 vs step 11.
2. Generic notice wording 「알림(<code>): 의도된 동작입니다. 대상 — …」 (shipped; per-code wording in steps 6/7).
3. `--cwd` create-then-blocked: agent stays registered, answer INTERNAL + agent_id "fix the listed items, then start it by that name"; pre-create check deferred to step 10a.
4. Notice record point: only when a process actually started; never moot/boot restore/failure/screen paths (yet).
5. CLI launch-block hint: no "engram agent list" tail; ends "fix the listed items, then start it again".
6. Folder entry details: add refuses missing/unreadable/cmd-unsafe paths; spelling variants deduped; codex terminal still blocks on a missing folder.
Also reported to the user: docs commit `05799d4c` had no separate doc review; CI for the push not checked yet.

## One-line state · next first action
Step 16 「실행 답 넓히기」 is implemented, reviewed (deep per piece, Claude + codex), QA PASS, committed and pushed (`05799d4c`). CI result for that push NOT checked yet.
**Next first action:** (1) check CI for `05799d4c` (`gh run list --branch v0.5.0/feat/preset-library -L 3`); (2) start TRD §5-2 step 5 via `/engram-implement` — `S25: feat(agent): claude 실행 재료 번역 + backend 설정 검사 포트` plus its ADR-0341 addendum (folder entries → `--add-dir` per entry + `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` via `LaunchArtifacts.env`, `path_is_cli_safe` guard → ②, the claude implementation of the folder load set). Begin with a code-map scout (worked well for step 16), then split into sequential coder pieces.

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- origin = local = `05799d4c` before this handoff commit.
- Step 16 commits: `b454a359` P1 command `CommandError` args + control body `{status,code,hint,args?}` + macro `Json` · `e4c442be` P2 agent notices out of a launch + once-per-agent fingerprint (compare-and-set) + ADR-0341 plan fields · `b223449e` P3 bus `agent.spawn` answers · `8200eae4` P4 wire `Error.launch_blocked` · `Spawned.notices` · `05799d4c` docs (TRD step 16 record, step 11 hand-over, review/QA record; step-log entry).
- Earlier this session: master `9152bcaa` merged (`5907bb15`); folder-entry feature research → ADR-0341 → PRD/TRD reviewed (`73317d2a` … `f5b37adc`).

## Verification state
- Step 16 QA (`/engram-qa`, tree HEAD `a1c440a5`): PASS — full regression local (Rust 5220 passed · frontend vitest 2760 · tsc · 30 gates · bindings sync) · claude lane 10 (lanes.tsv claude line updated) · codex lane n/a (path untouched, version same 0.163.0) · mutation 4/4 · GUI smoke (claude terminal agent answered `QAPONG7`, killed, left the list — screenshot checked by main).
- Not run: the separate `/engram-review doc` on the docs commit `05799d4c` (it records facts from reviewed commits; reported to the user as not done). CI for the push.
- Rerun: `cargo test --workspace -- --test-threads=4` (full procedure = `.claude/skill-bindings/qa-verify.md` 「전체 회귀」).

## Decisions (this session — all recorded in TRD/step-log/commit bodies)
- ADR-0341 preset folder entry (user decisions 2026-11-04/05): claude reads natively (`--add-dir` + env var), codex fills only what it lacks (AGENTS.override.md > AGENTS.md → instructions; `.agents/skills` + `.codex/skills` → `extraRoots`), codex terminal omits + notifies once per agent, missing folder blocks on every backend/mode, a folder a backend loads nothing from gets no notice.
- Step 16: fingerprint = canonical identity string; recorded only when a delivering caller's activation started a process (compare-and-set vs prelaunch baseline); WS paths never record yet; `--cwd` pre-create check → step 10a; notice codes/wording → steps 6/7; generic notice text accepted (user did not object).
- Moved to step 11 (user 「쭉 진행해」 on main's recommendation): WS activations → `Delivered` + `Spawn` Ack → `Spawned` when the webview shows notices; Tauri carrier folds `Error` to text (launch_blocked lost); shell `agent.spawnInto` gives the LLM neither notices nor reasons.
- Not fixed (in commit bodies): bidi/format chars not escaped in notice_text; an improbable reverse record interleaving; over-deep `args` vs serde recursion limit; small test-wording leftovers.

## Do-not / lessons
- Coders: Edit tool only (mixed line endings); format only own lines. Tell workers to wait inside one tool call (no polling across turns).
- `row-diff.mjs` needs a commit ref; for uncommitted snapshots use `--files` with blob copies. Git Bash heredocs mangle backslash paths — write briefs with the Write tool.
- When announcing a delegation, spawn it in the same turn (feedback filed in global-feedback.md).
- Unverified handoff assumptions must be confirmed with the user on load (feedback filed in engram-handoff/feedback.md).
- Big single-coder pieces exhaust context (P4a precedent); split by file overlap from a scout map — step 16's 4 pieces each landed in one coder session.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -8`
