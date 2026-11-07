# ADR-0047: Frontend styling = adopt Tailwind CSS v4 + shadcn/lucide (switching away from the pure-CSS stance)

- 상태: 확정 (2026-07-05, basis: /research medium + Codex adversarial review + CSS measurement)
- 관련: CLAUDE.md "기술 스택(프론트)" (the wording `CSS 변수(Tailwind X)` is to be updated) · `src/styles/theme.css` (three data-theme themes) · step-log 2026-07-05 · Amended by ADR-0048 (how the chat UI is rendered: native CC-look direct implementation, OSS as reference only (not copy-paste) → verbatim code port of Cline leaf components (Apache-2.0 attribution))

## 맥락
We tried to match the chat rendering of the structured-output (StreamJson) slot to the **Claude Code VS Code extension look** (a vertical dotted-line timeline · a collapsed "Thought for Ns" thinking · minimal tool IN/OUT), but the previous port stayed at an "approximation" (emoji icons, a hand-made `<details>` accordion) and its quality was much lower.

To find the cause, we ran `/research` (medium, design-decision mode) + a Codex (cross-family) adversarial review. The results:
1. **The Claude Code extension is fully closed source** ("All Rights Reserved"). The CLI source-map leak (2026-03) is also legally unusable → **there is no OSS source to copy the CC look from.**
2. **All OSS candidates (Cline, Roo, Kilo-legacy, Continue)** are tightly coupled to Tailwind CSS + shadcn/Radix (some to styled-components), and **none of them has the CC signature look** (all have a "VS Code sidebar" aesthetic).
3. We use **pure hand-written CSS**, so taking over candidate code means **translating every `className` utility into CSS** (porting difficulty HIGH). This is also the root cause of why the previous session stopped at "imitation" — there was no matching thing to copy, so forcing a fit ended in an approximation.

The stack question came up here: **if we also adopt Tailwind, does this friction go away?** The CSS measurement showed that almost no keeper CSS exists (see the grounds below), so the switching cost is at its lowest right now.

## 결정
Switch the frontend styling to **Tailwind CSS v4 + shadcn/ui (Radix-based, only what is needed) + lucide-react (icons)**.
- **Keep the three `data-theme` themes (dark/light/e-ink) as they are** — map the CSS variables onto the Tailwind color tokens (`bg-background` → `var(--...)`) so the theme-switching mechanism does not change.
- The existing keeper CSS (~20 lines) is absorbed into the Tailwind system; the `lab/` scratch CSS (layouts.css, etc.) is to be discarded.
- On top of that, the chat UI **implements the Claude Code look natively, directly** — no OSS provides that look, so this is not "copy-paste" but our own design (references: claude-code-webui = a data-mapping reference, Continue `ThinkingBlockPeek` = a reference for the "Thought for Xs" wording).

This decision **switches** the `CSS 변수(Tailwind X)` stance of the CLAUDE.md tech stack (there was no formal predecessor ADR → recorded fresh, with nothing to supersede).

## 거부한 대안
- **Keep pure CSS (the existing stance)** — keeper CSS is only ~20 lines, so the switching cost is *at its lowest right now*, and the long-term cost of hand-reinventing polish and speed without the shadcn ecosystem is large. All the OSS we would port is Tailwind, so interoperability also suffers. (A judgment that allows over-engineering given the 10-year-maintenance premise + low-risk, long-term = CLAUDE.md architecture §0.)
- **Port OSS (Cline, etc.) wholesale — translated into pure CSS** — the result is a "Cline look", so it **permanently falls short** of the CC benchmark (the verbose "Cline wants to…" header, no dotted-line timeline, no "Thought for Ns") + the translation cost is HIGH.
- **Fork claude-code-webui (skeleton)** — it renders the same claude stream-json, but it is **archived (2026-05, ~1.1k stars), its look is a generic web chat** (no CC dotted-line timeline/Thought-for-Ns — the adversarial review judged the "highest fidelity" claim an exaggeration), and **it is Tailwind, so the CSS has to be rewritten anyway**. Our backend already emits structured items, so the gain from its parser is also small.
- **siteboon/claudecodeui and the like** — the adversarial review confirmed **AGPL-3.0** (copyleft) → unsuitable for porting into our app.

## 근거
- **/research (medium) + Codex (cross-family, medium) adversarial review** — grounding (a) confirmed the CC extension is closed source (b) the four candidates are tightly coupled to Tailwind and lack the CC look (c) corrected the claude-code-webui exaggeration and siteboon's AGPL.
- **CSS measurement** — of 763 lines in total, `lab/` scratch 394 + the chat rendering to be rewritten 348 = **keeper CSS ~20 lines** (theme/font/index/App). Switching cost is at its lowest.
- **No performance harm** — Tailwind = static CSS generated at build time (zero runtime JS overhead). It is even lighter than the styled-components that some candidates use (which have a runtime cost), and unused classes are tree-shaken out of the bundle.

## 영향 / 불변식
- **CLAUDE.md "기술 스택(프론트)"**: `… CSS 변수(Tailwind X) …` → update to `Tailwind CSS v4 + shadcn/lucide + CSS 변수 테마` (as one bundle with this ADR).
- **Themes unchanged** — the three `data-theme` themes (dark/light/e-ink) stay as CSS variables, and the Tailwind color tokens reference those `var()`s. **The theme-switching mechanism is not changed** (the three themes are not expressed by the Tailwind `dark:` variant alone).
- **Chat UI look = native design** — there is no source to copy the CC look from, so we design it ourselves. OSS is limited to data-mapping and wording references (not copy-paste → no license legal trigger).
- **Unrelated to the §5 LLM control invariant** — changing the styling system does not affect the control surface (command bus, invoke).
- **Follow-up (what this ADR executes)** — Tailwind v4 + Vite plugin setup · theme variable ↔ token mapping · adopting lucide/shadcn · reimplementing the successor of `StructuredTextView`. Put a `// ADR-0047` anchor in load-bearing code.
