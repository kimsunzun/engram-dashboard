# How to verify TOCTOU/generation-guard concurrency tests — research (cross-family)

**Status:** complete / sourced report (medium, design-decision mode)
**Date:** 2026-06-28
**Method:** Claude (Sonnet) 2 tracks + Codex 1, all independent and BLIND → opus cross-check + adversarial verification of the key claims.
**Confidence legend:** certain / likely / uncertain.

## Motivation (why we investigated)

During S14 daemon_client tracing backfill QA, we found that the existing stress test `toctou_stress_reconnect_no_stale_down_clobber` **fails intermittently** (~1/20 even at baseline T2). Direct diagnosis: the guard (`lifecycle::publish_if_current` — atomizing the `generation==my_gen` comparison + `watch::send` as one critical section under a `std::sync::Mutex`) is correct, and it was a false positive: **the test depends on magic timing of server hold (5ms) vs assert sleep (3ms), and misjudges the current task's legitimate Down as a stale clobber**. We investigated following the user's instruction "매직넘버로 때우지 말고 OSS 사례로 풀라" (do not patch it with a magic number; solve it with OSS precedents).

## Findings (confidence · source)

### 1. Applying loom directly — not possible (for our async case) · certain
- loom **exhaustively explores interleavings** of concurrency code inside `loom::model` with DPOR, a tool that replaces probabilistic stress. (certain — https://docs.rs/loom/)
- **intrusive**: only code *instrumented* with `loom::sync`/`loom::thread` is visible. Plain `std`/third-party sync is visible only if that library is also loom-instrumented. (certain — https://docs.rs/loom/#intrusive-implementation)
- tokio has loom instrumentation **internally** (`crate::loom::sync`, loom CI `--cfg loom`). But **"tokio uses loom internally" ≠ "a user can verify tokio async code with loom as is."** For a user to look at `tokio::sync::watch`/`mpsc` with loom, **tokio has to be recompiled with `--cfg loom`** (patch override), and launching the `tokio::runtime` multi-thread inside `loom::model` panics (loom #256). (certain — loom #256, tokio loom CI, tokio/src/sync/watch.rs)
- Swapping only the `std::sync::Mutex` part for `loom::sync::Mutex` under cfg(loom) is a standard pattern and possible. But the `tokio::sync::watch::send` inside it is outside loom's tracking.

### 2. The solution = unit level + deterministic rendezvous · certain (both families converge)
- **Bring it down to a unit:** remove the socket/server and verify the guard methods by calling them directly in order — stale generation → not published, current → published. It proves the compare+send rule without network timing. (certain — both families + axum testing precedent)
- **Deterministic rendezvous:** reproduce the order "comparison passes → forced preempt → another thread bumps → stale write" *exactly* with `tokio::sync::Barrier`/`Notify`/`oneshot` or a `cfg(test)` pause-hook (seam). It hits only the stale-vs-current interleaving, without sleep → false positives are blocked at the source. (certain — tokio Barrier/Notify, turmoil barrier hooks, matklad "properly testing concurrent data structures")

### 3. Supporting tools — partially suitable · likely
- **tokio::time pause/advance** (`start_paused`): effective for removing wall-clock dependence, but cannot solve by itself the order problem of "which task fired the Down." (certain — tokio time docs; warns that advance does not guarantee that all timers are processed)
- **shuttle (awslabs):** scheduler-controlled randomized exploration (not exhaustive), has a tokio wrapper. Cheaper and more scalable than loom but no completeness guarantee. (certain — github.com/awslabs/shuttle)
- **turmoil (tokio-rs):** network simulation (delay · partition · barrier) — for distributed/network tests. Too much for unit verification of our guard. (certain)

## Cross-verification table (Claude ↔ Codex)

| Claim | Claude (2) | Codex | Convergence |
|---|---|---|---|
| loom is intrusive — it sees only instrumented code | ✓ | ✓ | converged (certain) |
| loom verification of a user's tokio::sync = tokio recompile needed / unrealistic | ✓ (#256) | ✓ (#256, CI) | converged (certain) |
| solution = bring it down to a unit + deterministic rendezvous | ✓ | ✓ | converged (certain) |
| adopting loom = low ROI now | ✓ | ✓ | converged |
| tokio::time cannot solve the order problem | ✓ | ✓ | converged |
| shuttle = a randomized alternative | ✓ | ✓ | converged |

Disagreement: none (complementary — the matklad pause-hook case vs the turmoil barrier-hook case).

## Recommendation

1. **Do not adopt loom/shuttle now** — low ROI (tokio recompile · cfg(loom) dual build, whereas, once separated out, rendezvous/unit tests give most of the value more cheaply). It is worth revisiting when T4 comes in (the concurrency surface goes up with reconnect · backoff) (matches the existing note in the lifecycle.rs comment).
2. **Replace the flaky test with the following:**
   - (a) **Guard unit test** — verify `Lifecycle::publish_if_current`/`store_cmd_if_current` directly with stale vs current generation (zero sockets · zero sleeps). A clean proof of the invariant.
   - (b) **Deterministic rendezvous test** (optional) — if integration-level coverage is wanted, make the stale-vs-current interleaving deterministic with Barrier/Notify or a cfg(test) pause-hook. Remove the existing sleep stress, or demote it to wiring-only.

## Gaps · limits

- Injecting a pause-hook (seam) adds `cfg(test)` points to production code — an intrusiveness vs determinism trade-off (it may be unnecessary if the guard is separated out as a unit).
- Controlled verification that this skill itself is better than a single model is not yet done (skill ⚠️ verification status).

## Sources (primary)
loom: docs.rs/loom · github.com/tokio-rs/loom (#256) · tokio loom CI (.github/workflows). Rendezvous: docs.rs/tokio Barrier·Notify·oneshot · docs.rs/turmoil (#barriers) · matklad.github.io/2024/07/05. Time/alternatives: docs.rs/tokio/time · github.com/awslabs/shuttle · github.com/tokio-rs/turmoil.
