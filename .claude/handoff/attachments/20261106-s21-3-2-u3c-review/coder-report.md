# U3c coder report — review round 3

Base `1d3a8486`. Nothing committed; `bindings/*.ts` restored (git status = the 5 files below). Wording items untouched.

## G1 (must) — done
- The gate now carries `handle_closes` — the close epoch the current handle was added under. `Driver::add` sets it under the gate **before** `Registry::add` (so a caller that sees the new peer live never reads the previous handle's epoch). The `Driver::handle_closes` field is gone; `on_link_up` publishes `Opened { closes: gate.handle_closes }` inside the same critical section.
- `send_fire_and_forget`: the first arm under the gate is now `_ if gate.handle_closes < closes => Closed` (`why="closed"`), covering the direct and the pending path alike (it replaces the direct arm's `opened.closes < closes` check, which it subsumes: `opened.closes` is the same handle's epoch).
- Test `a_fire_and_forget_after_a_close_while_the_connection_is_not_opened_yet_is_dropped`: usage lock held → the actor stops inside `on_link_up(g)` before the gate, peer live, not published; `close()` then `send_fire_and_forget` → a caller-side DEBUG line with `why="closed"`, the pending list empty; after release and `Down`, no frame on the endpoint.
- **Mutation checked**: removing the line `_ if gate.handle_closes < closes => FireOutcome::Closed,` locally made the test fail (`dropped at the caller as closed: []`); the file was then restored from a copy (`grep -c` = 1) and the full run is green. Note: the failing assertion is the caller-side one; the wire assertion alone would not catch the mutant reliably — the transport's control channel (`Ctl::Close`) can overtake a queued write, so a fire wrongly drained onto the closing link may or may not reach the endpoint.

## G2 (optional) — not added, why
A mutant that drains the pending list after releasing the gate / after the publish signal differs from the correct code only in the interval between `send_replace(())` and the drain inside one synchronous actor step. A test can catch it only if a round trip is issued and written inside that interval, which needs the actor to pause there; nothing in that interval blocks or yields (no lock other than the gate, no await), so without a new production hook (e.g. a test-only barrier between publish and drain) the interleaving is left to scheduling. Not deterministic → not added; the one-critical-section requirement stays documented in the code (drain and set-opened in one `lock_gate` block in `on_link_up`).

## Files
- `src-tauri/src/daemon_client/connection_v2.rs`, `src-tauri/src/daemon_client/connection_v2/tests.rs` (this round)
- unchanged this round: `crates/engram-dashboard-transport/src/peer.rs`, `src-tauri/src/daemon_client/mod.rs`, `src-tauri/src/daemon_client/tests.rs`

## Tests
- `cargo test -p engram-dashboard --test lib_unit connection_v2` → 58 passed; repeated 3 times → 58/58 each.
- Full `cargo test -p engram-dashboard --test lib_unit` → 1477 passed, 0 failed, 3 ignored (1480). Name diff vs base `1d3a8486` (1455): **lost 0, added 25** (round 2's 24 + `a_fire_and_forget_after_a_close_while_the_connection_is_not_opened_yet_is_dropped`).
- Transport unchanged since round 1 (384 passed; vs base lost 0, added 1).
- `cargo fmt --check` both packages → 0 diffs; `cargo build --workspace` → green. Clippy not run (component not installed for 1.95.0).

## Self-check
- `handle_closes` is written only in `Driver::add` (under the gate, before the peer exists) and read under the gate by callers and by `on_link_up`; `close` does not touch it — the epoch bump on the caller side is what makes callers compare newer.
- A connect after a close: `add` takes the new epoch → callers that read it are not dropped; callers that read the old epoch before the close are, by design, not affected (a write racing a close, as with the old driver).
- The round-trip path (`opened_now`) still checks `Opened.closes`, which now equals the gate's `handle_closes` of the publishing handle — unchanged behaviour.

## Most expensive step
Placing the epoch write before the peer exists (a later write would let a caller on the new peer read the old epoch and drop wrongly) and the local mutation run for the new test.
