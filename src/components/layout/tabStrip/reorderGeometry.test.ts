// TRD S21-slot-docking §4-1 · §8-2 · edition 6.3 §5-1 · §11 "TS tabStrip": table test of the pure geometry of the in-tab-bar order move — expected values are Chromium's rules computed by hand (skipping the in-place candidate · front-side tie · −1 after the dragged tab · clipping at the visible end · threshold).

import { describe, expect, it } from 'vitest'

import { TAB_MIN_ACTIVE_W, TAB_MIN_INACTIVE_W, TAB_STANDARD_W, tabStripLayout } from './layout'
import {
  EASE_OUT_CSS,
  SLIDE_MS,
  type StripBounds,
  dragAreaWidth,
  draggedLeft,
  dropIndex,
  dropMarkerX,
  easeOut,
  grabOffset,
  idealBounds,
  insertionIndex,
  moveDue,
  reorderAreaWidth,
  reorderThreshold,
  slideMs,
  slideTargets,
} from './reorderGeometry'

const A = TAB_MIN_ACTIVE_W
const I = TAB_MIN_INACTIVE_W

/** Width list → ideal bounds of one tab bar (accumulation of all tab widths). The first `shown` are visible, and `dragged` is counted as a visible tab even if hidden. */
function stripOf(widths: number[], shown = widths.length, dragged?: number): StripBounds {
  let x = 0
  const tabs = widths.map(w => {
    const b = { x0: x, x1: x + w }
    x += w
    return b
  })
  const visible = widths.map((_, i) => i < shown || i === dragged)
  return { tabs, visible, lastVisible: visible.lastIndexOf(true) }
}

const EMPTY: StripBounds = { tabs: [], visible: [], lastVisible: -1 }
const FOUR = stripOf([100, 100, 100, 100])
/** Stage B — active (dragged tab) A · inactive I and in between (the shapes `layout.ts` produces). */
const PHASE_B = stripOf([I, A, I, I])
/** Overflow — after the three visible (A · I · I), the two hidden stand at their own width beyond the visible end 116 (116–148 · 148–180). */
const OVERFLOW = stripOf([A, I, I, I, I], 3)
/** Overflow · the dragged tab (active) is 4 beyond the end — the layout hides it but it is the dragged tab so it is counted as visible (hidden 3 = 96–128 · 4 = 128–180). */
const DRAGGED_HIDDEN = stripOf([I, I, I, I, A], 3, 4)

describe('idealBounds — accumulated computed width of all tabs · visibility · last visible index', () => {
  it('a hidden tab also stands beyond the visible end at its own width · last visible = 2', () => {
    expect(idealBounds(tabStripLayout(A + 3 * I - 3, 5, 0))).toEqual(OVERFLOW)
    expect(OVERFLOW.tabs[3]).toEqual({ x0: A + 2 * I, x1: A + 3 * I })
    expect(OVERFLOW.lastVisible).toBe(2)
  })
  it('with room all are the standard width', () => {
    const S = TAB_STANDARD_W
    expect(idealBounds(tabStripLayout(1000, 2, 0))).toEqual({
      tabs: [
        { x0: 0, x1: S },
        { x0: S, x1: 2 * S },
      ],
      visible: [true, true],
      lastVisible: 1,
    })
  })
  it('no heads', () => {
    expect(idealBounds(tabStripLayout(500, 0, -1))).toEqual(EMPTY)
  })
  it('the dragged tab counts as visible even if the layout hides it (Chrome — the dragged tab is always visible)', () => {
    expect(idealBounds(tabStripLayout(135, 5, 4), 4)).toEqual(DRAGGED_HIDDEN)
    expect(DRAGGED_HIDDEN.lastVisible).toBe(4)
  })
  it('if the dragged tab is visible, dragged does not change the result', () => {
    expect(idealBounds(tabStripLayout(135, 5, 0), 0)).toEqual(OVERFLOW)
  })
})

