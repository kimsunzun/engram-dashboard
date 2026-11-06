# Should the time seam (`Clock`) be gathered in one place — peer survey

- **Status:** medium · design-decision mode · 2026-10-03 · adversarial review BLOCK → conclusion corrected (below) · decision: ADR-0269 decision 3 (main's call 2026-10-03)
- **Method:** collector 1 (peer code · `gh api` raw text + docs) → main grounding (checked against zed · Materialize · quinn raw text) → one cross-family adversarial review.
- **Confidence legend:** certain (independently cross-confirmed) · likely (single-source grounding) · uncertain (unverified).

## Why we investigated

ADR-0269 decision 3 decided to merge the four `Clock`s into base `time`, following 「시간은 공용이면 무조건 합친다」 (user, 2026-09-26). But the four have different shapes (review F2 — checked against code): daemon `command_delivery.rs` = one `now()` · discovery `lib.rs` = `now()` + blocking `sleep` · transport `clock.rs` = `now()` + async `sleep` (tokio) · daemon `usage_service/clock.rs` `UsageClock` = `mono() -> Duration` + `wall() -> i64`. Merging them wholesale makes base pull in tokio.

## Findings

1. **Putting the seam in a shared crate is common — but 「워크스페이스 전체에 Clock trait 하나」 (one Clock trait for the whole workspace) is not.** (likely) Large workspaces put it in an existing util or runtime crate: zed `scheduler` (`crates/scheduler/src/clock.rs` — checked against main's raw text) · Materialize `mz_ore::now` (`src/ore/src/now.rs` — checked against main's raw text) · TiKV `tikv_util::time` · Firecracker `utils::time`. Even so, zed has **two** seams (`SystemClock` in the `clock` crate · `scheduler::Clock`).
2. **Some peers let the executor (runtime) own async sleep.** (likely — we did not count how often) ★「거의 안 둔다」 (they almost never put it there) was an overstatement★ — some traits put sleep next to `now`: `cruxi_clock::Clock` (blocking sleep) · `async_speed_limit::clock::Clock` (async sleep — TiKV family) (counterexamples from the adversarial review). tokio `time::pause`/`start_paused` · zed gpui `advance_clock` (test dispatcher) · the fuchsia-async fake-time executor. The exception = the quinn `Runtime` trait (`new_timer` · `spawn` · `now` as one bundle — checked against main's raw text) — that is not a clock but an 「비동기 런타임 추상」 (async runtime abstraction).
3. **Several peers separate 「지금 읽기」 from 「기다리기」.** (likely — there is no frequency evidence to call it 「보통」) zed `Clock` = only `utc_now` · `now`, timers go through `scheduler.timer()` · Materialize `NowFn` = a single wall-clock millisecond closure · quanta · mock_instant = now only. Partial exception = Firecracker `MockClock` + `MockTimer` (imitates a non-blocking `timerfd` read — not a blocking sleep · adversarial review correction).
4. **Monotonic time and wall clock are often separate.** — zed has both in one trait · Materialize has the wall clock only · TiKV · Firecracker separate them by function · type.
5. **The sans-IO alternative** — quinn-proto takes `now: Instant` as an argument on every call (no trait · search result only — uncertain).
6. **A pitfall of tokio fake time** — `tokio::time::pause` stops only `tokio::time::Instant`; `std::time::Instant` keeps running (tokio docs). Mixing them silently breaks determinism.

## Against our four

| Our seam | Shape | Peer type | Merging |
|---|---|---|---|
| daemon `command_delivery` `Clock` | `now()` | zed `Clock` · quanta | into base `time`'s 「지금 읽기」 |
| discovery `Clock` | `now()` + blocking `sleep` | Firecracker (partial) | only 「지금 읽기」 goes to base · the blocking sleep stays in discovery |
| transport `Clock` | `now()` + async `sleep` (tokio) | quinn `Runtime` (runtime abstraction) · in the other peers the runtime owns it | only 「지금 읽기」 goes to base · the async sleep stays in transport (so base does not pull in tokio) |
| daemon `UsageClock` | `mono()` + `wall()` | zed `Clock` (mono + wall) | expressed as 「지금 읽기」 (monotonic) + wall-clock milliseconds (`now_epoch_ms`) — ★ADR-0269 decision 3 does not merge this now (contract difference — see 「`UsageClock` 은 같은 계약이 아니다」 below)★ |

**Implication (corrected after the adversarial review):** The peer evidence cannot set a universal boundary for 「어디까지 합치나」 (how far to merge) — some peers separate, some bundle. 「지금 읽기만 base · 기다리기는 쓰는 쪽」 (only read-now in base · waiting stays with the consuming crate) is **a design choice, not a conclusion derived from peers**, and to go that way the following four must be settled first (adversarial review · code check):

- **Sharing fake-time state** — the transport manual clock (`crates/engram-dashboard-transport/src/testing.rs`) and the discovery fake (the test clock in `lib.rs`) have sleep push `now` forward. Even if the two are separated, one fake must implement both seams with the same state (e.g. the consuming crate's sleep trait takes the base `Clock` as a supertrait).
- **`UsageClock` is not the same contract** — it is 「그 시계가 만들어진 뒤 경과」 (elapsed since this clock was created; holds an origin) + signed epoch **seconds** (including before 1970), and the fake moves the two axes separately (`usage_service/clock.rs`). It does not map directly onto `Instant` + `now_epoch_ms`.
- **The tokio time domain** — transport returns `std::time::Instant` while sleeping with tokio (`clock.rs`). `tokio::time::pause` does not stop the std `Instant` — no problem now because the test strategy is a manual clock, but the clock domain must be written down when merging.
- **Separating trait from implementation** — the transport trait signature uses only `Duration` · `BoxFuture`, and tokio is only in the implementation. 「통째로 합치면 base 가 tokio 를 끈다」 (merging wholesale makes base pull in tokio) applies when the implementation is moved too (adversarial review correction).

## Adversarial review

One cross-family run (codex · effort high · web_search) → **BLOCK** (the conclusion went beyond the evidence). Applied: downgraded the frequency claims · added two counterexamples · corrected Firecracker · rewrote the implication as 「설계 선택 + 먼저 정할 넷」 (a design choice + four things to settle first). Spot re-verification passed: zed `scheduler::Clock` (only `utc_now` · `now`) · Materialize `NowFn` (epoch milliseconds `u64`) · quinn `Runtime` (`now` + `new_timer`) · tokio `pause` docs. Reviewer's judgment: 「지금 읽기만 합치고 기다리기는 쓰는 쪽」 (merge only read-now and leave waiting to the consuming crate) is not a peer conclusion, but it is a workable design once the four above are settled.

## Limits · gaps

- GitHub code search hit its rate limit midway — deno · nushell · wezterm · rust-analyzer · databend had only 0 hits for a single phrase search, so the absence of a seam is not confirmed. cargo · helix were not examined.
- The Fuchsia API names · Firecracker shape were seen only through summaries.
- How widely zed `scheduler::Clock` is used outside gpui was not measured.
