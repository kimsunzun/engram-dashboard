// TRD S21-slot-docking §4-3 「what stays as is」 · edition 6.3 §5-6 · §11 「TS tabStrip」 · PRD AC40: table test of the closing mode pure state line — boundaries (39/40 · 59/60 · top ·
//   start side 0) · 300 ms (fake clock = the input `now`) · outside press · last tab · fixed width.

import { describe, expect, it } from 'vitest'

import { TAB_MIN_ACTIVE_W, TAB_MIN_INACTIVE_W, TAB_STANDARD_W, tabStripLayout } from './layout'
import {
  CLOSE_LEAVE_DELAY_MS,
  CLOSE_SLOP_BELOW_PX,
  CLOSE_SLOP_END_PX,
  CLOSING_OFF,
  type ClosingInput,
  type ClosingMode,
  type Rect,
  closingBounds,
  frozenAvailable,
  inClosingBounds,
  layoutAvailable,
  stepClosingMode,
} from './closingMode'

const A = TAB_MIN_ACTIVE_W
const I = TAB_MIN_INACTIVE_W
const S = TAB_STANDARD_W

const BAR: Rect = { x0: 0, y0: 0, x1: 500, y1: 28 }

function run(mode: ClosingMode, inputs: ClosingInput[]): ClosingMode {
  return inputs.reduce(stepClosingMode, mode)
}

describe('boundary — bottom 40 · end side 60 · top · start side 0 · half-open (AC40)', () => {
  it('closingBounds', () => {
    expect(closingBounds(BAR)).toEqual({ x0: 0, y0: 0, x1: 500 + CLOSE_SLOP_END_PX, y1: 28 + CLOSE_SLOP_BELOW_PX })
    expect(CLOSE_SLOP_END_PX).toBe(60)
    expect(CLOSE_SLOP_BELOW_PX).toBe(40)
  })

  const CASES: { name: string; x: number; y: number; inside: boolean }[] = [
    { name: 'inside the tab bar', x: 250, y: 14, inside: true },
    { name: 'top-left corner (included)', x: 0, y: 0, inside: true },
    { name: 'bottom 39 = inside', x: 250, y: 28 + 39, inside: true },
    { name: 'bottom 39.99 = inside', x: 250, y: 28 + 39.99, inside: true },
    { name: 'bottom 40 = outside', x: 250, y: 28 + 40, inside: false },
    { name: 'bottom 41 = outside', x: 250, y: 28 + 41, inside: false },
    { name: 'end side 59 = inside', x: 500 + 59, y: 14, inside: true },
    { name: 'end side 60 = outside', x: 500 + 60, y: 14, inside: false },
    { name: 'end side 61 = outside', x: 500 + 61, y: 14, inside: false },
    { name: 'end · bottom corner 59/39 = inside', x: 559, y: 67, inside: true },
    { name: 'no slack on top — just outside', x: 250, y: -0.5, inside: false },
    { name: 'no slack on the start side — just outside', x: -0.5, y: 14, inside: false },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(inClosingBounds(BAR, c.x, c.y)).toBe(c.inside)
    })
  }

  it('relative to the tab bar even if the tab bar is not at the window origin', () => {
    const bar = { x0: 30, y0: 6, x1: 330, y1: 34 }
    expect(inClosingBounds(bar, 29.9, 20)).toBe(false)
    expect(inClosingBounds(bar, 30, 5.9)).toBe(false)
    expect(inClosingBounds(bar, 389, 73)).toBe(true)
    expect(inClosingBounds(bar, 390, 20)).toBe(false)
    expect(inClosingBounds(bar, 100, 74)).toBe(false)
  })
})

