// TRD S21-slot-docking §8-2 `judge`: 셸 골든의 점 전부(탭바 구역 · 고른 띠 · 구역 선 — `bands_golden.json` 의 `points`)를 판정으로
//   다시 낸다 + 골든 밖 표(차례 · 탭바 구역 경계 · 출처 창 = Chrome `insertionIndex` · 다른 창 = `dropIndex` · 끄는 탭 자신의 캔버스
//   · 막힌 띠).

import { describe, expect, it } from 'vitest'

import { canvasZone, windowBands } from './bands'
import { judge, type JudgeStrip, type JudgeWindow } from './judge'
import { bandsGolden, bandsInputOf, goldenCase, type GoldenBandsCase } from './testing/bandsGolden'
import { idealBounds } from '../tabStrip/reorderGeometry'

const NO_HEADS: JudgeStrip = { role: 'other', left: 0, bounds: { tabs: [], visible: [], lastVisible: -1 } }

function windowOf(c: GoldenBandsCase, strip: JudgeStrip = NO_HEADS): JudgeWindow {
  const input = bandsInputOf(c)
  return {
    canvasOrigin: c.canvas_origin,
    tabBar: input.zone === null ? null : { zone: input.zone, strip },
    bands: windowBands(input),
  }
}

describe('골든 점 — 셸 bands.rs 와 한 파일', () => {
  it('점이 천 개를 넘는다(빈 파일 · 잘린 파일이 초록으로 통과하지 않게)', () => {
    expect(bandsGolden.bands.reduce((n, c) => n + c.points.length, 0)).toBeGreaterThan(1000)
  })

  for (const c of bandsGolden.bands) {
    it(`골든 창: ${c.name} — 점 ${c.points.length} 개`, () => {
      expect(c.points.length).toBeGreaterThan(0)
      const w = windowOf(c)
      const got = c.points.map(([x, y]) => {
        // 골든 점은 캔버스 좌표 — 판정은 뷰포트 점을 받는다(4 분의 1 px 격자 + 정수 원점이라 왕복이 정확하다).
        const j = judge(w, { x: x + c.canvas_origin.x, y: y + c.canvas_origin.y })
        return [x, y, j.type === 'strip' ? 'strip' : j.type === 'band' ? j.band : null, j.type === 'strip' ? null : j.zones]
      })
      expect(got).toEqual(c.points)
    })
  }
})

// 「left-right halves」 — 캔버스 1200 × 800 · 원점 (0, 28) · 탭 끌기 영역 (0, 0)–(600, 28) · 칸 1 | 칸 2.
const HALVES = goldenCase('left-right halves')

describe('차례 — 탭바 구역 > 바깥 띠 > 칸 띠 > 그 밖', () => {
  const w = windowOf(HALVES)
  const at = (x: number, y: number) => judge(w, { x, y })
  const bandAt = (x: number, y: number) => {
    const j = at(x, y)
    if (j.type !== 'band') return null
    const b = w.bands!.bands[j.band]
    return `${b.anchor.type === 'outer' ? 'outer' : 'slot'}/${b.side}`
  }

  it('탭바 구역이 바깥 위 띠 · 칸 띠를 덮는다', () => {
    expect(at(300, 40).type).toBe('strip')
    expect(at(300, 42.75).type).toBe('strip')
  })

  it('구역 밖 바깥 위 띠 → 그 아래 칸 위 띠 → 칸 가운데(구역 선만) → 캔버스 밖', () => {
    expect(bandAt(300, 43)).toBe('outer/Top')
    expect(bandAt(900, 30)).toBe('outer/Top')
    expect(bandAt(300, 48)).toBe('slot/Top')
    expect(at(300, 428)).toEqual({ type: 'elsewhere', zones: 0 })
    expect(at(300, 900)).toEqual({ type: 'elsewhere', zones: null })
  })

  it('바깥 띠 위에서는 구역 선을 그리지 않는다', () => {
    const j = at(5, 400)
    expect(j.type === 'band' && j.zones).toBe(null)
  })
})

