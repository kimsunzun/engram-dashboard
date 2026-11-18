import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

let label: () => string = () => 'main'
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: label() }),
}))

import { rememberTheme, THEME_KEY_PREFIX } from './themeMemory'

describe('rememberTheme', () => {
  beforeEach(() => {
    label = () => 'main'
    sessionStorage.clear()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('writes separately per window label', () => {
    rememberTheme('light')
    label = () => 'slot-popup-2'
    rememberTheme('e-ink')

    expect(sessionStorage.getItem(`${THEME_KEY_PREFIX}main`)).toBe('light')
    expect(sessionStorage.getItem(`${THEME_KEY_PREFIX}slot-popup-2`)).toBe('e-ink')
  })

  it('outside the shell, when the window label cannot be read, it neither writes nor throws', () => {
    label = () => {
      throw new TypeError('no __TAURI_INTERNALS__')
    }

    expect(() => rememberTheme('light')).not.toThrow()
    expect(sessionStorage.length).toBe(0)
  })

  it('does not throw even when the storage refuses', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError')
    })

    expect(() => rememberTheme('dark')).not.toThrow()
  })
})