describe('frozenAvailable — current width − removed width', () => {
  const CASES: { name: string; current: number; widths: number[]; closed: number; next?: number; out: number }[] = [
    { name: 'remove one inactive', current: 400, widths: [100, 100, 100, 100], closed: 1, out: 300 },
    { name: 'the base is the passed current width, not the width sum', current: 330, widths: [100, 100, 100], closed: 1, out: 230 },
    { name: 'remove the active in stage A — the closed width (even when given the next active)', current: 301, widths: [101, 100, 100], closed: 0, next: 1, out: 200 },
    { name: 'remove the active in stage B — ★the current width of the next active tab★', current: 3 * I + 8 + A, widths: [I + 3, A, I + 3, I + 2], closed: 1, next: 2, out: 2 * I + 5 + A },
    { name: 'even in stage B, removing an inactive gives the closed width', current: 3 * I + 8 + A, widths: [I + 3, A, I + 3, I + 2], closed: 2, out: 2 * I + 5 + A },
    { name: 'stage B · even if the next active is a hidden tab its computed width', current: A + 2 * I, widths: [A, I, I], closed: 0, next: 2, out: A + I },
    { name: 'removing a hidden tab also gives its computed width (visible in Chrome = pinned tabs · groups only)', current: 135, widths: [A, I, I, I, I], closed: 3, out: 103 },
    { name: 'exactly at the switch point (all A) = stage A', current: 3 * A, widths: [A, A, A], closed: 1, next: 2, out: 2 * A },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(frozenAvailable(c.current, c.widths, c.closed, c.next)).toBe(c.out)
    })
  }

  it('if the removed index is the last or out of range · the next active index is out of range or equals the removed index RangeError', () => {
    expect(() => frozenAvailable(300, [100, 100, 100], 2)).toThrow(RangeError)
    expect(() => frozenAvailable(300, [100, 100, 100], -1)).toThrow(RangeError)
    expect(() => frozenAvailable(300, [100, 100, 100], 0.5)).toThrow(RangeError)
    expect(() => frozenAvailable(300, [100, 100, 100], 0, 3)).toThrow(RangeError)
    expect(() => frozenAvailable(3 * I + 8 + A, [I + 3, A, I + 3, I + 2], 1, 1)).toThrow(RangeError)
  })
})

describe('stepClosingMode — entering · a tab removed', () => {
  const closed = (
    widths: number[],
    at: number,
    measured: number | null,
    opts: { mouse?: boolean; next?: number } = {},
  ): ClosingInput => ({ kind: 'closed', mouse: opts.mouse ?? true, widths, closed: at, measured, nextActive: opts.next })

  const CASES: { name: string; from: ClosingMode; input: ClosingInput; out: ClosingMode }[] = [
    {
      name: 'closed by mouse → enters · base = min(n·S, measured width)',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 1, 300),
      out: { on: true, available: 200, deadline: null },
    },
    {
      name: 'unmeasurable → base = n·S (480) — 380 ≥ 2·S so it does not enter',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 1, null),
      out: CLOSING_OFF,
    },
    {
      name: 'all standard width — does not enter if the frozen width is at least the sum of the standard widths of the remaining tabs',
      from: CLOSING_OFF,
      input: closed([S, S, S], 0, 1000),
      out: CLOSING_OFF,
    },
    {
      name: 'enters if 1 px narrower than the standard width sum',
      from: CLOSING_OFF,
      input: closed([S, S - 1, S], 0, 479),
      out: { on: true, available: 2 * S - 1, deadline: null },
    },
    {
      name: 'last tab (outside the mode) → enters at the width of that moment · does not subtract (tci #1435)',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 2, 300),
      out: { on: true, available: 300, deadline: null },
    },
    {
      name: 'last tab (during the mode) → mode · frozen width · deadline as is (tci #1435)',
      from: { on: true, available: 280, deadline: 1234 },
      input: closed([100, 100, 100], 2, 300),
      out: { on: true, available: 280, deadline: 1234 },
    },
    {
      name: 'last tab · the width at that moment is at least the standard width sum of the remaining tabs → does not enter (tci #1263)',
      from: CLOSING_OFF,
      input: closed([S, S, 100], 2, 420),
      out: CLOSING_OFF,
    },
    {
      name: 'hidden tabs count too — if a non-last hidden tab is removed its computed width is subtracted (700 − 32)',
      from: { on: true, available: 700, deadline: null },
      input: closed([A, I, I, I, I, I, I, I, I, I], 4, 135, { mouse: false }),
      out: { on: true, available: 668, deadline: null },
    },
    {
      name: 'even the last visible one is not the last tab if a hidden tab follows — subtract',
      from: CLOSING_OFF,
      input: closed([A, I, I, I, I], 2, 135),
      out: { on: true, available: 103, deadline: null },
    },
    { name: 'one tab → does not enter', from: CLOSING_OFF, input: closed([100], 0, 300), out: CLOSING_OFF },
    {
      name: 'one tab → leaves if in the mode',
      from: { on: true, available: 300, deadline: null },
      input: closed([100], 0, 300, { mouse: false }),
      out: CLOSING_OFF,
    },
    {
      name: 'closed by keyboard · command → does not enter',
      from: CLOSING_OFF,
      input: closed([100, 100, 100], 1, 300, { mouse: false }),
      out: CLOSING_OFF,
    },
    {
      name: 'closed by another path during the mode → only the width is fixed (Chrome)',
      from: { on: true, available: 300, deadline: null },
      input: closed([100, 100, 100], 0, 500, { mouse: false }),
      out: { on: true, available: 200, deadline: null },
    },
    {
      name: 'during the mode base = frozen width — 330, not the width sum 300 · measured width 500 (tci #1470-1473)',
      from: { on: true, available: 330, deadline: null },
      input: closed([100, 100, 100], 1, 500),
      out: { on: true, available: 230, deadline: null },
    },
    {
      name: 'consecutive close during the mode → the deadline is left as is',
      from: { on: true, available: 300, deadline: 1234 },
      input: closed([100, 100, 100], 0, 300),
      out: { on: true, available: 200, deadline: 1234 },
    },
    {
      name: 'close the active in stage B — subtract the next active width',
      from: CLOSING_OFF,
      input: closed([I + 3, A, I + 3, I + 2], 1, 3 * I + 8 + A, { next: 2 }),
      out: { on: true, available: 2 * I + 5 + A, deadline: null },
    },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(stepClosingMode(c.from, c.input)).toEqual(c.out)
    })
  }

  it('if the removed index is out of range RangeError', () => {
    expect(() => stepClosingMode(CLOSING_OFF, closed([100, 100], -1, 200))).toThrow(RangeError)
    expect(() => stepClosingMode(CLOSING_OFF, closed([100, 100], 2, 200))).toThrow(RangeError)
  })
})

