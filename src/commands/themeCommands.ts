
import { t } from '../i18n'
import { type ThemeName, useThemeStore } from '../store/themeStore'
import { register } from './registry'

const THEMES: readonly ThemeName[] = ['dark', 'light', 'e-ink']

register({
  id: 'theme.set',
  title: t('theme.set'),
  category: 'theme',
  // ★이 값은 저장되지 않는다★ — 디스크 설정(`ui-settings.json`)이 다음 `ui.refresh` 에 덮는다
  //   (`src/theme/uiSettings.ts` 「쓰기 경로가 없다」). 부르는 쪽이 그 성질을 알아야 해서 설명에 적는다.
  // ★버스 호출자에겐 「이 창」이 없다★ — 셸이 고른 목적지 창 하나에서만 돈다. 설명이 그것을 말해야
  //   호출자가 「전 창을 바꾸려면 무엇을 불러야 하나」를 안다(그 답은 ui.refresh 다).
  help: {
    summary:
      '창 하나의 테마를 바꾼다(버스로 부르면 셸이 고른 창 = 보통 main 하나). 인메모리라 다음 ui.refresh 가 디스크 값으로 덮고, 전 창을 한꺼번에 바꾸려면 파일을 고친 뒤 ui.refresh 를 부른다.',
    effect: 'write',
    args: { theme: { type: 'string', enum: [...THEMES], description: '적용할 테마 이름' } },
    required: ['theme'],
  },
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
  help: {
    summary: `창 하나의 테마를 다음 것으로 돌린다(${THEMES.join(' → ')} 순환, 인메모리). 버스로 부르면 셸이 고른 창 = 보통 main 하나다.`,
    effect: 'write',
  },
  // ★불변식★: theme.toggle 은 useThemeStore 를 단일 진실원으로 신뢰한다(테마는 오직 setTheme 로만 바뀐다).
  //   store 를 우회해 data-theme 를 직접 바꾸면 여기 순환 기준(cur)이 어긋나므로 그런 경로를 두지 않는다.
  run: () => {
    const cur = useThemeStore.getState().theme
    const next = THEMES[(THEMES.indexOf(cur) + 1) % THEMES.length]
    useThemeStore.getState().setTheme(next)
  },
})
