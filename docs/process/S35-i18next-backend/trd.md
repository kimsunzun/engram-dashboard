# S35 TRD — i18next behind `t()`

- Status: edition 1 (2026-11-08) · canonical ADR = ADR-0349 · PRD = `prd.md` (approved 2026-11-08, user: 「1단계 부터 해」; D1–D3 main defaults stand)

## 1. Scope and relation to the PRD

Implements PRD R1–R7 / AC1–AC4: only the backend inside `src/i18n/index.ts` changes. `ko.ts`, the `StringKey` / `Placeholders` / `ParamsArg` types and the exported `t` signature stay byte-for-byte as they are.

## 2. Module boundary

- `src/i18n/index.ts` owns one private i18next instance (`i18next.createInstance(...)`, not the global singleton) created and initialized at module load. Nothing outside `src/i18n/` imports `i18next` (R7).
- `ko.ts` is untouched (still `as const` + deep-frozen). The instance gets `structuredClone(ko)` (or an equivalent deep copy) as `resources.ko.translation` (R6).

## 3. Init options (i18next 27.x)

| Option | Value | Why |
|---|---|---|
| `lng` / `fallbackLng` | `'ko'` / `false` | one language; no fallback chain to mask a missing key |
| `initAsync` | `false` | synchronous init with inline resources (R5); command titles call `t()` at module load |
| `resources` | `{ ko: { translation: <copy of ko> } }` | R1, R6 |
| `keySeparator` / `nsSeparator` | `'.'` / `false` | keys are `ns.key`; no `:` namespace parsing |
| `interpolation.prefix` / `suffix` | `'{'` / `'}'` | today's `{name}` grammar (R3) |
| `interpolation.escapeValue` | `false` | no HTML escaping (R4) |
| `interpolation.skipOnVariables` | `true` (default, set explicitly) | values are not re-scanned; a missing variable keeps the token (R3) |
| nesting (`$t(...)`) | disabled or proven inert | values come from user data (tab names); `$t(...)` in a value must stay literal |
| `returnNull` / `returnEmptyString` | `false` / keep values as is | today returns strings only |

Exact option names are to be confirmed against the installed i18next version's `typescript/options.d.ts`; if a name differs, use the version's equivalent and note it in the report.

## 4. `t()` adapter

- Missing key guard stays in the wrapper: if the key is not in the table (`instance.exists(key)` false, or no `.`), return the key unchanged without interpolation — today's "a miss stays loud" contract (R3).
- Pass params as interpolation values only, not as top-level i18next options: i18next gives top-level `count`, `context`, `lng`, `ns`, `defaultValue`, `ordinal`, `returnObjects` special meaning, and `ko.ts` uses `{count}` 14 times. Use the `replace` option (or whatever the version provides) so that a param named `count` never triggers plural-key lookup. Confirm in the source; if `replace` does not isolate `count`, report before working around it.
- Return type stays `string`.

## 5. Tests (`src/i18n/index.test.ts`)

- Every existing assertion through `t()` stays and must pass unchanged.
- Cases that reach into removed internals (`__test__.WORD_PLACEHOLDER`, `__test__.interpolate`) are rewritten to ride the real i18next path with the same assertion; keep a `__test__` seam that interpolates an arbitrary template through the same instance/interpolator (e.g. a temporary resource on a separate test instance), so the edge cases still test production behaviour.
- The out-of-contract raw-behaviour case (`{a{b}c`) records whatever i18next now does (it documents behaviour outside the `ko.ts` grammar contract, not normal input); report the old vs new value.
- New cases: a param named `count` returns the same string as before (no plural lookup); a value containing `$t(tab.create)` stays literal; a value containing `<b>&` is not escaped.
- The dev-guard (no malformed braces in `ko` values) and the deep-freeze case stay.
- The `@ts-expect-error` compile-time cases stay (AC4).

## 6. Test plan (qa)

- `npx vitest run` (all 124 files; AC1) and `tsc --noEmit` / `npm run build` (AC2) — qa binding rows for frontend.
- GUI spot check (AC3): command palette, right-click slot menu, tab names, empty-slot text show the same Korean text.
- `git diff --stat` limited to `src/i18n/`, `package.json`, `package-lock.json` and S35 docs (AC2).

