# ADR-0350: Connection band has two kinds and the shell owns one stop path (3-2 U5)

- 상태: 확정 (2026-11-09, 근거: 사용자 결정 2026-11-08 U1 · U2 · 2026-11-09 D1 · D2 · U4 (「버전 다름 하고 연결안됨」 · 「A로 해 문구는 너무 디테일하게 말고 심플하게」) + main decisions D3–D11 · R1–R24 + TRD addendum `docs/process/S21-crate-boundaries/trd-3-2-u5-stop-and-band.md` edition 5.1 (PRD+TRD review PASSED, deep, 4 delta rounds))
- 관련: Amends ADR-0344 (결정 4와 5의 안내 문구) · Amends ADR-0025 (부팅 1회 ensure 범위와 죽은 데몬 되살리기 수단) · Amends ADR-0021 (결정 5 daemon_stop 명령과 영향의 stop 대체 순서) · Amends ADR-0343 (결정 3 강제 종료 시점) · Amends ADR-0271 (결정 5 정지 클라이언트에 더하는 강제 종료 대체와 10초 시한) · ADR-0342 (decision 2: decide from what the shell did, not the daemon's text) · ADR-0345 (reconnect action on the disconnected band) · ADR-0346 (refusals are terminal) · ADR-0287 (band layer and ✕ close — unchanged) · ADR-0026 (app exit) · ADR-0024 (daemon is detached) · ADR-0134 decision 4 (unwritable-folder remedy text) · `src-tauri/src/daemon_client/stop.rs` (today's stop client, deleted in U5a) · `crates/engram-dashboard-daemon/src/connection_core.rs` (`StopDaemon` arm) · `crates/engram-dashboard-platform/src/process.rs` (`pid_alive_with_start_time`) · `src-tauri/src/daemon_client/link/route.rs` (refusal text) · `src-tauri/src/discovery/mod.rs` (`daemon_stop`) · `src/components/layout/ConnectionNotice.tsx` · TRD `docs/process/S21-crate-boundaries/trd-3-transport.md` §6-2 U5 row and §6-4 ⑤ · step-log S21

## 맥락

3-2 U5 wires the stop client onto transport and gives the connection band its actions (ADR-0343 decision 5, ADR-0344 「영향」). Reading the code at the TRD base (`7354b9e0`) showed several gaps beyond the ADRs' own scope:

- The stop client (`daemon_client/stop.rs`, sync `tungstenite`) authenticates with the app's own version, so a mismatched daemon is `NoTarget`: tray 「데몬 끄기」 and app exit do nothing on a version mismatch.
- The daemon runs `shutdown_all` inside the connection's dispatch task. A client that closes while it runs cancels the await; the shutdown signal never fires and the daemon survives with its agents killed.
- Every 4001 refusal shows the version text, so a token refusal shows it too.
- A runtime loss shows no band (finding A); a failed boot `daemon_start` never calls `connect()`, so no shell link exists and a version refusal never reaches the shell (finding B).
- Every window, popouts and the prewarmed spare included, runs the boot `daemon_start` (against ADR-0021 decision 1 and ADR-0025 "once at boot").
- `pid_alive_with_start_time` compares creation time only, so an exited daemon whose process object is still held reads as live.

The parent TRD also carried a rule "intended disconnect → no band" (§6-4 ⑤). Keeping it after a stop needed a stopped latch, and review rounds 1 and 2 kept finding defects of the same class in that latch; main raised it to the user.

## 결정