describe('stepClosingMode — leaving (300 ms · outside press)', () => {
  const ON: ClosingMode = { on: true, available: 200, deadline: null }
  const move = (x: number, y: number, now: number): ClosingInput => ({ kind: 'move', x, y, bar: BAR, now })
  const press = (x: number, y: number): ClosingInput => ({ kind: 'press', x, y, bar: BAR })
  const tick = (now: number): ClosingInput => ({ kind: 'tick', now })

  it('300 ms', () => {
    expect(CLOSE_LEAVE_DELAY_MS).toBe(300)
  })

  it('while inside the tab bar it stays as is even as time passes', () => {
    expect(run(ON, [move(250, 14, 0), tick(10_000), move(300, 28 + 39, 20_000), tick(60_000)])).toEqual(ON)
  })

  it('leaving to bottom 40 leaves after 300 ms — 299 is not yet', () => {
    const left = run(ON, [move(250, 28 + 40, 1000)])
    expect(left).toEqual({ ...ON, deadline: 1300 })
    expect(stepClosingMode(left, tick(1299))).toEqual(left)
    expect(stepClosingMode(left, tick(1300))).toEqual(CLOSING_OFF)
  })

  it('leaving to end side 60 is the same · 59 is inside', () => {
    expect(run(ON, [move(559, 14, 0), tick(5000)])).toEqual(ON)
    expect(run(ON, [move(560, 14, 0), tick(300)])).toEqual(CLOSING_OFF)
  })

  it('top · start side leaves if just outside the boundary', () => {
    expect(run(ON, [move(250, -1, 0), tick(300)])).toEqual(CLOSING_OFF)
    expect(run(ON, [move(-1, 14, 0), tick(300)])).toEqual(CLOSING_OFF)
  })

  it('if it returns within 300 ms it is cancelled — the mode continues', () => {
    expect(run(ON, [move(250, 100, 0), move(250, 14, 299), tick(300), tick(5000)])).toEqual(ON)
  })

  it('even if it moves further outside the deadline is from the first time it left', () => {
    const out = run(ON, [move(250, 100, 0), move(250, 120, 200), move(250, 140, 290)])
    expect(out).toEqual({ ...ON, deadline: 300 })
    expect(stepClosingMode(out, tick(300))).toEqual(CLOSING_OFF)
  })

  it('after leaving and returning, leaving again gives a new deadline', () => {
    expect(run(ON, [move(250, 100, 0), move(250, 14, 100), move(250, 100, 1000)])).toEqual({ ...ON, deadline: 1300 })
  })

  it('on leaving the window after 300 ms · if it returns within that it is cancelled', () => {
    expect(run(ON, [{ kind: 'leaveWindow', now: 50 }, tick(349)])).toEqual({ ...ON, deadline: 350 })
    expect(run(ON, [{ kind: 'leaveWindow', now: 50 }, tick(350)])).toEqual(CLOSING_OFF)
    expect(run(ON, [{ kind: 'leaveWindow', now: 50 }, move(250, 14, 200), tick(350)])).toEqual(ON)
  })

  it('even if it leaves the window after moving outside the boundary the deadline is as is', () => {
    expect(run(ON, [move(250, 100, 0), { kind: 'leaveWindow', now: 250 }])).toEqual({ ...ON, deadline: 300 })
  })

  it('a press outside leaves at once', () => {
    expect(stepClosingMode(ON, press(250, 28 + 40))).toEqual(CLOSING_OFF)
    expect(stepClosingMode({ ...ON, deadline: 900 }, press(560, 14))).toEqual(CLOSING_OFF)
  })

  it('a press inside stays as is · a pending deadline is cancelled', () => {
    expect(stepClosingMode(ON, press(250, 14))).toEqual(ON)
    expect(stepClosingMode({ ...ON, deadline: 900 }, press(250, 28 + 39))).toEqual(ON)
  })

  it('when a tab is added it leaves at once', () => {
    expect(stepClosingMode(ON, { kind: 'inserted' })).toEqual(CLOSING_OFF)
  })

  it('outside the mode no input changes state', () => {
    const inputs: ClosingInput[] = [move(250, 100, 0), press(900, 900), { kind: 'leaveWindow', now: 0 }, tick(10_000), { kind: 'inserted' }]
    for (const input of inputs) expect(stepClosingMode(CLOSING_OFF, input)).toEqual(CLOSING_OFF)
  })
})

