//! engram-dashboard-net — the daemon's **network layer** (ADR-0129 decision 1, slice 1).
//!
//! ★What it owns★: connection lifecycle management **after** socket accept — WS upgrade + Origin allowlist · token handshake ·
//! connection lifetime · single writer per connection · keepalive (active Ping · half-open decision) · out-of-band close signal ·
//! all-connection fan-out registry (`ws`) · the frame port contract with the layer above (`frame_port`) · process lifecycle management
//! (`instance` single-instance guard · `portfile` daemon.json write/read · stale decision — since ADR-0135
//! **the two deal with one and the same file**: the file the guard holds is the connection file the client reads).
//! The per-module responsibility is owned by each file's header.
//! ★All of it sits behind the `server` feature, and **the default is empty**★: if not turned on, what remains is `auth` (a re-export of the protocol
//! handshake frame) alone — the boundary that keeps **consumers that need only the frame shape** from carrying an async runtime (that the default feature has no
//! async runtime is measured by CI `platform gate 7`). ★No consumer uses that combination
//! now★ — the shell takes the frame directly from protocol (TRD stage 3 3-1 U2). The side that uses the server layer
//! turns it on explicitly with `features = ["server"]` (the daemon crate). Which dependencies went behind it, why the default was
//! emptied, and how far the gain extends are **owned by the `[features]` comment in Cargo.toml**
//! (do not recount the list here).
//! ★The accept **loop itself** is not here yet★: `run_accept_loop` stays in the daemon assembly part and serves both accepting and
//! agent-layer assembly — it was planned to come to this crate in slice 3 (the thin assembly binary), but **that slice was put on hold
//! by ADR-0130** (it is not a plan but the target shape when it resumes). So the boundary is drawn as "after socket
//! accept" (accepting itself is still the assembly part's responsibility).
//!
//! ★Do not shorten it to "the only hole" (standing prohibition)★: that abbreviation once hid the fan-out path. How many surfaces
//! there are and in which direction is owned by ADR-0129 decision 1 + its 2026-08-04 note, and the actual traits and the split of implementations are
//! in the `frame_port.rs` header. Do not recount here —
//! counted three times, one entry short three times.
//!
//! ★What it does not know = agent vocabulary — zero exceptions (0-4 done)★: it does not know **even as a type** what is carried in a frame (command · event · output ·
//! messaging). But that does not mean frame decoding is "all the upper layer's" —
//! **only the first auth frame** is parsed by this crate. That is not a vocabulary exception — that frame is not a variant of the command enum
//! but a separate handshake type (`auth::AuthFrame` = a re-export of protocol `handshake::AuthFrame` ·
//! ADR-0310 decision 9 — token auth is network lifecycle management that decides whether to keep the socket alive).
//! Apart from that one, the rule stands as is: interpreting · encoding · dispatching frame meaning is owned by the upper layer (the agent system), and
//! this crate meets it only through the `frame_port` contract.
//!
//! ★The version constant itself stays in protocol★ — it is also the `DaemonInfo` contract of daemon.json, so the shell's `discovery`
//! module reads it through that file. Do not drag it here (it is a separate contract unrelated to auth).
//!
//! ★Why this crate — because of the cycle★: **before** slice 1 the network layer was inside the daemon crate, so
//! the auth type would also have been placed in `daemon`, and because `discovery`·`src-tauri` each build the auth frame,
//! `discovery → daemon` would have been needed, forming a **cycle** with the existing `daemon → discovery` (path helper). The network
//! layer being split into its own crate made `discovery → 이 crate` acyclic, which was the key that solved 0-4 (in fact 0-4
//! newly laid two edges: `discovery → 이 crate` · `engram-dashboard → 이 crate`). The only caller of this crate now
//! is the daemon (the shell does not depend on this crate — CI `shell gate 3`).
//! ★Confirmation recipe — look at the **workspace-local forward closure**★:
//!   `cargo tree -p engram-dashboard-net -e normal --target all --all-features --prefix none`
//!   `| rg "^engram-dashboard" | sort -u`
//!   → itself + `command` + `platform` + `protocol` (`command` comes through `protocol`). The consumer (the daemon) is not in
//!   the list = **this crate does not reach the consumer**. So adding `소비자 → 이 crate` closes
//!   no loop.
//!   ★Do not drop `--target all --all-features`★: looking at only the host target makes **inactive-target** (such as `cfg(unix)`)
//!   dependencies and **optional** dependencies vanish from the output (measured). Printing everything including external crates is hundreds of lines, so only the workspace's
//!   are filtered. `platform` (ADR-0266) · `command` (ADR-0155) are leaves that depend on no workspace crate at all,
//!   so the closure does not grow further below them.
//!   ★Do not use `--invert` (unsound)★: invert lists this crate's **ancestors**, but the condition for a cycle is
//!   `이 crate →* 소비자`, so even in a world with a cycle the invert output is **byte-identical** — that is, that
//!   command cannot tell whether a cycle exists.
//!
//! ★Isolation gates (the compiler catches first, and grep catches the paths that leak through comments · test helpers — same
//! shape as ADR-0110)★. Gates 1·2·4, which scan the source, write the pattern **in parenthesized form** like `_(이름)`·`_(문자클래스)` —
//! because writing it as a bare name makes this header trip the gate on itself (the core tauri gate already stepped on the same trap,
//! 2026-07-13). Gate 3 does not read files but looks at the resolved dependency graph, so it has no such problem at all.
//! When you touch a gate, **re-measure the post-edit non-self-matching property**:
//!   · **Gate 1 — source reference** (scope: `src/`):
//!     `rg "engram_dashboard_(daemon|messaging)" crates/engram-dashboard-net/src/` → **0 lines**
//!   · **Gate 2 — two pieces** (scope: `src/`). 2a pins down by name the five platform symbols this crate uses,
//!     and 2b measures that the liveness helper's original place (the agent runtime — ADR-0175 decision 1 cut it) is not
//!     refilled:
//!     · **2a — platform symbol allowlist (two checks)**:
//!     ① `rg -o --no-filename "engram_dashboard_platform::[A-Za-z0-9_:]+" crates/engram-dashboard-net/src/`
//!     `| sort -u` → **exactly these five lines** (compare names, not count) — two process
//!     liveness helpers under `process::` (`pid_alive_with_start_time` · `current_process_start_time` — only for portfile's stale
//!     decision), and one shared-restricted open and two of its failure classifiers under `fs::` (`open_deny_write` · `is_sharing_violation` ·
//!     `is_access_denied` — all three only for instance's single-instance guard). Counting only the number lets a swapped symbol pass
//!     as long as it is five, and one module import (`use …::fs;`) line covers that whole module — under name comparison both show up as names
//!     outside the list.
//!     ② `rg '^(?:[^/]|/[^/])*?\bengram_dashboard_platform(?:[^:]|:[^:]|::[^A-Za-z0-9_]|:{0,2}$)'`
//!     `crates/engram-dashboard-net/src/` → **0 lines** — calls where `::이름글자` does not follow the crate name.
//!     The regex of ① **misses such calls with 0 matches** — aliases (`… as p`) · braces (`…::{fs::x}`) · glob (`…::*`) ·
//!     paths cut at the end of a line. The prefix `(?:[^/]|/[^/])*?` = 「이 줄에서 아직 `//` 를 안 만났다」 (this line has not yet met a line comment) — it does not bite
//!     the quotations in this header.
//!     ★So when calling, use full paths or individual `use`★ — a grouped `use` (`…::process::{a, b}`) is printed by ① as the single line
//!     `…::process::`, which is a name outside the list, and an alias is bitten by ②.
//!     ★Limit★: if the dependency name is changed in the manifest (a different name given with `package = "engram-dashboard-platform"`),
//!     this crate name does not appear in the source and ①② are both blind. ★Gate 3 does not back that up either★ — the resolved
//!     graph prints the same package under the changed name and stays green with the allowed edge as is. Then nothing guards what is
//!     called through this edge (the symbol range) — it is left to review.
//!     · **2b — no re-entry of the agent runtime**:
//!     `rg "engram_dashboard_(a)gent" crates/engram-dashboard-net/src/` → **0 lines**
//!     (this crate does not know the agent runtime even as a dependency)
//!     ★2b bites the **bare crate name** — do not attach a symbol tail★: a 0-expectation gate has no
//!     reason to require a symbol shape, and attaching a tail means it does not bite when what follows `::` is outside the char class, so **the two most common forms are missed** —
//!     brace expansion (`use …::{platform::pid_alive, …}`) and aliases (`use … as ag;`) slipped through as is (measured).
//!     ★In exchange, the self-match defense is carried **explicitly**★: biting only the name makes this header trip the gate on itself, so
//!     it uses the same `_(첫 글자)` parenthesized form as gate 4. The old form with a tail escaped self-match
//!     only thanks to the **coincidence** of `[` following `::`, a defense that died the moment one example symbol was written here — so the coincidence was
//!     turned into a rule. 2a ① still stands on that coincidence (the tail is a char class, so it is needed — it is a gate that extracts symbol names) —
//!     if you write a symbol outside the list as a full path in this header, ① bites it as a name outside the list. 2a ② skips
//!     comments not by coincidence but by the prefix `(?:[^/]|/[^/])*?`.
//!     ★Do not inflate what 2b is worth keeping★: the living wall is gate 3 — net cannot even call an agent symbol
//!     without declaring the dependency, so that side goes red first. What 2b takes on is **vocabulary leaking first through comments · test helpers**
//!     and the gate running true to its name (`0줄`).
//!     ★Even so, do not **delete** 2b★: detach only it and 2a's five-name expectation is left alone, and since 2a sees only platform
//!     names, it passes even if agent vocabulary newly sticks next to it. It is a wall only when standing as a pair.
//!     ★Do not put the expectation on a **file name**★: "only `portfile.rs`" stays true even if an agent-vocabulary
//!     import is newly put **inside** that file, so the gate passes. The invariant is per **symbol**, not per file
//!     (ADR-0129 — isolation is per type, not per crate).
//!   · **Gate 3 — direct workspace dependency ceiling** (scope: resolved dependency graph):
//!     `cargo tree -p engram-dashboard-net --depth 1 --prefix none -e normal,dev,build --target all`
//!     `--all-features | rg "^engram-dashboard" | sort -u`
//!     → **exactly 3 lines** = itself · `engram-dashboard-platform` · `engram-dashboard-protocol`.
//!     ★Do not try to reduce it★ — the `platform` slot is the edge that takes portfile's liveness helper and instance's shared-restricted open
//!     (gate 2a), and the `protocol` slot is the `DaemonInfo` file-format contract · the handshake frame
//!     re-export (the dependency comment in Cargo.toml).
//!     ★What this gate closes (exactly this much)★: **declared direct workspace dependencies** show up regardless of
//!     manifest syntax — rename · `[dependencies.<이름>]` table form · indented declaration · `[build-dependencies]` ·
//!     inactive target (`cfg(unix)`) · `optional`, all 6 forms were **measured by inject → observe → revert** and confirmed all caught.
//!     ★What it does **not** close★: it does not see which symbols are used **inside** the allowed edges (protocol·platform) —
//!     the platform side is bound by gate 2, and the protocol side is pinned by gate 4 to **only two names**. That is, there is still no allowlist for protocol symbols
//!     **in general** (a known gap — the ★known gap★ section inside the 2026-08-05 note of ADR-0129 §영향/불변식 [impact / invariants], an intentional
//!     reservation. That ADR has no label "Note A"). Do not grow gate 4 into
//!     that general allowlist — that is a separate decision (outside the scope of 0-4).
//!     ★Do not grep the manifest **text**★: five of the 6 forms above slip past a text grep with normal Cargo syntax,
//!     and if the source does not call the name, gates 1·2 pass too. `cargo tree` prints the **resolved package
//!     identity**, so everything shows up under the real name. Reducing the flags leaks forms to that extent
//!     (`-e build`↔build-deps · `--target all`↔inactive target · `--all-features`↔optional), so do not reduce them.
//!     Before raising the expected value, read the dependency ceiling rule in Cargo.toml first.
//!   · **Gate 4 — no re-entry of auth vocabulary** (scope: `src/`, added by 0-4):
//!     `rg "(A)gentCommand|(P)ROTOCOL_VERSION" crates/engram-dashboard-net/src/` → **0 lines**
//!     ★What it guards★: it prevents **reopening** of the last vocabulary hole that 0-4 closed (the handshake borrowing the upper layer's command enum and
//!     version constant). The frame shape is owned by protocol `handshake::AuthFrame`
//!     (`auth` is only a re-export) and the expected version is injected into `ws::handle_connection`, so these two names reappearing in `src/`
//!     = going back.
//!     ★Comments · tests are in scope too (intended)★: the compiler also catches re-entry inside `#[cfg(test)]` but cannot catch comments, and
//!     when a test starts comparing shapes against the upper layer's types, that is the first step of re-entry.
//!     ★It bites partial strings too (intended — do not narrow)★: because no word boundary is applied, **local names** such as `TEST_(P)ROTOCOL_VERSION`
//!     are also caught (measured 2026-08-05 — this crate's test constant originally had that name). Wide is
//!     right: the invariant is that this crate keeps not even something resembling a protocol version, and when caught, renaming is the
//!     response (`TEST_WIRE_VERSION` in `ws.rs`). Narrowing with `\b` loses that discipline.
//!   · **Gate 5 — each of two feature combinations compiles** (scope: `-p`, added by the `default = []` switch):
//!     `cargo test -p engram-dashboard-net` → **PASS only if it succeeds** (zero features = `auth` re-export alone — that combination's
//!     tests number 0. The golden is in protocol `handshake`)
//!     `cargo test -p engram-dashboard-net --all-features` → **PASS only if it succeeds** (the `server` layer)
//!     ★The decision method differs from the four above★: 1·4 read by match or no match, 2·3 by line count, but this reads by **success or failure**.
//!     ★Why two lines★: the default is empty, so the bare command compiles only zero features, and the modules under `#[cfg(feature = "server")]`
//!     **drop out of the compile target** (hidden code raises no errors) — the bare command only measures compilation with **0** tests,
//!     and `--all-features` runs the server layer tests. In the opposite direction, workspace-scoped commands
//!     see only the ON side because the daemon turns on `server` — within `-p` scope the only command that compiles the ON side is the second line.
//!     Each line covers the side the other cannot see, so do not reduce them to one line.
//!     ★Why `--all-features` and not `--features server`★: gate 3 already uses that flag, so the net
//!     gates' ON-side notation is unified to one. With `server` as the only feature now, the two forms are the same set.
//!     ★Why `test` and not `build`★: it compiles `#[cfg(test)]` and the dev-dependency paths too (in the zero-feature
//!     combination that path is empty now — the golden is in protocol).
//!     ★What this gate does **not** close★: writing a new dependency without `optional` and silently
//!     loading it onto zero-feature consumers does not break compilation, so it is not caught here — the rationale · alternative gate is the Cargo.toml
//!     `[features]` comment.
//!
//! ★Standalone verification (ADR-0012)★: `cargo test -p engram-dashboard-net --all-features` runs with this crate alone —
//! the WS server · frame port tests run on real sockets (127.0.0.1:0) or synthetic frame sequences and do not use
//! the real agent system.

pub mod auth;
#[cfg(feature = "server")]
pub mod frame_port;
#[cfg(feature = "server")]
pub mod instance;
#[cfg(feature = "server")]
pub mod portfile;
#[cfg(feature = "server")]
pub mod ws;