1. **Two bands only.** The shell decides the band and sends `state` + `band` to the frontend; routing is the single emitter, deduped on `(state, band)` (D3, R6, R7).
   - **① 「연결 안 됨」** — on a cold-start failure, on a runtime loss after the first failed reconnect attempt (D1), on a token refusal, and after any stop. Text 「데몬에 연결하지 못했습니다.」 plus a cause: on a token refusal 「데몬이 이 앱의 인증을 거절했습니다」 (U2), otherwise the frontend start error `connectionError` while ① has no refusal cause (R14, R21). Button 「다시 연결」.
   - **② 「버전 다름」** — on any version mismatch. Text 「데몬에 연결하지 못했습니다.」 + 「실행 중인 데몬이 이 앱과 버전이 다릅니다」. Button 「데몬 끄기」 only when the stop applies: the daemon is older than the app and was not started after this app process (`can_stop`; the stale-exe and newer cases stay shell-internal). The button confirms first (ADR-0344 decision 2).
   - **After any user-initiated stop** (tray 「데몬 끄기」, LLM `daemon.stop`, band stop) **the band is the ordinary ①** — no "no band after an intended stop" rule, no special text, no stopped latch (U4). This supersedes parent TRD §6-4 ⑤ for stops.
   - No band: connected · an explicit frontend close · a runtime loss before the first reconnect attempt failed · a never-connected handle while the main window's boot start has not answered (R20).
   - Token vs version refusal is decided from what the shell did — a per-registration refusal slot written by the dialer before its version refusal — never from the daemon's text (D4, ADR-0342 decision 2).
   - Buttons live in the main window only; popouts show the band text only (R9). During a stop the shown band stays and the pressed button shows 「끄는 중…」; 「다시 연결」 shows 「연결 중…」 (R8).
   - ✕ memory is unchanged (U1): hidden while the same band kind and cause hold, cleared on connect. Nothing is added to the tray.
2. **「다시 연결」 starts the local daemon if none runs, then attaches** — the same as the boot start (D2, D9: `DaemonControl.start()`). A failed `daemon_start` still leaves an attach-only handle, so a version refusal surfaces as ② and a later daemon is attached (R15). Multi-daemon carry-over (user): the button targets one daemon; a remote daemon can only be attached, not started.
3. **One stop path in the shell** — `DaemonClient::stop_daemon(plan, origin)` serves the band, the tray, the LLM command, `DaemonControl.stop()` and app exit. Plan `User` (everything but exit) has a 10 s graceful deadline; plan `Exit` has 3 s and never kills. Every deadline is measured from flight start.
   - **Single flight**: one in-flight stop per client; later callers join it and get its outcome; the first caller's plan rules. A stopping mode lasts until the flight ends or its flight deadline (plan deadline + margin: `User` 10 + 3 s, `Exit` 3 + 1 s). In it the link does not redial, attach-only ensure answers not-connected, and explicit connect / tray 「데몬 켜기」 wait. On expiry the flight is aborted and every side effect is gated on the deadline (R2, R11, R19).
   - **Version-echo stop handshake** (ADR-0343 decision 2): authenticate with the daemon's own advertised version; nothing but `StopDaemon` follows `Hello`.
   - **One dial, bounded by the deadline**: the dial and handshake get the remaining graceful time (10 s from flight start minus a small reserve). If nothing was sent — anything that ends the dial before `Hello`, or a failed send — the identity-checked kill runs **at once**. If no `Hello` arrived or the daemon has not closed by the 10 s deadline, the kill runs then (D6, D7, R1, R13). No decision is read from failure texts.
   - **The kill** is `terminate_if_same(pid, start_time)` with the identity resolved at flight start (never re-read). With `start_time == 0` there is no kill ever.
   - **"Gone"** is answered on one process handle: gone = the open fails with error 87 (pid absent), or the creation time differs from a nonzero target `start_time`, or the exit code is not `STILL_ACTIVE`; any other open or query failure = unknown, not gone (R4). The stop finishes as soon as the daemon is gone. Outcomes: `closed` · `killed` · `timeout` · `no_target`.
   - **`Exit` never kills** and, after sending, keeps reading until the daemon closes the connection or 3 s pass — never a close right behind `StopDaemon` (R18). App exit also stops an older-version daemon (D8).
   - After a `User` stop the shell fires an attach-only re-add (not awaited); with no daemon its first attempt fails and ① shows.
   - The LLM `daemon.stop` runs with no confirm (D5). The LLM path is `__engramCmd` `connection.status` / `connection.reconnect` / `daemon.stop`, main window only (D10).
   - The frontend `daemon_stop` command surface becomes `daemon_shutdown`; `DaemonControl.stop()` no longer calls `client.disconnect()`.
   - Tray and app exit use a blocking wrapper bounded by the flight deadline (R5); with no `DaemonClient` they skip the stop with a warn log and quit still exits (R10).
