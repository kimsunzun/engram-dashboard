# U3c — review round 1 fixes (main → coder)

Review = deep (lead + concurrency/ordering lens + cross-family). Verdict FIX. Snapshot reviewed = tree `be0622637876ab361d5aa46de14a517c6bd21004` — do not revert unrelated parts. If you think a finding is wrong, do not implement it: reject it with evidence in your report.

## Must
F1. **A waiting `send_command` can cross a connection boundary (ADR-0195).** `opened_generation`/`opened_now` (`connection_v2.rs` ~461-490, ~1349-1358) re-read `peer.generation()` on every wake, so a caller that entered while g was live-but-not-opened can wake to Live(g+1) and write on g+1 (possibly a different daemon — the dialer re-reads `daemon.json`). Also a lost-wakeup window: `peer.state()` is cloned after the state read in `opened_now`. Fix: capture `bound = peer.generation()` once after `require_live`; clone the state receiver before the first check; succeed only when `opened == Some(bound)`; return `UNSENT_ON_DISCONNECT` as soon as the peer is not live or `peer.generation() != bound`. Test: deterministic (Bench or Harness with the usage lock held as in test 4): enter the wait on g, drop g, reconnect g+1 and let it publish → the caller fails `UNSENT_ON_DISCONNECT` and nothing of it is written on g+1.
F2. **Fire-and-forget buffering is unbounded before the cap.** `send_fire_and_forget` (~442) pushes every `Fire` into the unbounded actor channel; `HELD_FIRES_MAX` applies only after dequeue. The old path was a bounded `try_send`. Fix: enforce the old bound before enqueueing (e.g. an atomic in-flight counter decremented when the actor takes the `Fire`, or a separate bounded channel the actor drains in order with its other commands — keep A1 order of Fire vs Unsubscribe/RequestReplay as today, say how). Test saturation at the caller.

## Also fix (권장 chosen by main)
F3. **Fire order vs round trips on an opened connection.** Today `fire(A); send_command(B)` can put B on the wire first (Fire waits in the actor queue, round trips write straight to the peer). Restore FIFO on an opened connection: when the caller sees `opened == Some(g)` with `g` = the live generation at call, write directly with `notify_in(g)` on the caller (as U3b did); only when not yet opened go through the actor (held path). If this conflicts with F2 or A1 in a way you can show, report instead and keep the current shape with a module-doc line stating the non-FIFO window.
F4. **`close(); send_command()` still reaches the wire** (pre-existing since U3a; the old driver failed it at once). Publish the opened value together with the close epoch at publish time; the caller loads `closes` on entry and treats a publish from an older epoch as "not opened" (→ the close path's existing text). Same check for the F3 direct fire path. Test: `close()` then `send_command()` immediately → fails, nothing written.
F5. **A daemon `CommandRequest` from a closed link's tail still runs** (`Driver::close` leaves routing on the old handle until `LinkDown`; `apply` skips only `dropping`). Skip `ToDriver` command requests also while `current.is_none()` (after close), with the same reasoning as the `dropping` skip (comment: side effects whose outcome can never reach the daemon; the daemon ends it at its delivery deadline as TIMEOUT). Test it.
F6. **`start_now` relies on an unstated transport property.** In transport `Peer::request_in` doc (`crates/…transport…/peer.rs` ~404-437) state the contract "the request is queued (or the call fails) before the first await — the first poll queues it", and add a transport test pinning "queued at first poll". Doc + test only, no API change.
F7. **Opened-wait timeout logs nothing.** Add a warn with structured fields (peer key, generation, after_secs) when the wait times out.
F8. **Dead arm in `report_registration`** (~1420-1426): `Err(CommandFailure::Local)` cannot come from `classify_reply(Ok(_))` (`protocol_state.rs` ~293-298). Match directly; remove the dead arm.
F9. **`relay_usage_snapshot` takes `registry.get` (roster lock) inside the usage lock** — move the `get` before the lock, as `on_link_up` already does.

## Tests to add or harden
T1. Waiter endings: timeout text (advance ManualClock past the request timeout while Live and not opened — usage lock held as in test 4) + the F7 warn; loss while waiting → `UNSENT_ON_DISCONNECT`; actor gone → `CHANNEL_GONE` if drivable.
T2. Held-fire rules, deterministic (Bench calling `driver.fire(..)`): `fire(g2)` with `last_up = g1` → held; route Connected(g2) → flushed (written) before the publish; later `fire(g1)` → dropped; the cap; `close` clears held fires; a newer one kept across an older `LinkUp`.
T3. ADR-0281 "no box on a local failure" on V2: copy/adapt `a_drop_before_the_reply_shows_no_box` (close the endpoint before the reply, reconnect, refuse the second registration → only the second box).
T4. Copy `a_refresh_with_no_interest_writes_an_empty_subscribe_first` (old tests.rs ~5300) onto the memory link.
T5. `a_daemon_reply_or_request_too_broken_to_read_is_logged_and_dropped`: use a payload whose serde error would echo a marker value; assert no captured line contains the marker.
T6. `every_connect_registers_once…`: bound the responsiveness probes (timeout → fail, not hang).
T7. The 50 ms real sleep in `a_daemon_command_is_answered_…`: replace with waiting for a deterministic signal (e.g. the `send_command_outcome` log line) if clean.

## Not for you
Wording-only items (doc of `send_command`/`opened_generation` "old driver's text", `SharedUsageInterest` lock doc, `report_registration` "the next one registers") — main fixes them at the end. Do not edit them unless your code change makes the text wrong; then fix that text.

## Report
Overwrite `R\coder-report.md` (R = the scratch `u3c` folder): per item F1–F9 / T1–T7 → done / rejected (evidence); files; tests (commands, counts, full `lib_unit` + transport crate name diff vs base `1d3a8486`: lost 0 · added N); self-check; most expensive step. Same rules as the original brief (scratch, run-detached, fmt, bindings restore, no commit, no subagents).
