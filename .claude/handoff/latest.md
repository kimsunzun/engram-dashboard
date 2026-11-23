# Handoff: S37 — U3b committed; U4 merged onto the branch (uncommitted), QA PARTIAL (one GUI item needs an unlocked session)

## Know first
- Mode: autonomous. Usage limit hit at 22:46. No subagents left running.
- Never use coined labels in user-facing text; Korean, short.

## One-line state · next first action
U3b committed (e866ffc2). U4 applied onto the main tree and resolved (uncommitted, index partly staged); reviews PASS (lead + codex, merge rounds incl. app copy agents:false and peerStore agents:false). QA (`.claude/review-tmp/qa-U4-report.md`) = PARTIAL: everything passed except GUI check (c) — a real window raise and whether WebView2 fires 'focus' before the activating click — the Windows session was locked.
**Next:** with an unlocked session, re-run only GUI item (c) (evidence folder `.claude/review-tmp/qa-U4-evidence/`), or ask the user to accept it as PARTIAL (record in docs/unverified-tests.md). Then commit U4 (stage all U4 files incl. untracked `crates/engram-dashboard-protocol/bindings/PeerStateAgent.ts`, exclude `.claude/explain/`), mark TRD U4 Implemented (differences: ADR-0410; app copy and slot read without agent rows; protocol PeerStatePc directory/agents optional; rename takes_questions/present), push, CI.

## Repo state (measured 20261123-224608)
- Branch v0.5.0/feat/multi-daemon @ e866ffc2; unpushed commits since origin 7a82f131: docs (tracking T-126/T-127/T-128, ADR-0410/0411/0413) + U3b. U4 uncommitted in the main tree (QA fingerprint HEAD=e866ffc2 TREE=3425826c2ef3baeb11a3aef9aeef724e618fba9a).
- Old U4 worktree .claude/worktrees/agent-aa9b381c25983f83e is stale (merged) — can be removed later.

## Pending user questions (ask one at a time)
1. Main window closed to the tray: count as "app absent" → OS dialog? (recommend yes; reviewer hold F2)
2. T-128 (pre-existing race, immediate send answers pending): fix the test now or kernel later?
3. Directory copy row cap (reviewer hold; app copy no longer carries rows, so impact is memory only).
4. Accept U4 QA PARTIAL (GUI focus-order item) or re-run when unlocked.

## Decisions this session (user)
- Approval route = app attached with a PC list slot open, else OS dialog; no visibility/focus test, no 30 s fallback · 「그냥 앱이 있냐없냐만 구분하면 되지 않음?」「묻히는건 알아서 유저가 해야지」 → ADR-0410 (Claude narrowed to "a slot open").
- Taskbar flash later, tray launcher as separate topic · 「일단 작업표시줄 깜박임으로 한다고 치고 당장은 하지마」 → T-126 (merged T-81, T-15).
- Directory snapshot one frame, 4 MiB cap, no own split · 「iroh로 보낼때는 그쪽룰로 하면 되는거 아님?」 → ADR-0411.
- Local app↔daemon stays WS; remote/mobile client over iroh later → T-127 + docs/research/remote-client-carrier-2026-11-23.md.
Claude-decided: over-cap directory sent cut with a warning; Mail text > 512 KiB refused before queueing (ADR-0413); app copy without agent rows.

## Verification
- U3b QA: workspace 5997 passed, gates 32/32, frontend 2948, claude lane 4/5 (T-128 flake, pre-existing), mutation 2/2.
- U4 merged QA: automated stages, claude lane, mutation, GUI (a)(b)(d) PASS; (c) not run (session locked).

## Feedback to record (not yet)
- qa binding: core.hooksPath literal check rejects an absolute path to the same hook → forces isolated-copy mutation builds (qa.md fix direction: accept a path resolving to scripts/git-hooks).
- Previous handoff claimed round-3 edits were on disk; they were not (coders cut off before editing) — handoff save should measure, not infer.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -3 && git -C I:/Engram/apps/engram-dashboard status --short | head`