4. **The daemon runs `StopDaemon`'s shutdown in a task that a connection abort cannot cancel**; the Ack is best-effort; a second `StopDaemon` only joins the running one (R3). Shipped older daemons keep the old behaviour (accepted).
5. **Boot start runs only in the main window**; popouts and the prewarmed spare attach only (R17). A shell-side boot-pending flag keeps their early attach from showing ① (R20).
6. **Partial amendments — exact scope:**
   - **ADR-0344 decisions 4–5**: the guidance wording is dropped (no "use the newer app", no "quit and re-extract" text — ② shows the version text only). The button rule is kept: the button shows only when the daemon is older than the app, and not for a mismatched daemon started after this app process (the exe was not replaced). Decisions 1–3 and 6–8 hold.
   - **ADR-0025**: besides tray 「데몬 켜기」 and rerunning the UI, a user-pressed 「다시 연결」 also revives a dead daemon; the "once at boot" ensure runs in the main window only. The reconnect loop and the command path stay attach-only.
   - **ADR-0021 decision 5**: the `daemon_stop` surface becomes `daemon_shutdown`. The 「영향」 stop order (graceful → `disconnect()` → still-alive check → kill fallback, run by the frontend) now lives in the shell's one stop path (decision 3), without `disconnect()`.
   - **ADR-0343 decision 3**: the kill timing is decision 3's (at once when nothing was sent; at the 10 s deadline otherwise), with the `Exit` exception (never kills). Decision 2 holds.
   - **ADR-0271 decision 5**: the stop that moves into the shell's daemon-connection code at the transport stage also carries the identity-checked kill fallback and the 10 s graceful deadline (D6–D7).
   - **ADR-0343 decision 6** (a version-independent stop frame for future daemons) stays open; it has no landing unit in 3-2 U5.
7. **Lands in 3-2 U5**, sequential units: **U5a** — stop client, one stop path, daemon fix (no UI change) · **U5b-1** — refusal slot, band status in routing, the shell → frontend DTO, `daemon_shutdown` · **U5b-2** — frontend band, buttons, confirm, LLM commands, main-window-only boot start. Until U5b-2 the frontend `DaemonControl.stop()` keeps its old path.

## 거부한 대안