describe('탭바 구역 경계', () => {
  const w = windowOf(HALVES)
  const isStrip = (x: number, y: number): boolean => judge(w, { x, y }).type === 'strip'

  it('세로 = 바탕 위아래 STRIP_ZONE_PX(반열림) · 가로 = 바탕 그대로(여기 바탕 = 탭 끌기 영역)', () => {
    expect(isStrip(100, -15)).toBe(true)
    expect(isStrip(100, -15.25)).toBe(false)
    expect(isStrip(100, 42.75)).toBe(true)
    expect(isStrip(100, 43)).toBe(false)
    expect(isStrip(0, 10)).toBe(true)
    expect(isStrip(599.75, 10)).toBe(true)
    expect(isStrip(600, 10)).toBe(false)
  })

  it('탭바를 모르면 구역이 없다', () => {
    expect(judge({ ...w, tabBar: null }, { x: 300, y: 10 }).type).toBe('elsewhere')
  })
})

describe('탭바 자리', () => {
  // 탭 셋 · 각 160 · 끌기 영역 왼쪽 4.
  const bounds = idealBounds({ widths: [160, 160, 160], visible: [true, true, true] }, 0)
  const zone = canvasZone({ x0: 4, y0: 0, x1: 604, y1: 28 }, HALVES.canvas_origin)!
  const withStrip = (strip: JudgeStrip): JudgeWindow => ({ ...windowOf(HALVES), tabBar: { zone, strip } })

  it('출처 창 — Chrome insertionIndex(끄는 머리 왼쪽 = draggedLeft)', () => {
    const w = withStrip({ role: 'source', left: 4, bounds, dragged: 0, grabOffset: 80, available: 600 })
    // 커서 탭바 x 400 − 잡은 80 = 왼쪽 320 → 셋째 뒤(끄는 탭을 뺀 목록의 끝).
    expect(judge(w, { x: 404, y: 10 })).toEqual({ type: 'strip', index: 2, left: 320, marker: null })
    expect(judge(w, { x: 84, y: 10 })).toEqual({ type: 'strip', index: 0, left: 0, marker: null })
    // 탭 뒤 빈 곳(480..600)으로 끌어도 마지막 자리(320)에서 멈춘다.
    expect(judge(w, { x: 584, y: 10 })).toEqual({ type: 'strip', index: 2, left: 320, marker: null })
  })

  it('다른 창 — dropIndex · 세로선(가운데와 같은 x = 오른쪽)', () => {
    const w = withStrip({ role: 'other', left: 4, bounds })
    expect(judge(w, { x: 254, y: 10 })).toEqual({ type: 'strip', index: 2, left: null, marker: 320 })
    expect(judge(w, { x: 244, y: 10 })).toEqual({ type: 'strip', index: 1, left: null, marker: 160 })
    expect(judge(w, { x: 4, y: 10 })).toEqual({ type: 'strip', index: 0, left: null, marker: 0 })
  })

  for (const dragged of [-1, bounds.tabs.length]) {
    it(`출처 창인데 끄는 탭 자리 ${dragged} 가 목록 밖(탭바를 떠남) — 던지지 않고 다른 창 길(dropIndex · 세로선)`, () => {
      const w = withStrip({ role: 'source', left: 4, bounds, dragged, grabOffset: 80, available: 600 })
      expect(judge(w, { x: 254, y: 10 })).toEqual({ type: 'strip', index: 2, left: null, marker: 320 })
      expect(judge(w, { x: 4, y: 10 })).toEqual({ type: 'strip', index: 0, left: null, marker: 0 })
    })
  }
})

describe('띠가 없는 곳', () => {
  it('끄는 탭 자신이 보이는 캔버스(SelfTarget)는 띠도 구역 선도 없다 · 탭바는 그대로', () => {
    const w: JudgeWindow = { ...windowOf(HALVES), bands: null }
    expect(judge(w, { x: 5, y: 400 })).toEqual({ type: 'elsewhere', zones: null })
    expect(judge(w, { x: 300, y: 428 })).toEqual({ type: 'elsewhere', zones: null })
    expect(judge(w, { x: 300, y: 10 }).type).toBe('strip')
  })

  it('막힌 띠는 고르지 않는다 — 놓으면 새 창(R8 · AC15)', () => {
    const w = windowOf(goldenCase('a dragged tab that would leave a zero-area slot on the sides'))
    expect(judge(w, { x: 1150, y: 428 }).type).toBe('elsewhere')
  })
})
