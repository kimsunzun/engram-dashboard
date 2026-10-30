// DockOverlay 단위테스트 — TRD S21-slot-docking §4-4 안내 그리기(입력 = `OverlayView` 하나 · 판정 없음) · 움직임(나타남 · 사라짐
//   150 ms · 자리 옮김 70 ms).

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { DockOverlayView } from './DockOverlay'
import type { OverlayView, SlotZones } from './overlayTypes'

const SLOT: SlotZones = { slot: { x0: 0, y0: 28, x1: 600, y1: 828 }, centre: { x0: 60, y0: 108, x1: 540, y1: 748 } }
const FADE = 'opacity 150ms ease-out'
const MOVE = 'left 70ms ease-out, top 70ms ease-out, width 70ms ease-out, height 70ms ease-out'

function view(over: Partial<OverlayView> = {}): OverlayView {
  return { bands: [], outer: [], slot: SLOT, preview: { x0: 600, y0: 28, x1: 1000, y1: 828 }, ...over }
}

const box = (el: HTMLElement) => ({
  left: el.style.left,
  top: el.style.top,
  width: el.style.width,
  height: el.style.height,
})

const line = (el: Element) => ['x1', 'y1', 'x2', 'y2'].map(a => Number(el.getAttribute(a)))
const opacity = (id: string) => screen.getByTestId(id).style.opacity

afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('DockOverlayView — 그림', () => {
  it('그림이 없으면(null) 아무것도 그리지 않는다', () => {
    const { container } = render(<DockOverlayView guide={null} />)
    expect(container.innerHTML).toBe('')
  })

  it('칸 구역 선 = 가운데 사각형 + 칸 모서리에서 가운데 모서리로 가는 대각선 넷 · 얇고 옅은 선이고 칠하지 않는다', () => {
    render(<DockOverlayView guide={view({ preview: null })} />)
    expect(screen.getByTestId('dock-slot-centre').getAttribute('d')).toBe('M60 108H540V748H60Z')
    expect(screen.getAllByTestId('dock-slot-diagonal').map(line)).toEqual([
      [0, 28, 60, 108],
      [600, 28, 540, 108],
      [600, 828, 540, 748],
      [0, 828, 60, 748],
    ])
    const lines = screen.getByTestId('dock-slot-lines')
    expect(lines.style.opacity).toBe('1')
    expect(lines.style.strokeWidth).toBe('1')
    expect(Number(lines.style.strokeOpacity)).toBeLessThan(0.6)
    expect(lines.style.fill).toBe('none')
    expect(opacity('dock-preview')).toBe('0')
    expect(opacity('dock-frame')).toBe('0')
  })

  it.each([
    ['600 × 200 · 원점 (100, 50)', { slot: { x0: 100, y0: 50, x1: 700, y1: 250 }, centre: { x0: 160, y0: 70, x1: 640, y1: 230 } }],
    ['200 × 600 · 원점 (30, 40)', { slot: { x0: 30, y0: 40, x1: 230, y1: 640 }, centre: { x0: 50, y0: 100, x1: 210, y1: 580 } }],
    [
      '소수 원점 (20.5, 48.5)',
      { slot: { x0: 20.5, y0: 48.5, x1: 1180.5, y1: 808.5 }, centre: { x0: 140.5, y0: 128.5, x1: 1060.5, y1: 728.5 } },
    ],
  ])('대각선의 두 끝 = 칸 구역 틀 모서리 · 그린 가운데 사각형의 같은 쪽 모서리(%s)', (_, zones) => {
    render(<DockOverlayView guide={view({ slot: zones, preview: null })} />)
    const d = screen.getByTestId('dock-slot-centre').getAttribute('d') ?? ''
    const m = /^M(\S+) (\S+)H(\S+)V(\S+)H(\S+)Z$/.exec(d)
    expect(m).not.toBeNull()
    const [x0, y0, x1, y1, xBack] = m!.slice(1).map(Number)
    expect(xBack).toBe(x0)
    const drawn = [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ]
    const s = zones.slot
    const outer = [
      [s.x0, s.y0],
      [s.x1, s.y0],
      [s.x1, s.y1],
      [s.x0, s.y1],
    ]
    expect(screen.getAllByTestId('dock-slot-diagonal').map(line)).toEqual(outer.map((o, i) => [...o, ...drawn[i]]))
  })

  it('좁은 칸의 폭 0 가운데도 선분으로 그린다', () => {
    const narrow = { slot: { x0: 10, y0: 0, x1: 30, y1: 400 }, centre: { x0: 20, y0: 33, x1: 20, y1: 367 } }
    render(<DockOverlayView guide={view({ slot: narrow, preview: null })} />)
    expect(screen.getByTestId('dock-slot-centre').getAttribute('d')).toBe('M20 33H20V367H20Z')
  })

  it('칸 띠를 고르면 선과 함께 붙을 사각형을 옅은 채움 + 1px 테두리로 그린다(빗금 없음) · 층은 캔버스를 덮는 절대 층이고 클릭을 받지 않는다', () => {
    render(<DockOverlayView guide={view()} />)
    expect(opacity('dock-slot-lines')).toBe('1')
    const preview = screen.getByTestId('dock-preview')
    expect(preview.style.opacity).toBe('1')
    expect(box(preview)).toEqual({ left: '600px', top: '28px', width: '400px', height: '800px' })
    expect(preview.style.border).toBe('1px solid var(--accent)')
    expect(preview.style.background).toContain('18%')
    expect(preview.style.background).not.toContain('gradient')
    expect(opacity('dock-frame')).toBe('0')
    const layer = screen.getByTestId('dock-overlay')
    expect(layer.style.pointerEvents).toBe('none')
    expect(layer.style.position).toBe('absolute')
    expect(layer.style.position).not.toBe('fixed')
  })

  it('바깥 띠면(outer 가 비지 않음) 칸 선 없이 레이아웃 틀(층 테두리)과 붙을 사각형을 그린다 · 바깥 띠 영역은 칠하지 않는다', () => {
    render(
      <DockOverlayView
        guide={view({ slot: null, outer: [{ x0: 0, y0: 0, x1: 20, y1: 800 }], preview: { x0: 0, y0: 0, x1: 400, y1: 800 } })}
      />,
    )
    expect(opacity('dock-slot-lines')).toBe('0')
    expect(screen.queryByTestId('dock-slot-centre')).toBeNull()
    const frame = screen.getByTestId('dock-frame')
    expect(frame.style.opacity).toBe('1')
    expect(frame.style.position).toBe('absolute')
    expect(frame.style.border).toBe('2px solid var(--accent)')
    expect(frame.style.background).toBe('')
    expect(box(screen.getByTestId('dock-preview'))).toEqual({ left: '0px', top: '0px', width: '400px', height: '800px' })
  })

  it('칸 띠 영역(bands)은 칠하지 않는다 — 층 안 그림은 구역 선 · 틀 · 붙을 사각형 셋뿐이다', () => {
    render(<DockOverlayView guide={view({ slot: null, preview: null, bands: [{ x0: 150, y0: 28, x1: 210, y1: 328 }] })} />)
    const layer = screen.getByTestId('dock-overlay')
    const ids = [...layer.querySelectorAll(':scope > [data-testid]')].map(el => el.getAttribute('data-testid'))
    expect(ids).toEqual(['dock-slot-lines', 'dock-frame', 'dock-preview'])
    expect(['dock-slot-lines', 'dock-frame', 'dock-preview'].map(opacity)).toEqual(['0', '0', '0'])
  })

  it('slot 칸이 없는 몸도 받는다 — 선 없이 붙을 사각형을 그린다', () => {
    const old = { bands: [{ x0: 10, y0: 20, x1: 30, y1: 40 }], outer: [], preview: { x0: 10, y0: 20, x1: 50, y1: 40 } }
    render(<DockOverlayView guide={old} />)
    expect(opacity('dock-slot-lines')).toBe('0')
    expect(box(screen.getByTestId('dock-preview'))).toEqual({ left: '10px', top: '20px', width: '40px', height: '20px' })
    expect(opacity('dock-preview')).toBe('1')
  })
})