describe('consecutive closes — from entering to leaving', () => {
  it('after three consecutive closes, leaving and re-laying out at 300 ms', () => {
    // Five of the same width (not overflowing and narrower than the standard width) · measured width 500 · the cursor at the same place over the tab bar.
    const close = (widths: number[], at: number): ClosingInput => ({ kind: 'closed', mouse: true, widths, closed: at, measured: 500 })
    let mode = stepClosingMode(CLOSING_OFF, close([100, 100, 100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 400, deadline: null })
    mode = stepClosingMode(mode, close([100, 100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 300, deadline: null })
    mode = stepClosingMode(mode, close([100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 200, deadline: null })
    expect(layoutAvailable(mode, 500)).toBe(200)
    mode = run(mode, [{ kind: 'move', x: 150, y: 80, bar: BAR, now: 7000 }, { kind: 'tick', now: 7300 }])
    expect(mode).toEqual(CLOSING_OFF)
    expect(layoutAvailable(mode, 500)).toBe(500)
  })

  it('closing the last tab during consecutive closes keeps the fixed width — the remaining tabs widen inside it', () => {
    const close = (widths: number[], at: number): ClosingInput => ({ kind: 'closed', mouse: true, widths, closed: at, measured: 400 })
    let mode = stepClosingMode(CLOSING_OFF, close([100, 100, 100, 100], 1))
    expect(mode).toEqual({ on: true, available: 300, deadline: null })
    mode = stepClosingMode(mode, close([100, 100, 100], 2))
    expect(mode).toEqual({ on: true, available: 300, deadline: null })
    expect(tabStripLayout(400, 2, 0, layoutAvailable(mode, 400)).widths).toEqual([150, 150])
  })

  it('closing the same position three times in an overflowed tab bar — fixed 103 → 71 → 71 · remaining widths [A, I] (Chromium reproduction)', () => {
    // Measured width 135 · five tabs · active 0 · close position 2 three times with the mouse. The width is the cap (fixed width) · visibility uses the measured width.
    const measured = 135
    let n = 5
    let mode: ClosingMode = CLOSING_OFF
    const seen: number[] = []
    for (let k = 0; k < 3; k += 1) {
      const { widths } = tabStripLayout(measured, n, 0, layoutAvailable(mode, measured))
      mode = stepClosingMode(mode, { kind: 'closed', mouse: true, widths, closed: 2, measured })
      n -= 1
      seen.push(mode.on ? mode.available : -1)
    }
    expect(seen).toEqual([103, 71, 71])
    expect(tabStripLayout(measured, n, 0, layoutAvailable(mode, measured))).toEqual({ widths: [A, I], visible: [true, true] })
  })
})

describe('layoutAvailable — during the mode min(frozen width, measured width)', () => {
  const ON: ClosingMode = { on: true, available: 300, deadline: null }
  const CASES: { name: string; mode: ClosingMode; measured: number | null; out: number | null }[] = [
    { name: 'outside the mode = measured width', mode: CLOSING_OFF, measured: 500, out: 500 },
    { name: 'outside the mode · unmeasurable = null', mode: CLOSING_OFF, measured: null, out: null },
    { name: 'during the mode = frozen width', mode: ON, measured: 500, out: 300 },
    { name: 'during the mode if the window shrinks the measured width', mode: ON, measured: 250, out: 250 },
    { name: 'during the mode unmeasurable = frozen width', mode: ON, measured: null, out: 300 },
    { name: 'during the mode a non-finite measured width = frozen width', mode: ON, measured: Number.NaN, out: 300 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(layoutAvailable(c.mode, c.measured)).toBe(c.out)
    })
  }
})
