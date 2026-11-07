# Panic handling practice in long-running Rust programs — is our design excessive? (2026-10-12)

- **Status:** medium · reflects 1 adversarial review (codex BLOCK → 7 findings addressed) · awaiting decision — basis material for decisions (A) and (B) of panic TRD edition 11 (`docs/process/S21-panic-policy/trd.md`).
- **Trigger:** user 「우리가 너무 민감하지 않나. 다른 곳은 보통 어떻게 하나」 (2026-10-12) (are we too sensitive? what do other projects usually do?) — about option (A) of TRD edition 11, 「공유 락을 쥔 동안 가드 표시를 끈다」 (turn off the guard marker while holding a shared lock).
- **Method:** three collection tracks (servers/infrastructure / desktop, editors, terminals / official Rust docs and community) in parallel → main-session grounding (re-checked the four key ones against original source: rust-analyzer · materialize · tikv · zed) → codex adversarial review.
- **Confidence legend:** certain = source text verified · likely = single source or partially verified · uncertain = could not verify. Line numbers are against the default branch on 2026-10-12 and drift.
- Earlier survey = `docs/research/panic-policy-peers-2026-10-08.md` on the `origin/v0.3.3/feat/panic-policy` branch (comparison of options 1~4). This document adds 「보통은 어디까지 가두나 · 락 오염을 어떻게 다루나」 (how far do projects usually contain panics, and how do they handle lock poisoning) on top of it.

## Conclusions

(Version after the adversarial review — the exaggerations and omissions the review flagged as BLOCK are fixed. Where they were fixed = 「적대 리뷰」 (adversarial review) below.)

1. **「가두지 못한 패닉 = 끝」 (a panic that cannot be contained = the end) is a common shape — our 「즉시 종료」 (immediate exit) is not oversensitive (likely).** In the sample, tikv (`_exit(1)`/`abort()`) · materialize (`abort()`) · zed (`abort()` after a minidump) · zellij (exits after cleaning up the session) have a hook or a server that ends the process. Some projects set the release profile to abort outright — deno · OpenVMM · Turso, and most of Oxide's programs (author's statement). Unwind programs without a hook (vector · linkerd2-proxy, etc.) lose only that thread and the process stays alive — 「끝낸다」 (it ends the process) is not universal.
2. **The containment scope is usually narrow, and our design is on the wide side (likely).** Containment sits mostly at places that do not change state or at isolated units — rust-analyzer read-only requests · nushell completion sources · materialize's named places · a single herdr FFI callback. There are counterexamples — Qdrant contains a whole health-check future that changes shared readiness state, and runs it again. tokio's task panic propagation (default) and tower-http `CatchPanicLayer` (feature `catch-panic` + an explicit layer — opt-in) do not address the shared-state problem. Our design also contains requests and agents that change state, so it is wider than most of the sample.
3. **Bundles 1 and 2 (guard marker + the hook ends the process when outside the marker) have a precedent in materialize (certain).** A per-thread counter (`CATCHING_UNWIND`) marks 「자기 `catch_unwind` 안」 (inside its own catch_unwind), and the hook aborts when outside it. ★But for async it keeps a separate **per-tokio-task marker**, not a per-thread one★ (`CATCHING_UNWIND_ASYNC` · `src/ore/src/future.rs`) — in a multithreaded runtime a future can move threads at every `.await`, so a thread marker alone is wrong. Our request-side guards (WS command · control HTTP · MCP) are async, so the TRD must handle this.
4. **The principle (「락을 쥔 중의 패닉 = 공유 불변식이 깨졌을 수 있다 → 계속 쓰지 말라」, a panic while holding a lock = the shared invariant may be broken → do not keep using it) has precedent, but I found no precedent for the mechanism (turn off the guard marker while holding a lock, so the process exits immediately) (likely).** Principle = std `Mutex` docs (「likely tainted」 · propagate with `unwrap()`) · sunshowers 「on poisoning」 (a panic inside a critical section = damaged shared invariant · it discusses abort as the response). The same post separates out that **read-only critical sections and simple writes that do not break the invariant are different** — grouping 「공유 락 전부」 (all shared locks) under one rule is coarser than that. rust-analyzer resolves the same principle as 「상태를 바꾸는 처리는 아예 안 가둔다」 (do not contain state-changing handling at all).
5. **No project in the sample set a separate policy for lock poisoning (likely).** Two branches — ① `parking_lot` (no poisoning — the lock is released silently on panic · zed · wezterm · helix · tikv) ② std lock `.lock().unwrap()` (the next thread to take the lock panics — zellij about 227 places · herdr about 179 places). ★Neither guarantees that the process exits★ — ① goes on with possibly broken data, and ② only kills that thread. Exit comes separately, from the hook in 1.

