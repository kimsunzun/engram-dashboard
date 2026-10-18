// TRD S21-slot-docking R24(탭바 넘침 = Chrome): 탭바 머리 폭 · 숨김 순수 함수의 표 시험. jsdom 은 배치를 하지 않아 그려진
//   폭은 여기서 못 잰다(화면 QA AC29) — 이 함수가 그 규칙의 유일한 시험 표면이다.

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
  // ── 재기 전 ──
  { name: '재지 못함(null) = 제한 없음 → 모두 S · 숨김 없음', available: null, n: 3, active: 1, widths: [S, S, S], visible: [T, T, T] },
  { name: '유한하지 않은 수도 재지 못함', available: Number.NaN, n: 2, active: 0, widths: [S, S], visible: [T, T] },

  // ── 단계 A(available ≥ n·A) ──
  { name: '모두 S — 딱 n·S', available: 3 * S, n: 3, active: 0, widths: [S, S, S], visible: [T, T, T] },
  { name: '모두 S — 남는 px 를 나누지 않는다', available: 1000, n: 3, active: 0, widths: [S, S, S], visible: [T, T, T] },
  { name: 'A 와 S 사이 고른 폭 · 남는 1 px 은 맨 왼쪽', available: 400, n: 3, active: 2, widths: [134, 133, 133], visible: [T, T, T] },
  { name: '남는 2 px 은 왼쪽부터 하나씩', available: 302, n: 4, active: 0, widths: [76, 76, 75, 75], visible: [T, T, T, T] },
  { name: '넘김점 그대로(n·A) = 모두 A', available: 3 * A, n: 3, active: 1, widths: [A, A, A], visible: [T, T, T] },
  { name: '넘김점 바로 위 = 모두 A + 남는 1 px', available: 3 * A + 1, n: 3, active: 1, widths: [A + 1, A, A], visible: [T, T, T] },
  { name: '머리 하나 · 활성 — S 보다 좁으면 그 폭 그대로', available: 100, n: 1, active: 0, widths: [100], visible: [T] },
  { name: '머리 하나 · 활성 없음도 단계 A', available: 100, n: 1, active: -1, widths: [100], visible: [T] },

  // ── 단계 B(available < n·A) ──
  { name: '넘김점 바로 아래 — 활성 A 고정 · 비활성만 준다', available: 3 * A - 1, n: 3, active: 1, widths: [A, A, A - 1], visible: [T, T, T] },
  {
    name: '남는 px 은 활성을 건너뛰고 비활성만 왼쪽부터(TabAcceptsExtraSpace — 활성 최소 = 넘김점)',
    available: 140,
    n: 4,
    active: 0,
    widths: [A, I + 1, I + 1, I],
    visible: [T, T, T, T],
  },
  {
    name: '활성이 맨 왼쪽이 아니어도 — 그 앞 비활성은 받고 활성은 건너뛴다',
    available: 140,
    n: 4,
    active: 1,
    widths: [I + 1, A, I + 1, I],
    visible: [T, T, T, T],
  },
  { name: '활성 없음 — 모두 받으므로 맨 왼쪽이 받는다', available: 141, n: 4, active: -1, widths: [36, 35, 35, 35], visible: [T, T, T, T] },
  {
    name: 'f 가 0 으로 묶임(available < A + (n−1)·I) — 넘친 만큼 꼬리 숨김 · 남는 px 없음',
    available: 130,
    n: 4,
    active: 0,
    widths: [A, I, I, I],
    visible: [T, T, T, F],
  },
  { name: '활성 없음 — 모두 비활성 최소 I 기준', available: 140, n: 4, active: -1, widths: [A - 7, A - 7, A - 7, A - 7], visible: [T, T, T, T] },
  { name: '머리 하나 · 비활성 — I 와 A 사이', available: 35, n: 1, active: -1, widths: [35], visible: [T] },

  // ── 숨김(ShouldTabBeVisible) ──
  {
    name: '안정 집합 — 활성이 뒤면 들어가는 머리도 「오른 끝 + (활성 − 이 폭)」이 넘으면 숨김 · 끝 너머 활성도 숨김',
    available: 135,
    n: 5,
    active: 4,
    widths: [I, I, I, I, A],
    visible: [T, T, T, F, F],
  },
  {
    name: '안정 집합 — 활성이 앞이어도 같은 보이는 집합',
    available: 135,
    n: 5,
    active: 0,
    widths: [A, I, I, I, I],
    visible: [T, T, T, F, F],
  },
  {
    name: '활성이 그 자리면 규칙이 걸리지 않고 오른 끝만 본다',
    available: 135,
    n: 5,
    active: 3,
    widths: [I, I, I, A, I],
    visible: [T, T, T, F, F],
  },
  {
    name: '활성 없음 — 안정 집합 규칙이 걸리지 않아 하나 더 보인다',
    available: 135,
    n: 5,
    active: -1,
    widths: [I, I, I, I, I],
    visible: [T, T, T, T, F],
  },
  {
    name: '끝 너머의 활성만 숨고 앞은 다 보인다',
    available: 150,
    n: 5,
    active: 4,
    widths: [I, I, I, I, A],
    visible: [T, T, T, T, F],
  },
  { name: '머리 하나 · 활성 · A 보다 좁음 = 숨김', available: 30, n: 1, active: 0, widths: [A], visible: [F] },
  { name: '머리 하나 · 비활성 · I 보다 좁음 = 숨김', available: 30, n: 1, active: -1, widths: [I], visible: [F] },
  { name: '음수 폭 = 다 숨김', available: -10, n: 2, active: 0, widths: [A, I], visible: [F, F] },

  // ── 가장자리 ──
  { name: '머리 없음', available: 500, n: 0, active: -1, widths: [], visible: [] },
  { name: '머리 없음 · 음수 폭', available: -5, n: 0, active: -1, widths: [], visible: [] },
  { name: '소수 폭은 내린다', available: 400.9, n: 3, active: 0, widths: [134, 133, 133], visible: [T, T, T] },
  { name: '범위 밖 활성 = 활성 없음', available: 135, n: 5, active: 5, widths: [I, I, I, I, I], visible: [T, T, T, T, F] },
]

