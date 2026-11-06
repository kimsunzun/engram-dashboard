# Handoff: stage 3 transport — 3-2 U3a · U3b landed, 15 s auth landed · next = 3-2 U3c

## Know first
- Nothing is in flight. Working tree clean; all review/QA gates passed for what is committed.
- **Pending user approval (feedback, not code):** convention-first rule in three places (user-facing-rules §결정 규율 · implement TRD 「출처」 column · review checklist "unrecorded basis for constants/policies") — Engram `core/claude-global-shared/references/global-feedback.md` (`b9c37dc`), 미반영. User asked only to record it.
- User pattern this session: irritated when options/values are proposed without checking open-source convention (4 times). Check convention by reading OSS source before proposing user-visible behaviour/defaults; say "관례 미확인" if not checked. Keep replies short and in plain terms (the user wants "what is being done" in one line, not unit codes — 「u3b라고하면 나는 진도밖에 모르는데」).
- Auto-handoff hook (context 55%): save only at a real work boundary (commit / nothing running), not right after dispatching a subagent (user, filed `65115ad`).
- Project rules unchanged: new docs/comments English; full regression before commit (ADR-0340); delete = `bash ~/.claude/references/recycle.sh`. User standing condition: QA known failures only (T-52 stop_smoke · ws_e2e ignored_daemon_kill_cleans_pty_child) → proceed.

