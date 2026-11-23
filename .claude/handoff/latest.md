# Handoff: S37 — U3b and U4 committed and pushed; CI running; four user questions open

## Know first
- Mode: autonomous. Never use coined labels in user-facing text; Korean, short.
- U5 (two-PC test) is the user's, at home.

## One-line state · next first action
Branch v0.5.0/feat/multi-daemon @ d5b516f3 pushed (U3b e866ffc2, U4 d5b516f3, ADR-0410/0411/0413, tracking T-126..T-130). CI run 45199033570 was in progress (gates + frontend green, backend running) when this was written — check it: `gh run view 45199033570 --json status,conclusion,jobs`.
**Next:** if CI green, S37 is left with U5 (user's two-PC test) and the open questions; merge to master only after U5 and with the user's go (master push = ask first).

## Repo state (measured 20261124-001357)
- HEAD d5b516f3 = origin (this handoff commit comes after). Untracked: .claude/explain/ (old, leave).
- Stale worktree .claude/worktrees/agent-aa9b381c25983f83e (U4, merged) — remove with `git worktree remove` when convenient (ask first: deletion).

## Open user questions (one at a time)
1. T-129: main window closed to the tray = "app absent" → OS dialog? (recommend yes)
2. T-128: pre-existing race (immediate send answers pending) — fix the test now or kernel later?
3. T-130: row cap on directory copies.
4. Unverified (docs/unverified-tests.md): WebView2 activating-click order for the click-time hold — run when the desktop is unlocked.

## Verification
- U3b QA: workspace 5997, gates 32/32, frontend 2948, claude lane 4/5 (T-128), mutation 2/2.
- U4 QA (merged): workspace 6030, gates, frontend 3031, claude lane, mutation, GUI (a)(b)(d) PASS; (c) partly not run (session locked).

## Decisions this session
(see history/20261123-224608-s37-u4-merged-qa-partial.md — unchanged) plus Claude-decided: U4 committed with QA PARTIAL by the earlier user rule (record PARTIAL items in docs/unverified-tests.md); review holds recorded as T-129 / T-130.

## Feedback to record (not yet)
- qa binding: core.hooksPath literal check rejects an absolute path to the same hook → isolated-copy mutation builds every time.
- Handoff save: measure the tree; the previous handoff claimed round-3 edits on disk that did not exist.
- Main: `git commit -- <untracked path>` fails; add first (cost one failed handoff commit).

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -3 && gh run list -w ci.yml -b v0.5.0/feat/multi-daemon -L 1`
