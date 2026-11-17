// @vitest-environment node
// `index.ts` loads where there is no `document` (node-environment test files): the boot read and the `<html lang>` write
//   are guarded (ADR-0373).

import { describe, expect, it, vi } from 'vitest'

import { getLanguage, setLanguage, t } from './index'

describe('i18n in a node environment', () => {
  it('has no document here', () => {
    expect(globalThis.document).toBeUndefined()
  })

  it('the setup file put this file in Korean', () => {
    expect(getLanguage()).toBe('ko')
    expect(t('tab.create')).toBe('새 탭')
  })

  it('a fresh import boots in English, and setLanguage works without a document', async () => {
    vi.resetModules()
    const fresh = await import('./index')
    expect(fresh.getLanguage()).toBe('en')
    expect(fresh.t('tab.create')).toBe('New tab')
    fresh.setLanguage('ko')
    expect(fresh.t('tab.create')).toBe('새 탭')
  })

  it('setLanguage switches and switches back', () => {
    setLanguage('en')
    expect(t('tab.close', { name: 'X' })).toBe('Close tab: X')
    setLanguage('ko')
    expect(t('tab.close', { name: 'X' })).toBe('탭 닫기: X')
  })
})
