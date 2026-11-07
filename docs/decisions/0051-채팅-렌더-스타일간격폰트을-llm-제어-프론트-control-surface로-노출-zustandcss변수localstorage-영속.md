# ADR-0051: Expose chat rendering style (spacing, font) as an LLM-controlled frontend control surface — Zustand + CSS variables + localStorage persistence

- 상태: 확정 (2026-07-06, basis: user decision + two Explore investigations) · 부분 폐기 by ADR-0169 (the claim that the LLM handle __engramChat and the human UI do the same action) · 부분 폐기 by ADR-0265 (the clauses on frontend-only authority and localStorage persistence)
- 관련: CLAUDE.md §5 (LLM-first control) · ADR-0050 (own implementation of chat rendering) · `src/components/slot/StructuredTextView.tsx` · `src/components/slot/chat/chat.css` · `src/store/themeStore.ts` (the pattern inherited) · `src/store/eventBus.ts` (exposes the window handle) · step-log 2026-07-06 · Amended by ADR-0169 (the claim that the LLM handle __engramChat and the human UI do the same action) · Amended by ADR-0265 (the clauses on frontend-only authority and localStorage persistence)

## 맥락
In ADR-0050 we implemented chat rendering ourselves and are visually refining it to match the Claude Code VSCode extension look. The user compared our rendering and the extension side by side and pointed out "the line spacing is tight and the whole thing looks cluttered" — the core causes are ① too little vertical spacing between rows ② the user message does not break the conversation into chunks ③ the dot-rail connector line's clean-ends are unhandled.

The investigation found that the **colors** of chat rendering are already CSS variable tokens (`--text`/`--border`/`--surface-elevated`, etc., theme.css + the index.css `@theme`), but **spacing, font and line-height are all scattered as hardcoded Tailwind classes** (`pt-2.5`/`pt-3`/`top-[-12px]`/`top-[9px]`/`text-[13px]`/`leading-[1.45]` in `StructuredTextView.tsx` + the margins and paddings in `chat.css`). CLAUDE.md §5 states firmly that "every UI feature must be LLM-controllable, UI first and control later = a violation", so leaving these values hardcoded and merely tuning the spacing would violate the rule.

User instruction: work while pulling the spacing/font settings out so an agent (LLM) can command them, but **they must be saved (persisted)**. The control-surface pattern is already established — layout is `window.__engramLayout` (invoke loop, src-tauri authority), and theme, render mode and font preset are frontend-only Zustand + CSS variable handles.

## 결정
Extract the **spacing + font set** of chat rendering (row-gap, user bubble margin, dot-rail connector/dot offsets, base font-size, line-height) from hardcoded Tailwind classes into **CSS variables**, and expose it to the LLM as a **frontend-only control surface**.

- **Authority = frontend only:** a new Zustand chat-style slice owns the values and updates the CSS variables (`:root`). It inherits the existing "Zustand slice + CSS variable update + `window.__engramXxx` handle" pattern of theme, render mode and font preset.
- **LLM handle:** expose it as `window.__engramChat` (getter + setter) — the human UI operation and the LLM operation call **the same store action** (a single control surface, keeping the §5 hands/brain split: frontend = pure I/O).
- **Persistence = localStorage:** save to localStorage when a value changes, and load it at boot → apply to the CSS variables. It holds across a refresh and a client (src-tauri shell) restart.
- **dot-rail connector clean-ends:** add a run-position computation (the top/mid/bottom/single position within a consecutive assistant run) to resolve the line sticking out above the first dot. The coupling between the connector offsets and the CSS variables above is made explicit.

## 거부한 대안
- **Backend persistence (Tauri command + settings.json + emit loop, the layout pattern)** — a complete §5 "brain = backend" split, but for a pure rendering preference the cost of daemon, file and emit wiring exceeds the gain. Theme and render mode also have frontend-only precedent, so frontend authority is enough. (User choice: frontend)
- **Keep CSS hardcoded (tune only the spacing)** — there is no LLM control surface, so it directly violates §5 "UI first and control later = a violation".
- **No persistence (in-memory store only)** — the user stated explicitly "it must be saved". The current theme slice has no persistence, so it resets to dark on refresh, and we would repeat that trap as it is.
- **Tokenize all of chat.css (even heading/code/table margin, padding and radius)** — turning values we are not touching right now into CSS variables = YAGNI. Low risk, but unnecessary in this round, so the scope is limited to the spacing + font set (extend with the same pattern later if needed).

## 근거
- CLAUDE.md §5: every UI feature is LLM-controllable + a single control surface (human click = LLM call = the same entry point).
- CLAUDE.md architecture principle §0 (risk × duration): a CSS variable token layer = low risk + long term → lay the seam down properly now. Widening the scope later is easy.
- It inherits an existing pattern (theme, render mode, font preset), so new abstraction and wiring are minimal.
- Grounded by two Explore investigations: a location map of the style values + the current state of the control surface (`window.__engramLayout`/`__ENGRAM_AGENT__`, etc.).

## 영향 / 불변식
- Add a new Zustand chat-style slice + CSS variables (theme.css/index.css `@theme`) + the `window.__engramChat` handle. `src/store/eventBus.ts` exposes the handle at boot.
- The hardcoded spacing/font values in `StructuredTextView.tsx`/`chat.css` are replaced by CSS variable references.
- **The rail connector offset ↔ outer padding coupling is made explicit as CSS variables** — the two are tied into one variable group and adjusted together (removes the implicit coupling of the existing `top-[-12px]` ↔ `pt-3`).
- Add a localStorage key (load at boot → apply to CSS variables). If the value is absent or fails to parse, fall back to the default.
- §5 compliance is kept: the frontend is pure I/O (only calls store actions), and the LLM does the same operation through `window.__engramChat`. `StructuredTextView` stays pure rendering (no adding state/effect — ADR-0050 invariant).
- Code anchor: one `// ADR-0051` line at each new/modified load-bearing spot.
