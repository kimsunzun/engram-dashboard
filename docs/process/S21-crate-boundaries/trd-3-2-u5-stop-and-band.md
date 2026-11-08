# TRD addendum — 3-2 U5: stop client on transport · two connection bands (S21)

> Status: **Edition 2 (2026-11-09) — user answers applied (§7); simplified to two bands per the user (「너무 디테일하게 하지는 말아」).** No code changed while this was written. Canon ADRs: ADR-0343 decisions 2–3 · ADR-0344 (decisions 4–5 wording dropped — amendment pending, §7) · ADR-0345 · ADR-0346 · ADR-0342 · ADR-0271 decision 5 · ADR-0021 · ADR-0287 · ADR-0026. Base = HEAD `7354b9e0` (= master). Static read only.
> Notation: 「제안」 = this addendum's proposal (new names provisional) · 「미확인」 = not verified. Line numbers are from the base; re-grep before use.
> Edition 1 → 2: open questions Q1–Q8 answered and moved to §7 · the five problem kinds and ten band rows collapsed into two bands · units 5 → 3.

## 1. Scope

- Replaces the §6-2 U5 row of `trd-3-transport.md` (line 404); where they differ, this file wins.
- **U5a** = the stop client on transport + the one stop path (band, tray, app exit, LLM). **U5b** = the two bands, their actions, the refusal text, the LLM commands, and two boot/runtime gaps found in the survey (§2, last two rows).
- Out of scope: T-80 (fixed inside 3-3 — user decision 3) · U6 · the daemon side (still `net`) · a distinct wire close code for version / identity refusals (3-3).

## 2. What the code does today

