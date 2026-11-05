# ADR-0147: Delete the global `*` reset block — Tailwind preflight already covers it

- 상태: 확정 (2026-08-18, grounds: measured + user decision)
- 관련: ADR-0047(Tailwind adoption) · `src/index.css`

## 맥락

`src/index.css` had a `*, *::before, *::after { box-sizing; margin: 0; padding: 0 }` reset **outside any layer (unlayered)**. In the CSS cascade, an unlayered rule beats a rule inside `@layer` — Tailwind utilities go into `@layer utilities`, so **every padding and margin utility in this app was silently dead.**

How it was found: after a user report that the guidance text of the empty state input box sticks to the border, the horizontal spacing was raised from 16 to 24px, but **the screen did not change at all.** When a new element carrying only `px-6 py-4` was created in the running app and measured, the computed padding was `0px`, and when given inline it applied normally as 24px. It had been in this state ever since Tailwind adoption (ADR-0047).

## 결정

1. **Delete the reset block.** Tailwind preflight already does the same job, and it is inside `@layer base`, so it loses to utilities — our block was a duplicate that was not needed.
2. With this, the places that used spacing utilities (7 source files, about 25 tokens) are actually applied for the first time — the left and right indentation of chat rows, menu margins and so on come alive together.
3. **Do not put a global `*` padding·margin reset outside a layer again.** A unit test that keeps this prohibition is added (§영향).

## 거부한 대안

- **Move the reset inside `@layer base` (the first-draft decision)** — it was fixed this way first and the ADR was written that way. The ground was "if deleted, the default margins of elements that Tailwind preflight does not cover come back," but **review confirmed this is not true**: `node_modules/tailwindcss/preflight.css:7-16` applies `box-sizing: border-box; margin: 0; padding: 0; border: 0 solid` to `*, ::after, ::before, ::backdrop, ::file-selector-button`, and that preflight is inside `@layer base`. Our block is a **proper subset** of it (it covers the two pseudo-elements and border less, if anything), so there is no margin to come back. Moving the layer also behaves correctly, but leaving a rule that is not needed leaves a seed of the same mine (moving it back outside the layer) — so it was changed to deletion.
- **Force it with inline styles in just the two input boxes** — it covers only the symptom and leaves the same mine. The next person to use a spacing utility again experiences "I raised the value but the screen did not change."
- **Forbid spacing utilities and unify with dedicated CSS** — it partially invalidates the Tailwind adoption decision (ADR-0047). Utilities working was normal, and the reset was the abnormal part.

## 근거

- **Measured (2026-08-18)** — in the running release build, the computed value of a `px-6` element was `0px` and the inline specification was `24px`. That is, the class is generated but loses in the cascade.
- **User decision (2026-08-18)** — the user did not know the global rule existed, and after confirming chose the side of "그냥 푸는" (gloss: just releasing it).
- **Preflight comparison (review measurement 2026-08-18)** — the ground for being allowed to delete is not a guess but was confirmed by opening the installed Tailwind file (the path and range in 「거부한 대안」 item 1 above).

## 영향 / 불변식

- **If a global `*` padding·margin reset is placed outside a layer, all spacing utilities across the app die.** The symptom is quiet — the classes remain visible and only the screen does not change, and **the tests stay green** (in fact 695 passed in that state). So this prohibition is kept by a unit test, not by a document.
- **Unlayered CSS beats utilities** — the padding·margin that `structuredItems.css` · `chat.css` · `scroll-area.css` put directly on classes still beat utilities. This decision does not change those files.
- **The feel for margin values has changed** — the earlier values were "values that were not applied," so their sizes were never verified. When adjusting margins, confirm on screen.
