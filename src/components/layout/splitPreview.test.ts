// splitPreview unit test — reconcile rules (TRD §2f 1~6) · remapping · allowed range (ADR-0227).

import { describe, expect, it } from 'vitest'

import type { SlotRect, SplitRect } from '../../api/layoutTypes'
import {
  COMMIT_PULL_DELAY_MS,
  MIN_PANE_PX,
  needsPull,
  ratioFromPointer,
  ratioRange,
  reducePreview,
  remapRects,
  type PreviewState,
  type SplitPreview,
} from './splitPreview'

const ids = (...s: string[]): ReadonlySet<string> => new Set(s)

const dragging = (token = 1, splitId = 'A', ratio = 0.4): PreviewState =>
  reducePreview(null, { type: 'dragStart', token, splitId, ratio })
const committed = (token = 1, splitId = 'A'): PreviewState =>
  reducePreview(dragging(token, splitId), { type: 'release', token })
const awaiting = (version: number, token = 1): PreviewState =>
  reducePreview(committed(token), { type: 'response', token, outcome: 'Applied', version, cachedVersion: version - 1 })

// ── reconcile rules ──

describe('reducePreview — drag', () => {
  it('dragStart → drag, dragMove changes ratio', () => {
    const s = reducePreview(dragging(1, 'A', 0.4), { type: 'dragMove', token: 1, ratio: 0.62 })
    expect(s).toEqual({ token: 1, splitId: 'A', ratio: 0.62, phase: 'drag' })
  })

  it('dragStart replaces any state with a new drag', () => {
    expect(reducePreview(awaiting(9, 1), { type: 'dragStart', token: 2, splitId: 'B', ratio: 0.5 })).toEqual({
      token: 2,
      splitId: 'B',
      ratio: 0.5,
      phase: 'drag',
    })
  })

  it('dragCancel: null while dragging, ignored while committing', () => {
    expect(reducePreview(dragging(1), { type: 'dragCancel', token: 1 })).toBeNull()
    const c = committed(1)
    expect(reducePreview(c, { type: 'dragCancel', token: 1 })).toBe(c)
  })

  it('rule 1: a snapshot during drag leaves the preview as is if the dragged split remains', () => {
    const d = dragging(1, 'A', 0.7)
    expect(reducePreview(d, { type: 'snapshot', version: 50, splitIds: ids('A', 'B') })).toBe(d)
  })

  it('rule 1: a snapshot during drag in which the dragged split id vanished → null (drag cancelled)', () => {
    expect(reducePreview(dragging(1, 'A'), { type: 'snapshot', version: 50, splitIds: ids('B') })).toBeNull()
  })

  it('a response · failure arriving during drag (before release) is ignored', () => {
    const d = dragging(1)
    expect(reducePreview(d, { type: 'response', token: 1, outcome: 'Unchanged', version: 3, cachedVersion: 3 })).toBe(d)
    expect(reducePreview(d, { type: 'failure', token: 1 })).toBe(d)
  })

  it('with no preview nothing is made except by dragStart', () => {
    expect(reducePreview(null, { type: 'dragMove', token: 1, ratio: 0.3 })).toBeNull()
    expect(reducePreview(null, { type: 'release', token: 1 })).toBeNull()
    expect(reducePreview(null, { type: 'response', token: 1, outcome: 'Applied', version: 9, cachedVersion: 1 })).toBeNull()
    expect(reducePreview(null, { type: 'snapshot', version: 9, splitIds: ids('A') })).toBeNull()
  })
})

