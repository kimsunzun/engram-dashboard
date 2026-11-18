// TRD S21-slot-docking R24 (tab bar overflow = Chrome): table test of the pure functions for tab bar head width · hiding. jsdom does no layout, so the drawn
//   width cannot be measured here (screen QA AC29) — this function is the only test surface of that rule.

import { describe, expect, it } from 'vitest'

import { TAB_MIN_ACTIVE_W, TAB_MIN_INACTIVE_W, TAB_STANDARD_W, tabStripLayout } from './layout'

const S = TAB_STANDARD_W
const A = TAB_MIN_ACTIVE_W
const I = TAB_MIN_INACTIVE_W

interface Case {
  name: string
  available: number | null
  n: number
  active: number
  widths: number[]
  visible: boolean[]
}

const T = true
const F = false

const CASES: Case[] = [
  // ── before measuring ──
  { name: 'unmeasurable (null) = no limit → all S · no hiding', available: null, n: 3, active: 1, widths: [S, S, S], visible: [T, T, T] },
  { name: 'a non-finite number is also unmeasurable', available: Number.NaN, n: 2, active: 0, widths: [S, S], visible: [T, T] },

  // ── stage A (available ≥ n·A) ──
  { name: 'all S — exactly n·S', available: 3 * S, n: 3, active: 0, widths: [S, S, S], visible: [T, T, T] },
  { name: 'all S — leftover px are not divided', available: 1000, n: 3, active: 0, widths: [S, S, S], visible: [T, T, T] },
  { name: 'even width between A and S · the leftover 1 px goes to the leftmost', available: 400, n: 3, active: 2, widths: [134, 133, 133], visible: [T, T, T] },
  { name: 'the leftover 2 px one each from the left', available: 302, n: 4, active: 0, widths: [76, 76, 75, 75], visible: [T, T, T, T] },
  { name: 'exactly at the switch point (n·A) = all A', available: 3 * A, n: 3, active: 1, widths: [A, A, A], visible: [T, T, T] },
  { name: 'just above the switch point = all A + the leftover 1 px', available: 3 * A + 1, n: 3, active: 1, widths: [A + 1, A, A], visible: [T, T, T] },
  { name: 'one head · active — if narrower than S that width as is', available: 100, n: 1, active: 0, widths: [100], visible: [T] },
  { name: 'one head · no active is also stage A', available: 100, n: 1, active: -1, widths: [100], visible: [T] },

  // ── stage B (available < n·A) ──
  { name: 'just below the switch point — active fixed at A · only inactive ones shrink', available: 3 * A - 1, n: 3, active: 1, widths: [A, A, A - 1], visible: [T, T, T] },
  {
    name: 'leftover px skip the active and go only to inactive ones from the left (TabAcceptsExtraSpace — active minimum = switch point)',
    available: A + 3 * I + 2,
    n: 4,
    active: 0,
    widths: [A, I + 1, I + 1, I],
    visible: [T, T, T, T],
  },
  {
    name: 'even if the active is not leftmost — inactive ones before it receive and the active is skipped',
    available: A + 3 * I + 2,
    n: 4,
    active: 1,
    widths: [I + 1, A, I + 1, I],
    visible: [T, T, T, T],
  },
  { name: 'no active — all receive so the leftmost gets it', available: 141, n: 4, active: -1, widths: [36, 35, 35, 35], visible: [T, T, T, T] },
  {
    name: 'f clamped to 0 (available < A + (n−1)·I) — the overflowed tail is hidden · no leftover px',
    available: 130,
    n: 4,
    active: 0,
    widths: [A, I, I, I],
    visible: [T, T, T, F],
  },
  { name: 'no active — all based on the inactive minimum I', available: 140, n: 4, active: -1, widths: [I + 3, I + 3, I + 3, I + 3], visible: [T, T, T, T] },
  { name: 'one head · inactive — between I and A', available: 35, n: 1, active: -1, widths: [35], visible: [T] },

  // ── hiding (ShouldTabBeVisible) ──
  {
    name: 'stable set — if the active is later, even a head that fits is hidden if 「right end + (active − this width)」 overflows · an active beyond the end also hides',
    available: 135,
    n: 5,
    active: 4,
    widths: [I, I, I, I, A],
    visible: [T, T, T, F, F],
  },
  {
    name: 'stable set — the same visible set even if the active is earlier',
    available: 135,
    n: 5,
    active: 0,
    widths: [A, I, I, I, I],
    visible: [T, T, T, F, F],
  },
  {
    name: 'if the active is at that place the rule does not apply and only the right end is looked at',
    available: 135,
    n: 5,
    active: 3,
    widths: [I, I, I, A, I],
    visible: [T, T, T, F, F],
  },
  {
    name: 'no active — the stable-set rule does not apply so one more shows',
    available: 135,
    n: 5,
    active: -1,
    widths: [I, I, I, I, I],
    visible: [T, T, T, T, F],
  },
  {
    name: 'only the active beyond the end hides and the earlier ones all show',
    available: 150,
    n: 5,
    active: 4,
    widths: [I, I, I, I, A],
    visible: [T, T, T, T, F],
  },
  { name: 'one head · active · narrower than A = hidden', available: 30, n: 1, active: 0, widths: [A], visible: [F] },
  { name: 'one head · inactive · narrower than I = hidden', available: 30, n: 1, active: -1, widths: [I], visible: [F] },
  { name: 'negative width = all hidden', available: -10, n: 2, active: 0, widths: [A, I], visible: [F, F] },

  // ── edges ──
  { name: 'no heads', available: 500, n: 0, active: -1, widths: [], visible: [] },
  { name: 'no heads · negative width', available: -5, n: 0, active: -1, widths: [], visible: [] },
  { name: 'a fractional width rounds down', available: 400.9, n: 3, active: 0, widths: [134, 133, 133], visible: [T, T, T] },
  { name: 'an out-of-range active = no active', available: 135, n: 5, active: 5, widths: [I, I, I, I, I], visible: [T, T, T, T, F] },
]

