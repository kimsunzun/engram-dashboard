// TRD S21-slot-docking §4-1 · §8-2 · 6.3판 §5-1 · §11 「TS tabStrip」: 탭바 안 순서 이동 순수 기하의 표 시험 — 기대값은 Chromium 규칙을 손으로
//   셈한 것이다(제자리 후보 건너뜀 · 앞 쪽 동률 · 끄는 탭 뒤 −1 · 보이는 끝 자르기 · 문턱).

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

/** 폭 목록 → 한 탭바의 이상적 경계(모든 탭 폭 누적). 앞쪽 `shown` 개가 보이고, `dragged` 는 숨겨도 보이는 탭으로 센다. */
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
/** 단계 B — 활성(끄는 탭) A · 비활성 I 와 그 사이(`layout.ts` 가 내는 꼴). */
const PHASE_B = stripOf([I, A, I, I])
/** 넘침 — 보이는 셋(A · I · I) 뒤 숨긴 둘이 보이는 끝 116 너머에 제 폭으로 선다(116–148 · 148–180). */
const OVERFLOW = stripOf([A, I, I, I, I], 3)
/** 넘침 · 끄는 탭(활성)이 끝 너머 4 — 레이아웃은 숨기지만 끄는 탭이라 보이는 탭으로 센다(숨긴 3 = 96–128 · 4 = 128–180). */
const DRAGGED_HIDDEN = stripOf([I, I, I, I, A], 3, 4)

describe('idealBounds — 모든 탭 계산 폭 누적 · 보임 · 보이는 마지막 자리', () => {
  it('숨긴 탭도 보이는 끝 너머에 제 폭으로 선다 · 보이는 마지막 = 2', () => {
    expect(idealBounds(tabStripLayout(A + 3 * I - 3, 5, 0))).toEqual(OVERFLOW)
    expect(OVERFLOW.tabs[3]).toEqual({ x0: A + 2 * I, x1: A + 3 * I })
    expect(OVERFLOW.lastVisible).toBe(2)
  })
  it('넉넉하면 모두 표준 폭', () => {
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
  it('머리 없음', () => {
    expect(idealBounds(tabStripLayout(500, 0, -1))).toEqual(EMPTY)
  })
  it('끄는 탭은 레이아웃이 숨겨도 보이는 탭으로 센다(Chrome — 끄는 탭은 늘 보인다)', () => {
    expect(idealBounds(tabStripLayout(135, 5, 4), 4)).toEqual(DRAGGED_HIDDEN)
    expect(DRAGGED_HIDDEN.lastVisible).toBe(4)
  })
  it('끄는 탭이 보이면 dragged 는 결과를 바꾸지 않는다', () => {
    expect(idealBounds(tabStripLayout(135, 5, 0), 0)).toEqual(OVERFLOW)
  })
})

describe('dragAreaWidth — 잰 폭 · 재지 못하면 보이는 탭 폭 합', () => {
  const CASES: { name: string; available: number | null; B: StripBounds; out: number }[] = [
    { name: '마지막 탭 뒤 빈 곳까지(잰 폭이 넓음)', available: 500, B: FOUR, out: 500 },
    { name: '탭 폭 합이 더 넓어도 잰 폭(Chrome 탭바 폭 ≤ 쓸 수 있는 폭)', available: 300, B: FOUR, out: 300 },
    { name: '보이는 탭이 잰 폭을 채워도 잰 폭', available: 100, B: OVERFLOW, out: 100 },
    { name: '레이아웃이 숨긴 끄는 탭을 보이게 세도 잰 폭(96 + 52 = 148 이 아니라 135)', available: 135, B: DRAGGED_HIDDEN, out: 135 },
    { name: '소수 폭은 그대로', available: 450.5, B: FOUR, out: 450.5 },
    { name: '재지 못함(null) = 탭 폭 합', available: null, B: FOUR, out: 400 },
    { name: '재지 못함 · 숨긴 탭은 합에 안 든다', available: null, B: OVERFLOW, out: A + 2 * I },
    { name: '유한하지 않은 수도 재지 못함', available: Number.NaN, B: FOUR, out: 400 },
    { name: '탭 없음', available: 200, B: EMPTY, out: 200 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(dragAreaWidth(c.available, c.B)).toBe(c.out)
    })
  }
})

describe('reorderAreaWidth — 끌기 영역을 보이는 탭 폭 합으로 자름(끄는 탭이 마지막 자리 너머 `+` 위로 가지 않는다)', () => {
  const CASES: { name: string; available: number | null; B: StripBounds; out: number }[] = [
    { name: '마지막 탭 뒤 빈 곳은 뺀다', available: 500, B: FOUR, out: 400 },
    { name: '잰 폭이 더 좁으면 잰 폭', available: 300, B: FOUR, out: 300 },
    { name: '숨긴 탭은 합에 안 든다', available: 1000, B: OVERFLOW, out: A + 2 * I },
    { name: '재지 못함(null) = 탭 폭 합', available: null, B: FOUR, out: 400 },
    { name: '탭 없음', available: 200, B: EMPTY, out: 0 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(reorderAreaWidth(c.available, c.B)).toBe(c.out)
    })
  }

  it('탭 둘 · 넓은 탭바 — 오른쪽 끝까지 끌어도 끄는 탭은 마지막 자리에서 멈추고 그 자리에 넣는다', () => {
    const B = stripOf([160, 160])
    const L = draggedLeft(900, 80, 160, reorderAreaWidth(800, B))
    expect(L).toBe(160)
    expect(insertionIndex(B, 0, L)).toBe(1)
  })
})

