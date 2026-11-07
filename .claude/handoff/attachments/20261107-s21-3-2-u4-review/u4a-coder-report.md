# 3-2 U4a coder report — after review round 1 (fix-r1)

Base `0d4f79d1`. Nothing committed. Round-0 work (production switch · 30 s policy · `REPLY_TIMEOUT` removal · old driver
gated out of non-test builds) is unchanged except where noted below. Untracked `docs/research/client-without-daemon-message-2026-11-07.md`
is not mine (main's).

## Items

- **H1 — done.**
  - `HandshakeError::NoLiveDaemon` now carries the cause: `NoLiveDaemon(String)`. Display =
    `no live daemon ({cause}) — is the daemon running?`. All `HandshakeError` texts stay in one place (`connection.rs`
    Display).
  - New `link::dialer::no_live_daemon_cause(&str) -> Option<String>`, next to the dialer's own text constants:
    - `NO_RECORD` → `"no endpoint record"`
    - `"{UNUSABLE}: {why}"` → `why` (e.g. `daemon.json names pid N which is not alive`)
    - anything else → `None`
  - `Driver::answer_first` maps an `Unreachable` first failure through it: `NoLiveDaemon(cause)`, otherwise `Connect` as
    before. So `ensure` and `connect` show the new text whenever a handle's first failure is absent or unusable record.
    Version refusal text untouched.
  - Text reading: only this dialer writes those openings. The ws dialer starts with `ws connect`, the transport's texts
    are fixed words, and `mask_reason` keeps the prefix. The cause is the endpoint source's local diagnostic and quotes
    no file content. The dialer const doc now says the first answer reads these two texts.
  - The old driver (test-only) passes a fixed cause; its two old-suite assertions moved from the unit variant to
    `NoLiveDaemon(_)` (same property).
- **R1 — done.** `answer_first` no longer answers `Connected`. `on_routable` answers it with `Ok` after `apply(out)`, i.e.
  after the state watch and the frontend's `connected`. Failures are still answered before routing, which is also the old
  driver's order (its `ready_tx.send(Err)` came before its guarded Down). It keeps a retryable refusal answering
  `AuthRejected`, not `ClosedBeforeHello` from the stop's close. Module doc updated.
- **R2 — done, one deviation.**
  - `connect` passes the endpoint its search found (`DriverCmd::Add { found }`) to
    `LocalDaemonDialer::register(key, source, found)`. That seeds the registration's last-known endpoint, so a record
    held busy right after the search still dials the found daemon with its credential.
  - Deviation: the first dial **still reads `daemon.json` first** rather than skipping the read. This keeps ADR-0346's
    verified premise ("every dial reads `daemon.json` afresh and pairs port and token from one read") and the dialer's
    module contract intact. It also removes the failure the item names (`BUSY_UNKNOWN` → `daemon_connect` fails, `down`
    flashes).
  - No transport change.
- **R3 — done.**
  - The actor's sweep is an `Option<BoxFuture>`. It is armed at the top of each loop turn only while
    `ReplayFlightSet::awaits_deadline()` is true (a new pure method: any in-flight replay not yet failed), dropped when
    none waits, and re-armed after each fire.
  - It still runs on the injected clock; the existing deadline tests pass.
  - Cost: `awaits_deadline` scans the per-agent flight map once per actor turn (small map).
- **R4 — done.**
  - Extracted `DaemonClient::on_local_daemon(rt, owned_rt, data_dir, events, names, refusals, router, registry)`:
    `WsDialer::new(16 MiB)` + `LocalEndpointSource::new(…, data_dir)` + `SystemClock` → `on_link`. The production
    constructor calls it with `default_data_dir()` and Tauri ports, so the test drives exactly production's transport
    on a temp folder.
  - New real-socket test `the_production_transport_attaches_answers_and_follows_the_daemon_to_a_new_port`
    (~0.5 s; 15/15 repeat runs green):
    - `daemon.json` in a tempdir (pid = this test process, `start_time` 0 → real liveness accepts it).
    - `ensure` → Auth carries the record's token → one `ListAgents` round trip.
    - Write a new port and token to `daemon.json`, drop the daemon socket → the retry alone dials the new port with the
      new token → wait for `reconnecting` then `connected` → round trip again.
  - **Not a transport `WsListener`:** a consumer cannot get the link out of an `Accepted` (fields `pub(crate)`) except
    through `Registry::attach` with a daemon-side `Handshake`, and that handshake is 3-3 work. The fake daemon is
    `tokio_tungstenite::accept_async` (dev-dependency, the old suite's mock-daemon pattern) — the same wire.

## Files

- `src-tauri/src/daemon_client/connection.rs`: `NoLiveDaemon(String)` + text.
- `src-tauri/src/daemon_client/connection_v2.rs`: H1 mapping, R1, R2 `found`, R3 sweep, module doc.
- `src-tauri/src/daemon_client/link/dialer.rs`: `no_live_daemon_cause`, `register(.., found)`, const doc, unit test.
- `src-tauri/src/daemon_client/replay_flight.rs`: `awaits_deadline`.
- `src-tauri/src/daemon_client/mod.rs`: `on_local_daemon`; the old ensure's `NoLiveDaemon` cause.
- `src-tauri/src/daemon_client/tests.rs` (old suite): 2 pattern updates.
- `src-tauri/src/daemon_client/connection_v2/tests.rs`: test updates and additions (below).
- Round-0 files unchanged otherwise.

## Tests

`cargo test -p engram-dashboard --test layout_apply --test layout_commands --test daemon_client_pending --test daemon_client_replay --test lib_unit -- --test-threads=4`
→ 4 / 11 / 151 / 152 / lib_unit **1485 passed + 3 ignored**, 0 failed.

Name diff vs base `0d4f79d1` (1480 → 1488): lost 0 real tests. The one vanished name is the renamed policy test. Added 9:

- `the_link_policy_…_waits_30_s_for_a_reply_and_keeps_the_other_defaults` (rename)
- `a_frontend_command_whose_reply_never_comes_fails_after_30_s_with_the_reply_timeout_text` (round 0)
- `ensure_on_a_stale_record_names_its_cause` (H1 — exact stale text)
- `ensure_on_a_busy_record_with_no_known_address_is_a_connect_failure` (H1 — other failures keep `Connect`)
- `dialer::tests::only_a_missing_or_unusable_record_reads_as_no_live_daemon` (H1)
- `a_connect_resolves_only_after_its_connected_is_shown` (R1 — Bench; the frontend hook sees the oneshot still empty and
  the state watch `Connected`; the old order fails it deterministically)
- `a_connect_dials_the_daemon_its_search_found_while_the_record_is_busy` (R2 — without the seed, no link opens)
- `an_idle_actor_sets_no_timer` (R3 — `ManualClock::sleepers() == 0`; the old always-on tick makes it 1)
- `the_production_transport_attaches_answers_and_follows_the_daemon_to_a_new_port` (R4)

Changed existing assertions:

- `ensure_never_searches_and_a_missing_daemon_shows_down`: `Connect(_)` → the exact new text (the H1 test for the
  missing record).
- `a_re_add_of_a_once_connected_key_shows_its_first_failure_as_down`: `Connect(_)` → `NoLiveDaemon(_)` (its record is
  absent).

Other checks:

- `cargo build --locked` (workspace): green, no shell warnings. The remaining warnings (ts-rs ×4, Cargo multiple build
  targets) are identical to base.
- `cargo fmt -p engram-dashboard --check`: 0 diffs.
- CI `shell gate 4`: PASS (0 matches). `shell gate 5`: PASS (normal 0 / dev 1). `replay_flight purity` (I touched that
  file): 0 matches.
- Bindings regenerated byte-identical; restored.

## Self-check

- R1: `add` answering `Ok` at once for an already-live peer is unchanged. A `Connected` of a replaced handle is still
  ignored by `answer_first_with`'s newest-handle check.
- H1: the re-added handle of a once-connected key also answers `NoLiveDaemon` when its record is absent. It is a new,
  never-connected handle, matching the item's rule. The frontend matches none of these texts (`rg` over `src/`).
- R3: after a replay fails (failure marker), the sweep stops — not separately tested, because transport timers share the
  manual clock and make sleeper counts unattributable once a link exists.
- Clippy is still not installed. One more `#[allow(clippy::too_many_arguments)]` on `on_local_daemon` (8 parameters),
  the same as `on_link`.

## Most expensive step

The full five-target shell test build and run (plus a filtered `daemon_client::` run and two single-test reruns while
fixing the R4 harness race — the command was sent before the shell had read the new daemon's `Hello`).