| Thing | Today |
|---|---|
| Stop client `daemon_client/stop.rs` | Sync `tungstenite`; Auth with **our** version; `StopDaemon{force:true}`; drain read with a 3 s socket timeout; a mismatched or dead daemon = `NoTarget` (nothing sent) — so tray 「데몬 끄기」 and app exit silently fail on a version mismatch. Callers: tray `tray/mod.rs:221` (Tauri blocking pool), `quit_app` `tray/actions.rs:327` (main thread). Neither closes the shell link first. |
| Daemon on `StopDaemon` | `connection_core.rs:1645-1683`: `shutdown_all` (awaited) → Ack → close. **The connection closes only after `shutdown_all`.** |
| Daemon refusals (`net` `ws.rs:338-438`) | Token checked before version; every refusal = an `Error` text frame + `close(None)` (no close code). |
| Shell refusal paths | Version: dialer reads `daemon.json` and refuses before dialing (`link/dialer.rs:170-182`). Wire refusal: `ShellHandshake` turns the `Error` frame into `Reject{Terminal}` (`link/handshake.rs:125-145`). Both reach routing as `Rejected{4001}` — indistinguishable. |
| Refusal text | `link/route.rs:193` `refusal_message`: every 4001 → 「실행 중인 데몬이 이 앱과 버전이 다릅니다」, so a token refusal shows the version text (also in `connect()`'s error, `connection.rs:951-970`). Refusal notices reach a debug log only (`connection.rs:1490-1494`). |
| Frontend surface | Event `daemon-connection-state` + pull `daemon_connection_state`: one word (`connected`/`reconnecting`/`down`). `ConnectionNotice.tsx` renders only `agentClient.connectionError`, whose only writer is `DaemonControl.start()`'s catch. |
| **Finding A** | A runtime loss (`reconnecting`) shows **no band at all**; neither does a refusal after boot. |
| **Finding B** | Boot = `daemon_start` (spawning ensure) then `connect()`. If `daemon_start` fails, `connect()` is never called, so **no shell link is added** and nothing retries in the background (revision 9 ① holds only when `daemon_start` succeeded). |

## 3. U5a — stop client and the one stop path

### 3-1. Stale pointers and gaps in the TRD U5 row (`trd-3-transport.md:404`)

| # | Row | Fact | Fix |
|---|---|---|---|
| S1 | 「`stop.rs:13-15` 「ack/응답을 읽지 않는다」」 | Comment now English, same lines. | Quote the English. |
| S2 | `machine.rs:323-347` · `:386-404` | Drifted: give-up = `schedule_retry` `:428-447`; terminal refusal = `on_connect_failed` `:393-414`. `max_attempts = 0` passes `Policy::check`. | Repoint. |
| S3 | `connection_core.rs:1542-1545` | Now `:1677-1683`. | Repoint. |
| S4 | 「primary peer」 · 「attach-only `current()`」 | U6 not landed; the one link = `LinkClient` (key `DAEMON = "local"`, `connection.rs:161`): close = `LinkClient::close()` `:483`, attach-only re-add = `LinkClient::ensure()` `:443`. | Name the real calls. |
| S5 | 「assert when called on a runtime thread」 | The tray worker runs on Tauri's blocking pool, which is inside a tokio context — a naive check fires there. Only the shell's own runtime threads (`engram-daemon-client`, `mod.rs:344-346`) can deadlock; the handle is private to `DaemonClient` (`mod.rs:183`). | Blocking wrapper on `DaemonClient`: spawn on `rt`, `recv_timeout`, `debug_assert!` on the thread name. |
| S6 | 「`notify(StopDaemon)` with a one-shot peer」 | `Peer::notify` needs a live connection (`peer.rs:449-474`); `Registry::add` needs a runtime. | Wait for `Connected`, then notify. |
| S7 | Target = today's `check_acceptable` (mismatch → `NoTarget`) | ADR-0343 d2 / ADR-0344 d3, d4, d6: authenticate with **the daemon's own version**; stop a daemon that is **not newer** than the app; tray uses the same path. `ShellHandshake` forbids echoing (`handshake.rs:108-109` "Fix C"). | Own `StopHandshake` (single-purpose echo exception). |
| S8 | 「keep the one-way fire」, no fallback | ADR-0343 d3 / ADR-0344 d3: identity-checked kill when the graceful stop does not close in time; promote `platform::testing::terminate_if_same` (`testing.rs:75`). The old "one-way fire" note was about not waiting for the Ack — still true. | Decided (§7 D6). |
| S9 | Deadline 「short」 | Today 3 s per blocking step. | 10 s then kill (band/tray); 3 s, no kill (exit) — §7 D7. |
| S10 | 「bring it up with `add` when turned back on」 | Tray 「데몬 켜기」 calls only `ensure_daemon` (`tray/mod.rs:210`). | Then `DaemonClient::ensure()`. |
| S11 | 「delete shell `tungstenite`」 · gate | Normal dep `src-tauri/Cargo.toml:105-107`; also used by two `#[ignore]` diagnostics `tests/stop_smoke.rs:162-215`. | Drop or rewrite them on dev `tokio-tungstenite`; 「shell gate 6」: no `tungstenite` in `cargo tree -p engram-dashboard -e normal --depth 1`. |
| S12 | 「re-aim `stop_smoke`」 | Imports the old path (`:22`); known failure T-52 (early return ~1.2 s = `stop.rs` `Err(_)` → `Timeout` arm, a close without a handshake). "First `Disconnected` after notify" covers it — T-52 may clear (미확인). | Re-aim; close T-52 if it passes. |
| S13 | — | `qa.md` row and 「셸 실 데몬」 lane key `daemon_client/stop.rs`; comments in `discovery/mod.rs:16`, `:204`, `:214`, `:451-457`, `:593-595`, `:612`, `discovery/tests.rs:1`. | Repoint / reword; narrow `FileReader` if unused. |
| S14 | §6-4 GUI ③ 「rejection popup」 | ADR-0342: band. | Band. |

### 3-2. Spec

1. **`link/stop.rs`** (no `tauri`): `resolve_stop_target(data_dir)` reads `daemon.json` once → target `{address, token, protocol_version, pid, start_time}`, or `NoTarget` (no record · broken · dead pid · **newer than the app**). `StopHandshake` sends Auth with the daemon's own version, is `Done` on any text frame whose top-level JSON key is `"Hello"`, and treats an `Error` frame as a terminal refusal (masked log). `stop_daemon(target, dialer, clock, deadline)`: one-shot `Registry<ShellWire>` (`max_attempts = 0`), drain its inbox on the same task; `Connected` → `notify(StopDaemon{force:true, kill_agents:true})` → the first `Disconnected`/`GaveUp` after it = `DaemonClosed`; refused/failed before `Connected` = `NotConnected`; deadline = `Timeout`; then `close()` and drain. The Ack is ignored. Nothing but `StopDaemon` is sent after `Hello` (pinned by a test).
2. **`DaemonClient::stop_daemon(plan)`** (new `daemon_client/shutdown.rs`): `LinkClient::close()` first (an intended disconnect — no band) → stop client → `Timeout` and plan allows → `platform::process::terminate_if_same(pid, start_time)` (`Killed`) → closed/killed: link stays closed · otherwise: `LinkClient::ensure()` at once (except `Exit`). `stop_daemon_blocking(plan)` for the sync callers (S5).
   - Plans: **`Exit`** 3 s, no kill, no re-add · **`User`** (band, tray, LLM, `DaemonControl.stop()`) 10 s, then the identity-checked kill.
3. **Callers:** tray Stop → `User`; `quit_app` → `Exit` then `app.exit(0)`; tray Start → `ensure_daemon` then `DaemonClient::ensure()` (S10). Delete `stop.rs`; move its wire-shape tests; S11–S13.
4. User-visible: tray 「데몬 끄기」 and app exit now also stop an **older** daemon (§7 D8); a stop that hangs for 10 s ends in a kill (§7 D6–D7).

## 4. U5b — two bands

### 4-1. The bands (user decision: 「버전 다름 하고 연결안됨」)

| Band | When | Text | Button |
|---|---|---|---|
| ① 연결 안 됨 | cold-start failure (first failed attempt) · runtime loss **after the first failed reconnect** (§7 D1) · token refusal · after the version band's 「데몬 끄기」 succeeded | 「데몬에 연결하지 못했습니다.」 + token refusal: 「데몬이 이 앱의 인증을 거절했습니다」 · or the existing `connectionError` reason (e.g. unwritable folder), as today | 「다시 연결」 |
| ② 버전 다름 | any version mismatch | 「데몬에 연결하지 못했습니다.」 + 「실행 중인 데몬이 이 앱과 버전이 다릅니다」 | 「데몬 끄기」 only when the stop is possible (daemon older than the app and not the stale-exe case); otherwise none |

- No band: connected · closed by us (frontend `disconnect`, tray stop, app exit) · the first reconnect attempt after a runtime loss has not failed yet.
- 「데몬 끄기」 asks first (ADR-0344 d2): 「이전 데몬을 끕니다. 실행 중인 에이전트가 멈춥니다 — 나중에 이어서 실행할 수 있지만 진행 중인 응답은 사라집니다」 · 「끄기」 / 「취소」 (in-app modal like `RestoreModal.tsx`). On success the band becomes ① (nothing starts by itself — ADR-0344 d3).
- While a button's command runs: 「연결 중…」 / 「끄는 중…」, disabled.
- ✕ (user decision 1): dismiss key = band kind + its cause text; hidden while the key holds; cleared on `connected`.

### 4-2. Shell → frontend data — extend the existing surface

One DTO for the event and the pull (`tauriTransport.ts` already feeds both through one handler):

```rust
// 제안 — ts-rs export
pub struct ConnectionStatus {
    pub state: ConnectionStateWord,   // "connected" | "reconnecting" | "down" (unchanged words)
    pub band: Option<Band>,
}
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Band {
    NotConnected { refused: bool },   // refused = the daemon refused the handshake (token)
    Version { can_stop: bool },
}
```

- `can_stop = daemon.protocol_version < PROTOCOL_VERSION && !stale_exe`. `stale_exe` (ADR-0344 d5 button rule) stays **shell-internal**: the mismatched daemon started after this app process (`daemon.start_time != 0 && > platform::process::current_process_start_time()`) → it was spawned from the folder's old exe, so stopping it would only respawn the same version. Non-Windows: `false`.
- Routing (`link/route.rs`) stays the only emitter: `Routed::State` → `Routed::Status`. Rules: `Connected` → no band · first failure of a handle that never connected → ① · `Disconnected` → no band until the next `ConnectFailed` → ① · `Rejected` → ② or ① `refused` (§4-3) · a close by us → no band, except the driver marks ① after a successful stop that began on ② · a re-add keeps the shown band until the new handle's first answer.
- `events.rs`: `connection_state(word)` becomes `connection_status(&ConnectionStatus)` (same single emit); `daemon_connection_state` returns the DTO; the driver's watch carries the DTO (`state()` stays). Frontend: `applyConnectionState(raw)` → `applyConnectionStatus(status)`, unknown `state` → `down`, unknown `band.kind` → ① without cause; `connectionBand` exposed next to `connectionError` and notified by `onConnectionStateChange`.

### 4-3. Token vs version refusal (user decision 2 — main's mechanism)

- Decide from what the shell did, never from the daemon's text (ADR-0342 d2).
- `LocalDaemonDialer::register` also returns an `Arc<RefusalSlot>` (twin of `CredentialSlot`); `resolve` writes `{daemon, pid, start_time}` before returning the version `DialFailure::Refused` (needs `Current::VersionMismatch` to carry `pid` / `start_time` — `endpoint.rs:47`, `:126`) and clears it on every other answer. The driver hands the slot to `RouteState::adopt`; on the current handle's `Rejected{4001}` a record → ②, none → ① `refused`. The dialer writes before the transport emits, so the actor always sees it.
- `connect()`'s error text (`answer_first`) uses the same decision. Texts move out of `route.rs:193` into one place.
- Rejected: matching the `Rejected.reason` prefix (a 3-3 daemon close reason could imitate it) · re-reading `daemon.json` on the actor (blocking, racy) · a shell close code ≥ 4100 (`peer.rs:2589-2594` asserts `DialFailure::Refused` carries 4001).

### 4-4. Actions and the LLM path

| Command (`__engramCmd`, window-local, no `help`) | Effect | Runs | Reply |
|---|---|---|---|
| `connection.status` | read | the surface | `{state, band, text}` (text = the band text, also when dismissed) |
| `connection.reconnect` | write | `DaemonControl.start()` = the boot start: `daemon_start` (starts the local daemon if none runs — §7 D2) then `connect()`, which re-adds the link (fresh backoff; works after a refusal) | status after the first answer · on ② → error 「버전이 달라 다시 연결해도 거절됩니다 — daemon.stop 을 쓰세요」 |
| `daemon.stop` | write | `DaemonControl.stop()` → new Tauri command `daemon_shutdown` → `DaemonClient::stop_daemon(User)`; no confirm dialog (§7 D5) | `{outcome: closed \| killed \| timeout (kill failed) \| not_connected \| no_target, status}` · newer daemon → `no_target` with a clue |

- Not on the daemon bus: in every state with a button the shell has no daemon link, so the bus cannot reach it (CLAUDE.md 「LLM-우선 제어」 — existing `__engramCmd` surface; ADR-0344 d7, ADR-0345 d5).
- `DaemonControl.stop()` today (link round trip + `disconnect` + `daemon_stop` pid kill without a handle-pinned check) becomes `daemon_shutdown`; the `daemon_stop` command goes if nothing else calls it.

### 4-5. Fixes for findings A and B

- A: band ① on a runtime loss (rule in §4-2).
- B: `DaemonControl.start()`'s catch also calls the attach-only `daemon_ensure` (result ignored), so a failed boot start still leaves a link that retries in the background and produces ② / ① from the shell.
- **Verify first** (start of U5b): reproduce A (kill the daemon while connected → no band) and B (make boot start fail → no link activity in the shell log) before changing anything; if either does not reproduce, drop that fix and record why.

## 5. Tests plan

- **Unit / in-memory** (`cargo test -p engram-dashboard --test lib_unit`; `MemoryNetwork` + `ManualClock`):
  - `link/stop.rs`: Auth carries the daemon's version (older, equal) · only `StopDaemon` after `Hello` · `Hello` matched by tag · `DaemonClosed` on the first `Disconnected` after notify, before the deadline · close read while the fake daemon pours fan-out · Ack ignored · `Timeout` · `Error` frame → `NotConnected` · no record / dead / newer → `NoTarget`, no dial.
  - `stop_daemon`: link closed first, no band · re-added after a failed stop, not after closed/killed · `Exit` never re-adds or kills · kill only after the 10 s deadline (fake killer) · blocking wrapper bounded, asserts off the shell runtime threads.
  - Dialer slot written on version refusal, cleared otherwise. Routing → band for each case in §4-1 (incl. "no band before the first failed reconnect", "① after a stop from ②", "close by us → none"); `can_stop` rule (older / newer / stale / `start_time == 0`); token refusal → ① `refused` and the token text in `connect()`'s error. DTO golden + binding sync.
- **Frontend (vitest):** band model (two kinds, cause precedence, unknown kind) · `ConnectionNotice` button, confirm, busy label, ✕ key · `tauriTransport` parses the DTO from event and pull · the three commands · boot-failure attach.
- **QA:** full regression per unit · 「셸 실 데몬」 lane (`stop_smoke --ignored`, row re-keyed — S13; record T-52) · GUI (`/qa full`): ③ older daemon (needs a previous release's daemon exe — feasibility 미확인) → ② with button → confirm → exits → ① · ⑤ tray off / app exit → no band; tray on → reconnects · ⑧ kill the daemon → ① after the first failed retry → 「다시 연결」 starts a daemon and connects · ⑨ rewrite the token in an isolated `daemon.json` → ① with the token text → restore → 「다시 연결」 connects · ✕ stays hidden across retries. Reviews: `/review code full` per unit.

## 6. Units (sequential — shared files; each ends green)

| Unit | Scope |
|---|---|
| **U5a** | §3-2 in full: `link/stop.rs` · `daemon_client/shutdown.rs` (plans, 10 s + identity-checked kill, F4 re-add) · promote `terminate_if_same` into `platform::process` (production API of an existing crate — report to the user) · tray Stop / Start, `quit_app` · delete `stop.rs` · drop `tungstenite` + 「shell gate 6」 · `stop_smoke` re-aim · `qa.md` / comment follow-ups. No UI change. |
| **U5b-1** | Shell side: verify A and B (§4-5) · refusal slot · `Band` status in routing · driver watch + port + DTO + binding + pull · texts · `daemon_shutdown` command · frontend transport parses the DTO (contract changes in one commit; the band still renders `connectionError`). |
| **U5b-2** | Frontend: band model + `ConnectionNotice` (button, confirm modal, busy label, ✕ key) · `connectionCommands.ts` · `DaemonControl.stop()` → `daemon_shutdown` (remove `daemon_stop` if unused) · boot-failure attach · strings in `src/i18n/ko.ts` · GUI QA. |

Docs: main TRD row 404 → pointer here, §6-4 ③ wording · `docs/reference/architecture.md` (connection surface, bands) · `docs/tracking.md` T-52.

## 7. Decisions

### 7-1. User decisions

- **U1** (2026-11-08) Rev 9 ⑥ closed: a band the user closed with ✕ keeps today's behaviour (hidden while the same reason holds; memory clears on connect). No tray item, nothing added. (User: 「구지 뭘 추가할 필요는 없어보이는데?」)
- **U2** (2026-11-08) Token refusal band text = cause only, short: 「데몬이 이 앱의 인증을 거절했습니다」. Version-mismatch text unchanged. (User: 「이유만 짧게」). How the shell tells token vs version refusal apart is main's call — you propose. → §4-3.
- **U3** (2026-11-08) T-80 (reconnect storm when a slot spews output; daemon closes on full send queue) is fixed inside 3-3, not as its own topic. Not in U5.
- **D1** (2026-11-09, was Q1) On a runtime loss the band appears after the first failed reconnect attempt. User: 「끊기고 한번 실패해서 띄우는게 맞아보이는데」. Convention: Zulip re-adds its banner on a failed retry (`popup_banners.ts:39-47`); VS Code Remote shows its dialog after 40 s — `docs/research/disconnect-banner-dismiss-reconnect-2026-11-05.md`.
- **D2** (was Q2) 「다시 연결」 starts the local daemon if none is running, then attaches — same as the boot start. User: 「A로 하자」. Convention: wezterm `no_serve_automatically` default false (wezterm.org/multiplexing.html), gpg-agent auto-start on demand (GnuPG docs), `emacsclient -a ""`, VS Code Remote-SSH troubleshooting docs; counter-examples: tmux / zellij `attach` do not start a server.
  - **Carry-over to the multi-daemon PRD** — user: 「나중에 멀티데몬 할 경우에도 한개가 타겟이 되어야되는데 그 데몬을 붙이면 되겠지」: the button targets one daemon; a remote daemon can only be attached, not started.
- **Two bands** (was Q5/Q6) User: 「버전 다름 하고 연결안됨」 → §4-1. Dropped texts: "connection lost / reconnecting", "old daemon stopped", "daemon is newer — use the newer app", "stale exe — re-extract".

### 7-2. Main decisions

- **D3** One problem vocabulary of two bands; the shell sends `state` + `band` (§4-2); `newer` / `stale_exe` stay shell-internal (only `can_stop` crosses).
- **D4** Token vs version via a per-registration refusal slot (§4-3).
- **D5** (was Q7) The LLM `daemon.stop` runs without the confirm dialog — precedent: `__ENGRAM_DAEMON__.stop()` has none.
- **D6** (was Q3) Identity-checked kill fallback for band and tray — already decided by ADR-0343 d3 / ADR-0344 d3, d6; the old "one-way fire" note concerned not waiting for the Ack, which stays.
- **D7** (was Q4) Graceful deadline 10 s, then the kill (band / tray / LLM). Convention: `docker stop` 10 s (docs.docker.com), supervisord `stopwaitsecs` 10 s (supervisord.org); PM2 1.6 s; Windows SCM 5 s unconfirmed; k8s 30 s / systemd 90 s are server settings. App exit: 3 s, no kill.
- **D8** (was Q8) App exit (tray 「종료」, ADR-0026) also stops an older-version daemon — today that stop silently fails on a mismatch.
- **D9** 「다시 연결」 = `DaemonControl.start()` (spawn + re-add); no new shell command for it. `daemon.stop` = new `daemon_shutdown`.
- **D10** LLM path = window-local `__engramCmd` `connection.status` / `connection.reconnect` / `daemon.stop` (§4-4).
- **D11** Findings A and B are fixed in U5 after a verify-first step (§4-5).
- **ADR follow-ups (main files them):** amend ADR-0344 — decisions 4–5 keep the button rule but lose their guidance wording (two bands); record D2 against ADR-0021 (a user-pressed 「다시 연결」 may start the daemon; automatic reconnects still never do); D6–D7 against ADR-0343 d3 / ADR-0271 d5.

## 8. Open

- None from the user. Wording of the `connection.reconnect` error clue and the dismiss-key detail are main's to settle in review.

## 9. Risks

- **Kill during cleanup** — the daemon closes only after `shutdown_all` (duration unmeasured); with many agents 10 s may cut it. Measure in GUI QA during U5a.
- **Echo exception** — sending the daemon's version defeats its check by design (ADR-0343 d2); contained by a separate handshake type and a test that only `StopDaemon` follows `Hello`.
- **Older daemons' `Hello`** (v3–v7) not checked (미확인); tag-only matching limits the risk.
- **Stale-exe rule** only decides the button: a user who starts an old daemon by hand after the app started gets ② without a button.
- **Token text over-reach in 3-2** — a daemon `auth timeout` (15 s) and a wire version refusal (only if `daemon.json` lies) also show the token text.
- **Contract change** of the connection-state event and pull — all readers change in U5b-1 together.
- **Band churn under T-80** until 3-3 (D1 damps it).
- **「다시 연결」 spawns** — after an external kill (Task Manager) the button brings a daemon back; intended by D2.
- **`quit_app` blocks** the main thread up to 3 s before exit (as today).
- **LLM reach** — `__engramCmd` is reached via cdp; release exposure 미확인.

## 10. Review record

(empty)
