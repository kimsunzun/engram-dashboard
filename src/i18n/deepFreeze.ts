// FIX E: `as const` is compile-time readonly only — a JS consumer (or a deserialization boundary) could still write
//   `table.tab.close = ...` at run time. The string tables are frozen at module load so they cannot be changed.

/** Freezes `obj` and every object reachable through its own enumerable values; returns `obj`. Types are unchanged. */
export function deepFreeze<T>(obj: T): T {
  if (obj && typeof obj === 'object') {
    for (const value of Object.values(obj)) deepFreeze(value)
    Object.freeze(obj)
  }
  return obj
}
