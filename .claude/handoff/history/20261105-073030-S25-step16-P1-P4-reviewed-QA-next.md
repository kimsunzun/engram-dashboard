# Handoff: S25 step 16 「실행 답 넓히기」 — P1–P4 coded + reviewed (all PASS) · next = step-16 QA, then squash P4 + docs commit + push

## Know first
- One user decision is pending (it holds the final P4 commit and the push): two gaps found by P4 reviewers, outside step-16 scope, unreachable until real launch kits are wired (step 8):
  1. Tauri carrier folds the daemon `AgentEvent::Error` into a string (`protocol_state.rs` `classify_reply` → `CommandFailure::Daemon{message}` → `commands/agent.rs` `forward_to_daemon` `Err(String)`), so `launch_blocked` never reaches the webview; TRD §1-8/§1-10 expects the webview to open the blocked window from it.
  2. Shell bus `agent.spawnInto` (LLM/CLI-callable, `layout.rs` ~113-128) folds the `SpawnByCwd` reply to id/text — the LLM gets neither `notices` nor structured reasons, unlike daemon `agent.spawn` (TRD L290 / CLAUDE.md LLM-first).
  Main recommended: put both into TRD step 11 (webview↔daemon round trip). Alternative: fix now as a P5 shell piece. Asked the user; no answer yet.
- Also pending (non-blocking): user has not confirmed the generic notice wording `알림(<code>): 의도된 동작입니다. 대상 — v1 · v2`. Per-code wording belongs to steps 6/7. If no answer, proceed with it.
- The 26 ` M src-tauri/bindings/*.ts` files are line-ending-only noise from before this session — never stage them.

## One-line state · next first action
Step 16 split into 4 pieces, each coded by a fresh worker-senior coder, reviewed `/engram-review code deep` (Claude checklist + Claude risk-axis + codex), fixed, re-reviewed PASS.
**Next first action = `/engram-qa` for all of step 16** (full regression per `.claude/skill-bindings/qa.md` / `qa-verify.md`; base = `f5b37adc`, head = `280a015f`). Then: squash the 3 P4 WIP commits into one `S25: feat(protocol,daemon,shell): ... (step 16 P4)` commit (template = P1–P3 commit bodies) — only after the user answers the pending gap decision; then the docs commit; then push.

## Repo state (branch v0.5.0/feat/preset-library · wt1)
- origin/v0.5.0/feat/preset-library = `f5b37adc`; local HEAD = `280a015f` (before this handoff commit), 6 ahead, not pushed.
- Commits: `b454a359` P1 (CommandError args · control body `{status,code,hint,args?}` · macro `Json` field) · `e4c442be` P2 (agent notices out of a launch · once-per-agent fingerprint · ADR-0341 plan fields) · `b223449e` P3 (bus `agent.spawn` answers: wake block/race → CONFLICT + args.reasons, after-create failures → INTERNAL + args.agent_id, success `notices`, help text) · `0df2a2c2` WIP P4 (wire `AgentEvent::Error.launch_blocked` · `Spawned.notices`, PROTOCOL_VERSION stays 8) · `a9947479` WIP P4 test (WS Spawn records no fingerprint) · `280a015f` WIP P4 batched comment fixes by main.
- Uncommitted: none except the 26 bindings noise files.

## Verification state
- Coder-run tests per piece (green): command 119 · agent lib 1538 · activation 22 · daemon lib 624+ (connection_core 79) · daemon all 819+4 · cli 114 + engram_cli 45 · protocol 103+13+1 · shell lib_unit 1328 · tsc OK · `cargo build --workspace` OK.
- NOT run: full QA (all lanes, gates, frontend vitest, real-app smoke) for step 16 — that is the next action.
- Review records are in each commit body (P1–P3); P4: all three PASS, 권장 test gap fixed (mutation-checked), batched comments fixed by main.

## Decisions this session (main, internal — reported to user)
- Step 16 split P1 (command) → P2 (agent) → P3 (bus) → P4 (wire), sequential (shared target dir/build).
- Fingerprint = canonical identity string (no hash); recorded only when a caller activation started a process (fresh or caller resume), via compare-and-set against the prelaunch baseline; never moot/boot-restore/failure; `spawn_agent_reporting` never records; WS paths never record in step 16 (webview does not show notices yet) — TODO(TRD S25 §5-2 11) anchored in connection_core.rs.
- `--cwd` pre-create check deferred to step 10a (TODO anchored in commands.rs `create_and_start`); `agent.spawn` summary states current behaviour.
- `args: {}` = absence; unknown code's args stay raw (not typed); launch-refusal CONFLICT gets no roster tail (prefix `LAUNCH_REFUSAL`).
- Accepted (TRD): fingerprint recorded even if the reply is lost after leaving the manager.
- Not fixed (recorded in commit bodies): bidi/format chars not escaped in notice_text; improbable reverse interleaving; recursion-depth of args (no producer); a daemon unit fixture with old "launch blocked" CONFLICT wording; `LAUNCH_RACED_CAUSE` duplicates Display text.

## Docs to update in the step-16 docs commit (not done yet)
- TRD: step 16 record-point wording ("record when the call started a process", not RestoreOutcome enums); WS paths do not record until step 11 (Ack Spawn has no notices — move to Spawned when the webview needs it); `--cwd` pre-create check moved to step 10a; §1-8 text citing `ControlQueryResult::Error{code, hint}` is stale (now `args?`); notice codes/wording owned by steps 6/7; fingerprint = identity string; the two pending gaps → wherever the user decides.
- TRD review record line (PRD+TRD review PASS 2026-11-04, prd+trd full, Claude+codex) per implement binding.
- step-log `## S25`: step 16 entry.

## Earlier this session (done)
- Master `9152bcaa` merged (`5907bb15`) and pushed. Folder-entry feature: research → ADR-0341 → PRD/TRD → review PASS → pushed `f5b37adc`.
- Feedback filed: engram-handoff feedback (confirm unverified assumptions on load) · global-feedback (announced next step instead of running it).

## Do-not / lessons
- Do not `sed`/perl edit files with mixed line endings — Edit tool only (coders told so).
- `row-diff.mjs` needs a commit ref, not a tree hash; for uncommitted snapshots use `--files` with blob copies.
- Heredoc in Git Bash mangles backslashes in Windows paths — write briefs with the Write tool.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline f5b37adc..HEAD`