**What this means for us (main-session interpretation):** 「즉시 종료」 itself is not oversensitive. The cause of the complexity is **the decision to contain more widely than most of the sample** (ADR-0288 「패닉에 의한 종료는 최대한 하지 않는다」 (avoid exiting because of a panic as far as possible)), and the lock rule of (A) is the price of that wide containment. The axis to choose on: ① contain widely and add the lock rule (the current proposal — the principle has precedent · the mechanism is our own design) ② like rust-analyzer, do not contain requests that change shared state, so the lock rule goes away. Either way the async guard needs a per-task marker (conclusion 3).

## By project (axes: release-profile panic · containment scope · hook · lock poisoning)

### Servers · infrastructure

| Project | Release profile | Containment scope | Hook | Lock poisoning |
|---|---|---|---|---|
| tikv | unwind explicit (`Cargo.toml:447`) certain | a few places (scope not checked) uncertain | 「Exit the whole process when panic」 — log → `abort()` or `_exit(1)` (`components/tikv_util/src/lib.rs:525-607`) certain | depends on `parking_lot` certain · whether it is the main lock is likely |
| materialize | unwind (default) certain | only named places that call its own `catch_unwind` likely | backtrace · Sentry → 「Instructs the entire process to abort if any thread panics」 · exception when inside its own wrapper (`src/ore/src/panic.rs:42,115-140,261`) certain | unknown |
| deno | abort certain | — | not checked | unknown |
| databend | unwind certain | unknown | appears to log only likely | unknown |
| sccache | unwind certain | unknown | diagnostic (records the location) certain | unknown |
| vector · linkerd2-proxy | unwind certain | unknown | no operational hook likely (→ default behavior: only that thread dies) | unknown |
| tokio (library) | — | default `UnhandledPanic::Ignore` — a task panic goes to the `JoinHandle` and the runtime continues (`tokio/src/runtime/builder.rs`) certain | — | — |
| tower-http (library) | — | `CatchPanicLayer` = panic → 500 response · 「using panics for error handling is _not_ recommended」 · no mention of shared state certain | — | — |

### Desktop · editors · terminals

| Project | Release profile | Containment scope | Hook | Lock poisoning |
|---|---|---|---|---|
| rust-analyzer | unwind certain | read-only requests only — 「they don't modify the state, so it's OK to recover」 · the state-changing `on_sync_mut` is 「isn't guarded by `catch_unwind`, so, please, don't make bugs :-)」 (`crates/rust-analyzer/src/handlers/dispatch.rs:34-44`) certain | unknown | unknown |
| zed | unwind certain | effectively none likely | minidump via a crash-handling child process → `abort()` (`crates/crashes/src/crashes.rs`) certain | `parking_lot` certain |
| wezterm | unwind certain | unknown | fatal-notice toast → default hook certain | `parking_lot` certain |
| zellij | unwind certain | none likely | on the main thread: report → `exit(1)` · elsewhere: the server removes all clients and ends the session (`zellij-utils/src/errors.rs:812-869`) certain | std lock `.unwrap()` about 227 places → cascade certain |
| nushell | unwind certain | only completion sources: panic → `ShellError` certain | restore terminal → report · does not exit certain | unknown |
| alacritty | unwind certain | none uncertain | Windows error box certain | unknown |
| herdr | unwind (default) certain | one FFI callback certain | client = restore terminal → default hook certain | `into_inner` in one place · about 179 other places `unwrap`/`expect` certain |
| helix | unwind certain | unknown | unknown | `parking_lot` certain |

