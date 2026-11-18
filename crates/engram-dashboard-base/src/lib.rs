//! # engram-dashboard-base — the bottom foundation layer, a leaf crate (zero domain knowledge)
//!
//! The bottom foundation layer of the dependency graph — general helpers and foundation infrastructure go into **purpose-named modules**
//!   (ADR-0269 decisions 1 · 9). Nine tenants: [`file`](mod@file) (read/write decisions for data files, capped reads, atomic writes/copies,
//! corrupt copies, temp sweeping — the OS-dependent part passed in by the caller) · [`logging`](tracing global init + a file log for one
//!   process run) · [`text`](bytes → lowercase hex) · [`time`](wall-clock epoch ms · the "read now" clock seam) ·
//! [`path`](normalizing hand-typed path spelling) · [`sync`](reclaiming lock poisoning — no warning, poison mark kept) · `testing`(polls
//!   until a condition holds) · [`writable`](can a folder really be written to — write probe · one retry for a folder that vanished mid-check) ·
//! [`panic`](mod@panic)(panic containment mark · hook body that ends the process outside the mark). Each module (header and public
//!   function docs) is the source of truth for its responsibility.
//!
//! ★The fake clocks of `testing` and `time` are behind the cargo feature `test-support`★ (ADR-0275 decision 5) — seen only by this crate's
//!   own tests (`cfg(test)`) and consumers enabling it as a dev dependency. The feature is absent from the production
//! dependency graph of the daemon · shell · CLI, which the CI gate measures (`.github/workflows/ci.yml` — a 0-line expectation paired with
//!   an at-least-1-line expectation).
//!
//! ## Three tenancy conditions (if even one is unmet, it does not belong here)
//!
//! 1. **Used or copied in several places right now** (ADR-0269 decision 7 — replaces ADR-0175's "at least two consumer crates"). Judged by
//!   present use, by the main session, without asking the user. Something with only one copy is a module inside that consumer.
//! 2. **Zero domain knowledge.** No agent runtime · wire contract · daemon lifecycle management even as types, so this crate depends on no
//!   workspace crate (the gate below is that wall).
//! 3. **No references to each other inside this crate.** Entangled tenants are one lump, and putting one lump at the bottom drags all of
//!   it to every consumer.
//!
//! ★OS-dependent **production** code does not belong here even if it meets all three★ — it is for the OS-layer crate
//!   `engram-dashboard-platform` (ADR-0266 decisions 2 · 4); this crate does not depend on `windows`.
//! Test OS branches (`#[cfg(windows)]` tests) are outside this rule (ADR-0266 「Rationale」).
//!
//! ★**Not split into small purpose-specific crates, and the name stays base**★ (ADR-0269 decision 1) — divided into purpose-named modules
//!   inside. At the bottom with little entanglement, splitting later costs little, and gate ③ below backs that.
//! ADR-0175's clause "reopen the rejected alternative before accepting a third tenant" is replaced by this decision.
//!
//! ## Isolation gates (invariant) — three, each on a different axis
//!
//! **① Workspace dependency ceiling** (the wall of tenancy condition ② — the compiler does not enforce leaf-ness):
//! `cargo tree -p engram-dashboard-base --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **exactly 1 line = itself**.
//! ★Do not replace this with a grep over manifest text★ — rename · the `[dependencies.<name>]` table form · `[build-dependencies]` ·
//!   inactive targets · `optional` all slip through. Do not trim the flags either (same reasoning as the messaging · command ceiling gates).
//!
//! **② Tauri import 0** (ADR-0003):
//! `rg "^\s*use tauri" crates/engram-dashboard-base/src/` → **0 lines**.
//! ★Do not read ① as covering this axis★ — ① counts **only workspace members** (`rg "^engram-dashboard"`), so third-party `tauri` passes
//!   through.
//! ★"It is a leaf crate, so it is safe" is not true either★ — the axis ADR-0003 hangs on is **which binary it is linked into**, and this
//!   crate is linked into the headless daemon (no window or webview), which must not be bound to a transport method.
//! The `logging` init function is the typical spot where "firing just one event is convenient". The pattern is anchored on the import line
//!   so this header does not trip itself.
//!
//! **③ Tenants do not reference each other** (the wall of tenancy condition ③):
//! `rg "(crate|super)\s*::\s*(logging|text|time|path|sync|testing|writable|file|panic)" crates/engram-dashboard-base/src/` → **0 lines**.
//! ★Do not widen it to bare `crate::`★ — that is plain Rust (noise, not a gate). Module **names** are hand-written, so **when tenants
//!   grow, names must be added here** (the same hole as the messaging regex; ① looks only outside the crate).
//! ★Known limit★: if `logging` later gets submodules, a submodule calling **its own parent** via the crate root is falsely flagged
//!   (indistinguishable from a sibling reference) — then use a relative path or re-decide this gate.
//! The regex does not check the boundary after the name, so a name that **starts with** a common-word tenant name is also caught (noisy
//!   false positive, never a miss).
//! An import grouping several items in braces (`use crate::{a::x, b::y};`) is **missed** — a `{` sits between the prefix and the tenant
//!   name (rustfmt unfolds one-item groups, so only groups of two or more slip through).
//! ★It catches comments too★ — a comment spelling that path is caught. ★Do not write that path as an example in this section★ (this header
//!   would make the gate bite itself); the command line above escapes only thanks to the parentheses in the regex (as CI `daemon gate 2`).
//!
//! ★This crate produces no generated artifacts★ — `serde_json` is here (the `file` read decision looks at JSON documents — ADR-0291
//!   decision 3), but there is no serde derive, no ts-rs, no `declare_commands!`, so no `bindings/` directory.
//! Do not add it to the CI ts-rs sync gate path list.

// ADR-0175
// ADR-0269
// ADR-0291
pub mod file;
pub mod logging;
// ADR-0292
pub mod panic;
pub mod path;
pub mod sync;
// ADR-0275
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod text;
pub mod time;
pub mod writable;
