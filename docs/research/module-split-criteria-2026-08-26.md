# Splitting modules and libraries — benefits and split criteria (industry practice survey)

- **Status:** medium tier · 3 parallel collection tracks → main grounding → cross-family adversarial review **done**
- ★**Note on reading order**★ — the body below (§1–§7) is the synthesis from **before** the review. **§9 「적대 리뷰 결과」 (adversarial review results) corrects or discards several lines of the body** — read §9 first before quoting the body.
- **Survey date:** 2026-08-26
- ★★**Overlaps with an earlier survey — read that one first**★★ — [`crate-granularity-survey-2026-08-17.md`](crate-granularity-survey-2026-08-17.md) has already covered **the same question** (「소형 crate 분할이 옳은 방향인가」 — is splitting into small crates the right direction) at the same tier with the same method (medium + cross-family adversarial review). **This session did not know it existed when the survey started** (the collectors were not told either) and found it at the end of the session. How the two reports relate:
  - **Where the earlier survey is more authoritative = the decision criterion.** Its conclusion is not 「크기」 (size) or 「방향 강제」 (direction enforcement) but **「독립적으로 쓸 수 있는가」** (can it be used independently), and it records that as the criterion of the only authoritative industry guide that was found. It also decided that two premises we had been relying on (**「방향 강제엔 crate밖에 없다」** (only a crate can enforce direction) · **「두 번째 소비자가 있어야 한다」** (there must be a second consumer)) collapsed in that survey. ★The items 「재사용(소비자 둘)」 (reuse (two consumers)) and 「의존 방향 강제」 (dependency direction enforcement) in §1 and §3 of this document collide head-on with that decision — do not quote them without re-checking★.
  - **What this document newly adds** = ① the entire TS·JS axis (the earlier one is Rust-only) ② general literature (Parnas·Martin·Google SWE ch.21·Nx·DORA) ③ an empirical summary of the cost axis ④ 15 findings extracted by adversarial review (§9).
  - **Unresolved:** merging the criteria of the two surveys into one was not done. For the next session to use it for a big decision, **putting the two side by side and cross-checking them is a prerequisite**.
- **Method:** one collector per track (general theory / Rust / TS·JS), 8–12 searches each. The main session externally verified the claim↔source implications of the load-bearing claims. **Quotations:** the Korean source gives most outside quotes only in Korean; those are back-translated here and marked (back-translated) — they are not the original wording. Quotes whose original English the source itself gives (the grounding section, or English in the source) are verbatim.
- **Confidence legend:** **certain** = supported by grounding + independent cross-confirmation · **likely** = supported by grounding, single source · **uncertain** = unsupported or primary text not reached → on hold
- **Out of scope:** our repository's decisions (ADRs) were not given to the collectors — they were separated so that general practice would not be anchored to our decisions. Applying it is done outside this document.

---

## One-line conclusion

**The list of benefits that justify splitting is short, and most benefits come from 「경계」 (boundaries), not from 「별도 유닛」 (separate units).** What you get only with a separate unit is about four things — an independent deployment/release unit / a unit that tools count for caching and affected scope / acyclicity enforced by the language / publishing to external consumers. The rest (information hiding · comprehensibility · parallel work · isolated testing · ownership) can also be obtained with boundaries inside a unit.

---

## 1. What splitting buys — a table sorted by 「별도 유닛이 필요한가」 (is a separate unit needed)

