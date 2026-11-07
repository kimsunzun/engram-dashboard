// Writes the theme this window last painted into that webview's sessionStorage — the init script the shell put in when creating the window plants this value first on a
// reload (F5 · HMR full refresh) (`THEME_MEMORY_PREFIX` in `src-tauri/src/webview_env.rs`). That
// script is frozen at the theme at window creation time, so without this, a window whose theme changed afterwards is drawn once in the old theme on reload.
//
// ★It is not the authority of the theme★ — it handles only the first paint after a reload, and the shell's boot query · push (`uiSettings.ts`) still wins.
// ★Do not change it to localStorage★ — the reason is in the doc of the shell-side constant.

import { getCurrentWindow } from '@tauri-apps/api/window'

import type { ThemeName } from '../store/themeStore'

// ★Same spelling as the shell `THEME_MEMORY_PREFIX`★ — the shell test looks for this line verbatim (if you change the `export const …` shape, that test too).
export const THEME_KEY_PREFIX = 'engram.theme.'

/** This window's key — `null` if the window label cannot be read because it is outside the shell (bare browser · tests). */
function memoryKey(): string | null {
  try {
    return THEME_KEY_PREFIX + getCurrentWindow().label
  } catch {
    return null
  }
}

/** A failure (outside the shell · storage cannot be written) is swallowed — if missed, the first paint of the next reload is just the theme at window creation. */
export function rememberTheme(theme: ThemeName): void {
  const key = memoryKey()
  if (key === null) return
  try {
    sessionStorage.setItem(key, theme)
  } catch {
    // Doc above.
  }
}
