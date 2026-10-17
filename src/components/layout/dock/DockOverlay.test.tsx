// DockOverlay 단위테스트 — TRD S21-slot-docking §4-3 가이드 그리기(입력 = DockGuide 하나 · 판정 없음).

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ label: 'main' }) }))

import type { DockRegions } from '../../../../src-tauri/bindings/DockRegions'
import type { DockGuide } from './dockDrag'
import { DockOverlayView } from './DockOverlay'

function regions(): DockRegions {
  return {
    drag_id: 'd1',
    rev: 1,
    bar_rev: 1,
    blocked: false,
    view_id: 'v-main',
    canvas_origin: { x: 10, y: 28 },
    canvas: { w: 400, h: 300 },
    outer: [
      { side: 'Left', strip: { x0: 0, y0: 0, x1: 32, y1: 300 }, landing: { x0: 0, y0: 0, x1: 133, y1: 300 } },
      { side: 'Top', strip: { x0: 0, y0: 0, x1: 400, y1: 32 }, landing: { x0: 0, y0: 0, x1: 400, y1: 100 } },
    ],
    slots: [
      {
        slot_id: 's1',
        frame: { x0: 0, y0: 0, x1: 200, y1: 300 },
        bands: [
          { side: 'Right', anchor: 's1', rect: { x0: 140, y0: 0, x1: 200, y1: 300 } },
          { side: 'Bottom', anchor: 's1', rect: { x0: 0, y0: 225, x1: 200, y1: 300 } },
        ],
      },
      { slot_id: 's2', frame: { x0: 200, y0: 0, x1: 400, y1: 300 }, bands: [] },
    ],
    tab_bar: {
      rect: { x0: 4, y0: 0, x1: 404, y1: 28 },
      heads: [
        { view_id: 'v-main', rect: { x0: 4, y0: 0, x1: 104, y1: 28 } },
        { view_id: 'v-other', rect: { x0: 104, y0: 0, x1: 204, y1: 28 } },
      ],
      insert_xs: [0, 100, 200],
    },
  }
}

function guide(over: Partial<DockGuide> = {}): DockGuide {
  return { regions: regions(), shown: { type: 'none' }, canvas: true, cursorSlot: null, hover: null, ...over }
}

const box = (el: HTMLElement) => ({
  left: el.style.left,
  top: el.style.top,
  width: el.style.width,
  height: el.style.height,
})

afterEach(() => cleanup())

describe('DockOverlayView', () => {
  it('가이드가 없으면 아무것도 그리지 않는다', () => {
    const { container } = render(<DockOverlayView guide={null} />)
    expect(container.innerHTML).toBe('')
  })

  it('바깥 변 고름 — 바깥 띠 전부를 캔버스 원점 기준으로 빗금 · 결과는 landing', () => {
    render(<DockOverlayView guide={guide({ shown: { type: 'seam', anchor: null, side: 'Left' } })} />)
    const outer = screen.getAllByTestId('dock-outer-band')
    expect(outer.map(el => el.dataset.side)).toEqual(['Left', 'Top'])
    expect(box(outer[0])).toEqual({ left: '10px', top: '28px', width: '32px', height: '300px' })
    expect(box(screen.getByTestId('dock-landing'))).toEqual({
      left: '10px',
      top: '28px',
      width: '133px',
      height: '300px',
    })
    expect(screen.getByTestId('dock-overlay').style.pointerEvents).toBe('none')
  })

  it('칸 띠 고름 — 커서 밑 칸의 띠를 빗금 · 결과는 그 띠 사각형', () => {
    render(
      <DockOverlayView
        guide={guide({ cursorSlot: 's1', shown: { type: 'seam', anchor: 's1', side: 'Right' } })}
      />,
    )
    expect(screen.getAllByTestId('dock-slot-band').map(el => el.dataset.side)).toEqual(['Right', 'Bottom'])
    expect(box(screen.getByTestId('dock-landing'))).toEqual({
      left: '150px',
      top: '28px',
      width: '60px',
      height: '300px',
    })
  })

  it('칸 가운데(가이드 없음)여도 커서 밑 칸의 띠 · 바깥 띠는 그린다 — 결과 강조는 없다', () => {
    render(<DockOverlayView guide={guide({ cursorSlot: 's1' })} />)
    expect(screen.getAllByTestId('dock-slot-band')).toHaveLength(2)
    expect(screen.getAllByTestId('dock-outer-band')).toHaveLength(2)
    expect(screen.queryByTestId('dock-landing')).toBeNull()
  })

  it('캔버스 몫을 안 그리는 창(보이던 탭 ≠ 영역)에서도 탭바 끼울 자리는 그린다', () => {
    render(
      <DockOverlayView
        guide={guide({ canvas: false, cursorSlot: null, shown: { type: 'tab_bar', index: 1, over: null } })}
      />,
    )
    expect(screen.queryByTestId('dock-outer-band')).toBeNull()
    expect(screen.queryByTestId('dock-slot-band')).toBeNull()
    expect(box(screen.getByTestId('dock-tab-insert'))).toEqual({
      left: '103px',
      top: '0px',
      width: '2px',
      height: '28px',
    })
  })

  it('전환 대기 중인 머리에 진행 표시를 한다 — 대기마다 새로 시작한다', () => {
    const { rerender } = render(
      <DockOverlayView
        guide={guide({ shown: { type: 'tab_bar', index: 2, over: 'v-other' }, hover: { viewId: 'v-other', seq: 1 } })}
      />,
    )
    const first = screen.getByTestId('dock-hover-progress')
    expect(first.dataset.viewId).toBe('v-other')
    expect(box(first)).toEqual({ left: '104px', top: '26px', width: '100px', height: '2px' })

    rerender(
      <DockOverlayView
        guide={guide({ shown: { type: 'tab_bar', index: 2, over: 'v-other' }, hover: { viewId: 'v-other', seq: 2 } })}
      />,
    )
    expect(screen.getByTestId('dock-hover-progress')).not.toBe(first)
  })

  it('그릴 것이 하나도 없으면 겹 자체를 두지 않는다', () => {
    const r = regions()
    render(<DockOverlayView guide={guide({ canvas: false, regions: { ...r, outer: [] } })} />)
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })
})
