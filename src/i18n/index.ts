// ★Interim implementation — own lightweight t()★: for now it is Korean-only, so zero dependencies with no external i18n library
//   is enough (research conclusion — for single-language · internal tools an own t() is justified, not an anti-pattern). When real multilingual support (plurals · ICU ·
//   dates · translator TMS etc.) gets serious, we plan to replace what is behind this t() (the backend) via the seam of the "stable API contract" below with a public library (Lingui = lightweight · Vite official /
//   i18next = largest ecosystem) — call sites · key · signature unchanged. This file is a transitional stopgap until then.
//   Basis: /research medium survey (2026-07-12, ADR-0069 reconfirmed).
//
// ADR-0069: the `t(key, params?)` accessor — the stable API for centralizing UI strings.
//
// ★Stable API contract★: for now it returns only a single language (`ko`), but later localization only needs to replace the *inside* of this function (the backend) with
//   locale-aware, and the `(key, params)` signature · all call sites · keys stay unchanged. So this shape is
//   the localization base (ADR-0069 §consequences/invariants).
//
// ★Type-safe key (required)★: make the `key` type a union derived from the `ko` table so tsc catches typos · nonexistent keys
//   at compile time (safety net of the mass migration — ADR-0069 rationale).
//
// ★Type-safe params★: extract `{placeholder}` tokens from value literals as a template-literal type and split params into
//   required/forbidden. `as const` in ko.ts is the premise of this inference.

import { ko, type KoTable } from './ko'

export type StringKey = {
  [N in keyof KoTable]: `${N & string}.${keyof KoTable[N] & string}`
}[keyof KoTable]

/**
 * ★per-key precise indexing★: a key-remap mapped type crushes sub-values into a union (placeholder presence gets mixed), so
 *   it is not used.
 */
type ValueOf<K extends StringKey> = K extends `${infer N}.${infer S}`
  ? N extends keyof KoTable
    ? S extends keyof KoTable[N]
      ? KoTable[N][S]
      : never
    : never
  : never

/**
 * ★Grammar sync (FIX A — top priority)★: the two definitions below (type extraction `Placeholders` ⊕ runtime `WORD_PLACEHOLDER`)
 *   **must see the same grammar**. If they diverge, a split arises where the type *requires* a param but the runtime *does not* substitute (or the reverse)
 *   (keys are a stable API, so this is fatal). The adopted grammar = **a non-empty word char (`[A-Za-z0-9_]`, i.e. `\w`) token**.
 *   `{한글}`·`{first-name}`·`{ }`·`{}` are *not recognized* as placeholders by either side (no param required + the original text is kept).
 *   If the runtime matcher (WORD_PLACEHOLDER) is changed, this type must be changed together.
 */
type WordChar =
  | 'a' | 'b' | 'c' | 'd' | 'e' | 'f' | 'g' | 'h' | 'i' | 'j' | 'k' | 'l' | 'm'
  | 'n' | 'o' | 'p' | 'q' | 'r' | 's' | 't' | 'u' | 'v' | 'w' | 'x' | 'y' | 'z'
  | 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G' | 'H' | 'I' | 'J' | 'K' | 'L' | 'M'
  | 'N' | 'O' | 'P' | 'Q' | 'R' | 'S' | 'T' | 'U' | 'V' | 'W' | 'X' | 'Y' | 'Z'
  | '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9' | '_'

/** Character class matches the runtime `\w+` (FIX A sync). */
type IsWord<T extends string> = T extends ''
  ? false
  : T extends `${WordChar}${infer R}`
    ? R extends ''
      ? true
      : IsWord<R>
    : false

/**
 * A token inside `{...}` is accepted as a param only when it is a **non-empty token made only of word chars (`\w`)**
 * (same grammar as the runtime `WORD_PLACEHOLDER = /\{(\w+)\}/g` — FIX A).
 *
 * ★Beware non-greedy matching★: `${string}{${infer P}}${infer Rest}` matches `P` minimally up to the first `}` in TS.
 *   When `P` fails word validation, like `{ }`·`{first-name}`, that token is dropped but `Rest` is scanned on so valid later tokens survive.
 */
type Placeholders<S extends string> = S extends `${string}{${infer P}}${infer Rest}`
  ? (IsWord<P> extends true ? P : never) | Placeholders<Rest>
  : never

type ParamsFor<K extends StringKey> = { [P in Placeholders<ValueOf<K>>]: string }

