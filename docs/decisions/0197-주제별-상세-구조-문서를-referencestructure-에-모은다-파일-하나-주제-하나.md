# ADR-0197: Collect per-topic detailed structure documents in reference/structure/ — one file = one topic

- 상태: 확정 (2026-09-08, basis: user decision — reusable parts are needed)
- 관련: ADR-0034(document architecture — this decision adds one more document kind under that frame) · `docs/README.md` 「고아 금지」 · `docs/reference/structure/` · first occupant = `docs/reference/structure/agent-backend.md`

> Numbering history: it was originally numbered `ADR-0179`, but at merge time it came to light that a parallel branch had used the same number first (2026-09-04), so it was moved to `0197`. In records from before 2026-09-14 (handoff history and so on), an `ADR-0179` that points at this decision means this document.

## 맥락

Every time we made a briefing, we **explained and drew the same structure again.** The codex backend Phase 1 briefing was like that, and the next time we do the gemini backend, we will again have to explain it by laying the codex structure beside it and comparing. That is, what is needed is not a briefing written once and thrown away but **parts that are stacked per topic and taken out for use.**

The two existing places cannot play that role.

- **The bird's-eye view (`docs/reference/architecture-overview.md`)** is, as its name says, a document that shows the whole on one sheet. If we push per-topic detail into it, the bird's-eye view stops being a bird's-eye view.
- **Step folders (`docs/process/SN-*/`)** are **frozen snapshots** of what that step decided and how (ADR-0034 lifetime invariant: design/research become a snapshot after approval·freeze). If we put reusable parts here, they get cited while stale.

## 결정

**`docs/reference/structure/<주제>.md` — one topic = one file.** It is an evolving canon, so it is edited in place (the `reference/` lifetime).