describe('reducePreview — commit', () => {
  it('rule 2: release → commit (ratio kept)', () => {
    const s = reducePreview(reducePreview(dragging(1, 'A', 0.4), { type: 'dragMove', token: 1, ratio: 0.55 }), {
      type: 'release',
      token: 1,
    })
    expect(s).toEqual({ token: 1, splitId: 'A', ratio: 0.55, phase: 'commit' })
  })

  it('rule 2: Applied response → awaitVersion = response version, if the cache is still below the preview is kept', () => {
    const s = reducePreview(committed(1), { type: 'response', token: 1, outcome: 'Applied', version: 12, cachedVersion: 11 })
    expect(s).toEqual({ token: 1, splitId: 'A', ratio: 0.4, phase: 'commit', awaitVersion: 12 })
  })

  it('rule 2: when the cache is already at or above that version at the Applied response, immediately null (including equal)', () => {
    const r = (cachedVersion: number) =>
      reducePreview(committed(1), { type: 'response', token: 1, outcome: 'Applied', version: 12, cachedVersion })
    expect(r(12)).toBeNull()
    expect(r(13)).toBeNull()
  })

  it('rule 2: Unchanged · TooSmall response → immediately null without waiting for a snapshot', () => {
    for (const outcome of ['Unchanged', 'TooSmall'] as const) {
      // Even if the response version is above the cache, do not wait — if nothing changed, no snapshot of that version arrives.
      const s = reducePreview(committed(1), { type: 'response', token: 1, outcome, version: 99, cachedVersion: 1 })
      expect(s).toBeNull()
      expect(needsPull(s, 1)).toBe(false)
    }
  })

  it('rule 2·3: null at a snapshot at or above awaitVersion, kept if below', () => {
    const a = awaiting(12)
    expect(reducePreview(a, { type: 'snapshot', version: 11, splitIds: ids('A') })).toBe(a)
    expect(reducePreview(a, { type: 'snapshot', version: 12, splitIds: ids('A') })).toBeNull()
    expect(reducePreview(a, { type: 'snapshot', version: 13, splitIds: ids('A') })).toBeNull()
  })

  it('rule 3: a snapshot during commit before the response is kept regardless of version', () => {
    const c = committed(1)
    expect(reducePreview(c, { type: 'snapshot', version: 1_000, splitIds: ids('A') })).toBe(c)
  })

  it('rule 4: command failure → null', () => {
    expect(reducePreview(committed(1), { type: 'failure', token: 1 })).toBeNull()
    expect(reducePreview(awaiting(12, 1), { type: 'failure', token: 1 })).toBeNull()
  })

  it('rule 5: a new drag overwrites the earlier commit, and a late response · failure with an old token is ignored', () => {
    const newer = reducePreview(committed(1, 'A'), { type: 'dragStart', token: 2, splitId: 'B', ratio: 0.3 })
    expect(newer).toEqual({ token: 2, splitId: 'B', ratio: 0.3, phase: 'drag' })
    expect(
      reducePreview(newer, { type: 'response', token: 1, outcome: 'Applied', version: 5, cachedVersion: 9 }),
    ).toBe(newer)
    expect(reducePreview(newer, { type: 'failure', token: 1 })).toBe(newer)
    expect(reducePreview(newer, { type: 'release', token: 1 })).toBe(newer)
    expect(reducePreview(newer, { type: 'dragMove', token: 1, ratio: 0.9 })).toBe(newer)
    expect(reducePreview(newer, { type: 'dragCancel', token: 1 })).toBe(newer)
  })

  it('rule 5: even while awaiting commit, a response with another token does not change awaitVersion', () => {
    const a = awaiting(12, 3)
    expect(reducePreview(a, { type: 'response', token: 2, outcome: 'Unchanged', version: 0, cachedVersion: 0 })).toBe(a)
  })
})

describe('needsPull — rule 6 (the state in which to set a pull timer)', () => {
  it('true only while awaitVersion is above the cache after an Applied response', () => {
    const a = awaiting(12)
    expect(needsPull(a, 11)).toBe(true)
    expect(needsPull(a, 12)).toBe(false)
    expect(needsPull(a, 13)).toBe(false)
  })

  it('false with no preview · drag · commit before the response', () => {
    expect(needsPull(null, 0)).toBe(false)
    expect(needsPull(dragging(), 0)).toBe(false)
    expect(needsPull(committed(), 0)).toBe(false)
  })

  it('pull wait = 500ms', () => {
    expect(COMMIT_PULL_DELAY_MS).toBe(500)
  })
})

// ── remapping ──
//   A = left_right [0,1]² at 0.5 → x | B
//   B = top_bottom [0.5,1]×[0,1] at 0.4 → y | C
//   C = left_right [0.5,1]×[0.4,1] at 0.65 → z | w
// The values were solved by hand with the shell geometry boundary formula (`x0 + (x1 - x0) * ratio`).

const slot = (slot_id: string, x0: number, y0: number, x1: number, y1: number): SlotRect => ({ slot_id, x0, y0, x1, y1 })
const splitR = (split_id: string, dir: SplitRect['dir'], x0: number, y0: number, x1: number, y1: number, at: number): SplitRect => ({
  split_id,
  dir,
  x0,
  y0,
  x1,
  y1,
  at,
})

const cAt = 0.5 + (1 - 0.5) * 0.3
const SLOTS: SlotRect[] = [
  slot('x', 0, 0, 0.5, 1),
  slot('y', 0.5, 0, 1, 0.4),
  slot('z', 0.5, 0.4, cAt, 1),
  slot('w', cAt, 0.4, 1, 1),
]
const SPLITS: SplitRect[] = [
  splitR('A', 'left_right', 0, 0, 1, 1, 0.5),
  splitR('B', 'top_bottom', 0.5, 0, 1, 1, 0.4),
  splitR('C', 'left_right', 0.5, 0.4, 1, 1, cAt),
]
const preview = (splitId: string, ratio: number): SplitPreview => ({ token: 1, splitId, ratio, phase: 'drag' })
const bySlot = (rs: SlotRect[], id: string) => rs.find(r => r.slot_id === id)!
const bySplit = (rs: SplitRect[], id: string) => rs.find(r => r.split_id === id)!