## Official Rust docs · community

- `catch_unwind` docs: 「It is not recommended to use this function for a general try/catch mechanism」 — expected failures go through `Result` (https://doc.rust-lang.org/std/panic/fn.catch_unwind.html) certain.
- std `Mutex` docs: poisoning = 「likely tainted (some invariant is not being upheld)」 · 「Most usage of a mutex will simply unwrap() these results, propagating panics among threads」 · poisoning is 「only advisory」 (it can be taken out with `into_inner`) (https://doc.rust-lang.org/std/sync/struct.Mutex.html) certain.
- Whether to remove poisoning is under debate — `std::sync::nonpoison` tracking issue #134645 (unstable implementation · no stabilization PR) · Edition 2027 default-lock discussion #149359 (no consensus — the keep side = it prevents continuing with broken data · the remove side = little value for abort apps, and in operations it can break a service permanently) likely (read from summary pages).
- sunshowers 「on poisoning」: in favor of poisoning · recommends `.lock().unwrap()` · abort is fine for apps (https://sunshowers.io/posts/on-poisoning/) likely.
- `parking_lot` about 85.4 million downloads per month · 8,610 direct dependents (lib.rs, 2026-10) — it cannot be told apart whether it was chosen for the lack of poisoning or for speed.
- crash-only / microreboot (Candea & Fox): restarting a small isolated unit holds only when the state is isolated or kept outside (https://www.usenix.org/legacy/events/osdi04/tech/full_papers/candea/candea_html/) certain (concept) — 「공유 락을 쥔 중의 패닉 = 격리 실패 → 전체로 올림」 (a panic while holding a shared lock = isolation failed → escalate to the whole) is the main session's interpretation of that principle (no source).

## Gaps · abstentions

- Practice of Tauri apps · helix hook · wezterm containment scope · content of the deno hook — could not verify (GitHub code search rate limit).
- Statistics on the share of crates using release-profile abort — none.
- Mara Bos · BurntSushi · matklad on poisoning — no source found, so they are not cited.
- Primary sources on Erlang 「let it crash」 — not read.

## Adversarial review

codex (cross-family · effort high · web search), 1 round — decision: **BLOCK** (the conclusions are wider than the evidence). It confirmed that the four citations (rust-analyzer · materialize · tikv · zellij) match the original text. All seven findings addressed (conclusions 1~5 above rewritten):

| Finding | Type · severity | How addressed |
|---|---|---|
| Qdrant contains a future that changes shared state and runs it again (`src/common/health.rs`) | counterexample · high | conclusion 2: 「뿐」 (only) → 「대개」 (usually) + counterexample stated |
| sunshowers links a critical-section panic to damaged shared invariants and discusses abort | missing precedent · high | conclusion 4 = the principle has precedent · the mechanism was not found |
| `parking_lot` · `unwrap()` do not imply process exit | wrong inference · high | conclusion 5 corrected |
| materialize keeps a separate per-task marker for async (`src/ore/src/future.rs`) | incomplete analogy · high | added to conclusion 3 — to be reflected in the TRD |
| the inference that release-profile abort is exceptional — OpenVMM · Turso · Oxide | sample omission · medium | conclusion 1 corrected |
| did not distinguish read-only critical sections from invariant-preserving writes | logic gap · medium | stated in conclusion 4 |
| tower-http `CatchPanicLayer` is not the default (feature + explicit layer) | factual error · medium | conclusion 2 corrected |

Not re-verified by the main session: the Qdrant · OpenVMM · Turso sources, and the relevant paragraph of the sunshowers post (read by the reviewer — confidence kept at likely).