describe('DockOverlayView — 움직임', () => {
  it('나타나면 150 ms 페이드로 보이고 · null 이면 흐려진 뒤 150 ms 에 층을 걷는다', () => {
    vi.useFakeTimers()
    const { rerender } = render(<DockOverlayView guide={view()} />)
    const layer = screen.getByTestId('dock-overlay')
    expect([layer.style.opacity, layer.style.transition]).toEqual(['1', FADE])

    rerender(<DockOverlayView guide={null} />)
    expect(screen.getByTestId('dock-overlay').style.opacity).toBe('0')
    expect(box(screen.getByTestId('dock-preview'))).toEqual({ left: '600px', top: '28px', width: '400px', height: '800px' })
    act(() => vi.advanceTimersByTime(149))
    expect(screen.queryByTestId('dock-overlay')).not.toBeNull()
    act(() => vi.advanceTimersByTime(1))
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })

  it('흐려지는 동안 다시 그림이 오면 층을 걷지 않고 다시 보인다', () => {
    vi.useFakeTimers()
    const { rerender } = render(<DockOverlayView guide={view()} />)
    rerender(<DockOverlayView guide={null} />)
    act(() => vi.advanceTimersByTime(100))
    rerender(<DockOverlayView guide={view({ preview: null })} />)
    expect(screen.getByTestId('dock-overlay').style.opacity).toBe('1')
    act(() => vi.advanceTimersByTime(500))
    expect(screen.getByTestId('dock-overlay').style.opacity).toBe('1')
  })

  it('붙을 사각형 — 사라지면 마지막 자리에서 흐려지고 · 다시 나타나면 미끄러지지 않고 제자리에서 · 띠 사이 옮김은 70 ms', () => {
    const { rerender } = render(<DockOverlayView guide={view({ preview: { x0: 0, y0: 0, x1: 100, y1: 100 } })} />)
    const preview = () => screen.getByTestId('dock-preview')
    expect(preview().style.transition).toBe(FADE)

    rerender(<DockOverlayView guide={view({ preview: null })} />)
    expect(preview().style.opacity).toBe('0')
    expect(box(preview())).toEqual({ left: '0px', top: '0px', width: '100px', height: '100px' })
    expect(preview().style.transition).toBe(FADE)

    rerender(<DockOverlayView guide={view({ preview: { x0: 200, y0: 0, x1: 300, y1: 100 } })} />)
    expect(preview().style.opacity).toBe('1')
    expect(preview().style.left).toBe('200px')
    expect(preview().style.transition).toBe(FADE)

    rerender(<DockOverlayView guide={view({ preview: { x0: 300, y0: 0, x1: 500, y1: 100 } })} />)
    expect(preview().style.left).toBe('300px')
    expect(preview().style.transition).toBe(`${MOVE}, ${FADE}`)
  })

  it('처음 나타나는 붙을 사각형도 미리 깔린 요소의 불투명도로 나타난다(층 안에서 0 → 1)', () => {
    const { rerender } = render(<DockOverlayView guide={view({ preview: null })} />)
    expect(opacity('dock-preview')).toBe('0')
    rerender(<DockOverlayView guide={view()} />)
    expect(opacity('dock-preview')).toBe('1')
    expect(screen.getByTestId('dock-preview').style.transition).toBe(FADE)
  })

  it('구역 선 — 사라지면 마지막 선을 남긴 채 흐려진다', () => {
    const { rerender } = render(<DockOverlayView guide={view({ preview: null })} />)
    rerender(<DockOverlayView guide={view({ slot: null, preview: null })} />)
    expect(opacity('dock-slot-lines')).toBe('0')
    expect(screen.getByTestId('dock-slot-centre').getAttribute('d')).toBe('M60 108H540V748H60Z')
    expect(screen.getByTestId('dock-slot-lines').style.transition).toBe(FADE)
  })

  it('움직임 줄이기(prefers-reduced-motion: reduce)면 전환 0 ms · null 이면 곧바로 걷는다', () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q === '(prefers-reduced-motion: reduce)' }))
    const { rerender } = render(<DockOverlayView guide={view()} />)
    expect(screen.getByTestId('dock-overlay').style.transition).toBe('opacity 0ms ease-out')
    rerender(<DockOverlayView guide={view({ preview: { x0: 0, y0: 0, x1: 10, y1: 10 } })} />)
    expect(screen.getByTestId('dock-preview').style.transition).toContain('left 0ms')
    rerender(<DockOverlayView guide={null} />)
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })
})
