import { act, cleanup, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { __test__, setLanguage } from './index'
import { useLanguage } from './react'

describe('useLanguage (ADR-0373)', () => {
  afterEach(() => {
    cleanup()
    setLanguage('ko')
  })

  it('re-renders with the new language after setLanguage', () => {
    let renders = 0
    const { result } = renderHook(() => {
      renders += 1
      return useLanguage()
    })
    expect(result.current).toBe('ko')
    const before = renders

    act(() => {
      setLanguage('en')
    })
    expect(result.current).toBe('en')
    expect(renders).toBeGreaterThan(before)
  })

  it('subscribes while mounted and stops listening after unmount', () => {
    const base = __test__.listenerCount()
    const { unmount } = renderHook(() => useLanguage())
    expect(__test__.listenerCount()).toBe(base + 1)

    unmount()
    expect(__test__.listenerCount()).toBe(base)
  })
})
