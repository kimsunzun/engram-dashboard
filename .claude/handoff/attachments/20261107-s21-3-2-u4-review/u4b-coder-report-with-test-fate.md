# 3-2 U4b coder report — round 1 fixes on top of round 0

## Round 1 (fix-r1.md) — item by item
- **R1 done.** Added `connection::tests::a_command_during_the_reconnect_backoff_fails_at_once_and_never_reaches_the_next_link`: after a loss, wait for Backoff; `send_command` returns `NOT_CONNECTED` within 10 s; after redial + Hello, the first frame on the new endpoint is the order marker (the failed command never went out). Mutation: deleting `require_live(peer.peer_state()).map_err(Local)?;` (`connection.rs`) makes it fail (`left: UNSENT_ON_DISCONNECT, right: NOT_CONNECTED`); file restored (`cmp` identical).
- **R2 done** — no existing supervisor-level test found (only `machine.rs` `the_handshake_inherits_the_deadline_the_dial_was_given` / `a_dial_that_never_lands_never_reaches_the_handshake`). Added transport `peer::tests::a_handshake_that_never_answers_fails_at_the_connect_deadline_and_backs_off` (memory network + manual clock + `HelloHandshake`, endpoint held silent): still `Handshaking` at `connect_timeout - 1 ms`; at the deadline → `Backoff` with `ConnectFailed { cause: Unreachable("handshake timed out") }`. Fate list updated (below).
- **R3 done.** Nothing in `src-tauri` uses the daemon's `test-support` items (`start_test_server*` · `TestServerHandle` · `KeepaliveConfig` re-export — rg 0 hits), so the shell's daemon dev-dependency no longer turns it on; comment reworded. The daemon `Cargo.toml` feature comment named the shell as a place that turns it on — that path reference removed. Shell gates 1–5 still PASS; `Cargo.lock` unchanged.
- **R4 done.** Deleted `protocol_state::{PendingMap, take_pending, drain_pending}`, the now-unused `rid` helper, the `HashMap` import and their 6 tests (`pending_resolve_on_matching_reply`, `pending_resolve_void_ack`, `pending_take_for_error`, `pending_unknown_request_id_ignored`, `drain_pending_on_disconnect`, `close_drains_all_pending`); rewrote the module header (title + purity paragraph), the tests-module scope line and the event-routing note (no `take_pending` reference) in English; `mod.rs` header line fixed; transport `pending.rs` header no longer cites the shell's `PendingMap`. **`ConnectionState::Connecting` removed too**: never constructed, the IPC/frontend vocabulary has no such word (`daemon_connection_state` folded it into "down"); `frontend_state` now returns `ConnectionStateEvent` directly (the dead `None` arm and its debug log went), the "Never `Connecting`" doc lines in `connection.rs` / `link/route.rs` and the `commands/discovery.rs` fold + comment updated.
- **R5 done.** `a_conflict_refusal_shows_one_box_and_the_same_refusal_after_a_reconnect_shows_none` now waits, after the repeated refusal, until `Arc::strong_count(&client.refusals)` is back to its at-rest count (the registration reply task holds one clone until `report_registration` returns), then asserts only the first box is shown, before reconnecting. Mutation: making `RefusalAlerts::surface` always show fails the test at that new assertion; file restored (`cmp` identical).
- **R6 done.** `UNSENT_ON_DISCONNECT` · `SENT_OUTCOME_UNKNOWN` → `pub(crate)` (no integration test uses them); `SEND_FAILED_PREFIX` · `PENDING_SUPERSEDED` stay `pub` (`tests/daemon_client_pending.rs`).
- **R7 done.** Residue comment on `write_refresh_subscribe` (`connection.rs`): the `Err` branch (subscribe not queued → ⟳ not sent, send text) has no test.

### Round 1 files
`src-tauri/src/daemon_client/{connection.rs, connection/tests.rs, protocol_state.rs, mod.rs, link/errors.rs, link/route.rs}` · `src-tauri/src/commands/discovery.rs` · `src-tauri/Cargo.toml` · `crates/engram-dashboard-daemon/Cargo.toml` (comment) · `crates/engram-dashboard-transport/src/{peer.rs, pending.rs}`.

