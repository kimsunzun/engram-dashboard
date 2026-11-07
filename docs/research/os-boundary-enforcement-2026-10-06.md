# Enforcing the Boundary of OS-Dependent Code by Machine — a Rust Peer Survey (2026-10-06)

- **Status:** research done · adversarial review (codex BLOCK) applied · decision made (user 2026-10-06 — ADR-0278 · result = `crates/engram-dashboard-platform/src/lib.rs` header gates ④~⑥ · 「하지 않는 것」) · 2 corrections (2026-10-06 — the `std::os` ban practice in conclusion 3 · `cfg_if!` in §4)
- **Method:** `/research` medium · design-decision mode. Three collection tracks (direct peers = a source-reading worker · dependency unit · lint/cross target = a document-type worker) → main grounding → codex adversarial review (BLOCK — 8 items) → body corrected.
- **Confidence legend:** certain = primary material checked directly + independently confirmed · likely = primary material checked (single) or inferred from reading code · uncertain = based on a summary or memory · unknown = could not confirm.
- **Context:** engram 1-3 U8 set up the 「platform 밖 OS `cfg` 파일 명단 고정」 gate (ci.yml `platform gate 4` — a PCRE2-syntax regex), and the adversarial review pointed out an omission of a rare shape in round 4, reaching the loop cap. The principle = CLAUDE.md 「플랫폼 중립」 · ADR-0230 · ADR-0266.

## Conclusions (first)

