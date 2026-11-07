# ADR-0168: Frame visual cleanup — lowering the focus ring, turning status glyphs into icons, switching the UI font to sans

- 상태: 확정 (2026-08-23, basis: 5 user decisions (compared on the real screen) + measuring with canvas width whether the fonts are installed + measuring the theme token values + `/review code full` 2-person adversarial) · 부분 폐기 by ADR-0227 (decision 7 allotment sash token) · 부분 폐기 by ADR-0295 (of decision 5, only the top corner radius, the divider rule and the inactive tab token) · 부분 폐기 by ADR-0298 (of decision 5 ① the active tab is brighter than the bar and its invariant, only: drawn in dark only, and the inactive tab text color in dark and light)
- 관련: Amends ADR-0066 (lowering the focus ring strength to 40 percent) · Amends ADR-0062 (replacing the status display from characters to icons) · Amends ADR-0075 (swapping the glyph mapping symbols) · ADR-0047(Tailwind) · ADR-0143(empty slot guidance) · ADR-0072(tree hierarchy) · CLAUDE.md 「LLM-우선 제어」 · Amended by ADR-0227 (decision 7 allotment sash token) · Amended by ADR-0295 (of decision 5, only the top corner radius, the divider rule and the inactive tab token) · Amended by ADR-0298 (of decision 5 ① the active tab is brighter than the bar and its invariant, only: drawn in dark only, and the inactive tab text color in dark and light)

## 맥락

Putting a mature peer (orca) and our screen side by side, ours looked hand-made. Narrowing the cause to the frame (tabs · panel headers · tree · borders) and measuring gave four things.

1. **The whole UI shell was being drawn in monospace.** `--font-ui` is `'JetBrains Mono', monospace`, but ★that font was not installed in this environment★ — the repository has no `@font-face` at all and depends only on a system install, and in practice it was being rendered with a **generic monospace fallback font**. That is, the intended font was never applied even once. The `Inter` of `--font-claude-prose` is the same.
2. **The frame was divided by lines (border) and was high-contrast.** A vertical divider per tab, an underline per panel header, and on top of that the accent blue popped out in two places at once (the tab active underline 2px · the slot focus ring 65%).
3. **The status glyphs were font characters** (`● ◻ ✗ ○`). Characters differ in size, weight and baseline per font, so they misalign vertically with the row label. After fixing ① and moving to sans this misalignment showed even more.
4. **The split lines were cut off from the theme.** The `--separator-border` (a gray literal) and `--focus-border` (VS Code blue `#007fd4`) of allotment's default CSS were hard-coded in `:root` and did not follow our theme switch.

## 결정

**1. Slot focus ring = accent 40%** (formerly 65% — ADR-0066 decision 4). A value the user chose by comparing 65 → 50 → 40 with their eyes on the real screen.

**2. Status glyph = a lucide icon mapping.** Match the **shape 1:1** with the character — `Running`=filled circle · `Exiting`/`Exited`/`Killed`=square · `Failed`=X · `Reserved`/unknown=empty circle. The pure function `statusGlyphIcon(status)` returns `{ Icon, fill, shape }`, and the string version `statusGlyph` is deleted. **`statusGlyphColor` is unchanged.**

**3. Make two DOM read contracts explicit.** Changing characters to SVG empties `textContent`, so the status read channel disappears, hence `data-agent-glyph-shape` (4 tokens) takes it over. Tree selection also had the background color as its only channel, so `data-selected` and `aria-selected` are attached. Both are CDP and LLM selector surfaces.

**4. UI font = `'Segoe UI Variable Text', 'Segoe UI', 'Malgun Gothic', system-ui, sans-serif`.** All are built into Windows, so it is actually applied. Latin is Segoe UI, and Korean falls back automatically to Malgun Gothic (the Windows default behavior as is).

**5. Tabs = Chrome layout.** Remove the accent underline (solid color) and mark the active tab with the four below — ① **the active tab is brighter than the bar** (active = `--bg-secondary`, bar = `--bg`) ② top corners rounded 6px ③ a vertical divider **only between inactive tabs** (none on either side of the active tab — because it is a raised piece) ④ the close `×` **always shown**. The inactive tab fill is set per theme by the new token `--tab-inactive-bg` (only light is pressed down one tone so the white active tab rises, dark and e-ink are `transparent`).

**6. Tree selection background = `color-mix(in srgb, var(--text) 12%, var(--bg-secondary))`.** The reason for mixing in `--text` is that this token by definition contrasts with the background, so a difference remains **in all three themes**.

**7. Tie the allotment sash tokens to the theme** — in `html:root`, `--separator-border: var(--border)` · `--focus-border: var(--accent)`. The reason for using `html:root` (0,1,1) is that allotment declares at `:root` (0,1,0) and it comes **later** in the bundle (it would lose by order).

## 거부한 대안