### Round 1 tests
- `cargo test -p engram-dashboard --test lib_unit --test layout_commands --test daemon_client_pending --test daemon_client_replay --test layout_apply` (run-detached): lib_unit **1404 pass + 3 ignored = 1407**, layout_commands 149, layout_apply 151, daemon_client_pending 3, daemon_client_replay 11 — all green, no new warnings. Bindings restored byte-identical.
- `cargo test -p engram-dashboard-transport`: unittests **385** (base 384 + 1); `--all-features`: unittests 410 + socket suites 12 · 2 · 5 · 4 — all green.
- `cargo check -p engram-dashboard` 0 warnings · `cargo fmt --all --check` 0 diffs.
- Gates (extracted from `ci.yml`): shell gate 1–5 · replay_flight purity PASS.
- Name diff vs round 0: +1 `connection::tests::a_command_during_the_reconnect_backoff_fails_at_once_and_never_reaches_the_next_link`; −6 `protocol_state::tests` pending tests (R4); +1 transport `peer::tests::a_handshake_that_never_answers_fails_at_the_connect_deadline_and_backs_off`. (Round 0's `encode_is_the_same_json_text_the_old_driver_writes` → `encode_is_one_text_frame_of_the_serde_json_text` rename was already in round 0.)
- **Name diff vs base `31919dcb` (cumulative):** lib_unit 1488 → 1407 (−97 old suite, −6 R4 pending tests, +22 driver tests, +1 events test moved; renames: 65 `connection_v2::tests::*` → `connection::tests::*`, 4 old_driver-named tests); layout_commands 152 → 149; daemon_client_pending 4 → 3; transport unittests 384 → 385.

### Round 1 self-check
Production behaviour unchanged (the `Connecting` arm was unreachable; `frontend_state` maps the three real states as before). New tests fail without the property they name (R1, R5 mutation-checked; R2 asserts state before and at the deadline).

---

# Round 0 — delete the old shell connection driver

Base: `31919dcb`. Nothing committed. Steps 1–6 are all done.

## 0. Where the memo or brief differed from the code (read first)
- The memo is out of date because U4a landed after it: `connection_v2` was already compiled in every build (it was not `#[cfg(test)]`), and the link suite had **65** tests, not 58. Nothing depended on those numbers.
- The importer list matched, with one addition: `tests/daemon_client_replay.rs` also imports `RefusalPlan` and `plan_subscribe_refusal`, so I moved those too. `shell_owner_advert` was used only by `view_bus.rs` and `registration_command` (the link driver never imported it directly).
- **I did not edit CLAUDE.md.** My rules say an agent message cannot authorize a change to CLAUDE.md. Main should apply this line in 「모듈」:
  `- **transport** — 재사용 전송 lib. 셸의 데몬 연결이 그 위에서 돈다(`daemon_client/link/` · 구동 = `daemon_client/connection.rs`). 데몬은 3-3 에서 붙는다.`
- Steps 1 and 2 had to share one build point. While `connection.rs` still existed it kept its own `HandshakeError` and other copies, so the moved items could not compile next to it. The build and tests were green after move+delete, after the test fate, and again after the rename and docs.

## 1. Change summary and files
**New files (untracked — `git add` them):**
- `src-tauri/src/daemon_client/replay_events.rs` — moved here: `apply_replay_event`, `known_epoch`, `ReplayFollowUp`, `RefusalPlan`, `plan_subscribe_refusal`, `REPLAY_DEADLINE`, `REPLAY_DEADLINE_TICK`. Code is byte-identical; only the doc comments were reworded.
- `src-tauri/src/daemon_client/test_support.rs` (`#[cfg(test)]`) — the shared test helpers taken from the old suite (`RecordingEvents`, `wire_agent`, `relayout`, `usage_update`, `bus_table`, `bus_request`, `active_slot`, `POPUP`, `NoLayoutEvents`, `OpenWindows`, and the `No*` ports).
- `src-tauri/src/daemon_client/connection.rs` and `connection/tests.rs` — the renamed `connection_v2.rs` and its suite.

**Deleted:** `daemon_client/connection.rs` (old), `lifecycle.rs`, `tests.rs` (97 tests). All went through `recycle.sh`.

**Moved items:**
- `UNSENT_ON_DISCONNECT`, `SENT_OUTCOME_UNKNOWN`, `SEND_FAILED_PREFIX`, `PENDING_SUPERSEDED` → `link/errors.rs` (now `pub`).
- `registration_command` and `shell_owner_advert` → `inbound.rs`.
- `HandshakeError` → the driver module, re-exported from `mod.rs` as `pub use connection::HandshakeError`. Its public path is now `daemon_client::HandshakeError`; nothing outside the module used the old path.

**`mod.rs`:**
- Removed: `DaemonDiscovery`, `RealDiscovery`, `OnTheLink`, `REPLY_LOST`, the fields `discovery`, `events`, `names`, `lifecycle`, `handshake_timeout`, every old constructor, every `*_on_old_driver` path, `try_enqueue_on`, `nudge_usage_on`, `close_on_drop`, and the `Option` around `link`. `link` is now a plain `LinkClient` in every build.
- Added: `around_link` (a private split of `on_link`, so a test can build a driver it never starts) and `#[cfg(test)] unconnected()` (used by the popout harness, with the `NoDaemonAnywhere` source). I also stopped cloning `events` and `names` into `Ports` (they move now). Production behaviour is unchanged.

**`HandshakeError`:** I removed `AuthSend` and `Timeout`. Only the old driver produced them.

**`MAX_RECONNECT_ATTEMPTS`:** removed together with the old file. The "5회" prose in `src/api/agentClient.ts` and `src/api/daemonControl.ts` is rewritten (comments only, in English). `WsTransport.MAX_RECONNECT_ATTEMPTS`, the frontend's direct-connect behaviour, is left as it was.

**`Cargo.toml`:** `tokio-tungstenite` is gone from `[dependencies]`; no shell code outside tests uses it, and the dev declaration stays. I refreshed the comments on futures-util, the daemon dev dependency, tokio `test-util` and doctest. `Cargo.lock` did not change, because tokio-tungstenite is still in the graph through transport's `ws` feature.

**Rename `connection_v2` → `connection`:** updated in `mod.rs`, `commands/agent.rs`, `protocol_state.rs`, `replay_flight.rs` and `Cargo.toml`.

**Docs and comments:** stale old-driver, U4b, lifecycle and `tests.rs` mentions are removed or rewritten (in English where the paragraph's content changed):
- the driver module doc and the `link/mod.rs` header;
- `link/errors.rs`, `handshake.rs`, `route.rs` and `wire.rs` (including the `SERIALIZE_FAILED_PREFIX` reference);
- `events.rs`, `inbound.rs`, `usage_interest.rs` (socket-epoch guard and lock order), `protocol_state.rs` and `output_router.rs`;
- `commands/{layout,popout,view_bus,usage,agent}.rs`;
- the headers of `tests/daemon_client_replay.rs` and `tests/layout_commands.rs`;
- `docs/reference/architecture.md` (the transport entry).

**Popout harness:** `commands/popout.rs` now uses `DaemonClient::unconnected`.

**`tests/layout_commands.rs`:** the old (B) harness drove the old driver's command-channel plumbing (the `accept_inbound`/`outcome_sink`/`ConnectionCommand` functions). The rewrite builds a real `Registry<ShellWire>` on a memory link, calls `receiver.accept(envelope, |r| send_command_outcome(&peer, generation, &r))`, and reads the outcome frame the shell actually writes.

**`tests/daemon_client_pending.rs`:** rewritten to the M1 rule over `Registry<ShellWire>` and `link::errors::request_error_text`.

## 2. Tests
Commands I ran:
- `cargo test -p engram-dashboard --test lib_unit --test layout_commands --test daemon_client_pending --test daemon_client_replay --test layout_apply`, through run-detached.
- A filtered re-run, `cargo test -p engram-dashboard --test lib_unit daemon_client`, after the final rename: 274 pass.
- `cargo check -p engram-dashboard` (production build): 0 warnings.
- `cargo fmt --check`: 0 diffs.
- `npx tsc --noEmit`: 0 errors. `npm test`: 2802 pass in 124 files.
- The `bindings/*.ts` files rewritten by the test run were restored, so they are byte-identical.

Counts per target:
| Target | Now | Base |
|---|---|---|
| lib_unit | 1409 pass + 3 ignored = 1412 | 1488 |
| layout_commands | 149 | 152 |
| layout_apply | 151 | 151 |
| daemon_client_pending | 3 | 4 |
| daemon_client_replay | 11 | 11 |

The base lib_unit figure is derived, not run: 1412 − 21 added + 97 removed.

**CI gates:** I extracted the `ci.yml` steps and ran them. Shell gates 1, 2, 3, 4 and 5 and the replay_flight purity gate all PASS.

**Renamed tests (no behaviour change):**
- `connection_v2::tests::*` → `connection::tests::*` (65).
- `command_round_trips_and_their_failures_read_as_the_old_driver_texts` → `…_read_as_the_shell_texts`.
- `link::errors::{request,send}_failures_read_as_the_old_driver_texts_byte_for_byte` → `…_read_as_the_shell_texts_byte_for_byte`.
- `link::wire::encode_is_the_same_json_text_the_old_driver_writes` → `encode_is_one_text_frame_of_the_serde_json_text`.

**Added (24 tests):**
- In `connection::tests` (20):
  - Copied from the old suite onto the link driver: `connect_idempotent_when_already_connected`, `connect_discovery_join_failure_tells_every_window_down`, `forward_drops_a_webview_usage_subscribe`, `forward_rejects_a_webview_command_with_a_broken_envelope`, `forward_rejects_a_webview_outcome_with_a_broken_reply`, `get_usage_snapshot_hands_off_the_cache_with_the_socket_epoch`, `an_envelope_arriving_before_the_table_still_gets_an_answer`, `a_conflict_refusal_shows_one_box_and_the_same_refusal_after_a_reconnect_shows_none`, `an_unknown_code_refusal_shows_one_box`.
  - Copied with an unstarted actor (nudges are counted as `DriverCmd::UsageNudge`, unsubscribes as `DriverCmd::Unsubscribe`): `one_layout_write_recomputes_the_usage_interest_once`, `tray_hide_shrinks_after_the_delay_and_show_grows_at_once`, `showing_again_within_the_delay_cancels_the_shrink`, `a_destroyed_popup_leaves_the_interest_after_the_delay`, `a_destroyed_popup_forgets_its_hidden_mark_even_when_the_model_never_held_it`, `a_popout_close_request_unsubscribes_once_and_the_later_destroyed_sends_nothing`, `a_destroyed_popout_still_in_the_model_is_cleaned_up_in_one_pass_and_frees_its_number`, `a_failed_tray_hide_keeps_the_window_in_the_interest`, `moving_a_usage_slot_to_a_new_window_sends_nothing`.
  - New 「다시 씀」 tests on the manual clock: `a_close_during_the_reconnect_backoff_shows_down_and_never_dials_again`, `a_connect_during_the_reconnect_backoff_connects_on_one_new_link`.
- `events::tests::connection_state_event_strings_are_the_frontend_contract` (moved there).
- `daemon_client_pending`: `a_duplicate_request_id_fails_both_round_trips_and_the_link_lives`, `a_collided_request_id_stays_retired_and_its_late_reply_wakes_no_one`. `distinct_request_ids_coexist` is rewritten under the same name.

**Removed `daemon_client::tests` (97), with fate row and covering test:**
- **Handshake bytes (옮김):**
  - `connect_sends_auth_first_frame` and `auth_sends_compiled_protocol_version_not_echo` → `link::handshake::start_sends_the_auth_frame_with_the_slot_token_and_the_compiled_protocol_version`, plus `connection::tests::a_first_connect_resolves_on_hello…` (Auth carries the record's token).
  - `hello_consumed_internally_reaches_connected` → `link::dialer::auth_then_hello_brings_the_daemon_live_and_other_frames_before_hello_are_dropped` and `link::handshake::hello_ends_the_handshake`.
- **Discovery / spawn (옮김):**
  - `ensure_no_spawn_when_no_daemon` → `ensure_never_searches_and_a_missing_daemon_shows_down`.
  - `connect_may_spawn` → `a_first_connect_resolves_on_hello…` (searches == 1).
  - `connect_propagates_spawn_failure` and `connect_discovery_failure_tells_every_window_down` → `a_cold_start_failure_shows_down_at_once_and_retries_without_limit…`.
  - `connect_idempotent_when_already_connected` → copied.
- **퇴역 — hand-written lifecycle, generation guard, cancel races:**
  - `concurrent_connect_settles_connected_no_flap`, `connected_then_close_reconnect_no_down_clobber`, `lifecycle_guard_blocks_stale_publish`, `guard_stale_down_cannot_clobber_current_connected`, `guard_concurrent_connect_settles_to_newest_generation`, `guard_store_cmd_rejects_stale_generation`, `guard_is_current_reports_without_republishing`, `stale_task_gone_emits_nothing_newer_generation_owns_the_screen` → `link::route` `rule_1_drops_every_item_of_a_handle_that_is_not_the_keys_current_one`, `rule_2_…`, `rule_3_…`, `states_are_emitted_on_edges_only`; transport `two_supervisors_sharing_a_counter_never_mint_the_same_generation`; `connection::tests::a_connect_overtaken_by_a_re_add_gets_the_new_handles_answer`.
  - `close_in_flight_stays_down_no_revival`, `lifecycle_close_clears_cmd_and_blocks_stale_revival`, `guard_close_blocks_stale_revival`, `cancel_signal_fires_on_bump_and_close`, `bump_clears_stale_cmd_tx`, `reconnect_guard_proceeds_only_when_current_and_not_closed`, `reconnect_guard_stops_after_close_closed_by_user` → transport `close_is_final_and_swallows_everything_after_it`, `closing_before_ever_being_live_emits_no_disconnect`, `close_is_heard_while_a_write_is_parked`; `a_close_before_hello_answers_the_waiting_ensure`; `a_close_answers_the_waiting_connect_at_once…`.
  - `reconnect_close_during_read_live_no_socket_open`, `reconnect_close_during_handshake_self_closes_socket`, `reconnect_close_during_backoff_no_daemon_contact`, `reconnect_connect_during_backoff_no_extra_socket`, `reconnect_close_during_connect_async_no_stale_auth`, `reconnect_close_after_auth_send_self_closes_socket`, `supersede_connect_cancels_reconnect_before_discovery` → `a_close_during_the_daemon_search_wins_over_the_connect`; the two new backoff tests; `link::dialer::an_old_registrations_late_dial_cannot_change_what_the_new_handshake_sends`; transport `close_is_final…`.
  - `handshake_times_out_when_server_silent` → transport `peer::tests::a_handshake_that_never_answers_fails_at_the_connect_deadline_and_backs_off` (round 1, supervisor level) and `machine.rs` `the_handshake_inherits_the_deadline_the_dial_was_given`.
  - `caller_gone_after_hello_lands_on_down_not_connected` → this property does not exist on the link driver: the connection belongs to the actor and routing is the only source of state (`states_are_emitted_on_edges_only`). A dropped caller changes nothing.
- **다시 씀 — backoff, reconnect, exhaustion, port move:**
  - `backoff_delay_is_exponential_capped` → transport `backoff_is_capped_and_jitter_stays_inside_its_band`, `unreachable_burns_budget_and_the_schedule_is_500_1000_2000`; `connection::tests::the_link_policy_retries…`.
  - `reconnect_disconnect_recovers_to_connected` → `connected_is_announced_after_the_usage_state_sees_the_new_socket` and `every_connect_registers_once…` (both use `reconnected`), plus transport `a_lost_link_comes_back_on_the_first_backoff_step`.
  - `reconnect_exhausts_to_down` → inverted by ADR-0346: `a_cold_start_failure_shows_down_at_once_and_retries_without_limit…` and `the_link_policy_retries_network_failures_without_limit…`.
  - `reconnect_close_blocks_closed_by_user` → `close_shows_down_only_through_routing…` (no redial) and the new backoff close test.
  - `reconnect_close_during_backoff_no_zombie` → new `a_close_during_the_reconnect_backoff_shows_down_and_never_dials_again`.
  - `reconnect_connect_during_backoff_no_hijack` → new `a_connect_during_the_reconnect_backoff_connects_on_one_new_link`.
  - `reconnect_follows_hot_swapped_daemon` → `the_production_transport_attaches_answers_and_follows_the_daemon_to_a_new_port`, `link::dialer::a_reconnect_follows_the_port_and_credential_the_daemon_moved_to`.
  - `reconnect_down_then_connect_revives` → `a_connect_right_after_a_close_connects_again` and the cold-start test.
- **ADR-0195 row:**
  - `buffered_command_on_disconnect_is_drained_not_executed_post_reconnect`, `held_clone_enqueued_after_disconnect_is_rejected_and_never_runs`, `foreign_socket_command_is_dropped_and_its_waiter_is_woken`, `command_during_reconnect_fails_fast_and_never_reaches_next_socket` → transport `a_command_stamped_before_a_reconnect_is_not_written_on_the_next_link`, `a_request_future_made_before_a_reconnect_stays_bound_to_the_old_generation`, `requests_made_while_down_fail_without_waiting`.
  - Shell adapter test: `a_command_during_the_reconnect_backoff_fails_at_once_and_never_reaches_the_next_link` (round 1), plus `command_round_trips_and_their_failures_read_as_the_shell_texts` (never connected) and `a_command_right_after_a_close_fails_at_once_and_nothing_goes_out`.
- **Pending, commands, events (옮김):**
  - `send_command_resolves_ok_on_matching_ack`, `send_command_resolves_err_on_matching_error`, `disconnect_mid_flight_drains_pending_to_err`, `send_command_errs_when_not_connected`, `send_command_rejects_command_without_request_id` → `command_round_trips_and_their_failures_read_as_the_shell_texts`.
  - `unknown_request_id_event_is_ignored` → transport `a_reply_nobody_asked_for_is_reported`.
  - `reconnect_with_in_flight_command_drains_old_then_new_socket_usable` and `buffered_send_command_drain_yields_unsent_message` → transport `a_waiting_request_ends_as_sent_a_queued_one_as_not_sent_and_neither_runs_on_the_next_link`, plus the `link::errors` text tests.
  - `duplicate_request_id_insert_errs_prev_slot` → M1 in the rewritten `daemon_client_pending`, plus transport `a_colliding_tag_wakes_the_older_waiter_and_refuses_the_newer_one`.
  - `unsubscribe_sends_unsubscribe_wire`, `fire_resize_sends_resize_wire`, `fire_and_forget_when_not_connected_is_noop` → `a_fire_and_forget_goes_out_only_on_a_live_link`, plus the unsubscribe-marker tests (`nudges_carry_no_set…` and others).
  - `recording_events_capture_connected_then_broadcasts` → `a_first_connect_resolves_on_hello_and_broadcasts_reach_the_frontend_before_the_names`.
  - `agent_names_sink_receives_each_daemon_list_after_the_frontend_event` → same name in `connection::tests`.
  - `failed_handshake_tells_every_window_down` → `a_daemon_error_in_the_handshake_stops_for_good_without_a_second_dial` (states == [down]).
  - `connection_state_event_strings_are_the_frontend_contract` → moved to `events::tests`.
  - `dropping_client_releases_the_connection_task_channel` → `dropping_the_client_closes_the_link_and_gives_its_one_registration_back`.
  - `ensure_without_live_daemon_tells_every_window_down` → `ensure_never_searches_and_a_missing_daemon_shows_down`.
  - `connect_discovery_join_failure_tells_every_window_down` → copied.
  - `explicit_close_tells_every_window_down` → `close_shows_down_only_through_routing…`.
- **Usage (옮김):**
  - Same name in `connection::tests`: `a_usage_broadcast_notifies_once_with_labels_and_socket_epoch`, `a_mismatched_broadcast_writes_the_interest_back_on_its_socket`, `nudges_carry_no_set…`, `a_refresh_writes_the_interest_before_the_refresh_frame`, `a_refresh_with_no_interest_writes_an_empty_subscribe_first`, `connected_is_announced_after_the_usage_state_sees_the_new_socket`, `a_usage_slot_placed_over_the_bus_subscribes_and_a_bus_toggle_off_shrinks_later`, and `a_replay_request_after_the_close_request_sends_no_subscribe` (A1).
  - `a_new_socket_sends_the_interest_before_queued_commands` → `a_lost_connection_clears_the_usage_cache_and_the_next_one_gets_the_current_interest_first` and `a_fire_and_forget_then_a_round_trip_go_out_in_that_order_on_an_opened_connection`.
  - `a_snapshot_on_a_superseded_socket_is_ignored` and `a_refresh_on_a_superseded_socket_writes_no_subscribe_before_it` → `a_daemon_command_request_or_usage_snapshot_on_a_connection_being_dropped_is_not_handled`, route `rule_2…`, transport `notify_in_and_request_in_go_out_only_on_the_generation_they_name`.
  - `forward_*` (3), `get_usage_snapshot…`, the 10 idle-client usage/tray/popout tests, and `a_conflict…` / `an_unknown_code…` refusals → copied under the same names.
  - `an_invalid_argument_refusal_shows_one_box` → `every_connect_registers_once_and_a_refusal_shows_its_box…` (same INVALID_ARGUMENT text).
  - `a_drop_before_the_reply_shows_no_box` → `a_drop_before_the_registration_reply_shows_no_box`.
- **퇴역 — old-channel mechanics with no counterpart:**
  - `a_change_in_the_gap_before_the_channel_is_stored_goes_out_after_it` and `the_nudge_after_the_channel_is_stored_sends_nothing_without_a_change` → the link opens in one actor step; covered by `a_lost_connection_clears_the_usage_cache…` and `connected_is_announced…`.
  - `a_refresh_on_a_full_command_channel_still_writes_the_subscribe_first` → there is no command channel for round trips any more.
  - `a_failed_subscribe_write_fails_the_refresh_as_send_failed` and `a_failed_refresh_write_after_the_subscribe_fails_the_refresh` → the subscribe is queued, not written, by the caller; covered by transport `a_request_cut_mid_write_is_woken_not_counted` and the `link::errors` send texts. **Gap:** the driver branch "the usage subscribe could not be queued → the refresh is not sent" (`write_refresh_subscribe` `Err`) has no test.

**Removed from `layout_commands` (3):**
- `the_outcome_sink_does_not_keep_the_command_channel_alive` and `the_outcome_sink_stamps_the_socket_it_belongs_to` (퇴역; the type-check `const _` went with them) → route `a_command_outcome_goes_only_to_the_connection_its_request_came_on`, `a_daemon_command_is_answered_on_the_connection_it_came_on_and_not_on_a_later_one`.
- `an_envelope_arriving_before_the_table_still_gets_an_answer` → moved to `connection::tests`; the no-table branch is private to the driver.

**Removed from `daemon_client_pending` (2):**
- `duplicate_request_id_supersedes_instead_of_panicking` and `second_reply_for_same_id_finds_no_slot` → replaced by the two M1 tests.
- `supersede_survives_dropped_waiter` → retired: the old pending map is gone, and transport collision handling is pinned by `a_colliding_tag…`.

## 3. Self-check
- Production behaviour: no production code path changed. The only production-visible edits are the `HandshakeError` variants (never constructed) and moving `events`/`names` instead of cloning them.
- Gate 4 (no tauri or agent paths in `link/`) passes, and so do gates 1, 2, 3, 5 and replay_flight purity.
- No `connection_v2`, `U4b` or old-driver references are left in `src-tauri` code. The one exception is the provenance line "Ported from the old driver's suite" in `connection/tests.rs`.

## 4. Other differences and items for main
- **Test renames:** the four old_driver-named tests listed in §2 were renamed.
- `protocol_state::{PendingMap, take_pending, drain_pending}` and `ConnectionState::Connecting`: removed in round 1 (R4).
- **Stale references I did not edit** (reference docs written in Korean or record-like):
  - `docs/reference/logging-conventions.md:60` (mentions the generation guard).
  - `docs/reference/structure/session-path-ownership.md:889` (mentions `daemon_client/lifecycle.rs`).
- **A weakness in the conflict-refusal test:** it cannot tell whether the second (suppressed) refusal was processed before the third, because the result is the same either way. The old test had the same limit.

## 5. Most expensive step
Classifying the 97 old tests against the fate table and copying the 18 missing 「옮김」 tests (the 10 usage tests needed a driver that is built but never started).
