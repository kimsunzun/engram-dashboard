// TRD S21-slot-docking §8-2 `succession`: the same table as shell `succession_gives_the_same_tab_as_closing` (n = 1…4 × removed position).

import { describe, expect, it } from 'vitest'

import { succession } from './succession'

const TABS = ['t0', 't1', 't2', 't3']

describe('succession — right neighbour → left', () => {
  // [n, removed position, tab to succeed]
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
  it.each(table)('n=%i · removed index %i → %s', (n, removed, heir) => {
    const tabs = TABS.slice(0, n)
    expect(succession(tabs, tabs[removed])).toBe(heir)
  })

  it('if not in the list · if the list is empty null', () => {
    expect(succession(['a'], 'zzz')).toBeNull()
    expect(succession([], 'a')).toBeNull()
  })
})
