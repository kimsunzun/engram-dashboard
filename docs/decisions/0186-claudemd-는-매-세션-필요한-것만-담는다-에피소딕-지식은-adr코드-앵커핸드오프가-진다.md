# ADR-0186: CLAUDE.md holds only what every session needs — episodic knowledge is carried by ADRs, code anchors and handoffs

- 상태: 확정 (2026-09-09, basis: user decision — 「이거 이번에만 필요하고 다음번엔 필요 없는 거 아님?」)
- 관련: Amends ADR-0185 (the clauses in two principle documents) · the CLAUDE.md preface (「기조만 담는다 · 상태는 docs 에서 본다」) · ADR-0197 (per-topic detail documents — the split of canonical sources in 「영향」 below) · ADR-0008 (the plain-text home of the ban on extending the tracking file) · ADR-0082 (fresh fallback abolished)

## 맥락
ADR-0185 decision 3 was 「원칙 문서 두 곳(CLAUDE.md 「세션 복원」 · architecture-overview)의 첫 문장을 고친다」 (fix the first sentence of two principle documents: the CLAUDE.md section on session restore and architecture-overview), and how to write that CLAUDE.md sentence split the adversarial review in round 2 (cut-advocate 「그 절 자신이 되올리기를 금하므로 지워라」 vs load-bearing defender 「문구를 고쳐 남겨라」).

**The user asked a question that voids that dispute:** 「이게 CLAUDE.md 에 있어야 되냐? 이번에만 필요하고 다음번엔 필요 없는 거 아님?」 — the issue was not the axis being argued over (how to write the sentence) but **whether the container is right**.

**CLAUDE.md is loaded automatically every session.** Its cost is charged equally to sessions that never touch resume.

★**And this section had been growing — measured**★: the 「세션 복원」 section in `git show c8c7c7d~1:CLAUDE.md` was **754 bytes**, and the commit `c8c7c7d` that reflected ADR-0185 **grew it to 1304 bytes** (adding the codex issuing description + the wiring trap). So what this ADR deletes is not an old legacy but **what the previous commit grew**, and that growth itself was a signal that the container was wrong.

## 결정
1. ★**The criterion for inclusion in CLAUDE.md = 「매 세션이 필요한가」 (does every session need it)**★. It is not 「규칙이냐 사실 서술이냐」 (is it a rule or a statement of fact) — even a rule is not for CLAUDE.md if it is needed *only when touching that code*. The allocation of containers:
   - **Needed only by this task** → handoff · that phase's TRD
   - **Needed only when touching that code** → ADR + a `// ADR-` anchor in the code + a regression test
   - **Needed every session** → CLAUDE.md
2. **Delete the 「세션 복원」 section from CLAUDE.md in full** — the amount removed is **1304 bytes at HEAD (`c8c7c7d`)** (heading included). The file goes from 54202 to 53203 bytes.
3. **References that pointed at CLAUDE.md are turned toward the canonical source** — that those references pointed at CLAUDE.md was itself a wrong aim.

## 거부한 대안
- **Keep the 1304-byte original** — review round 2 kept putting the same item at priority 1, and above all the section **itself** had written 「매핑 근거·예외·혼동쌍은 여기 되올리지 않는다」 while containing all three.
- **Shorten to about 480 bytes** (leaving only 2 bans + the ban on re-raising) — this is the product of the 「규칙만 남긴다」 (keep only the rules) criterion, and **today it actually went as far as that shape once in the working tree (it was never committed)**. We discarded it because the criterion was wrong: even the two bans that remained are needed *only when touching resume*.
- **Reduce to a pointer line of about 200 bytes** — the session once recommended this, and the reason was 「들어오는 참조 5곳이 빈 이름을 가리킨다」. ★That is not a reason to keep the section but **a reason to fix the references**★ (decision 3). Leaving a pointer makes permanent the wrong aim that 「CLAUDE.md 가 이 주제의 입구」 (CLAUDE.md is the entrance to this topic).
- **Deciding the container by 「기능이냐 규약이냐」 (feature or convention)** — this was the criterion the session first wrote. The ban on reviving fresh fallback is clearly a convention, yet it does not earn a place in CLAUDE.md. This counterexample shows that the axis is not nature but **frequency**.

