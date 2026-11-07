# ADR-0053: Chat slot overlay scrollbar = adopt Radix ScrollArea (native CSS, a dedicated library and a DIY build rejected)

- 상태: 확정 (2026-07-07, basis: /research medium + Codex cross-family adversarial review + package.json measurement)
- 관련: CLAUDE.md §기술스택(Tailwind+shadcn) · §5 LLM control · package.json:19 · src/components/ui/scroll-area.tsx (promoted app-wide 2026-07-12 — formerly slot/chat/ChatScrollArea.tsx) · RichSlot.tsx · ADR-0047·0051 · step-log S15

## 맥락
Five scrollbar requirements for the scroll area of the chat slot (structured/JSON output):
(1) a true overlay — hide the native scrollbar and float over the content taking up zero space · (2) hidden normally · (3) appears only while scrolling · (4) hides ~0.5s after scrolling stops · (5) a thin dark thumb, customizable with CSS variables. (The initial requirement was "show 0.5s after hover", but GUI measurement showed that the chat fills the window completely, so "hover over the area = always shown", and it was settled as scroll-triggered.)
Currently it is a single `overflow-y-auto` line (RichSlot.tsx:181) → the WebView2 native gray scrollbar (takes up space, always shown), which meets none of the requirements.

## 결정
Replace the scroll container with **Radix ScrollArea (`@radix-ui/react-scroll-area`, the shadcn `scroll-area` wrapper)**.
- Requirements (1)(2)(5) = met by the Radix defaults (overlay · hidden normally · Tailwind/CSS-var theming).
- Requirements (3)(4) = **`type="scroll"` + `scrollHideDelay=500`** — shown only during actual scrolling (wheel/drag), hidden ~0.5s after it stops. It is within the Radix default scope, so no custom CSS is needed. (Trade-off: with a scroll trigger it is hard to grab and drag the thumb — accepted by the user.)
- Wrap it in a thin `ChatScrollArea` seam to keep it swappable (components depend only on the seam, and must not expose the Radix Root directly). Auto-scroll (pinned to the bottom) is preserved because the seam exposes the Radix **Viewport** (the node that actually scrolls) through forwardRef.
- Put the thumb color per theme (`:root[data-theme=...]`) — dark = light translucent, light/e-ink = dark translucent (a single white on the theme-independent `:root` is invisible in light/e-ink; found by measurement and review).

## 거부한 대안
- **Native `::-webkit-scrollbar` CSS only** — a true overlay is impossible in principle. Chromium switches to the classic (gutter-taking) scrollbar the moment a width is given to `::-webkit-scrollbar`, and `overflow: overlay` was removed in Chrome 114 (2023) and became an alias of `auto` → requirement (1) is impossible. (Firmly grounded: Chrome for Developers docs · chromestatus 5194091479957504 · blink-dev intent)
- **OverlayScrollbars** — a mature overlay-only library (thick touch, keyboard and accessibility support), but a **new dependency, +15KB gz**. Radix is already a dependency, which cancels the advantage, and it is too much for chat scrolling.
- **SimpleBar** — show-on-hover is not a built-in option (open issue #650, needs manual CSS `.simplebar-mouse-entered`), and theming is selector-based → more customization burden than Radix/OSS.
- **DIY (build it ourselves)** — hide the native one + sync an absolute thumb in JS (scroll + 2 ResizeObservers + drag + track click, ~150-250 lines + edge cases). A maintained 15KB MIT library exists, so reinventing it is rejected under the CLAUDE.md "no reinventing" rule.
- **Abandoned libraries** — react-custom-scrollbars-2 (2022, React18-only, non-overlay) · react-scrollbars-custom (2022, looking for maintainers) · rc-scrollbars (React18-only) · react-perfect-scrollbar (original stalled in 2021). All are abandoned, lack React19 support, or are non-overlay.

## 근거
- **Zero new dependencies** — `@radix-ui/react-scroll-area ^1.2.11` is already in `package.json:19` (measured). Nothing to install.
- **Explicit React 19 support** — peerDeps `^19.0` (the other candidates are inferred from a range, or React18-only).
- **Fits the stack** — matches the project's Tailwind v4 + shadcn/ui convention (ADR-0047). shadcn `scroll-area` = a thin Radix wrapper → the §5 swap seam comes naturally.
- **Overlay confirmed** — Radix official docs: "sits on top of the scrollable content, taking up no space".
- The survey = `/research` medium (3 survey collectors + 1 Radix backstop) + a Codex cross-family adversarial review — the missed candidate Radix was found and reversed the decision (the draft recommended OverlayScrollbars → Radix).
- **Trigger evolution (GUI measurement)** — the draft (hover + a CSS 0.5s show-delay) had to add the show-delay with CSS because `scrollHideDelay` is a "hide delay", and measurement revealed that hovering the area always shows it, so it switched to **`type="scroll"` (shown only while scrolling + a 0.5s hide-delay)** — it is within the Radix default scope, so neither custom CSS nor a reduced-motion branch is needed.

## 영향 / 불변식
- The scroll container goes through the `ChatScrollArea` seam — do not expose the Radix Root directly in components (preserves the swap point, §5 hands/brain split).
- Removing the header "JSON ● idle" is a **separate matter** from this decision (a simple UI change, unrelated to the ADR — the flow is only in the step-log).
- §5: if scrollbar styling becomes an LLM control target, put it on the chatStyle control surface (ADR-0051) (currently fixed as CSS variables, not exposed).
- **Scope expansion (2026-07-12):** this seam is no longer chat-only but the **app-wide scroll primitive** `src/components/ui/scroll-area.tsx` (`ScrollArea`) — the old `slot/chat/ChatScrollArea.tsx` promoted under a neutral name. The DOM scroll surfaces (agent tree, presets, monitoring picker, DomSlot, ThoughtRow, RichSlot) all go through this one → a change to the scroll behavior or tokens in one place propagates globally. All the invariants above (overlay, `type=scroll`, 500ms, ref→Viewport, Root not exposed) apply unchanged. It is not a new decision but an expansion of the existing seam's scope, so it is recorded here without a separate ADR.
  - **Exception (outside the seam — intentional):** the xterm terminal owns its own scrollbar and cannot be wrapped in a component → `.xterm-viewport` is unified with the same theme tokens (`--scrollbar-*`) in global CSS (`src/index.css`) (sharing the tokens even though it is not a component). The `StructuredTextView` code-block horizontal scroll and the `TabBar` horizontal scroll are not targets of the vertical overlay pattern, so they keep raw overflow.
