// Formatter locale follows the UI language (ADR-0373) — made per language when used, not at definition.

import { afterEach, describe, expect, it, vi } from 'vitest'

import { setLanguage } from './index'
import { perLanguage } from './intlLocale'

afterEach(() => {
  setLanguage('ko')
})

describe('perLanguage', () => {
  it('makes nothing until first used', () => {
    const make = vi.fn((locale: string) => locale)
    perLanguage(make)
    expect(make).not.toHaveBeenCalled()
  })

  it('makes one value per language, with that language locale, and reuses it', () => {
    const make = vi.fn((locale: string) => ({ locale }))
    const get = perLanguage(make)

    const ko = get()
    expect(get()).toBe(ko)
    setLanguage('en')
    const en = get()
    expect(get()).toBe(en)
    setLanguage('ko')
    expect(get()).toBe(ko)

    expect(ko.locale).toBe('ko-KR')
    expect(en.locale).toBe('en-US')
    expect(make).toHaveBeenCalledTimes(2)
  })
})