describe('tabStripLayout — table', () => {
  for (const c of CASES) {
    it(c.name, () => {
      expect(tabStripLayout(c.available, c.n, c.active)).toEqual({ widths: c.widths, visible: c.visible })
    })
  }
})

describe('tabStripLayout — invariants (heads 1–9 · every active · width 0–1600 exhaustive)', () => {
  // Collect only the mismatched inputs and assert once — calling expect per field would be hundreds of thousands of calls and slow.
  const violations = (bad: (available: number, n: number, active: number) => boolean): string[] => {
    const out: string[] = []
    for (let n = 1; n <= 9; n += 1) {
      for (let active = -1; active < n; active += 1) {
        for (let available = 0; available <= 1600; available += 1) {
          if (bad(available, n, active)) out.push(`${available}/${n}/${active}`)
        }
      }
    }
    return out
  }

  it('hiding is always the tail — visible = the front k', () => {
    const bad = violations((available, n, active) => {
      const { visible } = tabStripLayout(available, n, active)
      const k = visible.indexOf(false)
      return k >= 0 && visible.slice(k).some(v => v)
    })
    expect(bad).toEqual([])
  })

  it('visible heads fit within available', () => {
    const bad = violations((available, n, active) => {
      const { widths, visible } = tabStripLayout(available, n, active)
      return widths.filter((_, i) => visible[i]).reduce((s, w) => s + w, 0) > available
    })
    expect(bad).toEqual([])
  })

  it('if not overflowing and not all S then width sum = available (the leftover px are all divided)', () => {
    const bad = violations((available, n, active) => {
      const minimum = active >= 0 ? A + (n - 1) * I : n * I
      if (available < minimum || available >= n * S) return false
      return tabStripLayout(available, n, active).widths.reduce((s, w) => s + w, 0) !== available
    })
    expect(bad).toEqual([])
  })

  it('the active of stage B (available < n·A) is exactly A — it does not receive leftover px', () => {
    const bad = violations((available, n, active) => {
      if (active < 0 || available >= n * A) return false
      return tabStripLayout(available, n, active).widths[active] !== A
    })
    expect(bad).toEqual([])
  })

  it('among receiving heads (stage A = all · stage B = inactive) the left is wider or equal and the difference is at most 1 px', () => {
    const bad = violations((available, n, active) => {
      const { widths } = tabStripLayout(available, n, active)
      const takers = widths.filter((_, i) => available >= n * A || i !== active)
      return takers.some((w, i) => i > 0 && w > takers[i - 1]) || takers[0] - takers[takers.length - 1] > 1
    })
    expect(bad).toEqual([])
  })

  it('active is A or more · inactive is I or more · none exceeds S', () => {
    const bad = violations((available, n, active) =>
      tabStripLayout(available, n, active).widths.some((w, i) => w < (i === active ? A : I) || w > S),
    )
    expect(bad).toEqual([])
  })
})

