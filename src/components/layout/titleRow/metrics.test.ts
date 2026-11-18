// Minimum window size computation of TRD S26-window-chrome §7-7 — left margin + active tab 52 + `+` box 26 + empty draggable area 42 + number badge + buttons 3 × 46 ·
//   height = normal state row height 34 + area minimum 30.

import { describe, expect, it } from 'vitest'

import { minWindowSize } from './metrics'

describe('minWindowSize', () => {
  it.each([
    ['one digit (text width + 8 on each side is narrower than the minimum 28)', 23, 286],
    ['two digits', 30.5, 289],
    ['three digits', 37, 295],
  ])('number marker %s — marker width %s → width %s · height 64', (_name, markerW, width) => {
    expect(minWindowSize({ leadingInset: 0, markerW, buttons: true })).toEqual({ width, height: 64 })
  })

  it('an unmeasured marker (0 width) also counts as the minimum 28', () => {
    expect(minWindowSize({ leadingInset: 0, markerW: 0, buttons: true }).width).toBe(286)
  })

  it('waiting mode (no marker · button) = 120 · if the OS draws the buttons the button share is subtracted · the left margin is added', () => {
    expect(minWindowSize({ leadingInset: 0, markerW: null, buttons: false })).toEqual({ width: 120, height: 64 })
    expect(minWindowSize({ leadingInset: 0, markerW: 28, buttons: false }).width).toBe(148)
    expect(minWindowSize({ leadingInset: 80, markerW: 28, buttons: true }).width).toBe(366)
  })
})
