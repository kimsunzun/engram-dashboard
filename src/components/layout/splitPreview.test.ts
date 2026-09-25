// splitPreview 단위테스트 — 화해 규칙(TRD §2f 1~6) · 재사상 · 허용 범위(ADR-0227).

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

// ── 화해 규칙 ──

describe('reducePreview — 드래그', () => {
  it('dragStart → drag, dragMove 가 ratio 를 바꾼다', () => {
    const s = reducePreview(dragging(1, 'A', 0.4), { type: 'dragMove', token: 1, ratio: 0.62 })
    expect(s).toEqual({ token: 1, splitId: 'A', ratio: 0.62, phase: 'drag' })
  })

  it('dragStart 는 어떤 상태든 새 드래그로 갈아 끼운다', () => {
    expect(reducePreview(awaiting(9, 1), { type: 'dragStart', token: 2, splitId: 'B', ratio: 0.5 })).toEqual({
      token: 2,
      splitId: 'B',
      ratio: 0.5,
      phase: 'drag',
    })
  })

  it('dragCancel: drag 중이면 null, commit 중이면 무시', () => {
    expect(reducePreview(dragging(1), { type: 'dragCancel', token: 1 })).toBeNull()
    const c = committed(1)
    expect(reducePreview(c, { type: 'dragCancel', token: 1 })).toBe(c)
  })

  it('규칙 1: drag 중 스냅샷은 끌리는 split 이 남아 있으면 미리보기를 그대로 둔다', () => {
    const d = dragging(1, 'A', 0.7)
    expect(reducePreview(d, { type: 'snapshot', version: 50, splitIds: ids('A', 'B') })).toBe(d)
  })

  it('규칙 1: drag 중 끌리는 split id 가 사라진 스냅샷 → null(드래그 취소)', () => {
    expect(reducePreview(dragging(1, 'A'), { type: 'snapshot', version: 50, splitIds: ids('B') })).toBeNull()
  })

  it('drag 중(release 전) 에 온 response·failure 는 무시한다', () => {
    const d = dragging(1)
    expect(reducePreview(d, { type: 'response', token: 1, outcome: 'Unchanged', version: 3, cachedVersion: 3 })).toBe(d)
    expect(reducePreview(d, { type: 'failure', token: 1 })).toBe(d)
  })

  it('미리보기가 없으면 dragStart 말고는 아무것도 만들지 않는다', () => {
    expect(reducePreview(null, { type: 'dragMove', token: 1, ratio: 0.3 })).toBeNull()
    expect(reducePreview(null, { type: 'release', token: 1 })).toBeNull()
    expect(reducePreview(null, { type: 'response', token: 1, outcome: 'Applied', version: 9, cachedVersion: 1 })).toBeNull()
    expect(reducePreview(null, { type: 'snapshot', version: 9, splitIds: ids('A') })).toBeNull()
  })
})

