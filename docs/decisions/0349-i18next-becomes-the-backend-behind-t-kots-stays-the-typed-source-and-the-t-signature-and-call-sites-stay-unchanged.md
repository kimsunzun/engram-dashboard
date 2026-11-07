# ADR-0349: i18next becomes the backend behind t(); ko.ts stays the typed source and the t() signature and call sites stay unchanged

- 상태: 확정 (2026-11-08, basis: user decision 「ㅇㅇ 교체하고」 after the research report below)
- 관련: Amends ADR-0069 (full i18n deferred) · `src/i18n/` · `docs/research/i18n-oss-library-reevaluation-2026-11-07.md` · ADR-0055 (command registry, LLM addresses commands by id) · step-log S35

## 맥락
ADR-0069 centralized UI strings into `src/i18n/` (`ko.ts` table + a typed `t(key, params?)`) and deferred full i18n (react-i18next) as YAGNI, "deferred, not discarded". The user now wants real localization later (Korean stays the default, English added later) and prefers an OSS library over growing the home-made layer. The home-made backend only looks a key up in `ko.ts` and fills `{name}` placeholders; it has no plurals, language switching or fallback.

## 결정
- Replace the inside of `t()` with i18next. `ko.ts` stays the single source and the type source of keys and placeholders; i18next receives a deep copy of it as the `ko` resources.
- The public `t()` signature and its types stay as they are, so no call site changes (33 files, about 213 literal-key calls).
- Korean only for now. Adding English later = an `en` table plus a locale source; live switching additionally needs lazily rebuilt command titles and React subscriptions (out of this decision's scope).
- The adapter must keep today's behaviour: `interpolation.prefix/suffix` = `{` / `}`; `escapeValue: false` (React escapes on render); the wrapper keeps its own missing-key guard (a missing key returns the key uninterpolated); init is synchronous with inline resources, before any command module registers titles.
- Rust-side visible strings (tray labels, native refusal dialog, about 9) are not part of this decision.

## 거부한 대안
- **Keep the home-made backend until a second language is scheduled** — the later swap would cost the same, but the user chose to swap now so that the library change and the English addition do not land together. (약함 — 사용자 직접 기각)
- **react-intl / use-intl behind `t()`** — fewer peers in the surveyed React/Tauri apps (sample: i18next 12 of ~15 Tauri apps, 8 of ~35 well-known OSS apps vs react-intl 7), and no parameter-type inference from message text, so our placeholder typing would stay hand-rolled anyway.
- **Lingui or Paraglide (idiomatic)** — rewrite of about 213 call sites across 33 files, `ko.ts` stops being the source (PO / JSON catalogs), runtime override of labels (ADR-0069 §5) gets hard with compiled message functions. As a shim behind `t()` they lose their main benefits (extraction, tree-shaking).
- **A Rust i18n crate (rust-i18n, fluent)** — only about 9 fixed user-visible strings originate in Rust; a small table covers them if they are ever localized.

## 근거
- Research report `docs/research/i18n-oss-library-reevaluation-2026-11-07.md` (engram-research medium, cross-family review FIX applied): i18next is the most common choice in the sampled React apps and by npm downloads (34.8M/week); every caller imports only `{ t }`, so the backend swap touches about 2-4 files.
- i18next 27.0.0 package source (checked): `interpolation.prefix/suffix` are runtime options and the type layer reads `interpolationPrefix/Suffix`; a missing variable keeps the token (`skipOnVariables: true`), matching today; `escapeValue` defaults to `true`; a missing key falls back to the key string and interpolates it — hence the wrapper guard; `ko` is deep-frozen (`src/i18n/ko.ts`), and `addResource` over it throws — hence the copy.
- i18next's own inferred `t` types make `count` a number while callers pass strings, so the public `t` types stay ours.

## 영향 / 불변식
- `src/i18n/index.ts` is the only place that touches i18next; no other module imports i18next directly.
- `ko.ts` remains the key and placeholder contract; keys stay a stable API (ADR-0069).
- `src/i18n/index.test.ts` keeps pinning the current behaviour (lookup, `{name}` interpolation, missing-key stays loud, missing-param keeps the token); a backend change that breaks one of these is a regression.
- New UI strings still go through `t()`; features like plurals or language switching are added through i18next, not by growing a home-made layer.
