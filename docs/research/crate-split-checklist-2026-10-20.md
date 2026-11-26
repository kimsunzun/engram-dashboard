# Crate split decision table — one page + checked against S25 `library`

| | |
|---|---|
| Status | Draft · not reviewed. Decision table D steps 4 · 5, the 「실재로 세는 조건」 (conditions that count as real) field, and the 「이 표」 field of the 「두 조사가 갈린 자리」 (where the two surveys disagree) table are **judgment (not the main session — this document)**. The other fields carry the cited sources over |
| Date | 2026-10-20 (HEAD `19d6b638`) |
| Method | Read the two surveys · ADRs (0110 · 0129 · 0130 · 0151 · 0175 · 0236 · 0251 · 0266 · 0269 · 0273) · CLAUDE.md closely and built the decision table. Held that table against the S25 TRD and cross-checked the TRD wording against a few places in the code. 0 web searches · 0 build · test time measurements |
| Question | Make the user's criterion 「개념적으로 분리할 수 있으면 분리한다. 다만 분리에 대가가 크면 합친다」 (split it if it can conceptually be split, but merge if the split costs a lot) (2026-10-20) answerable yes/no by anyone. By that criterion, is S25's `library` a crate or a daemon module |
| Input | **Survey 1** = [crate-granularity-survey-2026-08-17.md](crate-granularity-survey-2026-08-17.md) · **Survey 2** = [module-split-criteria-2026-08-26.md](module-split-criteria-2026-08-26.md) (the body is cited after the §9 correction). The head of survey 2 left 「두 기준을 하나로 합치는 작업은 미해소」 (merging the two criteria into one is unresolved), and this document does that work |

## Conclusion

- **Decision order = 0 language-forced → A can it be split → C is the cost structural → B is there anything bought.**
  - If A is all 「예」 (yes) and the cost is only the fixed part (manifest · gate registration · public surface), split it off.
  - If the cost involves structural change (moving types · ports · an adapter layer), split it off only when there is a real gain.
- **`library` leans toward being split off (confidence: medium).** There is one condition — first move `validate.rs`'s backend knowledge behind the backend port. The review demanded that anyway. After that, the outside contact points are only the public APIs of other crates and paths that can be passed by value, so only the fixed part remains as cost.
- **The TRD's rejection reason 「소비자가 하나다」 (there is one consumer) is a criterion that ADR-0151 discarded on 2026-08-17.**
- ★It diverges from memo 96 (user decision 2026-10-14 「애매하긴 하지만 그렇게 해」 (it is ambiguous, but do it that way) — daemon module)★ — this document does not overturn that decision but raises it. The user decides.

## 1. Decision table

### 0. Is it language-forced

- If a proc-macro or a crate-level attribute (`#![no_std]` etc.) is needed, split it off. It ends here (survey 2 §9 E · survey 1 F7).

### A. Can it be split — conceptual independence (host = the crate it lives in now)

| # | Question (yes/no) | Source |
|---|---|---|
| A1 | Can it be used and tested without the host's other modules? Plugging in fakes or injecting values is allowed | MS 「if a submodule can be used independently」 — survey 1 F2 · ADR-0151 decision 1 |
| A2 | Does it avoid calling the host's non-public items or other modules? If it uses only paths · settings that can be passed by value, 「예」 (yes) | ADR-0175 rationale (「나가는 참조 0」 (outgoing references 0) measured) · ADR-0130 rationale ③ |
| A3 | Is there no mutual-calling relationship (module cycle) with the host? | ADR-0175 rationale (lifetime · pump ↔ transport ↔ backend is a cycle, so it stays in one crate) · survey 1 F6 (ruff's Checker ↔ rule) |
| A4 | Are the outside contact points only ports (traits) · public types · values? | ADR-0110 decision 3 (the lib owns the contract, the host owns the real adapter) |
| A5 | Is there no mixed-in role of someone else (knowledge whose canonical home is elsewhere)? If mixed in, move it to its proper place first | CLAUDE.md 「백엔드 확장」 (backend extension) · ADR-0266 decision 3 |

- **What is not asked** — how many consumers there are now (ADR-0151 decision 1 · survey 1 F7), how many lines (ADR-0151 decision 3 · survey 1 F1).

### B. What is gained only by splitting — counted only when the 「실재」 (real) condition is met

What is gained by a boundary alone is not counted. Information hiding · ease of understanding · isolated testing through a seam · ownership fall under this (survey 2 conclusion · §1 · §9 N).

| # | Gain | Condition for counting as real (실재로 세는 조건) | Source · precedent in this repository |
|---|---|---|---|
| B1 | An outside wall counted by the compiler — only declared dependencies can be used, and the production dependency graph is acyclic | The edge to block can be pointed at by name (an erosion history inside the host · a module that would be dragged along) | Survey 2 §4 · §9 F · survey 1 F4 (also stands as a gate) · messaging (ADR-0110 「약속이 아니라 구조로」 (by structure, not by promise)) · net (ADR-0129) |
| B2 | Another crate · exe that wants only this does not carry the host's entire dependencies | A named consumer exists now, or is in a started plan | Survey 2 §9 B · M (reuse inside a monorepo = only a smaller dependency closure) · net (ADR-0130 ① — closure 122→110) · cli (ADR-0273) · shell (ADR-0270) |
| B3 | The test build does not go through the host (TDD loop) | There is a measurement that the host's test loop is slow | Survey 2 §3 Nx 「Split when tests slow you down」 · §1 (isolation itself is the seam's role) |
| B4 | Parallel · incremental compilation unit | The graph gets wider (if it is an A→B→C chain, 0) | Survey 1 F3 · F6 (ruff) · survey 2 §4 · §9 L |
| B5 | Independent version · release | It is published | Survey 2 §9 M — not in this repository |

