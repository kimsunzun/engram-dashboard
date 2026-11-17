// ADR-0373: the per-window UI language memory — `sessionStorage[LANG_KEY_PREFIX + window label]`. The shell's init script
//   plants `<html lang>` from it on a reload (when it is `en` or `ko`), before the setting, so a reload does not flash the
//   old language. It is not the authority of the language: the setting `ui.language` still wins once it arrives.
// `index.ts` does not import this file — reading the window label pulls in Tauri.

import { getCurrentWindow } from '@tauri-apps/api/window'

import type { Lang } from './index'

// Must equal the prefix the shell's init script reads (ADR-0373 decision 6). Keep this one-line `export const` form — a
//   shell test is to pin the two by matching this line as text.
export const LANG_KEY_PREFIX = 'engram.lang.'

/** This window's key — `null` outside the shell (bare browser · tests), where there is no window label. */
function memoryKey(): string | null {
  try {
    return LANG_KEY_PREFIX + getCurrentWindow().label
  } catch {
    return null
  }
}

/**
 * Records `lang` as the language this window shows. Best effort: outside the shell nothing is written; a missing or
 * throwing `sessionStorage` is logged and otherwise ignored — the next reload then paints the language the window was
 * created with until the setting arrives.
 */
export function rememberLanguage(lang: Lang): void {
  const key = memoryKey()
  if (key === null) return
  try {
    globalThis.sessionStorage.setItem(key, lang)
  } catch (e) {
    console.warn('[i18n] could not write the window language memory:', e)
  }
}
