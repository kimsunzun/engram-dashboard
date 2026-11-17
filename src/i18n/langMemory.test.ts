// The per-window language memory writer (ADR-0373 decision 6) — the key the shell's init script reads, and best effort.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

let label: () => string = () => 'main'
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: label() }),
}))

import { LANG_KEY_PREFIX, rememberLanguage } from './langMemory'

describe('rememberLanguage', () => {
  beforeEach(() => {
    label = () => 'main'
    sessionStorage.clear()
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('writes engram.lang.<window label>, one key per window', () => {
    rememberLanguage('ko')
    label = () => 'slot-popup-2'
    rememberLanguage('en')

    expect(sessionStorage.getItem('engram.lang.main')).toBe('ko')
    expect(sessionStorage.getItem(`${LANG_KEY_PREFIX}slot-popup-2`)).toBe('en')
    expect(sessionStorage.length).toBe(2)
  })

  it('outside the shell (no window label) writes nothing and does not throw', () => {
    label = () => {
      throw new TypeError('no __TAURI_INTERNALS__')
    }

    expect(() => rememberLanguage('en')).not.toThrow()
    expect(sessionStorage.length).toBe(0)
  })

  it('a throwing sessionStorage is logged, not thrown', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('QuotaExceededError')
    })

    expect(() => rememberLanguage('ko')).not.toThrow()
    expect(warn).toHaveBeenCalledTimes(1)
  })

  it('a missing sessionStorage is logged, not thrown', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.stubGlobal('sessionStorage', undefined)

    expect(() => rememberLanguage('ko')).not.toThrow()
    expect(warn).toHaveBeenCalledTimes(1)
  })
})
