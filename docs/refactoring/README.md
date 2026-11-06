# Refactoring — where surveys, boundaries and plans are collected

**Purpose = browse in one place.** When refactoring material got mixed into `research/`, it was buried among 20-odd surveys and could not be found again (user's point, 2026-08-23 — the reason this folder exists).

## What goes here

- **Measuring split/merge boundaries** — how far can we cut things out, and in which direction do dependencies flow.
- **Measuring structural debt** — the cost the current structure pays, measured in numbers.
- **Refactoring plans and order** — what first, and bundled with what.

## What does not go here

| It goes there | Why |
|---|---|
| Design **decisions** (+ rejected alternatives) → `../decisions/` | Documents here are not decisions. Once a decision is made, record it as an ADR and link it from here. |
| General prior surveys (OSS comparisons · industry practice) → `../research/` | If it is not refactoring material, it goes to its original place. |
| **Registering deferred items** → `../tracking.md` (T-) | The fact that 「나중에 다룬다」 has its source of truth there. |

★**Use together with tracking items**★ — the T- items in `tracking.md` carry **"what is pending"**, and the documents here carry **"the measured result"**. Link the two to each other, and do not copy the same content to both sides.

## List

This table is the browsing surface of this folder (there is no other source of truth, so it is maintained here — **when you add a document, add a line here too**).

| Document | What | Status |
|---|---|---|
| `shell-crate-boundary-survey-2026-08-23.md` | How much of the shell (`src-tauri`, 10,637 lines) can be cut out into crates — `use tauri` measurement + dependency direction | Boundary measurement done · **shape undecided** (user reviewing) · link = T-27 |
| `socket-head-of-line-blocking-2026-09-17.md` | A single activation blocks the whole client↔daemon socket receive — 3 code points + a refutation of 「직렬은 가드가 아니다」 | Structural-debt measurement · **folded into the network refactoring** (user decision 2026-09-17) |
| `slot-docking-drag-code-audit-2026-10-18.md` | Structural audit of the slot-docking drag code (9c tree) — duplication · giant closures · dormant scroll plumbing · what to keep / what to clear away, judged against a Chrome-style redesign (ADR-0296 replaced that direction with an OS drag) | Measurement done · direction = ADR-0296 (OS drag) · source of truth for the clearing scope = TRD edition 7 §5 · companion survey = `../research/slot-docking-chrome-desktop-drag-2026-10-18.md` |
| `architecture-discussion-2026-09-26.md` | Discussion of the whole dependency graph and module splitting before transport attachment — facts and decision candidates from the module-order Q&A | **Discussion complete** (2026-10-02) · ADR-0266~0273 written (2026-10-02/03) · candidate 7 (clearing net after transport attachment) deferred to the transport TRD · candidate 3 (logs only via base) reversed — we do not wrap (ADR-0268) · candidate 2 (base dependency of `command`·`messaging`) withdrawn — connect only when needed (ADR-0267) · major section A (logging system design) requirements collected · link = T-47 |
