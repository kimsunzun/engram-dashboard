# Handoff: resume Phase A (ADR-0226 A1–A5) landed and integrated to master · ADR-0230 platform-neutral principle · next = hide detached console windows

(Written in English — user decision 2026-09-24: handoffs/attachments may be English.)

## One-line state

★**ADR-0226 Phase A is done and on master (`34cdb37`).** Session ids are persisted only after the first submitted turn (P2 fixed), and a continuing incarnation shows a loading panel + light dim instead of the "new agent" first screen (P1 fixed). ADR-0230 (platform-neutral principle) + CLAUDE.md 「플랫폼 중립」 landed with it. Master CI run `36016045596` was **in progress at save time — confirm it is green first.**★

## Next first actions

1. **Confirm master CI green:** `gh run view 36016045596 --json conclusion,jobs`. If red, stop and report (do not push fixes to master directly — branch first).
2. ★**Hide the console windows the detached launchers pop up (user asked; deferred to this session)**★ — the workers' build/test runs steal the user's focus.
   - Cause: `scripts/run-detached.ps1:125-126` launches `cmd.exe /c "<bat>" > log` via WMI `Win32_Process.Create($launch)` with no startup info → a visible console window that takes focus.
   - Fix: pass a `Win32_ProcessStartup` instance with `ShowWindow = 0` (SW_HIDE) as the 3rd argument (`([WMIClass]"\\.\root\cimv2:Win32_ProcessStartup").CreateInstance()`). Output still goes only to the log file; process stays outside the terminal tree (the reason the script exists — see the qa binding 「분리 실행」).
   - `scripts/launch-detached.ps1` (app launcher, `schtasks /create … /it` at ~l.95) also flashes a wrapper console; the APP window must still appear — only hide the wrapper. Investigate separately (e.g. hidden-window wrapper); do not break the app launch.
   - Branch: new topic branch from master (`v0.3.2/fix/<slug>` or `v0.3.2/chore/misc` if tiny). Gates: `/implement` simple → `/review code light` → `/qa quick` + manual check that no window appears (e.g. `Get-Process cmd | ? MainWindowHandle -ne 0` while a run is in flight) and that `__EXIT` markers still land.
