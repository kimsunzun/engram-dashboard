//! # engram-dashboard-base — the bottom foundation layer, a leaf crate (zero domain knowledge)
//!
//! It is the bottom foundation layer of the dependency graph — general helpers and foundation infrastructure go into **purpose-named modules**
//! (ADR-0269 decisions 1 · 9). There are nine tenants now: [`file`](mod@file) (read decisions for data files (version · corruption) · 「덮어도
//! 되나」 (may we overwrite?) write decisions · capped reads · atomic writes · atomic copies · corrupt copies · sweeping leftover temp files — the OS-dependent part is
//! passed in by the caller) · [`logging`](tracing global
//! init + a file log for one process run) · [`text`](bytes → lowercase hex) · [`time`](wall-clock epoch milliseconds ·
//! the 「지금 읽기」 (read now) clock seam) · [`path`](normalizing the spelling of hand-typed paths) · [`sync`](reclaiming lock poisoning — without a warning,
//! keeping the poison mark) · `testing`(test waiting — polls until a condition holds) · [`writable`](can a folder really be written to —
//! write probe · one retry for a folder that vanished mid-check) · [`panic`](mod@panic)(panic containment mark — containment · mark off ·
//! hook body that ends the process when outside the mark). Each module (header and public function docs) is the source of truth for its responsibility.
//!
//! ★The fake clocks of `testing` and `time` are behind the cargo feature `test-support`★ (ADR-0275 decision 5) — only this crate's
//! own tests (`cfg(test)`) and tests of consumers that turn that feature on as a dev dependency see them. That feature is absent from the production
//! dependency graph of the daemon · shell · CLI, which the CI gate measures (`.github/workflows/ci.yml` — a 0-line expectation paired with an at-least-1-line expectation).
//!
//! ## Three tenancy conditions (if even one is unmet, it does not belong here)
//!
//! 1. **Used or copied in several places right now** (ADR-0269 decision 7 — replaces ADR-0175's 「소비자 crate 둘
//!    이상」 (at least two consumer crates)). The standard is not set in advance but judged by present use — the main session judges and
//!    reports without asking the user. Something with only one copy is a module inside that consumer.
//! 2. **Zero domain knowledge.** It does not know the agent runtime · wire contract · daemon lifecycle management even as types. So this crate
//!    depends on no workspace crate at all (the gate below is that wall).
//! 3. **No references to each other inside this crate.** If tenants are entangled, they are one lump, and putting one lump
//!    at the bottom drags that whole lump along to every consumer — the reason for making this crate disappears.
//!
//! ★OS-dependent **production** code does not belong here even if it meets all three★ — it is for the OS-layer crate
//! `engram-dashboard-platform` (ADR-0266 decisions 2 · 4). So this crate does not depend on `windows`.
//! Test OS branches (`#[cfg(windows)]` tests) are outside this rule (ADR-0266 「근거」 (rationale)).
//!
//! ★**It is not split into small purpose-specific crates, and the name stays base**★ (ADR-0269 decision 1) — it is divided into
//! purpose-named modules inside. The judgment is that, sitting at the bottom with little entanglement, splitting later costs little, and what now backs that 「엮인 데가
//! 적다」 (little entanglement) is gate ③ below. ADR-0175's clause 「셋째 입주자를 받기 전에 「거부한 대안」을
//! 다시 연다」 (reopen the rejected alternative before accepting a third tenant) is replaced by this decision — the debate is not raised again each time tenants grow.
//!
//! ## Isolation gates (invariant) — three, each on a different axis
//!
//! **① Workspace dependency ceiling** (the wall of tenancy condition ② — the compiler does not enforce leaf-ness):
//! `cargo tree -p engram-dashboard-base --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **exactly 1 line = itself**.
//! ★Do not replace this with a grep over manifest text★ — rename · the `[dependencies.<이름>]` table form ·
//! `[build-dependencies]` · inactive targets · `optional` all slip through in normal Cargo syntax.
//! Do not trim the flags either (the shape leaks that much — same reasoning as the messaging · command ceiling gates).
//!
//! **② Tauri import 0** (ADR-0003):
//! `rg "^\s*use tauri" crates/engram-dashboard-base/src/` → **0 lines**.
//! ★Do not read ① as covering this axis★ — ① counts **only workspace members** with `rg "^engram-dashboard"`, so the
//! third-party `tauri` passes straight through. The two gates do not overlap.
//! ★Nor is "it is a leaf crate, so it is safe" true★ — the axis ADR-0003's invariant hangs on is neither the crate name nor leaf-ness but
//! **which binary it is linked into**, and this crate is linked into the headless daemon. It means that binary, with no window or webview, is
//! bound to a transport method. The `logging` init function is the typical spot where "firing just one event
//! is convenient". The pattern is anchored on the import line so this header does not trip itself.
//!
//! **③ Tenants do not reference each other** (the wall of tenancy condition ③):
//! `rg "(crate|super)\s*::\s*(logging|text|time|path|sync|testing|writable|file|panic)" crates/engram-dashboard-base/src/` → **0 lines**.
//! ★Do not widen it to bare `crate::`★ — that is plain Rust, so it becomes noise, not a gate. The price is that module
//! **names** are hand-written, so **when tenants grow, names must be added here to be seen** (the same kind of
//! hole as the messaging regex, and ① does not cover that hole — ① looks only outside the crate).
//! ★Known limit★: if `logging` later gets submodules, the path by which such a submodule calls **its own parent** via the crate root
//! is falsely flagged (the gate cannot tell it from a sibling reference). Then call by a relative path or re-decide this
//! gate. The regex does not check the boundary after the name, so another name (a sibling function · submodule) that **starts with** a tenant name that is a common word
//! is also caught if it comes after the same prefix — a noisy false positive, so there is no way to miss it.
//! An import that groups several items in braces (the `use crate::{a::x, b::y};` shape) is **missed** — because
//! a `{` sits between the prefix and the tenant name. rustfmt unfolds one-item groups, so only groups of two or more items slip through.
//! ★It catches comments too★ — if a comment in this crate spells that path, that line is caught.
//! ★Do not write that path as an example in this section★ — the moment you do, this header makes the gate bite itself. The command line above
//! is not caught only thanks to the parentheses in the regex (the same defense as net gates 4 · 2b).
//!
//! ★This crate produces no generated artifacts★ — `serde_json` is here (the read decision of `file` looks at JSON documents —
//! ADR-0291 decision 3), but there is no serde derive, no ts-rs, no `declare_commands!`, so there is no `bindings/` directory. Do not add it to the CI
//! ts-rs sync gate path list.

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
