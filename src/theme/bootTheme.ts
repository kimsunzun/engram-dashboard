import { isThemeName, type ThemeName } from '../store/themeStore'
import { themeManager } from './ThemeManager'

// The first theme for outside the shell (a bare browser · opening the dev server directly) and for windows the shell does not plant one in — the same value as the themeStore initial value ·
//   the background fallback in index.css.
const DEFAULT_BOOT_THEME: ThemeName = 'dark'

/**
 * Attach the theme before the first paint and return that value — call it before `createRoot`.
 *
 * If the already planted `data-theme` is one of the three spellings, leave that value and only sync the store; if absent or an unknown spelling, plant dark.
 * The shell plants the window's effective theme when creating the window — on a reload, the theme that window last painted (`themeMemory.ts`) — before the page
 * script (`Backdrop::Theme` in `src-tauri/src/webview_env.rs`). ★Do not overwrite that value with dark★ — light · e-ink windows get drawn once in dark until the shell's theme arrives by IPC. Then
 * the value the shell gives (`uiSettings.ts`) wins.
 */
export function applyBootTheme(): ThemeName {
  const preset = document.documentElement.dataset.theme
  const theme = isThemeName(preset) ? preset : DEFAULT_BOOT_THEME
  themeManager.apply(theme)
  return theme
}