/**
 * ★Tuple rest approach★: with `t<K>(key: K, ...rest: ParamsArg<K>)` K is inferred straight from the first argument
 *   (not buried in a conditional type, so no inference failure), and params required/forbidden is enforced by the rest tuple length.
 * ★Beware extra fields (FIX F)★: TS's excess-property check blocks extra fields only when an **inline object literal** is passed.
 *   If a pre-built object variable is passed, extra fields pass through under structural typing (a TS limit — full blocking is not attempted).
 *   At runtime extra fields with no matching token are ignored, so it is harmless (see 「여분 param」 (extra param) in index.test.ts).
 */
type ParamsArg<K extends StringKey> = Placeholders<ValueOf<K>> extends never ? [] : [params: ParamsFor<K>]

/**
 * ★Runtime placeholder grammar (FIX A)★: a single matcher that **must match** the type extraction `Placeholders`/`IsWord` above.
 *   `\w+` = only a non-empty word char (`[A-Za-z0-9_]`) token is seen as a placeholder. If you change this, change the type side too.
 *
 * ★Type↔runtime grammar contract (FIX③ — sealing nested/unmatched `{`)★: if an opening `{` gets caught unmatched inside, as in `{a{b}c`,
 *   this runtime matcher substitutes the inner `{b}`, but the type-side `Placeholders` (consumes the first `{` as the opening brace →
 *   `a{b` fails word validation) extracts `never` → a split where **only the runtime silently substitutes**. This cannot be
 *   blocked with a pure regex (tracking unmatched opening braces is not regular — it cannot be expressed with a fixed-width lookbehind).
 *   So **we seal it with a contract**: *a table value's placeholder is only a matched single `{\w+}` and nested/unmatched
 *   `{` is not put in* (ko.ts value-writing convention). This contract is enforced by the dev-guard in index.test.ts (full ko scan), so
 *   if a violating value comes in the future the test fails at once → there is no silent split reachable from real table values.
 */
const WORD_PLACEHOLDER = /\{(\w+)\}/g

/** Test seam (FIX①): expose the production matcher/interpolation as is — the test rides this real thing instead of a local copy. */
export const __test__ = { WORD_PLACEHOLDER, interpolate }

/**
 * ★Runtime query seam★: for now it takes values from the single `ko` sheet only. Later localization only needs to change here (active locale selection) —
 *   signature · call sites unchanged (ADR-0069). `found` lets the call site (t) skip interpolation on a miss (FIX②).
 */
function lookup(key: StringKey): [value: string, found: boolean] {
  const dot = key.indexOf('.')
  // ★no-dot early return (FIX②)★: if there is no dot, do not make a garbage namespace with slice(0,-1) but return the key as is at once.
  //   (A key outside the format that came through the unsafe-boundary — treated as a miss and the original text is kept loudly.)
  if (dot < 0) return [key, false]
  const ns = key.slice(0, dot) as keyof KoTable
  const sub = key.slice(dot + 1)
  // ★unsafe-boundary backstop (FIX E)★: on the normal path the StringKey union above guarantees at compile time that the key exists.
  //   But at the JS/deserialization boundary (a runtime call that bypassed the type gate) a nonexistent key can come in — then
  //   so it does not blow up with `undefined.replace(...)`, the key string itself is returned as the fallback (loud-visible, no-throw).
  const value = (ko[ns] as Record<string, string> | undefined)?.[sub]
  return value === undefined ? [key, false] : [value, true]
}

/**
 * A token not in params is left as the original text (prevents loud-blank).
 * Grammar = `WORD_PLACEHOLDER` (= synchronized with the type-side `Placeholders`). The replace callback does not rescan substituted values, so
 * `{token}` · regex special characters (`$&`·`$1`·`\`) inside a substituted value go in as literals (no backreference interpretation).
 */
function interpolate(template: string, params: Record<string, string>): string {
  return template.replace(WORD_PLACEHOLDER, (whole, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? params[name] : whole,
  )
}

export function t<K extends StringKey>(key: K, ...rest: ParamsArg<K>): string {
  const [template, found] = lookup(key)
  const params = rest[0] as Record<string, string> | undefined
  // ★A miss stays loud (FIX②)★: if the key is not found, the fallback raw key string is **not interpolated**. Otherwise
  //   `missing.{name}` that came through the unsafe-boundary would be substituted into `missing.X` and the "nonexistent key" signal erased.
  //   On a miss, return the original text (the key as is) so it stays identifiable.
  if (!found) return template
  return params ? interpolate(template, params) : template
}
