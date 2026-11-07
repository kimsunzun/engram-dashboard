# Handoff: stage 3 transport — 3-2 U3a + 15 s auth landed · U3b in review round 2 (uncommitted, coder running)

## Know first
- **In flight — this document is older than its outputs:** the U3b coder (round 2 = recommended items G1–G5, `.claude/handoff/attachments/20261106-s21-3-2-u3b-review/fix-r2.md`) was running when this was written. Its edits land in the working tree; its report arrives in the old session only. In a new session: inspect `git diff` against `beaf8f31`, then re-run review/QA as below.
- **Pending user question (blocks the U3b commit):** codex hit its usage limit (resets **2026-11-07 20:55**). Asked the user: A ★ = finish U3b with the two Claude reviewers, label it "단독 family 리뷰 — cross-family 교차검증 없음", re-run codex on the final U3b after the reset · B = hold the U3b commit until the reset. **No answer yet.**
- **Pending user approval (feedback, not code):** convention-first rule in three places (user-facing-rules §결정 규율 · implement TRD 「출처」 column · review checklist "unrecorded basis for constants/policies") — recorded in Engram `core/claude-global-shared/references/global-feedback.md` (`b9c37dc`), state 미반영; user asked only to record it.
- User irritation pattern this session: proposing options/values without checking open-source convention first (4 times — log language, band close, token retry as a workaround, 1 s auth). Check convention (source-reading OSS) before proposing user-visible behaviour/defaults; say "관례 미확인" if not checked. Keep user replies short (「간결하게」).
- Project rules unchanged: new docs/comments English; full regression before commit (ADR-0340); delete = `bash ~/.claude/references/recycle.sh`.

## One-line state · next first action
Branch `v0.4.1/refactor/transport-attach`, HEAD `beaf8f31` (+ this handoff commit), **84 commits ahead of origin, not pushed**. Committed this session: U3a `a835aeb3` (new driver core, test-only), 15 s auth `3e3d2147` + ADR-0347 `beaf8f31`/`59209a45`, ADR-0345 `75346d1e`/`d5fc68f5`, ADR-0346 `bc9ed7b5`/`f499f0fa`, TRD revision 9 `695845de` (+ later edits in `3e3d2147`), research ×5, handovers.
**Next first action:** get the user's A/B answer on codex; when the U3b coder's round 2 is in, snapshot the tree, send the incremental diff to the lead reviewer only (risk-axis lens used its 2 rounds; codex per the answer), then `/engram-qa` (full regression + mutation; GUI N/A — test-only code), batched doc fixes by main (`crates/engram-dashboard-transport/src/lib.rs:26-27` op list + `drop_link_in`; `replay_flight.rs` "no rollback entry" comment → note the sanctioned `on_refused` use), commit U3b. Then U3c (fresh coder — memo §2 U3c + §3 seam in `.claude/handoff/attachments/20261105-s21-3-2-u3a-handover-for-u3b.md`), then U4.
Progress: 3-0 all · 3-1 all · 3-2 U2 · U3a done; U3b in review; U3c/U4/U5/U6 · 3-3 not started.

## repo state (measured at save)
- Uncommitted (U3b, **review round 2 pending — 게이트 미통과, do not trust as reviewed**): `crates/engram-dashboard-transport/src/peer.rs` (new `Peer::drop_link_in` +97), `src-tauri/src/daemon_client/{connection.rs, connection_v2.rs, connection_v2/tests.rs, link/route.rs, mod.rs, tests.rs}`.
- Snapshot trees: U3b round 0 `c51461c7`, round 1 `32bc7378`.

## Verification state
- U3a `a835aeb3`: review deep 2 rounds (lead/risk PASS; codex must withdrawn after re-ask) · QA PASS (workspace 5430/0/35, lib_unit 1434/0/3, gates 34/34, frontend 2802+tsc, mutations killed).
- 15 s `3e3d2147`: review deep must 0 · QA: all required green; extra lane stop_smoke T-52 = FAIL(known failure) (user standing condition → proceed). GUI not run (only the no-auth branch changed).
- U3b: round 0 FIX (3 musts, all on "drop the connection") → round 1 lead PASS, risk-axis PASS (must 0), codex not run (limit) → round 2 in flight. Coder's round-1 run: lib_unit 1449/0/3, transport 383 (+ all-features green). QA not run.
- Rerun: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<path>.log"`; driver only: `cargo test -p engram-dashboard --test lib_unit daemon_client::connection_v2`.

