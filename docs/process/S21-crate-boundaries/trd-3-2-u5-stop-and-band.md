# TRD addendum — 3-2 U5: stop client on transport · two connection bands (S21)

> Status: **Edition 5.1 (2026-11-09) — PRD+TRD review PASSED (deep, 4 delta rounds); round-4 recommendations folded in.** Edition 5 — round-3 delta review applied: one-dial stop rule, strict "gone" check, flight expiry cancels side effects, no false band during a cold boot.** (Edition 4: user decision U4 removed the stopped latch.) No code changed while this was written. Canon ADRs: ADR-0343 d2–d3 · ADR-0344 (d4–d5 wording dropped — amendment pending) · ADR-0345 · ADR-0346 · ADR-0342 · ADR-0021 · ADR-0025 · ADR-0026 · ADR-0134 d4 · ADR-0271 d5 · ADR-0287. Base = `7354b9e0` — a merge of master into this branch whose tree is identical to master's. Static read only.
> Notation: 「제안」 = proposal (new names provisional) · 「미확인」 = not verified. Line numbers are from the base; re-grep before use. UI wording stays short (user: 「너무 디테일하게 하지는 말아」); the stop rules are precise.
> Edition history: 1 → 2 user answers, two bands · 2 → 3 review R1–R12 · 3 → 4 user decision U4 (no latch, ① after every stop) + review round 2 · 4 → 5 review round 3 (R13 rewritten, R4 tightened, R19–R23) · 5 → 5.1 round-4 recommendations (R24).

## 1. Scope

- Replaces the §6-2 U5 row of `trd-3-transport.md` (line 404); where they differ, this file wins. **It also supersedes parent §6-4 ⑤ "intended disconnect → no band" for stops**: after a tray / LLM / band stop the ordinary band ① shows (U4). App exit is moot — the app is gone.
- **U5a** = the stop client on transport, the one stop path (band, tray, app exit, LLM) with its single-flight and stopping-mode rules, and the daemon's `StopDaemon` fix. **U5b** = the two bands, their actions, the refusal text, the LLM commands, the boot-start fixes (findings A, B and N7).
- Out of scope: T-80 (3-3 — U3) · U6 · moving the daemon onto transport · a distinct wire close code for version / identity refusals (3-3).

## 2. What the code does today

