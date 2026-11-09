# Handoff: S21 stage 3 — 3-2 U5a landed (stop path on transport) · next = T-82 answer, then U5b-1

## Know first
- **Mode: autonomous.** User is at work during the day: **no GUI QA while the user works** (「나 업무중이야 테스트는 나중에 해야될듯」) — a GUI run moves the mouse. Run GUI stages only when the user says so.
- **Every worker brief that re-dispatches a stage must restate its items** — a resumed QA worker lost its original GUI items once.
- **QA moves the dev data folder aside during GUI runs.** If you stop a QA worker mid-GUI, restore `.engram-dev` yourself. Leftover from this session: untracked `I:\Engram\apps\engram-dashboard\.engram-dev.qa-bak-engram-qa-20261109-111846\` — `.engram-dev` was already restored from it by robocopy (contents identical, diff-checked); the backup folder is locked by some process, delete it later with `bash ~/.claude/references/recycle.sh <path>`.
- **Session scratchpad was wiped** mid-session (cause unknown) — the v0.3.2 daemon build is gone. Rebuild only when needed (see QA binding line on `ENGRAM_OLD_DAEMON_EXE`).
- User patterns (this session): plain words, no invented labels or shortened UI labels as verbs (「끄기가 뭐야?」, 「이상한 단어 쓰지 말고」 at "옛 데몬" — say "앱보다 이전 버전의 데몬"); every convention claim cites its source in the same sentence; if local research has nothing, run a light web check before writing 「관례 미확인」 (now in CLAUDE.md via master `77162ebb`); keep UI texts simple (「너무 디테일하게 하지는 말아」); explain process concretely when asked (who calls what, what happens step by step).
- Project rules unchanged: comments/docs English; commit subject Korean `S21: …`; full regression before commit; delete = `bash ~/.claude/references/recycle.sh`.

## One-line state · next first action
3-2 U5a is committed (`b85c0aaa`): the shell owns one stop path on transport (`daemon_client/link/stop.rs` + `daemon_client/shutdown.rs`), the daemon's StopDaemon shutdown can no longer be cancelled by a connection abort, the frontend's old stop (`DaemonControl.stop()`/`AgentClient.stopDaemon`) is removed. Left in 3-2: **U5b-1** (shell: band status DTO, refusal slot token vs version, routing R7/R20, `daemon_shutdown` command) → **U5b-2** (frontend band buttons/confirm/busy, `daemon.stop`/`connection.*` commands, boot start main-window only, remove `daemon_stop`) → **U6** (N-daemon seat; give the user the structure lecture first) → 3-3 (daemon on transport; T-80 there).
**Next first action = get the user's answer on T-82** (question below), record it in `docs/tracking.md` T-82, then start U5b-1 via `/engram-implement` (design = `docs/process/S21-crate-boundaries/trd-3-2-u5-stop-and-band.md` §4/§6 U5b-1, review already passed — go to the start question/coding).

## Repo state (measured 2026-11-09 12:24)
- `I:\Engram\apps\engram-dashboard` — branch `v0.4.1/refactor/transport-attach`, HEAD `2d64b4e9`, **ahead 10 / behind 0** vs origin branch (not pushed; the handoff commit comes after this text). **origin/master is 2 ahead of this branch** (`77162ebb`/`7a8f297d` — CLAUDE.md convention-source rule, chore/misc) — pull master into the branch before the next integration.
- Untracked: `.engram-dev.qa-bak-engram-qa-20261109-111846/` (see Know first). Bindings show CRLF-only modifications sometimes after tests (`git diff` empty) — harmless.
- Master integrations this session: `9b606dfc` (transport-attach 3-0/3-1/3-2 U2–U4c), `baa97e41` (S34 PTY kill wedge, T-79). Branch pulled master at `651ca5c3`, `7354b9e0`.
- `I:\Engram\apps\engram-dashboard-wt4` (S34 branch) merged; can be removed when convenient.
- Engram repo (feedback): `97ef6a0`, `6b6fbe3`, `12f3cda`, `8f5a79f` (not pushed).

## Verification state
- U5 detail TRD: PRD+TRD review **deep, 4 delta rounds → PASS** (edition 5.1, `d41b15f2`).
- U5a code: review **deep (Claude lead · Claude risk axis · codex) PASS** after 3 fix rounds; the frontend-stop removal got a light review + wording fixes.
- U5a QA (tree before the frontend removal): build PASS · full regression PASS (Rust 5455 · gates 35/35 · frontend 2804 · tsc) · shell real-daemon lane `stop_smoke --ignored` 4/4 incl. a v0.3.2 daemon · mutation 2/2 caught · `ws_e2e ignored_daemon_kill_cleans_pty_child` = known failure. After the removal: tsc clean, vitest 2799 passed (coder run), Rust comment-only changes `cargo check` OK, `stop_smoke` cross-version test now prints SKIPPED without the exe (ran it).
- **Not done: GUI smoke** (only (a) tray 「데몬 끄기」 passed — icon gray) → **PARTIAL**. Still to run when the user allows: (b) tray 「데몬 켜기」 → reconnects; (c) spawn 1–2 agents, tray 「종료」 → app exits in seconds, daemon + agent processes gone; (d) forced kill (suspend the daemon, tray 「데몬 끄기」 → killed after ~10 s, descendants gone) if provokable.
- Rerun full regression: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-detached.ps1 -Command "cargo test --workspace -- --test-threads=4" -WorkDir "I:/Engram/apps/engram-dashboard" -LogFile "<path>.log"` (full procedure = `.claude/skill-bindings/qa-verify.md`).