describe('grabOffset — round(r · 세션 시작 때 폭)', () => {
  const CASES: { name: string; r: number; w0: number; out: number }[] = [
    { name: '가운데', r: 0.5, w0: 100, out: 50 },
    { name: '.5 는 위로(0 에서 멀리) — 4.5 → 5', r: 0.125, w0: 36, out: 5 },
    { name: '.5 미만은 아래로 — 0.375·99 = 37.125 → 37', r: 0.375, w0: 99, out: 37 },
    { name: '음수 .5 도 0 에서 멀리 — −4.5 → −5', r: -0.125, w0: 36, out: -5 },
    { name: '왼쪽 끝', r: 0, w0: 160, out: 0 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(grabOffset(c.r, c.w0)).toBe(c.out)
    })
  }
})

describe('draggedLeft — clamp(p − offset, 0, max(0, A − w))', () => {
  const CASES: { name: string; p: number; offset: number; w: number; A: number; out: number }[] = [
    { name: '가운데를 잡음', p: 150, offset: 50, w: 100, A: 400, out: 100 },
    { name: '소수 커서는 그대로', p: 150.25, offset: 50, w: 100, A: 400, out: 100.25 },
    { name: '음수 오프셋', p: 50, offset: -5, w: 36, A: 400, out: 55 },
    { name: '왼쪽 끝에서 멈춤', p: 10, offset: 50, w: 100, A: 400, out: 0 },
    { name: '끌기 영역 오른쪽 끝 − w 에서 멈춤', p: 390, offset: 50, w: 100, A: 400, out: 300 },
    { name: '딱 끝', p: 350, offset: 50, w: 100, A: 400, out: 300 },
    { name: '탭이 영역보다 넓으면 늘 0', p: 200, offset: 50, w: 100, A: 60, out: 0 },
    { name: '폭이 세션 중 바뀌어도 오프셋은 그대로 — 자르기만 지금 폭', p: 390, offset: 21, w: 32, A: 400, out: 368 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(draggedLeft(c.p, c.offset, c.w, c.A)).toBe(c.out)
    })
  }
})