describe('dragAreaWidth — the measured width · if it cannot be measured the sum of visible tab widths', () => {
  const CASES: { name: string; available: number | null; B: StripBounds; out: number }[] = [
    { name: 'up to the empty place after the last tab (measured width is wide)', available: 500, B: FOUR, out: 500 },
    { name: 'measured width even if the tab width sum is wider (Chrome tab bar width ≤ usable width)', available: 300, B: FOUR, out: 300 },
    { name: 'measured width even if visible tabs fill the measured width', available: 100, B: OVERFLOW, out: 100 },
    { name: 'measured width even when the dragged tab hidden by the layout is counted as visible (135, not 96 + 52 = 148)', available: 135, B: DRAGGED_HIDDEN, out: 135 },
    { name: 'a fractional width as is', available: 450.5, B: FOUR, out: 450.5 },
    { name: 'unmeasurable (null) = tab width sum', available: null, B: FOUR, out: 400 },
    { name: 'unmeasurable · hidden tabs are not in the sum', available: null, B: OVERFLOW, out: A + 2 * I },
    { name: 'a non-finite number is also unmeasurable', available: Number.NaN, B: FOUR, out: 400 },
    { name: 'no tabs', available: 200, B: EMPTY, out: 200 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(dragAreaWidth(c.available, c.B)).toBe(c.out)
    })
  }
})

describe('reorderAreaWidth — clips the drag area to the sum of visible tab widths (the dragged tab does not go past the last position over `+`)', () => {
  const CASES: { name: string; available: number | null; B: StripBounds; out: number }[] = [
    { name: 'the empty place after the last tab is excluded', available: 500, B: FOUR, out: 400 },
    { name: 'if the measured width is narrower, the measured width', available: 300, B: FOUR, out: 300 },
    { name: 'hidden tabs are not in the sum', available: 1000, B: OVERFLOW, out: A + 2 * I },
    { name: 'unmeasurable (null) = tab width sum', available: null, B: FOUR, out: 400 },
    { name: 'no tabs', available: 200, B: EMPTY, out: 0 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(reorderAreaWidth(c.available, c.B)).toBe(c.out)
    })
  }

  it('two tabs · wide tab bar — even dragged to the right end the dragged tab stops at the last position and goes into that position', () => {
    const B = stripOf([160, 160])
    const L = draggedLeft(900, 80, 160, reorderAreaWidth(800, B))
    expect(L).toBe(160)
    expect(insertionIndex(B, 0, L)).toBe(1)
  })
})

describe('grabOffset — round(r · width at session start)', () => {
  const CASES: { name: string; r: number; w0: number; out: number }[] = [
    { name: 'center', r: 0.5, w0: 100, out: 50 },
    { name: '.5 rounds up (away from 0) — 4.5 → 5', r: 0.125, w0: 36, out: 5 },
    { name: 'below .5 rounds down — 0.375·99 = 37.125 → 37', r: 0.375, w0: 99, out: 37 },
    { name: 'a negative .5 also goes away from 0 — −4.5 → −5', r: -0.125, w0: 36, out: -5 },
    { name: 'left end', r: 0, w0: 160, out: 0 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(grabOffset(c.r, c.w0)).toBe(c.out)
    })
  }
})

describe('draggedLeft — clamp(p − offset, 0, max(0, A − w))', () => {
  const CASES: { name: string; p: number; offset: number; w: number; A: number; out: number }[] = [
    { name: 'grabbed at the center', p: 150, offset: 50, w: 100, A: 400, out: 100 },
    { name: 'a fractional cursor as is', p: 150.25, offset: 50, w: 100, A: 400, out: 100.25 },
    { name: 'negative offset', p: 50, offset: -5, w: 36, A: 400, out: 55 },
    { name: 'stops at the left end', p: 10, offset: 50, w: 100, A: 400, out: 0 },
    { name: 'stops at the drag area right end − w', p: 390, offset: 50, w: 100, A: 400, out: 300 },
    { name: 'exactly the end', p: 350, offset: 50, w: 100, A: 400, out: 300 },
    { name: 'if the tab is wider than the area it is always 0', p: 200, offset: 50, w: 100, A: 60, out: 0 },
    { name: 'even if the width changes during the session the offset is as is — only the clipping uses the current width', p: 390, offset: 21, w: 32, A: 400, out: 368 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(draggedLeft(c.p, c.offset, c.w, c.A)).toBe(c.out)
    })
  }
})

