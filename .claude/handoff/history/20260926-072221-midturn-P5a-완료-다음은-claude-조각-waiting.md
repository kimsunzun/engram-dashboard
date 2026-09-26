# 핸드오프: mid-turn input P0·P1·P4 게이트 통과 · P5a·P5b WIP(게이트 전) — 다음 첫 작업은 claude 조각 스트리밍 + 채팅 「waiting…」(사용자 지시)

(English body — user decision 2026-09-24. The user delegated TRD confirmation + implementation on 2026-09-26 (「나 자러갈테니깐 쭉쭉 진행해. 충분히 얘기한것같으니 구현 너가 알아서하고 후에 최종 보고하는걸로」), then asked to finish the in-flight work and hand off (「하던거 다 끝나면 핸드오프 하면 될듯」). Master merge and `v*` tags are left for the user.)

## One-line state

Mid-turn input implementation per `.claude/handoff/attachments/20260926-midturn-impl/plan.md`: **P0+P1** (`fcba626`) and **P4** (`96ead26`) are gated and pushed (CI green). **P5a** (`232724b`) and **P5b** (`43618c0`) are WIP-committed locally, coder gates green, NOT reviewed — they must land together in the P5 phase commit. No in-flight workers. **The user ordered a new next task first:** claude `--include-partial-messages` streaming + chat 「waiting…」 indicator (T-12) — see `.claude/handoff/attachments/20260926-next-task-claude-partial-waiting.md`.

## Next first actions

1. **Next task (user order):** claude partial streaming + chat waiting indicator. Brief = `.claude/handoff/attachments/20260926-next-task-claude-partial-waiting.md` (scope, pointers `docs/tracking.md:216/221` T-12, cautions). Follow CLAUDE.md dev steps (PRD/TRD options → user decides → implement). Do NOT run it in parallel with mid-turn P3 (both touch the claude decoder). Confirm with the user in one line at session start whether it goes before the rest of mid-turn (the user said 「다음작업」 = next).
   - ★Branch: this is a new topic → new branch from master per CLAUDE.md (`v<next release>/feat/<slug>`), NOT on the mid-turn branch — unless the user says otherwise. Note: mid-turn changes (PROTOCOL_VERSION 6 etc.) are not on master yet.
2. **Resume mid-turn afterwards:** P5c (UI list + reattach reconciliation; carried items in contacts: handleEvent branches for `QueuedInputs`/`QueuedInputCancelReply`, bare lease refusal → CONFLICT, cancel command without `help`, tolerate unknown words, seq tracking for reconciliation) → **P5 gate**: `/review code deep` on `96ead26..HEAD` (P5a+P5b+P5c) → fixes → squash (commit docs separately from code) → push → CI → local QA in an isolated worktree (GUI: JSON chat + replay/reconnect unchanged on both backends, terminal unchanged) → P3a → P3b → P3 gate → P2a…P2f → P2 gate → P7 (M15, needs P2e2) → P8 (only if M15 green) → P6 (docs + final QA) → final report.

## Working pattern (keep — proven this session)

- Coders: `worker-senior` (critical) / `worker-scout` (simple); every brief starts with `.claude/handoff/attachments/20260926-midturn-impl/coder-brief-common.md`; landed shapes + carried review items in `.claude/handoff/attachments/20260926-midturn-impl/contacts.md` (authoritative over plan §3). A chunk ≈ 250–390k tokens, 70–120 tool calls, 15–35 min.
- WIP-commit each chunk by explicit paths; phase gate = `git reset --soft <phase base>` → commit docs separately → commit code with an itemized body → push → CI (`gh run list --branch v0.3.2/feat/json-midturn-queue`).
- Review per phase = `/review code deep`: codex blind (`codex:codex-rescue`, `--fresh`, read-only, effort high, no `--background`) + Claude doc-aware (`worker-senior`) + one extra lens (concurrency / API contract). Re-verify fixes via `SendMessage` to the same reviewers.
- QA per phase: CI = build/test; local = real-claude tests (CI `--skip` list) + ADR-0130 trigger vs parent + GUI/CLI smoke in an isolated worktree built from the gated commit (shell AND daemon — PROTOCOL_VERSION 6).