- ★**The reader is the user — this folder is a 「실시간 설명 문서」 (“live explanation document”)**★ (user decision 2026-09-08). `step-log` and TRDs are documents that sessions read, and the user does not look at them. So the writing in this folder is **in an explanatory style** — but it does not just list things (user instruction: 「너무 내용 늘어놓지말고 설명해줘」 (“don't line up so much content, explain it”)). Each section is a short explanation + a figure·table + an anchor.
- **We do not aim at a finished version.** Each time we are asked, we write the answer in here, and we also include what is not yet done and what is unverified. Putting it in proper order is done **when the user asks**. So we do not make a split like 「부채는 캐논에서 빼고 추적 문서로」 (“take debts out of the canon and into a tracking document”) — the purpose of this document is that one place gives the answer.
- **The delivery copy is HTML and is 「그 당시 문서」 (“a document of its time”)** (user decision 2026-09-08) — it is baked from the same mermaid sources, but **it is not updated every time the structure changes.** Detail and the latest live in the md, and the HTML is a rough explanation as of that moment. Once the duty to update is removed, 「전달본이 낡았다」 (“the delivery copy is stale”) becomes a statement of fact, not a defect.
- ★**The record is a file in the repo — not a publication link.**★ Artifact URLs evaporate (it actually happened: the S21 briefing link died because the account could not be looked up). So we commit the HTML file to the repo, and use links only as a convenience of the moment. We do not write a publication URL in a document as if it were canonical.

- ★**The unit is 「주제」 (“topic”), and the topic is an extension axis, not a vendor.**★ So the first occupant is **`agent-backend.md`**, not `codex-backend.md`, and **a third backend adds one column, not a file** (user decision 2026-09-08). This repository's contract test bench is already built on that rule (a column grows per backend on the same question table), and we fitted the documents to the same rule.
- **So the inside of a file has two layers** — ⓐ a backend-neutral spine (figures: the two gestures · the spawn chain · the render branch · the closed doors — plumbing that any backend rides) ⓑ per-backend cells (tables: argv · capability declarations · LLM endpoint policy · wire variants). A new backend adds one column to ⓑ and does not touch ⓐ.
- **Cross-cutting topics that are not subsystems also belong in the same layer** (e.g. the render branch · the LLM endpoint) — but if the topic is part of some axis, it is kept as a section inside that axis's file. The criterion for adding a file is **whether the axis is different**.
- **The figure source is unified to a single mermaid.** mermaid renders from the same source both in the `.md` render view and in `<pre class="mermaid">` of the artifact HTML — so briefings and decks **take the source as it is, without redrawing the figures**. This is the point where reuse holds.
- ★**Evidence anchors give only the file and symbol names and do not give line numbers.**★ If line numbers are attached to a reused figure, it goes stale each time it is cited.
- **Each file puts a baseline stamp at its head** (branch · time of reading · scope) — so that staleness is visible.
- **The bird's-eye view and step documents point at this folder and do not copy it** (ADR-0034 SSoT). The bird's-eye view remains a bird's-eye view.
- **The folder name is `structure`** — it covers figures · tables · argv · capability declarations in one word, and its layer is distinct from `architecture-overview.md`.

## 거부한 대안

- **Push the detail into the bird's-eye view (`architecture-overview.md`)** — rejected by the user: 「오버뷰는 말그대로 전체를 보는건데 거기다 하나하나 다 넣게? 그럼 오버뷰 의미가 있음?」 (“the overview is literally for seeing the whole, and you'd put every single thing in there? Then does the overview mean anything?”). Measured reinforcement: that document is already 838 lines and 20 mermaid figures, yet mentions codex in only two lines, and of those the seam table is **already false** with 「`AgentCommand` 에 variant 자체가 없다」 (“AgentCommand has no variant at all”) (Phase 1 overturned it). Adding more detail only raises the speed at which this document goes stale. **An added conflict:** that document **deliberately does not put in line numbers** to prevent rot (the head text of that document) — it is not a place for a detailed figure built on line numbers.
- **Keep it inline in the TRD — keep leaving the detailed figures inside the TRD in the step folder** — rejected by measurement: the 7 figures in `docs/process/S21-codex-backend/trd.md` §2.5 are a 2026-09-07 reading built on `file.rs:line` anchors, and they **went stale one day after landing** (they are drawn as `CodexBackend 미배선` and a flat file `backend/claude.rs`, but the reality is a folder split + wired into the dispatch table). Right as a frozen snapshot, and **unusable as a reusable part**.
- **Attach the reusable parts to the step folder** — rejected by the user: 「스탭이나 그런곳 붙이기는 애매함」 (“attaching it to a step or such is awkward”). The lifetimes do not match (ADR-0034: immutable·cumulative vs living).
- **Folder name `diagrams/`** — rejected by the user: 「근데 다이어그램만 있을게 아닐수도 있는데」 (“but it may not be only diagrams”). Proof: the first occupant contains together a capability declaration table · argv templates · a wire contract table, and those three carry more information than the figures. The moment one table comes in, the folder name becomes false.
- **Folder name `subsystems/`** — fits a chunk like a backend, but there is no place for cross-cutting topics that are not subsystems, like 「렌더 분기」 (“the render branch”) and 「닫힌 문 셋」 (“the three closed doors”).
- **Folder name `topics/`** — nothing that comes in would break it, but the name says nothing. Everything under `reference/` is a 「주제」 (“topic”).
- **One file per vendor** (`codex-backend.md` · `gemini-backend.md` · …) — rejected by the user: 「나중에 제미나이 붙일때 또 따로 하지말고 문서 구조 통합하면 되는거잖아」 (“when we attach gemini later, we shouldn't do it separately again, we can just unify the document structure, right”). If a file is set up per vendor, **the neutral spine is duplicated as many times as there are vendors** (the two gestures · the spawn chain · the render branch · the closed doors are the same for any backend), and when only one of them is fixed it becomes impossible to tell which copy is right. Growing columns on one axis has zero duplication.
- **Use 「아키텍처」 (architecture) in the name** — rejected by the user: 「아키텍처라고 하기에는 기술문서 같은 느낌이고」 (“calling it architecture feels like a technical document”). The name overlaps with the bird's-eye view and the layers also blur.
- **Take debts and unverified items out of the canon and send them to a tracking document** (the cut-lens proposal from the review, proposed by the main agent) — rejected by the user: 「이것저것 다 넣으면됨 · 실시간 설명문서라고 생각하면됨」 (“just put everything in · think of it as a live explanation document”). If we split, the user has to ask the same question across two documents, and the reason this folder exists (one place gives the answer) is broken. **The opposing lens we rejected** (guarding what is load-bearing) demanded keeping it, saying 「`ADR 0건` 은 시점 수치가 아니라 정책 항목」 (“"zero ADRs" is not a point-in-time figure but a policy item”), and the user's decision goes in the same direction as that.

## 근거

- **The reuse requirement came from the user's own words** — 「매번 새로 설명하는것보다 중간중간 내용 다음으면서 재활용하면 되잖아」 (“rather than explaining anew every time, we can recycle it while refining the content along the way”) + 「나중에 제미나이 할때도 옆에도 붙여서 할거라서 독립주제여야 되는거잖아」 (“when we do gemini later we'll attach it beside, so it has to be an independent topic, right”). This is where the reason the unit must be a topic comes from.
- **There are two measurements of staleness** — ① the 7 figures in TRD §2.5 had their anchors and labels go off one day after landing ② one of the two codex lines in the bird's-eye view is already false. 「줄번호 금지 + 기준 스탬프」 (“No line numbers + a baseline stamp”) is the prescription for these two.
- **The folder-name options were compared on a real tree and the user chose** (2026-09-08) — we put up the four, `structure` / `diagrams` / `subsystems` / `topics`, each with its example occupants.

## 영향 / 불변식

- **A new document is registered in the `docs/README.md` discovery chain** (no orphans — ADR-0034). But since that file forbids a hand-copied list, it **does not enumerate the files in the folder** and points at `ls docs/reference/structure/`.
- **The flow is left in `docs/process/step-log.md`** (*when·what* — CLAUDE.md 「기록 분리」). With the ADR entry alone, there is nowhere that says when this folder came to be.
- **We put a pointer going down from the bird's-eye view to this folder** — the round trip of 「전체 그림 → 조감도, 주제별 상세 → `reference/structure/`」 (“whole picture → bird's-eye view, per-topic detail → reference/structure/”) has to hold for the SSoT (link, don't copy) to take effect. ★If it is left one-way, a session that reads only the bird's-eye view does not know the detail exists.★
- ★**Using line numbers in this folder's anchors is a violation.**★ If violated, cited figures quietly go stale, and the next session opens a line that is not there and meets code without grounds.
- **The bird's-eye view is kept as a bird's-eye view** — returning per-topic detail to `architecture-overview.md` invalidates this decision.
- **TRD §2.5 stays where it is** (a frozen snapshot). Promotion is 「줄번호를 떼고 다시 그리는 것」 (“removing the line numbers and redrawing”), not moving the file.
- **A third backend is only one more file** — `gemini-backend.md`. We do not decide the folder structure again.
- **There is no gate** — what keeps this decision is only people and review (we did not put in a check that measures the no-line-numbers rule by machine). If automation becomes necessary, we decide it then as a separate matter.
