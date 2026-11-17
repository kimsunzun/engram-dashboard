# Localization: re-evaluating an OSS i18n library behind `t()` (2026-11-07)

- Status: reviewed (cross-family verdict FIX, fixes applied). Decision 2026-11-08: option A, i18next behind `t()` now, Korean only, English later (ADR-0349)
- Method: engram-research medium, design-decision mode. 3 collectors (peer survey, library candidates, current code + Rust side + migration cost), main-session grounding of load-bearing claims, 1 cross-family review (see "Review").
- Starting point: ADR-0069 (2026-07-12) centralized UI strings into `src/i18n/` (`ko.ts` table + typed `t()`) and deferred full i18n (react-i18next) as YAGNI, "deferred, not discarded". What changed: the user now wants real localization later (Korean stays the default) and prefers an OSS library over growing the home-made one.
- Confidence legend: **certain** = grounded and independently cross-checked; **likely** = grounded on one source, no counter-evidence; **uncertain** = not grounded, or from memory/snippet. Sections marked *(inference)* are our mapping onto this project, one level less sure than the facts they rest on.

## Bottom line

1. **Put i18next behind the existing `t()`; keep `ko.ts` and our public `t` signature as the typed contract; change no call sites.** (likely) It is the majority in the sampled React Tauri apps that localize, and it supports our `{name}` syntax and missing-param behaviour through config. The adapter must also set `escapeValue: false`, keep our missing-key guard in the wrapper, and feed i18next a mutable copy of the frozen `ko` table (see F4). Cost: about 2-4 files in `src/i18n/`.
2. **The library swap is the small part.** A second language chosen at startup needs a locale source (setting or OS locale), an English table, and i18n init before command registration. Live switching while running additionally needs lazy/refreshed command titles (52 command titles + 2 submenu titles are evaluated at registration, per the reviewer's registration probe) and React subscriptions for plain-`t()` consumers; installing react-i18next alone does not subscribe them. (likely)
3. **Rust side: no i18n library.** About 9 fixed user-visible strings originate in Rust (tray, native refusal dialog). A small match table in `src-tauri` is proportionate; error text should keep flowing as error kinds the frontend words (existing `failureKinds.ts` pattern). (likely)
4. **Reject Lingui and Paraglide as used idiomatically** (rewrite ~213 calls, `ko.ts` stops being the source, runtime label override from ADR-0069 §5 gets hard; Lingui supports explicit stable IDs, so hashed IDs are only its default, not a blocker). (likely)

## Constraints (from CLAUDE.md, ADR-0055/0069/0307, code)

- React 19 + TS 5.9 + Vite 6 (`@vitejs/plugin-react`, Babel), Vitest; Tauri v2 shell; Rust daemon. (certain, `package.json`, `vite.config.ts`)
- `t()` must work outside React and synchronously: command titles are built at registration (52 commands + 2 submenu containers per the reviewer's probe; a plain grep for `title: t(` gives 48 and mixes in dialog titles). (likely)
- Keys are a stable API; the LLM drives commands by `id`, not label, so localizing labels does not break `__engramCmd` control (ADR-0055 registry; `registry.list()` returns titles for discovery). (certain)
- ADR-0069 §5: the central table should become one surface the LLM can read and override at runtime → the library must keep an enumerable, overridable table. (certain about the ADR text; fit per library is likely)
- Korean stays the default language.

## Findings

### F1. Current home-made layer (certain — code)

- `src/i18n/ko.ts`: 207 keys, 10 namespaces, exactly two levels (`ns.key`); 57 values use `{word}` placeholders; no plurals/ICU/formatters.
- `src/i18n/index.ts`: `t<K extends StringKey>(key, ...params)`; keys are a union derived from `ko`; a type-level placeholder parser makes params required/forbidden per string; a missing param keeps the literal token; a missing key returns the key unchanged (`index.ts:129-136`).
- 33 non-test files import `t` (verified by grep and by the reviewer's AST scan), about 213 literal-key calls, 2 dynamic keys (`StructuredTextView.tsx:326`, `chat/ToolGroupRow.tsx:103`); every caller imports only `{ t }`, nothing else touches `ko` → **any backend can go in behind `index.ts` with zero call-site changes.**
- Visible strings bypassing `t()`: 2 Korean (`ConnectionNotice.tsx:55,60`), 1 likely (`wsFrame.ts:31`), about 7 hard-coded English labels (`WaitRow.tsx`, `StructuredTextView.tsx`). Non-JSX English strings in `.ts` files were not fully counted (gap).

### F2. Peer practice — React/Tauri apps (likely; sample biased to popular apps that localize at all)

- ~15 desktop apps checked via repo `package.json`; i18next + react-i18next in 12 of them (Clash Verge Rev, Pot, Readest, Nuclear, RapidRAW, Tabularis, CompressO, en-croissant, claude-code-history-viewer, PasteBar; Stump and MarkFlowy via shared i18n packages). RapidRAW spot-checked by main: `i18next ^26.4.0`, `react-i18next ^17.0.12`, `i18next-cli`. (likely)
- Home-made: NextChat (typed `LocaleType` TS object per language — closest analogue to `ko.ts`; spot-checked by main), Jan (custom `t()` over JSON despite declaring i18next). next-intl: 1 (Note-gen). Lingui, Paraglide, react-intl, Fluent on the frontend: none found in this sample (absence claim limited to the sample).
- Typed keys are bolted on with generators (`i18next-cli types`, custom scripts), not the default. (likely; scripts seen, usage not read)
- Rust side: most scanned Tauri repos have no i18n crate; Clash Verge Rev has a dedicated crate (rust-i18n + sys-locale, own catalog); Pot passes the language into Rust to build the tray. (likely)

### F2b. Peer practice — well-known OSS apps in general (likely; sample of ~35, 2026-11-08)

- i18next family 8 (Grafana, Outline, Rocket.Chat, Cal.com, AppFlowy-Web, AFFiNE, LobeChat, Open WebUI); react-intl / @formatjs (ICU) 7 (Mattermost, Mastodon, Signal Desktop, Strapi, tldraw, Zulip web; Home Assistant uses intl-messageformat under its own wrapper); home-made 8 (VS Code NLS, Element `_t`, Joplin, Excalidraw, Bitwarden, Jellyfin, Discourse, Home Assistant); gettext / PO 6 (GitLab, Sentry, Nextcloud, Penpot, Metabase and Standard Notes via ttag); vue-i18n 4 (n8n, Directus, Hoppscotch, Mealie); Fluent only in Mozilla (from memory); Lingui seen only through code-search hits (Bluesky social-app and others, not read).
- Main spot-checked repo manifests: Mastodon `react-intl 10.2.2`; Grafana `i18next ^26.3.6` + `react-i18next ^17.0.9`; Signal Desktop `react-intl 10.1.0` + `@formatjs/intl 4.1.4`; Outline `i18next ^22.5.1`; Excalidraw `TranslationKeys = NestedKeyOf<typeof fallbackLangData>` (a typed home-made layer like ours).
- Translation management: Crowdin 10, Weblate 5, Transifex 2, Lokalise 1 (likely).
- Typed keys are rare: confirmed only in Excalidraw (home-made) and AFFiNE (i18next + codegen); most apps rely on extraction plus CI sync checks.
- npm last week (2026-10-31..11-06, api.npmjs.org): i18next 34.8M, react-i18next 25.7M, use-intl 10.3M, next-intl 9.7M, @formatjs/intl 5.0M, vue-i18n 4.9M, react-intl 4.5M, @lingui/core 2.6M, @inlang/paraglide-js 1.4M.
- Reading *(inference)*: among React apps the choice splits into i18next (most common, largest ecosystem) and react-intl/FormatJS (ICU-first; chosen by Mattermost, Mastodon, Signal). A typed home-made layer is a recognised pattern (Excalidraw, VS Code), so option B is not unusual.

### F3. Candidates (versions certain — npm registry checked by main on 2026-11-07)

| Candidate | Latest | Fit to our seam | Notes |
|---|---|---|---|
| i18next 27.0.0 + react-i18next 18.0.0 | 2026-11-07 | **best** — runtime lookup, works outside React, `ko.ts` as resources | see F4 |
| use-intl 4.14.9 | 2026-11-06 | good — ICU `{name}` native, `typeof messages` typing | docs centred on Next.js; param typing for non-Next unclear (uncertain) |
| react-intl 12.1.6 | 2026-10-27 | shim possible via `createIntl` | params not inferred from text (uncertain); ICU escaping of `'` and braces |
| Lingui 6.12.0 (`@lingui/core`) | 2026-11-03 | poor unless idiomatic | macros + extraction + PO catalogs; hashed IDs by default, explicit IDs supported |
| Paraglide JS 3.2.1 (`@inlang/paraglide-js`) | 2026-11-07 | poor unless idiomatic | compiler → typed message functions; JSON source, not a TS object; runtime override awkward |
| typesafe-i18n 5.27.1 | 2026-02-11 | close in spirit | low activity (last publish 9 months ago) |

Bundle sizes (bundlephobia, collector-reported, not re-checked): i18next ~15 kB + react-i18next ~10 kB gzip; react-intl ~15 kB; use-intl ~13 kB; Lingui core ~2.5 kB. (uncertain)

### F4. i18next fit — grounded in the i18next 27.0.0 package source (certain)

- `interpolation.prefix` / `suffix` are runtime options (`typescript/options.d.ts:388,393`; regex built from them in `dist/esm/i18next.js:1181,1204`) → `{name}` works without touching `ko.ts`.
- The type layer reads `interpolationPrefix` / `interpolationSuffix` from `CustomTypeOptions` (`typescript/options.d.ts:177,182`; used in `typescript/t.d.ts:42-43,217`) → param types can follow our single-brace syntax.
- Missing variable: with no handler and the variable not passed, the default `skipOnVariables: true` (`dist/esm/i18next.js:1788`) keeps the token as is (`:1262-1268`) — same as today's behaviour. If the key is present in params with value `undefined`, it becomes `''` (differs; edge case).
- `initAsync` option exists (`options.d.ts:704`) for synchronous init with inline resources; `addResourceBundle` / `getResourceBundle` exist (`index.d.ts:555,572`) for the ADR-0069 §5 runtime read/override.
- **Conditions the adapter must meet (reviewer reproduced on 27.0.0; main re-checked the evidence):**
  - `escapeValue` defaults to `true` (`dist/esm/i18next.js:1161`), so `{name: 'A&B <C>'}` becomes `A&amp;B &lt;C&gt;` → set `escapeValue: false` (React escapes on render).
  - A missing key falls back to the key string **and interpolates it** (`t('missing.{name}', {name: 'X'})` → `missing.X`), breaking our "a miss stays loud" contract tested at `src/i18n/index.test.ts:65` → keep the existence check in our wrapper before calling i18next.
  - i18next's own types make `count` a `number` (`typescript/options.d.ts:797`) while our callers pass strings (`ToolGroupRow.tsx:103`), and they accept an options object for strings without placeholders → keep our public `t` types and adapt internally; do not switch callers to i18next's inferred signature.
  - `ko` is deep-frozen (`src/i18n/ko.ts:318`); `addResource`/`addResourceBundle` over it threw when replacing a nested value → initialize i18next with a deep copy, keep `ko.ts` for typing.
- Not verified: compile-time cost of i18next's types on our table (official guidance documents selector-based mitigation for large dictionaries).

### F5. Rust-originated visible strings (certain — code)

- Fixed: 4 tray labels (`src-tauri/src/tray/core.rs:37-40`), tray tooltip "Engram", native refusal dialog (`daemon_client/refusal.rs:144-153`, 4 strings) — about 9.
- Dynamic/raw: about 82 `Result<_, String>` Tauri commands feed `{err}` placeholders; daemon `Error.message` is shown raw at `StructuredTextView.tsx:612`. Developer-grade, mixed Korean/English.
- The tray exists before/without a webview and the refusal dialog deliberately bypasses the webview (`refusal.rs:1-5`) → these need a Rust-side table if localized. (certain about design intent)
- Rust options (rust-i18n, i18n-embed/fluent, tauri-plugin-i18n) exist; licenses/maturity from snippets only (uncertain). For ~9 strings a `match (locale, key)` table is proportionate *(inference)*.

## Options *(inference)*

- **A (recommended): i18next behind `t()` now, plus the second-language prerequisites when a second language is scheduled.** Swap the backend in `src/i18n/index.ts` (resources = deep copy of `ko`, prefix/suffix `{`/`}`, `escapeValue: false`, sync init, our missing-key guard kept), keep our public `t` signature and types, move the 2 Korean + ~7 English bypasses into the table. Later: lazy command titles, locale setting/OS detection, `en` table, `react-i18next` only if live switching must re-render, Rust tray/dialog match table. Trade-off: adds ~15 kB and a dependency today for a capability used later.
- **B: keep the home-made layer until a second language is actually scheduled, then do A.** The seam makes the later swap equally cheap (0 call sites either way), so waiting costs nothing in migration. Trade-off: the custom layer keeps growing features (plurals, locale switching) if needs arrive piecemeal.
- **C: use-intl / react-intl behind `t()`.** Viable; ICU gives plurals/select natively. Trade-off: fewer peers, weaker param typing (keep our `Placeholders` machinery), ICU escaping rules.
- **Rejected: Lingui, Paraglide (idiomatic).** Rewrite ~213 calls across 33 files, `ko.ts` stops being the source, runtime override (ADR-0069 §5) gets awkward, Lingui's hashed IDs conflict with stable keys. As a shim behind `t()` they lose their main benefits (extraction, tree-shaking).
- **Rejected: Rust i18n crate now.** ~9 strings; a match table covers it; revisit if plurals or translator workflow reach Rust.
- **Rejected: typesafe-i18n, rosetta.** Low activity / unmaintained.
- **Not assessed: Intlayer** (TypeScript declarations, Vite integration, provider-free sync reads per its docs; generated-dictionary workflow). Raised by the review as an omission; not evaluated here.

## Review

Cross-family reviewer (GPT via codex, effort high, independence level 2-3): **FIX**. Main re-checked each refutation's evidence in the package source / repo before applying it.

| # | Finding | Type | Label | Applied |
|---|---|---|---|---|
| 1 | Default HTML escaping and key-fallback interpolation break current behaviour unless configured/guarded | omitted condition, confident-wrong | refuted (reproduced) | F4 conditions, bottom line 1, option A |
| 2 | i18next inferred types are not interchangeable with ours (numeric `count`, extra options accepted) | overclaim | refuted | F4, bottom line 1 |
| 3 | Frozen `ko` cannot be overridden through i18next without a copy | dropped condition | refuted | F4, option A |
| 4 | 2 dynamic calls, not 1; 52 command + 2 submenu titles, not 48; startup vs live switching differ | wrong counts, scope gap | refuted | F1, constraints, bottom line 2 |
| 5 | "Majority among React Tauri apps" is a sample claim; named set is 12 | unsupported generalization | not-found | bottom line 1, F2 |
| 6 | Intlayer not assessed | omission | not-found | options (not assessed) |

Reviewer re-checked and confirmed: `ko` counts (207/10/57), 33 files / 213 calls, single-brace interpolation and sync init, missing-variable behaviour, ~9 Rust strings, higher cost of idiomatic Lingui/Paraglide. Reviewer could not reach the npm registry (versions corroborated only partially; main checked them with `npm view`). Reviewer found no reason to overturn i18next, and no technical necessity that decides A over B.

## Gaps / limits

- Peer sample = popular GitHub apps that localize; apps without i18n are under-represented. Absence of Lingui/Paraglide/react-intl is limited to this sample.
- Bundle sizes, i18next type-check cost, use-intl non-Next param typing, Rust crate licenses: not grounded.
- How many of the ~82 Rust error strings actually reach the screen: unknown.
- Which 16 of 207 keys are not hit by literal calls (dead vs dynamic): not checked.

## Sources

- Code: `src/i18n/index.ts`, `src/i18n/ko.ts`, `src/commands/*`, `src-tauri/src/tray/core.rs`, `src-tauri/src/daemon_client/refusal.rs`, `src/components/agent/failureKinds.ts`; ADR-0055, ADR-0069, ADR-0307.
- i18next 27.0.0 npm tarball (`npm pack i18next@27.0.0`): `typescript/options.d.ts`, `typescript/t.d.ts`, `dist/esm/i18next.js`, `index.d.ts`.
- npm registry (`npm view <pkg> version time.modified`, 2026-11-07).
- i18next docs: https://www.i18next.com/overview/typescript, https://www.i18next.com/translation-function/interpolation
- Lingui: https://lingui.js.org/tutorials/explicit-vs-generated-ids ; Paraglide: https://github.com/opral/paraglide-js ; react-intl: https://formatjs.github.io/docs/react-intl/ ; use-intl: https://next-intl.dev/docs/usage/typescript ; typesafe-i18n: https://github.com/ivanhofer/typesafe-i18n
- Peer repos (package.json via `gh api`): clash-verge-rev/clash-verge-rev, pot-app/pot-desktop, janhq/jan, readest/readest, nukeop/nuclear, CyberTimon/RapidRAW, TabularisDB/tabularis, codeforreal1/compressO, franciscoBSalgueiro/en-croissant, jhlee0409/claude-code-history-viewer, PasteBar/PasteBarApp, stumpapp/stump, drl990114/MarkFlowy, ChatGPTNextWeb/NextChat, codexu/note-gen.
- Collector notes (scratch, not committed): `i18n-collect-1.md` … `-4.md`.
