# TRD addendum — 3-2 U5: stop client on transport · two connection bands (S21)

> Status: **Edition 3 (2026-11-09) — deep review (FIX ×3) applied: precise stop / lifecycle rules (§3-3), a daemon fix (§3-4), band authority and popout rules (§4).** No code changed while this was written. Canon ADRs: ADR-0343 d2–d3 · ADR-0344 (d4–d5 wording dropped — amendment pending) · ADR-0345 · ADR-0346 · ADR-0342 · ADR-0021 · ADR-0025 · ADR-0026 · ADR-0271 d5 · ADR-0287. Base = `7354b9e0` — a merge of master into this branch whose tree is identical to master's. Static read only.
> Notation: 「제안」 = proposal (new names provisional) · 「미확인」 = not verified. Line numbers are from the base; re-grep before use. UI wording is kept short on purpose (user: 「너무 디테일하게 하지는 말아」); the stop rules are not.
> Edition 1 → 2: user answers, two bands, 3 units. Edition 2 → 3: review decisions R1–R12 (§7-2).

## 1. Scope

- Replaces the §6-2 U5 row of `trd-3-transport.md` (line 404); where they differ, this file wins.
- **U5a** = the stop client on transport, the one stop path (band, tray, app exit, LLM) with its single-flight / stopping / latch rules, and the daemon's `StopDaemon` fix. **U5b** = the two bands, their actions, the refusal text, the LLM commands, and the fixes for findings A and B (§2).
- Out of scope: T-80 (3-3 — user decision U3) · U6 · moving the daemon onto transport · a distinct wire close code for version / identity refusals (3-3).

## 2. What the code does today

