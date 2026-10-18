// measure 단위테스트 — 잰 값(줄 사각형 · 첫 탭 왼쪽 · 쓸 수 있는 폭 = 줄 폭 − `+` 폭 · 캔버스)과 이상적 경계 · 끌기 영역이
//   크롬 보고의 `tab_bar` 와 같은 사각형인지.

import { describe, expect, it } from 'vitest'

import { dragAreaRect, frameChromeOf } from '../dock/frameReport'
import { tabStripLayout } from './layout'
import { dragAreaOf, frameMeasureOf, measureStrip, stripBounds } from './measure'
import { idealBounds } from './reorderGeometry'

function box(left: number, top: number, width: number, height: number): Element {
  const el = document.createElement('div')
  el.getBoundingClientRect = () =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect
  return el
}

describe('measureStrip', () => {
  it('줄 사각형 · 첫 탭 왼쪽 = 줄 왼쪽 · 쓸 수 있는 폭 = 줄 폭 − + 폭 · 캔버스 왼쪽 위', () => {
    const m = measureStrip({ row: box(100, 40, 700, 28), add: box(760, 40, 40, 28), canvas: box(100, 68, 700, 500) })
    expect(m).toEqual({
      row: { x0: 100, y0: 40, x1: 800, y1: 68 },
      left: 100,
      available: 660,
      canvas: { left: 100, top: 68 },
    })
  })

  it('+ 단추가 없으면 줄 폭 그대로 · 캔버스를 주지 않으면 null', () => {
    const m = measureStrip({ row: box(0, 0, 500, 28), add: null })
    expect(m.available).toBe(500)
    expect(m.canvas).toBeNull()
  })
})

describe('stripBounds · dragAreaOf · frameMeasureOf', () => {
  const m = measureStrip({ row: box(100, 40, 700, 28), add: box(760, 40, 40, 28), canvas: box(100, 68, 700, 500) })

  it('이상적 경계 = layout.ts 폭 · 숨김의 누적(닫기 모드 고정 폭 · 끄는 탭 포함)', () => {
    expect(stripBounds({ available: 300, count: 5, active: 1, dragged: 4, widthCap: 250 })).toEqual(
      idealBounds(tabStripLayout(300, 5, 1, 250), 4),
    )
  })

  it('끌기 영역 사각형 = 크롬 보고의 tab_bar — 첫 탭 왼쪽부터 쓸 수 있는 폭 · 탭바 줄 위아래', () => {
    const bounds = stripBounds({ available: m.available, count: 4, active: 0 })
    const area = dragAreaRect(dragAreaOf(m, bounds))
    expect(area).toEqual({ x0: 100, y0: 40, x1: 760, y1: 68 })
    const frame = frameMeasureOf(m, bounds)
    expect(frame).not.toBeNull()
    expect(frameChromeOf(frame!).tab_bar).toEqual(area)
    expect(frameChromeOf(frame!).canvas_origin).toEqual({ x: 100, y: 68 })
  })

  it('캔버스를 재지 않았으면 크롬 보고 재료가 없다', () => {
    const noCanvas = measureStrip({ row: box(0, 0, 500, 28), add: null })
    expect(frameMeasureOf(noCanvas, stripBounds({ available: 500, count: 1, active: 0 }))).toBeNull()
  })
})