describe('remapRects', () => {
  it('no preview · unknown split id · non-finite ratio → the input array as is', () => {
    for (const p of [null, preview('nope', 0.3), preview('A', Number.NaN), preview('A', Number.POSITIVE_INFINITY)]) {
      const out = remapRects(SLOTS, SPLITS, p)
      expect(out.slotRects).toBe(SLOTS)
      expect(out.splitRects).toBe(SPLITS)
    }
  })

  it('the dragged split\'s boundary B → B′ is exact, and the area · split coordinates sharing that boundary are all the same B′', () => {
    const B2 = 0.5 + (1 - 0.5) * 0.6
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('C', 0.6))
    expect(bySplit(splitRects, 'C').at).toBe(B2)
    expect(bySlot(slotRects, 'z').x1).toBe(B2)
    expect(bySlot(slotRects, 'w').x0).toBe(B2)
    // Box edges stay in place.
    expect(bySlot(slotRects, 'z').x0).toBe(0.5)
    expect(bySlot(slotRects, 'w').x1).toBe(1)
    expect(bySplit(splitRects, 'C')).toMatchObject({ x0: 0.5, x1: 1, y0: 0.4, y1: 1 })
  })

  it('rectangles outside the box are the same object as is, other-axis coordinates too', () => {
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('C', 0.6))
    expect(bySlot(slotRects, 'x')).toBe(bySlot(SLOTS, 'x'))
    expect(bySlot(slotRects, 'y')).toBe(bySlot(SLOTS, 'y'))
    expect(bySplit(splitRects, 'A')).toBe(bySplit(SPLITS, 'A'))
    expect(bySplit(splitRects, 'B')).toBe(bySplit(SPLITS, 'B'))
    expect(bySlot(slotRects, 'z')).toMatchObject({ y0: 0.4, y1: 1 })
  })

  it('does not mutate the input arrays · objects', () => {
    const before = JSON.stringify([SLOTS, SPLITS])
    remapRects(SLOTS, SPLITS, preview('A', 0.2))
    expect(JSON.stringify([SLOTS, SPLITS])).toBe(before)
  })

  it('dragging an ancestor also remaps the nested split at (same axis) of the subtree and equals the shell recomputation', () => {
    // A: 0.5 → 0.3. When the shell unfolds the new tree, B box x = [0.3,1], C box x = [0.3,1], C.at = 0.3 + 0.7·0.3.
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('A', 0.3))
    const A2 = 0 + (1 - 0) * 0.3
    const c = bySplit(splitRects, 'C')
    expect(bySplit(splitRects, 'A').at).toBe(A2)
    expect(bySlot(slotRects, 'x')).toMatchObject({ x0: 0, x1: A2 })
    expect(bySlot(slotRects, 'y').x0).toBe(A2)
    expect(bySplit(splitRects, 'B')).toMatchObject({ x0: A2, x1: 1, at: 0.4 })
    expect(c.x0).toBe(A2)
    expect(c.at).toBeCloseTo(0.3 + 0.7 * 0.3, 12)
    // Shared boundaries remain bit-for-bit equal.
    expect(bySlot(slotRects, 'z').x1).toBe(c.at)
    expect(bySlot(slotRects, 'w').x0).toBe(c.at)
    expect(bySlot(slotRects, 'z').x0).toBe(bySlot(slotRects, 'y').x0)
  })

  it('a top/bottom split moves only the y axis (ADR-0140 — a = top)', () => {
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('B', 0.6))
    const B2 = 0 + (1 - 0) * 0.6
    expect(bySplit(splitRects, 'B').at).toBe(B2)
    expect(bySlot(slotRects, 'y')).toMatchObject({ x0: 0.5, x1: 1, y0: 0, y1: B2 })
    expect(bySplit(splitRects, 'C')).toMatchObject({ y0: B2, y1: 1, x0: 0.5, x1: 1, at: cAt })
    expect(bySlot(slotRects, 'z').y0).toBe(B2)
    expect(bySlot(slotRects, 'w').y0).toBe(B2)
    expect(bySlot(slotRects, 'x')).toBe(bySlot(SLOTS, 'x'))
  })

  it('a ratio outside the range is confined inside the box', () => {
    const { slotRects } = remapRects(SLOTS, SPLITS, preview('C', 1.7))
    for (const id of ['z', 'w']) {
      const r = bySlot(slotRects, id)
      expect(r.x0).toBeGreaterThanOrEqual(0.5)
      expect(r.x1).toBeLessThanOrEqual(1)
      expect(r.x0).toBeLessThanOrEqual(r.x1)
    }
  })

  it('even in a degenerate split whose boundary touches the box end (B == X0) no NaN or infinity comes out', () => {
    const slots = [slot('p', 0, 0, 0, 1), slot('q', 0, 0, 1, 1)]
    const splits = [splitR('D', 'left_right', 0, 0, 1, 1, 0)]
    const { slotRects, splitRects } = remapRects(slots, splits, preview('D', 0.5))
    for (const r of [...slotRects, ...splitRects]) {
      for (const v of [r.x0, r.x1, r.y0, r.y1]) expect(Number.isFinite(v)).toBe(true)
    }
    expect(bySplit(splitRects, 'D').at).toBe(0.5)
  })
})

