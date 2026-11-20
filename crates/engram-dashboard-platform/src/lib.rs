//! # engram-dashboard-platform — OS layer, leaf crate (0 workspace dependencies, 0 domain knowledge)
//!
//! Gathers **production** code that differs per OS into this one crate (ADR-0266 decision 1). Whether something belongs here is decided by
//!   one thing — whether it depends on the OS, not by the number of consumers
//! (decision 2). Callers use only this crate's functions and handle types; OS branches (`#[cfg(windows)]`, `cfg!(windows)`,
//!   `#[cfg(unix)]`) live only inside it. OS branches of tests, test harness bins and `examples/`
//! stay there (ADR-0266 「Rationale」). Machine gates of this invariant: ④ (`cfg` location), ⑤ (OS crate dependencies), ⑥ (`std::os` paths)
//!   below. No other crate has a production `cfg` OS branch (ADR-0291 closed
//! ADR-0275 decision 15's time-limited exception); runtime OS rules without `cfg` are outside ④.
//!
//! Residents: [`process`] (PID liveness, start time and its three-way verdict, process table, child PIDs, identity list under a root,
//!   ending a process tree — Windows `taskkill`, ending one identity only if it still runs) ·
//! [`group`] (strong owner of a process group, weak handles, held members, join-notification port — Windows Job Object) · [`file_holders`]
//!   (processes holding a file open — Restart Manager) · [`fs`] (opening while denying
//! others' writes, classifying that failure, verdict and retry for a brief-holder failure, folder sync, creating and walking directory
//!   links — Windows junction, path keys and path comparison, folder-path normalization, the Win32 reserved-device-name test, file
//!   identity, the network share a path is on) · [`spawn`] (windowless spawn,
//!   tree-root spawn with a kill handle — Windows = suspended, put in the group, resume;
//! "program not found" verdict; spawning outside this process's Job — WMI) · [`shell`] (interactive default shell, wrapping a CLI in a
//!   console shell) · [`env`] (home dir, executable name/sibling path, env var name
//! comparison) · [`window`] (identity, placement and z-order, restoring placement, topmost window under the cursor, diagnostics,
//!   foreground checks, title bar chrome, snap overlay behind feature `snap-overlay`) ·
//! [`input`] (cursor position, primary button, Esc state) · [`pty`] (whether closing a pseudo-terminal waits for its output to be read).
//!   Module headers and public docs are the source of truth.
//!
//! Functions that take a window accept `raw-window-handle`'s `HasWindowHandle`; variant matching happens here too (TRD S21-slot-docking
//!   revision 6.3 §8, B10, revision 7 §5-1).
//!
//! ★`testing` (real-process helpers), `group::GroupRef::gone` and `spawn::wmi_create_raw` (diagnostic) are behind cargo feature
//!   `test-support`★ (TRD 1-3 §3-8; same name as base's ADR-0275 decision 5): declared
//! `#[cfg(any(test, feature = "test-support"))]`. CI gate ③ (a 1-or-more-lines expectation paired with a 0-lines expectation) measures
//!   that the feature is not in the daemon/shell/CLI production graph; commands and
//! expected values are canonical in `.github/workflows/ci.yml`.
//!
//! ★`window::snap` (snap layout overlay) is behind cargo feature `snap-overlay`, off by default★ (TRD S26-window-chrome §8-4, user
//!   decision Q1): only the shell feature `snap-overlay` turns it on; CI measures that it
//! is not in the shell production graph and that the build with it on compiles. This crate has only these two features (widening ADR-0275
//!   decision 13's "one `test-support`"); both only open/close code and do not turn on
//! `windows` bindings.
//!
//! ## Rules for taking things in
//!
//! 1. **0 domain knowledge** (ADR-0266 decision 3): "find the home directory" belongs here, but backend knowledge (`.claude` paths, codex
//!   home subfolders, lock file names) stays with the caller. Public functions take raw
//!    values (program, arguments, PID, path), and domain values such as exit codes arrive as arguments (ADR-0275 decision 12).
//! 2. **Interface = functions + handle types, compile-time branching** (ADR-0266 decision 5): no trait object picking the OS; a test fake
//!   is **the caller's** small trait, so this API has no test seam traits.
//! 3. **Branches outside Windows are placeholders** (ADR-0266 decision 7): no new implementations for other OSes — the branches that
//!   existed when moved (an empty answer, a `bash`/`HOME` one-liner) are just gathered. Exception = `fs`'s directory links (symbolic
//!   link), file identity (device number · inode) and path key (as is), whose behaviour on other OSes the design fixes — std only
//!   (TRD S25 §1-6 · ADR-0325 · ADR-0336).
//!    ★`#[cfg]`-split branches get no compile verification here★ (development and CI are Windows only — ADR-0230). `cfg!(windows)`
//!   branches (`env::env_key_eq`, the three `fs` failure classifications) are type-checked on
//!    both sides but never run on another OS.
//! 4. **Logs are printed only on paths with no way to return a result (`Drop`)** (TRD 1-3 §3-7 —
//!   `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`). Other failures return as values and the caller words them
//!    (no domain words in this crate's logs — rule 1). Printing calls the `tracing` facade directly (ADR-0266 decision 9, ADR-0268); base
//!   is not pulled in.
//!
//! ## Dependencies (invariant)
//!
//! - **0 workspace crate dependencies — not even base** (ADR-0266 「Rejected alternatives」 second, ADR-0268). Gate ① is that wall; the
//!   compiler does not enforce the leaf property.
//! - **Production third parties: `windows` · `junction` (both Windows targets only), the `tracing` facade, `raw-window-handle`** (TRD
//!   1-3 §0 ①, TRD S21-slot-docking 6.3 §8 · B10 · 7 §5-1 · ADR-0336); dev dependencies are outside this rule. `windows` turns on the
//!   union of binding features and is not split per cargo feature (ADR-0275 decision 13). `junction` only for `fs`'s junction
//!   creation; it is an OS crate (inside gate ⑤'s list — its `windows-sys` is not a direct dependency, so the gate does not count
//!   it). `tracing` only for rule 4 paths.
//!   `raw-window-handle` only as the argument trait (`HasWindowHandle`); a no-dependency leaf, not an OS
//!   crate (outside gate ⑤'s list).
//! - ★**No async runtime (`tokio` etc.)**★ — sync crates (agent, now) call this layer too, and a runtime here would silently spread to
//!   them; gate ⑦ turns red.
//!
//! ## Isolation gates (invariant) — ①② are separate axes (③ is in the `testing` paragraph above; ④–⑥ and ⑦ are at the bottom)
//!
//! **① Workspace dependency cap**: `cargo tree -p engram-dashboard-platform --depth 1 --prefix none -e normal,dev,build --target all
//!   --all-features | rg "^engram-dashboard" | sort -u` → **exactly 1 line = itself**.
//! ★Do not replace it with a manifest-text grep★ (rename, `[dependencies.<name>]` form, `[build-dependencies]`, inactive targets,
//!   `optional` all slip out) and do not reduce the flags (same basis as the base · command ·
//! messaging · transport cap gates). ★Two holes★ (members are identified by the name prefix `engram-dashboard`, ADR-0151 「rename trap」):
//!   (a) a workspace member without that prefix passes; (b) if this crate dropped the
//! prefix, this gate would go red but the **other** crates' cap gates and the messaging name regex would stop seeing edges to this crate —
//!   the forbidden base → platform edge (ADR-0175 tenancy condition ②, restated by
//! ADR-0266 decision 8) would slip out.
//!
//! **② Tauri import 0** (ADR-0003): `rg "^\s*use tauri" crates/engram-dashboard-platform/src/` → **0 lines**; first check `test -d
//!   crates/engram-dashboard-platform/src` (a missing path also matches 0). ★① does not cover
//! this★: it counts only workspace members, so third-party `tauri` passes. ADR-0003's invariant is about **which binary it links into**,
//!   and the OS layer sits under every binary, even the headless daemon. The pattern is
//! anchored to the import line so this header does not catch itself.
//!
//! **④ OS branches only inside this crate** (TRD 1-3 §4-4; ADR-0278 records the shape of ④–⑥ and rejected alternatives): the list of `.rs`
//!   files outside this crate that use OS `cfg` predicates must be **exactly** a fixed list.
//! - **Where** — the regex (`os_cfg_re`), file finder (`os_cfg_files`) and list (heredoc) live only in the `platform gate 4` step of
//!   `.github/workflows/ci.yml`, and the local substitute (`.claude/skill-bindings/qa-verify.md`
// hangul-ok-begin
//!   「gates 잡」) runs that step as is — those three are not copied here or into other docs. Target = the `.rs` files of `crates` ·
// hangul-ok-end
//!   `src-tauri` minus this crate (including `src-tauri/tests/` and `build.rs` — wider than TRD §4-4's `src-tauri/src`; measuring showed
//!   the same list). Check that the three paths exist first — rg dies without a reason otherwise.
//! - **matcher = one PCRE2 regex (`rg -P -U -l`; no PCRE2 in rg = the gate FAILs)** built from lexical units via `(?(DEFINE)…)`. Outside a
//!   predicate it skips comments, strings (byte/C prefixes, raw strings), char literals with `(*SKIP)(*FAIL)`;
//!   ★do not remove the branch accepting a newline after `\` inside a string (line continuation)★ — without it the quote pairing flips and
//!   a real `#[cfg(windows)]` is missed (measured). It reads `cfg!`/`cfg`/`cfg_attr` predicates by
//!   **matching parentheses** recursively and bites an OS word outside comments/strings at any depth: bare `windows`/`unix`, or `=` after
//!   `target_os`/`target_family`/`target_env`/`target_vendor`. Not caught: `feature = "windows"`, OS words in
//!   comments inside a predicate, `windows_subsystem = "windows"`. rg on Windows prints `\` paths; they are normalized to `/` and sorted
//!   before comparing with the list.
//! - **The list holds** files with only test branches (`#[cfg(test)]` modules, `#[cfg(all(test, …))]` items), `tests/`, test harness bins,
//!   `examples/`. **Two-way decision** — red if a file outside the list is caught (move a production
//!   branch here; list a test/harness) and red if a listed file is not caught (the branch moved — delist it, keeping the list from going stale).
//! - **Pair** — the same matcher must bite 1+ files inside this crate; otherwise a broken regex would read as "both lists empty".
//! - ★**Limits — a rough gate for common shapes, not a guarantee** (decided: string level is enough)★: only new files outside the list are
//!   caught (a production branch added inside a listed file is review's job); runtime OS splits without
//!   `cfg` (home: `env`) are unseen; lexical analysis is an approximation (nested block comments hide a later `cfg`; a predicate where
//!   `"`/`#` sticks to a name cannot be read); calls to functions named `cfg` are false positives that only list
//!   the file. Unbitten (measured): `cfg_select!`, `cfg_aliases`, a comment between `cfg` and `(`, `cfg!{…}`, `cfg![…]`; `cfg_if!` is
//!   bitten by its inner `#[cfg(…)]`. Nesting 19+ levels hits the PCRE2 limit — rg errors and the gate FAILs (not a
//!   silent pass; measured with rg 14.1.1 · PCRE2 10.45 — the limit may differ per rg build). Do not grow the regex.
//!
//! **⑤ Production OS crate dependencies only in this crate**: among workspace members, only this crate may have an OS crate **directly**
//!   as a production dependency (normal, build — any target OS).
//! - **Where it is** — the OS crate list (`os_dep_re`) · the decision function (`os_dep_gate`) are only in the `platform gate 5` step of
// hangul-ok-begin
//!   `.github/workflows/ci.yml`, and the local substitute (qa-verify 「gates 잡」) runs that step as is. The list is a hand-written explicit
// hangul-ok-end
//!   list — not copied here.
//! - **What it reads** = the roots of `cargo tree --workspace --no-dedupe --depth 1 --prefix depth -e normal,build --target all
//!   --all-features` and their direct dependencies. Members are not hand-written, so new members are
//!   seen; the resolved graph covers rename (printed as the package name), `[target.'cfg(…)'.dependencies]`, `[build-dependencies]`,
//!   `optional` (measured by fault injection). ★Do not remove `--no-dedupe`★ — a member
//!   already printed as another member's dependency folds into `(*)` at its root and shows none of its own (measured — down to this
//!   crate's `windows`).
//! - **dev dependencies are not counted** (tests, `examples/`; agent's `windows` dev = `examples/spike*.rs`).
//! - **Pair** — this crate must have 1+ (`windows` · `junction` now); a broken list regex or `cargo tree` output shape (including dropping
//!   `--no-dedupe`) would read as "0 violations".
//! - ★**Limits**★ — only direct dependencies: a third party wrapping OS APIs (outside the list) or an unlisted OS crate (`-sys` siblings,
//!   `windows-core`) taken in by another member is unseen; widen the list when such a crate appears.
//!
//! **⑥ OS modules of `std::os` only inside this crate** (a rough string gate): the `.rs` files outside this crate that use
//!   `std::os::windows`/`std::os::unix` must be **exactly** a fixed list. `std`'s general APIs
//! (`std::fs`, `std::process`) adapt on their own and are not targets.
//! - **Where** — the regex (`std_os_re`), decision function (`std_os_gate`: target range, pair, comparison) and
// hangul-ok-begin
//!   the list (heredoc) are only in the `platform gate 6` step of `ci.yml`, and the local substitute (qa-verify 「gates 잡」) runs that
// hangul-ok-end
//!   step as is. Target paths, path check, separator normalization, list kinds, two-way decision and pair are the same as ④.
//! - **matcher** = `std::os::windows`/`std::os::unix` after "not yet met `//`" from the line head (`^(?:[^/]|/[^/])*?`), whitespace
//!   allowed around `::` (rg's default engine — no PCRE2). `//`, `///`, `//!` comments are not
//!   caught; `use` lines, full-path calls (`::std::os::unix::…::f(…)`) and the whitespace form (macro bodies rustfmt does not touch) are.
//! - ★**Limits**★: ① new production uses inside a listed file are review's job (as ④) ② unseen: aliases and multi-item groups (`use
//!   std::os as o;`, `use std::os::{unix::…, windows::…};`, `use std::{io, os::windows::…};` —
//!   a single-item group is written out by rustfmt and caught) and other OS modules (`std::os::linux`/`macos`/`fd`) ③ the path inside
//!   block comments/strings is caught (safe-side false positive); if a string before the path on the
//!   same line has `//`, it is missed.
//!
//! **⑦ 0 async runtime imports into sync crates** (the wall of the third Dependencies item, ADR-0282): in the normal graph (`cargo tree
//!   --locked -e normal --prefix none --target all`) count lines matching
//! `^(tokio|mio|tokio-tungstenite|futures-util) `.
//! - **Where** — the canonical command and decision are the `platform gate 7` step of `.github/workflows/ci.yml`; this header writes the
//!   pattern too, so fix both together.
//! - **Two targets** — ① agent → 0 lines: a sync crate carrying this crate, base and command as production dependencies; measuring agent
//!   (not this crate alone) also catches a runtime agent pulls in by another path
//!   ② cli → 0 lines: the sync CLI (`engram-dashboard-cli`) left the daemon package to avoid tokio (ADR-0273); it does not call this crate
//!   directly but shares the function and decision, so it also covers the CLI's own
//!   third-party dependencies (serde_json today).
//! - **Pair** — transport → at least 1 line (its tokio dependency is unconditional); a broken pattern or changed `cargo tree` form would
//!   pass both 0 expectations blind.
//! - ★**Do not remove the `^…␣` anchor (trailing space included)**★: with `--prefix none` the crate name starts the line; without it the
//!   agent side bites `termios` (via portable-pty) and the 0 expectation breaks (measured).
//! - ★**Limits**★ — only the normal graph (dev/build runtimes not counted); whether to add a new sync crate is decided when it appears;
//!   async runtimes outside the name list (`async-std`, `smol`) are unseen.
//!
//! **Not done — again when porting to macOS** (decided): `cargo check` for other OS targets (the portability axis — catches unconditional
//!   OS code; CI is Windows only), clippy `disallowed-*`, Dylint custom lints,
//! cargo-deny; ④–⑥ do not replace these. Research = `docs/research/os-boundary-enforcement-2026-10-06.md`, decision = ADR-0278 decision 4.
//!
//! ★No generated files★: neither serde nor ts-rs, so no `bindings/` directory; do not add this crate to CI's ts-rs sync gate path list.
//!
//! **⑦ 0 async runtime imports into sync crates** (the wall of the third item of 「Dependencies」 above · ADR-0282): in the normal graph
//!   (`cargo tree
//! --locked -e normal --prefix none --target all`), count the lines matching `^(tokio|mio|tokio-tungstenite|futures-util) `.
//! - **Where it is** — the canonical command · decision is the `platform gate 7` step of `.github/workflows/ci.yml`. This header
//!   writes the pattern too, so when the pattern is fixed, fix both together.
//! - **Two targets** — ① agent → 0 lines: a synchronous crate without tokio that carries this crate · base · command
//!   as production dependencies. A runtime in this crate would actually spread there, so the gate measures agent rather
//!   than this crate alone (a runtime agent pulls in by another path is caught too). ② cli → 0 lines: the synchronous
//!   CLI (`engram-dashboard-cli`) left the daemon package so as not to pull tokio in (ADR-0273). It does not call this
//!   crate directly, but it takes the same function and the same decision, so it sits in this gate — on top of what ①
//!   covers it covers the CLI's own direct third-party dependencies (serde_json today).
//! - **Pair** — transport → at least 1 line (its tokio dependency is unconditional). If the pattern breaks or the
//!   `cargo tree` output form changes, the two 0 expectations pass blind.
//! - ★**Do not remove the `^…␣` anchor of the pattern (including the trailing space)**★ — with `--prefix none` the crate name comes at the
//!   very start of the line, so
//!   the anchor is exact. Without it, the agent side bites `termios` (via portable-pty) and the 0 expectation breaks (measured).
//! - ★**Limits**★ — it sees only the normal graph (runtimes of dev · build dependencies are not counted). When a new sync crate outside
//!   the targets
//!   appears, whether to add it to the targets is decided then. Async runtimes outside the name list (`async-std` · `smol` and the like)
//!   are not seen.
//!
//! **Not done — again when porting to macOS** (user decision 2026-10-06): `cargo check` for other OS targets (the portability axis —
//! catches OS-specific code used unconditionally. CI is Windows only, so such code passes now) · clippy
//! `disallowed-*` · Dylint custom lints · cargo-deny. ④–⑥ do not replace these. Research =
//! `docs/research/os-boundary-enforcement-2026-10-06.md` · decision = ADR-0278 decision 4.
//!
//! ★This crate produces no generated files★ — it has neither serde nor ts-rs, so there is no `bindings/` directory. Do not put it into
//! CI's ts-rs sync gate path list.

// ADR-0266, ADR-0275, ADR-0278
// ADR-0278
pub mod env;
pub mod file_holders;
pub mod fs;
pub mod group;
pub mod input;
pub mod process;
pub mod pty;
pub mod shell;
pub mod spawn;
#[cfg(all(test, windows))]
mod test_window;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod window;
