# Logging in a low-level OS wrapper crate — how not to pull in a shared base crate

- **Status:** medium · design-decision mode · 2026-10-02 · adversarial review FIX applied (「적대 리뷰」 (adversarial review) below) · decision: do not wrap (user 2026-10-03) — ADR-0268 rewritten · platform uses tracing directly (ADR-0266)
- **Method:** 2 collectors (peer code · mechanisms/official guidelines) → main grounding (5 load-bearing items checked against source text) → one cross-family adversarial review.
- **Confidence legend:** certain (independent cross-confirmation) · likely (single-source grounding) · uncertain (unverified).

## Why it was researched

ADR-0266 (OS-dependent code → the `platform` crate, workspace dependencies 0) collides with 「로그는 base 입구로만」 (logs only through the base entry) in the first version of ADR-0268 (`ebfdafc` — rewritten before push). The `Drop` of `RmSession`, to be moved into `platform`, leaves a failure as a warning log (`impl Drop for RmSession` in `crates/engram-dashboard-agent/src/platform/file_holders.rs` — 「돌려보낼 곳이 없으므로 남기는 것은 로그뿐」 (there is nowhere to return it, so all that can be left is a log)). It makes no sense for `platform` to depend on the whole of base because of one log line (user 2026-10-02). And the atomic write to be sent to base (`src-tauri/src/fsutil.rs`) holds a Windows-only retry, so it requires base → platform — with both together it is a cycle.

## Findings

### 1. The peer mainstream = a low crate depends directly on a log facade — without going through a shared util crate (certain)

- wezterm `portable-pty` → `log` directly (`pty/Cargo.toml` — `log.workspace = true`, depends only on `filedescriptor`, not on a util) · rust-analyzer `stdx` (the util crate itself) → `tracing` directly · cargo `cargo-util` → `tracing` · zed `util` → `log` · ripgrep `grep-cli`/`grep-searcher` → `log` · alacritty `alacritty_terminal` (including ConPTY) → `log` · nushell `nu-system`/`nu-utils` → `log`. Source = each repository's `Cargo.toml` (portable-pty · stdx were compared against source text by main).
- **The facade itself already fills the slot of 「작은 로깅 crate」 (a small logging crate)** — in the peer sample there was no case of a low crate using a small crate that pulls only logging out of util. zed's `zlog` (a logger implementation + scope macros) is used by low crates (`sum_tree` · `rope`) only as a dev-dependency, and production code calls `log` directly — but higher crates (`project` · `zlog_settings`) use it as a production dependency (adversarial-review correction · main checked it in `[dependencies]` of `crates/project/Cargo.toml`). The closest precedent is rustc's `rustc_log` (a crate re-exporting `tracing`), but **it was seen only through search summaries (uncertain)**.
- Variants: mio keeps `log` as an optional feature that is on by default, and its internal macros do nothing when it is off (`src/macros.rs`) · tokio keeps `tracing` as a non-default optional dependency.

### 2. The lowest handle layer does not print logs at all (certain)

std (`OwnedFd` · `OwnedHandle` · `File`) · wezterm `filedescriptor` · rust-analyzer `paths` · cargo `home` have no logging dependency. Errors are returned as `Result`, and a close failure in `Drop` is silently discarded. The std `File` docs: 「Errors detected on closing are ignored by the implementation of `Drop`. Use the method `sync_all` if these errors must be manually handled.」

### 3. There are only three ways to handle a `Drop` failure, and the official guideline recommends two (certain)

- Rust API Guidelines **C-DTOR-FAIL** (compared against source text): 「Instead of failing in a destructor, provide a separate method for checking for clean teardown, e.g. a `close` method, that returns a `Result` to signal problems. If that `close` method is not called, the `Drop` implementation should do the teardown and ignore or log/trace any errors it produces.」 — https://rust-lang.github.io/api-guidelines/dependability.html
- Real peers: (a) silently discard — std · wezterm `OwnedHandle`/`PseudoCon` · alacritty `Conpty` · rust-analyzer `JodChild` (b) an explicit `close() -> Result` + a best-effort `Drop` — tempfile `TempDir::close` (std `File` has no close method, and `sync_all` is only content synchronization — adversarial-review correction) (c) a facade warning in `Drop` — **only at a higher layer**: cargo `FileLock` (`src/util/flock.rs` — `tracing::warn!("failed to release lock: {e:?}")`, compared against source text by main).
- Low crates that substitute a callback · hook for logging were 0 in the sample (this is not a proof of absence).

### 4. The official facade guideline = libraries link only the facade (certain)

Both `log` (`src/lib.rs`: 「Libraries should link only to the `log` crate…」) and `tracing` (`tracing/src/lib.rs`: 「Libraries should link only to the `tracing` crate…」, a library does not call `set_global_default`) (compared against source text by main). Weight: the `tracing` default features pull in a proc-macro (`tracing-attributes` → the syn family), and with `default-features = false` it drops out · `log` is effectively dependency-free (the exact count is unchecked with `cargo tree` — uncertain).

### 4-1. The caller's subscriber also receives `log` events (likely)

base does `try_init()` with the `tracing-subscriber` default features (near `crates/engram-dashboard-base/src/logging/mod.rs:417` · `Cargo.toml` only adds `features = ["env-filter"]`). If its default feature `tracing-log` is on, `try_init()` installs a `LogTracer` and passes `log` facade events to the same subscriber (quoted by the adversarial review — docs.rs `tracing-subscriber` features · `util.rs` · not compared by main). That is, even if `log` is used in track C, a path to the file log already exists.