### C. Cost — how to count it in this repository

**Fixed part — almost the same for any crate**

| # | Cost | Source |
|---|---|---|
| C1 | A new `Cargo.toml` and one line in the root `members` are needed. Give the name the `engram-dashboard-` prefix — if violated, the cap gate silently fails to see that crate. The version gate looks only at exe packages, so for a lib it is 0 | ADR-0151 「개명 함정」 (rename trap) · ADR-0273 decision 2 · `ci.yml:1468-1469` |
| C2 | Register the gates. Three places per kind (`ci.yml` · CLAUDE.md 「빌드·검증 명령」 (build · verify commands) · `.claude/skill-bindings/qa.md`).<br>- Minimum in the precedents = two (test line + dependency cap — command). Usually three to five (messaging 3 · base 5).<br>- The crate names hard-coded in the messaging isolation regex also get one more.<br>- Upkeep — these gates move together on a rename · move. One rename changed 12 lines of qa.md | ADR-0175 impact · ADR-0266 gate · CLAUDE.md messaging isolation line |
| C3 | One entry in the CLAUDE.md 「백엔드 모듈 맵」 (backend module map) | Same practice in the same section |
| C4 | What the host calls becomes `pub`. The rest stays private | Survey 2 §2 (a language mechanism, so certain) · survey 1 F6 (no actual damage case found) |
| C5 | (Optional) handing over test helpers brings in a `test-support` feature and an operating graph gate (three checks expecting 0 + a counterpart check) | base · platform precedent (CLAUDE.md) |

**Structural part — if any one is present, 「대가가 크다」 (the cost is large)**

| # | Cost | Source |
|---|---|---|
| C6 | Move types · ports to break a cycle, or build a new neutral type | Survey 1 F6 (ruff) · ADR-0110 decision 5 (moving the envelope layer) |
| C7 | Write a new adapter layer on the host side | ADR-0110 decision 3 |
| C8 | Have to work around a language constraint — an inherent impl cannot be attached to another crate's type, and the orphan rule applies | Survey 2 §9 J · §4 · survey 1 F6 |
| C9 | User veto — do not make a one-file lib. Keep libs similar in weight | ADR-0175 decision 6 |

- **What is not counted** — the version · dependency management tax hardly applies inside a monorepo (survey 2 §9 B). Feature unification is a cost that is not yet established (survey 1 F6 · survey 2 §9 I).

### D. Decision rules

1. **If 0 applies, split it off.**
2. **If A5 is 「아니오」 (no),** first move the mixed-in role to its proper place and measure again.
3. **If any one of A1–A4 is 「아니오」 (no), keep it as a module inside the host** (ADR-0151 decision 2). If C9 applies, it is a module too.
4. **If A is all 「예」 (yes) and the cost is only the fixed part (C1–C5), split it off** (ADR-0151 decision 1 · user 2026-10-20). B is used only to record 「무엇을 사 오나」 (what is being bought).
   - ★Judgment (not the main session — this document): the fixed part is counted as 「크지 않은 대가」 (not a large cost)★ — it is nearly the same mechanical work for every crate, so if 「대가가 크다」 (the cost is large) stood on that, no split would ever stand.
