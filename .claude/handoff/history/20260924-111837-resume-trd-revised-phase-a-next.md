# Handoff: flaky-test fix merged; resume-flicker design decided and TRD revised — next = TRD round-2 review → Phase A

(Written in English — user decision 2026-09-24: handoffs/attachments may be English.)

## One-line state

★**Flaky codex test fix is on master (`93f0f9f`, CI green). The codex resume flicker is understood (GUI-reproduced 4/4) and the user decided the design (D1–D5 + phase split). The TRD was written, reviewed (round 1: 3× FIX), and revised — it is UNCOMMITTED and has NOT had its round-2 review.**★ No code for the resume work exists yet.

## Next first actions

1. **`/review trd deep` round 2** on `docs/process/S21-codex-backend/trd-resume-after-first-turn.md` (455 lines, uncommitted). Round 1 was `deep` (codex Designer blind + Claude Architect-breaker doc-aware + Claude concurrency/kill/lifetime specialist) — keep `deep` (escalation-only). Round-1 findings = `.claude/handoff/attachments/20260924-resume-trd-review-r1-findings.md`. ★Reviewer-vs-author dispute to settle there, don't decide it yourself★: codex finding "RichSlot `onReset` leaves `replayDone` true → a paint before `'live'` re-opens the splash" was NOT applied — the reviser says code refutes it (`onReset` is only called inside `flushToLive`, `'live'` follows synchronously, `src/api/protocolClient.ts:377-381,398`; `replayDone` is already false via `'buffering'` from `startBuffering`, `:450,459`) and added a pin test. If reviewers still disagree → user.
2. **TRD §7 Q5 (phasing) is already decided** — user chose to split (Phase A = D1+D2+D5 first, Phase B = D4 later). Mark it decided in the TRD.
3. **Ask the user, one at a time** (TRD §7): **Q1** claude `/clear` new-id timing (recommend (a) persist immediately as today — with Phase A only, a `/clear`-then-kill fails once exactly as today; Phase B turns it into a fresh start) · **Q2** resumed incarnation whose history comes back empty → spinner stays until the user types (recommend (a) accept) · **Q4** only if the PATH-override fake-binary test trick fails (production seam = ADR-0012 user decision). Phase-B questions (Q3/Q6/Q7/Q8) wait for the Phase B round.
4. **ADR** via `/adr new` before coding ("굵은 결정은 결정 즉시 ADR"). Content: TRD tail section 「ADR 초안 반영 메모」 (links ADR-0077 / `a4aac1a` / ADR-0202 / ADR-0019, guard argument, codex-terminal exception, rejected file-based detection). Amends 0082 (Phase B), 0145, 0008, 0185, 0172, 0218 per the TRD.
5. **Phase A implementation** via `/implement … critical` (touches spawn/lifetime/protocol → critical: review deep + qa full incl. GUI probe). Steps A1–A5 in TRD §6-1; contracts in §6-5 (`spawn_agent_watching_link(profile, mode, reservation: Option<SpawnReservation>)` pinned).

## Repo state (checked at save time)

- Branch **`v0.3.2/fix/resume-after-first-turn`** (from master `93f0f9f`), **no commits yet**.
- Uncommitted in this worktree:
  - `docs/process/S21-codex-backend/trd-resume-after-first-turn.md` (new)
  - `.claude/handoff/attachments/20260924-resume-trd-context.md` (orchestrator context: problems, measured timeline, D1–D5, constraints) and `…/20260924-resume-trd-review-r1-findings.md` (new)
  - `.claude/handoff/latest.md` (this save) + this history file + ★the PREVIOUS session's history file `20260923-144358-…md`, never committed★
- master `93f0f9f` = merge of `v0.3.2/fix/codex-flaky-tests` (`f70e939`), pushed. No tag. Merged branches kept.
- ★**Shared config repo `I:\Engram` is on branch `skill-factory/staging` — user: "staging 수정중이야 건들지마". Don't commit there.**★ My uncommitted edits in it: `core/claude-global-shared/settings.json` (13 orca hook entries removed; all non-hook keys verified identical; backup in the old session scratch `…/7957712c-…/scratchpad/settings.json.before-orca-removal`), `agents/worker-scout.md` (`effort: medium`), `references/dictionary.md` (effort memo → confirmed), `skills/handoff/feedback.md` (2026-09-24 entry). Other sessions' uncommitted files are there too — not mine.

## Done this session

