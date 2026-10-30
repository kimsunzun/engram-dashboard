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

  it('창 label 마다 따로 적는다', () => {
    rememberTheme('light')
    label = () => 'slot-popup-2'
    rememberTheme('e-ink')

    expect(sessionStorage.getItem(`${THEME_KEY_PREFIX}main`)).toBe('light')
    expect(sessionStorage.getItem(`${THEME_KEY_PREFIX}slot-popup-2`)).toBe('e-ink')
  })

  it('셸 밖이라 창 label 을 못 읽으면 적지 않고 던지지도 않는다', () => {
    label = () => {
      throw new TypeError('__TAURI_INTERNALS__ 없음')
    }

    expect(() => rememberTheme('light')).not.toThrow()
    expect(sessionStorage.length).toBe(0)
  })

  it('저장소가 거절해도 던지지 않는다', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError')
    })

    expect(() => rememberTheme('dark')).not.toThrow()
  })
})