describe('insertionIndex — Chromium CalculateInsertionIndex', () => {
  const CASES: { name: string; B: StripBounds; d: number; L: number; out: number }[] = [
    // ── 같은 폭 넷 · 가운데 탭(d=1) — 후보 이상적 x: k0 0 · k1 100 · (k2 건너뜀) · k3 200 · k4 300 ──
    { name: '제자리(L = 지금 x)', B: FOUR, d: 1, L: 100, out: 1 },
    { name: '앞 쪽 동률(k0 · k1 거리 50) → 앞 후보 k0', B: FOUR, d: 1, L: 50, out: 0 },
    { name: 'k0 쪽으로 1 px 더', B: FOUR, d: 1, L: 49, out: 0 },
    { name: 'k1 쪽으로 1 px 더', B: FOUR, d: 1, L: 51, out: 1 },
    { name: '뒤 쪽 동률(k1 · k3 거리 50) → 앞 후보 k1 = 제자리', B: FOUR, d: 1, L: 150, out: 1 },
    { name: '동률을 넘으면 k3 → 끄는 탭 뒤라 −1 = 2', B: FOUR, d: 1, L: 150.5, out: 2 },
    { name: 'k3 · k4 동률 → k3 → 2', B: FOUR, d: 1, L: 250, out: 2 },
    { name: '끝(k4 → 3)', B: FOUR, d: 1, L: 300, out: 3 },
    { name: '영역 끝 너머 L 도 끝', B: FOUR, d: 1, L: 900, out: 3 },
    { name: '왼쪽 너머 L 은 맨 앞', B: FOUR, d: 1, L: -40, out: 0 },

    // ── 맨 앞 탭(d=0) — k1 건너뜀 · k2 100 · k3 200 · k4 300 ──
    { name: 'd=0 · k0 · k2 동률 → k0 = 제자리', B: FOUR, d: 0, L: 50, out: 0 },
    { name: 'd=0 · 동률을 넘으면 k2 → 1', B: FOUR, d: 0, L: 50.01, out: 1 },
    { name: 'd=0 · 끝', B: FOUR, d: 0, L: 300, out: 3 },

    // ── 맨 끝 탭(d=3) — k4 건너뜀 · k3 300 이 제자리 ──
    { name: 'd=3 · k2 · k3 동률 → k2(끄는 탭 앞이라 −1 없음)', B: FOUR, d: 3, L: 250, out: 2 },
    { name: 'd=3 · 제자리', B: FOUR, d: 3, L: 251, out: 3 },
    { name: 'd=3 · 맨 앞', B: FOUR, d: 3, L: 0, out: 0 },

    // ── 폭이 다름(단계 B · 끄는 탭 = 활성 A, d=1) — k0 0 · k1 32 · k3 116 − 52 = 64 · k4 96 ──
    { name: '단계 B · k1 · k3 가운데(48) 동률 → k1 = 제자리', B: PHASE_B, d: 1, L: 48, out: 1 },
    { name: '단계 B · 가운데 넘음 → k3 → 2', B: PHASE_B, d: 1, L: 48.5, out: 2 },
    { name: '단계 B · k3 · k4 가운데(80) 동률 → 2', B: PHASE_B, d: 1, L: 80, out: 2 },
    { name: '단계 B · k4 → 3', B: PHASE_B, d: 1, L: 81, out: 3 },

    // ── 넘친 탭바 — 숨긴 탭은 제 폭 · 보이는 마지막 자리로 자른다 ──
    // OVERFLOW · d=0(A): k0 0 · k2 32 · k3 64 · k4 148 − 52 = 96 · k5 128.
    { name: '넘침 · 숨긴 탭 사이 후보(k4 → 3)가 이겨도 보이는 마지막 자리 2 로 자른다', B: OVERFLOW, d: 0, L: 93, out: 2 },
    { name: '넘침 · 보이는 범위 안은 그대로(k3 → 2)', B: OVERFLOW, d: 0, L: 64, out: 2 },
    { name: '넘침 · 보이는 범위 안(k2 → 1)', B: OVERFLOW, d: 0, L: 40, out: 1 },
    // DRAGGED_HIDDEN · d=4(A): k0 0 · k1 32 · k2 64 · k3 96 · k4 128(제자리) · k5 건너뜀.
    { name: '★끝 너머 끄는 탭의 제자리 — 숨긴 탭 앞 후보와 동률이 아니라 그대로 4★', B: DRAGGED_HIDDEN, d: 4, L: 128, out: 4 },
    { name: '끝 너머 끄는 탭 · 제자리 쪽이 가까우면 그대로', B: DRAGGED_HIDDEN, d: 4, L: 120, out: 4 },
    { name: '끝 너머 끄는 탭 · k3 · k4 가운데(112) 동률 → 앞 k3', B: DRAGGED_HIDDEN, d: 4, L: 112, out: 3 },
    { name: '끝 너머 끄는 탭 · 숨긴 탭 앞으로', B: DRAGGED_HIDDEN, d: 4, L: 96, out: 3 },
    { name: '끝 너머 끄는 탭도 맨 앞으로는 간다', B: DRAGGED_HIDDEN, d: 4, L: 0, out: 0 },

    // ── 가장자리 ──
    { name: '탭 하나 — 늘 0', B: stripOf([160]), d: 0, L: 500, out: 0 },
    { name: '보이는 탭 없음 — 0(Chrome 「last_insertion_point = 0」)', B: stripOf([100, 100], 0), d: 1, L: 100, out: 0 },
    { name: '탭 없음 — 0(Chrome 「the only position to insert at is 0」)', B: EMPTY, d: 0, L: 10, out: 0 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(insertionIndex(c.B, c.d, c.L)).toBe(c.out)
    })
  }

  it('끄는 탭 자리가 범위 밖이면 RangeError', () => {
    expect(() => insertionIndex(FOUR, 4, 0)).toThrow(RangeError)
    expect(() => insertionIndex(FOUR, -1, 0)).toThrow(RangeError)
    expect(() => insertionIndex(FOUR, 1.5, 0)).toThrow(RangeError)
  })

  it('불변식 — 결과는 늘 0..보이는 마지막 자리 · L = 지금 x 면 제자리(레이아웃 1–7 탭 · 폭 0–600 / 7 · 끄는 탭 = 활성 전부)', () => {
    const bad: string[] = []
    for (let n = 1; n <= 7; n += 1) {
      for (let available = 0; available <= 600; available += 7) {
        for (let d = 0; d < n; d += 1) {
          const B = idealBounds(tabStripLayout(available, n, d), d)
          if (insertionIndex(B, d, B.tabs[d].x0) !== d) bad.push(`제자리 n=${n} av=${available} d=${d}`)
          for (let L = -40; L <= available + 40; L += 9) {
            const to = insertionIndex(B, d, L)
            if (to < 0 || to > B.lastVisible) bad.push(`범위 n=${n} av=${available} d=${d} L=${L} → ${to}`)
          }
        }
      }
    }
    expect(bad).toEqual([])
  })
})

