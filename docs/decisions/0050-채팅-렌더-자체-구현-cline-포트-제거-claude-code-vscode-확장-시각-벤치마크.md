# ADR-0050: Chat rendering = our own implementation (Cline port removed) + the Claude Code VSCode extension as the visual benchmark

- 상태: 확정 (2026-07-06, basis: user decision to recalibrate the benchmark + the implementation passed the /review code and /qa (cdp measurement) gates)
- 관련: Supersedes ADR-0048 (Cline leaf port + Apache-2.0 attribution) · ADR-0047 (the Tailwind adoption stays, the own-implementation premise is restored) · `src/components/slot/chat/` (new renderer) · `src/components/slot/StructuredTextView.tsx` (dispatch) · `src/components/slot/RichSlot.tsx` (send is Enter-only) · `src/components/slot/structuredAccumulator.ts` (StructuredItem model, unchanged) · step-log 2026-07-06

## 맥락
ADR-0048 decided chat rendering as "verbatim port of Cline leaf components + our dispatch + the react-markdown stack + Apache-2.0 attribution". Its premise was that (a) we actually copy Cline code to get the look, and (b) in return we bear the Apache-2.0 §4 attribution duties (LICENSE copy + file headers + THIRD_PARTY_NOTICES).

After starting, the benchmark was recalibrated by a screenshot comparison (user decision 2026-07-06): it was settled that the look the user wanted is **not Cline but the Claude Code VSCode extension**. With that, the reason for the Cline port ceased to exist — carrying Cline code around does not bring us closer to the target look (the Claude Code extension), and only the Apache-2.0 attribution burden remains. The Claude Code extension is closed source (All Rights Reserved), so a code port is impossible, and the only path to the look is **benchmarking by measuring screenshots**.

## 결정
Rewrite chat rendering as **our own implementation with the Cline port removed completely**, and fix the visual target as the **Claude Code VSCode extension (a screenshot-measured benchmark)**.

- **New own implementation in `src/components/slot/chat/`:**
  - `Markdown.tsx` — `react-markdown` + `remark-gfm` + `remark-math` + `rehype-highlight` + `rehype-katex` (+ katex css). Sanitizes zero-width characters (U+200B and the like; streaming leaks them before a fence), a single `ReactMarkdown` (no splitting into blocks), language alias normalization, a `pre` hover copy button, wrapper `.chat-markdown`.
  - `ThoughtRow.tsx` — a collapsed "Thought" label + chevron. **The label row is shown even when the content is empty (encrypted thinking)** (non-interactive, tooltip "내용 비공개"). If there is content it toggles, and it pulses while streaming.
  - `CopyButton.tsx` — lucide Copy→Check. **No dependency** on `ui/button`, cva or radix (a plain button).
  - `chat.css` — first-approximation typography/spacing values (the base before precise matching to the extension).
- **Cline port and attribution removed completely:** delete `slot/cline/*` (5 files) · `ui/button.tsx` · the three attribution items (`THIRD_PARTY_NOTICES.md` · `LICENSES/cline-Apache-2.0.txt` · headers on copied files).
- **Add KaTeX math rendering** (remark-math + rehype-katex) — resolves the raw `$$…$$` strings being shown.
- **Remove the send button → Enter-only.** The RichSlot input bar goes full width, "메시지 입력 (Enter 전송 · Shift+Enter 줄바꿈)".
- **dispatch layout = left dot rail.** `StructuredTextView` groups the assistant-family rows (text, thinking, tool, error) with a left gutter + dot marker like one thread, and puts the user bubble as a full-width box with no rail (the extension look). First approximation — the connector line (vertical thread) comes later.
- **Process = first approximation, then iterate.** Build and commit the base (engine, features) first, and adjust the precise matching of the extension look afterwards by iterating on user screenshots.

## 거부한 대안
- **Keep the Cline port + only rename it as ours** — it is not worth carrying the Apache-2.0 attribution (LICENSE copy, file headers, NOTICES) forever for effectively 300–400 lines of rendering code. The benchmark changed from Cline to the Claude Code extension, so the reason for the Cline port itself ceased to exist. (This is the decision of ADR-0048, which this ADR discards.)
- **Directly port the Claude Code extension code** — closed source (All Rights Reserved), cannot be copied. So screenshot-measured benchmarking is the only path to the look.
- **Reintroduce marked / styled-components** — the Cline stack pulled them in, but they are unnecessary for our own implementation. `marked` is replaced by the react-markdown path and removed; styled-components is excluded because it violates the Tailwind stance (ADR-0047).
- **Approximate the extension look by "feel", without a screenshot benchmark** — the failure mode of the ADR-0047 native direct implementation, which stopped at repeated approximation. Since there is no source, we fix screenshot measurement as the benchmark and catch the drift with it.

## 근거
- **User benchmark recalibration (2026-07-06)** — after the Cline fidelity work in the morning (82a8108·8b844ed), a screenshot comparison settled "target look = Claude Code extension". The user directly chose a complete rewrite of the Cline port + deleting the attribution.
- **Implementation passed the gates** — `/review code` (doc-aware) PASS (guards undamaged, tests non-vacuous) + `/qa`: vitest 242/242 · `tsc --noEmit` 0 · BOM clean · **cdp measurement** (markdown, KaTeX box, code highlighting, hover copy button, send button removal confirmed live).
- **This ADR does not guarantee that the decision is right** — precise visual matching is incomplete (first approximation). Reaching the extension look is verified by follow-up screenshot iterations.

## 영향 / 불변식
- **ADR-0048 discarded entirely** — the decision of the Cline verbatim port + Apache-2.0 attribution is gone. **The license trigger is resolved** — no real Cline code is in the codebase, so the Apache-2.0 §4 duties disappear and the three attribution items are deleted.
- **Relation to ADR-0047** — 0048 had amended the "native direct implementation / OSS reference only" clause of 0047, but with 0048 discarded that amendment is reverted, and **the own-implementation premise of 0047 is restored**. This ADR adds to it "Claude Code extension = an explicit screenshot benchmark". (The adoption of Tailwind v4 + shadcn/lucide and the theme CSS variables of 0047 stay.)
- **Data model unchanged** — the `StructuredItem` order-preserving stream (ADR-0045) and the subscribe/accumulate/send data flow (ADR-0044/45/46) are unchanged. `structuredAccumulator` (in particular the `Date.now` ban — the replay idempotence invariant) is untouchable. This round replaces only dispatch/rendering.
- **Showing the empty (encrypted) thinking row** — the thinking of opus-4-8 is encrypted (plain text is an empty string) as upstream behavior, so the "Thought" label row is shown even when the content is empty (non-interactive). sonnet gives plain text normally. (No re-debugging — upstream behavior.)
- **Send = Enter-only (button removed)** — the Enter/Shift+Enter/IME guard (`isComposing||229`) and the `send()` await-catch in RichSlot are load-bearing invariants (a regression test guards this behavior through the Enter path). Removing the button does not change this logic.
- **Dependency delta (subject to change report)** — added `katex`, `remark-math`, `rehype-katex` / removed `class-variance-authority`, `@radix-ui/react-slot`, `unist-util-visit`, `marked`.
- **Anchor** — put a `// ADR-0050` anchor in the load-bearing chat-rendering dispatch/leaf files. The `// ADR-0049` that was mislabeled in `StructuredTextView.tsx` and its test (dispatch belongs to 0048→0050, and 0049 is the backend thinking-token injection) is corrected to 0050. (The 0049 citation in ThoughtRow's "encrypted-thinking grounds clause" is legitimate — kept.)