5. **If A is all 「예」 (yes) but the structural part (C6–C8) is involved, split it off only when B has at least one real gain.** If there is none, keep it as a module but attach an outside-edge test — so that the price of splitting later does not grow. Measure again when that structural change comes on its own reasons (CLAUDE.md 「판단 기준」 (decision criteria) risk × duration). ★Judgment (not the main session — this document)★
6. **If it is decided to split, the time of creation is cheapest.** Splitting later adds fixing the import paths of incoming edges and rearranging the places where the host's tests reach into neighboring modules (ADR-0130 rejected alternatives option A · impact 「테스트 코드의 옆걸음」 (the sidestep in test code)).

### Where the two surveys · ADRs disagree, and how this table resolves it

| Issue | One side | The other side | This table (이 표) |
|---|---|---|---|
| Is enforcing direction a reason to split | Survey 1 F4 · ADR-0151 decision 4 — it stands even without a crate through `pub(in path)` · rule tests · rg/cargo tree gates | Survey 2 §4 — if you use the compiler as the enforcer, it is a valid reason. ADR-0110/0129 — the user rejected the option of having only an rg gate as 「약속」 (a promise) | Not used as a basis for A (decision 4). Counted only as a B1 gain. Condition for real = an erosion edge that can be pointed at by name |
| Reuse · number of consumers | Survey 2 §1 · §3 — REP 「재사용 단위 = 릴리스 단위」 (reuse unit = release unit) · the third-user rule | Survey 1 F7 · ADR-0151 decision 1 — there is no consumer-count rule and there is a counterexample (rust-analyzer `ide`) | Not asked in A (can it be done). Named consumers are counted only in B2 (what is bought) |
| Compile time | Survey 1 F2 — MS 「dramatic compile time improvements」 | Survey 1 F6 · survey 2 §4 — conditional. If it is a chain it is 0, and there is no controlled A/B measurement | B4 is counted only when the graph gets wider. If unmeasured, counted as 0 |
| Isolated testing | Survey 2 §1 — it is a seam problem, not a unit problem | Survey 2 §3 Nx — if tests are slow, split. Memo 96's reopening condition 「isolated testing」 | Tests that run on fakes are the boundary's role, so not counted. Only the test build scope is counted as B3 (when there is a measurement) |
| Size | ADR-0151 decision 3 · survey 1 F1 — size is not an axis | ADR-0175 decision 6 — no one-file lib · keep weight similar (user) | Do not split by size. Decision 6 is used only as C9 (veto). ★Both ADRs are alive, and neither has a mark of amending the other★ |
| Dependency management tax | Survey 2 §2 (SWE at Google) | Survey 2 §9 B — hardly applies inside a monorepo | Not counted |

## 2. Checked against library

Target = the daemon `library` module of TRD (`docs/process/S25-preset-library/trd.md`) §1-4, fourteen files. Host = the daemon crate — `src` 39 files · 41,610 lines (including tests · `wc -l` measured).

### A