| Thing | Today |
|---|---|
| Stop client `daemon_client/stop.rs` | Sync `tungstenite`; Auth with **our** version; `StopDaemon{force:true}`; flush; then **reads until the daemon closes or 3 s** — an immediate close after the flush was a measured regression (the daemon's write fails, `net` aborts the reader before `StopDaemon` is dispatched, the daemon survives — `stop.rs:169-176`). A mismatched or dead daemon = `NoTarget`, so tray 「데몬 끄기」 and app exit do nothing on a version mismatch. Callers: tray `tray/mod.rs:221` (Tauri blocking pool), `quit_app` `tray/actions.rs:327` (main thread). Neither closes the shell link first. |
| Daemon on `StopDaemon` | `connection_core.rs:1645-1681`: `spawn_blocking(shutdown_all).await` (`:1676`) → Ack → `shutdown_tx.send(true)` (`:1680`) → `Close`, all inside the connection's `dispatch_task`; `net` aborts `dispatch_task` when `write_task` ends (`ws.rs:556-560`). **A client that closes while `shutdown_all` runs cancels the await; `shutdown_tx` never fires and the daemon survives with its agents killed.** On `Close`, `net` aborts `write_task` (`ws.rs:550-554`), so the queued Ack **can be lost** (`ws_e2e` `case20` awaits it and passes today — not always lost). |
| `shutdown_all` | `agent/manager.rs:3112-3132`: `shutting_down` set before any kill; parallel kills, 5 s join each (`:3078`) → ≈5 s worst case. |
| Daemon refusals (`net` `ws.rs:338-438`) | Token checked before version; every refusal = an `Error` text frame + `close(None)`. |
| Shell refusal paths | Version: the dialer refuses from `daemon.json` before dialing (`link/dialer.rs:170-182`). Wire: `ShellHandshake::on_frame` (`link/handshake.rs:108-133`) turns the `Error` frame into `Reject{Terminal}`. Both reach routing as `Rejected{4001}`. `ShellHandshake` never echoes the daemon's version (`:97-98` "Fix C"). |
| Refusal text | `link/route.rs:193`: every 4001 → the version text, so a token refusal shows it too (also `connect()`'s error, `connection.rs:951-970`). Refusal notices reach a debug log only (`connection.rs:1490-1494`). |
| Frontend surface | Event `daemon-connection-state` + pull: one word; `TauriTransport.setState` notifies on a word change only. `ConnectionNotice.tsx` renders `agentClient.connectionError` (only writer: `DaemonControl.start()`'s catch). `DaemonControl.stop()` = link round trip + `client.disconnect()` (its only production caller) + `daemon_stop` pid kill without a handle-pinned identity check — removed in U5a (§7-1 U5). |
| Boot start | Every window runs `App.tsx:52` `bootstrapDaemonIfNeeded()` → `daemon_start` (may spawn) — popouts and the prewarmed spare (built in `setup`, `lib.rs:357` `spawn_spare_build`) included (N7 — against ADR-0021 d1 / ADR-0025 "once at boot"). |
| Liveness | `platform::process::pid_alive_with_start_time` (`process.rs:164-184` — dialer liveness and `ensure_daemon`) compares creation time only: an exited daemon whose process object is still held reads as live. `process_start` says `Gone` only on open error 87 (`process.rs:36-68`, `:114-123`). |
| `DaemonClient` may be absent | `lib.rs:235-243`: managed only when setup built it. |
| **Finding A** | A runtime loss (`reconnecting`) shows **no band**; neither does a refusal after boot. |
| **Finding B** | `DaemonControl.start()` = `daemon_start` then `connect()`; a failed `daemon_start` never calls `connect()` → **no shell link**, nothing retries, a version refusal never reaches the shell. (`connect()` itself starts the handle even after a failed search — `connection.rs:409-412`.) |

## 3. U5a — stop client, one stop path, daemon fix

### 3-1. Stale pointers and gaps in the TRD U5 row (`trd-3-transport.md:404`)

| # | Row | Fact | Fix |
|---|---|---|---|
| S1 | 「`stop.rs:13-15` 「ack/응답을 읽지 않는다」」 | Comment now English. | Quote the English. |
| S2 | `machine.rs:323-347` · `:386-404` | Give-up = `schedule_retry` `:428-447`; terminal refusal = `on_connect_failed` `:393-414`; `max_attempts = 0` passes `Policy::check`. | Repoint. |
| S3 | `connection_core.rs:1542-1545` | Now `:1676-1681`. | Repoint. |
| S4 | 「primary peer」 · 「attach-only `current()`」 | U6 not landed; the link = `LinkClient` (`DAEMON = "local"`, `connection.rs:161`): close `:483`, attach-only re-add `ensure()` `:443`. | Name the real calls. |
| S5 | 「assert when called on a runtime thread」 | The tray worker runs on Tauri's blocking pool (a tokio context); only the shell's own `engram-daemon-client` threads (`mod.rs:344-346`) can deadlock. | Thread-name `debug_assert!` (R5). |
| S6 | 「`notify(StopDaemon)` with a one-shot peer」 | `notify` needs a live connection (`peer.rs:449-474`). | Wait for `Connected`; `notify` `Err` = nothing sent. |
| S7 | Target = `check_acceptable` | ADR-0343 d2 / ADR-0344 d3, d4, d6: daemon's own version; not newer than the app. | Own `StopHandshake`. |
| S8 | 「keep the one-way fire」, no fallback | ADR-0343 d3 / ADR-0344 d3 / ADR-0021 「영향」: identity-checked kill. The old note was about not waiting for the Ack — still true. | §3-3. |
| S9 | Deadline 「short」 | 3 s per blocking step today. | §3-3. |
| S10 | 「bring it up with `add` when turned back on」 | Tray 「데몬 켜기」 calls only `ensure_daemon` (`tray/mod.rs:210`). | §3-3 R10. |
| S11 | 「delete shell `tungstenite`」 · gate | Normal dep `src-tauri/Cargo.toml:105-107`; two `#[ignore]` diagnostics use it (`tests/stop_smoke.rs:162-215`). | Drop / rewrite on dev `tokio-tungstenite`; 「shell gate 6」: no `tungstenite` in `cargo tree -p engram-dashboard -e normal --depth 1`. |
| S12 | 「re-aim `stop_smoke`」 | Old import (`:22`); known failure T-52. | Re-aim; close T-52 if it passes. |
| S13 | — | `qa.md` row / 「셸 실 데몬」 lane key `daemon_client/stop.rs`; comments `discovery/mod.rs:16`, `:204`, `:214`, `:451-457`, `:593-595`, `:612`, `discovery/tests.rs:1`, `commands/tray.rs:3`. | Repoint / reword. |
| S14 | §6-4 GUI ③ 「rejection popup」 · §6-4 ⑤ 「no band」 | ADR-0342: band · U4: ① after a stop. | Update both. |
| S15 | Close the primary peer first; rejected 「intent marker」 | Both premised on "no band after an intended stop", which U4 removes; the marker's argument also relied on a retry budget ADR-0346 removed. | The close stays only to stop redials during the flight (stopping mode, R2). |

### 3-2. Stop client (`link/stop.rs`, no `tauri`)

- `resolve_stop_target(reader, liveness)` reads `daemon.json` → `{address, token, protocol_version, pid, start_time}` or `NoTarget` (no record · broken · dead pid · **newer than the app**). A sharing-violation read is retried briefly (제안 5 × 100 ms) inside the flight deadline. Reader / liveness are seams (B1).
- `StopHandshake`: Auth with the daemon's own version; `Done` on any text frame whose top-level JSON key is `"Hello"`; an `Error` frame = refusal (masked log). Only `StopDaemon` follows `Hello` (pinned by a test).
- One-shot `Registry<ShellWire>` per flight, `max_attempts = 0`, **one dial**, `connect_timeout` (dial + handshake) = the remaining graceful time: 10 s − elapsed − a small reserve (제안 0.5 s), so a slow-but-alive daemon gets the whole window to answer `Hello`. Inbox drained on the same task. `Connected` → `notify(StopDaemon{force:true, kill_agents:true})`. Raw outcomes (R13): `Sent` (then read until the first `Disconnected`/`GaveUp`; the Ack is ignored) or `NothingSent` (anything that ends the dial without `Hello` — refused, reset, closed in handshake, `Error` frame, timeout — or a `notify` `Err`). No decision is read from failure texts.

### 3-3. The one stop path — `DaemonClient::stop_daemon(plan, origin)` (`daemon_client/shutdown.rs`)

Plans: **`User`** = band, tray, LLM `daemon.stop` — graceful deadline 10 s · **`Exit`** = app exit — 3 s, never kills. **One absolute clock: every deadline below is measured from flight start**, including the `daemon.json` read and its retries (R13).

**R1 / R13 — order and fallback:**

1. Join or start the single flight (R2); enter stopping mode; `LinkClient::close()` (no redials during the flight).
2. Resolve the target (kept for the whole flight — the kill uses this identity, never a re-read). `NoTarget` → done.
3. Run the stop client. `Exit`: after `Sent`, keep reading until the first `Disconnected` or the 3 s bound, then return — never `close_after_flush` right behind `StopDaemon` (the `stop.rs:169-176` regression).
4. "Gone" (R4) — one handle, three answers: **gone** = the open fails with 87 (`ERROR_INVALID_PARAMETER`), or the creation time differs from a **nonzero** target `start_time`, or the exit code ≠ `STILL_ACTIVE`; **alive** = same creation time (or `start_time == 0`) and `STILL_ACTIVE`; **unknown** = any other open / query failure (not gone). After `Disconnected`, poll "gone" and finish as soon as it is gone (no sleeping to the deadline).
5. Fallback, `User` only, when the target is alive or unknown: `NothingSent` → identity-checked kill **at once** (nothing graceful is in progress); `Sent` and not gone by the 10 s deadline → kill then. Kill = `platform::process::terminate_if_same(target.pid, target.start_time)`, then poll "gone" within the margin. `start_time == 0` → no kill ever (`no_identity`); gone only by 87 or the exit code. `Exit` keeps its 3 s bound (its dial bound = 3 s − elapsed − the reserve) and never kills. The reserve is not a graceful budget.
6. Outcome: gone without a kill → `closed`; gone after a kill (whatever `terminate_if_same` returned) → `killed`; alive or unknown at the end → `timeout`.
7. Leave stopping mode; then (not `Exit`) **fire** `LinkClient::ensure()` (not awaited) — after success and failure alike. With no daemon its first attempt fails and band ① shows (U4); a daemon that comes back later is attached by the background retries.

**R2 / R11 — single flight and stopping mode (no latch — U4):**

- One in-flight stop per `DaemonClient`; later callers (band in any window, tray, LLM) join it and get its outcome; `Exit` joins up to its own bound. The first caller's plan rules.
- **Stopping mode** lasts from flight start until the flight ends or the **flight deadline** = plan deadline + margin (`User` 10 s + 3 s, `Exit` 3 s + 1 s) — whichever comes first. **On expiry (R19) the flight task is aborted**: every side effect (dial, notify, kill, re-add) checks the flight deadline before it runs; the continuation of a blocking `daemon.json` read is discarded; the mode is released and the re-add is fired explicitly at expiry. Pins: the mode carries its own deadline (`ensure()` / `connect()` treat it as off past it); the expiry timer runs outside the flight task; the `daemon.json` read runs on `spawn_blocking`; joiners and the blocking wrapper get `timeout`, never a dropped sender; after a kill, an optional short "gone" wait on the same handle before the re-add.
- While in stopping mode: attach-only `ensure()` (every frontend `ensureReady` / `daemon_ensure`) answers not-connected and adds nothing; explicit `connect()` (「다시 연결」) and tray 「데몬 켜기」 wait for the mode to end (at most the flight deadline). A `connect()` that waited re-reads the close epoch after the wait (R16).
- Routing holds the shown band during the mode (R8).
- No frontend stop calls `client.disconnect()` — `DaemonControl.stop()` was removed in U5a (§7-1 U5).

**R5 — blocking wrapper** `stop_daemon_blocking(plan, origin)` (tray, `quit_app`): spawns on the client's runtime and waits on a `std::sync::mpsc` channel with `recv_timeout(flight deadline)` — `User` ≤ 13 s, `Exit` ≤ 4 s from flight start; the margin covers busy-read retries, the kill and the "gone" poll. The re-add is fired, never awaited. `debug_assert!` that the thread is not an `engram-daemon-client` thread.

**R10 — callers, no `DaemonClient`:** tray Stop → `User` (icon: `closed`, `killed` → `force_daemon_down`, else probe) · `quit_app` → `Exit`, then `app.exit(0)` whatever the outcome · tray Start → wait for stopping mode, `ensure_daemon`, then `DaemonClient::ensure()` even when `ensure_daemon` failed or timed out · clients via `app.try_state::<Arc<DaemonClient>>()`; none (setup failed to build it) → tray Stop / `quit_app` skip the stop with a warn log (quit still exits). Reason: the only runtime the shell owns is the client's, and the daemon is detached (ADR-0024).

### 3-4. Daemon fix — `StopDaemon` survives its connection (R3)

- `connection_core.rs:1645-1681`: spawn a task that runs `shutdown_all` (blocking pool) then `shutdown_tx.send(true)`; the handler does not await it, replies the Ack best-effort and returns `Close` (order bundle ⑤ — `:332`). A second `StopDaemon` while one runs only joins.
- Tests to rewrite to the new order: `connection_core.rs:6164-6187` (`stop_daemon_no_active_closes_and_signals` asserts the watch changed synchronously — it must await the task) and `ws_e2e.rs:1399-1447` `case20` (`await_ack` at `:1440` — the Ack is best-effort; assert the close and the shutdown instead).
- Shipped older daemons keep the bug (§9).

### 3-5. Cleanup

Delete `stop.rs` (move its wire-shape tests); promote `terminate_if_same` from `platform::testing` to `platform::process` with a non-Windows stub inside `platform` (B1 — production API of an existing crate: report to the user); S11–S13. **Interim (U5a → U5b-2):** the frontend `DaemonControl.stop()` and `AgentClient.stopDaemon` were removed in U5a (§7-1 U5), so there is no old frontend stop path. Until U5b-2 adds `daemon.stop` there is no supported LLM stop surface; tray Stop / Quit work. The `daemon_stop` command stays with no frontend caller until U5b-2 removes it.

## 4. U5b — two bands

### 4-1. The bands (user: 「버전 다름 하고 연결안됨」)

| Band | When | Text | Button (main window only) |
|---|---|---|---|
| ① 연결 안 됨 | cold-start failure · runtime loss after the first failed reconnect (D1) · token refusal · after any stop (U4 — the re-add fails) | 「데몬에 연결하지 못했습니다.」 + token refusal: 「데몬이 이 앱의 인증을 거절했습니다」 · or `connectionError` (R14) | 「다시 연결」 |
| ② 버전 다름 | any version mismatch | 「데몬에 연결하지 못했습니다.」 + 「실행 중인 데몬이 이 앱과 버전이 다릅니다」 | 「데몬 끄기」 only when `can_stop` |

- No band: connected · an explicit frontend close (`daemon_close` — `window.__ENGRAM_AGENT__.disconnect()`; no production caller since U5a (`DaemonControl.stop()` was its only one)) · a runtime loss before the first reconnect attempt failed.
- 「데몬 끄기」 confirms first (ADR-0344 d2): 「이전 데몬을 끕니다. 실행 중인 에이전트가 멈춥니다 — 나중에 이어서 실행할 수 있지만 진행 중인 응답은 사라집니다」 · 「끄기」 / 「취소」.
- **R8** — during a stop the shown band stays, the pressed button shows 「끄는 중…」 (disabled); 「다시 연결」 shows 「연결 중…」 while it runs.
- **R9** — popouts show the band text only: no buttons, no confirm, no `connection.*` / `daemon.stop` commands.
- ✕ (U1): dismiss key = band kind + cause text; hidden while the key holds; cleared on `connected`.

### 4-2. Shell → frontend data (extend the existing surface)

```rust
// 제안 — one DTO for the event and the pull; ts-rs export
pub struct ConnectionStatus { pub state: ConnectionStateWord, pub band: Option<Band> }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Band { NotConnected { refused: bool }, Version { can_stop: bool } }
```

- `can_stop = daemon.protocol_version < PROTOCOL_VERSION && !stale_exe`; `stale_exe` (shell-internal) = the mismatched daemon started after this app process (`start_time != 0 && > current_process_start_time()`); non-Windows → `false`.
- **R7 — routing is the single emitter**, deduped on `(state, band)`; `Routed::State` → `Routed::Status`. Rules: `Connected` → none · first failure of a never-connected handle → ① · `Disconnected` → none until the next `ConnectFailed` → ① · `Rejected` → ② or ① `refused` (§4-3) · explicit frontend close → none · **boot start pending (R20) → no band for a never-connected handle** · stopping mode → hold the shown band · a re-add keeps the shown band until the new handle's first answer.
- Port `connection_state(word)` → `connection_status(&ConnectionStatus)`; the pull returns the DTO; the driver watch carries it. `TauriTransport` notifies when `(state, band)` changes; unknown `state` → `down`, unknown `band.kind` → ① without cause.
- **R20 — no false ① during a cold boot:** the shell sets a `boot_start_pending` flag when `DaemonClient` is built in `setup` — before the main window or the spare exists, so no window's attach can precede it. While it is set, routing shows no band for a never-connected handle (state `down`). It clears when the main window's boot start answers: a failed `daemon_start` command clears it before returning its error; a successful one is followed by `connect()`, whose first answer — or its `is_live()` early return — clears it. Bounds: a long cap (60 s, "main never boots") from construction; when main's `daemon_start` begins, a short bound (`daemon_start` timeout + margin) is armed. The frontend `start()` returns early when already connected (`tauriTransport.ts:438-439`) — then only a bound clears the flag (harmless: the link is connected). **Clearing is an actor step** that re-derives the current handle's band from its last answer and re-emits, so a terminal refusal held while pending shows on clear. Chosen over "non-main windows do not attach at boot" because it covers every attach path (boot pulls such as `getAgents`, LLM calls, commands in any window) in one place, with no frontend call site to audit.
- **R14 / R21 — `connectionError`** (the frontend text from a failed start, e.g. the unwritable-folder remedy of ADR-0134 d4) belongs to one start attempt. It shows as ①'s cause while ① carries no `refused` cause; never under ②. `reportConnectionError` ignores writes while the current shell band is ② (mirrors the guard while connected, `protocolClient.ts:227-231`). It clears on band ②, on ① `refused`, on `connected`, on a stop success, and when a new start attempt begins — the "first shell status after a start attempt" of N2 read as the status that follows a *new* attempt, not the ① that the failed attempt's own attach produces (§8 note).

### 4-3. Token vs version refusal (U2 — main's mechanism)

- Decide from what the shell did, never the daemon's text (ADR-0342 d2). `LocalDaemonDialer::register` also returns an `Arc<RefusalSlot>`; `resolve` writes `{daemon, pid, start_time}` before the version `DialFailure::Refused` (`Current::VersionMismatch` gains `pid` / `start_time` — `endpoint.rs:47`, `:126`) and clears it otherwise. Routing reads the current handle's slot on `Rejected{4001}`: record → ②, none → ① `refused`. `connect()`'s error text uses the same decision.
- Rejected: the `Rejected.reason` prefix (a 3-3 daemon close reason could imitate it) · re-reading `daemon.json` on the actor · a shell close code ≥ 4100 (`peer.rs:2589-2594` asserts 4001).

### 4-4. Actions and the LLM path

| Command (`__engramCmd`, main window, no `help`) | Effect | Runs | Reply |
|---|---|---|---|
| `connection.status` | read | the surface | `{state, band, text}` |
| `connection.reconnect` | write | `DaemonControl.start()` (D2): `daemon_start` (starts the local daemon if none runs) then `connect()`; on a failed `daemon_start` the catch fires the attach-only `daemon_ensure`, which starts the handle as `connect()`'s failed-search path does (R15) — not `connect()` itself, which would run a second spawning search | status after the first answer · on ② → error: with `can_stop` 「버전이 달라 다시 연결해도 거절됩니다 — daemon.stop 을 쓰세요」, without it the version text only |
| `daemon.stop` | write | new `daemon_shutdown` → `stop_daemon(User)`; no confirm (D5) | `{outcome: closed \| killed \| timeout \| no_target, reason?, status}` — `status` is the snapshot taken when the flight ends, before the fired re-add answers |

- Not on the daemon bus: in every state with a button the shell has no daemon link (ADR-0344 d7, ADR-0345 d5).
- `daemon.stop` is a new surface that calls `daemon_shutdown`, no `disconnect()` — not a rewrite of `DaemonControl.stop()`, which U5a removed (§7-1 U5). The `daemon_stop` command goes (no frontend caller since U5a).

### 4-5. Boot and runtime fixes

- **A:** band ① on a runtime loss (R7). **B / R15:** a failed `daemon_start` still leaves a handle (attach-only `daemon_ensure`), so a version refusal surfaces as ② (stale exe → no button) and a later daemon is attached.
- **N7 / R17:** only the main window runs the boot `daemon_start`; popouts and the prewarmed spare attach only (`ensureReady`), and R20 keeps their early attach from showing ①.
- **Verify first** (start of U5b-1): reproduce A (kill the daemon while connected → no band) and B (failing boot start → no link activity in the shell log); a fix whose finding does not reproduce is dropped with a note.

## 5. Tests plan

- **Daemon:** `StopDaemon` then the client closes at once → `shutdown_tx` still fires after `shutdown_all` · second `StopDaemon` does not re-run `shutdown_all` · rewritten `stop_daemon_no_active_closes_and_signals` and `case20` (§3-4).
- **Shell unit / in-memory** (`--test lib_unit`; `MemoryNetwork` + `ManualClock`; seams: killer port, reader / liveness, "gone" probe — B1):
  - Stop client: daemon's version in Auth · only `StopDaemon` after `Hello` · tag-only `Hello` · `connect_timeout` = remaining graceful time · refused / reset / `Error` frame / `notify` `Err` → `NothingSent` · busy `daemon.json` retried within the flight deadline · newer / dead / absent → `NoTarget`, no dial.
  - Stop path: `NothingSent` + alive → kill at once · a slow `Hello` within the window → `Sent`, no kill · no `Hello` by 10 s → kill · `Sent` + alive at 10 s → kill · `Sent` + gone → `closed` as soon as gone (early finish) · "gone": 87 → gone, other open failure → unknown (not gone), exit code with the same creation time → gone, nonzero `start_time` mismatch → gone · kill uses the flight-start identity · `start_time == 0` → no kill, gone only by 87 / exit code · flight expiry: a late `daemon.json` read after expiry → no dial, no kill, re-add fired · `Exit`: reads until `Disconnected` or 3 s, never kills, no `close_after_flush` before that · deadlines measured from flight start (a slow `daemon.json` read eats into them) · re-add fired after success and failure, not for `Exit` · single flight: joiners get the same outcome; `Exit` joining waits only its bound · stopping mode: `ensure()` answers not-connected and adds nothing; `connect()` / tray Start wait ≤ flight deadline; expiry releases the mode and drops the late result; a waited `connect()` re-reads the close epoch · blocking wrapper bounded by the flight deadline; asserts off the shell runtime threads · no `DaemonClient` → tray / quit skip with a log.
  - Liveness: `pid_alive_with_start_time` returns false for an exited process whose object is still held (exit-code check added).
  - Routing: **cold boot with a spare → no band before main's start answers** (the spare's attach fails first; the flag was set at construction) · flag cleared by a failed `daemon_start`, by `connect()`'s first answer or `is_live()` return, by the bounds · main's start begins late → no band before it · refused while pending → band shown on clear · each band case of §4-1, incl. ① after the first failed reconnect with `state` unchanged and ① after a stop's re-add · held band during stopping mode · dedupe on `(state, band)` · dialer slot · `can_stop` (older / newer / stale / `start_time == 0`) · token text in `connect()`'s error · DTO golden + binding sync.
- **Frontend (vitest):** listeners fire on a band change with the same word · `connectionError`: shown as ①'s cause after a failed start (survives that attempt's own ①), hidden under ② and under ① `refused`, cleared by ②, ① `refused`, `connected`, a stop success and a new start attempt · "set after ② has arrived" → ignored · "tray stop after a token refusal" → plain ①, no stale cause · failing `daemon_start` with an old exe → ② without button (R15) · popout and spare windows: no `daemon_start` on boot, band text only, no commands (R17, R9) · buttons, confirm, busy labels, ✕ key · `tauriTransport` parses the DTO (event and pull).
- **QA:** full regression per unit · 「셸 실 데몬」 lane (`stop_smoke --ignored`, row re-keyed) + a cross-version case against a daemon built from tag `v0.3.2` (protocol 6; current = 7) · quit with N agents running → daemon process gone · **forced kill with children (R22):** a band / tray stop that ends in the kill (e.g. a test daemon that ignores `StopDaemon`) while an agent runs → daemon and its agent descendants gone (re-enable `ws_e2e` `ignored_daemon_kill_cleans_pty_child`, `:2441`, with an explicitly spawned agent that starts a grandchild; assert recursive descendants — the test collects direct children only, `:2466`) · GUI (`/qa full`): ③ `v0.3.2` daemon → ② with button → confirm → 「끄는 중…」 → exits → ① · ⑤ tray off → ① ; tray on → reconnects (also after a failed `ensure_daemon`) · ⑧ kill the daemon → ① after the first failed retry → 「다시 연결」 starts and connects · ⑨ rewrite the token in an isolated `daemon.json` → ① with the token text → restore → 「다시 연결」 · open a popout → no second `daemon_start` in the log, band text only. `/review code full` per unit.

## 6. Units (sequential — shared files; each ends green)

| Unit | Scope |
|---|---|
| **U5a** | §3 in full: daemon `StopDaemon` fix + its two tests · `link/stop.rs` · `daemon_client/shutdown.rs` (R1/R13 absolute deadline and fallback, R2 single flight + stopping mode with flight deadline, R4 "gone" incl. the one-handle probe and the `pid_alive_with_start_time` exit-code alignment in `platform` (review / test its callers `codex/thread_lock.rs:288`, `net/portfile.rs:86`), R5, R16 epoch re-read, R19 expiry) · `terminate_if_same` promotion + non-Windows stub · tray Stop / Start, `quit_app` with `try_state` · delete `stop.rs` · drop `tungstenite` + 「shell gate 6」 · `stop_smoke` re-aim + `v0.3.2` case · `qa.md` / comment follow-ups. No UI change. |
| **U5b-1** | Verify A and B · refusal slot · `Band` status in routing (R7, held band, R20 boot-pending flag) · driver watch + port + DTO + binding + pull · texts · `daemon_shutdown` · frontend transport parses the DTO and notifies on band change (contract in one commit). |
| **U5b-2** | Frontend: band model + `ConnectionNotice` (buttons, confirm, busy labels, ✕ key, popout text-only, R14) · `connectionCommands.ts` (main window) · `daemon.stop` on a new surface → `daemon_shutdown` without `disconnect` (not a rewrite of `DaemonControl.stop()`, removed in U5a) · remove `daemon_stop` (no frontend caller since U5a) · `connectionError` guard (R21) · `DaemonControl.start()` attach on failure (R15) · boot start in the main window only (R17) · `src/i18n/ko.ts` · GUI QA. |

Docs: main TRD row 404 → pointer here, §6-4 ③ and ⑤ wording · `docs/reference/architecture.md` · `docs/tracking.md` T-52.

## 7. Decisions

### 7-1. User decisions

- **U1** (2026-11-08) Rev 9 ⑥ closed: a band the user closed with ✕ keeps today's behaviour (hidden while the same reason holds; memory clears on connect). No tray item, nothing added. (User: 「구지 뭘 추가할 필요는 없어보이는데?」)
- **U2** (2026-11-08) Token refusal band text = cause only, short: 「데몬이 이 앱의 인증을 거절했습니다」. Version-mismatch text unchanged. (User: 「이유만 짧게」). How the shell tells token vs version refusal apart is main's call — you propose. → §4-3.
- **U3** (2026-11-08) T-80 (reconnect storm when a slot spews output; daemon closes on full send queue) is fixed inside 3-3, not as its own topic. Not in U5.
- **D1** (2026-11-09) On a runtime loss the band appears after the first failed reconnect attempt. User: 「끊기고 한번 실패해서 띄우는게 맞아보이는데」. Convention: Zulip re-adds its banner on a failed retry (`popup_banners.ts:39-47`); VS Code Remote shows its dialog after 40 s — `docs/research/disconnect-banner-dismiss-reconnect-2026-11-05.md`.
- **D2** 「다시 연결」 starts the local daemon if none is running, then attaches — same as the boot start. User: 「A로 하자」. Convention: wezterm `no_serve_automatically` default false (wezterm.org/multiplexing.html), gpg-agent auto-start (GnuPG docs), `emacsclient -a ""`, VS Code Remote-SSH troubleshooting docs; counter-examples tmux / zellij `attach`.
  - **Carry-over to the multi-daemon PRD** — user: 「나중에 멀티데몬 할 경우에도 한개가 타겟이 되어야되는데 그 데몬을 붙이면 되겠지」: the button targets one daemon; a remote daemon can only be attached, not started.
- **Two bands** User: 「버전 다름 하고 연결안됨」 → §4-1.
- **U4** (2026-11-09) After a user-initiated stop (tray 「데몬 끄기」, LLM `daemon.stop`, band stop) the band is just the ordinary ① 「연결 안 됨」 — no "no band after an intended stop" rule, no special text. User: 「A로 해 문구는 너무 디테일하게 말고 심플하게」. → no stopped latch; stopping mode only during a flight; supersedes parent §6-4 ⑤ for stops.
- **U5** (2026-11-09) Remove the frontend `DaemonControl.stop()` / `AgentClient.stopDaemon` in U5a. User: 「ㅇㅇ 그렇게 해」. Closes the U5a review hold on an explicit disconnect reversed by the stop's re-add. Accepted: until U5b-2 there is no supported LLM stop surface (tray Stop / Quit work).

### 7-2. Main decisions

- **D3** Two bands; the shell sends `state` + `band`; `newer` / `stale_exe` stay shell-internal.
- **D4** Token vs version via a per-registration refusal slot.
- **D5** The LLM `daemon.stop` runs without the confirm dialog (precedent: `__ENGRAM_DAEMON__.stop()`; that surface was removed in U5a — §7-1 U5).
- **D6** Identity-checked kill fallback for band and tray (ADR-0343 d3 / ADR-0344 d3, d6).
- **D7** Graceful deadline 10 s, then the kill. Convention: `docker stop` 10 s (docs.docker.com), supervisord `stopwaitsecs` 10 s (supervisord.org); PM2 1.6 s; Windows SCM 5 s unconfirmed; k8s 30 s / systemd 90 s are server settings.
- **D8** App exit (ADR-0026) also stops an older-version daemon.
- **D9** 「다시 연결」 = `DaemonControl.start()`; `daemon.stop` = new `daemon_shutdown`.
- **D10** LLM path = `__engramCmd` `connection.status` / `connection.reconnect` / `daemon.stop`.
- **D11** Findings A and B fixed in U5 after a verify-first step.
- **R1** (round 1: codex must 1 · lead R1 · risk F4/F6) Fallback also when nothing was sent (dial bound now per R13); identity re-check after a kill; `start_time == 0` → no kill, reported; busy `daemon.json` retried. `Exit` never kills.
- **R2** (round 1: codex must 2 · lead M1/R3 · risk F2/F8) Single-flight stop + stopping mode; no frontend stop calls `disconnect()` (`DaemonControl.stop()` removed in U5a — §7-1 U5). Latch and band-origin rule removed by U4.
- **R3** (risk F1 · lead R4) Daemon `StopDaemon` shutdown in a task a connection abort cannot cancel; Ack best-effort; older daemons accepted as is.
- **R4** (risk F3 · round 2: lead N4 · risk N5 · codex · round 3: codex must · risk N-5) "Gone" = open fails with 87 only, or creation time ≠ a nonzero target `start_time`, or exit code ≠ `STILL_ACTIVE` on the same handle; any other failure = unknown (not gone); `start_time == 0` → no creation-time compare, no kill. Kill identity = the target resolved at flight start. `pid_alive_with_start_time` gains the exit-code check, done on one handle with the creation time (a leftover `daemon.json` of an exited daemon is not live); its other callers — `agent/src/backend/codex/thread_lock.rs:288`, `net/src/portfile.rs:86` — are in U5a's review and test scope. Poll and finish early. Tray maps `killed` like `closed`.
- **R5** (risk F5 · round 2: risk N4/N6a · lead 6) Blocking bound = the flight deadline from flight start (`User` 10 + 3 s, `Exit` 3 + 1 s), covering busy-read retries, kill and "gone" poll; re-add fired, not awaited.
- **R6** (lead M2) The shell decides the band; `connectionError` is only a cause text (R14).
- **R7** (lead R2) Band changes route through `RouteState`, deduped on `(state, band)`; the frontend notifies on band change.
- **R8** (risk F7) The shown band stays during a stop with the busy label.
- **R9** (lead R7) Popouts: band text only.
- **R10** (lead R6 · risk F9) `try_state`; without a client tray stop / quit skip with a log; tray Start attaches even after a failed `ensure_daemon`.
- **R11** (round 2: risk N4 · N6a) Flight deadline: on expiry the stopping mode is released and the late result dropped; connect / tray Start wait at most that long.
- **R12** (batched, round 1) B1 seams · B2 tray `killed` mapping · B4 line drift · B5 §9 · B6 reconnect hint depends on `can_stop`.
- **R13** (round 2: codex must · risk N3 · lead 7 · rewritten round 3: lead · risk N-3/N-4 · codex) One absolute deadline from flight start; **one dial** whose handshake bound is the remaining graceful time, so a slow daemon gets the whole window. Anything ending the dial before `Hello` (refused, reset, closed in handshake, `Error` frame, `notify` `Err`) = nothing sent → identity-checked kill at once; no `Hello` by the deadline → kill. `Exit`: 3 s, never kills. No decision is read from failure texts.
- **R14** (round 2: lead N2 — refines main 2026-11-09 decision on ADR-0134 d4) `connectionError` is ①'s cause only while ① has no `refused` cause and no newer start attempt superseded it; cleared on ②, `connected`, a stop success and a new start attempt.
 Main confirmed (2026-11-09): "a start attempt" = a NEW attempt; a status that follows the failed attempt itself does not clear the text.
- **R21** (round 3: lead M-B · batched 4) `reportConnectionError` ignores writes while band ② is current; `connectionError` also clears on ① `refused`.
- **R15** (round 2: lead / risk N1) A failed `daemon_start` still starts the handle, so a version refusal surfaces as ② (stale exe → no button).
- **R16** (round 2: risk N6b) A `connect()` that waited on a flight re-reads the close epoch.
- **R17** (round 2: risk N2) Boot `daemon_start` only in the main window; popouts / spare windows attach only.
- **R18** (round 2: codex must · lead N3 · risk N7) `Exit` keeps reading after `Sent` until the first `Disconnected` or 3 s — no `close_after_flush` behind `StopDaemon`.
- **R19** (round 3: risk N-2 · codex · lead batched 2) Flight expiry aborts the flight task; every side effect is gated on the deadline; a late `daemon.json` read is discarded; the re-add is fired at expiry.
- **R20** (round 3: lead M-A = risk N-1) Shell-side `boot_start_pending` flag, set at `DaemonClient` construction, keeps never-connected handles bandless until the main window's boot start answers (failed `daemon_start`, `connect()`'s first answer, or a safety bound).
- **R22** (round 3: codex, carried twice) Forced kill with agent children — expected: daemon and descendants gone. Each agent's tree runs in a Job with `KILL_ON_JOB_CLOSE` whose handle the daemon holds (`platform/src/group/windows.rs:4-5`), so terminating the daemon closes it and the Job kills the tree. A process that broke away from its Job survives by design (`agent/examples/spike_breakaway.rs`), and so may a child launched before its parent was assigned — the agent paths assign after launch (`group/windows.rs:10-14`). conhost membership 미확인. QA row in §5.
- **R24** (round 4: lead 권장 · risk R20-a/b, B-1, B-2, B-4, B-6 · codex 권장) R20 short bound armed when main's `daemon_start` begins (60 s cap before); R20 clear = actor step that re-derives and re-emits; R19 pins; `Exit` dial bound; R22 assign-after-launch gap + grandchild fixture; R4 caller scope; §9 additions.
- **R23** (round 3 batched: lead 1, lead 3) `daemon.stop`'s `status` is the snapshot before the re-add answers; lead 3 moot after R13.
- **ADR follow-ups (main files them):** amend ADR-0344 d4–d5 (button rule kept, wording dropped) · ADR-0025 — a user-pressed 「다시 연결」 revives a dead daemon, and boot start is main-window only (R17) · ADR-0021 d5 — the `daemon_stop` surface becomes `daemon_shutdown`; its 「영향」 fallback order is now the shell's · ADR-0343 d3 — kill timing R13 and the `Exit` exception · ADR-0343 d6 (version-independent stop frame) has no landing unit · D6–D7 against ADR-0271 d5 · parent §6-4 ⑤ superseded for stops (U4).

