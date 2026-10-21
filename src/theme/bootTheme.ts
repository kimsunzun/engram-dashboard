import { isThemeName, type ThemeName } from '../store/themeStore'
import { themeManager } from './ThemeManager'

// 셸 밖(맨 브라우저 · dev 서버 직접 열기)과 셸이 박지 않는 창의 첫 테마 — themeStore 초기값 ·
//   index.css 의 바탕 폴백과 같은 값이다.
const DEFAULT_BOOT_THEME: ThemeName = 'dark'

/**
 * 첫 페인트 전에 테마를 붙이고 그 값을 돌려준다 — `createRoot` 보다 먼저 부른다.
 *
 * 이미 박힌 `data-theme` 이 세 철자 중 하나면 그 값을 그대로 두고 스토어만 맞추고, 없거나 모르는 철자면 dark 를 박는다.
 * 셸은 창을 만들 때 그 창의 유효 테마를 — 다시 읽기면 그 창이 마지막으로 칠한 테마(`themeMemory.ts`)를 — 페이지
 * 스크립트보다 먼저 박는다(`src-tauri/src/webview_env.rs` 의 `Backdrop::Theme`). ★그 값을 dark 로 덮지 말 것★ — light · e-ink 창이 셸의 테마가 IPC 로 닿기까지 dark 로 한 번
 * 그려진다. 그 뒤 셸이 주는 값(`uiSettings.ts`)이 이긴다.
 */
export function applyBootTheme(): ThemeName {
  const preset = document.documentElement.dataset.theme
  const theme = isThemeName(preset) ? preset : DEFAULT_BOOT_THEME
  themeManager.apply(theme)
  return theme
}