## Decisions (authoritative records)

- User answers, design direction, delegation, next task: `.claude/handoff/attachments/20260926-midturn-answers.md`, `.claude/handoff/attachments/20260926-next-task-claude-partial-waiting.md`; PRD 6판 §3/§6; TRD 8판 §10-1; ADR-0231.
- Delegated rulings: one mail gate = kernel `TurnFact.last_end_failed`, both backends, ALL versions; interrupt neither sets nor releases; turn-end probe removed; late echo after turn end → bubble, no automatic answer; plan §6; P1 review: halt fold with its own cursor, finish does not ring `inputs_drained`; P4: errors `[NOT_FOUND, CONFLICT]`, WS reply `QueuedInputCancelReply`.

## Repo state

- Branch `v0.3.2/feat/json-midturn-queue` (wt1). Pushed to `96ead26`. Local on top: `43618c0` WIP P5b · `ec2d308` handoff · `232724b` WIP P5a · (this handoff commit). P5 phase base = `96ead26` (the squash must keep the handoff/docs commits out of the code commit).
- Uncommitted noise: `src-tauri/bindings/*.ts` line-ending-only "M" (no content diff) — leave.

## Verification state

- P0+P1: review deep PASS (after fixes) · CI green · local QA PASS.
- P4: review deep PASS (after fixes) · CI green · local QA: real-claude 6/6 PASS, GUI PASS, **CLI PARTIAL** (`engram.exe` needs an agent token; bus verified via the WS `Command` entrance), **ADR-0130 alert** (2 new production `use` lines of an existing module edge — see contacts).
- P5a/P5b: coder gates only (protocol/agent/daemon/lib_unit + 4 shell targets + tsc + vitest 1264 all green); NOT reviewed, NOT QA'd.
- Measurements: M9/M10/M13/M16 green; M15 not measured.
- No user-visible feature yet (policy `None` on every backend until P3b/P2c).

## Traps

1. TRD lines are 3–5 KB — never let a worker read it whole.
2. Builds/tests only via `scripts/run-detached.ps1` (call it directly; never wrap it so its `PID=`/`LOG=` output is lost), judged by `__EXIT`; agent/daemon/base tests need `-- --test-threads=4`.
3. Phase 0 harness `.js` fails in-repo (ESM) — copy as `.cjs` outside.
4. Shared worktree: WIP-commit only the finishing worker's files; never `git add -A`.
5. P5a + P5b land in the same phase commit.
6. A worker can die on an API SSL error — resume it with `SendMessage` (partial edits stay).
7. `engram.exe` CLI only works inside an engram-spawned agent (NO_TOKEN otherwise) — CLI smoke needs an agent-run probe or the WS `Command` entrance.
8. `nmfc-origin-dev-harness` hooks misfire every turn — ignore; this project's handoff is the user's `handoff` skill.

## Stop conditions

- `/review code` BLOCK, direct reviewer contradiction, implement loop cap → stop that phase, ask the user.
- M15 red → no P8, keep `HandOverPolicy::Immediate` (decided; report).
- Never: master merge, `v*` tag push, history rewrite/force-push, touching wt2 or apps not started by us.

## For the user (collected — also the final report)

- ★**Privacy (needs a user decision):** the repo is PUBLIC; the first push of this branch published older committed Phase 0 raw logs (`.claude/handoff/attachments/20260925-midturn-phase0/logs/claude-M*.jsonl`, `codex-M6/M7…`) with the OS account name (= GitHub handle) and internal plugin names, plus a 2026-09-25 handoff mentioning the company account domain. master already had similar content (`claude/fixtures/claude_{text,tool}.jsonl`, `.claude/handoff/attachments/codex-control-2026-09-16.md`, a June handoff). Options: forward scrub commit and/or history rewrite + force-push (destructive).
- Old codex now also stops mail after a failed turn (delegated ruling).
- Late echo after turn end → bubble without an automatic answer (differs from the N12 explanation).
- ADR-0130 trigger: pre-existing production matches + 2 new lines from P4 (const import) — worth a separate look.
- P6 owed docs: full list in contacts.md.
