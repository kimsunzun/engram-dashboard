You are a subagent — this brief is the source of truth. You act as the QA runner (`/engram-qa` executor) for stage 3-2 U3c of `I:\Engram\apps\engram-dashboard`, branch `v0.4.1/refactor/transport-attach`.

## Read first (must)
- QA skill procedure: `C:\Users\kimsunzun\.claude\skills\engram-qa\references\flow.md` (§2 execution, broken-test triage, mutation check, §3 failure handling, §4 verdict + evidence, §5 cleanup).
- Project binding (commands, full-regression definition, known failures, isolated copies, temp folder, cleanup): `.claude/skill-bindings/qa.md`, and `.claude/skill-bindings/qa-verify.md` section 「전체 회귀 — CI 세 잡 로컬 실행」.

## Target
- Review base point (commit) = `1d3a8486` (the commit just before this task's first change). The change is uncommitted in the working tree; reviewed snapshot tree `f27b804bed960c087db9acf545078461d75699c0` plus main's comment-only edits after review (connection_v2.rs doc/log wording, `mod.rs` `SharedUsageInterest` doc). Do not modify the working tree (except restoring byte-identical `src-tauri/bindings/*.ts` that a test run rewrites — command in the binding / memo).
- Changed files: `crates/engram-dashboard-transport/src/peer.rs` (doc + 1 test), `src-tauri/src/daemon_client/connection_v2.rs`, `connection_v2/tests.rs`, `mod.rs`, `daemon_client/tests.rs` (visibility only). No untracked files.
- What changed (coder summary): the new shell connection driver `connection_v2` (compiled only under `#[cfg(test)]`; production still runs the old driver) gained inbound daemon command handling with outcomes bound to the arrival connection, command registration on every connect, usage interest (open/lost/nudge/snapshot/⟳ pre-subscribe), an "opened" gate so no caller command reaches the wire before a connection's on-open writes, fire-and-forget pending list bounded at the old channel bound, close-epoch checks; transport `Peer::request_in` doc + test "first poll queues the request".
- Completion criteria: full regression green (except the binding's known failures); 25 new shell tests + 1 new transport test pass; name diff vs base: lost 0.

## What to run (decided by main)
1. ① Full regression (binding row "Rust source" / "transport" → 전체 회귀 = the three CI jobs locally, per qa-verify).
2. Name diff for shell `lib_unit` and the transport crate vs base `1d3a8486` (lost 0; added = 25 shell + 1 transport expected — verify).
3. Mutation check (tests were added): per flow §2, in an isolated copy, pick 1–2 changed production lines the new tests claim to cover — at least one in the `Gate` logic of `connection_v2.rs` (e.g. the caller's `gate.handle_closes < closes` arm or the `peer.generation() != bound` check in `opened_now`, or the pending drain's `pending == generation` branch) — break it, run only the covering test, it must fail; restore and confirm by hash.
4. ② Live run (GUI): main's decision = 해당 없음 — the new driver code is `#[cfg(test)]`; production callers of it = 0. Confirm mechanically: `rg -n "connection_v2" src-tauri/src --glob '!**/connection_v2/**'` shows only `#[cfg(test)]`-gated module declarations/branches, and `cargo build` of the release graph does not compile it (the binding's gate commands cover this if they exist). If you find a production path reaching the new code, stop and report — then GUI is required.
- Cost-bearing checks: the binding's list — run what the table requires; do not run claude/codex token lanes (not required by the table for this change).

## Rules
- Long runs via `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "<cmd>" -WorkDir "<dir>" -LogFile "<log>"`; temp folder per the binding (or `C:\Users\kimsunzun\AppData\Local\Temp\claude\I--Engram-apps-engram-dashboard\31fa411b-c656-4850-8c1d-2128bb9a19dd\scratchpad\u3c\qa\` for logs). Read completion markers/counts — a 0-test run is not a pass.
- Broken tests: triage per flow §2 (known-failure list first; otherwise base copy).
- Forbidden: editing code files (except the isolated mutation copy), committing, spawning subagents, writing progress records.
- Cleanup per §5 (remove worktrees with `git worktree remove`).

## Report
Verdict PASS / FAIL / PARTIAL; per stage (build · tests · extra checks/gates · mutation · live) result with evidence (commands, counts, log excerpts); broken tests with labels; name diff; cleanup done; most expensive step.
