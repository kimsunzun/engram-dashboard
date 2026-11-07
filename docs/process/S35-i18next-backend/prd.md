# S35 PRD — i18next behind `t()`

- Status: approved (2026-11-08, user: 「1단계 부터 해」) · decision = ADR-0349 · research = `docs/research/i18n-oss-library-reevaluation-2026-11-07.md`

## 1. What · why

Replace the home-made lookup-and-interpolate code inside `src/i18n/index.ts` with i18next, keeping `ko.ts` as the single, typed source and keeping the public `t()` exactly as callers use it today. Why: the user wants real localization later (Korean default, English added later) on a mainstream library rather than a home-made layer that would have to grow plurals, fallback and language switching by itself. Doing the swap now, alone, keeps it separate from the later English work.

## 2. Scope / non-goals

- In scope: the backend of `t()` (`src/i18n/index.ts`), the i18next dependency, init before command registration, `src/i18n/index.test.ts` adjustments that keep its contract.
- Non-goals: an English table; a language setting or OS-locale detection; live language switching (lazy command titles, React subscriptions, `react-i18next`); moving the 2-3 Korean strings that bypass `t()`; Rust-side strings; any change to `ko.ts` keys or values; LLM read/override of labels (ADR-0069 §5 later slice).

## 3. Requirements

- R1 — `t()` looks strings up through i18next; `ko.ts` stays the source of keys and values and the source of the `StringKey` / placeholder types.
- R2 — The public `t()` signature and types are unchanged: no call site changes, params stay required/forbidden per string as today, values stay `string`.
- R3 — Behaviour parity with today: `{name}` single-brace interpolation; global replacement of repeated tokens; values are not re-scanned for tokens; regex-special characters in values are literal; extra params ignored; a missing param keeps the `{token}` text; a missing key returns the key unchanged and uninterpolated; braces outside the `{word}` grammar stay literal.
- R4 — No HTML escaping of values (React escapes on render).
- R5 — i18next is initialized synchronously with inline resources before any module calls `t()` at load (command titles).
- R6 — The frozen `ko` table stays frozen; i18next gets a copy.
- R7 — Only `src/i18n/` imports i18next.

## 4. Acceptance criteria

- AC1 — `npx vitest run` green; the existing `src/i18n/index.test.ts` cases still pass (rewritten only where they reach into removed internals, keeping the same assertion).
- AC2 — `tsc` green with no call-site edits (`git diff --stat` shows no files outside `src/i18n/`, `package.json`, the lock file and the S35 docs).
- AC3 — In the running app, the command palette / right-click menus / tab names / empty-slot text show the same Korean strings as before (GUI spot check).
- AC4 — A type error still appears when a caller passes a wrong key or omits a required param (kept compile-time tests).

## 5. Open decisions (main defaults)

- D1 — Install `react-i18next` now? Default: no — nothing re-renders on language change yet; add it with live switching.
- D2 — Keep our own `Placeholders` type machinery for the public `t()` (default) vs switch callers to i18next's inferred types — rejected in ADR-0349 (i18next types `count` as a number while callers pass strings).
- D3 — Where init runs: inside `src/i18n/index.ts` at module load (default; any import of `t` triggers it, so ordering is automatic) vs a separate setup call in `main.tsx` (ordering risk with command modules).

## 6. Risks

- Subtle interpolation differences (value re-scan, `$&` handling, missing-param behaviour) — pinned by the existing edge tests (R3).
- i18next type-check cost on the table — our public types stay; i18next's `CustomTypeOptions` is not required.
- Bundle size +~15 kB gzip — acceptable for a desktop app.