| Claimed benefit | Who put it forward | Is a separate unit needed | Confidence |
|---|---|---|---|
| Information hiding — changes do not spread | Parnas 1972 | **No** — the criterion is the interface, and the module he spoke of is a unit of work assignment | uncertain (primary text not reached — see grounding below) |
| Parallel development · comprehensibility | Parnas 1972 | **No** — a stable interface + ownership is enough | uncertain (same reason) |
| Reuse | R.C. Martin, REP — "the unit of reuse is the unit of release" (back-translated) | **Yes** — the only principle that explicitly ties reuse to the release unit | likely |
| Independent deployment | Sam Newman (one of the 3 reasons for splitting a service) | **Yes** (by definition) | likely |
| Independent scaling · failure isolation · technology heterogeneity | Grzybek (architecture-driver table) | **Yes** | likely |
| Build·compile time | matklad · Anderson (TiKV) · Feldera | **Yes**, but conditional — the graph must **get wider**. matklad: "if you ended up with a graph like the first one, you are not winning much" (a chain-like graph) | certain (quoted text checked) |
| Dependency direction enforcement | matklad — "the most important property of a crate is which crates it doesn't (transitively) depend on" | **Partial** — with a crate the compiler counts it, and it also stands through visibility·lint·architecture tests | certain (quoted text checked) |
| Cache·affected-scope unit | Nx · Turborepo | **Yes**, but being a 「도구가 인식하는 프로젝트」 (a project the tool recognizes) is enough and publishing to a registry is not needed | certain |
| Isolated testing | widely claimed | **No** — it is a seam·DI problem. No authoritative source found saying a package is needed | uncertain |
| Team ownership boundary | MacCormack–Baldwin–Rusnak · Newman | **No** — directory ownership does it. What was measured is organizational coupling ↔ design modularity, not the shape of the artifact | likely |
| Option value (independent experimentation · swappability) | Baldwin & Clark, *Design Rules* | **No** — the value comes from hiding, experiment cost and visibility, not from the packaging form | likely |

## 2. Cost of splitting — this side is better documented than the benefits