### 5. Wrapping in a macro keeps the call-site information (likely)

A `macro_rules!` re-export · wrapper expands `module_path!()` · `file!()` · `line!()` **at the call site**, so the caller's module · file · line remain (basis = `log`'s `macros.rs` uses `module_path!()` via `$crate` · the std `module_path!` docs). A function wrapper prints the wrapper's module (`#[track_caller]` gives only file · line — standard semantics, no dedicated source). This is the basis of decision 3 (the macro entry) of the first version of ADR-0268 (`ebfdafc` — rewritten before push), and it means the track where 「작은 로그 crate 가 매크로를 재수출」 (a small log crate re-exports the macros) does not lose the call-site filter (`EnvFilter`).

## Fit with our constraints

Constraints: 「로그는 base 입구로만」 (logs only through the base entry) of the first version of ADR-0268 (`ebfdafc` — rewritten before push) (「나중에 컨트롤이 안 된다」 (it can't be controlled later)) · ADR-0266 (`platform` workspace dependencies 0) · ADR-0269 (base is a general-purpose helper that does not know the OS) · the user's 「로그 때문에 base 전체를 끄는 건 말이 안 된다」 (pulling in all of base because of logging makes no sense) · review F3 (the Windows retry of the atomic write).

| Track | Peer precedent | ADR-0268 first version (`ebfdafc` — rewritten before push) | ADR-0266 dependencies 0 | Atomic-write cycle (F3) |
|---|---|---|---|---|
| **A. platform prints no logs** — failure is a `Result`, cleanup is an explicit `close()/end() -> Result` + a silent `Drop` (C-DTOR-FAIL). The caller (agent etc.) prints the logs through base | std · filedescriptor · paths · home (the general form of the lowest layer) · the official guideline | kept (no logs in platform) | kept | resolved — platform is at the very bottom, base → platform is allowed (base only asks platform for the Windows decision) |
|  ↳ A's condition | | | | an explicit `end()` must be called on **both the success and the early-error paths** — `holders_of` has four exits after opening the session (`file_holders.rs:63`) (two `return Err` · one early `return Ok` inside the loop · the convergence-failure `Err` at the end), so with only a silent `Drop`, an `RmEndSession` failure on that path is not seen. Restart Manager limits the number of open sessions per user session (MS `RmStartSession` docs — quoted by the adversarial review · not compared by main, likely) |
| **B. A small log-only crate** — split the entry macros of the first version of ADR-0268 off base into a leaf crate, and base · platform depend only on it | rustc_log (uncertain) · zed zlog is an implementation crate, so it differs | **breaks (literally)** — the first version of ADR-0268 (`ebfdafc` — rewritten before push) puts the entry in **base** and allows a direct `tracing` dependency only for base. It keeps the intent that the entry is in one place, but that version needs an amendment (adversarial review) | breaks (depends on one log crate) — but that crate is only a facade | resolved — the log crate is at the very bottom, platform → log, base → platform/log |
| **C. An exception where only platform depends directly on the facade** | peer mainstream (portable-pty · alacritty etc.) | one exception (name written into the gate) | **kept** — ADR-0266 means zero workspace crate dependencies and the gate counts only the `engram-dashboard` prefix (adversarial-review correction) | resolved — the same direction as A |
| (rejected) platform → base | — | kept | breaks + pulls in all of base | cycle (cannot live together with base → platform) |

## Rejected candidates → the ADR's rejected alternatives

- **platform → base (because of logging)** — rejected by the user (「base 전체를 포함시키는 게 말이 안 됨」 (including all of base makes no sense)) + the F3 cycle.
- **The app receives platform's logs through a callback · global hook** — the grounds for rejection are weak: 0 in the peer sample is not a proof of absence, and C-DTOR-FAIL does not exclude a hook either (adversarial review). It is left as a **gap**, not a rejection.

## Adversarial review

cross-family (codex · effort high · web_search), once → **FIX**. Applied:
- The verdict that track B 「지킨다」 (keeps) the first version of ADR-0268 (`ebfdafc` — rewritten before push) was wrong — it breaks it literally (an amendment is needed). Table corrected.
- The verdict that track C 「깬다」 (breaks) ADR-0266 was wrong — a third-party facade does not break zero workspace dependencies. Table corrected.
- For track A, the explicit `end()` must cover the early-error paths for failures to be seen — a condition line added.
- The use of zed `zlog` corrected (higher crates use it as a production dependency) · the classification of std `File::sync_all` corrected · the `LogTracer` path added (§4-1) · the hook rejection demoted to a gap.
- Spot re-verification passed: the API Guidelines wording · cargo `FileLock`'s `Drop` warning · the call-site target of `tracing` macros.
- The reviewer's judgment: the one that keeps the constraints literally is **A** (condition = an explicit `end()` covers every path, and decide the path by which cleanup errors reach the caller).

## Limits · gaps

- The peers are a sample (about 10 repositories · reading source text with `gh api`). deno · rustup were not looked at.
- No real-use precedent was confirmed for 「util 에서 로그만 뗀 작은 crate」 (a small crate with only the logging taken out of util) (rustc_log is only from search summaries).
- The exact dependency counts of `log`/`tracing` were not measured with `cargo tree`.
