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
    'does not overwrite the %s the shell stamped and aligns the store to that value',
    preset => {
      root.dataset.theme = preset

      expect(applyBootTheme()).toBe(preset)
      expect(root.getAttribute('data-theme')).toBe(preset)
      expect(useThemeStore.getState().theme).toBe(preset)
    },
  )

  it('stamps dark when nothing is stamped', () => {
    expect(applyBootTheme()).toBe('dark')
    expect(root.getAttribute('data-theme')).toBe('dark')
    expect(useThemeStore.getState().theme).toBe('dark')
  })

  it('changes an unknown spelling that is stamped to dark', () => {
    useThemeStore.setState({ theme: 'light' })
    root.dataset.theme = 'sepia'

    expect(applyBootTheme()).toBe('dark')
    expect(root.getAttribute('data-theme')).toBe('dark')
    expect(useThemeStore.getState().theme).toBe('dark')
  })
})
