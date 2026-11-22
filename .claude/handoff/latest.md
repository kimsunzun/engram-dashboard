# Handoff: S37 — U2 (C2, C3) and U3a committed + pushed; U3b and U4 in fix round 3 (uncommitted); two user decisions pending

## Know first
- Mode: autonomous (user: 「자율모드는 그대로 유지하고 상한은 초기화해」, later approved a 4th C3 round past the cap). The user went to sleep; they test two PCs at home themselves (U5) — leave it.
- Never use coined labels in user-facing text (no "U3b", "C3", "piece"); describe what happens. Korean, short, big picture first.
- Agent cap: 5 at once. Parallel coders must put temp files under their own scratchpad subfolder (`u3b/`, `u4/`) — they collided once.
- **Two user decisions are pending (each blocks a commit, `hold`):**
  1. App: what "the PC list slot is visible" means for routing approval questions. TRD says "active tab of a non-minimized window"; the main's U4 brief made it stricter: active tab of the **focused** window, plus a 30 s fallback to the OS dialog while visible, plus a 1 s delay before reporting "no longer seen". Options: keep the stricter rule (record it in ADR-0394 / TRD) or go back to the TRD rule.
  2. Remote mail: a size cap on directory copies / in-progress multi-part snapshots from a trusted but hostile PC (unbounded today; ADR-0391 rejected an entry cap, ADR-0390 says survive a broken peer). Suggested narrow fix: bound only the in-progress partial (parts / bytes → drop + DirResync).
- Ask both, one at a time, with what changes visibly (rules in user-facing-rules).

## One-line state · next first action
Committed and pushed on `v0.5.0/feat/multi-daemon`: C2 peer links (`4f99b2f7`), U3a messaging remote branch (`e289c93b`), C3 approvals + `peer.*` + typed caller (`1e1d308e`), ADR-0403 / 0404 / 0406, TRD U2 Implemented (`ce0c7d91`). In progress (uncommitted): **U3b** (remote mail wiring + agent directory, main tree) and **U4** (PC list slot + peer-state broadcast + app route + protocol v9, worktree `.claude/worktrees/agent-aa9b381c25983f83e`). Both coders were cut off by the usage limit during **fix round 3** — their partial edits are on disk.
**Next first action:** resume both coders with SendMessage ("continue round 3 from where you stopped; report as before") — U3b coder agent id in this session `a269f52b52d1d17cc`, U4 coder `aa9b381c25983f83e` (if a new session cannot reach them, spawn a fresh coder with the round-3 request below + "finish or redo round 3 on the current tree; report"). Round-3 requests: U3b G1–G7 and U4 H1–H4 — the exact text is in this session's SendMessage; the gist is in `.claude/review-tmp/fix-U3b-r2.md` / `fix-U4-r2.md` follow-ups (see Findings below). Then re-review (lead + codex; the risk-axis lens has used its 2 rounds for both), QA, then integrate U4 onto the branch, then commit, push.

## Repo state (measured 2026-11-23 05:02)
- Branch `v0.5.0/feat/multi-daemon` @ `ce0c7d91`, 0/0 with origin (this handoff commit comes after).
- Main tree uncommitted: U3b (26 status lines; snapshot of round 2 = tree `0eaddab0…`; round-3 edits partly applied, unreviewed).
- U4 worktree `.claude/worktrees/agent-aa9b381c25983f83e` (branch `worktree-agent-aa9b381c25983f83e`, base `ce0c7d91`), uncommitted; round-2 snapshot tree `1b103c10…`; round-3 edits partly applied.
- Untracked `.claude/explain/` (old, leave). Other old worktrees under `.claude/worktrees/` belong to other sessions — leave them.

## Verification state
- Committed pieces: each passed deep review + QA (details in their commit messages). C3 QA PARTIAL items in `docs/unverified-tests.md`.
- U3b: review round 2 → lead PASS, codex FIX (4 must → G1–G4); last full tests at round 2: daemon lib 945, messaging 328, gates 30/30. **Round-3 code: not built, not reviewed — do not trust it.**
- U4: review round 2 → lead FIX (G1), codex FIX (3 must) → H1–H4; last tests at round 2: daemon lib 905, npm 3020, gates 31/31. **Round-3 code: not built, not reviewed.** GUI check not done (focus path, ResizeObserver timing, wrap in Radix ScrollArea).
- Re-run: `cargo test -p engram-dashboard-daemon -- --test-threads=4`, `cargo test -p engram-dashboard-messaging -- --test-threads=4`, `npm test`, CI gates static steps (`.github/workflows/ci.yml` gates job) — via `scripts/run-detached.ps1`.

