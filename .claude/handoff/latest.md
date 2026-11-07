# Handoff: stage 3 transport — 3-2 U3c · U4a · U4b · U4c landed; T-79 fixed on S34 branch · next = user decisions, then 3-2 U5

## Know first
- Nothing is in flight. Both worktrees clean. **Nothing pushed** this session (user decides push/merge).
- Three user decisions are pending (see "Decisions → Open"); stop at U5 until they are answered.
- User patterns this session: (1) check OSS convention / do a light research BEFORE asking a user-visible text/behaviour question — the user corrected me once (「일단 리서치 간단하게 하면 되는데」); (2) plain words in questions, no internal names (「뭔소리야」 at an `ensure` question); (3) do not spam "still running" replies — a worker that backgrounds long runs and ends its turn floods the main with interim notifications (user: 「너 무슨 메세지를 계속뿌려 일부로 토큰낭비한거?」). **Put in every worker brief: "if you background a long command, do not end your turn — wait in place until it completes (Monitor until-loop / foreground); report once."** Filed Engram `cdefe31`/`2ce8114`.
- Auto-handoff prompt is turned off for this session's id only (`~/.claude/state/auto-handoff/31fa411b….off`).
- Project rules unchanged: new comments/docs English (touched paragraph rewritten whole in English); commit subject Korean `S<n>: …`; full regression before commit; delete = `bash ~/.claude/references/recycle.sh`; user standing condition: QA known failures only (T-52 stop_smoke · ws_e2e ignored_daemon_kill_cleans_pty_child) → proceed.

## One-line state · next first action
3-2 shell attach: U2 · U3a/b/c · U4a/b/c done — production shell now runs on the transport link driver (`daemon_client/connection.rs` + `link/`); old driver deleted. Left in 3-2: **U5** (stop client on transport `link/stop.rs` + band buttons ADR-0344/0345) and **U6** (N-daemon seat — give the user the network/structure lecture first). Then 3-3 (daemon on transport, net removed). Rough progress: ~70% by unit count, ~60% by weight; shell side ~90%, daemon side 0%.
**Next first action = ask the user the open decisions** (one at a time, plain words, convention checked first), then U5 via `/engram-implement`.

## Repo state (measured at save)
- `I:\Engram\apps\engram-dashboard` — branch `v0.4.1/refactor/transport-attach`, HEAD `73031727`, ahead 93 / behind 0 vs origin (before this handoff commit), clean.
  - This session: U3c `0d4f79d1` · U4a `7f841624` · T-79 tracking `31919dcb` · U4b `5257314f` · tracking T-79 cause/T-80 `269a24ba` · U4c `73031727`.
- `I:\Engram\apps\engram-dashboard-wt4` — branch `v0.5.0/fix/pty-kill-hang` (topic **S34**, from master `1da73635`), HEAD `75de8476`, 5 commits ahead of master, clean, **not on origin** (S34 number not claimed by push yet).
- Engram repo (feedback only): `c9c2a38`, `68772a1`, `cdefe31`, `2ce8114`.

## Verification state
- U3c: review deep 3 fix rounds (gate redesign prescribed by main) → PASS; QA PASS (workspace 5477/0/35, mutations 2/2).
- U4a: review deep PASS (codex 16 MiB must withdrawn — tungstenite default `max_frame_size` already 16 MiB); QA full **PARTIAL**: R7 real-socket checks NOT run (no shell↔daemon real-socket harness exists), tray click emulated (session locked), real other-version daemon not produced. GUI ①②④ + cold start + new ensure text passed.
- U4b: review full PASS (2 rounds); QA PASS (workspace 5401/0/35, gates 34/34, mutations 2/2, GUI smoke; every one of the 97 deleted tests accounted for — fate list in `.claude/handoff/attachments/20261107-s21-3-2-u4-review/u4b-coder-report-with-test-fate.md`).
- U4c: review deep PASS (1 fix round; secret-exposure lens tried 11 paths — closed); QA: only FAIL = known T-52 stop_smoke; all else PASS (token value 0 hits in all WebView paths).
- S34: PRD+TRD review full 3 rounds PASS; code review deep PASS; QA PASS (AC1 base 5/5 fail → fix pass < 1 s; GUI kill mid-burst reply 14–161 ms; base reproduces the wedge).
- Not run anywhere: clippy (component not installed for toolchain 1.95.0).
- Rerun: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<path>.log"`.