describe('insertionIndex — Chromium CalculateInsertionIndex', () => {
  const CASES: { name: string; B: StripBounds; d: number; L: number; out: number }[] = [
    // ── four of the same width · middle tab (d=1) — candidate ideal x: k0 0 · k1 100 · (k2 skipped) · k3 200 · k4 300 ──
    { name: 'in place (L = current x)', B: FOUR, d: 1, L: 100, out: 1 },
    { name: 'front tie (k0 · k1 distance 50) → front candidate k0', B: FOUR, d: 1, L: 50, out: 0 },
    { name: '1 px more toward k0', B: FOUR, d: 1, L: 49, out: 0 },
    { name: '1 px more toward k1', B: FOUR, d: 1, L: 51, out: 1 },
    { name: 'back tie (k1 · k3 distance 50) → front candidate k1 = in place', B: FOUR, d: 1, L: 150, out: 1 },
    { name: 'past the tie k3 → after the dragged tab so −1 = 2', B: FOUR, d: 1, L: 150.5, out: 2 },
    { name: 'k3 · k4 tie → k3 → 2', B: FOUR, d: 1, L: 250, out: 2 },
    { name: 'end (k4 → 3)', B: FOUR, d: 1, L: 300, out: 3 },
    { name: 'an L beyond the area end is also the end', B: FOUR, d: 1, L: 900, out: 3 },
    { name: 'an L beyond the left is the very front', B: FOUR, d: 1, L: -40, out: 0 },

    // ── first tab (d=0) — k1 skipped · k2 100 · k3 200 · k4 300 ──
    { name: 'd=0 · k0 · k2 tie → k0 = in place', B: FOUR, d: 0, L: 50, out: 0 },
    { name: 'd=0 · past the tie k2 → 1', B: FOUR, d: 0, L: 50.01, out: 1 },
    { name: 'd=0 · end', B: FOUR, d: 0, L: 300, out: 3 },

    // ── last tab (d=3) — k4 skipped · k3 300 is in place ──
    { name: 'd=3 · k2 · k3 tie → k2 (before the dragged tab so no −1)', B: FOUR, d: 3, L: 250, out: 2 },
    { name: 'd=3 · in place', B: FOUR, d: 3, L: 251, out: 3 },
    { name: 'd=3 · very front', B: FOUR, d: 3, L: 0, out: 0 },

    // ── different widths (stage B · dragged tab = active A, d=1) — k0 0 · k1 32 · k3 116 − 52 = 64 · k4 96 ──
    { name: 'stage B · k1 · k3 center (48) tie → k1 = in place', B: PHASE_B, d: 1, L: 48, out: 1 },
    { name: 'stage B · past the center → k3 → 2', B: PHASE_B, d: 1, L: 48.5, out: 2 },
    { name: 'stage B · k3 · k4 center (80) tie → 2', B: PHASE_B, d: 1, L: 80, out: 2 },
    { name: 'stage B · k4 → 3', B: PHASE_B, d: 1, L: 81, out: 3 },

    // ── overflowing tab bar — hidden tabs are clipped to their own width · the last visible position ──
    // OVERFLOW · d=0(A): k0 0 · k2 32 · k3 64 · k4 148 − 52 = 96 · k5 128.
    { name: 'overflow · even if the candidate between hidden tabs (k4 → 3) wins it clips to the last visible index 2', B: OVERFLOW, d: 0, L: 93, out: 2 },
    { name: 'overflow · inside the visible range as is (k3 → 2)', B: OVERFLOW, d: 0, L: 64, out: 2 },
    { name: 'overflow · inside the visible range (k2 → 1)', B: OVERFLOW, d: 0, L: 40, out: 1 },
    // DRAGGED_HIDDEN · d=4(A): k0 0 · k1 32 · k2 64 · k3 96 · k4 128 (in place) · k5 skipped.
    { name: '★in place of a dragged tab beyond the end — not a tie with the candidate before the hidden tab but 4 as is★', B: DRAGGED_HIDDEN, d: 4, L: 128, out: 4 },
    { name: 'dragged tab beyond the end · as is if the in-place side is closer', B: DRAGGED_HIDDEN, d: 4, L: 120, out: 4 },
    { name: 'dragged tab beyond the end · k3 · k4 center (112) tie → front k3', B: DRAGGED_HIDDEN, d: 4, L: 112, out: 3 },
    { name: 'dragged tab beyond the end · before the hidden tab', B: DRAGGED_HIDDEN, d: 4, L: 96, out: 3 },
    { name: 'a dragged tab beyond the end can also go to the very front', B: DRAGGED_HIDDEN, d: 4, L: 0, out: 0 },

    // ── edges ──
    { name: 'one tab — always 0', B: stripOf([160]), d: 0, L: 500, out: 0 },
    { name: 'no visible tab — 0 (Chrome 「last_insertion_point = 0」)', B: stripOf([100, 100], 0), d: 1, L: 100, out: 0 },
    { name: 'no tabs — 0 (Chrome 「the only position to insert at is 0」)', B: EMPTY, d: 0, L: 10, out: 0 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(insertionIndex(c.B, c.d, c.L)).toBe(c.out)
    })
  }

  it('if the dragged tab index is out of range RangeError', () => {
    expect(() => insertionIndex(FOUR, 4, 0)).toThrow(RangeError)
    expect(() => insertionIndex(FOUR, -1, 0)).toThrow(RangeError)
    expect(() => insertionIndex(FOUR, 1.5, 0)).toThrow(RangeError)
  })

  it('invariant — the result is always 0..last visible index · if L = current x it stays in place (layout 1–7 tabs · width 0–600 / 7 · dragged tab = every active)', () => {
    const bad: string[] = []
    for (let n = 1; n <= 7; n += 1) {
      for (let available = 0; available <= 600; available += 7) {
        for (let d = 0; d < n; d += 1) {
          const B = idealBounds(tabStripLayout(available, n, d), d)
          if (insertionIndex(B, d, B.tabs[d].x0) !== d) bad.push(`in place n=${n} av=${available} d=${d}`)
          for (let L = -40; L <= available + 40; L += 9) {
            const to = insertionIndex(B, d, L)
            if (to < 0 || to > B.lastVisible) bad.push(`range n=${n} av=${available} d=${d} L=${L} → ${to}`)
          }
        }
      }
    }
    expect(bad).toEqual([])
  })
})