- **Stopped latch — no band after an intended stop (parent TRD §6-4 ⑤)** — removed after review rounds 1 and 2 each found defects of the same class in the latch; raised to the user, who chose plain ① after every stop: 「A로 해 문구는 너무 디테일하게 말고 심플하게」. The related "intent marker" also relied on a retry budget that ADR-0346 removed. 약함 — 사용자 직접 기각.
- **Kill timing from failure-text classes** (edition 4: a fixed 3 s handshake bound, `NoHello` classified from the transport's fixed failure texts) — rewritten in review round 3 (codex must, lead, risk axis): the only fact that matters is whether `StopDaemon` was sent, and a fixed short handshake bound cut a slow-but-alive daemon off before the 10 s window. Decisions are not read from texts (ADR-0342 decision 2). 근거 = 리뷰 round 3 + ADR-0342.
- **Separate texts per situation** (newer daemon, stale exe, after a stop, etc.) — user: 「버전 다름 하고 연결안됨」 · 「너무 디테일하게 하지는 말아」. 약함 — 사용자 직접 기각.
- **A tray 「다시 연결」 item** (a way back after the band was closed with ✕) — user: 「구지 뭘 추가할 필요는 없어보이는데?」. 약함 — 사용자 직접 기각.
- **Token vs version from the `Rejected.reason` prefix** — a 3-3 daemon close reason could imitate it. 근거 = 코드(close reason is daemon-controlled).
- **A shell close code ≥ 4100 for token refusals** — `peer.rs` asserts 4001 for refusals. 근거 = 코드.
- **Non-main windows do not attach at boot (instead of the boot-pending flag)** — the flag covers every attach path (boot pulls such as `getAgents`, LLM calls, commands in any window) in one place, with no frontend call site to audit. 근거 = 코드(attach paths).

## 근거

- **User decisions** — U1 (2026-11-08, ✕ unchanged: 「구지 뭘 추가할 필요는 없어보이는데?」) · U2 (2026-11-08, token text: 「이유만 짧게」) · D1 (2026-11-09: 「끊기고 한번 실패해서 띄우는게 맞아보이는데」) · D2 (2026-11-09: 「A로 하자」) · two bands (「버전 다름 하고 연결안됨」) · U4 (2026-11-09: 「A로 해 문구는 너무 디테일하게 말고 심플하게」).
- **Conventions** — D1: Zulip re-adds its banner on a failed retry; VS Code Remote shows its dialog after 40 s (`docs/research/disconnect-banner-dismiss-reconnect-2026-11-05.md`). D2: wezterm `no_serve_automatically` default false, gpg-agent auto-start, `emacsclient -a ""`, VS Code Remote-SSH; counter-examples tmux / zellij `attach`. D7: `docker stop` 10 s, supervisord `stopwaitsecs` 10 s; PM2 1.6 s; k8s 30 s / systemd 90 s are server settings.
- **Code read** (static, base `7354b9e0`) — the facts in 「맥락」: `stop.rs` `NoTarget` on mismatch; the `StopDaemon` arm awaiting `shutdown_all` inside the dispatch task; `route.rs` one text for every 4001; `pid_alive_with_start_time` creation-time-only compare; `App.tsx` boot start in every window.
- **Review** — TRD addendum deep review 2026-11-08, delta rounds 2–4, PRD+TRD review PASSED 2026-11-09 (lead PASS · risk FIX no must · codex PASS).

## 영향 / 불변식

- **One stop path**: band, tray, LLM, `DaemonControl.stop()` and app exit all go through `DaemonClient::stop_daemon`. A second stop implementation (frontend pid kill, a separate tray client) breaks single flight and the kill identity rule.
- **Only `StopDaemon` follows `Hello`** on the version-echo handshake (ADR-0343 「영향」) — pinned by a test.
- **The kill uses the identity resolved at flight start and never runs with `start_time == 0`; `Exit` never kills.**
- **No band rule after a stop**: after any user-initiated stop the band is ①. Reintroducing a stopped latch or a "no band after an intended stop" rule needs a new decision.
- **Band decisions come from what the shell did** (refusal slot, connection state), never from the daemon's text.
- ADR-0021 decision 2 holds: the reconnect loop and the command path stay attach-only; only boot start (main window) and 「다시 연결」 spawn.
- Code consequences (U5): new `link/stop.rs` and `daemon_client/shutdown.rs` (names 제안 in the TRD); `stop.rs` deleted; `terminate_if_same` promoted from `platform::testing` to `platform::process` with a non-Windows stub; `pid_alive_with_start_time` gains the exit-code check (its callers `codex/thread_lock.rs` and `net/portfile.rs` are in U5a's review scope); the shell drops `tungstenite` (「shell gate 6」); the connection-state event and pull carry `{state, band}` (contract change, all readers in U5b-1); `daemon_stop` removed if unused.
- Accepted risks (TRD §9): a shipped older daemon stopped by `Exit` whose `shutdown_all` outlasts 3 s may survive with zero agents · 「다시 연결」 revives a daemon killed from outside (intended) · a restart right after a stop can attach to a daemon still exiting, then lose it → ① and background retry · token text also shows for a daemon auth timeout · `quit_app` blocks the main thread up to 4 s.
