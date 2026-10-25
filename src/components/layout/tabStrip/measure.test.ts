// measure 단위테스트 — 잰 값(줄 사각형 · 첫 탭 왼쪽 · 쓸 수 있는 폭 = 줄 폭 − `+` 폭)과 이상적 경계 · 탭바 구역의 바탕.

import { describe, expect, it } from 'vitest'

import { tabStripLayout } from './layout'
import { measureStrip, stripBounds, stripZoneArea, tabsEnd } from './measure'
import { idealBounds } from './reorderGeometry'

function box(left: number, top: number, width: number, height: number): Element {
  const el = document.createElement('div')
  el.getBoundingClientRect = () =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect
  return el
}

describe('measureStrip', () => {
  it('줄 사각형 · 첫 탭 왼쪽 = 줄 왼쪽 · 쓸 수 있는 폭 = 줄 폭 − + 폭', () => {
    const m = measureStrip({ row: box(100, 40, 700, 28), add: box(760, 40, 40, 28) })
    expect(m).toEqual({
      row: { x0: 100, y0: 40, x1: 800, y1: 68 },
      left: 100,
      available: 660,
      addWidth: 40,
    })
  })

  it('+ 단추가 없으면 줄 폭 그대로 · + 폭 0', () => {
    const m = measureStrip({ row: box(0, 0, 500, 28), add: null })
    expect(m.available).toBe(500)
    expect(m.addWidth).toBe(0)
  })
})

// 사용자 결정 2026-10-26(T-66) — 탭이 있는 곳(첫 탭 ~ `+`)만 탭바 구역.
describe('stripZoneArea — 탭바 구역의 바탕 = 첫 탭 왼쪽 ~ + 오른 끝 × 제목 줄(없으면 탭바 줄)', () => {
  // 탭바 줄 x 100..800 · y 40..68 · + 상자 폭 26.
  const m = measureStrip({ row: box(100, 40, 700, 28), add: box(0, 40, 26, 28) })
  const three = idealBounds({ widths: [160, 160, 160], visible: [true, true, true] })
  const TITLE = { top: 34, bottom: 68 }

  it('+ 자리 = 보이는 마지막 머리 오른 끝 · 보이는 머리가 없으면 0', () => {
    expect(tabsEnd(three)).toBe(480)
    expect(tabsEnd(idealBounds({ widths: [52, 32, 32], visible: [true, true, false] }))).toBe(84)
    expect(tabsEnd(idealBounds({ widths: [], visible: [] }))).toBe(0)
  })

  it('가로 = 첫 탭 ~ + 오른 끝 — 그 오른쪽 빈 곳 · 끌 자리 · 번호 표식 · 창 단추는 구역 밖 · 세로 = 탭바 줄', () => {
    expect(stripZoneArea(m, three, null)).toEqual({ x0: 100, y0: 40, x1: 606, y1: 68 })
  })

  it('제목 줄 안이면 세로만 제목 줄(위 여백 포함) — 가로는 같다', () => {
    expect(stripZoneArea(m, three, TITLE)).toEqual({ x0: 100, y0: 34, x1: 606, y1: 68 })
  })

  it('숨긴 머리는 세지 않는다 — + 가 서는 보이는 끝까지', () => {
    const narrow = idealBounds({ widths: [52, 32, 32, 32], visible: [true, true, false, false] })
    expect(stripZoneArea(m, narrow, null).x1).toBe(100 + 84 + 26)
  })

  it('탭이 없으면 + 만 · + 도 없으면 폭 0(구역 없음)', () => {
    const none = idealBounds({ widths: [], visible: [] })
    expect(stripZoneArea(m, none, null)).toEqual({ x0: 100, y0: 40, x1: 126, y1: 68 })
    const bare = measureStrip({ row: box(100, 40, 700, 28), add: null })
    expect(stripZoneArea(bare, none, null)).toEqual({ x0: 100, y0: 40, x1: 100, y1: 68 })
  })

  it('+ 가 없으면 보이는 마지막 머리 오른 끝까지', () => {
    const bare = measureStrip({ row: box(100, 40, 700, 28), add: null })
    expect(stripZoneArea(bare, three, null).x1).toBe(580)
  })

  it('탭바 줄 오른 끝에서 자른다(재지 못해 머리가 줄보다 넓을 때 — 탭바가 가로로 잘라 보이지 않는 곳) · 줄 폭 0 = 폭 0', () => {
    const short = measureStrip({ row: box(100, 40, 300, 28), add: box(0, 40, 26, 28) })
    expect(stripZoneArea(short, three, null).x1).toBe(400)
    const zero = measureStrip({ row: box(100, 40, 0, 28), add: box(0, 40, 26, 28) })
    expect(stripZoneArea(zero, idealBounds({ widths: [], visible: [] }), null).x1).toBe(100)
  })
})

describe('stripBounds', () => {
  it('이상적 경계 = layout.ts 폭 · 숨김의 누적(닫기 모드 고정 폭 · 끄는 탭 포함)', () => {
    expect(stripBounds({ available: 300, count: 5, active: 1, dragged: 4, widthCap: 250 })).toEqual(
      idealBounds(tabStripLayout(300, 5, 1, 250), 4),
    )
  })
})