describe('reorderThreshold — round(current-position tab width ÷ 160 × 16)', () => {
  const CASES: { name: string; widths: number[]; to: number; out: number }[] = [
    { name: 'standard width = 16', widths: [160, 160], to: 1, out: 16 },
    { name: '100 → 10', widths: [100, 100], to: 0, out: 10 },
    { name: 'active minimum A (52) → 5.2 → 5', widths: [A, I], to: 0, out: 5 },
    { name: '45 → 4.5 → 5(.5 rounds up)', widths: [32, 45], to: 1, out: 5 },
    { name: 'inactive minimum I (32) → 3.2 → 3', widths: [A, I], to: 1, out: 3 },
    { name: 'uses the tab width at index to (not the width of the dragged tab)', widths: [A, I, I], to: 2, out: 3 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(reorderThreshold(stripOf(c.widths), c.to)).toBe(c.out)
    })
  }
  it('if the index is out of range RangeError', () => {
    expect(() => reorderThreshold(FOUR, 4)).toThrow(RangeError)
  })
})

describe('moveDue — right after attaching or |p − pLast| > threshold', () => {
  const CASES: { name: string; pLast: number; p: number; thr: number; just: boolean; out: boolean }[] = [
    { name: 'exactly the threshold is not enough', pLast: 100, p: 116, thr: 16, just: false, out: false },
    { name: 'exceeds the threshold', pLast: 100, p: 116.5, thr: 16, just: false, out: true },
    { name: 'the same to the left', pLast: 100, p: 83.9, thr: 16, just: false, out: true },
    { name: 'exactly the left threshold is not enough', pLast: 100, p: 84, thr: 16, just: false, out: false },
    { name: 'right after attaching without a threshold', pLast: 100, p: 100, thr: 16, just: true, out: true },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(moveDue(c.pLast, c.p, c.thr, c.just)).toBe(c.out)
    })
  }
})