## 8. Unverified

- `shutdown_all` real duration with many agents (code bound ≈5 s) · whether T-52 clears · release reach of `__engramCmd` (cdp) · `v0.2.0` / `v0.3.0` / `v0.3.1` `Hello` (only `v0.1.0` and `v0.3.2` read).

## 9. Risks

- **A spawning search racing a stop that starts after it** (U5a code review, risk 권장) — a `daemon_connect` / tray Start whose search spawns a daemon after a tray stop resolved `NoTarget` leaves that daemon running (the User re-add attaches it). No crash or data loss; the tray probe shows the truth. Accepted.
- **Older daemons cancel their own shutdown** (accepted): a shipped daemon stopped by `Exit` whose `shutdown_all` outlasts the 3 s wait may survive with zero agents (`shutting_down` was set first, so boot auto-restore marks stay). `User` stops wait 10 s and kill a survivor.
- **Kill during cleanup** — bounded: `shutdown_all` ≈5 s under the 10 s deadline; an at-once kill hits only a daemon to which nothing was sent.
- **Unknown liveness** — an open / query failure other than 87 is "unknown": the stop reports `timeout` rather than claim gone; a process that denies `PROCESS_QUERY_LIMITED_INFORMATION` never reads as gone.
- **Boot-pending bound** — if the main window never starts, early failures stay bandless until the 60 s cap.
- **Late `Hello`** — a `Hello` arriving after ≈10 s − 5 s leaves too little time for `shutdown_all`, so the stop still ends in a kill mid-shutdown; benign (`shutting_down` is set first).
- **`start_time == 0` + pid reuse** — no identity, no kill: `timeout` (rare).
- **Main webview dead and not reloaded** — popouts show ① text only (no buttons); the tray (Windows) is the only way back.
- **Restart race** — tray 「데몬 켜기」 or 「다시 연결」 right after a stop can attach to a daemon still exiting (its `daemon.json` names a live pid), then lose it → ① and background retry. Tray paths are Windows-only.
- **Echo exception** (ADR-0343 d2) — separate handshake type + the "only `StopDaemon` after `Hello`" test.
- **Stale-exe rule** only decides the button: an old daemon started by hand after the app started gets ② without a button.
- **Token text over-reach in 3-2** — a daemon `auth timeout` (15 s) and a wire version refusal (only if `daemon.json` lies) also show the token text.
- **Contract change** of the connection-state event / pull — all readers change in U5b-1 together.
- **Band churn under T-80** until 3-3 (D1 damps it).
- **`quit_app` blocks** the main thread up to 4 s before `app.exit`.
- **「다시 연결」 revives** a daemon killed from outside (D2 — intended; ADR-0025 follow-up).

## 10. Review record

- 2026-11-08 · deep · FIX (lead M1 M2 · risk F1 · codex 2 must) → edition 3
- 2026-11-08 · delta round 2 · FIX ×3 → same-class raised to user (latch) → edition 4
- 2026-11-09 · delta round 3 · FIX ×3 → edition 5
- 2026-11-09 · delta round 4 · lead PASS · risk FIX (no must) · codex PASS → **PRD+TRD review PASSED** (deep, 4 delta rounds); 권장/batched folded into 5.1
