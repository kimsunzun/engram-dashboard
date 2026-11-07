# Handover — 3-2 U4 (production switch) — after U3c

From the U3c coder. U3c is committed as `0d4f79d1` (review deep, 3 rounds → PASS; QA PASS). Line numbers below are from
`0d4f79d1`. Re-grep before relying on any number. Everything this memo calls a fact was read from code at that commit;
the TRD is quoted where it is the source.

Read first: the TRD §6-2 U4 row (`grep -n '^| \*\*U4\*\* | 운영 생성자' docs/process/S21-crate-boundaries/trd-3-transport.md`
— line ~403, 20 KB lines, cut with `cut -c`), TRD line 24 (revision 9 — it overrides §7 where they differ), §7 (~456–472),
the fate table "셸 시험 운명" (~409–418), ADR-0344 … ADR-0347, and the U3a→U3b memo
(`.claude/handoff/attachments/20261105-s21-3-2-u3a-handover-for-u3b.md` — §1 and §4 still hold except where this memo
says otherwise). U3c review trail: `.claude/handoff/attachments/20261106-s21-3-2-u3c-review/`.

## 1. What V2 is now (`src-tauri/src/daemon_client/connection_v2.rs`)

Compiled for tests only (`mod.rs:21-22` `#[cfg(test)] mod connection_v2;`). Built only by `DaemonClient::new_on_link`
(`mod.rs:369`). Module doc (`:1-63`) is the contract — read it whole; it still says "Not in production yet" (`:3-5`).

- **Callers' side `LinkClient`** (`:183`): `start` (`:258`) = `build` (`:271`) + spawn the actor. Holds the registry
  clone, `closes` (close epoch), `cmd_tx`, `state_rx`, the **gate** (`Arc<Mutex<Gate>>`), `opened_rx: watch<()>` (wake-up
  only), the usage interest, the clock.
  - `send_command` (`:436`): reads `closes` first → `registry.get` → `require_live` → `opened_generation(peer, closes)`
    (`:543`) → for `RefreshUsageLimits` the forced usage subscribe under the usage lock (`write_refresh_subscribe` `:593`)
    → `request_in(generation, cmd)`. The wait is bound to the generation live at call (never moves to a later
    connection — ADR-0195), wakes on the `opened` watch and on peer-state changes, bounded by
    `link_policy().request_timeout` on the injected clock, `select!` is `biased` toward the wait (`:574`).
  - `send_fire_and_forget` (`:473`): sync; decides under the gate — closed (caller's `closes` newer than
    `gate.handle_closes`) → drop `why="closed"`; `opened == g` → `notify_in(g)` under the gate; else push to
    `gate.pending` (cap `QUEUED_FIRES_MAX` = 512 `:126`) → drained by the actor on `LinkUp(g)`.
  - `unsubscribe` · `request_replay` · `nudge_usage` → actor commands; `usage_nudger()` → `UsageNudger` (weak sender,
    `:210`) for the shrink-deferral timer.
- **Gate** (`Opened` `:131`, `Gate` `:140`, `lock_gate` `:151`): `{ opened: Option<Opened{generation, closes}>, pending:
  VecDeque<(Generation, AgentCommand)>, handle_closes }`. Leaf lock (poison-recovering `base::sync::lock`): nothing else
  is locked while it is held; registry and usage lock are read before it; only non-blocking `notify_in` under it;
  logging after release. `handle_closes` is written only in `Driver::add` (`:760`), under the gate, **before**
  `Registry::add` (a caller that sees the new peer live must not read the previous handle's epoch).
- **Actor commands `DriverCmd`** (`:220`): `IsLive` · `Add{closes, first}` · `Close` · `Unsubscribe` · `RequestReplay` ·
  `UsageNudge` · `#[cfg(test)] CurrentHandle`.
- **Ports** (`:165`): `events` · `names` · `router` · `windows` · `inbound` (command table slot) · `refusals`
  (`RefusalAlerts`) · `usage` (`SharedUsageInterest`) — all the `DaemonClient`'s own objects (`new_on_link` builds them
  once and passes clones), so `install_command_table` · `refusal_alerts()` · `usage_interest()` see the same objects.