describe('reorderThreshold — round(지금 자리 탭 폭 ÷ 160 × 16)', () => {
  const CASES: { name: string; widths: number[]; to: number; out: number }[] = [
    { name: '표준 폭 = 16', widths: [160, 160], to: 1, out: 16 },
    { name: '100 → 10', widths: [100, 100], to: 0, out: 10 },
    { name: '활성 최소 A(52) → 5.2 → 5', widths: [A, I], to: 0, out: 5 },
    { name: '45 → 4.5 → 5(.5 는 위로)', widths: [32, 45], to: 1, out: 5 },
    { name: '비활성 최소 I(32) → 3.2 → 3', widths: [A, I], to: 1, out: 3 },
    { name: '자리 to 의 탭 폭을 쓴다(끄는 탭 폭 아님)', widths: [A, I, I], to: 2, out: 3 },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(reorderThreshold(stripOf(c.widths), c.to)).toBe(c.out)
    })
  }
  it('자리가 범위 밖이면 RangeError', () => {
    expect(() => reorderThreshold(FOUR, 4)).toThrow(RangeError)
  })
})

describe('moveDue — 붙은 직후 또는 |p − pLast| > 문턱', () => {
  const CASES: { name: string; pLast: number; p: number; thr: number; just: boolean; out: boolean }[] = [
    { name: '문턱 그대로는 아님', pLast: 100, p: 116, thr: 16, just: false, out: false },
    { name: '문턱을 넘음', pLast: 100, p: 116.5, thr: 16, just: false, out: true },
    { name: '왼쪽으로도 같다', pLast: 100, p: 83.9, thr: 16, just: false, out: true },
    { name: '왼쪽 문턱 그대로는 아님', pLast: 100, p: 84, thr: 16, just: false, out: false },
    { name: '붙은 직후는 문턱 없이', pLast: 100, p: 100, thr: 16, just: true, out: true },
  ]
  for (const c of CASES) {
    it(c.name, () => {
      expect(moveDue(c.pLast, c.p, c.thr, c.just)).toBe(c.out)
    })
  }
})