describe('reducePreview — 확정', () => {
  it('규칙 2: release → commit(ratio 유지)', () => {
    const s = reducePreview(reducePreview(dragging(1, 'A', 0.4), { type: 'dragMove', token: 1, ratio: 0.55 }), {
      type: 'release',
      token: 1,
    })
    expect(s).toEqual({ token: 1, splitId: 'A', ratio: 0.55, phase: 'commit' })
  })

  it('규칙 2: Applied 응답 → awaitVersion = 응답 version, 캐시가 아직 아래면 미리보기 유지', () => {
    const s = reducePreview(committed(1), { type: 'response', token: 1, outcome: 'Applied', version: 12, cachedVersion: 11 })
    expect(s).toEqual({ token: 1, splitId: 'A', ratio: 0.4, phase: 'commit', awaitVersion: 12 })
  })

  it('규칙 2: Applied 응답 때 캐시가 이미 그 version 이상이면 즉시 null(같음 포함)', () => {
    const r = (cachedVersion: number) =>
      reducePreview(committed(1), { type: 'response', token: 1, outcome: 'Applied', version: 12, cachedVersion })
    expect(r(12)).toBeNull()
    expect(r(13)).toBeNull()
  })

  it('규칙 2: Unchanged·TooSmall 응답 → 스냅샷을 기다리지 않고 즉시 null', () => {
    for (const outcome of ['Unchanged', 'TooSmall'] as const) {
      // 응답 version 이 캐시보다 위여도 기다리지 않는다 — 무변경이면 그 version 의 스냅샷은 오지 않는다.
      const s = reducePreview(committed(1), { type: 'response', token: 1, outcome, version: 99, cachedVersion: 1 })
      expect(s).toBeNull()
      expect(needsPull(s, 1)).toBe(false)
    }
  })

  it('규칙 2·3: awaitVersion 이상 스냅샷에서 null, 미만이면 유지', () => {
    const a = awaiting(12)
    expect(reducePreview(a, { type: 'snapshot', version: 11, splitIds: ids('A') })).toBe(a)
    expect(reducePreview(a, { type: 'snapshot', version: 12, splitIds: ids('A') })).toBeNull()
    expect(reducePreview(a, { type: 'snapshot', version: 13, splitIds: ids('A') })).toBeNull()
  })

  it('규칙 3: 응답 전 commit 중 스냅샷은 version 과 무관하게 유지', () => {
    const c = committed(1)
    expect(reducePreview(c, { type: 'snapshot', version: 1_000, splitIds: ids('A') })).toBe(c)
  })

  it('규칙 4: 명령 실패 → null', () => {
    expect(reducePreview(committed(1), { type: 'failure', token: 1 })).toBeNull()
    expect(reducePreview(awaiting(12, 1), { type: 'failure', token: 1 })).toBeNull()
  })

  it('규칙 5: 새 드래그가 앞선 확정을 덮고, 옛 token 의 늦은 응답·실패는 무시된다', () => {
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

  it('규칙 5: 확정 대기 중에도 다른 token 의 응답은 awaitVersion 을 바꾸지 않는다', () => {
    const a = awaiting(12, 3)
    expect(reducePreview(a, { type: 'response', token: 2, outcome: 'Unchanged', version: 0, cachedVersion: 0 })).toBe(a)
  })
})

describe('needsPull — 규칙 6(당김 타이머를 걸어 둘 상태)', () => {
  it('Applied 응답 뒤 awaitVersion 이 캐시보다 위인 동안만 참', () => {
    const a = awaiting(12)
    expect(needsPull(a, 11)).toBe(true)
    expect(needsPull(a, 12)).toBe(false)
    expect(needsPull(a, 13)).toBe(false)
  })

  it('미리보기 없음·drag·응답 전 commit 에서는 거짓', () => {
    expect(needsPull(null, 0)).toBe(false)
    expect(needsPull(dragging(), 0)).toBe(false)
    expect(needsPull(committed(), 0)).toBe(false)
  })

  it('당김 대기 = 500ms', () => {
    expect(COMMIT_PULL_DELAY_MS).toBe(500)
  })
})

// ── 재사상 ──
//   A = left_right [0,1]² at 0.5 → x | B
//   B = top_bottom [0.5,1]×[0,1] at 0.4 → y | C
//   C = left_right [0.5,1]×[0.4,1] at 0.65 → z | w
// 값은 셸 geometry 의 경계식(`x0 + (x1 - x0) * ratio`)으로 손으로 푼 것이다.

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
  it('미리보기 없음·모르는 split id·비유한 비율 → 입력 배열 그대로', () => {
    for (const p of [null, preview('nope', 0.3), preview('A', Number.NaN), preview('A', Number.POSITIVE_INFINITY)]) {
      const out = remapRects(SLOTS, SPLITS, p)
      expect(out.slotRects).toBe(SLOTS)
      expect(out.splitRects).toBe(SPLITS)
    }
  })

  it('끌린 분할의 경계 B → B′ 가 정확하고, 그 경계를 공유하는 칸·분할 좌표가 모두 B′ 로 같다', () => {
    const B2 = 0.5 + (1 - 0.5) * 0.6
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('C', 0.6))
    expect(bySplit(splitRects, 'C').at).toBe(B2)
    expect(bySlot(slotRects, 'z').x1).toBe(B2)
    expect(bySlot(slotRects, 'w').x0).toBe(B2)
    // 상자 가장자리는 제자리.
    expect(bySlot(slotRects, 'z').x0).toBe(0.5)
    expect(bySlot(slotRects, 'w').x1).toBe(1)
    expect(bySplit(splitRects, 'C')).toMatchObject({ x0: 0.5, x1: 1, y0: 0.4, y1: 1 })
  })

  it('상자 밖 사각형은 같은 객체 그대로, 다른 축 좌표도 그대로', () => {
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('C', 0.6))
    expect(bySlot(slotRects, 'x')).toBe(bySlot(SLOTS, 'x'))
    expect(bySlot(slotRects, 'y')).toBe(bySlot(SLOTS, 'y'))
    expect(bySplit(splitRects, 'A')).toBe(bySplit(SPLITS, 'A'))
    expect(bySplit(splitRects, 'B')).toBe(bySplit(SPLITS, 'B'))
    expect(bySlot(slotRects, 'z')).toMatchObject({ y0: 0.4, y1: 1 })
  })

  it('입력 배열·객체를 바꾸지 않는다', () => {
    const before = JSON.stringify([SLOTS, SPLITS])
    remapRects(SLOTS, SPLITS, preview('A', 0.2))
    expect(JSON.stringify([SLOTS, SPLITS])).toBe(before)
  })

  it('조상을 끌면 서브트리의 중첩 분할 at(같은 축)까지 재사상되고 셸 재계산과 같다', () => {
    // A: 0.5 → 0.3. 셸이 새 트리를 풀면 B 상자 x = [0.3,1], C 상자 x = [0.3,1], C.at = 0.3 + 0.7·0.3.
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('A', 0.3))
    const A2 = 0 + (1 - 0) * 0.3
    const c = bySplit(splitRects, 'C')
    expect(bySplit(splitRects, 'A').at).toBe(A2)
    expect(bySlot(slotRects, 'x')).toMatchObject({ x0: 0, x1: A2 })
    expect(bySlot(slotRects, 'y').x0).toBe(A2)
    expect(bySplit(splitRects, 'B')).toMatchObject({ x0: A2, x1: 1, at: 0.4 })
    expect(c.x0).toBe(A2)
    expect(c.at).toBeCloseTo(0.3 + 0.7 * 0.3, 12)
    // 공유 경계는 비트 단위로 같게 남는다.
    expect(bySlot(slotRects, 'z').x1).toBe(c.at)
    expect(bySlot(slotRects, 'w').x0).toBe(c.at)
    expect(bySlot(slotRects, 'z').x0).toBe(bySlot(slotRects, 'y').x0)
  })

  it('위아래 분할은 y 축만 옮긴다(ADR-0140 — a = 위)', () => {
    const { slotRects, splitRects } = remapRects(SLOTS, SPLITS, preview('B', 0.6))
    const B2 = 0 + (1 - 0) * 0.6
    expect(bySplit(splitRects, 'B').at).toBe(B2)
    expect(bySlot(slotRects, 'y')).toMatchObject({ x0: 0.5, x1: 1, y0: 0, y1: B2 })
    expect(bySplit(splitRects, 'C')).toMatchObject({ y0: B2, y1: 1, x0: 0.5, x1: 1, at: cAt })
    expect(bySlot(slotRects, 'z').y0).toBe(B2)
    expect(bySlot(slotRects, 'w').y0).toBe(B2)
    expect(bySlot(slotRects, 'x')).toBe(bySlot(SLOTS, 'x'))
  })

  it('범위 밖 비율은 상자 안에 갇힌다', () => {
    const { slotRects } = remapRects(SLOTS, SPLITS, preview('C', 1.7))
    for (const id of ['z', 'w']) {
      const r = bySlot(slotRects, id)
      expect(r.x0).toBeGreaterThanOrEqual(0.5)
      expect(r.x1).toBeLessThanOrEqual(1)
      expect(r.x0).toBeLessThanOrEqual(r.x1)
    }
  })

  it('경계가 상자 끝에 붙은 퇴화 분할(B == X0)에서도 NaN·무한이 나오지 않는다', () => {
    const slots = [slot('p', 0, 0, 0, 1), slot('q', 0, 0, 1, 1)]
    const splits = [splitR('D', 'left_right', 0, 0, 1, 1, 0)]
    const { slotRects, splitRects } = remapRects(slots, splits, preview('D', 0.5))
    for (const r of [...slotRects, ...splitRects]) {
      for (const v of [r.x0, r.x1, r.y0, r.y1]) expect(Number.isFinite(v)).toBe(true)
    }
    expect(bySplit(splitRects, 'D').at).toBe(0.5)
  })
})

