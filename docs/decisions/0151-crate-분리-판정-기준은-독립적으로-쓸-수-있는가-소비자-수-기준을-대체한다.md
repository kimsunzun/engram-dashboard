# ADR-0151: The crate-split criterion is whether it can be used independently — it replaces the consumer-count criterion

- 상태: 확정 (2026-08-17, basis: user decision 2026-08-17 — with the corrections from `/research medium` and the other-family adversarial review applied) · 부분 폐기 by ADR-0175 (bottom crate composition) · 부분 폐기 by ADR-0236 (positive evidence that the control leaf) · 부분 폐기 by ADR-0269 (the decision on rejected alternative E and the per-chunk decision expectation in the impact section)
- 관련: Amends ADR-0130 (split criterion) · CLAUDE.md 「판단 기준」·「코어 격리」 · ADR-0012 (module isolation) · ADR-0155 (the new tool crate prompted this question) · ADR-0110·ADR-0129 (precedents for a split with a cohesion reason of its own) · Amended by ADR-0175 (bottom crate composition) · Amended by ADR-0236 (positive evidence that the control leaf) · Amended by ADR-0269 (the decision on rejected alternative E and the per-chunk decision expectation in the impact section)

## 맥락

The `command` crate is small — **1,519 lines of real code** (32% of 4,705 total lines; the rest is 48.5% tests and 15% comments) — and it is **the only crate that exists solely for dependency direction**, so the user asked: *"소스 몇 개 안 되는데 이렇게 crate로 만드는 게 맞는지. 최상위 바닥이 없어서 생긴 증상 아닌가."* (There are only a few sources, so is it right to make a crate like this? Isn't it a symptom of having no top-level bottom?)

The existing decision criterion was **re-discussion trigger 1** of ADR-0130 — 「따로 쓸 소비자가 **실제로** 생긴다(가정이 아니라 착수된 소비처)」 (a consumer who would use it separately **actually** appears — not an assumption but a consumer that has been started). It is the criterion that ADR set up when it rejected splitting out the control plane with "싸다는 게 해야 할 이유가 되지 못한다" (being cheap is not a reason to do it).

External practice was investigated with `/research medium`, and the other-family adversarial review returned a **MATERIALLY-FLAWED** verdict with six HIGH findings. What follows is the result with those corrections applied.

## 결정

1. **The crate-split criterion = 「독립적으로 쓸 수 있는가」 (can it be used independently).** Whether a real consumer exists now is not asked. When in doubt, lean toward splitting.
2. **What cannot be used independently stays as a module inside the crate that uses it.** It is not gathered into a bottom crate.
3. **Size (line count) is not a judgment axis.** There is no basis for saying it is wrong because it is small, nor for saying it must be large.
4. **★Do not use 「의존 방향을 강제하려면 crate로 쪼개야 한다」 (to enforce dependency direction you must split into crates) as a basis — it is false.★**
5. **ADR-0130's decision to hold stays valid.** Replacing this criterion does not automatically reopen it (→ impact section).

## 거부한 대안

- **A. Keep 「지금 두 번째 소비자가 있나」 (is there a second consumer now)** — ㉠ **No authoritative external source has a consumer-count-based rule** (Cargo Book · rust-analyzer architecture document · three matklad posts · a full search of users.rust-lang.org — absence confirmed). ㉡ **A counterexample exists in real code:** rust-analyzer takes **`ide`, which has only one consumer, as an API boundary crate**, and conversely **explicitly excludes** `hir-expand`·`hir-def`·`hir_ty` with "never be an api boundary" — boundary decisions do not run on consumer count. ★**Strength of the rejection basis: the counterexample is real, but the 「규칙이 없다」 (there is no rule) side is an absence confirmation and therefore weak** (indistinguishable from the limits of the search scope)★ — the user chose the replacement after hearing this weakness (2026-08-17).
- **B. Set a line-count threshold** — **Not one project had documented a threshold.** Every case investigated decides by 「역할이 독립적으로 의미 있는가」 (does the role mean something independently).
- **C. Create a top-level bottom crate now** — The user's judgment (「공수가 좀 있으면 나중에」, if it takes some effort, later). The research supported that slot itself as the standard form, so this is **a hold, not a rejection**.
- **D. Name that bottom crate `core`** — It collides with Rust's built-in `core` and **breaks once a derive macro is involved** (axum#1133 was closed as "no plan to fix" · cargo#7760 and rust#90960 are open). This repository has many `ts-rs` and `serde` derive dependencies, so it is hit head-on. **But this item comes from an earlier session's investigation and is outside this round's grounding scope.**
- **E. Gather domain-neutral code into a bottom crate whose name only points at a layer position** — A precedent where Bevy's `bevy_core` sucked in even time, name tags and the task pool and then was **dismantled and scattered by purpose** (issue #2931). **From an earlier session's investigation — outside this round's grounding scope.**

## 근거

**Microsoft Pragmatic Rust Guidelines (verbatim, checked against the original on 2026-08-17).**

> "You should err on the side of having too many crates rather than too few, as this leads to dramatic compile time improvements—especially during the development of these crates—and prevents cyclic component dependencies. Essentially, **if a submodule can be used independently, its contents should be moved into a separate crate.**"

- "crates are for items that can reasonably be used on their own" · **no** numeric threshold.
- The same document also **admits the case of re-joining**: "In some cases, it is desirable to re-join individual crates back into a single umbrella crate."
- ★Caveat: **it is one company's public guideline, not an ecosystem consensus.** The basis is that one document alone. This decision does not hide that it rests on that one source.
- https://microsoft.github.io/rust-guidelines/guidelines/universal/index.html

**matklad "Fast Rust Builds" (four verbatim quotes checked against the original).**

- The ideal shape: "a common vocabulary crate, a number of independent features, and a leaf crate to tie everything together"
- "The most important property of a crate is which crates it doesn't (transitively) depend on."
- A linear chain compiles sequentially: "all the crates need to be compiled sequentially: A -> B -> C -> D -> E"
- → **`command`, which depends on nothing, sits in the 「공통 어휘 crate」 (common vocabulary crate) slot of this graph.** Caveat: this is one author's design guidance, not a measured universal threshold.

**★The basis of decision 4 — why 「방향 강제엔 crate밖에 없다」 (only crates can enforce direction) is false★**

- `pub(in path)` **restricts visibility to the subtree of a named ancestor module** and any access outside it is a compile error (Rust Reference). The statement "`pub(crate)` is binary so it cannot make a layer boundary" is wrong.
- **A tool that enforces module direction declaratively and checks it in a unit test really exists** — `rust_arkitect`'s `rules_for_module()` + `it_may_depend_on([...])`·`it_must_not_depend_on_anything()` (source parsing, `Arkitect::ensure_that(project).complies_with(rules)`). **Its maturity is low — version 0.3.7.** `arch_test_core` also provides layer rules (`MayNotAccess`·`MayOnlyAccess`·cycle checking).
- **This repository already uses that category** — the CI `rg` isolation gate + the `cargo tree` dependency-ceiling gate are exactly 「테스트로 방향 강제」 (enforcing direction by test).
- What is true is **only 「컴파일러가 강제하는 1급 수단이 없다」** (there is no first-class compiler-enforced means), and "so we have no choice but to split into crates" does not follow.

**The cost evidence is weak (in both directions).**

- **A controlled A/B (same codebase, only crate count as the variable) does not exist.** That itself is a finding of the research.
- Feldera's 1,106 crates and 30 min → 2 min split **code generated automatically by a SQL compiler**, and the author himself warned against generalizing: "In most Rust projects, splitting logic across dozens (or hundreds) of crates is impractical at best, a nightmare at worst."
- ruff split into a **linear chain, so build time did not shrink** ("since `ruff_cli` depends on `ruff` this doesn't really change the total build time"). But the same issue also reports a gain when building only the library, so it is **workload-dependent, not categorically ineffective**.
- The Cargo metadata overhead of 28% (Bevy, bjorn3) is a single workload from 2020 and did not control crate count as a separate variable → used **only as circumstantial reference**.

**A contested point — no winner is picked.** tokio **merged** its sub-crates into one (maintenance burden · dependency management complexity · "Users feel that large number of dependencies == bloat"), and **the same issue also notes the semver isolation benefit of splitting** (a less stable component can ship a breaking release without breaking the stable core). ratatui (0.30) and datafusion, on the contrary, **split further**. Both stated their reasons and both are alive.

**Adversarial-review corrections applied.** We **withdrew** the conclusion in the first synthesis that 「1.5천 줄은 성숙 워크스페이스에서 평범하다」 (1.5 thousand lines is ordinary in a mature workspace) — the 8 quotations were verbatim-accurate, but no source implied the conclusion that combined them (the rust-analyzer document gives only crate count and total line count, not a size distribution). So decision 3 was written not as "small is normal" but as **"size is not an axis"**.

## 영향 / 불변식

- **★ADR-0130's decision to hold stays valid — but what holds it is not a basis; it is decision 5 above.★** No defense argument is set up in this place; only facts are written.
  - **0130 rejected option A (split out only the control plane) on three bases** — ㉠ **no consumer** ㉡ **mismatch** with the shape drawn by ADR-0129 decision 2 (the **whole** application as one lib) (one more decision would be added) ㉢ **the rot argument is weak for this chunk**. **㉠ is exactly the criterion this ADR has just discarded.** ㉡ and ㉢ are not 「지금 쪼개면 안 되는 이유」 (reasons not to split now) but 「그때 그 순서로 안 한 이유」 (reasons it was not done in that order back then).
  - **The rest of 0130's bases (the purpose achieved by splitting net · the scale illusion · "거기까지는 오버") are not the rejection text for option A** — they sit in the place that rejected scope expansion in general, in particular option B (completing 0129). Do not move them over as a defense of option A.
  - **★Measured by the new criterion, 0130's own bases rather give positive evidence★** — `control/` is a **leaf with zero outgoing edges** to other top-level daemon modules (0130 §근거 ③), has a **separate entrance** that does not go through the webview WS (§근거 ④), and **does not touch `net`**, so it sidesteps the `frame_port` feature problem (§거부한 대안 option A). On 「독립적으로 쓸 수 있는가」 all three are signs toward **yes**. Even the scale-illusion paragraph says "그중 절반 가까이가 leaf인 제어 평면" (nearly half of that is the control plane, which is a leaf), pointing the same way.
    - ★**Amendment marker — ADR-0236**★ — `control/` has not been a leaf since 2026-08-19 (S20), so this positive evidence disappeared then. ADR-0236 put down the leaf property and withdrew re-discussion trigger 2.
  - **So what actually holds the hold is decision 5 — a one-line user judgment** (the "거기까지는 오버" in 0130's 맥락). **It is not a conclusion derived from a basis.** It is stated here so the next session does not read this line as an argument.
  - **What must be in place for it to be reopened (matters for the user to re-decide)** — ① decide how to reconcile splitting out only `control/` with the shape of ADR-0129 decision 2 (the whole application as one lib) ② decide the `frame_port` feature membership **before starting** (the last item of 0130 §영향 — it is a public API change, so deferring is expensive) ③ the user's answer on whether to spend now the effort of fixing the import paths on the incoming-edge side (three modules). **All three are human judgments, so they cannot be made into a machine trigger.**
  - **Re-discussion trigger 2 (a production edge going out of `control/`) stays valid as is** — it now has one more property: if it breaks, it is no longer a leaf and **the positive evidence above disappears itself.**
    - ★**Amendment marker — ADR-0236**★ — Re-discussion trigger 2 was withdrawn by ADR-0236 (`control/` has had this edge since 2026-08-19 (S20)).
- **Change the justification wording of the `command` crate** — not 「의존 방향 강제」 (enforcing dependency direction) but **「독립적으로 쓸 수 있고 순환을 막는다」** (can be used independently and prevents cycles). If the header of that crate's `src/lib.rs` states direction as the reason for existing, it is a target for updating.
- **Candidate for the top-level bottom crate = about 880 domain-neutral lines parked in `core`** (playback state machine 610 · name derivation 206 · logging 66 — they are there for the reason 「여기 둬야 테스트가 돈다」, tests only run if they are here). **Decide chunk by chunk, separately** — there is no basis for putting the three together into one, and several small crates with purpose names may come out (that fits matklad's "independent features" and also avoids the `bevy_core` failure).
- **★Rename trap★** — the dependency-ceiling gates identify workspace members **by the `engram-dashboard` name prefix**. If renamed to a name without the prefix, **that gate quietly stops seeing that crate.** Fix the regex in the same commit.
- **Revision stamp duty** — our own figures above (1,519 real-code lines · 880 domain-neutral lines, etc.) are **values as of 2026-08-17**. A document or ADR that uses these figures as a basis leaves **the re-measurement method with them**. If it does not, one refactor makes them false (an adversarial-review finding).
  - **★The method to re-measure this ADR's own figures = the same as ADR-0130 §근거 ⑤★** — separate real code from tests by **splitting at the start line of each file's `#[cfg(test)] mod tests`** (the very method 0130 used to break the 「약 8,000줄」 (about 8,000 lines) illusion). The 880 domain-neutral lines are a sum of three chunks (playback state machine · name derivation · logging) measured **separately**, so re-measurement is also done chunk by chunk. ★**Details beyond that precedent have no record**★ — by what rule the 15% comments were stripped and exactly where the file boundaries of the three chunks were are not on record. **So if a re-measured value diverges from the original, it cannot now be told whether the code changed or the way of counting differs** — the next measurer closes that hole by writing the criterion used alongside the figures. This was a place where it did not keep its own rule, so this line was filled in late.
- This decision is a **criterion** — which crate to make and when is still the user's decision.