## One-line state · next first action
Branch `v0.4.1/refactor/transport-attach`, HEAD `db6ae571` (+ this handoff commit), **86 commits ahead of origin, not pushed**.
Committed this session: U3a `a835aeb3` (new driver core, test-only) · 15 s auth `3e3d2147` + ADR-0347 · U3b `db6ae571` (subscribe/replay/output relay on the new driver + transport `Peer::drop_link_in`) · ADR-0344/0345/0346 · TRD revision 9 (①–⑨) · research ×5.
**Next first action = 3-2 U3c** with a fresh coder (in plain terms: the new connection code also handles commands the daemon sends to the shell, registers the shell's own commands on every connect, and carries the usage-limit display data). Map: `.claude/handoff/attachments/20261105-s21-3-2-u3a-handover-for-u3b.md` §2 "U3c" + §3 seam (file:line in `connection.rs` / `mod.rs`); U3b has landed so rebase notes in §3 are moot. Note from U3b review: a daemon `CommandRequest` on a connection being dropped (`dropping`) is currently skipped with a debug log — U3c should make that a deliberate choice. Same pipeline: brief → coder → `/engram-review code deep` (lead + risk-axis + codex) → `/engram-qa` → commit.
Then U4 (production switch — first user-visible change, GUI `/qa full`) → U5 (stop client + band buttons) → U6 (N daemons — **give the user the network/structure lecture first**) → 3-3.
Progress: 3-0 all · 3-1 all · 3-2 U2 · U3a · U3b done; U3c/U4/U5/U6 · 3-3 not started.

## repo state (measured at save)
- Clean working tree. HEAD `db6ae571`. ahead 86 / behind 0 vs origin (before this commit).

## Verification state
- U3a `a835aeb3`: review deep (2 rounds) PASS · QA PASS (workspace 5430/0/35).
- 15 s `3e3d2147`: review deep must 0 · QA all required green; extra lane stop_smoke T-52 = FAIL(known failure). GUI not run (no-auth branch only).
- U3b `db6ae571`: review deep 3 rounds + codex final PASS · QA PASS (workspace 5451/0/35, lib_unit 1452/0/3, transport 383/431, frontend 2802, gates 34/34, 3 mutations caught). GUI N/A (test-only). Not fixed (cap reached, recorded in the commit): deadline test single sweep + 50 ms sleep; close test one waiter; drop_connection info wording with no peer.
- Rerun: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<path>.log"`; driver: `cargo test -p engram-dashboard --test lib_unit daemon_client::connection_v2`.

## Decisions
User-decided (this session):
- (묻지 않음) Log language · hard-coded strings English · 「하드코딩은 무조건 영어지」 · 조건: 「이거 나중에 한꺼번에 바꿀꺼니깐 그냥 그렇게 알면됨」 (existing Korean logs later, one batch).
- Reconnect button exists · A keep today ★ / B build · 「재연결 버튼이 있어야지」 · 갈림.
- Button on the band (ADR-0345) · band ★ / popup · 「일단 음.. 띠에두자. 연결안되도 클라는 조작되게.」 · 같음.
- Pressing reconnect resets the count · 「ㅇㅇ 버튼을 누르면 횟수를 처음부터 다시셈」 · 같음.
- Network failures retry without limit (ADR-0346) · retry forever ★ / bounded + tray · 「그냥 계속 재시도하는게 구현적으로 편한거지?」→「ㅇㅇ 쭉 진행해」 · 같음.
- Token refusal not retried (ADR-0346) · retry once ★ / stop · 「토큰 거절을 재시도하는건 그냥 버그 수정때문에 땜빵채운다는거잖아 … 뭐하는짓이야」 · 갈림.
- Auth window 15 s (ADR-0347) · 「ㅇㅇ 15초로 올려」 · 같음. Pre-auth exposure accepted · A ★ / B cap now · 「a로 하고 계속 진행해」 · 같음.
- Flap retries stay at base delay (TRD rev 9 ⑨) · proposed per convention; no objection (not explicit).
- Codex was limited; user lifted the limit: 「아 codex 한도 풀었음. 진행하면 될듯.」 → codex reviewed U3b final (PASS), no degraded label needed.
Claude-decided:
- U3 split a/b/c; handshake `Error` frame = terminal (net daemon closes without code); cold start Down at once + background retries; "drop the connection" = generation-bound `Peer::drop_link_in` through link-lost path (synthesis of three reviewers; rejected `reconnect_now`); explicit connect/ensure re-add failure shows Down (old-driver parity; rejected "keep Reconnecting"); follow-up Subscribe window check in the new driver only.
Open (before U5): band text for 4001 (today every handshake refusal shows the version text); whether a closed band re-shows on the next failure. Convention-first rule approval.

## Do-not / lessons
- Don't propose user-visible behaviour/values without checking OSS convention (source reading beats help-center docs).
- Don't offer workaround policies for a bug as options; fix the cause separately.
- Don't invent names in user text ("옛 데몬" confused the user).
- Codex runs > 600 s go to the background and are awaited; codex usage limits happen — ask the user (§0-2), don't substitute silently.

## Open notes
- TRD R11 says to record its outcome in `docs/tracking.md` — no such entry (pre-existing gap).
- QA binding improvements to record: mutation-copy CRLF vs LF hash compare; `tail -f` Monitor left `tail.exe` holding the temp folder; brief nit — cargo accepts one TESTNAME before `--`.
- Carried: tracing capture helper dedupe (other copies); step-log entries for 3-1 U5 / 3-2 U2 / U3a / U3b; ADR-0272 landing notes; `logging-conventions.md:32` stale (Korean).
- Feedback filed this session (Engram repo): `67866ed`, `af17841`, `8df2d73`, `432645f`, `b9c37dc`, `65115ad`.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -8 && git -C I:/Engram/apps/engram-dashboard status -sb`

## References
- TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` line 24 (revision 9), §6-2 U3 row.
- ADRs 0344–0347. Research `docs/research/*-2026-11-05.md`, `first-auth-frame-timeout-2026-11-06.md`.
- U3c map: `.claude/handoff/attachments/20261105-s21-3-2-u3a-handover-for-u3b.md`; U3b review artifacts `.claude/handoff/attachments/20261106-s21-3-2-u3b-review/`.
- Code: `src-tauri/src/daemon_client/connection_v2.rs`, `link/route.rs`, old driver `connection.rs`, transport `peer.rs` `Peer::drop_link_in`.