1. **The only mature project we found that measures 「`cfg` 가 어디 있나」 (where is cfg) in CI is rustc, and it too is a text scanner** (within what we found — the peer search is not exhaustive · see "Issues · limits" below). rustc tidy `pal` finds `cfg(` and `cfg!(` as strings in the `.rs` files of `library/`, matches the parentheses, flags the text if an OS word appears in it **as a substring**, and allows it through a list of path-substring exceptions (likely — checked against source). It runs as a required gate even though it knowingly misses `cfg_attr` and a `cfg` after a leading doc comment (reading the code — likely). → **Our gate ④ catches more broadly than that, among the items compared** (`cfg_attr` · multiple lines · deep nesting · parentheses inside strings — measured with 44 fault injections in U8). 「업계 최고 선례도 알려진 빈틈을 두고 돈다」 (even the best precedent runs with known gaps) is the ground for being allowed to stop chasing rare shapes.
2. **The other mature projects use, instead of a location check,** ① physical separation (std `sys/` · mio `src/sys/` · zed's per-OS crates pulled in as target-conditional dependencies) ② `cargo check` against other targets (mio · tokio) ③ review · convention (likely). ★**② does not enforce location**★ — a `cfg` on a false branch drops out of that target's compilation entirely, so a Windows-only branch outside platform passes both the Windows check and the Linux check (Rust Reference — conditional compilation). What ② catches is **OS-specific code written without a condition** and code that does not build on another OS.
3. **To block API paths or crates, use resolver-based tools**: clippy `disallowed-methods`/`disallowed-types` (deno enforces with a meta check that each crate has its designated banned entries) · cargo-deny `[bans]` `wrappers`. clippy cannot block the **location** of a `#[cfg]` attribute (likely — we found no such lint and no counter-evidence).
   - ★**Correction (2026-10-06): banning `std::os` is not a practice**★ — among the peers surveyed, **0** block `std::os` paths with this tool. What deno blocks is file-system functions and `std::env::var`, not `std::os`. The conclusion above read like a `std::os` ban practice, and main also overstated it that way at first. The `std::os` in the clippy · Dylint lines of the fit table below also means 「할 수 있다」 (can do), not a peer precedent.
4. **So there are three layers — they do not substitute for each other:** location = gate ④ (+ review) · dependency = let only platform pull in OS API crates (a graph check) · portability = `cargo check` for other OS targets (something like an unconditional `use std::os::windows` — CI is Windows only, so it passes now). The latter two do not exist now (★updated 2026-10-06: the dependency layer was set up as platform header gate ⑤ — what does not exist now is one layer, portability (cross-target check)★).

## Findings by track

### 1. Direct peers — do they enforce the boundary in CI

| Project | Enforced | Means | Scope · allowance | Known gaps | Confidence |
|---|---|---|---|---|---|
| rust-lang/rust | ○ (`library/` only) | tidy `pal` — string scan + parenthesis matching · OS words as substrings | a list of path-substring exceptions (`library/std/src/sys` etc. · a temporary exception 「FIXME: sys 로 옮길 것」) · skipped if the path has `tests`/`benches` · skipped if the `cfg` text has `test` · self-asserts that the parser actually ran | `cfg_attr` · a leading doc comment on the same line · macro branches (the `cfg_select!` kind) · direct use of `std::os::*` · substring false positives | likely (checked against source) · gaps from reading code |
| deno | ○ (system access) | a text meta check that each crate's `clippy.toml` `disallowed-methods` has **designated function-path entries** (file-system functions · `std::env::var` etc.) (`ensureDisallowedMethodsEnforced` in `tools/lint.js` — as of main `3d44d1d82fda`) | `ext/*` · `libs/*` · `runtime` | does not audit `#[allow]` in code · `cfg` location is out of scope | likely (meta check checked against source) |
| mio · tokio | ✕ location / ○ multi-target compile | mio `Makefile` 7 targets `cargo hack check --feature-powerset` · tokio `cross-check` (`cargo check --workspace --target …`) | OS code follows the `src/sys/{unix,windows,…}` convention | no location check | certain (mio checked by main + tokio checked by the reviewer) |
| zed (gpui) | △ structure only | per-OS crates (`gpui_windows` etc.) pulled in only through `[target.'cfg(…)'.dependencies]` | — | `cfg(target_os)` is spread across other crates | uncertain (not checked) |
| rust-analyzer · servo · wezterm · alacritty · helix · nushell · uv · ruff | ✕ | convention · review (alacritty cross-builds for a macOS target) | — | — | uncertain (collector report · not checked) |

- We found no public clippy or dylint lint that restricts the **location** of `cfg` (unknown — code-search rate limit · not exhaustive). Cases of a Dylint pre-expansion lint from two personal repositories were reported, but they are from the same author's line, so we do not generalize.
- The reviewer separately looked for bevy · fuchsia · firefox · chromium, but no pal-style check was confirmed (not a counter-proof — unconfirmed).

### 2. Dependency unit — OS API crates in only one crate

- **cargo-deny `[bans]` `deny = [{ crate = "…", wrappers = ["…"] }]`** — only the crates on the list may depend **directly** on the banned crate, and every other path is rejected (certain — doc source text · re-checked by the reviewer). It applies to the whole checked graph — if third parties such as tokio · mio use `windows-sys` directly, they too must be on the list, and it wobbles with every dependency update (likely — derived from the doc's meaning). We found no real case of using it for OS crate restriction (unknown).
- **A graph script (`cargo tree` / `cargo metadata`)** — ★a bare `cargo tree -i` sees **only the host target** and does not filter nodes down to workspace members★ (Cargo docs · the reviewer's finding). So the script has to do `--target all` and member filtering — engram's existing dependency-ceiling gate uses `--target all` + name-prefix filtering (the gap = prefix identification, a hole noted in CLAUDE.md). `cargo metadata` gives member ids · dependency kinds · target conditions in structured form, so it is sturdier than parsing output (likely — docs). ★**The meaning of the rule differs from `wrappers`**★ — a member-only rule means 「워크스페이스 안에서 platform 만」 (only platform within the workspace), while `wrappers` means 「그래프 전체에서 명단만」 (only the list within the whole graph). For our purpose (the boundary of our own code), the former fits.
- Cargo itself has no dependency-visibility feature (uncertain — based on memory · no counter-evidence found). Bazel/Buck `visibility` is the control case where the build system enforces it.

### 3. API · build unit — clippy · cross target

- **clippy `disallowed-methods` / `disallowed-types`** — specified by path · a single root `clippy.toml` + `#![allow(…)]` in the allowed crate is the realistic layout (no settings inheritance — likely). `disallowed-types` **also catches a single-item `use` declaration** (clippy source — the reviewer's finding). `disallowed-methods` looks at resolved calls, and **giving a trait method path catches calls across all implementations** (it cannot be narrowed to a specific implementing type — a clippy maintainer's answer). On a Windows host, a `std::os::unix::…` path does not resolve, so `allow-invalid = true` is needed (likely). **What it cannot do:** the location of a `#[cfg]` attribute (an attribute is not a macro — likely) · constants such as `std::env::consts::OS` (uncertain). It takes effect only if run in CI as `cargo clippy --all-targets -- -D warnings`.
- **Cross-target `cargo check`** — the scope must be stated explicitly: a bare command looks only at default members · lib/bin targets · default features (Cargo docs). The form to use = `cargo check --workspace --exclude <셸> --all-targets --all-features --target x86_64-unknown-linux-gnu` (and `aarch64-apple-darwin`). `check` does not do the final code generation or linking of that package, but **it compiles build scripts as host executables and runs them** — if the dependency graph has a build script that builds C code or looks for a native library, it may fail. So 「`rustup target add` 만으로 충분」 (rustup target add alone is enough) is true **only for a graph without such build scripts** (before measuring our workspace — the shell package requires webkit-family libraries on Linux, so it is excluded from the start). Cost = recompiling dependencies per target (unknown — not measured).
- **Runtime OS branching** (`"USERPROFILE"` · the `.exe` string · case folding) has no dedicated lint to block it (likely). The convention is to block `std::env::var*` with `disallowed-methods` and allow it only in one env module (starship · cargo PR cases — uncertain, not checked).

### 4. Shapes a text scanner must handle separately (the reviewer's supplement)

- **`cfg_select!`** (included in the Rust Reference) — writes the predicate without the `cfg(…)` spelling. **`cfg_aliases`** (creates aliases such as `#[cfg(linux)]` with a build script) · `cfg_if!` are the same kind (★correction 2026-10-06: `cfg_if!` is not — it writes the `#[cfg(…)]` spelling as is inside, so gate ④ catches it (measured). What it cannot catch is the `cfg_select!` · `cfg_aliases` side★). engram has zero of all three (measured 2026-10-06 — `rg "cfg_if!|cfg_select!|cfg_aliases"` 0). Of these, if `cfg_select!` · `cfg_aliases` come in, gate ④ cannot see them — something to note in the header limits (noted — platform header gate ④ 「한계」).
- **`unexpected_cfgs`** (rustc check-cfg) only checks `cfg` names and values and does not enforce location.

## Fit to engram constraints

Constraints (canonical = CLAUDE.md · ADR): core isolation · platform neutrality (so a macOS port does not become a full refactor · the regression net is not tied to one OS — ADR-0230) · gates run in CI and the local `/qa` binding runs the same thing · the only CI runner is Windows · clippy · cargo-deny are not in CI now (measured — 0 in ci.yml).

| Means | Layer | What it blocks | Precision | Adoption cost | Fit |
|---|---|---|---|---|---|
| Current gate ④ (PCRE2-syntax regex · list of 33) | location | an OS `cfg` in a new file | at least as broad as rustc tidy among the compared items · a few rare shapes · cannot see `cfg_select!`/aliases | 0 (done) | ○ |
| A graph check (`cargo tree --target all` or `cargo metadata`) so that production dependencies on OS API crates (`windows` · `windows-sys` · `libc` · `nix`) = only platform among the members | dependency | other members pulling in OS crates | a resolved graph · gap = how members are identified | low (like the existing gate) | ○ |
| Cross-target `cargo check` (Linux · macOS targets · shell excluded · `--all-targets --all-features`) | portability | OS-specific code written without a condition · code that does not build on another OS | the compiler | medium (measure build scripts · CI time) | ○ (a separate unit · measure first) |
| clippy `disallowed-*` | API | calls to OS-specific std APIs · a single-item `use` · `std::env::var` (peers using it for a `std::os` ban: 0 — correction 2026-10-06) | resolver-based | medium (add clippy to CI · clean up existing warnings) | △ |
| cargo-deny `wrappers` | dependency | direct dependents across the whole graph | graph · including third parties | medium (the tool · maintaining the third-party list) | ✕ |
| Dylint custom lint | location · API | `cfg` location · `std::os` paths | compiler level | large (nightly pin · `rustc_private` · per-target runs) | ✕ |
| Continuing to chase rare shapes in the regex gate | location | rare `cfg` shapes | does not converge (4 review rounds measured) | hundreds of thousands of coder tokens per round | ✕ |

## Rejection candidates (ADR rejected-alternative candidates)

- **Dylint custom lint** — nightly pin · driver version pin · per-target runs. The upkeep is large compared with the boundary it protects.
- **cargo-deny `wrappers`** — the rule means 「그래프 전체」 (the whole graph), so maintaining a third-party list follows. Our purpose (the member boundary) is met by a graph script without noise.
- **Continuing to chase rare shapes in the regex gate** — even the best precedent we found (rustc tidy) runs with known gaps. The big hole that remains is not on the location axis but on the portability axis (single-OS CI).

## grounding (main external check + reviewer re-check)

| Claim | Source | Decision |
|---|---|---|
| tidy `pal` = `EXCEPTION_PATHS` path substrings · `library/std/src/sys` · FIXME temporary exception | `rust-lang/rust` `src/tools/tidy/src/pal.rs` L38 · L57 · L60 · L86 | supported (main + reviewer) |
| skips `tests`/`benches` paths · skips if `cfg` contains `test` · self-assertion | same file L92 · L163 · L99-100 | supported |
| OS-word substring decision | same file L135-141 | supported |
| misses `cfg_attr` · skips after a leading doc comment | same file `parse_cfgs` (L171-) | partly supported (reading code — collector and reviewer agree · not reproduced) |
| meaning of cargo-deny `wrappers` | `EmbarkStudios/cargo-deny` `docs/src/checks/bans/cfg.md` L90-96 | supported (main + reviewer) |
| deno meta check | `denoland/deno` `tools/lint.js` `ensureDisallowedMethodsEnforced` (main `3d44d1d82fda`) | supported |
| mio 7 targets `cargo hack check` · tokio `cross-check` | `tokio-rs/mio` `Makefile` L2 · L27-29 · tokio `ci.yml` | supported (mio main · tokio reviewer) |
| a cross-target check cannot enforce location | Rust Reference — conditional compilation | supported (reviewer · logic) |
| `disallowed-types` catches `use` · a trait method path catches calls across all implementations | clippy source · maintainer's answer (rust-clippy discussions #16401) | partly supported (reviewer's source · not checked by main) |
| clippy cannot block `#[cfg]` location | collector's doc summary | partly supported (no counter-evidence) |

## Adversarial review (codex · BLOCK → applied)

| Finding | Defect | Applied |
|---|---|---|
| The assertions 「rustc 만」 (rustc only) and 「우리가 더 촘촘」 (we are tighter) | no ground · overstated | narrowed to 「찾은 범위 안에서」 (within what we found) and stated the compared items (U8 fault injection) |
| Presented the cross-target check as the answer to the location boundary | a logic gap (critical) | rewrote conclusions 2 and 4 as 「층 셋 — 서로 대신하지 않는다」 (three layers — they do not substitute for each other) |
| `cargo check` scope not stated | tool semantics | stated the form `--workspace --exclude <셸> --all-targets --all-features` |
| 「`rustup target add` 만으로 충분」 (rustup target add alone is enough) | overstated | narrowed to conditional on build scripts |
| `cargo tree -i` sees only members · does the same thing as `wrappers` | logic · tool semantics | host by default · filtering is the script's job · the meaning differs · `cargo metadata` alternative |
| clippy cannot catch `use` · trait matching not confirmed | tool semantics | corrected to `disallowed-types` single `use` · trait path matching |
| deno description · citation location | citation precision | corrected to function name + commit pin · 「정해진 항목 대조」 |
| `cfg_select!` · `cfg_aliases` · `cargo metadata` · `unexpected_cfgs` missing | completeness | added §4 · measured 0 in the repository |

## Issues · limits

- The peer survey is not exhaustive (GitHub code-search rate limit) — we say no more than 「찾은 것은 rustc 하나」 (we found rustc only).
- The clippy matching details and whether cross-target `cargo check` works for our workspace (build scripts) are **before measurement**.
- A re-review of this report was not run (medium = a single pass). The corrections applied the reviewer's findings as they were.
