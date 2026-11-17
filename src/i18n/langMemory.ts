// ADR-0373: the per-window UI language memory — `sessionStorage[LANG_KEY_PREFIX + window label]`. The shell's init script
//   is to plant `<html lang>` from it on a reload (when it is `en` or `ko`), before the setting, so a reload does not flash the old
//   language. This file only holds the key prefix for now; the writer comes with the live-switch wiring.
// `index.ts` does not import this file — reading the window label pulls in Tauri.

// Must equal the prefix the shell's init script reads (ADR-0373 decision 6). Keep this one-line `export const` form — a
//   shell test is to pin the two by matching this line as text.
export const LANG_KEY_PREFIX = 'engram.lang.'
