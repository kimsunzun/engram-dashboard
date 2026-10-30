// 이 창이 마지막으로 칠한 테마를 그 웹뷰의 sessionStorage 에 적는다 — 셸이 창을 만들며 넣은 초기화 스크립트가 다시
// 읽기(F5 · HMR 전체 새로고침) 때 이 값을 먼저 박는다(`src-tauri/src/webview_env.rs` 의 `THEME_MEMORY_PREFIX`). 그
// 스크립트는 창을 만들 때의 테마로 굳어 있어, 이것이 없으면 그 뒤 테마가 바뀐 창이 다시 읽힐 때 옛 테마로 한 번 그려진다.
//
// ★테마의 권위가 아니다★ — 다시 읽은 뒤의 첫 페인트만 맡고, 셸의 부팅 조회 · 밀기(`uiSettings.ts`)가 그대로 이긴다.
// ★localStorage 로 바꾸지 말 것★ — 사유는 셸 쪽 상수의 doc.

import { getCurrentWindow } from '@tauri-apps/api/window'

import type { ThemeName } from '../store/themeStore'

// ★셸 `THEME_MEMORY_PREFIX` 와 같은 철자★ — 셸 시험이 이 줄을 그대로 찾는다(`export const …` 형태를 바꾸면 그 시험도).
export const THEME_KEY_PREFIX = 'engram.theme.'

/** 이 창의 키 — 셸 밖(맨 브라우저 · 시험)이라 창 label 을 못 읽으면 `null`. */
function memoryKey(): string | null {
  try {
    return THEME_KEY_PREFIX + getCurrentWindow().label
  } catch {
    return null
  }
}

/** 실패(셸 밖 · 저장소를 못 씀)는 삼킨다 — 놓치면 다음 다시 읽기의 첫 페인트가 창을 만들 때의 테마일 뿐이다. */
export function rememberTheme(theme: ThemeName): void {
  const key = memoryKey()
  if (key === null) return
  try {
    sessionStorage.setItem(key, theme)
  } catch {
    // 위 doc.
  }
}