- **Keep the focus ring at 65%** — it popped out most in the frame and was the main culprit of "high contrast". The user decided by comparing on the real screen. ★It is the same 40% as the value ADR-0066 explicitly rejected★ — the ground of that rejection was a **general survey** saying "a weak focus indication is a repeated UX complaint (VS Code #24586, JetBrains IDEA-102931)", not a comparison with the real thing. This time it is a 3-step comparison on the real screen, so this takes priority. **To revert, raise only this line.**
- **Focus ring 50%** — actually applied it as the middle value, but judged it still pops out (real screen).
- **Keep the text character glyphs** — the font-dependent alignment mismatch worsened after the sans switch. But ★the principle of ADR-0062 that shape carries the status was not abandoned★ — a lucide icon is also a shape, so even when color is neutralized in e-ink the four shapes still separate (measured: in e-ink `--status-running` is `var(--text-muted)`, so the four glyphs become the same color and only the shape remains).
- **Distinguish status by color alone** — e-ink disables color, so shape is the only signal. Rejected as is per the ADR-0062/0075 principle.
- **Tree selection background `--surface-elevated`** — actually adopted once and then filtered out in review. ★In light (`#ffffff`) and e-ink (`#f0f0f0`) it has **the same value** as the panel background `--bg-secondary` (`#fff`·`#f0f0f0`), so the selection indication vanishes entirely★. Only dark (#141414 vs #262626) differs, so it was not caught by screen comparison (measured from the theme file values).
- **Tree selection background accent 15% wash** — being a blue wash it clashes with the solid-color tone of item 5.
- **Tab layout: active tab = `--bg` (bar = `--bg-secondary`)** — actually applied and then reverted. The active tab became **the darkest in the frame and read like a hole** (the user's judgment, real screen).
- **Tab layout: bar = `color-mix(--text 25%, --bg)`** — a proposal to raise the contrast from 1.07 to 1.8:1. Actually applied and then reverted — the bar became a gray band at odds with the tree panel and clashed head-on with the goal of this work, "make the frame quiet" (the user's judgment, real screen).
- **Raise the tab active contrast to the WCAG UI criterion of 3:1** — to get there 40%+ must be mixed in and the gray band problem above recurs as is. Chrome and VS Code also keep the same place at 1.3~2:1. It was resolved by **not resting the active decision on lightness alone** (text color + divider + the always-visible `×`).
- **Show the close `×` only on hover** — actually put in and then reverted. Since it only sets the opacity to 0, **the button still received keyboard focus and closed with Enter** — with no indication on screen of what is being closed (review finding + the user's judgment "닫기 버튼은 그냥 보여야 한다" — the close button should simply be visible).
- **Keep JetBrains Mono in `--font-ui`** — it is not installed, so it was never applied in the first place (canvas width measurement: the requested width of `"JetBrains Mono"` = the generic sans fallback width). That lists and labels read like terminal output is also a separate problem.
- **Specify Nanum Gothic / Pretendard / Noto Sans KR** — all three are not installed (same width measurement). They are not bundled with Windows, so even if specified they are silently replaced.

## 근거

- **Whether fonts are installed = canvas text width measurement** (`document.fonts.check()` almost always returns true for local fonts in Chromium and cannot be used). At 14px with `"Handgloves 123 illWW"` — generic sans 144.59 / JetBrains Mono 144.59 (= not installed) / Inter 144.59 (= not installed) / Pretendard 144.59 (= not installed) / **Segoe UI 139.67 (= installed)** / Malgun Gothic 144.51 (= installed).
- **Theme token contrast = reading the `src/styles/theme.css` values directly.** light `--bg-secondary: #fff` / `--surface-elevated: #ffffff`, e-ink both `#f0f0f0`.
- **allotment specificity = confirmed by comparing byte offsets in the build output CSS** (our rule first, allotment later → loses by order → wins by specificity).
- **`/review code full`** 2-person adversarial — doc-aware (invariant breaker) + cross-family blind. The tree selection loss and the absence of this ADR itself came out of that.

## 영향 / 불변식

- **★The tab bar must be darker than the active tab★** (bar = `--bg` · active = `--bg-secondary`). If reversed, the active tab reads as a "carved groove" — a failure actually experienced and reverted. ☞ **In e-ink this relation is inverted** (`--bg: #ffffff` > `--bg-secondary: #f0f0f0`) — in that theme the text color and the inactive divider carry the active indication. An unresolved residual.
- **★Do not rest the active decision on the lightness difference alone★** — the bar↔active tab contrast is around 1.07:1 in dark (measured). The active decision is carried together by four: lightness + text color (`--text` vs `--text-muted`) + the inactive divider + the always-visible `×`. **If you remove one of them, re-measure whether the rest hold.**
- **★Do not hide the close `×` with opacity★** — `opacity: 0` turns off only painting, while hit-test and tab order stay. To hide it, remove the render itself or exclude it from focus as well.
- **The status icon needs `role="img" + aria-label`** — without an accessibility prop lucide automatically attaches `aria-hidden` to the svg and erases the icon from the accessibility tree. In the character glyph days `textContent` played that role. Without it a Running row and an Exited row read the same.
- **★Do not use `--surface-elevated` or `--bg-secondary` as is for the tree selection background★** — they become the same color as the panel background in some themes. The regression net = "선택 배경은 패널 배경을 그대로 쓰지 않는다" (the selection background does not use the panel background as is) in `AgentList.test.tsx`.
- **★`◐` (waiting for input) is still forbidden to light up★** — this clause of ADR-0062 was not amended. The regression net moved from character comparison to a **range allowlist** (it fails if a branch is added that returns an icon outside the 4 shapes). However, it is still driven by an input list, so a status name not in the list falls to default and passes — the same limit as the old test.
- **`data-agent-glyph-shape` and `data-selected` are selector contracts** — deleting them or changing the tokens silently blinds the CDP measurement scripts and the LLM control path.
- **The font still depends on a system install** — this stack is guaranteed only on Windows. Making it pixel-identical in every environment needs `@font-face` bundling, and that is **open** (checking the license comes first). Not a rejection but a deferral.
- The `Inter` in `--font-claude-prose` and `--font-claude-header` is also not installed, but it is **outside this scope**, so it was left as is (chat body = inside a slot).
