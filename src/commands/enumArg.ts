// The one place that folds an enum word coming in through the command argument bag to its **declared spelling**.
//
// ★Only letter case is folded — the valid set is not widened★: the caller may write any case, but the value that
//   goes out through here is always the spelling written in `declared`. An unknown word still falls to `undefined`, and
//   the call site does the rejection it used to do (the rejection wording · condition is each call site's concern — this module only gives the decision).

/**
 * Returns the item of `declared` equal to `raw` ignoring case. `undefined` if not found or not a string.
 *
 * ★The return is always an element of `declared` — not the string the caller gave★. The wire value going out to the backend must be
 * byte-identical to the declared spelling, so this return can be sent as is.
 *
 * ★Exact match is checked first★ — even on the day two spellings differing only in case enter `declared`, a caller who writes it exactly
 * receives the spelling they wrote (in that case the rest is resolved by declaration order, and such a set is never built in the first place).
 *
 * Folding uses `toLowerCase` (locale-independent) — `toLocaleLowerCase` folds `I` to `ı` in the Turkish locale,
 * so the same word would be decided differently from machine to machine.
 */
export function matchDeclaredSpelling<T extends string>(
  raw: unknown,
  declared: readonly T[],
): T | undefined {
  if (typeof raw !== 'string') return undefined
  if (declared.includes(raw as T)) return raw as T
  const folded = raw.toLowerCase()
  return declared.find(d => d.toLowerCase() === folded)
}
