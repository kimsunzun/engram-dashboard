# Research report — is splitting into small crates the right direction (Rust workspaces)

| | |
|---|---|
| Status | Done (medium — 1 adversarial review incorporated) |
| Date | 2026-08-17 |
| Method | 4 main-family collection tracks (parallel) → main grounding (claim ↔ source implication) → cross-family adversarial review → 3 refutations re-checked by main |
| Review verdict | **MATERIALLY-FLAWED** — 6 HIGH findings. What follows is the version **after revising to reflect those findings** |
| Confidence legend | **certain** (supported by grounding + independently confirmed) · **likely** (supported, single source) · **contested** (a refutation exists) · **uncertain** (on hold) |

---

## Conclusion

**Extracting things into crates is not itself against practice. Size is not the axis of judgment.** However, two premises we had relied on collapsed under the research — "only a crate can enforce direction" and "there must be a second consumer". Both have no external basis or have counterexamples.

**The criterion must change: not 「크기」 or 「방향 강제」, but 「독립적으로 쓸 수 있는가」.** This is the criterion of the only authoritative industry guide found, and it is broader than our current criterion.

---

## F1. Line count is not the axis of judgment — **certain** (absence confirmed) / "1.5 thousand lines is ordinary" is **uncertain**

- **Not a single project had documented a line-count threshold.** Every case judges by "is the role meaningful on its own?"
- ★**Correction:** The first synthesis concluded that "1.5 thousand lines is an ordinary lower range in mature workspaces"; **the review hit this exactly, and it was right.** What it cited as grounds was rust-analyzer's 32 crates · 200 thousand lines in total and "Some crates consist only of a single-file" (original text confirmed verbatim), but **the context of this quotation is directory layout, and it does not present a distribution of crate sizes.** The quotation was accurate, but it did not imply the conclusion.
  - https://matklad.github.io/2021/08/22/large-rust-workspaces.html
- **Downgraded items:** tower-layer 425 lines (LOC lookup failed) · Zed 231 crates / 1.3 million lines (secondary source DeepWiki, original not cross-checked) → both **uncertain**
- What remains: **there is also no basis for "wrong because small."** Size is not the question.

## F2. ★The authoritative industry guide says the opposite, strongly — **certain** (confirmed verbatim)

Microsoft Pragmatic Rust Guidelines:

- **"If in doubt, split the crate"**
- Criterion: **"Essentially, if a submodule can be used independently, its contents should be moved into a separate crate."**
- Rationale: **"as this leads to dramatic compile time improvements—especially during the development of these crates—and prevents cyclic component dependencies."**
- **"crates are for items that can reasonably be used on their own"**
- Numeric threshold: none
- https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html

★This is an item the **first synthesis missed entirely**, found by the adversarial review through the omission lens. Its direction is the opposite of our conclusion, so it is especially important.

**Applied to our `command`:** it has zero workspace dependencies so it can be used independently, and it was extracted because of cycles and direction → it **meets this guide's criterion head-on.**

## F3. Ideal graph shape — a bottom crate is the standard position — **certain** (4 confirmed verbatim)

matklad "Fast Rust Builds":

- **"a common vocabulary crate, a number of independent features, and a leaf crate to tie everything together"**
- **"The most important property of a crate is which crates it doesn't (transitively) depend on."**
- A linear chain gains nothing: **"This is slow to compile, as all the crates need to be compiled sequentially: A -> B -> C -> D -> E"**
- **"you want to keep serialization at the boundary of the system, in the leaf crates"**
- https://matklad.github.io/2021/09/04/fast-rust-builds.html

**Applied:** `command`, which depends on nothing, sits exactly in this graph's "common vocabulary crate" slot. **This quotation supports the diagnosis the user suspected, 「최상위 바닥이 없다」** — the starting point of the ideal shape is exactly that slot.

Caveat: this is **one author's design guidance and not a measured universal threshold** (review finding, valid).

## F4. ★The premise "only a crate can enforce direction" is false — **certain** (refutation confirmed)

The core claim of the first synthesis was **refuted by the review, and I confirmed it by re-checking.**

1. **`pub(in path)` creates fine-grained visibility even inside a crate.** The statement "pub(crate) is binary so it cannot create layer boundaries" was **wrong.** `pub(in crate::foo)` restricts visibility to the specified ancestor module subtree, and access from anywhere else is a compile error.
   - https://doc.rust-lang.org/reference/visibility-and-privacy.html