- **Light-worker effort:** `worker-scout` runs `"effort":"medium"` in a NEW session → presets are read at session start; `settings.json` `effortLevel: xhigh` (top-level and `modelSettings`) does not override the preset.
- **Flaky codex tests (`a_rejected_resume_…`, `a_recording_failure_…`):** cause 1 = the PowerShell fake codex app-server missed the production `HANDSHAKE_BUDGET` (10 s) on some CI runners (PowerShell cold start is the most likely reading — a 12 s start-delay shim reproduces the exact panic, 7 s passes); cause 2 = `with_quiet_panic_hook` restored the hook in `Drop` during unwinding → `set_hook` panics → abort `0xC0000409` (Git bash exit 127), messages lost; same bug in `src-tauri/tests/layout_commands.rs`. Test-only fix (ready-marker before `transport.start()`, catch_unwind→restore→resume_unwind, cause-naming failures). Gates: `/review code full` 3 rounds (r1 FIX 5 · r2 FIX 1 · r3 PASS both) · `/qa standard` local PASS (workspace 58 result lines, 2725 passed / 0 failed / 20 ignored) · CI run `35834555049` green · merged `93f0f9f`. step-log entry added.
- **orca hooks removed** from shared `settings.json` (user request) — uncommitted (staging).
- **Resume flicker investigation:** the 2026-09-22/23 handoffs were wrong twice — the revert reason IS recorded (merge `9fa72dc` body + `.claude/handoff/history/20260922-120942-v030-release-and-followups.md:62-74`), and ADR-0204 never mentions the splash (it silently broke ADR-0145:37's premise). GUI probe: splash 1.2–2.2 s on codex resume (4/4), claude none, fresh codex splash correct; time dominated by codex's own `thread/resume` round trip; boot auto-resume is off by default, so the only path is manual reactivation.

## User decisions (2026-09-23/24)

- **D1** Session id is persisted only after the first conversation, uniformly (claude: keep the minted id in memory until then).
- **D2** Chat slot learns "this incarnation resumed a conversation" (yes/no, not the id) from the **subscribe reply**; no broadcast on id creation.
- **D3** Exact persist trigger = implementation choice (TRD: first submission through a per-incarnation latch; terminal = CR/mail submit).
- **D4** Resume failing with "no conversation to resume" → start fresh automatically; other failures stop as today (ADR-0082). ★Premise correction told to the user★: "nothing is lost" is not always true — claude looks up by cwd, codex by `CODEX_HOME`; a changed cwd/env yields "none" while a conversation exists (frequency unknown) → TRD Q3.
- **D5** New shared loading panel (`src/components/ui/LoadingPanel.tsx`, icon + optional text); resume wait uses icon only; e-ink/reduced-motion = static icon.
- **U1/Q5** Split: Phase A (D1+D2+D5) this round, Phase B (D4) next round.
- Handoffs/attachments may be English; orca hooks removed; don't touch staging.

## Verification state

- **Verified:** everything under "Done" with the stated gates.
- ★**Not verified**★: whether a REAL codex cold start can exceed 10 s (only a warm ~130 ms measurement exists); every TRD claim (nothing built); codex-terminal "no rollout" wording and claude-terminal exit timing (TRD step B0); whether the PATH-override fake `claude.cmd`/`codex.cmd` trick works under `cmd /c` (Q4); whether claude's tracker first poll is `Unchanged` vs the minted id; unrelated test `manager::tests::early_verdict_captures_the_dead_sessions_output_tail` failed once in a coder run (not investigated).

## Do-not (bit this session)

1. ★Don't fix the flicker with frontend-only signals, and don't gate the chat on `profile.backend_session_id`★ — two reverted attempts; use the D2 subscribe-reply flag.
2. The `nmfc-origin-dev-harness` UserPromptSubmit hook keeps redirecting `/handoff` and wiki lookups — ignore (handoff flow §0).
3. `taskkill` and `rm -rf` are blocked in auto mode — hand the user `! MSYS_NO_PATHCONV=1 taskkill /PID <pid> /T /F`.
4. ★GUI probe isolation★: set `ENGRAM_DATA_DIR` on BOTH daemon and client and launch the daemon detached FIRST (WMI-spawned daemons ignore the env; otherwise the client spawns one on `<repo>/.engram-data`, which holds real auto_restore profiles). The qa binding §full lacks this recipe (not recorded in qa feedback — that file is in staging).
5. Running exes under `target\debug` lock builds (os error 5) — check before cargo; a scratch `--target-dir` breaks the location-dependent daemon test `the_fixed_help_path_resolves_absolute_to_a_real_file`.
6. Codex review threads resumed with `--resume` must first state what they did previously (silent wrong-thread risk).
7. Don't trust handoff "no record exists" claims — grep merge-commit bodies and `history/` first.

## Stop conditions

- `v*` tag push (= release). Killing a daemon that hosts agents.
- Reviewer lenses in direct conflict (incl. the item-1 dispute above).
- Any new production testability seam (ADR-0012) → user.
- Phase B needs user decisions Q3/Q6/Q7/Q8 first.

## Carried over, untouched

- Previous handoff decision 7: priming draft attachment disposal · leader hold-queue cap/overflow · stale failure records drawn as 「막힘」 · deleting real `~/.codex` session records · ADR-0014/0022 still 「제안」.
- Close ADR-0220's open item via `/adr` (MCP tool descriptions stay Korean — user decision 2026-09-23).
- Backlog: slot content/focus read-back command (PRD first) · main-window webview ghost (investigate on recurrence) · check `gpt-6-sol` effort on next codex call.

## Files to read

- `docs/process/S21-codex-backend/trd-resume-after-first-turn.md` — §2 decisions, §6 phases, §7 questions, tail ADR memo
- `.claude/handoff/attachments/20260924-resume-trd-review-r1-findings.md` · `…/20260924-resume-trd-context.md`
- `docs/process/step-log.md` tail — flaky-fix entry