describe('tabStripLayout — 표', () => {
  for (const c of CASES) {
    it(c.name, () => {
      expect(tabStripLayout(c.available, c.n, c.active)).toEqual({ widths: c.widths, visible: c.visible })
    })
  }
})

describe('tabStripLayout — 불변식(머리 1–9 · 활성 전부 · 폭 0–1600 전수)', () => {
  // 어긋난 입력만 모아 한 번에 단언한다 — 칸마다 expect 를 부르면 수십만 번이라 느리다.
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

  it('숨김은 늘 꼬리다 — 보이는 것 = 앞쪽 k 개', () => {
    const bad = violations((available, n, active) => {
      const { visible } = tabStripLayout(available, n, active)
      const k = visible.indexOf(false)
      return k >= 0 && visible.slice(k).some(v => v)
    })
    expect(bad).toEqual([])
  })

  it('보이는 머리는 available 안에 든다', () => {
    const bad = violations((available, n, active) => {
      const { widths, visible } = tabStripLayout(available, n, active)
      return widths.filter((_, i) => visible[i]).reduce((s, w) => s + w, 0) > available
    })
    expect(bad).toEqual([])
  })

  it('넘치지 않고 모두 S 도 아니면 폭 합 = available(남는 px 를 다 나눈다)', () => {
    const bad = violations((available, n, active) => {
      const minimum = active >= 0 ? A + (n - 1) * I : n * I
      if (available < minimum || available >= n * S) return false
      return tabStripLayout(available, n, active).widths.reduce((s, w) => s + w, 0) !== available
    })
    expect(bad).toEqual([])
  })

  it('단계 B(available < n·A)의 활성은 정확히 A — 남는 px 를 받지 않는다', () => {
    const bad = violations((available, n, active) => {
      if (active < 0 || available >= n * A) return false
      return tabStripLayout(available, n, active).widths[active] !== A
    })
    expect(bad).toEqual([])
  })

  it('받는 머리(단계 A = 모두 · 단계 B = 비활성)끼리는 왼쪽이 넓거나 같고 차이는 많아야 1 px', () => {
    const bad = violations((available, n, active) => {
      const { widths } = tabStripLayout(available, n, active)
      const takers = widths.filter((_, i) => available >= n * A || i !== active)
      return takers.some((w, i) => i > 0 && w > takers[i - 1]) || takers[0] - takers[takers.length - 1] > 1
    })
    expect(bad).toEqual([])
  })

  it('활성은 A 이상 · 비활성은 I 이상 · 누구도 S 를 넘지 않는다', () => {
    const bad = violations((available, n, active) =>
      tabStripLayout(available, n, active).widths.some((w, i) => w < (i === active ? A : I) || w > S),
    )
    expect(bad).toEqual([])
  })
})

