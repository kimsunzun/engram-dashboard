// followCore table test — this file is the test table of rules 1–8 in `docs/process/S21-chat-ux/trd.md` §2-2 (ADR-0242).
//   Geometry is fixed as one set: content 1000 · window 200 → bottom = 800. Threshold = 48.

import { describe, expect, it } from 'vitest'

import {
  FOLLOW_THRESHOLD_PX,
  initialFollowState,
  isScrollable,
  step,
  type FollowInput,
  type FollowState,
  type Metrics,
} from './followCore'

const HEIGHT = 1000
const CLIENT = 200
const BOTTOM = HEIGHT - CLIENT

const at = (top: number, height = HEIGHT, client = CLIENT): Metrics => ({ top, height, client })
const hidden: Metrics = { top: 0, height: 0, client: 0 }

const state = (over: Partial<FollowState> = {}): FollowState => ({ ...initialFollowState(), ...over })

const last = <T,>(xs: T[]): T => xs[xs.length - 1]

/** Feeds inputs in turn and returns the last result and the writes in between. */
function run(s: FollowState, ...inputs: FollowInput[]): { state: FollowState; writes: (number | null)[] } {
  const writes: (number | null)[] = []
  let cur = s
  for (const i of inputs) {
    const out = step(cur, i)
    cur = out.state
    writes.push(out.scrollTo)
  }
  return { state: cur, writes }
}

describe('rule 8 — initial state', () => {
  it('attached · not frozen · lastTop 0 · no notice or saved · unseenGrowth false', () => {
    expect(initialFollowState()).toEqual({
      pinned: true,
      frozen: false,
      lastTop: 0,
      expectTop: null,
      savedTop: null,
      unseenGrowth: false,
    })
  })
})

describe('rule 1 — freeze', () => {
  it('when client 0 arrives it thaws — savedTop = expectTop (if present)', () => {
    const out = step(state({ lastTop: 300, expectTop: 500 }), { type: 'resize', m: hidden })
    expect(out.state.frozen).toBe(true)
    expect(out.state.savedTop).toBe(500)
    expect(out.scrollTo).toBeNull()
  })

  it('without a notice savedTop = lastTop (that moment\'s m.top is not used)', () => {
    const out = step(state({ lastTop: 300 }), { type: 'scroll', m: { ...hidden, top: 7 } })
    expect(out.state.frozen).toBe(true)
    expect(out.state.savedTop).toBe(300)
  })

  it.each<FollowInput['type']>(['attach', 'scroll', 'resize', 'intentUp'])(
    'thaws on any measured input when frozen (%s)',
    type => {
      const out = step(state({ lastTop: 300 }), { type, m: hidden } as FollowInput)
      expect(out.state.frozen).toBe(true)
    },
  )

  it('while frozen, scroll · client 0 resize/attach · intentUp change no field', () => {
    const frozen = state({ frozen: true, savedTop: 300, lastTop: 300, pinned: false })
    for (const i of [
      { type: 'scroll', m: at(600) },
      { type: 'resize', m: hidden },
      { type: 'attach', m: hidden },
      { type: 'intentUp', m: at(600) },
    ] as FollowInput[]) {
      const out = step(frozen, i)
      expect(out.state, i.type).toEqual(frozen)
      expect(out.scrollTo, i.type).toBeNull()
    }
  })

  it('even when frozen, pin · unpin change only attachment and do not write', () => {
    const frozen = state({ frozen: true, savedTop: 300, lastTop: 300, pinned: false })
    const pinned = step(frozen, { type: 'pin' })
    expect(pinned.scrollTo).toBeNull()
    expect(pinned.state).toEqual({ ...frozen, pinned: true })
    const unpinned = step(pinned.state, { type: 'unpin' })
    expect(unpinned.state).toEqual({ ...frozen, pinned: false })
  })
})