// ── 허용 범위 ──

// 셸 `ViewManager::set_split_ratio` ④ 의 px 클램프(src-tauri/src/layout/manager.rs)를 옮긴 오라클.
//   `len = canvas × extent; edge = m / len; lo = max(RATIO_MIN, edge); hi = min(RATIO_MAX, 1 − edge); lo > hi → 빈 범위`.
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

  it('두 쪽 모두 30px 이상', () => {
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

  it('스냅샷의 ratio_min/max 를 넘지 않는다', () => {
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 1920, boxExtent: 1 })).toEqual({ lo: 0.1, hi: 0.9 })
    expect(ratioRange({ ratioMin: 0.2, ratioMax: 0.7, minPanePx: 30, axisPx: 1920, boxExtent: 1 })).toEqual({ lo: 0.2, hi: 0.7 })
  })

  it('L < 60 → null(잠김), L = 60 → 0.5 한 점', () => {
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 59, boxExtent: 1 })).toBeNull()
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 1000, boxExtent: 0.05 })).toBeNull()
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 0, boxExtent: 1 })).toBeNull()
    expect(ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: 30, axisPx: 60, boxExtent: 1 })).toEqual({ lo: 0.5, hi: 0.5 })
  })

  it('같은 입력(보고한 정수 캔버스 × 스냅샷 상자 폭)이면 셸 식과 비트 단위로 같다', () => {
    // 상자 폭은 셸 geometry 처럼 경계 차로 만든다.
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
  it('분할 상자 기준 비율로 바꾼다', () => {
    expect(ratioFromPointer({ pointerNorm: 0.5, box0: 0, box1: 1, range })).toBe(0.5)
    expect(ratioFromPointer({ pointerNorm: 0.75, box0: 0.5, box1: 1, range })).toBe(0.5)
  })
  it('범위 밖은 가장 가까운 끝으로 자른다', () => {
    expect(ratioFromPointer({ pointerNorm: 0, box0: 0, box1: 1, range })).toBe(0.1)
    expect(ratioFromPointer({ pointerNorm: 2, box0: 0, box1: 1, range })).toBe(0.9)
    expect(ratioFromPointer({ pointerNorm: 0.52, box0: 0.5, box1: 1, range: { lo: 0.3, hi: 0.7 } })).toBe(0.3)
  })
  it('NaN(폭 0 상자) → range.lo', () => {
    expect(ratioFromPointer({ pointerNorm: 0.5, box0: 0.5, box1: 0.5, range })).toBe(0.1)
    expect(ratioFromPointer({ pointerNorm: Number.NaN, box0: 0, box1: 1, range })).toBe(0.1)
  })
})