| # | Answer | Basis |
|---|---|---|
| A1 | Yes | The store is swapped through the `LibraryStore` port + a fake `MemStore` (§1-4 `store.rs`). Resolution is a pure function (§1-3). The profile registry plugs in a fake through `ProfileRegistry::new(Arc<dyn ProfileStore>)` (`agent/src/profile.rs:377`) |
| A2 | Yes (value-injection condition) | What the daemon side uses is three — the state · cache · launch paths that `DataLayout` (`daemon/src/data_dir.rs:207`) produces, `suggested_new` (§1-4 「루트 · 기본 경로」), and the install location (`builtin\` — review #4). All can be passed by value. Plugging in the bus is handled by `blocking_handler` (`command/src/table.rs:365`) of the command crate. Moving `SpawnByCwd` to `spawn_blocking` is a fix on the daemon side |
| A3 | Yes | Only one direction, daemon → library. Since there is no broadcast or subscription (§0 · §1-4 flow ①), the TRD has no edge going back to `status_fanout` · `connection_core` |
| A4 | Yes | There are four outside contact points — two agent ports (`LaunchKitSource` · `LibraryCommandHost` — §1-5 · `trd.md:253`), agent types (`AgentProfile` · `ProfileRegistry` — the profiles input of `catalog.rs` · `ops.rs` (e)), platform `fs` (§1-6), command. All are public APIs of other crates. A precedent of the same shape = the daemon's `usage_service` implements `UsageCommandHost` (`agent/src/commands.rs:462` · ADR-0251) |
| A5 | **No** | `validate.rs` (§1-4) knows the claude schema source · the codex `-c` dotted-key resolution · `--version`. Review #2 (both reviewers agreed) saw this as a 「백엔드 확장」 (backend extension) violation and said to move it behind the backend check port. **It is work that has to be done whether or not it is split off** |

### B

| # | Decision | Basis |
|---|---|---|
| B1 | Possible | `control/` in the same host, which was a leaf, stopped being a leaf only two days after it was written as a leaf. After that the reopening trigger fired for over a month with no judgment (ADR-0236 context · rationale). There is also an erosion history of the daemon (ADR-0129 rejected alternatives → ADR-0020). But **the leaking path the review actually found (A5) is knowledge duplication, not symbol references, so the crate wall cannot stop it** |
| B2 | None | The shell does not read the disk (PRD invariant constraint · §1-1 rejected alternatives). cli only does authentication and delivery (ADR-0285) |
| B3 | Not measured | If split off, the library test loop does not go through the daemon crate and the dependencies only the daemon carries (rmcp · axum · net · messaging — `daemon/Cargo.toml:92-149`). It still goes through agent |
| B4 | 0 | It is an agent → library → daemon chain |
| B5 | 0 | Not published |

### C — shape of the split = 「얇은 library crate」 (a thin library crate)

The thin library crate depends on agent · platform · command · base, and the daemon depends on it.

| # | Count |
|---|---|
| C1 | Manifest 1 + `members` 1 |
| C2 | Three required × three places = 9 places: test line · dependency cap (cli-style name set) · messaging isolation regex name. Adding two optional makes at most 15 places — `use tauri` 0 (it is linked into the daemon, so the base · platform precedent), staying synchronous (platform gate 7 shape · the TRD uses `std::sync::RwLock` sync port) |
| C3 | 1 |
| C4 | Estimated 5–8 (not implemented): `LibraryService` constructor · two boot hooks (journal roll-forward · `launch\` sweep) · trash-emptying entry point (review #3) · path bundle · settings · identity type |
| C5 | 0 is possible — if the daemon tests use a real `FsStore` + a temporary folder, no feature wiring is needed |
| C6 | 0 — agent does not depend on library, and the ports are already in agent. There are no types to move |
| C7 | 0 — the port implementation (`impl LaunchKitSource for LibraryService`) stays inside library. It attaches someone else's trait to its own type, so the orphan rule does not apply either |
| C8 | 0 — not being able to attach an inherent impl to an agent type is the same even if it is a daemon module |
| C9 | Passes — fourteen files |

**Cost = only the fixed part** (9–15 registrations + 3 manifest · map entries + 5–8 public items). The A5 fix is not a cost of the split.

### The TRD's rejection reason

- `trd.md:49` says 「소비자가 하나다. 다른 crate 가 필요해지면 뺀다(메모 96)」 (there is one consumer. If another crate needs it, pull it out (memo 96)), and `trd.md:253` says 「라이브러리 전용 crate(소비자 하나)」 (a library-only crate (one consumer)). Both are criteria that ADR-0151 decision 1 (2026-08-17) discarded (ADR-0130 reopening condition ①).
- Memo 96 (`.claude/handoff/attachments/20261012-preset-decisions.md:96`) has three reopening conditions, but the TRD carried over only the first.
  - Another crate becomes necessary — a discarded consumer criterion.
  - The preset logic grows — size. ADR-0151 decision 3 says size is not an axis.
  - Wanting isolated tests — only this one of the three is still alive, as B3.
- The TRD review saw 「crate 배치」 as clean (end of `20261020-trd-review-findings.md`). There is no trace of measuring it against the decision criteria.

## 3. Conclusion + confidence

- **Recommendation — D step 4: as a thin library crate, split off at creation. Before that, first fix A5 (check knowledge to the backend port).** Confidence = **medium**.
- **Two facts that decide it**
  1. If A5 is fixed, the TRD's outside contact points of library are only the public APIs of other crates and paths that can be passed by value. So only the fixed part remains as cost (§2 C6–C8 = 0).
  2. Now is the cheapest. There is not yet an edge from the daemon into library (D 6). As with `control/` in the same host, if edges pile up, the price of splitting later grows (ADR-0236).
- **To be honest — what is bought is also small.** Among B there is no item settled as 「실재」 (real) (B1 possible · B3 not measured · B2 · B4 · B5 = 0). The result 「뗀다」 (split it off) is not the consequence of a measured gain but the consequence of following 「대가가 고정분이면 뗀다」 (split it off if the cost is a fixed part) (ADR-0151 decision 1 · user 2026-10-20). Memo 96 (a user decision) is on the opposite side, so it is a matter for the user to confirm.
- **What would overturn it (→ daemon module)**
  - Implementation reveals that library has to call daemon internals (`connection_core` · `command_delivery` · `control`) — A2 becomes 「아니오」 (no).
  - The fix for review #6 requires the agent → library notification in a non-port shape — C6 arises. Review #6 = the problem that duplicate · create · delete do not go through the library, so the reference index goes stale.
  - The user reconfirms memo 96.
- **If it goes as a module**, attach an outside-edge test per D 5 — edges from `library/` to other top-level daemon modules = 0. Module names are derived from the `mod` declarations in `lib.rs` (the command shape of ADR-0130 rationale ③). The history of false positives · 0-line false passes of that command is in ADR-0236 「영향」 (impact).
- **What would not overturn it** — number of consumers, number of files.

## 4. Limits

- Build · test time measurements are 0. B3 · B4 were inferred from mechanism. According to survey 1 F6, there is no controlled A/B measurement anywhere.
- library is not yet implemented. The number of public surface items in C4 and the 「데몬 내부 0」 (daemon internals 0) in A2 are based on the TRD wording.
- No web search was done. The cost axis was counted only from the two surveys and this repository's ADRs.
- The 「한 종류 × 세 곳」 (one kind × three places) in C2 is a convention written in ADR-0175 · ADR-0266. Whether every precedent crate is registered in all three places was not checked one by one — only the number of name mentions was looked at with `grep -c`.
- Decision table D 4 · 5 is a synthesis of this document and has not been through review.
- Side fact: the workspace member `engram-dashboard-transport` (`members` of the root `Cargo.toml`) is neither in CLAUDE.md 「백엔드 모듈 맵」 (backend module map) nor in the test line of 「빌드·검증 명령」 (build · verify commands).

## 5. Addendum — criteria that well-divided open source wrote down (2026-10-20 · light · 1 collector + main grounding)

- **rust-analyzer** (main confirmed the original — https://raw.githubusercontent.com/rust-lang/rust-analyzer/master/docs/book/src/contributing/architecture.md): the reason to split a crate off is consistently **「이 덩어리는 무엇을 모르는가」** (what this chunk does not know) — 「syntax crate is completely independent from the rest of rust-analyzer. It knows nothing about salsa or LSP.」 · 「`base-db` knows nothing about cargo.」 and it marks **boundary crates** (`syntax` · `hir` · `ide` — 「API Boundary」) and **internal crates** (`hir-def` · `hir-expand` · `hir-ty` — 「these crates are not, and will never be, an api boundary」) separately. An internal crate is also a crate — 「경계가 아니라 마음대로 고친다」 (it is not a boundary, so edit it freely) is only a marker. → This is on the same axis as this table's A5 (is other knowledge mixed in) · B1 (leaking edges to the outside, by the compiler). Confidence = certain (original).
- **tokio**: types that will be exposed in another crate's public API are put out into a lower crate (tokio-util etc.) — search summary only (likely).
- **zellij · herdr (local clone)**: nowhere states a crate criterion. herdr `AGENTS.md` has only a module principle — 「State is separated from runtime. AppState is pure data, testable without PTYs or async.」 (certain — read directly).
- **Bevy**: merged and removed `bevy_core` in 0.16 (migration guide title — certain) · the reason was not found.
- **Could not read**: Zed · ruff/uv · Deno · Nushell · Helix · wezterm · Cargo · Servo. This time we found no place that **wrote down** compile time · cycles · feature isolation as crate criteria.
- **User decision history (main confirmed)**: the user decision of 2026-10-14 (「애매하긴 하지만 그렇게 해」 — `.claude/handoff/attachments/20261012-preset-decisions.md:96`) = a dedicated **module** inside the daemon + a trait seam from the start · 「다른 crate 가 필요하거나 · 프리셋 로직이 커지거나 · 따로 시험하고 싶어지면」 (if another crate becomes necessary, or the preset logic grows, or separate testing is wanted) pull it out as a crate. This document's lean toward 「떼는 쪽」 (splitting off) differs from that decision, so it is a matter for the user to decide again.