- ★**The dependency-management tax replaces the source-control tax**★ — *Software Engineering at Google* ch.21: "all else being equal, prefer source control problems over dependency management problems." (back-translated) The sentence just before: "source control problems are much easier to think about and much cheaper to deal with than dependency management problems." (back-translated) **Certain** (primary text checked directly) https://abseil.io/resources/swe-book/html/ch21.html
- **SemVer is off in both directions** — over-constraining (a change to an unused API is also a major) + under-protecting (Hyrum's Law: someone depends on every observable behavior). In the same chapter, "in theory, a patch that changes only implementation details is 'safe' — this is directly at odds with Google's experience of Hyrum's Law" (back-translated). **Certain** (checked directly)
- **Diamond dependency** — the same chapter: "C++ has nearly zero tolerance for diamond dependencies in a normal build, and it is very likely to cause arbitrary bugs and undefined behavior." (back-translated) **Certain** (checked directly)
- **API surface growth / loss of internal visibility** — once you cross a boundary, internals become public. **Certain** (language mechanism) / no authoritative source found that names this as the main cost
- **Cross-cutting refactoring gets harder** — cycles become illegal, so a split that needs mutual references requires a redesign. **Likely**
- **The cost of making something reusable itself** — Glass *Facts and Fallacies* Fact 18: a reusable component is about **3 times** harder than a single-use one, and you must see **3** real uses before generalizing (Biggerstaff's rule of three). **Likely** (secondary source)
- **Runtime and communication cost when the split is a *deployment unit*** — Prime Video reported cutting infrastructure cost by about 90% by merging a distributed detector into one process. **That the claim was made is certain / generalizing it is uncertain** — it is a case of one component moving to a modular monolith, and it is not that Amazon abandoned microservices.

## 3. Named split criteria

- **Parnas (1972)** — split along *design decisions* that are likely to change and hide them behind a minimal interface. He explicitly rejects decomposition by processing step (flowchart). *(primary text not reached — confidence uncertain)*
- **R.C. Martin — component cohesion triangle** — REP (unit of reuse = unit of release) · CCP (things that change together go together) · CRP (things reused together go together). REP·CCP push components **bigger**, CRP pushes them **smaller**, and Martin himself defines it not as rules but as a **tension diagram**. **Likely**
- **Sam Newman** — three things that justify splitting a service: independent deployability · data isolation · reflecting the organizational structure. And he says a modular monolith is "a great choice for many organizations" (back-translated). **Likely**
- **matklad** — make the crate graph **wide, not deep**. The design question is "what should this crate *not* depend on" (back-translated). For large workspaces, **a flat `crates/` up to 10k–1M LOC**, layers only beyond 100 crates (roughly a million lines). **Certain** (quote checked directly)
- **Nx (official) — the sharpest criterion: "split when tests slow you down."** Original text: "Split when tests slow you down. A project whose test suite dominates CI, while most changes only touch a slice of it, is the clearest split candidate." And "the more granular the projects, the more effective `nx affected` and the computation cache are." (back-translated) It also states the ownership axis: "if two teams keep editing the same project, boundaries and review assignment blur." (back-translated) **Certain** (checked directly) https://nx.dev/docs/kb/project-size
- **Glass/Biggerstaff rule of three** — do not generalize into a reuse unit before there are three real uses. **Likely**
- **DORA (an outcome criterion, not a structure criterion)** — can a team deploy on demand regardless of what it depends on, can it test without a shared integration environment. **Certain**
- **Baldwin & Clark** — when net option value (technical potential × cost of independent experimentation × module visibility) − module architecture cost > 0. **Certain** (as a method) / cases of actually applying it to a library boundary are uncertain

## 4. Rust specifics — be precise about the compile-time claim

- **A crate is the unit of (re)compilation and of Cargo parallelization.** "In Rust the compilation unit is the *crate*, which consists of multiple files" (back-translated) (Brian Anderson, TiKV/PingCAP). **Certain**
- ★**But 「쪼개면 빨라진다」 (splitting makes it faster) is conditional, and the most-cited counterexample comes from the same article that popularized the claim**★ — Anderson: "the gains expected from increased parallelism are often erased by other factors such as downstream monomorphization, generic duplication and LTO." (back-translated) **Certain** (quote)
- **One crate is not fully serial in the backend** — rustc splits a crate into codegen units and hands them to LLVM in parallel (default max 16, incremental max 256). nnethercote's measurement: `codegen-units=1` 9.7s vs 16 CGUs 4.5s, but fewer CGUs give a smaller binary and better code quality. **Certain** (measurement as given in that article)
- **Pipelined compilation** (Rust 1.38, 2019-09) — a dependent crate starts as soon as the `.rmeta` of the crate it depends on is out. Official claim of a **10–20% improvement**. **Certain**
- **With many generics, splitting does not reduce it** — "generics are translated only when instantiated, so even if crate sizes were perfectly even for parallelization, generic types are not translated until late in the crate graph" (back-translated) (Anderson). Leaf/binary crates absorb that work. **Certain**
- **The only enforcement a crate boundary buys** = **acyclicity**. Modules inside one crate compile even if they are cyclic, but the package graph must be acyclic. **Certain**
- **Visibility is hierarchical, not directional** — `pub(crate)`·`pub(in path)` restrict *where it is visible* but cannot say "B may call A but A must never call B". So "direction enforcement" is a **valid** reason to split if you want to use the compiler as the enforcer. **Likely** (the collector's interpretation — no primary source says it in this form)
- **A crate boundary ≠ an API boundary** — the rust-analyzer architecture document states for some crates, "these crates are not API boundaries and never will be" (back-translated). Splitting alone does not buy a stable interface. **Certain**
- **Cost of splitting (Rust)** — loss of cross-crate inlining (recovered only with `#[inline]` or LTO) · monomorphization duplication · **feature unification** (not avoidable among workspace members, and avoiding it takes a separate cargo invocation) · **orphan rule** (if the trait and the type are in different crates, a third crate cannot impl → newtype workaround) · duplicated dev-dependency·integration-test targets. **Certain to likely**
- **Real cases go both ways** — Bevy **dismantled** a grab-bag base crate (`bevy_core`) **into domain crates** ("this crate is basically a utility crate and we want to avoid that" (back-translated)). Tokio, in the opposite direction, **merged** several `tokio-*` crates into one with feature flags (maintenance burden · dependency consistency · bloat users feel · release cost). **Certain**
- **The largest measured case is atypical** — Feldera split one generated crate into 1,106 and went from 1617s to 150s. The authors themselves say this shape is "impractical for most Rust projects" (back-translated). **The claim is certain / generalizing it is uncertain**

## 5. TS·JS specifics — 「패키지로 쪼개면 강제된다」 (splitting into packages enforces it) is false

- ★**Key point**★ — Node.js official docs, the `exports` field: "when the `exports` field is defined, all subpaths of the package are encapsulated and no longer available to importers." (back-translated) It goes on: **"this is not a strong encapsulation — directly requiring an absolute subpath such as `require('/path/to/node_modules/pkg/subpath.js')` still loads `subpath.js`."** (back-translated) The only enforcement point the docs describe is **an error raised at the specifier resolution step**. **Certain** (primary text checked directly) https://nodejs.org/api/packages.html
- **Three bypass routes** — ① relative·absolute file paths ② TypeScript `moduleResolution: node10` does not support `exports` at all ③ **when a `paths` alias matches, resolution proceeds as a relative one and `exports` processing does not happen** — that is, the common monorepo `paths` setting silently turns off `exports` enforcement in type checking. **Certain**
- **The tools prove it themselves** — the reason Nx `@nx/enforce-module-boundaries` and `turbo boundaries` (experimental) exist is that **splitting into packages alone did not block it**. Turborepo RFC maintainer: "since the code is on the same filesystem, you can easily leave the package root with `../../`." (back-translated) **Certain**
- **Boundary enforcement without packages** — `good-fences` (a `fence.json` per directory: what is reachable from outside · consumable tags · allowed dependencies) · `dependency-cruiser` · `eslint-plugin-boundaries` · **TypeScript project references** (the handbook example references a sibling directory, and its goals include "enforce logical separation between components" (back-translated) — the referenced thing does not have to be an npm package. A `composite` project rejects sources outside its file list with TS6307, but it is a noisy gate with frequent false positives). **Certain to likely**
- **What you get only as a package·project** — publishing · independent versions · **cache·affected granularity** (Nx: the more granular the more effective, and affected false positives also drop) · cache-invalidation consistency (Turborepo: if you leave with `../../`, there are dependencies the cache cannot track) · independent CI scope · per-package dependency isolation. **Certain**
- **Cost of splitting (TS)** — phantom dependency (an undeclared dependency "works" thanks to hoisting) · NPM doppelganger (duplicate install of the same dependency → singleton·type-identity conflicts) · version skew · the choice for internal packages between shipping source and precompiling (the latter adds build·watch to every dev loop) · project-reference upkeep · no support for nested packages · cross-cutting refactoring·navigation cost · without `declarationMap`, go-to-definition lands on `.d.ts`. **Certain to likely**

## 6. The opposing position (steel-man)

- **Fowler, MonolithFirst (2015)** — almost all successful microservice cases were split out of a monolith, and systems built microservice-first ran into "serious trouble" (back-translated). As a separate concept, the **microservice premium** — the real cost that makes a monolithic design more favorable in apps of low complexity. **Certain**
- **The opposition to the opposition** — Stefan Tilkov: if microservices are the goal, do not start with a monolith (extraction is harder than it seems). **Likely**
- **Sandi Metz, The Wrong Abstraction (2016)** — "duplication is far cheaper than the wrong abstraction." (back-translated) The prescription is to **paste back** (re-inline) into each call site, delete what that call site does not use, and then extract again. ★**It is explicitly an opinion and has no data**★. **Certain** (that the article has no data)
- **Google's institutional opposition — the One Version Rule** — developers must have *no choice* of version. Violations lead to merge-strategy debates, diamond dependencies and lost work. It is a direct argument against internally versioned libraries. **Certain**
- **On the Rust side** — adding·removing a module is cheaper than adding·removing a package · modules in one crate can be cyclic · ultimately "a trade-off between convenience and compile speed" (back-translated). That Yoshua Wuyts proposes "inline crates" to get crate-level encapsulation without workspace procedure is itself evidence of split friction. **Certain**
- **On the TS side** — `good-fences` provides a per-directory public surface and an inbound allowlist inside a single TypeScript project (half of package encapsulation is obtained without a package). And **the enforcement you get after splitting is in the end lint-shaped** (Nx and Turborepo both put a linter *on top of* the split). **Certain / the latter is the collector's inference**

## 7. What was measured and what is opinion

- **Measured (peer-reviewed):** MacCormack, Baldwin & Rusnak, *Research Policy* 41(8), 2012 — matched product pairs. Loosely coupled organizations produced significantly more modular designs, and the difference in the likelihood that a design change in one component propagates to another is **up to 6 times**. **Likely — with a numeric discrepancy** (see grounding below)
- **Measured (survey·statistics):** DORA — a loosely coupled architecture is one of the strongest predictors of continuous delivery, and it is explicitly separated from technology choice ("achievable with a mainframe, and can fail with the latest fashionable technology" (back-translated)). **Certain**
- **Measured (practice·single case):** Feldera 1617s→150s · rustc CGU 9.7s vs 4.5s · Rust 1.38 pipelining 10–20%. **Reporting the numbers is certain / generalizing is uncertain**
- **Weakly holds:** the conventional wisdom that coupling·cohesion is *the* driver of modularization — research raises doubts. **Likely**
- **Does not hold:** the benefits of microservices — SLRs report that the empirical evidence is very limited. **Likely**
- **Opinion only:** Metz · the premature-abstraction family · Fowler MonolithFirst (anecdote-based by its own account) · the Grzybek driver table. **Certain**

---

## Grounding results (main-session external verification)

**Supported — checked directly in the primary text (4):**
1. The three *SWE at Google* ch.21 quotes (source control first · SemVer/Hyrum · C++ diamond) — wording matches. However, the original says "nearly zero tolerance" and the collector rendered it "almost no tolerance" (same meaning, not same wording).
2. Two matklad quotes + caveat — wording matches ("The most important property of a crate is which crates it doesn't (transitively) depend on." · "if you ended up with a graph like the first one, you are not winning much.").
3. Node.js `exports` "**not a strong encapsulation**" + the absolute-path bypass example — wording matches. It is the core of the front-end conclusion, so it was checked directly.
4. Nx "Split when tests slow you down." + affected·cache granularity + the ownership axis + the counter-argument ("Related code should be close together") — wording matches.

**Unsupported·on hold (1):** **The primary text of Parnas 1972 was not reached** — 3 hosts tried (MIT connection refused · the TU/e PDF could not be converted to text · acolyer 403) + local PDF rendering failed (poppler not installed). So 「모듈 = 작업 배정 단위」 (module = unit of work assignment) and the benefit list **rely on secondary summaries and model memory → downgraded to uncertain**. ★This is the first basis for the conclusion 「대부분의 이점은 별도 유닛을 요구하지 않는다」 (most benefits do not require a separate unit), so it is the weakest bridge in this document.★

**Numeric discrepancy (1):** the propagation-likelihood multiplier in the MacCormack paper splits into **6x vs 8x**. Domain-restricted search (dash.harvard·hbs) says 6x, general search summaries say 8x. The primary abstract was blocked with 403/405. → **Adopt 6x but state the discrepancy.**

**Downgrade for misattribution (1):** the compile-time figure 2m22s→26s that the collector attributed jointly to "corrode/matklad" is **not in the matklad original** (the numbers in that article are CI 8 minutes · target 10 minutes). The corrode side is unchecked → **this document does not use that figure.**

**Source-bias caution (collector's self-report):** corrode.dev enumerates none of the downsides of workspace splitting, so it was treated as an advocacy document, and the Nx·Turborepo docs are vendor documents with a stake in package boundaries.

## Disputes · limits · gaps

- **Parnas primary text unchecked** — see above. Do not quote conclusion 1 of this document without re-checking.
- **There is no A/B measurement on the same codebase of 「한 큰 crate + incremental」 (one big crate + incremental) versus 「쪼갠 워크스페이스」 (a split workspace).** Feldera is generated code and Anderson gives only the mechanism. The biggest evidence gap in the Rust compile-time section.
- **The current (2026) state of the rustc parallel front end is unknown** — only the 2023-11 nightly opt-in announcement was confirmed. Whether it is stabilized or the default is unchecked.
- **Whether the workspace feature-unification opt-out is stabilized is unknown.**
- **No primary source was found from a named engineer who states "hold with folders + boundary lint and split at point X"** — several people claim the pattern, but not in an attributable form.
- **Literature that treats the DDD bounded context as a split criterion (Evans·Vernon) was not looked at** — the collector itself pointed to it as "the biggest missing axis".
- **No authoritative source was found that treats security·blast radius as a split axis.**
- **Parts of the Nx docs and the Turborepo JIT vs precompile comparison page did not render**, so that trade-off is unchecked.

---

## 9. Adversarial review results (cross-family · codex, effort=high, web_search on)

Reviewer's verdict: **"Overall soundness is low — individual quotes are mostly accurate, but several load-bearing conclusions are refuted by primary-source counterexamples."** 15 findings extracted. Below is the result of the main session verifying each one and adopting, rejecting or adjusting it (the reviewer's verdict is not transcribed as is).

### Adopted — the body is corrected

**A. In the §1 table, discard the three rows 「독립 스케일링·장애 격리·기술 이종성」 (independent scaling · failure isolation · technology heterogeneity).** (severity high · logic error)
Those three are properties of an **out-of-process service boundary** and do not arise from splitting a library. The Grzybek·Newman driver tables are about microservices, so pulling them as is into the lib-split question is a category error. → **For the lib-split question this axis drops out entirely.**

**B. ★The biggest correction — the first item of §2 (SWE at Google ch.21) hardly applies to splitting inside a monorepo.★** (severity high · overstatement)
Main-session re-check (primary text directly): that chapter defines the boundary as **"does our organization control the development, updating and management of this sub-project"** (back-translated), and says "if every team in the company has a **separate repository**, goals and development practices, their interaction is closer to dependency management than to source control" (back-translated), and "a large organization with a (hypothetical) single repository **can scale much further with source-control policy — this is Google's way**" (back-translated). That is, the quote is a statement that separates **monorepo vs multirepo**, and it is **not cost evidence for splitting crates inside one monorepo.** There are no versions and no releases, so the dependency-management tax does not arise. → **Remove this item from the 「쪼개지 말라」 ("do not split") side of the argument.**

**C. Narrow the wording of the TS verdict in §5.** (severity high · overstatement)
"Even splitting into packages does not enforce the boundary" is too absolute. The same Node doc says an **unexported bare subpath is rejected with `ERR_PACKAGE_PATH_NOT_EXPORTED`** and that `exports` gives tools and SemVer a trustworthy package interface guarantee. → The accurate proposition: **it is enforced on the bare-specifier axis and not on the relative·absolute-path axis.** The practical conclusion (lint is needed) stands — the basis is the Turborepo maintainer's "same filesystem, so you can easily leave the package root with `../../`" (back-translated).

**D. Distinguish crate · Cargo package · workspace.** (severity high · logic error)
The body blurred the three. Precisely — **visibility·compilation·coherence belong to the crate**, **manifest·version·publish·resolver choice belong to the package/workspace**. One package can have several targets (= crates).

**E. The 「강제 분할」 (forced split) axis is missing — in Rust there are cases where the language requires crate separation.** (severity high · omission)
A proc-macro can be defined only in a `proc-macro` crate and cannot be used inside the crate that defines it. `#![no_std]` applies to the whole crate — if you need partial no_std you must split. → **Add a 「언어가 강제」 (enforced by the language) row to the §1 table.**

**F. Narrow the scope of acyclicity enforcement.** (severity medium · confident-wrong)
Main-session re-check (Cargo docs primary text): "**the resolver generally does not allow cycles in the graph, but it does allow them for dev-dependencies.**" (back-translated) → The accurate proposition: **only the acyclicity of the production dependency graph** is enforced by the compiler·resolver.

**G. 「독립 CI 범위」 (independent CI scope) is not a package-specific benefit.** (severity medium · confident-wrong)
CI can pick workflows with a **directory path filter** without package metadata. → Remove this item from the package-only benefit list in §5. (What remains is the tool's cache·affected granularity, and that requires a "project the tool recognizes" unit.)

**H. Fix the cross-crate inlining wording.** (severity medium · confident-wrong)
"Recovered only by `#[inline]` or LTO" is too strong. **A generic function is monomorphized in the consuming crate, so its body can already be inlined**, and the current rustc also has a path that infers cross-crate inlinability for simple functions without the attribute. `#[inline]` itself is a hint, not a guarantee.

**I. Fix the feature unification wording.** (severity medium · overstatement)
"Not avoidable among workspace members" is too strong. resolver 2 already avoids some axes (target·build·proc-macro·dev), and **nightly `-Z feature-unification`** can build different feature sets in duplicate within one invocation in a per-package mode. (Whether it is stable is still unchecked.)

**J. The inherent impl constraint is missing from the Rust cost of splitting.** (severity medium · omission)
An inherent `impl` must be **in the crate where that nominal type is defined**. If you move a type to a separate crate, the consuming crate cannot add methods directly and must redesign with traits·free functions·newtypes. ★It bears directly on our case — if `layout/types.rs` is moved into a crate, the shell cannot attach inherent methods to those types.★

**K. The non-structural alternatives to the Nx criterion are missing.** (severity medium · omission)
A test bottleneck is a signal to split, but it is not the only response — there are Nx's own task decomposition, and Jest `--shard` and changed-only runs.

**L. Qualify the pipelining figure.** (severity low · overstatement)
The 10–20% of Rust 1.38 is a report limited to **the optimized clean build of some crate graphs**, and the announcement states explicitly that it has no effect on a single crate and no improvement on some graphs.

### Adjusted — adopted in a narrowed form

**M. "To publish to external consumers, a separate package"** — the reviewer saw it as confident-wrong (severity high). **Partially adopted:** it is true that one package can offer several external entry points through subpath exports. But the reviewer himself admits "independent versions·installation need a separate package", so if the original proposition is narrowed to **「독립 버전·릴리스 케이던스」** (independent version · release cadence) it holds. The severity is lowered to the level of wording precision.

**N. Parnas — the premise is resolved, the inference is labeled as mine.** The reviewer reached the primary text and confirmed: Parnas defines a module as a **responsibility assignment, not a subprogram**, and actually lists three expected effects (shorter development time · flexibility · comprehensibility) → **the grounding "unchecked" is resolved into supported.** However, since he **does not compare** folder boundaries with compilation/package boundaries, "therefore a boundary inside a unit is enough" is **my inference**. It is labeled as such.

### Two open items the review resolved

- **MacCormack 6x vs 8x = a version difference.** The HBS working paper (08-039) says 8x, the final *Research Policy* 2012 edition says 6x. → Adopting 6x in the body is correct. Promoted to **certain**.
- **The 2m22s → 26.63s figure is real but is not a crate-split case** — it is a case of replacing heavy dependencies such as clap·lalrpop·regex, reducing dependencies from 105→22. → The judgment not to use this figure was right.

### Net effect

**The direction of the conclusion does not change** — 「별도 유닛이 있어야만 얻는 것은 짧은 목록이고 대부분은 유닛 안 경계로 얻는다」 (what you get only with a separate unit is a short list and most is obtained with boundaries inside a unit) stands. What changed is **the contents of that list and the weight of the counter-argument**:

- **Dropping out of** the list = independent scaling·failure isolation·technology heterogeneity (the service axis) · independent CI scope (a path filter does it)
- **Coming into** the list = cases enforced by the language (proc-macro · crate-level attributes)
- **Narrowed in** the list = acyclicity (production dependencies only) · publishing (to independent version·release cadence) · build time (only when the graph gets wider, no effect on a single crate)
- ★**One counter-argument got weaker**★ — the dependency-management tax hardly applies to splitting inside a monorepo (B above). So the basis for 「쪼개지 말라」 ("do not split") shifts toward **concrete language costs** (pub surface · inherent impl · orphan rule · moving the generated-output gate) and **failing the criteria** (number of consumers · test bottleneck · graph width).
