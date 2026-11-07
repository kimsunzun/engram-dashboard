# ADR-0069: Centralizing UI strings — an i18n-ready strings module (full i18n deferred)

- 상태: 확정 (2026-07-12, basis: user decision — "centralize for now, localize later on top of that central source") · 부분 폐기 by ADR-0349 (full i18n deferred)
- 관련: CLAUDE.md §5 (LLM control) · §0 (low-risk, long-horizon seam) · `src/i18n/` (new) · step-log "UI 문자열 중앙화" entry · backlog "data-driven 우클릭 메뉴" · Amended by ADR-0349 (full i18n deferred)

## 맥락
Dashboard UI strings (command titles, right-click menu labels, component text, aria-labels, the default tab name `View 1`, the empty state `- empty -`, etc.) are **hardcoded inline** in Korean across the code. There is no i18n/locale infrastructure (checked package.json and `src/` — 0 libraries, `t()`, locale files). `src/commands/` alone has ~59 spots. The user pointed out: "we are hardcoding things like menu names in Korean, shouldn't they be moved to a local table?"

The two goals must be separated, since the scale differs: **centralization** (a single source — preventing drift + in §5 the LLM reading/editing labels in one place + a string source for the tutorial, low cost) vs **full i18n** (react-i18next + locale + `t()` runtime multilingual support, heavy). This is currently an internal Korean-only tool, and there is no plan for multiple languages.

## 결정
User decision = **centralize for now, later use that central source as the localization base**. Accordingly:
- Add a `src/i18n/` module — `ko.ts` (a namespaced `key → 한국어 문자열` table) + the `t(key, params?)` accessor in `index.ts`.
- **The key is a union type derived from the `ko` table** (type-safe) — tsc catches typos and missing keys at compile time.
- `t(key, params?)` **supports interpolation** (e.g. `t('tab.close', {name})` → replaces `{name}` in `'탭 닫기: {name}'`). For now it returns only `ko` (a single language).
- **Gradually migrate** user-visible UI strings to this `t()` (commands → components). New strings must not be inline and go through `t()`.
- Localization later = replace the `t()` backend with a locale-aware one (call sites and keys unchanged). Placing it in `src/i18n/` rather than `src/strings/` is the signal of that future.

## 거부한 대안
- **Full i18n now (react-i18next + locale + `t()` runtime)** — YAGNI and too heavy in runtime/layers for an internal tool with no plan for multiple languages. **Deferred, not discarded**: it is layered on top of this central source later (call sites unchanged).
- **Keep inline strings** — drift (aria-label ↔ displayed text mismatch), no §5 single source, and tutorial strings scattered throughout the code.
- **An untyped flat map (`Record<string,string>`)** — during migration, tsc cannot catch typos and nonexistent keys, so they leak at runtime as broken labels (empty/undefined). A type-safe key is the safety net for the bulk replacement.
- **The `src/strings/` name** — it does not reveal the localization intent. `src/i18n/` plants the "localize later" signal in the code map.

## 근거
- **CLAUDE.md §0 (low risk + long horizon → lay the seam down early):** the string access seam is low risk and expensive to change later (rework of every call site) → laid down now (the over-engineering-allowed zone).
- **§5 (LLM control):** a central table = a future surface where the LLM reads (informs) and changes (controls) all UI labels in one place — consistent with the backlog "data-driven 우클릭 메뉴".
- **Roadmap "tutorial Engram":** many strings are planned → a central source pays off.
- **Type-safe key:** a compile-time safety net for the bulk migration (missing/typo found immediately).

## 영향 / 불변식
- **key = stable API** — changing a key affects every call site (be careful). Changing string values is free.
- **Migration scope = user-visible UI strings** (command titles, menu labels, component text, aria-labels, default names, empty states). Internal `console.warn`/error logs are **out of scope** (not UI — a separate judgment).
- **Adding a locale later = only replacing the `t()` backend** — call sites and keys unchanged (that is why the current form is the localization base).
- **§5 LLM exposure** (a table read/override command) is a **later slice** (the MVP stops at centralization).
- **No new inline UI strings** — go through `t()` (a review checklist item).