## 7. Implementation units

One unit (one coder): add the `i18next` dependency (`npm install i18next@^27`), rewrite the inside of `src/i18n/index.ts`, adjust `index.test.ts` per §5. Build and tests green at the end.

## 8. Decisions

- Main: private instance via `createInstance` (no global state shared with any future library); params via `replace`; nesting off; `fallbackLng: false`.
- User: swap now, Korean only (ADR-0349); stage 1 only (2026-11-08).

## 9. Risks

- A param name colliding with an i18next option — handled by §4.
- Nesting expanding user text — handled by §3 and a test.
- Type-check slowdown from i18next types — our public types do not use i18next's `CustomTypeOptions`.

## 10. Review record

- 2026-11-08 · PRD+TRD review, deep (lead Claude + risk-axis Claude "behaviour parity" + cross-family GPT) on `522008f6` · **FIX** (all three) · not yet applied — topic paused by the user (「나중에 토큰 남을때」).
  - must (all three, reproduced by the risk-axis reviewer with a probe against i18next 27.0.0): i18next's matcher is `\{(.+?)\}` with name trimming, `{-x}`, `{x,fmt}` and dotted paths, so `{한글}`, `{first-name}`, `{ name }` get substituted — breaks R3 / AC1 (`index.test.ts:81-82`) and the type-runtime grammar sync. Main's pick (internal choice): keep our own `{\w+}` interpolate and call i18next with `skipInterpolation: true` (lookup only) — exact parity, nesting off as a side effect. The lead reviewer filed this as `hold` H1 because PRD §1 / ADR-0349 say "replace lookup-and-interpolate" — confirm with the user when resuming.
  - 권장: set `returnEmptyString: true` (v27 default `false` turns `''` into a key fallback that `exists()` does not catch); disable nesting explicitly (or via `skipInterpolation`); write the init as `createInstance().init(opts)` (createInstance alone does not init) and add a module-scope `t()` test for sync init; fix the §5 seam contradiction (seam stays in `index.ts`).
  - batched: §4 reasoning — a string `count` never triggers plurals in v27 (`replace` still right for `lng`/`ns`/`context`/`defaultValue`…); AC1 needs "except out-of-contract cases"; align R7 (`src/i18n/`) with ADR-0349 (`index.ts` only); update the `index.ts` header comment.
- 2026-11-08 · **User decision after this edition (not yet in PRD/TRD/ADR):** the UI source and default display language becomes English — `en.ts` becomes the typed source, `ko.ts` stays as a translation not shown yet, Korean display comes later with a language setting. Needs: a new ADR amending ADR-0349 ("ko.ts stays the source, Korean only"), CLAUDE.md 「언어」 (app screen text = Korean → English source), translating ~207 strings, the 2-3 bypassing strings and the Rust tray / refusal-dialog strings (main default: included), and updating tests that assert Korean UI text (up to ~85 files, rough count). PRD / TRD must be revised and re-reviewed before coding.
- Related, separate topic (S36, not opened): English for logs (user: yes), plus panic/assert messages, internal error strings, LLM-facing strings (main recommendation; user answer pending); test names excluded. Requires changing `docs/reference/logging-conventions.md` ("log message = one line of Korean"). Research notes: OSS keeps logs English-only and localizes only user-facing errors via keys/codes (Mattermost, OpenStack, Home Assistant; UE `UE_LOG` is not localized, `FText` is).
- 2026-11-08 · **User confirmed all pending choices (「ㅇㅇ 그렇게 하면 될것같은데?」):** (1) i18next for lookup only, our own `{\w+}` interpolate stays (`skipInterpolation: true`) — closes hold H1; (2) Rust tray + refusal-dialog strings go English in S35; (3) English labels in sentence case ("Close tab"; 관례 미확인 — check the Microsoft style guide before writing); (4) a Korean/English comparison table (HTML) for the user to skim before the S35 commit; (5) S36 = logs, panic/assert messages, internal error strings, LLM-facing strings to English + `logging-conventions.md` rule change + Korean prose in fenced blocks. Then a last topic: test names and test messages to English. Order: S35 → S36 → test names, sequentially.
