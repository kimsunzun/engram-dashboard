// TRD S21-slot-docking §3-3 · §4-2: a mirror of shell `layout::manager::succession` — the next active the shell picks on tab close · drop, and
//   the remaining tab the source window shows during a drag must be the same tab so the screen does not change after the drop. If the rule is changed, change it together with the shell version
//   (the same table test exists on both sides — shell `succession_gives_the_same_tab_as_closing`).

/** The tab to succeed as active when `removed` is removed from the tab list `tabs` — the right neighbor first, otherwise the left. `null` if `removed` is not in the list or is alone. */
export function succession(tabs: readonly string[], removed: string): string | null {
  const pos = tabs.indexOf(removed)
  if (pos < 0) return null
  return tabs[pos + 1] ?? tabs[pos - 1] ?? null
}