describe('rule 2 — thaw', () => {
  it('thawing while attached writes to the bottom · lastTop = where the write will land · savedTop = null', () => {
    const out = step(state({ frozen: true, savedTop: 300 }), { type: 'resize', m: at(0) })
    expect(out.scrollTo).toBe(BOTTOM)
    expect(out.state).toMatchObject({ frozen: false, lastTop: BOTTOM, savedTop: null, expectTop: BOTTOM })
  })

  it('thawing while detached restores with savedTop', () => {
    const out = step(state({ frozen: true, pinned: false, savedTop: 300 }), { type: 'attach', m: at(0) })
    expect(out.scrollTo).toBe(300)
    expect(out.state).toMatchObject({ frozen: false, savedTop: null, expectTop: 300 })
  })

  it('thawing while detached but already at that place does not write (preserved across display:none)', () => {
    const out = step(state({ frozen: true, pinned: false, savedTop: 300 }), { type: 'resize', m: at(300) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ frozen: false, savedTop: null, expectTop: null, lastTop: 300 })
  })

  // ★A hand-made state★ (TRD §12 low 2): rule 1 fills savedTop every time it freezes, so step alone does not
  //   reach this branch.
  it('thawing while detached with an empty savedTop does not write (a hand-made state)', () => {
    const out = step(state({ frozen: true, pinned: false, savedTop: null }), { type: 'resize', m: at(0) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ frozen: false, savedTop: null })
  })

  it('thaw is only resize · attach — scroll · intentUp with client > 0 are still frozen', () => {
    const frozen = state({ frozen: true, savedTop: 300 })
    expect(step(frozen, { type: 'scroll', m: at(0) }).state.frozen).toBe(true)
    expect(step(frozen, { type: 'intentUp', m: at(0) }).state.frozen).toBe(true)
  })

  it('a notice from before hiding is dropped on thaw — no write means no notice', () => {
    const { state: s } = run(
      state({ lastTop: 700, expectTop: 800 }),
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(BOTTOM) },
    )
    expect(s.expectTop).toBeNull()
    expect(s.frozen).toBe(false)
  })
})

describe('rule 3 — our own writes never release', () => {
  it('growth while attached → bottom write → that scroll event only moves lastTop', () => {
    const grown = step(state({ lastTop: 600 }), { type: 'resize', m: at(600) })
    expect(grown.scrollTo).toBe(BOTTOM)
    const out = step(grown.state, { type: 'scroll', m: at(BOTTOM) })
    expect(out.state).toMatchObject({ pinned: true, lastTop: BOTTOM, expectTop: null })
  })

  it('within 1px it is ours', () => {
    const out = step(state({ lastTop: 600, expectTop: 800 }), { type: 'scroll', m: at(799.2) })
    expect(out.state).toMatchObject({ lastTop: 799.2, expectTop: null })
  })

  it('an upward move to the announced position does not release either (a hand-made state — skips the attachment verdict)', () => {
    const out = step(state({ lastTop: 800, expectTop: 100 }), { type: 'scroll', m: at(100) })
    expect(out.state.pinned).toBe(true)
  })

  it('a restore write while detached (downward · within the threshold) does not re-attach', () => {
    const { state: s } = run(
      state({ frozen: true, pinned: false, savedTop: BOTTOM - 10 }),
      { type: 'resize', m: at(0) },
      { type: 'scroll', m: at(BOTTOM - 10) },
    )
    expect(s.pinned).toBe(false)
  })
})

describe('rule 4 — user scroll', () => {
  it('while attached, upward beyond the threshold → releases', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'scroll', m: at(BOTTOM - FOLLOW_THRESHOLD_PX - 1) })
    expect(out.state.pinned).toBe(false)
    expect(out.state.lastTop).toBe(BOTTOM - FOLLOW_THRESHOLD_PX - 1)
  })

  it('while attached, an upward move within the threshold (clamp of shrinking content) does not release', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'scroll', m: at(BOTTOM - 20) })
    expect(out.state.pinned).toBe(true)
    expect(out.state.lastTop).toBe(BOTTOM - 20)
  })

  it('while attached, moving downward (even if the bottom distance is beyond the threshold) does not release', () => {
    const out = step(state({ lastTop: 100 }), { type: 'scroll', m: at(300) })
    expect(out.state.pinned).toBe(true)
  })

  it('while detached, moving downward within the threshold re-attaches and lowers unseenGrowth', () => {
    const out = step(state({ pinned: false, lastTop: 500, unseenGrowth: true }), {
      type: 'scroll',
      m: at(BOTTOM - 10),
    })
    expect(out.state).toMatchObject({ pinned: true, unseenGrowth: false })
  })

  it('while detached, even moving downward, beyond the threshold it stays detached', () => {
    const out = step(state({ pinned: false, lastTop: 100 }), { type: 'scroll', m: at(500) })
    expect(out.state.pinned).toBe(false)
  })

  it('while detached, an in-place · upward scroll does not re-attach even within the threshold (latch)', () => {
    const still = step(state({ pinned: false, lastTop: BOTTOM - 10 }), { type: 'scroll', m: at(BOTTOM - 10) })
    expect(still.state.pinned).toBe(false)
    const up = step(state({ pinned: false, lastTop: BOTTOM }), { type: 'scroll', m: at(BOTTOM - 5) })
    expect(up.state.pinned).toBe(false)
  })

  it('a scroll not matching the notice drops the notice (the user moved after our write)', () => {
    const out = step(state({ lastTop: 600, expectTop: 800 }), { type: 'scroll', m: at(550) })
    expect(out.state.expectTop).toBeNull()
    expect(out.state.lastTop).toBe(550)
  })
})