## Decisions
User-decided:
- (묻지 않음) Log language · 상황: logging-conventions said Korean, new code English · 선택지 없음 · 고른 것 「하드코딩은 무조건 영어지」 · 「이거 나중에 한꺼번에 바꿀꺼니깐 그냥 그렇게 알면됨」 · 추천과 — · 조건: existing Korean logs converted later in one batch.
- Reconnect button exists · 상황: today permanent Down band, restart only · A keep today ★ / B build now · 「재연결 버튼이 있어야지」 · 갈림.
- Reconnect button placement (ADR-0345) · band ★ / popup · 「일단 음.. 띠에두자. 연결안되도 클라는 조작되게.」 · 같음 · 근거(research light).
- Pressing reconnect resets the count · 「ㅇㅇ 버튼을 누르면 횟수를 처음부터 다시셈」 · 같음.
- Network failures retry without limit (ADR-0346) · after medium research (6 apps from source) · A retry forever ★ / B bounded + tray · 「그냥 계속 재시도하는게 구현적으로 편한거지?」 then 「ㅇㅇ 쭉 진행해」 · 같음.
- Token refusal not retried (ADR-0346) · 상황: TRD D20 ① retried token refusal to cover a race · A retry once ★ / B stop at once · 「토큰 거절을 재시도하는건 그냥 버그 수정때문에 땜빵채운다는거잖아 … 뭐하는짓이야」 · 갈림 · code read showed the race yields a connection failure (daemon `bind 127.0.0.1:0`, token per start; dialer reads port+token from one fresh daemon.json read).
- Auth window 15 s (ADR-0347) · 상황: 1 s from S12 draft with no reason; OSS defaults 3–60 s · 「ㅇㅇ 15초로 올려」 · 같음.
- Pre-auth exposure accepted (ADR-0347) · A accept ★ / B cap net daemon now · 「a로 하고 계속 진행해」 · 같음.
- Flap retries stay at base delay (TRD rev 9 ⑨) · proposed keep per convention (5/7 reset on connect); user did not object (not an explicit answer).
Claude-decided:
- U3 split U3a/U3b/U3c (file overlap; coder seam).
- Shell handshake `Error` frame = terminal refusal (net daemon closes without a code; else infinite retry) — implements ADR-0346 decision 2.
- Cold start: show Down at once and keep retrying in background.
- U3b: "drop the connection" = new transport `Peer::drop_link_in(generation, cause)` through the link-lost path (keeps backoff, generation-checked) — synthesis of three reviewers' prescriptions; rejected `reconnect_now` (no backoff, unbound).
- U3b: explicit connect/ensure re-add failure shows Down (parity with old driver `mod.rs:602-631` + rev 9 ①); rejected "keep Reconnecting".
- U3b: follow-up Subscribe checks window targets (fixes an A1-shaped leak present in both drivers; old driver left as is until U4).
Open:
- Codex A/B (above). Band text for 4001 (every handshake refusal shows the version text today) — decide before U5. Whether a closed band re-shows on the next failure — before U5. Convention-first rule approval.

## Do-not / lessons
- Don't propose user-visible behaviour/values without checking OSS convention (source reading beats help-center docs — the doc-only collector came back empty).
- Don't offer workaround policies for a bug as options; fix the cause as a separate item.
- Don't invent names in user text ("옛 데몬" confused the user — say "지금 운영 중인 데몬(net crate)").
- Codex rescue runs >600 s must run in the background and be awaited.

## Open notes
- TRD R11 says to record the outcome in `docs/tracking.md` — no such entry exists (pre-existing gap).
- QA binding improvements to record: mutation-copy CRLF vs LF hash compare; `tail -f` Monitor left `tail.exe` holding the temp folder.
- Carried from earlier handoff: tracing capture helper dedupe (three other copies), step-log entries for 3-1 U5/3-2 U2, ADR-0272 landing notes, `logging-conventions.md:32` stale (Korean messages).
- Improvement feedback filed this session (Engram repo): research light too thin for app UI (`67866ed`), OSS source-reading first for app UI (`af17841`), convention-first (`8df2d73`, `432645f`, `b9c37dc`).

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -8 && git -C I:/Engram/apps/engram-dashboard status -sb && git -C I:/Engram/apps/engram-dashboard diff --stat beaf8f31`

## References
- TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` line 24 (revision 9 ①–⑨), §6-2 U3 row.
- ADRs 0344–0347. Research: `docs/research/{banner-action-buttons,reconnect-give-up-ui,disconnect-banner-dismiss-reconnect}-2026-11-05.md`, `reconnect-backoff-after-flap-2026-11-05.md`, `first-auth-frame-timeout-2026-11-06.md`.
- U3b map: `.claude/handoff/attachments/20261105-s21-3-2-u3a-handover-for-u3b.md`; U3b review artifacts: `.claude/handoff/attachments/20261106-s21-3-2-u3b-review/`.
- Code: `src-tauri/src/daemon_client/connection_v2.rs` (`Driver`, `drop_connection`, `write_follow_up`), `link/route.rs`, transport `peer.rs` `Peer::drop_link_in`.
