import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { useThemeStore } from '../store/themeStore'
import { applyBootTheme } from './bootTheme'

const root = document.documentElement

describe('applyBootTheme', () => {
  beforeEach(() => {
    root.removeAttribute('data-theme')
    useThemeStore.setState({ theme: 'dark' })
  })

  afterEach(() => {
    root.removeAttribute('data-theme')
  })

  it.each(['light', 'e-ink', 'dark'] as const)(
    '셸이 박은 %s 를 덮지 않고 스토어를 그 값에 맞춘다',
    preset => {
      root.dataset.theme = preset

      expect(applyBootTheme()).toBe(preset)
      expect(root.getAttribute('data-theme')).toBe(preset)
      expect(useThemeStore.getState().theme).toBe(preset)
    },
  )

  it('아무것도 박히지 않았으면 dark 를 박는다', () => {
    expect(applyBootTheme()).toBe('dark')
    expect(root.getAttribute('data-theme')).toBe('dark')
    expect(useThemeStore.getState().theme).toBe('dark')
  })

  it('모르는 철자가 박혀 있으면 dark 로 바꾼다', () => {
    useThemeStore.setState({ theme: 'light' })
    root.dataset.theme = 'sepia'

    expect(applyBootTheme()).toBe('dark')
    expect(root.getAttribute('data-theme')).toBe('dark')
    expect(useThemeStore.getState().theme).toBe('dark')
  })
})