describe('tabStripLayout — 폭 상한(닫기 모드의 고정 폭) · 보임은 잰 폭으로', () => {
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
      name: '상한이 좁으면 폭은 상한으로 · 보임은 잰 폭 135 로(상한 80 으로 재면 둘째 뒤가 숨는다)',
      available: 135,
      n: 5,
      active: 0,
      cap: 80,
      widths: [A, I, I, I, I],
      visible: [T, T, T, F, F],
    },
    { name: '닫기 모드 재현 — 고정 103 · 머리 넷', available: 135, n: 4, active: 0, cap: 103, widths: [A, I, I, I], visible: [T, T, T, F] },
    { name: '닫기 모드 재현 — 고정 71 · 머리 둘', available: 135, n: 2, active: 0, cap: 71, widths: [A, I], visible: [T, T] },
    { name: '재지 못함 + 상한 — 폭은 상한으로 · 숨김 없음', available: null, n: 3, active: 0, cap: 200, widths: [67, 67, 66], visible: [T, T, T] },
  ]
  for (const c of CAP_CASES) {
    it(c.name, () => {
      expect(tabStripLayout(c.available, c.n, c.active, c.cap)).toEqual({ widths: c.widths, visible: c.visible })
    })
  }

  it('상한이 잰 폭 이상 · null · 유한하지 않으면 상한 없음과 같다', () => {
    for (const cap of [400, 1000, null, undefined, Number.NaN]) {
      expect(tabStripLayout(400, 3, 2, cap)).toEqual(tabStripLayout(400, 3, 2))
    }
  })

  it('불변식 — 폭 = min(잰 폭, 상한) 의 폭 · 숨김은 꼬리 · 보이는 머리는 잰 폭 안(머리 1–6 · 폭 0–400 · 상한 0–400 / 7)', () => {
    const bad: string[] = []
    for (let n = 1; n <= 6; n += 1) {
      for (let active = -1; active < n; active += 1) {
        for (let available = 0; available <= 400; available += 1) {
          for (let cap = 0; cap <= 400; cap += 7) {
            const { widths, visible } = tabStripLayout(available, n, active, cap)
            const ref = tabStripLayout(Math.min(available, cap), n, active).widths
            const k = visible.indexOf(false)
            const shown = widths.filter((_, i) => visible[i]).reduce((s, w) => s + w, 0)
            if (widths.some((w, i) => w !== ref[i])) bad.push(`폭 ${available}/${cap}/${n}/${active}`)
            if (k >= 0 && visible.slice(k).some(v => v)) bad.push(`꼬리 ${available}/${cap}/${n}/${active}`)
            if (shown > available) bad.push(`넘침 ${available}/${cap}/${n}/${active}`)
          }
        }
      }
    }
    expect(bad).toEqual([])
  })
})
