
import { t } from '../i18n'
import { type ThemeName, useThemeStore } from '../store/themeStore'
import { register } from './registry'

const THEMES: readonly ThemeName[] = ['dark', 'light', 'e-ink']

register({
  id: 'theme.set',
  title: t('theme.set'),
  category: 'theme',
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
  // ★불변식★: theme.toggle 은 useThemeStore 를 단일 진실원으로 신뢰한다(테마는 오직 setTheme 로만 바뀐다).
  //   store 를 우회해 data-theme 를 직접 바꾸면 여기 순환 기준(cur)이 어긋나므로 그런 경로를 두지 않는다.
  run: () => {
    const cur = useThemeStore.getState().theme
    const next = THEMES[(THEMES.indexOf(cur) + 1) % THEMES.length]
    useThemeStore.getState().setTheme(next)
  },
})
