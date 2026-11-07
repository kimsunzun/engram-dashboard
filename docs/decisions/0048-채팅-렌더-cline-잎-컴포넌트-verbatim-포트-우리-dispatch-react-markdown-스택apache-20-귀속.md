# ADR-0048: Chat rendering = verbatim port of Cline leaf components + our own dispatch (react-markdown stack, Apache-2.0 attribution)

- 상태: **폐기 (Superseded by ADR-0050)** — the benchmark was recalibrated from Cline to the Claude Code VSCode extension by a screenshot comparison (user decision 2026-07-06), so the reason for the Cline port ceased to exist and only the Apache-2.0 attribution burden remained → remove the port and attribution and rewrite as our own implementation. ~~확정 (2026-07-05, basis: Cline reconnaissance and attribution audit (subagent) + user decision · implementation to pass the follow-up /review code + /qa gates)~~
- 관련: Amends ADR-0047 (how the chat UI is rendered: native CC-look direct implementation, OSS as reference only (not copy-paste) → verbatim code port of Cline leaf components (Apache-2.0 attribution)) · `src/components/slot/StructuredTextView.tsx` (successor of dispatch) · `src/components/slot/structuredAccumulator.ts` (StructuredItem model) · Cline clone `I:\Engram_Workspace\references\cline` · step-log 2026-07-05

## 맥락
ADR-0047 decided the chat UI as "Claude Code (CC) look **implemented natively, directly**, OSS as **reference only** for data mapping and wording (not copy-paste → no license trigger)". Its grounds were (a) the CC extension is closed source, so there is nothing to copy, (b) the OSS candidates do not have the CC signature look, and (c) our stack was pure CSS at the time, so the cost of porting OSS (all Tailwind) was HIGH.

But the native direct implementation repeatedly stopped at an "approximation", and the user pointed it out repeatedly (the hand-made icons, accordion and timeline kept drifting from the real look). At the same time, two of the rejection grounds in ADR-0047 collapsed:
1. **Adopting Tailwind (the core of ADR-0047) defeated the "translation cost HIGH" ground on its own** — now Cline's Tailwind/shadcn/lucide code can be moved over as it is, without rewriting the CSS.
2. **Benchmark relaxation (user decision)** — "permanently falls short of the CC look" was the reason for rejecting the Cline port, but the user reset the goal to "real Cline code is better than my hand-designed CC approximation".

Reconnaissance and an attribution audit of the Cline chat components before starting confirmed that Cline's chat **top-level dispatch component `ChatRow.tsx` (1208 lines) cannot be copied verbatim** — it is wholly tied to the VSCode-only gRPC clients (`FileServiceClient`/`UiServiceClient`), the `useExtensionState` context and `@shared/proto/*`, so it cannot be detached to work with our data. Only the **leaf presentation components** that draw content can be copied.

## 결정
Implement chat rendering as **a port of the verbatim-copyable leaf components of Cline + a dispatch that we write**.

- **Verbatim copy (only the VSCode wiring removed):** `chat/ThinkingRow`, `chat/MarkdownRow`, `common/MarkdownBlock` (remove only the `useExtensionState` Plan/Act callout and the `FileServiceClient` file-exists callout (two places in total)), `common/CopyButton`, `ui/button`, `chat/ExpandHandle`.
- **Adapted port (not verbatim):** `common/CodeAccordian` (codicon icon span → lucide; use the `MarkdownBlock` highlighting path instead of the styled-components `CodeBlock`).
- **We write the dispatch** — since `ChatRow` cannot be copied, we write ourselves the dispatch (the successor of `StructuredTextView`) that maps our order-preserving `StructuredItem[]` stream (text/tool/usage/error/structured/separator + user) onto the leaf components above. Layout placement is also ours.
- **Adopt the react-markdown stack** — using Cline's `MarkdownBlock` verbatim requires `react-markdown` + `remark-gfm` + `rehype-highlight` + `unist-util-visit` + `marked`. The button needs `@radix-ui/react-slot` + `class-variance-authority`. (This replaces the existing lab mini markdown parser.)
- **Do not adopt styled-components** — Cline's `CodeBlock`/`MermaidBlock` pull it in, but it violates the ADR-0047 Tailwind stance, so it is excluded.
- **Attach the Apache-2.0 attribution** — Cline is Apache-2.0 (copyright holder `Cline Bot Inc.`) and **has no NOTICE file** (confirmed by the audit). What is needed: ① a copy of the full LICENSE text (`LICENSES/cline-Apache-2.0.txt`) ② a "Cline original · Apache-2.0 · Modified by" header on each copied/adapted file (§4b). NOTICE is unnecessary because Cline has none.

