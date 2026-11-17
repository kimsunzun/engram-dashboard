// ADR-0069: `t(key, params?)` is the stable API for UI strings — the signature, the keys and every call site stay unchanged
//   when what is behind it changes.
// ADR-0373: behind `t()` is a private i18next instance used for lookup only. Interpolation is ours (`interpolate`) — i18next's
//   own interpolation, with its prefix set to `{`, would also substitute `{한글}`, `{first-name}` and `{ name }`, which this
//   file's grammar leaves as text.
//
// The only importer of i18next. Imports neither React (`react.ts` holds `useLanguage`) nor Tauri (the per-window language
//   memory is `langMemory.ts`, written by its caller), so modules that only need `t` stay free of both.
// Boot language = `<html lang>`, read at module evaluation; absent or unknown → `en`. Today the value is `index.html`'s static
//   one; per ADR-0373 decision 6 the shell's init script is to plant it per window.
//   The instance is initialized synchronously at module evaluation, so `t()` works at module scope right after import.
//
// Type-safe keys: `StringKey` is the union of `ns.key` paths of `en` — typos and unknown keys fail tsc (ADR-0069).
// Type-safe params: the `{placeholder}` tokens of the `en` value literal make `params` required or forbidden. `ko` has the same
//   placeholder set per key (`Translation`, checked in `ko.ts`), so the params type holds for both languages.

import { createInstance } from 'i18next'

import { en, type EnTable } from './en'
import { ko } from './ko'

export type StringKey = {
  [N in keyof EnTable]: `${N & string}.${keyof EnTable[N] & string}`
}[keyof EnTable]

/**
 * Per-key precise indexing: a key-remap mapped type would crush the values into a union (placeholder presence gets mixed),
 * so it is not used.
 */
type ValueOf<K extends StringKey> = K extends `${infer N}.${infer S}`
  ? N extends keyof EnTable
    ? S extends keyof EnTable[N]
      ? EnTable[N][S]
      : never
    : never
  : never

/**
 * Grammar sync (FIX A): the type extraction `Placeholders` and the runtime `WORD_PLACEHOLDER` must see the same grammar. If
 *   they diverge, the type requires a param the runtime does not substitute (or the reverse). The grammar = a non-empty token
 *   of word chars (`[A-Za-z0-9_]`, i.e. `\w`). `{한글}` · `{first-name}` · `{ }` · `{}` are placeholders for neither side (no
 *   param required, the text is kept). Change both together.
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
 * The names of the `{word}` tokens in `S` (same grammar as `WORD_PLACEHOLDER` — FIX A).
 *
 * Non-greedy matching: `${string}{${infer P}}${infer Rest}` matches `P` up to the first `}`. When `P` fails the word check,
 *   like `{ }` or `{first-name}`, that token is dropped but `Rest` is still scanned, so valid later tokens survive.
 */
type Placeholders<S extends string> = S extends `${string}{${infer P}}${infer Rest}`
  ? (IsWord<P> extends true ? P : never) | Placeholders<Rest>
  : never

type SameSet<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false

type SamePlaceholders<A, B> = A extends string ? (B extends string ? SameSet<Placeholders<A>, Placeholders<B>> : false) : false

/** One namespace of a translation: `S` = the `en` namespace, `T` = the translated one. */
type TranslatedNamespace<S, T> = {
  readonly [K in keyof S | keyof T]: K extends keyof S
    ? K extends keyof T
      ? SamePlaceholders<S[K], T[K]> extends true
        ? T[K]
        : { readonly placeholdersMustBe: S[K] extends string ? Placeholders<S[K]> : never }
      : S[K]
    : never
}

/**
 * ADR-0372: the shape a translated table `T` must satisfy (`table satisfies Translation<typeof table>`) — exactly the
 * namespaces and keys of `en`, and per key the same placeholder set. A missing namespace or key is a missing property, an
 * extra one has type `never`, and a different placeholder set names the expected set (`placeholdersMustBe`).
 */
export type Translation<T> = {
  readonly [N in keyof EnTable | keyof T]: N extends keyof EnTable
    ? N extends keyof T
      ? TranslatedNamespace<EnTable[N], T[N]>
      : EnTable[N]
    : never
}

type ParamsFor<K extends StringKey> = { [P in Placeholders<ValueOf<K>>]: string }

/**
 * Tuple rest: with `t<K>(key: K, ...rest: ParamsArg<K>)` K is inferred straight from the first argument, and required or
 *   forbidden params are enforced by the tuple length.
 * Extra fields (FIX F): TS blocks extra fields only for an inline object literal; a pre-built object passes them under
 *   structural typing. At run time a field with no matching token is ignored (see 「여분 param」 in index.test.ts).
 */
type ParamsArg<K extends StringKey> = Placeholders<ValueOf<K>> extends never ? [] : [params: ParamsFor<K>]