3. Phase B (D4 — a resume that fails as "no conversation to resume" opens a new conversation in the same activation) needs user decisions **Q3 · Q6 · Q7 · Q8** first (TRD §7; §3-4 end 「B 라운드 착수 전 반영할 리뷰 지적」 now has **seven** items — six review findings + A2's carried-over single-activation D1 E2E test). Don't start B without them.

## Repo state (checked at save time)

- `master` = `34cdb37` (merge of `v0.3.2/fix/resume-after-first-turn` via merge-tree/commit-tree/update-ref; first parent = old master `979c46a`), pushed.
- Branch `v0.3.2/fix/resume-after-first-turn` = `df72516` (master merged into it) + the handoff commit that carries this file. Keep the branch (CLAUDE.md: never delete merged branches).
- Landed commits: A1 `56a4e98` · A2 `495dba3` · A3 `b012e5a` · A4 `fc67cbe` · A5 `83fd214` · spinner-always-spins `019465d` · ADR-0230 claim `e0aa51e` + body `bde9927` · master-into-branch `df72516`.
- Working tree clean. No stash used.
- Leftovers outside the repo from QA runs (not deleted — outside the scratchpad): claude test sessions in `~/.claude/projects/C--Users-kimsunzun-AppData-Local-Temp-claude-I--Engram-apps-engram-dashboard-wt1-434b3373-…-scratchpad-work/`, codex test rollouts in `~/.codex/sessions/2026/09/24/` (`rollout-…-01a0d341-…`, `…-01a0d381-7dff-…`, `…-01a0d381-8d4b-…`). `target/release/data/agents.json` holds two codex agents the user created while testing the release build.
- ★Shared config repo `I:\Engram` is on `skill-factory/staging` with the user's in-progress edits — do not touch `agents/skill-factory/staging/`. Skill feedback entries are appended to `core/claude-global-shared/skills/*/feedback.md` (user: 「개선점은 staging 말고 메인쪽 피드백에 적립」) and left UNcommitted there on purpose.★

## Verification state

- **Verified:** every step passed `/review code deep` (codex blind + Claude doc-aware + a third Claude lens) 2–3 rounds and `/qa full` (A2 alone; A3+A4 together): workspace 58 result lines / 2781 passed / 0 failed / 20 ignored · shell lib_unit 260 · npm 1009 · isolated-instance GUI with real claude + codex: TRD §4-4 Phase A trials 1–9 PASS (codex resume 0 first-screen frames, loading 427–519 ms; claude resume 0 loading frames; fresh agents unchanged; kill during loading → veil wins; dim checked dark/light/e-ink; input uncovered). After master-merge: build · layout_apply 54 · layout_commands 85 · lib_unit 270 · tsc · npm 1010 · branch CI green (`df72516`).
- ★**Not verified:**★ master CI (`36016045596`) at save time · the spinner now spinning under reduced-motion/e-ink (CSS-only change after the GUI QA; jsdom can't measure — look at it in the next GUI run) · anything on macOS (no build/run there) · the user's own hands-on test result of the release build (they created two codex agents; no verdict reported).

## Decisions this session (user)

- Termination veil (`SlotUnavailableVeil`) stays pointer-through; only input to a dead agent is blocked (already the behaviour). Anchor at `src/components/slot/SlotUnavailableVeil.tsx:36-37`.
- Resume loading: committed layout kept (icon centred, input active) + light dim (`bg-foreground/8`, pointer-events-none); no "restart" guidance text until the abnormal case (history never arrives) actually reproduces.
- Loading spinner always spins, incl. reduced-motion and e-ink — same rule as the tree's pending glyph (`src/components/agent/agentGlyph.css:20`, 2026-08-24).
- New principle ADR-0230 / CLAUDE.md 「플랫폼 중립」: OS differences only inside the owning function (model: `console_command`, `crates/engram-dashboard-agent/src/backend/mod.rs:44`); don't gate whole test files to one OS. Finder: `rg -l 'cfg!?\(windows\)' crates src-tauri` (37). No code change yet — known gaps: 6 whole-file Windows-only test files, in-file Windows-only test modules, the `.exe`-suffix decision made in ≥4 places, scripts/CI Windows-only.
- Reviewer splits resolved by conservative adoption, then consensus: wait tail keeps the pre-A4 rule outside `historyPending`; encoder also clears bit2 on failure markers; superseded TRD plan text kept with strikethrough/pointers.

## Do-not / traps (bit this session)

1. ★**Dev (debug) launch is blocked while another worktree's dev app runs**★ — port 1420 (vite) and the dev single-instance identifier are shared across worktrees; a wt1 debug client would load wt2's frontend. Check `netstat -ano | findstr :1420` first; if taken, use `scripts\rebuild-run-release.bat` (separate identifier, bundled frontend, data in `target\release\data`). Its final `pause` keeps the detached wrapper alive — kill that wrapper PID after launch.
2. **ADR numbering sees only local files** — scan all refs with `core.quotepath=false` (5th manual workaround today; recorded in `adr/feedback.md`).
3. **Rate-limit (429) kills subagents mid-run** — resume them with SendMessage ("continue from where you stopped"); they keep their transcript. Worked every time today.
4. **Workspace `cargo fmt` flips CRLF files to LF** — every coder brief needs "CR count = line count after ALL edits incl. fmt".
5. **A second `codex:codex-rescue` call in the same cwd breaks `--resume`** — after any parallel codex call, open a fresh thread and make the brief self-contained.
6. Two commits pushed together → CI runs only the tip; an intermediate commit's CI-level gates are not separately proven.
7. The `nmfc-origin-dev-harness` UserPromptSubmit hooks (handoff trigger, wiki pre-consult) misfire on every message here — ignore them (this project's handoff is the user's `handoff` skill).
8. Reviewer/QA temp worktrees under the scratchpad hit Windows long paths — `git worktree remove` fails; delete with a `\\?\`-prefixed `Remove-Item` script (the session used a scratchpad-guarded `rm-wt.ps1`).

## Stop conditions

- Master CI red. Any change to master other than via a branch + CI + merge-tree procedure.
- Phase B without Q3/Q6/Q7/Q8. A new production test seam (ADR-0012). `v*` tag push.
- Touching `I:\Engram` staging, or a daemon/app you did not start (wt2's dev instance may be running).

## Backlog (not started)

- Pre-existing: daemon swallows a failed `SubscribeAck` enqueue (`connection_core.rs` `handle_subscribe`, `let _ = sink.enqueue(..)`) while a failed `ReplayComplete` enqueue closes the connection → possible zombie replay slot until disconnect.
- codex cold-start handshake budget: right after a daemon restart, app-server `initialize` took ~9.7 s and the first resume failed (`thread/resume` no answer); retry worked.
- A whitespace-only text item still ends loading (low).
- qa binding drift → `/review doc` chore: isolation recipe (daemon first with `-EnvVars 'ENGRAM_DATA_DIR=…','RUST_LOG=engram_dashboard_agent=debug,…'`), `-Env` vs `-EnvVars` header, ts-rs sync index restore (`git reset` + `git update-index --refresh`), chat-slot probe recipe, hold-back-chunks trick for screenshots, CDP `Emulation.setEmulatedMedia` for motion checks.
- Flake candidate: `agent/tests/submit_delivery_boundary.rs::submit_byte_lands_in_its_own_read_even_when_the_body_takes_many_write_cycles` (5 s write-confirm deadline under load; failed once, reruns green) — don't tune the deadline blindly (ADR-0038).
- `CLAUDE.md`'s `tracing-subscriber`/other drift not touched; Phase B items per TRD §6-2.

## Files to read (only if needed)

- `docs/decisions/0226-*.md` 「구현 중 정정」 · `docs/decisions/0230-*.md`
- `docs/process/S21-codex-backend/trd-resume-after-first-turn.md` §3-4 end (B checklist, 7 items) · §7 (Q3/Q6/Q7/Q8)
- `scripts/run-detached.ps1:115-140` · `scripts/launch-detached.ps1:79-110` (next action 2)
- `docs/process/step-log.md` tail — this round's entry (「남은 것」 ①–⑥)