describe('slideTargets · slideMs — stepping aside (C2)', () => {
  it('tabs other than the dragged tab get a new ideal x · the dragged tab · hidden tabs are null', () => {
    expect(slideTargets(stripOf([100, 100, 100]), 2)).toEqual([0, 100, null])
    expect(slideTargets(stripOf([A, I, I, I], 3), 0)).toEqual([null, A, A + I, null])
  })

  it('200 ms · 0 if reduced motion or just attached', () => {
    expect(SLIDE_MS).toBe(200)
    expect(slideMs(false, false)).toBe(200)
    expect(slideMs(true, false)).toBe(0)
    expect(slideMs(false, true)).toBe(0)
  })

  it('EASE_OUT_CSS draws 1 − (1 − t)² (checked against samples of the cubic Bezier)', () => {
    const [x1, y1, x2, y2] = EASE_OUT_CSS.replace(/^cubic-bezier\(|\)$/g, '').split(',').map(Number)
    const bez = (a: number, b: number, s: number) => 3 * (1 - s) ** 2 * s * a + 3 * (1 - s) * s ** 2 * b + s ** 3
    let worst = 0
    for (let i = 0; i <= 100; i += 1) {
      const s = i / 100
      worst = Math.max(worst, Math.abs(bez(y1, y2, s) - easeOut(bez(x1, x2, s))))
    }
    // It is off only by as much as the control points were cut at the fourth decimal place.
    expect(worst).toBeLessThan(1e-3)
    expect(easeOut(0)).toBe(0)
    expect(easeOut(1)).toBe(1)
    expect(easeOut(0.5)).toBe(0.75)
  })
})

describe('dropIndex · dropMarkerX — drop position · vertical line of another window\'s tab bar (W13)', () => {
  it('the number of visible heads whose center is left of x · an x equal to the center = the left of that head', () => {
    // middle = 50 · 150 · 250 · 350
    expect(dropIndex(FOUR, -5, null)).toBe(0)
    expect(dropIndex(FOUR, 49, null)).toBe(0)
    expect(dropIndex(FOUR, 50, null)).toBe(0)
    expect(dropIndex(FOUR, 51, null)).toBe(1)
    expect(dropIndex(FOUR, 150, null)).toBe(1)
    expect(dropIndex(FOUR, 349, null)).toBe(3)
    expect(dropIndex(FOUR, 351, null)).toBe(4)
  })

  it('an empty place (beyond the visible end) = the end', () => {
    expect(dropIndex(FOUR, 1000, null)).toBe(4)
    expect(dropMarkerX(FOUR, 4, null)).toBe(400)
  })

  it('a tab bar overflowed by hidden tabs clips at the position after the last visible head (R24)', () => {
    // The three visible = 0–52 · 52–84 · 84–116 · the two hidden stand at 116–148 · 148–180 at their own width.
    expect(dropIndex(OVERFLOW, 120, null)).toBe(3)
    expect(dropIndex(OVERFLOW, 160, null)).toBe(3)
    expect(dropMarkerX(OVERFLOW, 3, null)).toBe(A + 2 * I)
  })

  it('0 tabs = position 0 · line 0', () => {
    expect(dropIndex(EMPTY, 30, null)).toBe(0)
    expect(dropMarkerX(EMPTY, 0, null)).toBe(0)
  })

  it('1 tab = before center 0 · after 1', () => {
    const one = stripOf([100])
    expect(dropIndex(one, 50, null)).toBe(0)
    expect(dropIndex(one, 51, null)).toBe(1)
    expect(dropMarkerX(one, 0, null)).toBe(0)
    expect(dropMarkerX(one, 1, null)).toBe(100)
  })

  it('line = right end of the left head of that position (= left end of the right head) · an out-of-range position is clipped to the end', () => {
    expect(dropMarkerX(FOUR, 0, null)).toBe(0)
    expect(dropMarkerX(FOUR, 1, null)).toBe(100)
    expect(dropMarkerX(FOUR, 2, null)).toBe(200)
    expect(dropMarkerX(FOUR, -3, null)).toBe(0)
    expect(dropMarkerX(FOUR, 9, null)).toBe(400)
  })

  it('if the dragged tab is in this tab bar it is not counted', () => {
    // Removing dragged tab 1 (100–200), the heads = 0–100 · 200–300 · 300–400.
    expect(dropIndex(FOUR, 260, 1)).toBe(2)
    expect(dropIndex(FOUR, 240, 1)).toBe(1)
    expect(dropMarkerX(FOUR, 1, 1)).toBe(100)
  })
})
