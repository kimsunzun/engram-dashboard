// TRD S21-slot-docking §8-2 `succession`: 셸 `succession_gives_the_same_tab_as_closing` 과 같은 표(n = 1…4 × 빠지는 자리).

import { describe, expect, it } from 'vitest'

import { succession } from './succession'

const TABS = ['t0', 't1', 't2', 't3']

describe('succession — 오른쪽 이웃 → 왼쪽', () => {
  // [n, 빠지는 자리, 이을 탭]
  const table: Array<[number, number, string | null]> = [
    [1, 0, null],
    [2, 0, 't1'],
    [2, 1, 't0'],
    [3, 0, 't1'],
    [3, 1, 't2'],
    [3, 2, 't1'],
    [4, 0, 't1'],
    [4, 1, 't2'],
    [4, 2, 't3'],
    [4, 3, 't2'],
  ]
  it.each(table)('n=%i · 빠지는 자리 %i → %s', (n, removed, heir) => {
    const tabs = TABS.slice(0, n)
    expect(succession(tabs, tabs[removed])).toBe(heir)
  })

  it('목록에 없으면 · 빈 목록이면 null', () => {
    expect(succession(['a'], 'zzz')).toBeNull()
    expect(succession([], 'a')).toBeNull()
  })
})
