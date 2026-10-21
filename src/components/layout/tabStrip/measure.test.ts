// measure 단위테스트 — 잰 값(줄 사각형 · 첫 탭 왼쪽 · 쓸 수 있는 폭 = 줄 폭 − `+` 폭)과 이상적 경계.

import { describe, expect, it } from 'vitest'

import { tabStripLayout } from './layout'
import { measureStrip, stripBounds } from './measure'
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
    })
  })

  it('+ 단추가 없으면 줄 폭 그대로', () => {
    const m = measureStrip({ row: box(0, 0, 500, 28), add: null })
    expect(m.available).toBe(500)
  })
})

describe('stripBounds', () => {
  it('이상적 경계 = layout.ts 폭 · 숨김의 누적(닫기 모드 고정 폭 · 끄는 탭 포함)', () => {
    expect(stripBounds({ available: 300, count: 5, active: 1, dragged: 4, widthCap: 250 })).toEqual(
      idealBounds(tabStripLayout(300, 5, 1, 250), 4),
    )
  })
})