describe('tabStripLayout — width cap (frozen width of closing mode) · visibility by measured width', () => {
  interface CapCase {
    name: string
    available: number | null
    n: number
    active: number
    cap: number | null
    widths: number[]
    visible: boolean[]
  }
  const CAP_CASES: CapCase[] = [
    {
      name: 'if the cap is narrow the width is the cap · visibility by the measured width 135 (measured with cap 80 the second onward would hide)',
      available: 135,
      n: 5,
      active: 0,
      cap: 80,
      widths: [A, I, I, I, I],
      visible: [T, T, T, F, F],
    },
    { name: 'closing mode reproduction — frozen 103 · four heads', available: 135, n: 4, active: 0, cap: 103, widths: [A, I, I, I], visible: [T, T, T, F] },
    { name: 'closing mode reproduction — frozen 71 · two heads', available: 135, n: 2, active: 0, cap: 71, widths: [A, I], visible: [T, T] },
    { name: 'unmeasurable + cap — width is the cap · no hiding', available: null, n: 3, active: 0, cap: 200, widths: [67, 67, 66], visible: [T, T, T] },
  ]
  for (const c of CAP_CASES) {
    it(c.name, () => {
      expect(tabStripLayout(c.available, c.n, c.active, c.cap)).toEqual({ widths: c.widths, visible: c.visible })
    })
  }

  it('if the cap is at least the measured width · null · non-finite it equals no cap', () => {
    for (const cap of [400, 1000, null, undefined, Number.NaN]) {
      expect(tabStripLayout(400, 3, 2, cap)).toEqual(tabStripLayout(400, 3, 2))
    }
  })

  it('invariant — width = the width of min(measured width, cap) · hiding is the tail · visible heads within the measured width (heads 1–6 · width 0–400 · cap 0–400 / 7)', () => {
    const bad: string[] = []
    for (let n = 1; n <= 6; n += 1) {
      for (let active = -1; active < n; active += 1) {
        for (let available = 0; available <= 400; available += 1) {
          for (let cap = 0; cap <= 400; cap += 7) {
            const { widths, visible } = tabStripLayout(available, n, active, cap)
            const ref = tabStripLayout(Math.min(available, cap), n, active).widths
            const k = visible.indexOf(false)
            const shown = widths.filter((_, i) => visible[i]).reduce((s, w) => s + w, 0)
            if (widths.some((w, i) => w !== ref[i])) bad.push(`width ${available}/${cap}/${n}/${active}`)
            if (k >= 0 && visible.slice(k).some(v => v)) bad.push(`tail ${available}/${cap}/${n}/${active}`)
            if (shown > available) bad.push(`overflow ${available}/${cap}/${n}/${active}`)
          }
        }
      }
    }
    expect(bad).toEqual([])
  })
})