## 근거
### Every deleted item has a living home (exhaustive — we pin file:line so the next session can reproduce it)
| # | The deleted statement | The living home |
|---|---|---|
| 1 | Invariant (depends solely on the stored backend sid · the issuer is the backend) | `docs/reference/architecture-overview.md:321` |
| 2 | ★Ban: **no feature extension** through the tracking file★ | **`docs/decisions/0008-세션복원-sid-통제.md:22`** (plain-text ban) · the grade fact is in the `crates/engram-dashboard-agent/src/session_tracker.rs` header |
| 3 | Warning: do not put the issuer into the invariant | `architecture-overview.md:321` (★no code anchor — unmet, see 「영향」 below★) |
| 4 | Fact: claude issues / lossless · codex is issued | `architecture-overview.md:321` |
| 5 | Trap: no wiring · 0 wire markers · the entrances do not check `needs_session()` | `architecture-overview.md:323-325` (more detailed) |
| 6 | ★Ban: no reviving fresh fallback★ | `architecture-overview.md:328` + **regression test `activate_resume_early_exit_ends_failed_no_fresh_fallback`** (`crates/engram-dashboard-agent/tests/activation.rs` — find it by name, not by line number) |
| 7 | Confusable pair: the endpoint is `Exited`, not `Failed` | `architecture-overview.md:329` · the `ADR-0082` status line · the ADR index row for 0082 |
| 8 | Ban on re-raising | **Promoted to a general rule by decision 1 of this ADR** (it went away together with the old section) |

★**「전부 architecture-overview 에 있다」 (It is all in architecture-overview) is false**★ — the plain-text bans of 2 and 6, and 8, are not in AO. That is why the table above writes the homes **per document**. What the session once wrote as 「7개 주장 전부 AO 실재」 (all 7 claims exist in AO) was the result of grepping *facts*, not *bans*.

### Why deleting is fine
- **Ban 6 is carried by a test, not a sentence** — if someone revives fresh fallback, that test turns red. A sentence can be ignored, but a red test cannot.
- **The plain text of ban 2 is in ADR-0008** — ★do not read the 「그뿐이다」 in the `session_tracker.rs` header as its ground★: that closes the question of *what the module owns*, and the ban that file writes is narrower (「정확성의 1차 근거로 삼지 말 것」).
- **What surfaced this invariant yesterday was not CLAUDE.md but the handoff** — the previous `latest.md` had written 「우리 「세션 복원」 원칙은 …글자 그대로면 안 맞는다」 (confirmed by the removed lines of `git diff`). ★But what this evidence supports is 「핸드오프가 표면화했다」 (the handoff surfaced it), not 「CLAUDE.md 가 기여하지 않았다」 (CLAUDE.md did not contribute)★ — that handoff wording also matches the phrasing at `ADR-0008:19`.
- **One quantitative ground is a value from outside the repo** — 「CLAUDE.md 가 매 세션 약 25k 토큰」 (CLAUDE.md is about 25k tokens every session) is the `/context` value in the user's terminal and cannot be reproduced from repository artifacts. What is reproducible is only the file size (decision 2 above).

## 영향 / 불변식
- ★**Before setting up a new section in CLAUDE.md, run it through 「매 세션 필요한가」 (does every session need it) first**★. If it does not pass, send it to an ADR + a code anchor (+ a regression test if possible).
- ★**The split of canonical sources — read it so it does not conflict with ADR-0197**★: ADR-0197 rejected 「조감도에 상세를 밀어 넣는다」 (push detail into the overview) and pinned per-topic detail to `docs/reference/structure/<주제>.md`. The scope in which this ADR calls `architecture-overview` canonical is **the invariants and state mapping of the resume/activation axis**, and **the per-backend table remains the share of `structure/agent-backend.md`**. So this change also attached a pointer for this axis to that file — if not attached, a newer document explains this axis with the pre-revision invariant.
- ★**The container allocation this ADR set up is still unmet for the content it moved**★ — `rg "ADR-0185|ADR-0186" crates/ src/ src-tauri/` = **0 lines**. That is, the issuer-neutral invariant now has **only an ADR + documents**, and **neither a code anchor nor a regression test.** This means that, of the two discovery surfaces named by CLAUDE.md 「설계 결정 기록」, the anchor side is empty, and filling it is Phase 2's share (the same item in ADR-0185 「영향」).
- **Two gaps remain.**
  - **The design document surface** — a regression test catches an *implementation* and cannot catch a *proposal*. If the next session writes 「resume 실패하면 새로 파자」 in a TRD, nothing turns red.
  - **Missing anchor** (the item above) — the only surface where a session touching the code meets the revised invariant is the documents.
  This ADR **closes neither.**
- **The claude and codex runtime behavior does not change by even one line** — it is a decision about document placement. Decisions 1 and 2 of ADR-0185 remain valid, and what this ADR amended is **only the 「CLAUDE.md 도 고친다」 (CLAUDE.md is fixed too) half of decision 3**.
- ★**The remaining full sweep is out of this ADR's scope**★ — running the same criterion over the whole of CLAUDE.md gives several sections the same verdict. **The closest case is 「핵심 불변식」, the parent of the deleted section** (kill causality · finalize once · lock order · registration order) — those ten lines are also all 「그 코드 만질 때만 필요」 (needed only when touching that code). Next come 「백엔드 모듈 맵」 and 「빌드·검증 명령」. They are set up as a separate item.