- **Actor (`Driver` `:638`)** — `apply` (`:1391`):
  - `LinkUp(g)` → `on_link_up` (`:1248`): `open = g`; usage `on_socket_open(g)` + on-open `UsageSubscribe` under the
    usage lock; `register_commands` (`:1311` — `RegisterCommands` queued with `request_in(g)` via `start_now` `:1513`,
    reply read on a spawned task → `report_registration` `:1525`: daemon `Error` → warn + refusal box; timeout → warn
    `after_secs`); then under the gate: drain `pending` (`== g` written, older dropped, newer kept) and set
    `opened = {g, gate.handle_closes}`; then signal the watch.
  - `LinkDown` → flights cleared, unknown tags reset, `dropping` cleared, usage `on_socket_lost`, gate `opened = None`.
  - `ToDriver`: skipped on a `dropping` generation (`:1430`) and, for `CommandRequest`, while `current.is_none()` (closed
    tail, `:1438`) — both comments carry the daemon evidence (`command_delivery.rs` `drop_origin` doc: owner-side loss
    ends the round trip at the daemon's 7 s delivery deadline as `TIMEOUT`).
    Otherwise `on_driver_event`: replay events · `CommandRequest` → `accept_inbound` (`:1145`, outcome via
    `outcome_sink` `:1169` → `send_command_outcome(peer, g)`) · broken `CommandReply`/envelope → warn (class/line/column
    only) · `UsageLimitsUpdated` → `relay_usage_snapshot` (`:1187`).
  - `Notice` → **debug log only** (`:1405-1407`) — see §3.
- **Policy**: `link_policy()` (`:158`) = transport defaults + `max_attempts = u32::MAX` (ADR-0346). Nothing else injected
  yet — request timeout is the transport default **8 s**.
- Transport contract added in U3c: `Peer::request_in` doc "the first poll queues the request" + test
  `peer::tests::the_first_poll_of_request_in_queues_the_request` (`crates/engram-dashboard-transport/src/peer.rs`).

### `#[cfg(test)]` seams U4 removes (all in `src-tauri/src/daemon_client/`)

| Where | What |
|---|---|
| `mod.rs:21-22` | `#[cfg(test)] mod connection_v2;` |
| `mod.rs:56-63` | test-only imports (`NoDaemonEvents`, `NoRefusalNotice`, `RefusalNotice`) — keep only what remaining test constructors need |
| `mod.rs:172-187` | `OnTheLink` discovery stub (only `new_on_link` uses it) |
| `mod.rs:270-274` | `#[cfg(test)] link: Option<connection_v2::LinkClient>` field |
| `mod.rs:368-415` | `#[cfg(test)] new_on_link(rt, inner, source, clock, events, names, refusal_notice)` |
| `mod.rs:342` · `:448` · `:478` · `:517` | `#[cfg(test)] link: None` in the other constructors (`:517` = `new_real_with_owned_runtime`) |
| `mod.rs:584` connect · `:661` ensure · `:932` close · `:1011` send_command_with_origin · `:1055` unsubscribe · `:1086` send_fire_and_forget · `:1106` nudge_usage_interest · `:1206` request_replay | `#[cfg(test)] if let Some(link) = &self.link { … }` branches |
| `mod.rs:1165` · `:1176` | shrink-deferral timer: `link_nudger` capture + branch |
| `connection_v2.rs:189` · `:192` | `LinkClient` test fields `dialer`, `routable_tx` |
| `connection_v2.rs:249` · `:707` | `DriverCmd::CurrentHandle` + its arm |
| `connection_v2.rs:311` · `:313` | `build` initializers of those test fields |
| `link/dialer.rs:104` · `link/endpoint.rs:79` | test helpers (`registered`, `with_liveness`) — keep as test-only |
| `link/mod.rs:26` | `test_log` (test-only log capture) — keep |

Other `#[cfg(test)]` in `mod.rs` (`:126-129` `NoAgentNames`, `:291` `new`, `:298`, `:317`, `:349` `with_refusal_notice`,
`:357` `with_agent_names`, `:420` `new_real`, `:427`, `:456`) are the **old driver's** test constructors — they go or are
rewritten with the old suite (§2).

## 2. What U4 must do (TRD §6-2 U4 row, quoted items in order)

1. **Production constructor → V2.** `new_real_with_owned_runtime` (`mod.rs:487`, called from `src-tauri/src/lib.rs:235`)
   builds the owned runtime and the old `Lifecycle`/`RealDiscovery`. Switch it to `LinkClient::start` with the real ws
   dialer (`engram_dashboard_transport::ws::WsDialer` — see `link/route.rs:1188` test for construction) and the real
   endpoint source (`link::endpoint::LocalEndpointSource` `:64`), the real clock (`SystemClock`), ports
   `TauriEmitter` · `TauriRefusalBox` · the `router`/`registry` from `lib.rs`. Then the `link` field becomes
   unconditional and the old-path bodies after each branch go.
2. **Policy injection (§7 with revision 9):**
   - reconnect count: **unlimited** (ADR-0346 — already in `link_policy`; §7's "D14 ① inject 5" is retired by rev 9 ②);
   - request timeout: **30 s common** (D15 ① — ADR-0181; re-measure spawn real time in QA) → `policy.request_timeout`;
   - backoff 500 ms ×2 cap 10 s jitter 0.2 · `live_dwell` 30 s · connect/handshake 10 s · keepalive 20 s/50 s · write
     deadline 5 s · outbound queue 512 (shell) · per-peer inbound 512 / 16 MiB (shell, D11 provisional): transport
     defaults — check each against `Policy::default()` (`crates/engram-dashboard-transport/src/policy.rs:191-213`);
   - handshake refusals: D20 ①, but **token `Retryable` retired** (rev 9 ③) — every refusal terminal on the local path
     (already so in V2: `retryable_refusal` + `stop_refused`).
3. **`REPLY_TIMEOUT` removal** — `src-tauri/src/commands/agent.rs:283-294` (`forward_daemon_command`'s 30 s
   `tokio::time::timeout` wrapper; its text is already `link::errors::reply_timeout_text`). With 30 s in the policy the
   transport's `RequestError::TimedOut` gives the same text through `request_error_text`.
4. **Delete the old driver** — `connection.rs`, `lifecycle.rs`, the old discovery path (`DaemonDiscovery` /
   `RealDiscovery` in `mod.rs`; `commands/popout.rs:738-750` implements it in a test) — and the fate table's **"퇴역"**
   tests with a reason line naming the transport test that covers each (hand-written lifecycle · generation guard ·
   cancel races: Fix B, (a)–(e), T4 `reconnect_guard`, bump, read_live/handshake-window close — in `daemon_client/tests.rs`).
   **Before deleting, move what V2 and others still import from `connection.rs`:**
   - `connection_v2.rs:91-94`: `apply_replay_event`, `known_epoch`, `registration_command`, `HandshakeError`,
     `ReplayFollowUp`, `REPLAY_DEADLINE`, `REPLAY_DEADLINE_TICK`, `SEND_FAILED_PREFIX`, `UNSENT_ON_DISCONNECT`;
   - `link/errors.rs:18`: `PENDING_SUPERSEDED`, `SEND_FAILED_PREFIX`, `SENT_OUTCOME_UNKNOWN`, `UNSENT_ON_DISCONNECT`;
   - `commands/view_bus.rs:111`: `shell_owner_advert`;
   - `protocol_state.rs:717` (test) `SENT_OUTCOME_UNKNOWN`; doc refs `:267` · `:287` · `:290`;
   - `tests/layout_commands.rs:42-44`: `accept_inbound`, `outcome_sink`, `registration_command`, `ConnectionCommand`,
     `OutcomeSender` (V2 has its own `accept_inbound`/`outcome_sink` as `Driver` methods — that harness must be rewritten);
   - `tests/daemon_client_pending.rs:30-32`: `register_pending`, `CommandReply`, `PENDING_SUPERSEDED` → the TRD says
     **rewrite to M1** (overlapping request id = both fail — transport TRD §4-1);
   - `tests/daemon_client_replay.rs:39-41`: `apply_replay_event`, `plan_subscribe_refusal`, `ReplayFollowUp`;
   - `link/wire.rs:68`: comment referencing `connection::SERIALIZE_FAILED_PREFIX` (TRD: reword).
   - the count constant `MAX_RECONNECT_ATTEMPTS` (`connection.rs:93`) and the "5회" prose in `src/api/agentClient.ts:191`
     and `src/api/daemonControl.ts:92` (rev 9: they go with the old driver).
5. Shell `tokio-tungstenite` → dev-dependency (`src-tauri/Cargo.toml:115`; dev re-declaration `:131`).
6. **D23 ①** — drop the token from `DaemonInfoDto` (`src-tauri/src/commands/discovery.rs:41`) for all three commands
   that return it (`discover_daemon` · `daemon_start` · `read_daemon_info`), including the direct-connect `wsTransport.ts`
   Auth handling.
7. Docs: `CLAUDE.md:50` transport entry ("운영은 아직 옛 연결 코드"), `docs/reference/architecture.md` transport entry,
   `link/mod.rs:9-12` header ("Production does not run on this yet"), `connection_v2.rs:3-5` module doc, and decide the
   final module name (`connection_v2` is a working name).
8. Test fate: the V2 suite is in `connection_v2/tests.rs` (58 tests incl. copies of old-suite usage/replay/A1/A2 tests);
   the old suite (`daemon_client/tests.rs`, 97 tests) is deleted per the fate table — "옮김" rows are already copied
   (U3b/U3c reports list which); "다시 씀" rows (backoff/reconnect/exhaustion/port move on paused tokio time) must be
   rewritten on `ManualClock` + `MemoryNetwork` if not covered yet; ADR-0195 row = transport + one shell adapter test
   (commands fail at once while disconnected).
9. QA per the row: R7 real socket (shell consumer stalled → daemon write deadline, disconnect, full-replay count) and
   the R7 mirror (daemon read stalled 8 × 5 s → shell big paste beyond B + M → shell write deadline 5 s), `/qa full`
   (GUI §6-4 ①–⑤), `/review code full`.

## 3. User-visible differences V2 brings (check with the user before the switch)

| # | Old driver | V2 | Where |
|---|---|---|---|
| 1 | first connect failure → Down, no background retry | Down at once **and** keeps re-reading `daemon.json` every backoff forever; a daemon started later connects by itself (rev 9 ①, ADR-0346) | `link/route.rs` cold-start rule |
| 2 | reconnect gives up after 5 → Down | never gives up — stays "reconnecting" while the daemon is away (log throttled — `FailureLog`) | `link_policy` |
| 3 | token mismatch = one failed attempt; version → same path | every handshake refusal terminal; UI text = `refusal_message(code)` — today the **version text for every refusal** (4001 shared with identity refusal — pending decision) | `link/route.rs:193` |
| 4 | — | refusal/budget **notices are only a debug log** (`Routed::Notice`, `connection_v2.rs:1405-1407`); the band / 「다시 연결」 / 「데몬 끄기」 UI (ADR-0344/0345) is not wired — U5 | |
| 5 | round trips: no timeout except `forward_daemon_command` 30 s | transport request timeout on **every** round trip — 8 s now, 30 s after D15 ① injection; text `reply_timeout_text(after)` | §2 item 2–3 |
| 6 | registration: waited until disconnect | registration reply under the request timeout (8 s now / 30 s after injection); timeout → warn, **no refusal box** | `report_registration` |
| 7 | command in the first-connect gap → `NOT_CONNECTED` | a round trip while the connection is live but not opened by the actor **waits** (≤ request timeout); new text on timeout: `"명령 송신 실패: the daemon connection did not finish opening within 8s"` (prefix + English) | `opened_generation` |
| 8 | fire-and-forget while the window is closed → dropped; queued behind on-open writes once open | same rule, but queued in the gate (cap 512) for a live, not-yet-opened connection and written on open; **no order between fire-and-forgets and Unsubscribe/RequestReplay** (U3b parity) | module doc |
| 9 | — | shell pings (20 s idle → ping, 50 s → dead) and a 5 s write deadline: the shell now notices a dead daemon / stalled write itself (R7 mirror) | §7 |
| 10 | backoff without jitter, reset on success | jitter 0.2, `live_dwell` 30 s; a flapping link retries at the base delay (rev 9 ⑨) | transport |
| 11 | close → Down | close shows a brief `reconnecting` then `down` (routing shows every loss as Reconnecting, then `Closed` → Down); the frontend acts only on the `connected` edge — confirm no visible flicker in GUI QA | `link/route.rs` Disconnected arm |
| 12 | usage `socket_epoch` = lifecycle socket counter | = transport connection generation (monotonic across re-adds, ≥ 1) — the webview pull fences on it | `on_link_up` |
| 13 | daemon command on a link being dropped / closed: never read | skipped (same outcome: the daemon times it out at 7 s → `TIMEOUT` to the LLM/CLI caller) | `apply` skip arms |
| 14 | Korean log lines | new code logs in English (rev 9 ⑦) | |
| 15 | first-auth window on the daemon 1 s | 15 s (ADR-0347 — daemon side, already landed) | |

## 4. Traps and test infrastructure learned in U3c

- **Gate = leaf lock.** Never take the registry (roster lock), the usage lock or log while holding it; read the peer
  first. The one-critical-section rule in `on_link_up` (drain pending **and** set `opened` under one `lock_gate`) is what
  gives "one caller's fire then round trip keep their order" — a drain after the publish signal is **not caught by any
  test** (no deterministic interleaving without a production hook — round-3 G2); keep it structural.
- **Close epoch**: callers load `closes` before anything else; the gate's `handle_closes` is the add epoch, set before
  the peer exists. `Opened.closes` (round trips) and `gate.handle_closes` (fires) carry the same value.
- **Actor never awaits a request** — the registration is queued by polling its `request_in` future once on the actor
  (`start_now`, no-op waker) and finished on a spawned task; relies on the transport contract pinned in `peer.rs`.
- **Usage-lock hold technique** (stop the actor inside its `LinkUp` step, before the gate): `let usage =
  h.client.usage_interest().lock();` then push `hello`, wait for `PeerState::Live` — the peer is live but not opened.
  Needs `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]` and
  `#[allow(clippy::await_holding_lock)]`. Used by 7 tests in `connection_v2/tests.rs`.
- **Hand-polled futures**: `let mut call = Box::pin(h.link().send_command(..)); call.as_mut().poll(&mut
  Context::from_waker(Waker::noop()))` — enter a wait, change the world, poll again; with `link::test_log::capture`
  around a poll to capture a warn emitted on the caller's path (thread-local subscriber).
- **`biased` select** in `opened_generation`: a re-poll after the manual clock passed the request timeout must not be
  decided by a random branch.
- **Single-thread runtime ordering**: on `#[tokio::test]` (current_thread) the actor cannot run between two sync calls
  with no await in between — used to make "close then write" deterministic.
- **Log assertions**: filter `level=<LEVEL> ` + a structured field (e.g. `why="closed" `, `generation=2 `,
  `error=<text> ` — `%` fields print without quotes, `&str` fields with quotes).
- **Mutation checks** were done by copying the file to scratch, deleting the check line with `sed`, running the single
  test, and copying the file back (`grep -c` to confirm).
- Harness pieces (`connection_v2/tests.rs`): `Record` (`:45`), `wait_for` (`:134`), `joined` (`:143`), `Harness`
  (`:148` — now takes a `RecordingRefusals`), `Bench` (`:644` — unspawned actor; drive `route_item`/`apply`; bench peer
  is never live, so writes are refused and logged), `HeldView` (`:1535`), `reconnected` (`:1623`), `CountingSpawner`
  (`:2216` — counts inbound tasks that finished, i.e. outcome handed over), `labels_until` (`:2255`). Old-suite helpers
  made `pub(super)`: `bus_table`, `bus_request`, `relayout`, `usage_update`, `RecordingEvents::then_on_connected`.
- **Bindings restore** after any `lib_unit` run (rewrites ~24 `src-tauri/bindings/*.ts` with identical bytes):
  `for f in $(git status --porcelain -- src-tauri/bindings | awk '{print $2}'); do [ -z "$(git diff --numstat -- "$f")" ] && git checkout -- "$f"; done`.
  A filtered run (`… --test lib_unit connection_v2`) does not rewrite them.
- **Name diff**: `--list` names end in `: test`; run output lines are `test <name> ... ok|ignored[, reason]`; normalize
  the ` - should panic` suffix and strip the ignore reason. Base `1d3a8486` = 1455 shell `lib_unit` names, 383 transport
  unittests; at `0d4f79d1` shell = 1480 (1477 pass + 3 ignored), transport 384.
- Clippy is **not installed** for toolchain 1.95.0 on this machine (`rustup component add clippy`) — nobody ran it on
  U3a–U3c.
- Write edit scripts with the Write tool (`py` for multi-block splices); heredocs fold backslashes.

## 5. Unfixed recommended items (carried)

- **Drain-after-publish untested** (round-3 G2) — structural only; main documents the one-critical-section requirement.
- **Stale pending fire**: a fire queued by a caller that saw the peer an instant before it was lost/closed stays in
  `gate.pending` until the next `LinkUp` drops it as older — never written, bounded by the cap.
- **`CHANNEL_GONE` waiter ending is undrivable** (the caller's `LinkClient` keeps the actor alive) — defensive arm.
- **F1 "loss while waiting" test** (`a_waiting_command_fails_as_unsent_when_its_connection_is_lost`) uses a 50 ms real
  sleep to let the call reach its wait (a miss fails visibly with a different text, not a false pass).
- **Publish-before-on-open window is closed for callers, not for the daemon**: on-open writes and caller writes are
  ordered, but the registration reply can still time out (8 s) on a slow daemon → no box (difference #6).
- **Wording batch** (main's, may still be open): `send_command`/`opened_generation` "old driver's text",
  `SharedUsageInterest` lock-order doc (the gate is not in it), `report_registration` "the next one registers", the
  fire-order-vs-Unsubscribe line in reports.
- Still pending user decisions from U3a (unchanged): flap retry at the base delay (rev 9 ⑨ adopted), a handshake
  `Error` frame is terminal even for a transient daemon `auth timeout`, the 4001 UI text (difference #3), whether a
  closed band shows again on the next failure (rev 9 ⑥).