## Decisions
User-decided (this session):
- (묻지 않음) Pull master translation into the branch · 「번역관련해서 수정사항이 많아서 업데이트 당기고」.
- Merge transport-attach to master now · 상황: 3-2 shell side done, daemon side on net, same wire · 선택지 지금 ★ / U5·U6 뒤 / 3-3 뒤 · 고른 것 「ㅇㅇ 지금 그럼 일단 머징하자」 · 추천과 같음 · 근거: translation of the branch's files was blocked.
- Merge S34 to master · 선택지 지금 ★ / 나중 · 고른 것 「ㅇㅇ 진행 계속해」 (after asking what kill means) · 같음.
- ✕-closed band keeps today's behaviour, no tray item · 고른 것 「구지 뭘 추가할 필요는 없어보이는데?」 · 갈림 (Claude had proposed a tray 「다시 연결」).
- Token refusal band text = cause only 「데몬이 이 앱의 인증을 거절했습니다」 · 선택지 cause+hint ★ / cause only · 고른 것 「ㅇㅇ 이유만 짧게」 · 갈림.
- T-80 fixed inside 3-3 · 선택지 3-3 ★ / own topic · 고른 것 「A 안으로해.」 · 같음.
- Band after the first failed reconnect on a runtime loss · 고른 것 「끊기고 한번 실패해서 띄우는게 맞아보이는데」 · 같음 · 근거(Claude): Zulip `popup_banners.ts:39-47`, VS Code Remote 40 s dialog (docs/research/disconnect-banner-dismiss-reconnect-2026-11-05.md).
- 「다시 연결」 starts the local daemon if none runs · 고른 것 「A로 하자」 · 같음 · 조건: 「나중에 멀티데몬 할 경우에도 한개가 타겟이 되어야되는데 그 데몬을 붙이면 되겠지」 (multi-daemon PRD carry-over).
- Two bands only (「연결 안 됨」 / 「버전 다름」), simple texts · 고른 것 「ㅇㅇ 버전 다름 하고 연결안됨. 너무 디테일하게 하지는 말아」 · 같음 (Claude proposed the merge).
- After any user stop the band is just ① (stopped latch removed) · 상황: two review rounds of same-class findings · 선택지 ① ★ / no band + latch · 고른 것 「A로 해 문구는 너무 디테일하게 말고 심플하게」 · 같음.
- Remove the frontend `DaemonControl.stop()` now (closes U5a hold 1) · (묻지 않음, user proposed) 「옛 함수인데 그냥 나중에 어차피 수정될거니깐 없애는게 좋지 않음?」 → 「ㅇㅇ 그렇게 해」 · 조건: LLM has no supported stop surface until U5b-2.
- Cross-version smoke test skips without the v0.3.2 exe (U5a hold 2) · 선택지 skip ★ / always require · 고른 것 「A」 · 같음.
- Tray split from the app: later, keep as is now · 「트레이는 지금 클라에 종속되어있는데 분리해야되긴함. 일단은 이대로 하고.」 → T-81.
- (묻지 않음) Autonomous run · 「쭉 진행해」 / 「쭉 진행해 나 외출」.
Claude-decided: 10 s graceful deadline then identity-checked kill (docker stop / supervisord 10 s); nothing sent → kill at once; Exit never kills and waits ≤3 s for the daemon to close; app exit stops an older daemon too (ADR-0026); LLM `daemon.stop` without confirm (D7); boot start main-window only + boot-pending band hold (R17/R20); single flight + bounded stopping mode; daemon StopDaemon shutdown in a spawned task; one clock; net test restructured instead of widening `net gate 2a`. All in ADR-0350 + TRD §7.
Open (ask the user):
1. **T-82** — daemons from v0.2.0–v0.3.2 write `daemon.json` at the data root; the current app does not see them → a second daemon starts, empty agent list, the old one keeps running unseen (investigated: code + local experiment). Affects only same-folder in-place updates while a daemon runs; downloads of those versions total 10. 선택지 A detect legacy record + band / B daemon-side probe / **C ★ accept + release-note line** ("before updating in place, use tray 「데몬 끄기」"). The user asked what C means; got the explanation; **no answer yet**.
2. (carried) U6 needs the structure lecture before starting.

## Do-not / lessons
- Don't run GUI QA while the user is working.
- Don't re-dispatch a stage with "per your original brief" — restate the items.
- A fix that adds a lock/latch to suppress a symptom kept spawning same-class review findings (U5 latch) — when a class repeats two rounds, raise the class to the user early.
- `pid_alive_with_start_time` now checks the exit code — net crate may use only its 5 allowed platform symbols (`net gate 2a`).

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -6 && git -C I:/Engram/apps/engram-dashboard status -sb | head -3 && git -C I:/Engram/apps/engram-dashboard rev-list --left-right --count HEAD...origin/master`

## References
- Design: `docs/process/S21-crate-boundaries/trd-3-2-u5-stop-and-band.md` (§4 bands/DTO/commands, §6 units U5b-1/U5b-2, §7 decisions, §9 risks); band situations page `docs/process/S21-crate-boundaries/spec-brief-3-2-u5.html`; ADR-0350.
- Code: `src-tauri/src/daemon_client/shutdown.rs` (Stopper, StoppingMode, stop_flight), `link/stop.rs` (send_stop, StopHandshake), `connection.rs` (`ensure`/`connect` vs stopping mode, `LinkControl`), `src-tauri/src/tray/{mod,actions}.rs` (`TrayDaemonClient`), `src/components/layout/ConnectionNotice.tsx` (band today), `src-tauri/src/daemon_client/link/route.rs` `refusal_message`.
- Tracking: `docs/tracking.md` T-80, T-81, T-82; step-log S21 stage-3 entries 2026-11-08/09.