/**
 * Runtime placeholder grammar (FIX A): must match the type extraction `Placeholders` / `IsWord` above.
 *
 * Nested or unmatched `{` (FIX③): in `{a{b}c` this matcher substitutes the inner `{b}` while `Placeholders` extracts nothing
 *   (it takes the first `{` as the opener and `a{b` fails the word check). A regex cannot track unmatched braces, so this is
 *   sealed by a contract instead: a table value holds only matched single `{\w+}` placeholders, enforced by the brace scan
 *   over both tables in index.test.ts.
 */
const WORD_PLACEHOLDER = /\{(\w+)\}/g

export type Lang = 'en' | 'ko'

/** Whether `value` is a UI language this build has a table for — the check for a value from outside (a setting, `<html lang>`). */
export function isLang(value: unknown): value is Lang {
  return value === 'en' || value === 'ko'
}

/** `globalThis.document` is absent in a node-environment test file. */
function bootLanguage(): Lang {
  const planted = globalThis.document?.documentElement?.lang
  return isLang(planted) ? planted : 'en'
}

let current: Lang = bootLanguage()
const listeners = new Set<() => void>()

const NS = 'translation'

const i18n = createInstance()
// Synchronous with inline resources and `initAsync: false`; the returned promise is already settled.
//   No nesting option is needed — the per-call `skipInterpolation` skips nesting too. `fallbackLng: false`: `ko` has every key
//   by type, so a miss stays a miss. `returnEmptyString: true`: an empty value stays `''` instead of turning into the key.
//   The resources are copies because the tables are frozen.
void i18n.init({
  lng: current,
  fallbackLng: false,
  initAsync: false,
  resources: {
    en: { [NS]: structuredClone(en) },
    ko: { [NS]: structuredClone(ko) },
  },
  keySeparator: '.',
  nsSeparator: false,
  returnNull: false,
  returnEmptyString: true,
  returnObjects: false,
})

/**
 * Test seam (FIX①): the production matcher and interpolation, so tests run the real thing instead of a copy; the
 * instance, so a test can plant a value no table holds (`$t(…)`, `''`) and restore it; and the subscriber count, so a test
 * can see an unsubscribe.
 */
export const __test__ = { WORD_PLACEHOLDER, interpolate, i18n, listenerCount: (): number => listeners.size }

/**
 * The value of `key` in the current language, uninterpolated; `undefined` on a miss (FIX E — a key that bypassed the type
 * gate through a cast or a JS caller). A miss = the key is not exactly `ns.key`, or its resolved resource value is not a
 * string (a namespace object, or nothing). The check and the lookup both name `current`, so they cannot disagree on the
 * language even if the instance's own language were changed elsewhere.
 */
function lookup(key: string): string | undefined {
  const dot = key.indexOf('.')
  if (dot <= 0 || dot === key.length - 1 || key.includes('.', dot + 1)) return undefined
  if (typeof i18n.getResource(current, NS, key) !== 'string') return undefined
  return i18n.t(key, { lng: current, skipInterpolation: true })
}

/**
 * A token not in params is left as the original text (no silent blank).
 * Grammar = `WORD_PLACEHOLDER`. The replace callback does not rescan substituted values, so `{token}` and regex specials
 * (`$&` · `$1` · `\`) inside a substituted value go in as literals.
 */
function interpolate(template: string, params: Record<string, string>): string {
  return template.replace(WORD_PLACEHOLDER, (whole, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? params[name] : whole,
  )
}

/**
 * The text of `key` in the current language with `{word}` tokens filled from `params`. Params never reach i18next, so names
 * like `count`, `lng` or `context` are plain values.
 *
 * A miss returns the key itself, uninterpolated (FIX②) — otherwise `missing.{name}` would become `missing.X` and the
 * "unknown key" signal would be lost.
 */
export function t<K extends StringKey>(key: K, ...rest: ParamsArg<K>): string {
  const template = lookup(key)
  if (template === undefined) return key
  const params = rest[0] as Record<string, string> | undefined
  return params ? interpolate(template, params) : template
}

export function getLanguage(): Lang {
  return current
}

/**
 * Switches the UI language: `t()` returns the new language from the next call, `<html lang>` follows (when there is a
 * document), and every subscriber is called once. Does nothing when `lang` is already current. Does not write the
 * per-window memory — that is `langMemory.ts`, called by whoever applies the setting.
 *
 * Returns `false` and changes nothing (no state, no `<html lang>`, no notification) when `lang` is not a known language —
 * a value that bypassed the type through a cast or a JS caller; `true` otherwise, including the no-op for the current value.
 */
export function setLanguage(lang: Lang): boolean {
  if (!isLang(lang)) return false
  if (lang === current) return true
  current = lang
  void i18n.changeLanguage(lang)
  const root = globalThis.document?.documentElement
  if (root) root.lang = lang
  for (const listener of [...listeners]) listener()
  return true
}

/** Calls `listener` after each language change; returns the unsubscribe function. */
export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