2. **A tool that enforces module direction declaratively really exists.** `rust_arkitect` declares `it_may_depend_on([...])` · `it_must_not_depend_on_anything()` through `rules_for_module()` and **enforces them with unit tests** (source parsing, `Arkitect::ensure_that(project).complies_with(rules)`). Maturity is low — **version 0.3.7**. `arch_test_core` also provides layer rules (`MayNotAccess` · `MayOnlyAccess` · cycle checks).
   - https://docs.rs/crate/rust_arkitect/latest/source/README.md · https://docs.rs/arch_test_core/latest/arch_test_core/
3. ★**And our repo already uses this category** — the CI `rg` isolation gate + the `cargo tree` dependency ceiling gate are exactly "enforcing direction with tests."

**Conclusion: only the claim that there is no first-class compiler-enforced means is true.** "So we have no choice but to split into crates" does not follow.

Correction: the first synthesis grouped `cargo-deny` · `machete` · `udeps` · `geiger` as "all see only the external crate graph", but `geiger` scans source and `machete` looks at source + manifest. **All that remains is the conclusion that none of them is a module-direction checker** (review finding, valid).

## F5. Direction is not one-sided — **certain**, but both sides come with caveats

**The side that reverted — tokio.** "rfc: collapse Tokio sub crates into single `tokio` crate"(closed). Verbatim reasons: "Maintaining a large number of crates comes with an increased maintainership burden." · "Maintaining correct dependencies between crates is complex." · "Users feel that large number of dependencies == bloat."
- https://github.com/tokio-rs/tokio/issues/1318
- ★**Selective omission caught by the review:** the same issue **also records the benefit of splitting** — less stable components can ship breaking releases without breaking the stable core (semver isolation). Leaving this out skews the case to one side. **Valid finding.**
- ★**Attribution correction:** "version coordination failure" and "users confusing public/internal subcrates" are **not in this issue's body** — they are secondary blog narration.

**The side that split further — ratatui** (from 0.30). Verbatim: "improve modularity, reduce compilation times, enable more flexible dependency management, and provide better API stability for third-party widget libraries."
- https://github.com/ratatui/ratatui/blob/main/ARCHITECTURE.md
- Caveat: the document **only states the motivation; there is no measurement that each effect actually improved** (review finding, valid).

**datafusion** also split further (reason = ease of code management and of reasoning about dependencies) — https://github.com/apache/datafusion/issues/1750

## F6. Cost evidence is weak — **certain** (absence confirmed)

- **A controlled A/B (same codebase, only the crate count as the variable) does not exist.** This is itself a finding.
- **Feldera's 30 minutes → 2 minutes does not generalize to us.** 1,106 crates, 64 cores/128 threads. Decisively, **what was split is code auto-generated by the SQL compiler** ("Since the Rust code is entirely auto-generated from this structure, we had total control over how to split it up."). The author's own warning, verbatim: **"In most Rust projects, splitting logic across dozens (or hundreds) of crates is impractical at best, a nightmare at worst."**
  - https://www.feldera.com/blog/cutting-down-rust-compile-times-from-30-to-2-minutes-with-one-thousand-crates
- **Splitting a linear chain had no effect (ruff).** Verbatim: "since `ruff_cli` depends on `ruff` this doesn't really change the total build time" — https://github.com/astral-sh/ruff/issues/1820
  - Caveat (review): the same issue also reports a **gain when building only the library** (avoiding about 100 CLI dependencies) → **workload-dependent, not categorically ineffective**. Valid.
