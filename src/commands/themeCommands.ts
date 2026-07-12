// ADR-0055: 첫 어댑터 — 테마 command. register 로 기존 store 액션(useThemeStore.setTheme)에 라우팅만
//   한다(새 상태 경로 0). import 부수효과로 등록되므로 부팅 경로(App.tsx)에서 side-effect import 한다.
//   검증: window.__engramCmd.run('theme.set',{theme:'light'}) → document.documentElement.dataset.theme.

import { t } from '../i18n'
import { type ThemeName, useThemeStore } from '../store/themeStore'
import { register } from './registry'

const THEMES: readonly ThemeName[] = ['dark', 'light', 'e-ink']

register({
  id: 'theme.set',
  title: t('theme.set'),
  category: 'theme',
  // args.theme 만 destructure(단일 객체 가방, ADR-0055). 유효 테마명 검증 후 기존 setter 로 라우팅.
  run: (args) => {
    const theme = args?.theme as ThemeName | undefined
    if (!theme || !THEMES.includes(theme)) {
      throw new Error(`[theme.set] 알 수 없는 테마: ${String(theme)} (허용: ${THEMES.join(', ')})`)
    }
    useThemeStore.getState().setTheme(theme)
  },
})

register({
  id: 'theme.toggle',
  title: t('theme.toggle'),
  category: 'theme',
  keybinding: 'Ctrl+Shift+T',
  // 현재 테마 다음 것으로 순환. 상태는 읽기만 하고 실행은 기존 setter 로 라우팅(권위는 store 유지).
  // ★불변식★: theme.toggle 은 useThemeStore 를 단일 진실원으로 신뢰한다(테마는 오직 setTheme 로만 바뀐다).
  //   store 를 우회해 data-theme 를 직접 바꾸면 여기 순환 기준(cur)이 어긋나므로 그런 경로를 두지 않는다.
  run: () => {
    const cur = useThemeStore.getState().theme
    const next = THEMES[(THEMES.indexOf(cur) + 1) % THEMES.length]
    useThemeStore.getState().setTheme(next)
  },
})
