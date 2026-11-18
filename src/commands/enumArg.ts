// The one place that folds an enum word from the command argument bag to its **declared spelling**. ★Only letter case is folded — the valid set is not widened★: the value going out is always the spelling in `declared`; an unknown word still falls to `undefined` and each call site does its own rejection (wording · condition).

/**
/**
 * Returns the item of `declared` equal to `raw` ignoring case; `undefined` if not found or not a string.
 * ★The return is always an element of `declared`, not the caller's string★ — the wire value to the backend must be byte-identical to the declared spelling.
 * ★Exact match is checked first★ — even if two spellings differing only in case enter `declared`, a caller writing it exactly gets what they wrote.
 * Folding uses `toLowerCase` (locale-independent) — `toLocaleLowerCase` folds `I` to `ı` in the Turkish locale, so the same word would be decided differently per machine.
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