// ── allowed range ──

// An oracle that transcribes the px clamp of shell `ViewManager::set_split_ratio` ④ (src-tauri/src/layout/manager.rs).
//   `len = canvas × extent; edge = m / len; lo = max(RATIO_MIN, edge); hi = min(RATIO_MAX, 1 − edge); lo > hi → empty range`.
function shellRange(canvasLen: number, extent: number, m: number, rmin: number, rmax: number) {
  const len = canvasLen * extent
  const edge = m / len
  const lo = Math.max(rmin, edge)
  const hi = Math.min(rmax, 1 - edge)
  if (lo > hi) return null
  return { lo, hi }
}

describe('ratioRange', () => {
  it('MIN_PANE_PX = 30', () => {
    expect(MIN_PANE_PX).toBe(30)
  })

  it('both sides at least 30px', () => {
    for (const axisPx of [60, 61, 100, 333, 1366, 1920]) {
      for (const boxExtent of [1, 0.5, 0.35, 0.9 * 0.1 + 0.2]) {
        const r = ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx, boxExtent })
        const L = axisPx * boxExtent
        if (L < 60) {
          expect(r).toBeNull()
          continue
        }
        expect(r).not.toBeNull()
        expect(r!.lo * L).toBeGreaterThanOrEqual(30 - 1e-9)
        expect((1 - r!.hi) * L).toBeGreaterThanOrEqual(30 - 1e-9)
        expect(r!.lo).toBeLessThanOrEqual(r!.hi)
      }
    }
  })

  it('does not exceed the snapshot\'s ratio_min/max', () => {
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 1920, boxExtent: 1 })).toEqual({ lo: 0.1, hi: 0.9 })
    expect(ratioRange({ ratioMin: 0.2, ratioMax: 0.7, minPanePx: 30, axisPx: 1920, boxExtent: 1 })).toEqual({ lo: 0.2, hi: 0.7 })
  })

  it('L < 60 → null (locked), L = 60 → the single point 0.5', () => {
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 59, boxExtent: 1 })).toBeNull()
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 1000, boxExtent: 0.05 })).toBeNull()
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 0, boxExtent: 1 })).toBeNull()
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 60, boxExtent: 1 })).toEqual({ lo: 0.5, hi: 0.5 })
  })

  it('the same input (reported integer canvas × snapshot box width) is bit-for-bit equal to the shell formula', () => {
    // The box width is made from the boundary difference like the shell geometry.
    const extents = [1, 0.5 - 0, 1 - (0.5 + 0.5 * 0.3), (0.5 + 0.5 * 0.3) - 0.5, 0.9 - 0.1 * 0.9, 1 / 3, 0.0625]
    for (const axisPx of [1, 59, 60, 61, 119, 120, 121, 480, 1001, 1366, 1920, 3840]) {
      for (const boxExtent of extents) {
        for (const m of [1, 30, 45]) {
          expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: m, axisPx, boxExtent })).toEqual(
            shellRange(axisPx, boxExtent, m, 0.1, 0.9),
          )
        }
      }
    }
  })
})

describe('ratioFromPointer', () => {
  const range = { lo: 0.1, hi: 0.9 }
  it('converts to a ratio relative to the split box', () => {
    expect(ratioFromPointer({ pointerNorm: 0.5, box0: 0, box1: 1, range })).toBe(0.5)
    expect(ratioFromPointer({ pointerNorm: 0.75, box0: 0.5, box1: 1, range })).toBe(0.5)
  })
  it('out of range is clipped to the nearest end', () => {
    expect(ratioFromPointer({ pointerNorm: 0, box0: 0, box1: 1, range })).toBe(0.1)
    expect(ratioFromPointer({ pointerNorm: 2, box0: 0, box1: 1, range })).toBe(0.9)
    expect(ratioFromPointer({ pointerNorm: 0.52, box0: 0.5, box1: 1, range: { lo: 0.3, hi: 0.7 } })).toBe(0.3)
  })
  it('NaN (zero-width box) → range.lo', () => {
    expect(ratioFromPointer({ pointerNorm: 0.5, box0: 0.5, box1: 0.5, range })).toBe(0.1)
    expect(ratioFromPointer({ pointerNorm: Number.NaN, box0: 0, box1: 1, range })).toBe(0.1)
  })
})