describe('rule 5 — upward input', () => {
  it('if there is something to scroll, even a small wheel within the threshold releases at once', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'intentUp', m: at(BOTTOM) })
    expect(out.state.pinned).toBe(false)
    expect(out.scrollTo).toBeNull()
  })

  it('if there is nothing to scroll it does not release', () => {
    const out = step(state(), { type: 'intentUp', m: at(0, 150, CLIENT) })
    expect(out.state.pinned).toBe(true)
  })
})

describe('rule 6 — growth does not re-measure attachment', () => {
  it('while attached it writes to the bottom — even if the current position is far from the bottom it does not release', () => {
    const out = step(state({ lastTop: 100 }), { type: 'resize', m: at(100) })
    expect(out.state.pinned).toBe(true)
    expect(out.scrollTo).toBe(BOTTOM)
  })

  it('if content is smaller than the window bottom = 0', () => {
    const out = step(state({ lastTop: 50 }), { type: 'resize', m: at(50, 150, CLIENT) })
    expect(out.scrollTo).toBe(0)
  })

  it('while detached it does not write and only raises unseenGrowth', () => {
    const out = step(state({ pinned: false, lastTop: 100 }), { type: 'resize', m: at(100) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ pinned: false, unseenGrowth: true, lastTop: 100 })
  })
})

describe('rule 7 — write notice', () => {
  it('a write that is already at the bottom has neither notice nor write', () => {
    const out = step(state({ lastTop: BOTTOM }), { type: 'resize', m: at(BOTTOM) })
    expect(out.scrollTo).toBeNull()
    expect(out.state.expectTop).toBeNull()
  })

  it('within 0.5px is already that place · farther leaves a notice', () => {
    expect(step(state(), { type: 'resize', m: at(BOTTOM - 0.4) }).state.expectTop).toBeNull()
    const far = step(state(), { type: 'resize', m: at(BOTTOM - 0.6) })
    expect(far.scrollTo).toBe(BOTTOM)
    expect(far.state.expectTop).toBe(BOTTOM)
  })

  it('writing moves lastTop to where the write will land', () => {
    const out = step(state({ lastTop: 600 }), { type: 'resize', m: at(600) })
    expect(out.state.lastTop).toBe(BOTTOM)
  })

  // The scrollbar was dragged to 900 before the scroll event of the growth write (800 → 1000) came — events are coalesced and only the single 900 comes.
  it('if the user drags up before the write\'s event comes it releases — the next growth does not pull down', () => {
    const grow = step(state({ lastTop: BOTTOM }), { type: 'resize', m: at(BOTTOM, HEIGHT + 200) })
    expect(grow.scrollTo).toBe(BOTTOM + 200)
    const dragged = step(grow.state, { type: 'scroll', m: at(BOTTOM + 100, HEIGHT + 200) })
    expect(dragged.state).toMatchObject({ pinned: false, lastTop: BOTTOM + 100, expectTop: null })
    expect(step(dragged.state, { type: 'resize', m: at(BOTTOM + 100, HEIGHT + 400) }).scrollTo).toBeNull()
  })
})

describe('isScrollable — button condition', () => {
  it('true if content is larger than the window by over 0.5px', () => {
    expect(isScrollable(at(0))).toBe(true)
    expect(isScrollable(at(0, CLIENT, CLIENT))).toBe(false)
    expect(isScrollable(at(0, CLIENT + 0.5, CLIENT))).toBe(false)
    expect(isScrollable(at(0, CLIENT + 1, CLIENT))).toBe(true)
  })
})