## Decisions
User-decided (this session):
- Failed-ensure text when no daemon · 상황: U4a switch changed it from an order ("connect 필요") to cause-only · 선택지 옛 문구 ★ / 새 문구 → after light research (Docker/kubectl/Tailscale = cause + question hint) · 고른 것 「ㅇㅇ 그렇게가」 (cause + "— is the daemon running?") · 갈림 · 근거: 「클라를 켰는데 데몬이 없다고 데몬을 키라는게 이상한데?」 · 조건: multi-daemon later needs a "no connection chosen" state — 「접속이 없는 상태가 있어야 될것같아」 (left to the multi-daemon PRD; `docs/research/client-without-daemon-message-2026-11-07.md`).
- (묻지 않음) Commit · 「커밋이야 그냥 너가 알아서하고」.
- (묻지 않음) Autonomous run while away, no handoff mid-way · 「쭉 진행해. 완료되면 브리핑해주고. 핸드오프 하지말고」.
- (묻지 않음) Comments in English confirmed (user asked; current practice matches).
Claude-decided:
- U3c gate design (one leaf mutex: opened + pending fire-and-forgets) replacing the counter/hold-back after round-2 findings; dropping/closed-tail daemon CommandRequests skipped (daemon ends them at its 7 s deadline as TIMEOUT).
- U4 split U4a/U4b/U4c; request timeout 30 s (D15 ①); module rename `connection_v2` → `connection`; `ConnectionState::Connecting` and `protocol_state::PendingMap` removed as dead.
- T-79 fixed on a separate topic S34 (branch rule: one topic per branch, from master): Windows-only drain via platform fact `CLOSE_NEEDS_DRAIN`, daemon Kill on `spawn_blocking`; bounded-wait + Error reply rejected (no measured timeout value).
- U4c: `WsTransport` keeps an explicit `authToken` constructor option (dev only) instead of losing Auth entirely.
Open (ask the user):
1. Push + merge S34 (`v0.5.0/fix/pty-kill-hang`) to master? (push → CI green → `--no-ff`, checkout-less merge per `docs/handbook/branch-commit.md`). Recommendation: yes — it fixes a wedge users hit today.
2. Before U5: band text for a 4001 refusal (today every handshake refusal shows the version text); whether a band the user closed re-shows on the next failure.
3. T-80 (spewing agent in a slot → daemon closes on full send queue → reconnect storm every ~20–35 ms; pre-existing): fix within 3-3 (daemon on transport, byte budget) or as its own topic?
- Also pending from earlier: convention-first rule approval (Engram `b9c37dc` + 5th occurrence `68772a1`).

## Do-not / lessons
- Don't hand a concurrency/ordering fix to the coder as a goal only — prescribe the mechanism (U3c cost a round). Filed `c9c2a38`.
- Don't run two GUI QAs at once (single instance / ports) — serialize them.
- QA workers recycle their temp folder; tell them to copy failure/hang evidence into the scratchpad first.
- A synthetic `subscribeOutput` view gets no output unless a real slot shows the agent (QA note).

## Open notes
- U4a QA PARTIAL items stay open: R7 real-socket harness (TRD §6-2 U4 row) — consider it with 3-3.
- `/engram-adr link` S34 → ADR-0001 (premise refined) — after S34 merges.
- Remaining residues recorded in code: `write_refresh_subscribe` Err branch untested; a close with no live pump can still block (S34 TRD §3B).
- Carried: tracing capture helper dedupe; step-log entries for 3-2 U2–U4c on the S21 branch; ADR-0272 landing notes.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -8 && git -C I:/Engram/apps/engram-dashboard status -sb && git -C I:/Engram/apps/engram-dashboard-wt4 log --oneline -3`

## References
- TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` §6-2 U5/U6 rows, §7; ADRs 0344–0347, 0310.
- U4 review/QA artifacts `.claude/handoff/attachments/20261107-s21-3-2-u4-review/`; U3c `20261106-s21-3-2-u3c-review/`; U4 handover memo `20261106-s21-3-2-u3c-handover-for-u4.md` (U5 notes in §2/§3).
- S34: `I:\Engram\apps\engram-dashboard-wt4\docs\process\S34-pty-kill-hang\{prd,trd}.md`.
- Tracking `docs/tracking.md` T-79 (fixed on S34 branch), T-80.
- Code: `src-tauri/src/daemon_client/connection.rs` (driver), `link/route.rs`, `mod.rs`; stop client today `daemon_client/stop.rs`.