## 거부한 대안
- **Native direct implementation of the CC look (the original decision of ADR-0047)** — it repeatedly stopped at an approximation and the user pointed it out repeatedly. It kept drifting from the real look, so quality fell short. (This is what this ADR amends.)
- **Rebuild everything up to Cline's `ChatRow`** — rebuild the top-level dispatch/layout on the Cline structure. It is a major job rather than verbatim, all the VSCode gRPC has to be stripped out, and there is a risk of regressing to an approximation. The leaf port already gets us the actual rendering internals (markdown, thinking, code), so the gain for the cost is low.
- **Copy the Claude Code extension directly** — closed source (All Rights Reserved). It cannot be copied (settled in ADR-0047).
- **Copy Cline's styled-components-based `CodeBlock`** — violates the Tailwind stance (ADR-0047) + runtime cost. It can be replaced by the rehype-highlight path of `MarkdownBlock`.
- **Keep the existing lab mini markdown parser** — it supports only headings, paragraphs, lists, bold/code fences (no GFM tables, nesting or blockquotes; spike quality). It conflicts with the "real Cline" goal and its rendering completeness is lacking.

## 근거
- **Cline reconnaissance and attribution audit (subagent)** — classification of files into copyable/adapt/skip, whether each file has a license header (all have none), **confirmed that the Cline NOTICE is absent** (resolves the item the handoff had left unconfirmed), LICENSE = Apache-2.0/`Cline Bot Inc.`, dependency delta (the 5+2 above), a draft StructuredItem→component props mapping.
- **User decision (2026-07-05)** — "faithful port": leaf parts verbatim + our dispatch, accept the react-markdown stack.
- **ADR-0047 revisited** — adopting Tailwind removed the "port = CSS rewrite" friction, and the user relaxed the benchmark (reaching the CC look) → the grounds of 0047 for rejecting the Cline port disappeared.
- **This ADR does not guarantee that the decision is right** — the port implementation is verified by the follow-up `/review code` + `/qa` (cdp measurement) gates.

## 영향 / 불변식
- **ADR-0047 amended** — the clause "chat UI look = native direct implementation / OSS reference only (not copy-paste → no license trigger)" is discarded. The rest (adopting Tailwind v4 + shadcn/lucide styling, keeping the theme CSS variables) stays.
- **License trigger fires** — since we now copy real code, the Apache-2.0 §4 compliance duties (LICENSE copy + "Modified by" header on changed files + keeping the copyright notice) are enforced. Violating them is a license violation. (Legal confirmation is for the user to do at publication time — deferred for now because of personal local use, user decision.)
- **Data model unchanged** — the `StructuredItem` order-preserving stream (ADR-0045) is not changed. The port replaces only dispatch/rendering, and the subscribe/accumulate/send data flow (ADR-0044/45/46) is unchanged.
- **Dependencies added (subject to change report)** — `react-markdown`, `remark-gfm`, `rehype-highlight`, `unist-util-visit`, `marked`, `@radix-ui/react-slot`, `class-variance-authority`. `styled-components` is not adopted.
- **Anchor** — put a `// ADR-0048` anchor in the load-bearing port/dispatch files. Copied files also carry the Cline attribution header.