- **Circular dependencies blocked further splitting (ruff, same issue).** Verbatim: "the rule implementations depend on `ast::Checker` and `ast::Checker` depends on the rule implementations."
- **Cargo metadata overhead of 28% (Bevy, bjorn3)** — confirmed verbatim. Caveat (review): **a single workload from 2020, and the crate count was not controlled as a separate variable.** Cargo has since gained resolver caching and performance improvements → **downgraded: likely → reference circumstance.**
- **A real orphan rule case (Rust-for-Linux)** — a split attempt ran into the orphan rule: https://github.com/rust-lang/rust/issues/136979
  - ★**2 corrections (review):** the workaround is an **additional trait**, not a "free function" · **`extern_impl` (RFC PR #3482) is a closed design proposal, not an "unstable feature"** — it must not be written as if it were technically available.
- **feature unification** — **contested / uncertain.** The first synthesis wrote cargo#12676 as "acknowledged as a regression by cargo", but this was **not confirmed** (the issue is S-triage, with no maintainer decision). The review rebutted that "the reproducer did not use `resolver = "2"`, so it is just v1's documented behavior", but **it was confirmed that the reproducer did not specify a resolver, yet there is still no maintainer decision.** Neither side is established.
  - https://github.com/rust-lang/cargo/issues/12676 · https://doc.rust-lang.org/cargo/reference/resolver.html
- **monomorphization duplication** — downgraded: depends on a specific dependency topology (sibling-crate diamond) and optimization settings, and can be shared with `-Zshare-generics` → **conditional, uncertain.**
- **A real-harm case from widening `pub(crate)` to `pub`** — not found → **uncertain.**

## F7. The "second consumer" criterion does not exist externally, and has counterexamples — **likely**

- Not in the Cargo Book, the rust-analyzer docs, matklad or the forums is there **a rule based on consumer count** (absence confirmed).
- ★**Counterexample (review finding, valid):** rust-analyzer makes `ide` an API-boundary crate **even though it has only one consumer**, and conversely explicitly excludes `hir-expand` · `hir-def` · `hir_ty` as **"never be an api boundary"**. That is, the boundary decision does not run on consumer count.
  - https://rust-analyzer.github.io/book/contributing/architecture.html
- A proc-macro is a case where splitting is **forced**. But the universal claim "the only forced case" has no basis (review finding, valid) — it cannot rule out other build, link or target constraints.

## F8. Changes in progress (missed by the first synthesis — found by the review)

All are **not yet stable**, so do not use them as grounds for the current decision; record them only as "the picture may change later."

- Cargo `resolver.feature-unification` (nightly) — per-package unification mode — tracking issue #14774
- Cargo open namespaces (nightly, #13576) — several packages take part in one API namespace
- Cargo workspace bulk-publish support — eases part of the burden of multi-crate releases. **But it is not atomic**
- Rust project goals: "relink, don't rebuild" · incremental system rethought
- The "Inline crates" proposal: **the author himself states he does not intend to pursue it** and it presupposes an overhaul of the compiler → not a near-term solution

---

## Applied to our repo (fact check)

| Axis | Our state | What the research says |
|---|---|---|
| Size | `command` real code 1,519 lines (32% of 4,705 total) | Not the axis of judgment |
| Independent usability | zero workspace dependencies | **Meets the Microsoft criterion** → the split is justified |
| Graph position | the bottom that depends on nothing | the "common vocabulary crate" slot of matklad's ideal shape |
| Motivation for splitting | enforcing dependency direction | **that reason alone does not need a crate** (F4) |
| Consumer count | core + protocol (planned) = 2 | not an external criterion, has counterexamples (F7) |
| Compile gain | not measured | no evidence at our scale (F6) |
| Dead code | route · link about 185 lines, zero callers · trait implementations and calls zero | a net gain regardless of the structural decision |
| Domain-neutral parking | about 880 lines in `core` | the real basis for a top-level bottom (F3) |

★**Caveat (review finding, valid):** our side's figures in this table **have no revision stamp and no reproduction query.** One refactor can make them false, so when recording them in a decision record, the commit hash and the command that produced them must be left alongside.

---

## Points of contention (no winner picked)

1. **tokio (merging) vs ratatui · datafusion (splitting)** — both stated their reasons and both are alive. The tokio issue itself also records the semver isolation benefit of splitting.
2. **Our ADR-0130's "second consumer" criterion vs Microsoft's "if it can be used independently"** — the latter is much broader. Whether to keep or replace our criterion is a **user decision.**
3. **Whether feature unification is a living cost** — neither side is established.

## Limits / gaps

- 3 absence confirmations are carried in the conclusion (no line-count threshold · no consumer-count rule · no controlled A/B). An absence confirmation cannot be told apart from the limits of the search scope.
- Primary documents on the rationale for rustc's own crate split, and the relevant clause of the Rust API Guidelines — failed to reach.
- tower-layer LOC · Zed scale — not cross-checked.
- The previous session's findings (the ban on the name `core` · dismantling bevy_core) are **outside this grounding's scope.**
- **The adversarial review was done once** (medium). There is no backstop for what the review missed.