## Findings carried into round 3 (what the coders were asked)
- U4: H1 click-time guard (`lastChange` ref bumped in the ResizeObserver callback, on hold-key change and on viewport scroll; yes-buttons / ConfirmBtn refuse a press within HOLD_MS) + hold keyed by a revision counter (A→B→A); H2 Strict Mode — new watched set per observer; H3 GhostCard words the translated heading, not the English summary; H4 fixed buttons `maxWidth: 100%`.
- U3b: G1 resync worker vs latest reply target (completion must not erase a newer request); G2 clear overdue queues on revoke / stop, check trust + stopped per job; G3 re-check local ids on an assembled multi-part snapshot at commit; G4 reject resync insertion from a retired epoch; G5 keep chosen key on the copies side so directory application does not hold the link book; G6 recheck copies only when the local id set changed, off the async task; G7 assert the flush trigger is wired (gate never spans inline injection). Record only: inbound Mail trust-check-then-commit window is an accepted residual.

## Integration notes (main)
- U4 merges onto the branch after U3b commits: expected conflicts in `link/peer/{commands,runtime,testing}.rs`, peer catalog version, `prompts/engram-help.md`; `the_app_state_serializes_like_the_peer_list_answer` must absorb U3b's agent rows. Protocol is v9 in U4 (ADR-0383's legacy-marker removal done there).
- Record at commit: R7 rule (first 64 rows to one PC go, the rest fail REMOTE_LIMIT with a split hint) in TRD §7 U3b / ADR; U4 visibility rule after the user decides; U3b differences (adapters in messaging_host.rs because of daemon gate 1, `reply_by_raw` optional — codex downgraded to a note, PeerWire unreleased).
- QA lanes: claude lane needed for both (control/** and messaging/src/** touched).

## Do-not / lessons
- Codex re-asks with facts work (two protocol-bump findings: one withdrawn, one kept → v9). Use the fixed re-ask format.
- A one-shot coder brief must list the CI gates static steps (feedback recorded in engram-implement).
- The Codex default model once failed ("gpt-6.1-sol not supported with a ChatGPT account"); the user refilled and it worked — if it recurs, ask the user.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -4 && git -C I:/Engram/apps/engram-dashboard status -sb | head -5 && git -C I:/Engram/apps/engram-dashboard/.claude/worktrees/agent-aa9b381c25983f83e status -sb | head -3`

## Decisions (this session)
User-decided:
- Continue autonomously, push at the end, two-PC test at home · (묻지 않음) · 「이어서 진행해. 실제 외부 테스트는 내가 별도로 집에서 할테니 놔두고 나머지 다 하면될듯」.
- Reset the fix-round cap · (묻지 않음) · 「자율모드는 그대로 유지하고 상한은 초기화해」.
- C3 4th fix round past the cap · 선택지 4th round ★ / stop and record · 고른 것 「4번째 수정 진행 (Recommended)」 · 같음.
- Next session: lighten subagent base load · (묻지 않음) · 「다음에는 토큰 경량화좀 확인해보자. 서브에이전트한테 기본 도구가 너무 무거워」 → recorded in global-feedback.
- Continue in this session overnight vs hand off · 선택지 continue ★ / hand off now · 고른 것 「ㅇㅇ 쭉 진행해.」 · 같음.
Claude-decided: dialog text en + ko with the shell telling the daemon (ADR-0406, by the CLAUDE.md language rule; user may still choose English-only); protocol bump to v9 in U4 (codex kept it as must); U3a / U4 in worktrees, U3b and C2 / C3 in the main tree; PARTIAL QA items recorded in unverified-tests per the earlier user rule.
Open: the two holds above.

## Feedback recorded this session
global (count per-spawn fixed load; trim subagent base load next), engram-implement (coders run CI static gates). Not yet recorded: parallel coders' scratchpad collision (engram-implement flow §2 already says "own prefix" — the main did not copy it into the briefs).