| Thing | Today |
|---|---|
| Stop client `daemon_client/stop.rs` | Sync `tungstenite`; Auth with **our** version; `StopDaemon{force:true}`; drain read with a 3 s socket timeout; a mismatched or dead daemon = `NoTarget` (nothing sent) — tray 「데몬 끄기」 and app exit silently do nothing on a version mismatch. Callers: tray `tray/mod.rs:221` (Tauri blocking pool), `quit_app` `tray/actions.rs:327` (main thread). Neither closes the shell link first. |
| Daemon on `StopDaemon` | `connection_core.rs:1645-1681`: `spawn_blocking(shutdown_all).await` (`:1676`) → Ack → `shutdown_tx.send(true)` (`:1680`) → `DispatchFlow::Close`. This runs inside the connection's `dispatch_task`; `net` aborts `dispatch_task` when `write_task` ends (`ws.rs:556-560`). **If the client's socket closes while `shutdown_all` runs, the await is cancelled, `shutdown_tx` never fires, and the daemon survives with its agents killed.** On `Close`, `net` aborts `write_task` (`ws.rs:550-554`), so the queued Ack is often never written — a likely cause of T-52's "close without handshake" (미확인). |
| `shutdown_all` | `agent/manager.rs:3112-3132`: sets `shutting_down` before any kill, then kills every session in parallel, each with a 5 s join bound (`:3078`) → ≈5 s worst case. |
| Daemon refusals (`net` `ws.rs:338-438`) | Token checked before version; every refusal = an `Error` text frame + `close(None)` (no close code). |
| Shell refusal paths | Version: the dialer refuses from `daemon.json` before dialing (`link/dialer.rs:170-182`). Wire refusal: `ShellHandshake::on_frame` (`link/handshake.rs:108-133`) turns the `Error` frame into `Reject{Terminal}`. Both reach routing as `Rejected{4001}`. `ShellHandshake` never echoes the daemon's version (`:97-98` "Fix C"). |
| Refusal text | `link/route.rs:193`: every 4001 → 「실행 중인 데몬이 이 앱과 버전이 다릅니다」, so a token refusal shows the version text (also in `connect()`'s error, `connection.rs:951-970`). Refusal notices reach a debug log only (`connection.rs:1490-1494`). |
| Frontend surface | Event `daemon-connection-state` + pull `daemon_connection_state`: one word. `TauriTransport.setState` notifies only when the word changes. `ConnectionNotice.tsx` renders only `agentClient.connectionError` (only writer: `DaemonControl.start()`'s catch). `DaemonControl.stop()` = link round trip + `client.disconnect()` + `daemon_stop` pid kill (no handle-pinned identity check). |
| `DaemonClient` may be absent | `lib.rs:235-243`: built in `setup`; on a construction error it is not managed. |
| **Finding A** | A runtime loss (`reconnecting`) shows **no band**; neither does a refusal after boot. |
| **Finding B** | Boot = `daemon_start` then `connect()`. If `daemon_start` fails, `connect()` is never called → **no shell link**, nothing retries in the background (revision 9 ① holds only when `daemon_start` succeeded). |

## 3. U5a — stop client, one stop path, daemon fix

### 3-1. Stale pointers and gaps in the TRD U5 row (`trd-3-transport.md:404`)

| # | Row | Fact | Fix |
|---|---|---|---|
| S1 | 「`stop.rs:13-15` 「ack/응답을 읽지 않는다」」 | Comment now English, same lines. | Quote the English. |
| S2 | `machine.rs:323-347` · `:386-404` | Give-up = `schedule_retry` `:428-447`; terminal refusal = `on_connect_failed` `:393-414`. `max_attempts = 0` passes `Policy::check`. | Repoint. |
| S3 | `connection_core.rs:1542-1545` | Now `:1676-1681`. | Repoint. |
| S4 | 「primary peer」 · 「attach-only `current()`」 | U6 not landed; the link = `LinkClient` (`DAEMON = "local"`, `connection.rs:161`): close `:483`, attach-only re-add `ensure()` `:443`. | Name the real calls. |
| S5 | 「assert when called on a runtime thread」 | The tray worker runs on Tauri's blocking pool (a tokio context) — a naive check fires there. Only the shell's own runtime threads (`engram-daemon-client`, `mod.rs:344-346`) can deadlock. | `debug_assert!` on the thread name (§3-3 R5). |
| S6 | 「`notify(StopDaemon)` with a one-shot peer」 | `Peer::notify` needs a live connection (`peer.rs:449-474`). | Wait for `Connected`; a `notify` `Err` = nothing sent. |
| S7 | Target = `check_acceptable` (mismatch → `NoTarget`) | ADR-0343 d2 / ADR-0344 d3, d4, d6: authenticate with the daemon's own version; stop a daemon not newer than the app. | Own `StopHandshake`. |
| S8 | 「keep the one-way fire」, no fallback | ADR-0343 d3 / ADR-0344 d3 / ADR-0021 「영향」 (graceful → fallback): identity-checked kill. The old note was about not waiting for the Ack — still true. | §3-3. |
| S9 | Deadline 「short」 | 3 s per blocking step today. | §3-3. |
| S10 | 「bring it up with `add` when turned back on」 | Tray 「데몬 켜기」 calls only `ensure_daemon` (`tray/mod.rs:210`). | §3-3 R10. |
| S11 | 「delete shell `tungstenite`」 · gate | Normal dep `src-tauri/Cargo.toml:105-107`; two `#[ignore]` diagnostics use it (`tests/stop_smoke.rs:162-215`). | Drop / rewrite on dev `tokio-tungstenite`; 「shell gate 6」: no `tungstenite` in `cargo tree -p engram-dashboard -e normal --depth 1`. |
| S12 | 「re-aim `stop_smoke`」 | Imports the old path (`:22`); known failure T-52. | Re-aim; close T-52 if it passes. |
| S13 | — | `qa.md` row / 「셸 실 데몬」 lane key `daemon_client/stop.rs`; comments `discovery/mod.rs:16`, `:204`, `:214`, `:451-457`, `:593-595`, `:612`, `discovery/tests.rs:1`, `commands/tray.rs:3`. | Repoint / reword; narrow `FileReader` if unused. |
| S14 | §6-4 GUI ③ 「rejection popup」 | ADR-0342: band. | Band. |
| S15 | Rejected alternative 「intent marker, no close」 | Its argument (reconnects burn the budget into `GaveUp`) relied on a retry budget ADR-0346 removed. The latch (§3-3 R2) is a marker *plus* the close. | Note only. |

### 3-2. Stop client (`link/stop.rs`, no `tauri`)

- `resolve_stop_target(reader, liveness)` reads `daemon.json` → `{address, token, protocol_version, pid, start_time}` or `NoTarget` (no record · broken · dead pid · **newer than the app**). A sharing-violation read is retried briefly (제안 5 × 100 ms) before `NoTarget`. Reader / liveness are seams (B1).
- `StopHandshake`: Auth with the daemon's own version; `Done` on any text frame whose top-level JSON key is `"Hello"`; an `Error` frame = terminal refusal (masked log). Only `StopDaemon` follows `Hello` (pinned by a test) — the echo is a single-purpose exception to "Fix C".
- One-shot `Registry<ShellWire>`, `max_attempts = 0`, **`connect_timeout = 3 s`** (well under the 10 s deadline, so every connect / handshake failure resolves as "nothing sent" before it). Its inbox is drained on the same task. `Connected` → `notify(StopDaemon{force:true, kill_agents:true})`. Raw outcomes: `Sent` (notify Ok) → then the first `Disconnected`/`GaveUp` = `Disconnected`; `NothingSent` (refused · no `Hello` · connect failure · `notify` `Err`); the Ack is ignored.

### 3-3. The one stop path — `DaemonClient::stop_daemon(plan, origin)` (`daemon_client/shutdown.rs`)

**R1 — order and fallback** (plans: **`User`** = band, tray, LLM, `DaemonControl.stop()`; **`Exit`** = app exit):

1. Join or start the single flight (R2). Enter stopping mode. `LinkClient::close()`.
2. Resolve the target. `NoTarget` → done (`no_target`).
3. Run the stop client. `User`: deadline 10 s. `Exit`: 3 s; after `Sent`, `close_after_flush` (bounded) so the frame is written, then stop talking.
4. "Closed" = **the process is gone** (R4): after `Disconnected`, poll `pid + start_time` until gone, within the remaining deadline.
5. Fallback, `User` only (ADR-0021 「영향」 · ADR-0344 d3): if the path ended without the process gone — `NothingSent`, or `Sent` but still alive at the 10 s deadline — and `daemon.json`'s pid + start_time still names a live process → `platform::process::terminate_if_same(pid, start_time)`, then poll for gone (bounded by the R5 margin). Timing (R13): a `NothingSent` kills as soon as the graceful path has ended (≤ 3 s — no graceful shutdown is in progress to wait for); a `Sent` kills at the 10 s deadline.
6. After any kill result, re-check identity: gone → `killed`; `Ok(false)` or `Err` with the process gone → `closed`; still alive → `timeout`. `start_time == 0` (old `daemon.json`) → the kill is not attempted and the outcome says so (`timeout`, reason `no_identity`). `Exit` never kills: still alive at 3 s → `timeout`.
7. Result: `closed` / `killed` → set the **stopped latch** (R2), leave stopping mode. Anything else → leave stopping mode, **fire** `LinkClient::ensure()` (F4 re-add, not awaited; clears nothing because no latch was set) — except `Exit`.

**R2 — single flight, stopping mode, latch:**

- One in-flight stop per `DaemonClient`. Later callers (band in any window, tray, LLM) join it and get its outcome; `Exit` joins up to its own bound. The plan of the first caller rules (a joining `User` does not add a kill to an `Exit` flight; `Exit` starting while a `User` flight runs just waits).
- **Stopping mode** (while a flight runs): attach-only `ensure()` — every frontend `ensureReady` / `daemon_ensure` — answers not-connected and adds nothing; explicit `connect()` (「다시 연결」) and tray 「데몬 켜기」 wait for the flight to finish.
- **Stopped latch** (after `closed` / `killed`): attach-only `ensure()` keeps answering not-connected and adds nothing until an explicit `connect()`, tray 「데몬 켜기」 or app restart clears it.
- Band while latched (via routing — §4-2): if any caller of the flight came from band ② → band ① (user: after 「데몬 끄기」 the band falls back to ①); a tray / LLM / exit stop → no band (parent §6-4 ⑤ "intended disconnect, no band").
- `DaemonControl.stop()` no longer calls `client.disconnect()` — the latch replaces it.

**R5 — blocking wrapper** `stop_daemon_blocking(plan, origin)` for tray and `quit_app`: spawns on the client's runtime, waits on a `std::sync::mpsc` channel with `recv_timeout(plan deadline + margin)`; margin (제안 3 s) covers the kill and the gone-poll. Bounds: `User` ≤ 13 s, `Exit` ≤ 3 s + 1 s. The F4 re-add is fired, never awaited. `debug_assert!` that the thread is not an `engram-daemon-client` thread.

**R10 — callers, and no `DaemonClient`:**

- Tray Stop → `User`; icon: `closed` and `killed` → `force_daemon_down`, else probe.
- `quit_app` → `Exit`, then `app.exit(0)` regardless of the outcome (ADR-0026 best effort).
- Tray Start → wait for any flight, `ensure_daemon`, then clear the latch and `DaemonClient::ensure()` — **also when `ensure_daemon` failed or timed out** (a daemon that comes up late is then attached).
- Clients are fetched with `app.try_state::<Arc<DaemonClient>>()`. No client (setup failed to build it — `lib.rs:235-243`): tray Stop and `quit_app` **skip the stop with a warn log** (quit still exits; tray refreshes the icon by probe). Justification: the stop client needs a tokio runtime and the only one the shell owns is the client's; building a runtime on the main thread at exit for a case where setup already failed costs more than it saves, and the daemon is detached anyway (ADR-0024).

### 3-4. Daemon fix — `StopDaemon` survives its connection (R3)

- In `connection_core.rs:1645-1681`: spawn a task that runs `shutdown_all` (on the blocking pool) and then `shutdown_tx.send(true)`; the handler no longer awaits it. The reply / Ack is best-effort; the handler still returns `Close` (order bundle ⑤ — `StopDaemon` stays the last command of its connection, `:332`). A second `StopDaemon` while one runs only joins (no second `shutdown_all`).
- Effect: a client that closes early (the `Exit` plan, a deadline) can no longer cancel the shutdown; `Exit` therefore only needs the frame written.
- Shipped older daemons keep the bug (accepted — §9).

### 3-5. Cleanup

Delete `stop.rs` (move its wire-shape tests); promote `terminate_if_same` from `platform::testing` into `platform::process` with a non-Windows stub inside `platform` (B1 — production API of an existing crate: report to the user); S11–S13.

## 4. U5b — two bands

### 4-1. The bands (user decision: 「버전 다름 하고 연결안됨」)

| Band | When | Text | Button (main window only) |
|---|---|---|---|
| ① 연결 안 됨 | cold-start failure (first failed attempt) · runtime loss after the first failed reconnect (D1) · token refusal · latched after a band ② stop | 「데몬에 연결하지 못했습니다.」 + token refusal: 「데몬이 이 앱의 인증을 거절했습니다」 · or `connectionError` (R6, R14) | 「다시 연결」 |
| ② 버전 다름 | any version mismatch | 「데몬에 연결하지 못했습니다.」 + 「실행 중인 데몬이 이 앱과 버전이 다릅니다」 | 「데몬 끄기」 only when `can_stop` |

- No band: connected · closed by us (frontend `disconnect`, tray / LLM / exit stop, latched without band origin) · runtime loss before the first reconnect attempt failed.
- 「데몬 끄기」 confirms first (ADR-0344 d2): 「이전 데몬을 끕니다. 실행 중인 에이전트가 멈춥니다 — 나중에 이어서 실행할 수 있지만 진행 중인 응답은 사라집니다」 · 「끄기」 / 「취소」 (in-app modal like `RestoreModal.tsx`).
- **R8 — during a stop** the originating band stays with 「끄는 중…」 (disabled) until the flight decides: success → ① (band origin) · failure → whatever the F4 re-add yields. Routing holds the shown band while stopping mode is on. 「다시 연결」 shows 「연결 중…」 while its command runs.
- **R9 — popout windows** show the band text only: no buttons, no confirm, and the `connection.*` / `daemon.stop` commands are registered in the main window only.
- ✕ (user decision U1): dismiss key = band kind + cause text; hidden while the key holds; cleared on `connected`.

### 4-2. Shell → frontend data (extend the existing surface)

```rust
// 제안 — one DTO for the event and the pull; ts-rs export
pub struct ConnectionStatus { pub state: ConnectionStateWord, pub band: Option<Band> }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Band { NotConnected { refused: bool }, Version { can_stop: bool } }
```

- `can_stop = daemon.protocol_version < PROTOCOL_VERSION && !stale_exe`; `stale_exe` (ADR-0344 d5 button rule, shell-internal) = the mismatched daemon started after this app process (`start_time != 0 && > current_process_start_time()`); non-Windows → `false`.
- **R7 — routing is the single emitter**, deduped on `(state, band)`: `Routed::State` → `Routed::Status`, emitted whenever either changes (② → ① after a band stop and ① after the first failed reconnect keep `state` equal). Rules: `Connected` → none · first failure of a never-connected handle → ① · `Disconnected` → none until the next `ConnectFailed` → ① · `Rejected` → ② or ① `refused` (§4-3) · close by us → none, except the latch-with-band-origin ① (the driver tells routing) · stopping mode → hold the shown band · a re-add keeps the shown band until the new handle's first answer.
- Port `connection_state(word)` → `connection_status(&ConnectionStatus)` (same single emit); `daemon_connection_state` returns the DTO; the driver watch carries it. Frontend: `applyConnectionStatus(status)` — `TauriTransport` notifies listeners when `(state, band)` changes, not only the word; unknown `state` → `down`, unknown `band.kind` → ① without cause.
- **R6 / R14 — the shell decides the band, `connectionError` only fills a cause:** it is shown as ①'s cause while ① carries no `refused` cause (so the unwritable-folder remedy of ADR-0134 d4 survives the boot-failure attach) and with no shell band; it is never shown under ② or next to the token text; it clears on `connected` and on a stop success.

### 4-3. Token vs version refusal (user decision U2 — main's mechanism)

- Decide from what the shell did, never the daemon's text (ADR-0342 d2). `LocalDaemonDialer::register` also returns an `Arc<RefusalSlot>`; `resolve` writes `{daemon, pid, start_time}` before the version `DialFailure::Refused` (`Current::VersionMismatch` gains `pid` / `start_time` — `endpoint.rs:47`, `:126`) and clears it on every other answer. Routing reads the current handle's slot on `Rejected{4001}`: record → ②, none → ① `refused`. `connect()`'s error text uses the same decision.
- Rejected: matching the `Rejected.reason` prefix (a 3-3 daemon close reason could imitate it) · re-reading `daemon.json` on the actor · a shell close code ≥ 4100 (`peer.rs:2589-2594` asserts 4001).

### 4-4. Actions and the LLM path

| Command (`__engramCmd`, main window, no `help`) | Effect | Runs | Reply |
|---|---|---|---|
| `connection.status` | read | the surface | `{state, band, text}` |
| `connection.reconnect` | write | `DaemonControl.start()` = boot start: `daemon_start` (starts the local daemon if none runs — D2) then `connect()` (clears the latch, re-adds) | status after the first answer · on ② → error: with `can_stop` 「버전이 달라 다시 연결해도 거절됩니다 — daemon.stop 을 쓰세요」, without it the version text only (B6) |
| `daemon.stop` | write | `DaemonControl.stop()` → new `daemon_shutdown` → `stop_daemon(User, Llm)`; no confirm (D5) | `{outcome: closed \| killed \| timeout \| no_target, reason?, status}` |

- Not on the daemon bus: in every state with a button the shell has no daemon link (ADR-0344 d7, ADR-0345 d5 — the existing `__engramCmd` surface).
- `DaemonControl.stop()` → `daemon_shutdown`, no `client.disconnect()` (R2); the `daemon_stop` command goes if nothing else calls it.

### 4-5. Fixes for findings A and B

- A: band ① on a runtime loss (R7 rules). B: `DaemonControl.start()`'s catch also calls the attach-only `daemon_ensure` (ignored result), so a failed boot start leaves a retrying link.
- **Verify first** (start of U5b-1): reproduce A (kill the daemon while connected → no band) and B (make boot start fail → no link activity in the shell log); a fix whose finding does not reproduce is dropped with a note.

## 5. Tests plan

- **Daemon** (`connection_core` unit / `ws_e2e`): `StopDaemon` then the client closes at once → `shutdown_tx` still fires after `shutdown_all` · a second `StopDaemon` does not run `shutdown_all` again.
- **Shell unit / in-memory** (`--test lib_unit`; `MemoryNetwork` + `ManualClock`; seams: killer port, reader / liveness — B1):
  - Stop client: daemon's version in Auth (older, equal) · only `StopDaemon` after `Hello` · tag-only `Hello` · refused / no `Hello` / connect timeout (3 s) / `notify` `Err` → `NothingSent` · busy `daemon.json` retried · newer / dead / absent → `NoTarget`, no dial.
  - Stop path: link closed first · `NothingSent` + live identity → kill (`User`), never (`Exit`) · `Sent` + alive at 10 s → kill · gone-poll after `Disconnected` decides `closed` · kill `Ok(false)` / `Err` + gone → `closed`; still alive → `timeout` · `start_time == 0` → no kill, reason · F4 re-add fired after failure, none after success / `Exit` · single flight: a second caller joins and gets the same outcome; `Exit` joining waits only its bound · stopping mode: `ensure()` answers not-connected and adds nothing; `connect()` waits · latch: holds until explicit `connect()` / tray Start; F4 does not set it · blocking wrapper returns within deadline + margin and asserts off the shell runtime threads · no `DaemonClient` → tray / quit skip with a log.
  - Routing: each band case of §4-1 incl. ① after the first failed reconnect with `state` unchanged · ② → ① after a band stop · held band in stopping mode · dedupe on `(state, band)` · dialer slot · `can_stop` (older / newer / stale / `start_time == 0`) · token text in `connect()`'s error · DTO golden + binding sync.
- **Frontend (vitest):** listeners fire on a band change with the same word · `connectionError` shown as ①'s cause (no band or ① without `refused`), hidden under ② and under ① `refused`, cleared on `connected` and by a stop success · two band kinds + unknown kind · buttons, confirm, busy labels, ✕ key · popout: text only, no commands · `tauriTransport` parses the DTO (event and pull) · boot-failure attach.
- **QA:** full regression per unit · 「셸 실 데몬」 lane (`stop_smoke --ignored`, row re-keyed) + a cross-version case against a daemon built from tag `v0.3.2` (protocol 6; current = 7) · new row: quit with N agents running → daemon process gone · GUI (`/qa full`): ③ `v0.3.2` daemon → ② with button → confirm → 「끄는 중…」 → exits → ① · ⑤ tray off / app exit → no band; tray on → reconnects (also after a failed `ensure_daemon`) · ⑧ kill the daemon → ① after the first failed retry → 「다시 연결」 starts and connects · ⑨ rewrite the token in an isolated `daemon.json` → ① with the token text → restore → 「다시 연결」 · popout shows text only. `/review code full` per unit.

## 6. Units (sequential — shared files; each ends green)

| Unit | Scope |
|---|---|
| **U5a** | §3 in full: daemon `StopDaemon` fix (`connection_core.rs`) · `link/stop.rs` · `daemon_client/shutdown.rs` (R1, R2 single flight / stopping mode / latch gating in `ensure` / `connect`, R4, R5) · `terminate_if_same` promotion + non-Windows stub · tray Stop / Start, `quit_app` with `try_state` (R10) · delete `stop.rs` · drop `tungstenite` + 「shell gate 6」 · `stop_smoke` re-aim + `v0.3.2` case · `qa.md` / comment follow-ups. No UI change. |
| **U5b-1** | Verify A and B · refusal slot · `Band` status in routing (R7, held band, latch ①) · driver watch + port + DTO + binding + pull · texts · `daemon_shutdown` · frontend transport parses the DTO and notifies on band change (contract in one commit). |
| **U5b-2** | Frontend: band model + `ConnectionNotice` (buttons, confirm, busy labels, ✕ key, popout text-only, R6) · `connectionCommands.ts` (main window) · `DaemonControl.stop()` → `daemon_shutdown` without `disconnect` (remove `daemon_stop` if unused) · boot-failure attach · `src/i18n/ko.ts` · GUI QA. |

Docs: main TRD row 404 → pointer here, §6-4 ③ wording · `docs/reference/architecture.md` · `docs/tracking.md` T-52.

## 7. Decisions

### 7-1. User decisions

- **U1** (2026-11-08) Rev 9 ⑥ closed: a band the user closed with ✕ keeps today's behaviour (hidden while the same reason holds; memory clears on connect). No tray item, nothing added. (User: 「구지 뭘 추가할 필요는 없어보이는데?」)
- **U2** (2026-11-08) Token refusal band text = cause only, short: 「데몬이 이 앱의 인증을 거절했습니다」. Version-mismatch text unchanged. (User: 「이유만 짧게」). How the shell tells token vs version refusal apart is main's call — you propose. → §4-3.
- **U3** (2026-11-08) T-80 (reconnect storm when a slot spews output; daemon closes on full send queue) is fixed inside 3-3, not as its own topic. Not in U5.
- **D1** (2026-11-09) On a runtime loss the band appears after the first failed reconnect attempt. User: 「끊기고 한번 실패해서 띄우는게 맞아보이는데」. Convention: Zulip re-adds its banner on a failed retry (`popup_banners.ts:39-47`); VS Code Remote shows its dialog after 40 s — `docs/research/disconnect-banner-dismiss-reconnect-2026-11-05.md`.
- **D2** 「다시 연결」 starts the local daemon if none is running, then attaches — same as the boot start. User: 「A로 하자」. Convention: wezterm `no_serve_automatically` default false (wezterm.org/multiplexing.html), gpg-agent auto-start (GnuPG docs), `emacsclient -a ""`, VS Code Remote-SSH troubleshooting docs; counter-examples tmux / zellij `attach`.
  - **Carry-over to the multi-daemon PRD** — user: 「나중에 멀티데몬 할 경우에도 한개가 타겟이 되어야되는데 그 데몬을 붙이면 되겠지」: the button targets one daemon; a remote daemon can only be attached, not started.
- **Two bands** User: 「버전 다름 하고 연결안됨」 → §4-1. Dropped texts: "connection lost / reconnecting", "old daemon stopped", "daemon is newer — use the newer app", "stale exe — re-extract".

### 7-2. Main decisions

- **D3** Two bands; the shell sends `state` + `band`; `newer` / `stale_exe` stay shell-internal.
- **D4** Token vs version via a per-registration refusal slot (§4-3).
- **D5** The LLM `daemon.stop` runs without the confirm dialog (precedent: `__ENGRAM_DAEMON__.stop()`).
- **D6** Identity-checked kill fallback for band and tray (ADR-0343 d3 / ADR-0344 d3, d6).
- **D7** Graceful deadline 10 s, then the kill. Convention: `docker stop` 10 s (docs.docker.com), supervisord `stopwaitsecs` 10 s (supervisord.org); PM2 1.6 s; Windows SCM 5 s unconfirmed; k8s 30 s / systemd 90 s are server settings.
- **D8** App exit (ADR-0026) also stops an older-version daemon.
- **D9** 「다시 연결」 = `DaemonControl.start()`; `daemon.stop` = new `daemon_shutdown`.
- **D10** LLM path = `__engramCmd` `connection.status` / `connection.reconnect` / `daemon.stop`.
- **D11** Findings A and B fixed in U5 after a verify-first step.
- **R1** (codex must 1 · lead R1 · risk F4/F6) Fallback also when nothing was sent; one-shot `connect_timeout` 3 s; `notify` `Err` = nothing sent; identity re-check after a kill; `start_time == 0` → no kill, reported; busy `daemon.json` retried. `Exit` never kills. (§3-3)
- **R2** (codex must 2 · lead M1/R3 · risk F2/F8) Single-flight stop, stopping mode, stopped latch; band ① only for a band-② origin; `DaemonControl.stop()` drops `disconnect()`. (§3-3)
- **R3** (risk F1 must · lead R4) Daemon `StopDaemon` shutdown runs in a task a connection abort cannot cancel; Ack best-effort; in U5a; older daemons accepted as is. (§3-4)
- **R4** (risk F3) "Closed" = pid + start_time gone, bounded by the deadline; then kill (`User`) / `timeout` (`Exit`). Tray maps `killed` like `closed` (B2).
- **R5** (risk F5) Blocking bound = plan deadline + fixed margin; F4 re-add fired, not awaited.
- **R6** (lead M2) The shell decides the band; `connectionError` is only a cause text (refined by R14).
- **R7** (lead R2) Band changes route through `RouteState`, deduped on `(state, band)`; the frontend notifies on band change.
- **R8** (risk F7) The originating band stays with 「끄는 중…」 until the stop decides.
- **R9** (lead R7) Popouts: band text only.
- **R10** (lead R6 · risk F9) `try_state`; without a client tray stop / quit skip with a log (quit still exits); tray Start attaches even after a failed `ensure_daemon`.
- **R11** (lead R5) Cross-version fixture = daemon from tag `v0.3.2` (protocol 6). Verified: `v0.1.0` (protocol 3) and `v0.3.2` emit `AgentEvent::Hello` as an externally tagged `{"Hello":{…}}` (`protocol/src/messages.rs` at those tags), and `StopDaemon {force, kill_agents, request_id}` is unchanged since `v0.1.0`.
- **R13** (main, 2026-11-09) Kill timing: a stop that sent nothing goes to the identity-checked kill as soon as the graceful path has ended (≤ 3 s); a stop that sent `StopDaemon` kills at the 10 s deadline. Reason: with nothing sent there is no graceful shutdown in progress to wait for.
- **R14** (main, 2026-11-09 — R6 vs ADR-0134 d4) `connectionError` stays ①'s cause while ① carries no `refused` cause; not shown under ②; cleared on `connected` and on a stop success.
- **R12** (batched) B1 seams (killer port, reader / liveness, `terminate_if_same` non-Windows stub in `platform`) · B2 tray `killed` mapping · B4 line drift fixed · B5 §9 tightened · B6 the reconnect hint depends on `can_stop`.
- **ADR follow-ups (main files them, B3):** amend ADR-0344 d4–d5 (button rule kept, wording dropped) · ADR-0025 — a user-pressed 「다시 연결」 now revives a dead daemon (ADR-0025 said revival only by 「데몬 켜기」 / rerunning the UI) · ADR-0021 d5 — the `daemon_stop` surface becomes `daemon_shutdown`, and the fallback order of its 「영향」 is now the shell's · ADR-0343 d6 (version-independent stop frame) has no landing unit · D6–D7 against ADR-0343 d3 / ADR-0271 d5.

## 8. Unverified

- `shutdown_all` real duration with many agents (code bound ≈5 s) · whether T-52 clears · release reach of `__engramCmd` (cdp) · `v0.2.0` / `v0.3.0` / `v0.3.1` `Hello` (only `v0.1.0` and `v0.3.2` read).

## 9. Risks

- **Older daemons cancel their own shutdown** (accepted): a shipped daemon stopped by `Exit` (3 s) whose `shutdown_all` outlasts the wait may survive with zero agents (its `shutting_down` flag was set before the kills, so boot auto-restore marks stay intact). `User` stops wait 10 s, above the ≈5 s `shutdown_all` bound, and kill a survivor.
- **Kill during cleanup** — bounded: `shutdown_all` ≈5 s (parallel kills, 5 s join each — `manager.rs:3078`) under the 10 s deadline; a kill only hits a daemon still alive at 10 s.
- **Echo exception** (ADR-0343 d2) — contained by a separate handshake type and the "only `StopDaemon` after `Hello`" test.
- **Latch surprises** — while latched, frontend commands answer not-connected until 「다시 연결」 / tray 「데몬 켜기」 / restart; a tray or LLM stop shows no band (intended — the tray icon shows the state).
- **Stale-exe rule** only decides the button: an old daemon started by hand after the app started gets ② without a button.
- **Token text over-reach in 3-2** — a daemon `auth timeout` (15 s) and a wire version refusal (only if `daemon.json` lies) also show the token text.
- **Contract change** of the connection-state event / pull — all readers change in U5b-1 together.
- **Band churn under T-80** until 3-3 (D1 damps it).
- **`quit_app` blocks** the main thread up to 4 s (3 s `Exit` deadline + 1 s margin) before `app.exit`.
- **「다시 연결」 revives** a daemon killed from outside (D2 — intended; ADR-0025 follow-up).

## 10. Review record

- 2026-11-08 · deep · FIX (lead M1 M2 · risk F1 · codex 2 must) → edition 3