describe('attach — new node', () => {
  it('while attached it writes to the bottom and drops the old node\'s notice', () => {
    const out = step(state({ lastTop: 400, expectTop: 400 }), { type: 'attach', m: at(0) })
    expect(out.scrollTo).toBe(BOTTOM)
    expect(out.state).toMatchObject({ lastTop: BOTTOM, expectTop: BOTTOM })
  })

  it('while detached it does not write', () => {
    const out = step(state({ pinned: false, expectTop: 400 }), { type: 'attach', m: at(0) })
    expect(out.scrollTo).toBeNull()
    expect(out.state).toMatchObject({ pinned: false, lastTop: 0, expectTop: null })
  })
})

describe('pin · unpin', () => {
  it('pin attaches and lowers unseenGrowth · no measured value so it does not write', () => {
    const out = step(state({ pinned: false, unseenGrowth: true }), { type: 'pin' })
    expect(out.state).toMatchObject({ pinned: true, unseenGrowth: false })
    expect(out.scrollTo).toBeNull()
  })

  it('unpin only lowers attachment', () => {
    const out = step(state({ lastTop: 300 }), { type: 'unpin' })
    expect(out.state).toEqual(state({ lastTop: 300, pinned: false }))
    expect(out.scrollTo).toBeNull()
  })
})

describe('combination', () => {
  // TRD §2-7 ★latch★ — a small upward wheel within the threshold → that wheel's upward scroll (within the threshold) → still off the bottom → down within the threshold → pinned.
  it('latch: after a small upward wheel releases, that wheel\'s scroll does not re-attach, and returning downward re-attaches', () => {
    const start = state({ lastTop: BOTTOM })
    const wheel = run(start, { type: 'intentUp', m: at(BOTTOM) }, { type: 'scroll', m: at(BOTTOM - 10) })
    expect(wheel.state.pinned).toBe(false)
    const back = step(wheel.state, { type: 'scroll', m: at(BOTTOM) })
    expect(back.state.pinned).toBe(true)
  })

  it('growth after a small upward wheel does not pull down', () => {
    const { state: s, writes } = run(
      state({ lastTop: BOTTOM }),
      { type: 'intentUp', m: at(BOTTOM) },
      { type: 'scroll', m: at(BOTTOM - 10) },
      { type: 'resize', m: at(BOTTOM - 10, HEIGHT + 300) },
    )
    expect(s.pinned).toBe(false)
    expect(last(writes)).toBeNull()
  })

  it('an explicit pin releases the latch — the following growth writes to the bottom', () => {
    const { state: s, writes } = run(
      state({ pinned: false, lastTop: BOTTOM - 10 }),
      { type: 'pin' },
      { type: 'resize', m: at(BOTTOM - 10, HEIGHT + 300) },
    )
    expect(s.pinned).toBe(true)
    expect(last(writes)).toBe(BOTTOM + 300)
  })

  it('attached throughout streaming growth — even when writes and their events alternate', () => {
    let s = state({ lastTop: BOTTOM })
    for (let grow = 1; grow <= 5; grow++) {
      const h = HEIGHT + grow * 40
      const out = step(s, { type: 'resize', m: at(h - 40 - CLIENT, h) })
      expect(out.scrollTo).toBe(h - CLIENT)
      s = step(out.state, { type: 'scroll', m: at(h - CLIENT, h) }).state
    }
    expect(s).toMatchObject({ pinned: true, expectTop: null })
  })

  it('returning after hiding while detached goes to the place being read, returning after hiding while attached goes to the bottom', () => {
    const free = run(
      state({ pinned: false, lastTop: 300 }),
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(0, HEIGHT + 500) },
    )
    expect(last(free.writes)).toBe(300)
    const pinned = run(
      state({ lastTop: BOTTOM }),
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(0, HEIGHT + 500) },
    )
    expect(last(pinned.writes)).toBe(BOTTOM + 500)
  })

  it('pin while hidden → no write → bottom when shown', () => {
    const { writes, state: s } = run(
      state({ pinned: false, lastTop: 300 }),
      { type: 'resize', m: hidden },
      { type: 'pin' },
      { type: 'resize', m: hidden },
      { type: 'resize', m: at(300) },
    )
    expect(writes).toEqual([null, null, null, BOTTOM])
    expect(s.pinned).toBe(true)
  })
})
