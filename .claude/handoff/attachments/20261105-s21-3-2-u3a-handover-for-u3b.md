# Handover — 3-2 U3b / U3c (after U3a)

From the U3a coder. U3a passed review (deep, 2 rounds) and QA; main applied comment wording; commit pending a user
decision. Line numbers below are from the working tree after U3a (old driver files are unchanged from base `c4bb4954`,
so `connection.rs` lines are stable; `mod.rs` lines include U3a's additions). Re-grep before relying on any number.

Read first: the U3 coder brief (`scratchpad/u3-coder-brief.md`), `scratchpad/u3a-review/fix-r1.md` and `fix-r2.md`,
`.claude/handoff/attachments/20261105-s21-3-2-u2-handover-for-u3.md` (§3 facts, §6 risks, both addenda), the TRD row
U3 and the "셸 시험 운명" fate table (`docs/process/S21-crate-boundaries/trd-3-transport.md` §6-2, ~line 402–419 —
lines are 20 KB long, extract with `sed -n`/`grep -o`).

## 1. What U3a built (you build on this)

Files: `src-tauri/src/daemon_client/connection_v2.rs` (driver), `connection_v2/tests.rs` (driver tests),
`link/route.rs` · `link/handshake.rs` · `link/dialer.rs` (changed), `link/test_log.rs` (new, test-only log capture),
`daemon_client/mod.rs` (test constructor + branches), `daemon_client/tests.rs` (`RecordingEvents` made `pub(super)`).

- **Compiled for tests only.** `mod.rs`: `#[cfg(test)] mod connection_v2;`, `#[cfg(test)] link: Option<LinkClient>`
  field, `#[cfg(test)] new_on_link(rt, inner_dialer, endpoint_source, clock, events, names)` (`mod.rs:360`), and
  `#[cfg(test)] if let Some(link) = &self.link { … }` branches at the top of `connect` · `ensure` · `close` ·
  `send_command_with_origin` · `send_fire_and_forget`. Ports go in through `new_on_link` only — `with_agent_names` /
  `with_refusal_notice` do not reach the link driver. U4 removes the `cfg(test)`s.
- **`LinkClient` (callers' side)** holds `key` (`DaemonKey("local")`), `rt`, `source`, a `Registry<ShellWire>` clone,
  `closes: Arc<AtomicU64>` (close epoch), `cmd_tx` (unbounded), `state_rx`, and `#[cfg(test)] dialer`.
  `start()` = `build()` + `rt.spawn(driver.run(..))` (`connection_v2.rs:113`, `:128`). `build()` returns
  `(client, driver, cmd_rx, routable_rx)` unspawned — the test bench uses it.
- **`DriverCmd`** (`:95`): `IsLive{reply}` · `Add{closes, first}` · `Close`. Callers' round trips (`send_command` →
  `require_live` + `Peer::request` + `link::errors` texts; `send_fire_and_forget` → `notify`) run on the CALLER's task
  through the registry clone — never on the actor.
- **Actor (`Driver`)** fields: `registry`, `dialer: Arc<LocalDaemonDialer>`, `route: RouteState` (clock injected),
  `routable_tx` (cloned into every pump), `registrations: HashMap<HandleToken, Address>`, `current: Option<HandleToken>`
  (the key's current handle; cleared by `close()`), `first: Option<(token, oneshot)>` (the caller waiting for the newest
  handle's first answer), `closes`, `state_tx`, `events`, `names`. Its only awaits are its two channels.
  - `add(closes, first)` (`:371`): stale epoch → `Err(TaskGone)`, build nothing; `is_live()` → `Ok`, build nothing;
    else ONE step: `HandleToken::mint` + `dialer.register` (fresh address) + `route.adopt` + `registry.add` +
    `spawn_pump` + `registrations.insert` + `current = Some` + replace `first` (old waiter gets `TaskGone`) + apply
    `adopt`'s output. A re-add of the key takes this same step (it already works generically; U3b brings its tests).
  - `is_live()` (`:355`): current token open as routed (`RouteState::is_open`) OR peer `Live` — the supervisor publishes
    `PeerState` after it queued `Connected`, so routed state can be ahead (review G2).
  - `close()` (`:366`): `current = None` + `registry.remove`. Shows nothing itself — the `Closed` routes `Down`.
  - `route_item(key, token, item)` (`:418`): `answer_first` → if `retryable_refusal` (a `ConnectFailed{Rejected{
    Retryable}}` on the current token) then `stop_refused` (warn + `close()`) and push `Notice(Refused)` FIRST → then
    `route(..)`. Returns the whole `Vec<Routed>`; `on_routable` applies it. Unit-testable without spawning.
  - `Ended`: unregister that token's address (current or replaced), answer its waiter `TaskGone`, apply `route_end`.
  - Teardown when the client drops: `registry.remove` + unregister every registration (pumps get no `Ended` then).
  - `apply()` (`:506`): `State` → `state_tx.send_replace` + `events.connection_state`; `Broadcast` → `emit_broadcast`;
    `Notice` → debug log (UI wiring is U4); **`LinkUp`/`LinkDown` → nothing; `ToDriver` → debug log "not handled yet";
    `Output` → dropped** (`:524-531`). U3b/U3c fill these arms.
- **Routing changes in U3a** (`link/route.rs`): a handle that never connected shows its first `ConnectFailed` as `Down`
  (cold start, TRD rev 9 ①); failure log throttled per handle by `FailureLog` keyed on `FailureKind`
  (`Unreachable(UnreachableClass)` | `Refused{code}`, class read from dialer consts + transport literals — log key
  only); `RouteState::new(clock)`; `is_open(key, token)`.
- **Policy**: `link_policy()` = defaults with `max_attempts = u32::MAX` (ADR-0346). Request timeout is still the
  transport default (TRD §7 values are U4).

## 2. Old-driver pieces still missing in V2

Old driver = `src-tauri/src/daemon_client/connection.rs`; `DaemonClient` = `mod.rs`. Every one of these is silently a
no-op or `NOT_CONNECTED` on a V2 client today (they go through `lifecycle.current_cmd_tx()`, which is `None` there).

### U3b — subscribe / unsubscribe / replay actor + output relay
- `DaemonClient::unsubscribe` `mod.rs:1032` (sync, `try_enqueue`) → old arm `connection.rs:1165-1168` (wire
  `Unsubscribe`).
- `DaemonClient::request_replay` `mod.rs:1165` (oneshot gen) → old arm `connection.rs:1206-1268`: `router.targets`
  empty → no Subscribe, drop reply (`:1214-1220`, the A1 safety); `known_epoch` (`:1724`); `flight.request_replay`;
  send `Subscribe{after_seq:None}` if `send_now`; **send failure ⇒ drop the connection** (`:1237-1262`); reply gen.
- Replay events (`ToDriver` `SubscribeAck` · `ReplayComplete` · `SubscribeFailed`) → `apply_replay_event`
  (`connection.rs:1635`, call site `:1011-1053`, follow-up Subscribe with the same "send failure ⇒ disconnect" rule).
- Output frames (`Routed::Output`) → `frame_relay::relay_binary_frame` (`frame_relay.rs:69`, call site
  `connection.rs:1117-1136`; `FrameRelay::Disconnect` ⇒ drop the connection) with `output_channel::send_to_windows`
  (`output_channel.rs:62`), `router`/`registry` = `DaemonClient` fields.
- Replay deadline sweep: `REPLAY_DEADLINE` 10 s / `REPLAY_DEADLINE_TICK` 1 s (`connection.rs:58`, `:61`), sweep arm
  `:1283-1301` — use the injected clock, not `tokio::time::interval`, so tests run on `ManualClock`.
- Per-connection state: `subs: HashMap<AgentId, SubState>` + `ReplayFlightSet` (`connection.rs:651-654`); cleared on
  link loss (`flight.on_disconnect()` `:692`) → in V2 on `Routed::LinkDown` (route already emits exactly one `LinkDown`
  per `LinkUp`, including on re-add via `adopt` and on abnormal inbox end).
- "Drop the connection" in V2 = `peer.reconnect_now()` (or close/re-add — decide; `reconnect_now` keeps the handle and
  shows `Reconnecting` via the `Disconnected{Explicit}` it produces).
- **A1 invariant (TRD U3 row, master re-check A1):** Subscribe · Unsubscribe · RequestReplay must be received by ONE
  actor in order and written to ONE Peer queue — so put them on the V2 actor as new `DriverCmd`s and write with
  `notify` from the actor (non-blocking; never `request`). The router check must happen when the actor takes the
  command (same reason as `connection.rs:1207-1213`).
- Same-key re-add path (`connect`/`ensure` re-entry while a handle exists and is not live) — code exists (§1); add the
  driver-level re-add test (old supervisor tail overlapping: its lifecycle events and a stale `AgentListUpdated` after
  the new snapshot leave the new flight, frontend list and name book intact).
- Move tests per the TRD fate table: A1 `a_replay_request_after_the_close_request_sends_no_subscribe`
  (`tests.rs:6069`), A2 `agent_names_sink_receives_each_daemon_list_after_the_frontend_event` (`tests.rs:3954`) — onto
  the new driver; the old suite stays on the old driver (do not parametrize).

### U3c — inbound daemon commands + command registration + usage interest
- `CommandRequest` (`ToDriver`) → `envelope_from_wire` + `accept_inbound` (`connection.rs:1901`, call site
  `:1063-1076`). The outcome goes back with `link::route::send_command_outcome(peer, arrival_generation, reply)` (U2,
  already exists — never `notify()`); the old path is `outcome_sink` (`:1871`) + arm `:1175-1198`. Note: the V2
  `ToDriver` item carries `generation` — keep it with the request for the outcome.
- Broken `CommandReply` → log only (`:1083-1090`).
- `install_command_table` `mod.rs:511` (uses `self.inbound` + `self.rt` — no driver change needed beyond reading the
  slot).
- Command registration on every connect: `register_own_commands` (`connection.rs:1963`, called at `:969`),
  `registration_command` (`:1934`), refusal box `RefusalAlerts` (field `refusals`). In V2 trigger on `Routed::LinkUp`;
  its reply must not be awaited on the actor (spawn a task: `peer.request_in(generation, ..)`).
- Usage interest: open/lost per socket (`usage.lock().on_socket_open` `:309`/`:888` + `write_usage_on_open` `:1460`;
  `on_socket_lost` `:689`), nudge (`DaemonClient::nudge_usage_interest` `mod.rs:1078`, the arm `connection.rs:1200-1205`
  + `send_usage_subscribe` `:1429`), snapshot relay (`UsageLimitsUpdated` arm `:1093-1107` → `relay_usage_snapshot`
  `:1477`), the post-connect nudge in `start_connection` (`mod.rs:751`), the ⟳ pre-subscribe for
  `RefreshUsageLimits` inside `send_request` (`connection.rs:1354`, `:1374`) — in V2 this means `send_command` must write
  the usage subscribe before the request on the same queue. The usage "socket epoch" maps to the routed connection
  generation (`LinkUp{generation}`).

## 3. Proposed seam U3b | U3c
- U3b: everything that shares `subs` + `ReplayFlightSet` (Subscribe/Unsubscribe/RequestReplay commands, replay events,
  output relay, deadline sweep, `LinkDown` clearing) + re-add tests + A1/A2 move. One actor state, one test harness.
- U3c: inbound commands/outcomes, registration on `LinkUp`, usage interest. Independent state; needs only `LinkUp`
  generation + the `ToDriver` arms U3b leaves alone.
- Shared file: `connection_v2.rs` `apply()` arms and `DriverCmd` — U3c rebases on U3b.

## 4. Contracts and traps learned
- **Actor never awaits a request / close_after_flush / close_all** (transport invariant 1 — `link/pump.rs` doc). A
  reply the actor needs (registration) goes on a spawned task.
- **Every add is one actor step; every `Ended` unregisters** (U2 addendum). Do not reuse an `(Address, Handshake)` pair.
- **Routing is the only state emitter.** Never emit `Down`/`Reconnecting` from the driver.
- **Order relative to close** goes through the actor channel + close epoch; never decide on `state_rx` in the caller
  (review F1/F2 — `close(); connect()` used to return `Ok` with the link down).
- **Tests — hold the daemon-side endpoint.** Dropping the `MemoryEndpoint` returned by `connected()`/`next_link()` is a
  failed link (`"peer endpoint gone"`), not a close — bind it (`let _endpoint = …`). This produced a wrong result once.
- **Tests — `advance_until` advances only while the peer is in `Backoff`**; advancing while dialing/handshaking fires
  the connect deadline. Never bare-`await` a task that may wait on the manual clock — `joined(..)` waits for
  `is_finished()` first.
- **Tests — waits use real-sleep polling (`wait_for`)**: the dialer reads the endpoint record on the blocking pool, which
  transport `settle` does not wait for. Pure `Registry`+`MemoryNetwork` tests without the dialer can use `settle_until`.
- **Bench (`Bench` in `connection_v2/tests.rs:582`)** drives an unspawned actor: `Bench::new()` (via `build()`),
  `add()` returns the token, then call `driver.route_item(..)` / `driver.close()` / `driver.is_live()` directly with
  hand-made `Incoming` items (`retryable_refusal(&key)` builder at `:623`). Use it for ordering/state tests that the
  real network cannot make deterministic. Its pumps' real items pile up unread in `_routable_rx` — ignore them.
- **Harness (`:129`)**: `Harness::new(Current)`, `connect()`/`ensure()` spawn the call, `next_link()` (endpoint + Auth
  text), `connected()`, `advance_until()`, `written()` (next text frame the shell wrote), `states()` (frontend state
  lines from `RecordingEvents`), `peer_state()`, `link()` (the `LinkClient`). `Record` (`:26`) = scripted endpoint
  source: `set(Current)`, `fail_search`, `hold_search()` (gate the `ensure` search), `reads()`, `searches()`.
- **Log assertions**: `link::test_log::capture(lines, || …)` — lines start with `level=<LEVEL> ` then `name=value `
  fields. Filter by level + a structured field, not by message text.
- **`lib_unit` rewrites ~24 `src-tauri/bindings/*.ts`** with identical bytes (autocrlf). Restore with
  `for f in $(git status --porcelain -- src-tauri/bindings | awk '{print $2}'); do [ -z "$(git diff --numstat -- "$f")" ] && git checkout -- "$f"; done`
  (note: `git diff --name-only` lists nothing for them — use `git status`).
- **Name diff**: base `--list` (1411 at `c4bb4954`) includes the 3 ignored tests; compare listed vs (passed+ignored).
  The `layout::dock … - should panic` line differs between `--list` and run output — normalize the suffix.
- Run shell tests only as `cargo test -p engram-dashboard --test lib_unit …` through `scripts/run-detached.ps1`
  (never `--lib`/`--all-targets`). `rustfmt --edition 2021` your files; `cargo fmt -p engram-dashboard --check` must be
  0 diffs. Heredocs fold backslashes — write edit scripts with the Write tool.
- `link/` stays free of `tauri`/agent paths (gate 4); `test_log.rs` uses only `tracing`.

## 5. Review findings deferred to U3b (decided by main)
- **Replaced waiters get `TaskGone`** (lead R3): a second `connect()` while the first waits makes the first return
  `TaskGone` (`add`, `first.replace`). Decide whether it should share the new handle's answer instead.
- **Re-add of a once-connected key shows its first failure as `Down`** (lead H4): `route.rs` cold-start rule keys on
  the handle's `seen.is_none()`; after `adopt` from a shown `Connected` (→ `Reconnecting`), the new handle's first
  `ConnectFailed` shows `Down`. Decide with the re-add tests.
- **`retryable_refusal` current-token guard is untested** (risk-axis): a stale token's retryable `ConnectFailed` must not
  stop the current peer. Add a bench test: add A, add B (re-add), feed A's retryable refusal → no notice, peer B kept.
- **B-e triple token filter in `route.rs`**: "is this the current token" is written three times — `route()`
  (`keys.get_mut(key).filter(|r| r.token == token)` `:326`), `RouteState::current` (`:372`, used by `route_end`), and
  `is_open` (`:366`). Fold into one helper when you touch it.
- Not for U3b (pending user decisions — do not change): flap retry at a fixed ~0.5 s; "handshake `Error` frame =
  terminal" also for a transient daemon `auth timeout`; the 4001 UI text (`refusal_message` — today the version text for
  every handshake refusal).
