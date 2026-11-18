//! # engram-dashboard-platform — OS layer, leaf crate (0 workspace dependencies · 0 domain knowledge)
//!
//! Gathers **production** code that differs per OS into this one crate (ADR-0266 decision 1). Whether to take something in here is decided
//! by one thing, whether it depends on the OS — not by the number of consumers (decision 2). Callers call only this crate's functions and handle types,
//! and OS branches (`#[cfg(windows)]` · `cfg!(windows)` · `#[cfg(unix)]` and the like) live only inside this crate. OS branches of tests · test
//! harness bins · `examples/` stay there — they do not go into the production build (ADR-0266 「Rationale」).
//! The machine gates of this invariant are ④ (`cfg` location) · ⑤ (OS crate dependencies) · ⑥ (`std::os` paths) below. No other crate has a `cfg`
//! production OS branch (ADR-0291 closed the time-limited exception of ADR-0275 decision 15). Spots that use OS rules at runtime without `cfg`
//! are not counted here (the limit of ④).
//!
//! There are ten residents today — [`process`](PID liveness · process start time and its three-way verdict ·
//! the process table · child PID enumeration · the identity list under one root · ending one process tree —
//! Windows `taskkill` · whether one identity still runs and ending it only if it does — one handle) · [`group`](the strong owner of a process group · weak handles · held members · the
//! join-notification port — Windows Job Object) · [`file_holders`](the processes that have this file open
//! now — Restart Manager) · [`fs`](opening while denying others' writes · classifying that failure — sharing
//! violation · access denied · the verdict and retry for a failure caused by someone holding it briefly ·
//! folder sync) · [`spawn`](spawning without a window · spawning as a tree root with a kill handle for that
//! tree — Windows = spawn suspended, put in the group, then resume · the "program not found" verdict of a
//! failed shell · spawning outside this process's Job — Windows = WMI) · [`shell`](the interactive default
//! shell · wrapping a CLI in a console shell) · [`env`](home directory · executable file name · the path of
//! an executable next to the current one · environment variable name comparison) · [`window`](window
//! identity · placement and z-order in one call · restoring placement · the topmost window under the
//! cursor — the tab-drop end verdict · window diagnostic facts · whether the foreground window belongs to
//! this process · whether that window is the foreground window · title bar chrome shape · the snap layout
//! overlay — behind feature `snap-overlay`) · [`input`](cursor position · primary button · Esc state) ·
//! [`pty`](whether closing a pseudo-terminal waits for its output to be read). The module headers and
//! public function docs are the source of truth for those responsibilities.
//!
//! Functions that take a window accept `raw-window-handle`'s `HasWindowHandle`, and the variant matching
//! happens here too (TRD S21-slot-docking revision 6.3 §8 · B10 · revision 7 §5-1).
//!
//! ★`testing` (real-process test helpers) · `group::GroupRef::gone` (a handle whose owner never existed) ·
//! `spawn::wmi_create_raw` (the raw call of WMI launching — for diagnostic tests) are behind the cargo feature `test-support`★
//! (TRD 1-3 §3-8 · the name is the same as base's ADR-0275 decision 5) — the declaration is `#[cfg(any(test, feature = "test-support"))]`, so only this crate's own tests and consumer tests that turned on that feature as a dev dependency
//! see them. That the feature is not in the production dependency graph of the daemon · shell · CLI is measured by CI gate ③ (a 1-or-more-lines expectation paired with the
//! 0-lines expectation) — it is a test feature leak gate separate from isolation gates ①② below, and the canonical source of the commands · expected values is `.github/workflows/ci.yml`.
//!
//! ★`window::snap` (snap layout overlay) is behind the cargo feature `snap-overlay` · off by default★ (TRD S26-window-chrome §8-4 · user
//! decision Q1 2026-10-24) — the only thing that turns it on is the shell feature `snap-overlay`, and CI measures that the feature is not in the shell production graph (a 1-or-more-lines
//! expectation paired with the 0-lines expectation) and that a build with it on compiles. This crate has only these two cargo features — it widens
//! 「one `test-support`」 of ADR-0275 decision 13. Both only open and close code and do not turn on `windows` bindings (「Dependencies」 below).
//!
//! ## Rules for taking things in
//!
//! 1. **0 domain knowledge** (ADR-0266 decision 3). 「Find the home directory」 belongs here, but backend knowledge such as `.claude` paths · subfolders of the codex home
//!    · lock file names stays with the caller. So public functions do not take domain types but
//!    raw values (program · arguments · PID · path), and domain values (constants like exit codes) are taken as arguments too
//!    (ADR-0275 decision 12).
//! 2. **Interface = functions + handle types, compile-time branching** (ADR-0266 decision 5). No trait object that picks the OS.
//!    If a fake for tests is needed, **the caller** keeps a small trait — so this crate's public
//!    API has no test seam traits.
//! 3. **Branches outside Windows are placeholders** (ADR-0266 decision 7). This crate does not build new implementations for other OSes —
//!    the branches that existed when moved over (an empty answer, or a one-liner like `bash` · `HOME`) are merely seen gathered in one place. ★Those branches
//!    split with `#[cfg]` get no compile verification here either★ — development · CI are Windows only (ADR-0230 status).
//!    Branches split with `cfg!(windows)` (`env::env_key_eq` · the three failure classifications of `fs`) get type checking on both sides, but they have
//!    never been run on any other OS.
//! 4. **Logs are printed only on paths with no way to return a result (`Drop`)** (TRD 1-3 §3-7 —
//!    `docs/process/S21-crate-boundaries/trd-1-3-platform-crate.md`). Other failures are returned as values and the caller
//!    prints them in its own wording — domain words are not brought into this crate's logs (rule 1). When printing,
//!    the `tracing` facade is called directly (ADR-0266 decision 9 · ADR-0268) — base is not pulled in for logging.
//!
//! ## Dependencies (invariant)
//!
//! - **0 workspace crate dependencies — not even base** (ADR-0266 「Rejected alternatives」, the second · ADR-0268). Gate
//!   ① below is that wall — the compiler does not enforce the leaf property.
//! - **The only third parties among production dependencies are `windows` (Windows targets only) · the `tracing` facade · `raw-window-handle`** (TRD 1-3
//!   §0 ① · TRD S21-slot-docking edition 6.3 §8 · B10 · edition 7 §5-1) — test-only dev dependencies are outside this rule. `windows` turns on the union of the binding
//!   features used by that one dependency and is not split into per-feature cargo features (ADR-0275 decision 13). `tracing` is used only for printing on the
//!   paths of rule 4. `raw-window-handle` serves only as the argument trait (`HasWindowHandle`) of functions that take a window — it is a leaf crate
//!   with no dependencies and not an OS crate (outside the gate ⑤ list).
//! - ★**No async runtime (`tokio` etc.) is taken in**★ — sync crates also call the OS layer (agent, now). A runtime taken in here
//!   would silently spread to those sync consumers, and gate ⑦ below turns that red.
//!
//! ## Isolation gates (invariant) — ①② are each a different axis (the test feature leak gate ③ is in the `testing` paragraph above · the invariant
//! gates ④–⑥ and the async import gate ⑦ are at the very bottom)
//!
//! **① Workspace dependency cap** (the wall of the first item of 「Dependencies」 above):
//! `cargo tree -p engram-dashboard-platform --depth 1 --prefix none -e normal,dev,build --target all`
//! `--all-features | rg "^engram-dashboard" | sort -u` → **exactly 1 line = itself**.
//! Third parties have a different name prefix and are not counted. ★Do not replace this with a grep over manifest text★ — rename ·
//! the `[dependencies.<name>]` table form · `[build-dependencies]` · inactive targets · `optional` all slip out as normal Cargo
//! syntax. Do not reduce the flags either (the same basis as the base · command · messaging · transport cap gates).
//! ★Two holes remain★ — because members are identified by the **name prefix** `engram-dashboard` (ADR-0151 「rename trap」).
//! (a) Depending on a workspace member without that prefix passes this gate straight through. (b) If this crate dropped that prefix,
//! this gate would turn red (`-p` dies or its own line loses the prefix), but the cap gates of the **other** crates
//! (base · messaging · command · transport) and the messaging name regex would no longer see edges going to this crate —
//! the forbidden base → platform edge (ADR-0175 tenancy condition ② — ADR-0266 decision 8 restates it) would slip out
//! as is.
//!
//! **② Tauri import 0**(ADR-0003):
//! `rg "^\s*use tauri" crates/engram-dashboard-platform/src/` → **0 lines**. Before running it,
//! check the path first with `test -d crates/engram-dashboard-platform/src` — even if the path is missing, rg matches 0 and it
//! reads as a pass.
//! ★Do not read ① as covering this axis★ — ① counts only workspace members, so the third-party `tauri` passes
//! straight through. The axis ADR-0003's invariant is about is not the crate name but **which binary it links into**, and
//! the OS layer is the spot laid at the bottom of every binary, even the headless daemon with no window or webview. The pattern is anchored to the import
//! line so this header does not catch itself.
//!
//! **④ OS branches only inside this crate** (the invariant at the very top · TRD 1-3 §4-4 — the gate shape was left by ADR-0266 「Impact」 to the main
//! session's judgment, and ADR-0278 records the shape of ④–⑥ and the rejected alternatives): the list of `.rs` files outside this crate that use OS `cfg` predicates
//! must be **exactly** the same as a fixed list.
//! - **Where it is** — the regex (`os_cfg_re`) · file-finding function (`os_cfg_files`) · list (heredoc) are only in
//!   the `platform gate 4` step of `.github/workflows/ci.yml`, and the local substitute (`.claude/skill-bindings/qa-verify.md`
// hangul-ok-begin
//!   「gates 잡」) runs that step as is — those three are not copied here or into other docs. Target = the `.rs` files of `crates` ·
// hangul-ok-end
//!   `src-tauri` minus this crate (including `src-tauri/tests/` · `build.rs` — wider than `src-tauri/src` of TRD §4-4,
//!   and measuring showed the list was the same). Before running, check that the three paths (`crates` · `src-tauri` · this crate's `src`) exist
//!   — without them rg dies without a reason.
//! - **matcher = one regex in PCRE2 syntax (`rg -P -U -l` — if rg has no PCRE2, the gate FAILs for that reason)**.
//!   It defines lexical units with `(?(DEFINE)…)` (comments · strings — including byte · C prefixes · raw strings `[bc]?r#*"…"#*` · names · any other
//!   single character) and then:
//!   - **Outside** a predicate, it skips comments · strings · raw strings · char literals with `(*SKIP)(*FAIL)` so it does not bite the `cfg(` inside them.
//!     ★Do not remove the branch that accepts a newline after `\` inside a string (line continuation)★ — without it, the quote pairing after it flips
//!     and a real `#[cfg(windows)]` is missed (measured in discovery).
//!   - It reads the predicates of `cfg!(…)` · `cfg(…)` · `cfg_attr(…)` by lexical units, **matching parentheses** (recursively), and bites if an OS predicate word
//!     is **outside comments · strings** at any depth inside — a bare `windows` · `unix`, or `=` coming after `target_os` ·
//!     `target_family` · `target_env` · `target_vendor` with only whitespace · newlines · comments in between.
//!   - So `feature = "windows"` · OS words in comments inside a predicate · the word outside the parentheses in `cfg!(test) && windows()` · the attribute value
//!     `windows_subsystem = "windows"` are not caught, while predicates nested any number of levels · multi-line attributes · strings with `[` `;` `)` in the value
//!     · raw strings · `target_os /* … */ =` · `target_os` and `=` split across lines are caught.
//!   - rg on Windows prints paths with `\`, so they are normalized to `/`, sorted, and compared with the list.
//! - **The kinds that go in the list** = files with only test branches (under `#[cfg(test)]` modules · `#[cfg(all(test, …))]` items) ·
//!   `tests/` · test harness bins · `examples/`.
//! - **The decision is two-way** — it turns red if a file outside the list is caught (move it into this crate if it is a production branch; add it to the list
//!   if it is a test · harness), and it also turns red if a file in the list is not caught (the branch was removed or moved — take it out of the list). Making the latter
//!   a failure keeps the list from going stale.
//! - **Pair** — the same matcher must bite 1 or more files inside this crate. On the day the list becomes empty, if the regex
//!   broke (escaping · quotes), the comparison would go blind as 「both empty lists」.
//! - ★**Limits — do not read this as a guarantee**★:
//!   - **It catches only new files outside the list** — adding a production branch inside a file already in the list is not caught
//!     (review's job).
//!   - It does not see spots that split by OS at runtime without `cfg` — things like picking environment variables · environment variable name
//!     comparisons that always fold case, and the home of those rules is this crate's `env`.
//!   - The lexical analysis is a regex approximation — when a shape it does not handle comes (like nested block comments `/* /* */ */`), it can misread what follows and
//!     miss a `cfg`. A shape inside a predicate where `"` · `#` sticks right after a name (without being a literal prefix) cannot be read as a lexical unit,
//!     so the whole predicate is missed — valid Rust `cfg` predicates have no such shape.
//!   - Calls to functions · methods named `cfg` are read as predicates too — if an argument has an OS word, it is a false positive (only that the file goes into the list,
//!     so it errs on the safe side).
//!   - ★**Rare shapes are deliberately not chased — it is a rough gate that blocks only common shapes**★ (user decision 2026-10-06 — string
//!     level is enough). Shapes it cannot bite (measured): `cfg_select!` (writes predicates without the `cfg(` spelling) · `cfg_aliases` (the alias `#[cfg(win)]` a build
//!     script made — the side defining the alias has no `cfg(` spelling either) · a comment between `cfg` and `(`
//!     (`cfg /* … */ (windows)`) · `cfg!{…}` · `cfg![…]`. `cfg_if!` is bitten by the `#[cfg(…)]` spelling inside. Predicates nested 19 or more levels
//!     hit the PCRE2 limit, rg ends with an error, and the gate FAILs — it is not a silent pass (measured with local rg
//!     14.1.1 · PCRE2 10.45 — the limit may differ per rg build). The regex is not grown to catch these shapes.
//!
//! **⑤ Production OS crate dependencies only in this crate** (user decision 2026-10-06 — the wall on the other members' side of the second item of 「Dependencies」 above):
//! Among the workspace members, the one that has an OS crate **directly** as a **production dependency** (normal · build — regardless of the target OS) must be this crate
//! alone.
//! - **Where it is** — the OS crate list (`os_dep_re`) · the decision function (`os_dep_gate`) are only in the `platform gate 5` step of
// hangul-ok-begin
//!   `.github/workflows/ci.yml`, and the local substitute (qa-verify 「gates 잡」) runs that step as is. The list is a hand-written explicit
// hangul-ok-end
//!   list (cargo-deny style) — not copied here.
//! - **What it reads** = the roots (members) of `cargo tree --workspace --no-dedupe --depth 1 --prefix depth -e normal,build --target all
//!   --all-features` and the dependencies right below them. Members are not written by hand, so new members are visible too, and since it is the resolved
//!   graph it sees rename (printed as the package name) · `[target.'cfg(…)'.dependencies]` · `[build-dependencies]` ·
//!   `optional` (target-conditional dependencies with a rename and build dependencies were measured by fault injection).
//!   ★Do not remove `--no-dedupe`★ — without it, a member already printed as another member's dependency is folded into `(*)` at its root spot and
//!   none of its own dependencies show (measured — down to this crate's `windows`).
//! - **dev dependencies are not counted** — they belong to tests · `examples/` (agent's `windows` dev = `examples/spike*.rs`).
//! - **Pair** — this crate must have 1 or more (`windows` now). If the list regex or the shape of the `cargo tree` output
//!   breaks (including removing `--no-dedupe`), the decision goes blind as 「0 violations」.
//! - ★**Limits**★ — it sees only direct dependencies: if another member takes in a third party wrapping OS APIs (outside the list) or directly takes in an OS
//!   crate not in the list (`-sys` siblings · `windows-core` and the like), it cannot see it. Whether to widen the list is decided when such a crate comes
//!   in.
//!
//! **⑥ OS modules of `std::os` only inside this crate** (user decision 2026-10-06 — a rough string gate): the list of `.rs` files outside this
//! crate that use `std::os::windows` · `std::os::unix` must be **exactly** the same as a fixed list.
//! `std`'s general APIs (`std::fs` · `std::process` and the like) adapt to the OS on their own, so they are not targets.
//! - **Where it is** — the regex (`std_os_re`) · the decision function (`std_os_gate` — target range · pair · comparison, all of it) ·
// hangul-ok-begin
//!   the list (heredoc) are only in the `platform gate 6` step of `ci.yml`, and the local substitute (qa-verify 「gates 잡」) runs that
// hangul-ok-end
//!   step as is. The target paths · path check ·
//!   path separator normalization · the kinds that go in the list (files with only test branches · `tests/` · test harness bins · `examples/`) ·
//!   two-way decision · pair (1 or more files inside this crate) are the same as ④.
//! - **matcher** = `std::os::windows` · `std::os::unix` after 「not yet met `//`」 from the line head (`^(?:[^/]|/[^/])*?`)
//!   (whitespace allowed around `::` between segments · rg's default engine — no PCRE2 needed). `//` · `///` · `//!`
//!   comments are not caught, while `use` lines and full-path calls (including `::std::os::unix::…::f(…)`) · the whitespace form (left in macro bodies rustfmt does not
//!   touch, etc.) are caught.
//! - ★**Limits**★:
//!   - It cannot catch new production uses inside a file already in the list (same as ④ — review's job).
//!   - It does not see aliases · multi-item groups (`use std::os as o;` · `use std::os::{unix::…, windows::…};` ·
//!     `use std::{io, os::windows::…};` — a single-item group is written out by rustfmt and caught) or other OS
//!     modules (`std::os::linux` · `std::os::macos` · `std::os::fd` and the like).
//!   - That path inside block comments · strings is caught (a false positive — the safe side). If a string before that path on the same line has `//`,
//!     it is missed.
//!
//! **⑦ 0 async runtime imports into sync crates** (the wall of the third item of 「Dependencies」 above · ADR-0282): in the normal graph (`cargo tree
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
//! - ★**Do not remove the `^…␣` anchor of the pattern (including the trailing space)**★ — with `--prefix none` the crate name comes at the very start of the line, so
//!   the anchor is exact. Without it, the agent side bites `termios` (via portable-pty) and the 0 expectation breaks (measured).
//! - ★**Limits**★ — it sees only the normal graph (runtimes of dev · build dependencies are not counted). When a new sync crate outside the targets
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

// ADR-0266
// ADR-0275
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