describe('slideTargets · slideMs — 비켜남(C2)', () => {
  it('끄는 탭 밖의 탭은 새 이상적 x · 끄는 탭 · 숨긴 탭은 null', () => {
    expect(slideTargets(stripOf([100, 100, 100]), 2)).toEqual([0, 100, null])
    expect(slideTargets(stripOf([A, I, I, I], 3), 0)).toEqual([null, A, A + I, null])
  })

  it('200 ms · 줄인 움직임 또는 막 붙음이면 0', () => {
    expect(SLIDE_MS).toBe(200)
    expect(slideMs(false, false)).toBe(200)
    expect(slideMs(true, false)).toBe(0)
    expect(slideMs(false, true)).toBe(0)
  })

  it('EASE_OUT_CSS 는 1 − (1 − t)² 를 그린다(3차 베지어를 표본으로 대조)', () => {
    const [x1, y1, x2, y2] = EASE_OUT_CSS.replace(/^cubic-bezier\(|\)$/g, '').split(',').map(Number)
    const bez = (a: number, b: number, s: number) => 3 * (1 - s) ** 2 * s * a + 3 * (1 - s) * s ** 2 * b + s ** 3
    let worst = 0
    for (let i = 0; i <= 100; i += 1) {
      const s = i / 100
      worst = Math.max(worst, Math.abs(bez(y1, y2, s) - easeOut(bez(x1, x2, s))))
    }
    // 제어점을 넷째 자리에서 끊은 만큼만 어긋난다.
    expect(worst).toBeLessThan(1e-3)
    expect(easeOut(0)).toBe(0)
    expect(easeOut(1)).toBe(1)
    expect(easeOut(0.5)).toBe(0.75)
  })
})

describe('dropIndex · dropMarkerX — 다른 창 탭바의 놓을 자리 · 세로선(W13)', () => {
  it('보이는 머리 중 가운데가 x 보다 왼쪽인 것의 수 · 가운데와 같은 x = 그 머리의 왼쪽', () => {
    // 가운데 = 50 · 150 · 250 · 350
    expect(dropIndex(FOUR, -5, null)).toBe(0)
    expect(dropIndex(FOUR, 49, null)).toBe(0)
    expect(dropIndex(FOUR, 50, null)).toBe(0)
    expect(dropIndex(FOUR, 51, null)).toBe(1)
    expect(dropIndex(FOUR, 150, null)).toBe(1)
    expect(dropIndex(FOUR, 349, null)).toBe(3)
    expect(dropIndex(FOUR, 351, null)).toBe(4)
  })

  it('빈 곳(보이는 끝 너머) = 끝', () => {
    expect(dropIndex(FOUR, 1000, null)).toBe(4)
    expect(dropMarkerX(FOUR, 4, null)).toBe(400)
  })

  it('숨긴 탭이 넘친 탭바는 보이는 마지막 머리 다음 자리에서 자른다(R24)', () => {
    // 보이는 셋 = 0–52 · 52–84 · 84–116 · 숨긴 둘이 116–148 · 148–180 에 제 폭으로 선다.
    expect(dropIndex(OVERFLOW, 120, null)).toBe(3)
    expect(dropIndex(OVERFLOW, 160, null)).toBe(3)
    expect(dropMarkerX(OVERFLOW, 3, null)).toBe(A + 2 * I)
  })

  it('탭 0 개 = 자리 0 · 선 0', () => {
    expect(dropIndex(EMPTY, 30, null)).toBe(0)
    expect(dropMarkerX(EMPTY, 0, null)).toBe(0)
  })

  it('탭 1 개 = 가운데 앞 0 · 뒤 1', () => {
    const one = stripOf([100])
    expect(dropIndex(one, 50, null)).toBe(0)
    expect(dropIndex(one, 51, null)).toBe(1)
    expect(dropMarkerX(one, 0, null)).toBe(0)
    expect(dropMarkerX(one, 1, null)).toBe(100)
  })

  it('선 = 그 자리 왼쪽 머리 오른쪽 끝(= 오른쪽 머리 왼쪽 끝) · 범위 밖 자리는 끝으로 자름', () => {
    expect(dropMarkerX(FOUR, 0, null)).toBe(0)
    expect(dropMarkerX(FOUR, 1, null)).toBe(100)
    expect(dropMarkerX(FOUR, 2, null)).toBe(200)
    expect(dropMarkerX(FOUR, -3, null)).toBe(0)
    expect(dropMarkerX(FOUR, 9, null)).toBe(400)
  })

  it('끄는 탭이 이 탭바에 있으면 세지 않는다', () => {
    // 끄는 탭 1(100–200)을 빼면 머리 = 0–100 · 200–300 · 300–400.
    expect(dropIndex(FOUR, 260, 1)).toBe(2)
    expect(dropIndex(FOUR, 240, 1)).toBe(1)
    expect(dropMarkerX(FOUR, 1, 1)).toBe(100)
  })
})
