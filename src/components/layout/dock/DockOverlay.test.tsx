// DockOverlay 단위테스트 — TRD S21-slot-docking §2-5 안내 창 그리기(입력 = 셸이 민 `OverlayView` 하나 · 판정 없음).

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import type { OverlayView } from '../../../../src-tauri/bindings/OverlayView'
import { DockOverlayView } from './DockOverlay'

function view(over: Partial<OverlayView> = {}): OverlayView {
  return {
    outer: [
      { x0: 0, y0: 28, x1: 32, y1: 328 },
      { x0: 0, y0: 28, x1: 400, y1: 60 },
    ],
    bands: [{ x0: 150, y0: 28, x1: 210, y1: 328 }],
    preview: { x0: 110, y0: 28, x1: 210, y1: 328 },
    ...over,
  }
}

const box = (el: HTMLElement) => ({
  left: el.style.left,
  top: el.style.top,
  width: el.style.width,
  height: el.style.height,
})

afterEach(() => cleanup())

describe('DockOverlayView', () => {
  it('그림이 없으면(null) 아무것도 그리지 않는다', () => {
    const { container } = render(<DockOverlayView guide={null} />)
    expect(container.innerHTML).toBe('')
  })

  it('바깥 띠 · 칸 띠 · 붙을 사각형을 민 사각형 그대로(안내 창 뷰포트 px) 그린다 · 겹은 클릭을 받지 않는다', () => {
    render(<DockOverlayView guide={view()} />)
    const outer = screen.getAllByTestId('dock-outer-band')
    expect(outer.map(box)).toEqual([
      { left: '0px', top: '28px', width: '32px', height: '300px' },
      { left: '0px', top: '28px', width: '400px', height: '32px' },
    ])
    expect(box(screen.getByTestId('dock-slot-band'))).toEqual({ left: '150px', top: '28px', width: '60px', height: '300px' })
    expect(box(screen.getByTestId('dock-preview'))).toEqual({ left: '110px', top: '28px', width: '100px', height: '300px' })
    const layer = screen.getByTestId('dock-overlay')
    expect(layer.style.pointerEvents).toBe('none')
    expect(layer.style.position).toBe('fixed')
  })

  it('고른 띠가 없으면(preview null) 띠만 그리고 강조는 없다', () => {
    render(<DockOverlayView guide={view({ preview: null })} />)
    expect(screen.getAllByTestId('dock-outer-band')).toHaveLength(2)
    expect(screen.getAllByTestId('dock-slot-band')).toHaveLength(1)
    expect(screen.queryByTestId('dock-preview')).toBeNull()
  })

  it('그릴 것이 하나도 없으면 겹 자체를 두지 않는다', () => {
    render(<DockOverlayView guide={{ outer: [], bands: [], preview: null }} />)
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })
})
