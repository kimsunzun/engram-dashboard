// DockOverlay unit test — TRD S21-slot-docking §4-4 guide drawing (input = one `OverlayView` · no decision) · motion (appear · disappear
//   150 ms · move 70 ms).

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

describe('DockOverlayView — drawing', () => {
  it('if there is no drawing (null) it draws nothing', () => {
    const { container } = render(<DockOverlayView guide={null} />)
    expect(container.innerHTML).toBe('')
  })

  it('area zone lines = a center rectangle + four diagonals from the area corners to the center corners · thin faint lines and not filled', () => {
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
    ['600 × 200 · origin (100, 50)', { slot: { x0: 100, y0: 50, x1: 700, y1: 250 }, centre: { x0: 160, y0: 70, x1: 640, y1: 230 } }],
    ['200 × 600 · origin (30, 40)', { slot: { x0: 30, y0: 40, x1: 230, y1: 640 }, centre: { x0: 50, y0: 100, x1: 210, y1: 580 } }],
    [
      'fractional origin (20.5, 48.5)',
      { slot: { x0: 20.5, y0: 48.5, x1: 1180.5, y1: 808.5 }, centre: { x0: 140.5, y0: 128.5, x1: 1060.5, y1: 728.5 } },
    ],
  ])('both ends of a diagonal = the area zone frame corner · the same-side corner of the drawn center rectangle (%s)', (_, zones) => {
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

  it('a center of width 0 in a narrow area is also drawn as a segment', () => {
    const narrow = { slot: { x0: 10, y0: 0, x1: 30, y1: 400 }, centre: { x0: 20, y0: 33, x1: 20, y1: 367 } }
    render(<DockOverlayView guide={view({ slot: narrow, preview: null })} />)
    expect(screen.getByTestId('dock-slot-centre').getAttribute('d')).toBe('M20 33H20V367H20Z')
  })

  it('choosing an area band draws the rectangle to attach as a faint fill + 1px border together with the lines (no hatching) · the layer is an absolute layer covering the canvas and takes no clicks', () => {
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

  it('for an outer band (outer is non-empty) it draws the layout frame (layer border) and the rectangle to attach without area lines · the outer band region is not painted', () => {
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

  it('the area band region (bands) is not painted — the only drawings inside the layer are the zone lines · frame · rectangle to attach', () => {
    render(<DockOverlayView guide={view({ slot: null, preview: null, bands: [{ x0: 150, y0: 28, x1: 210, y1: 328 }] })} />)
    const layer = screen.getByTestId('dock-overlay')
    const ids = [...layer.querySelectorAll(':scope > [data-testid]')].map(el => el.getAttribute('data-testid'))
    expect(ids).toEqual(['dock-slot-lines', 'dock-frame', 'dock-preview'])
    expect(['dock-slot-lines', 'dock-frame', 'dock-preview'].map(opacity)).toEqual(['0', '0', '0'])
  })

  it('a body without the slot area is also accepted — it draws the rectangle to attach without lines', () => {
    const old = { bands: [{ x0: 10, y0: 20, x1: 30, y1: 40 }], outer: [], preview: { x0: 10, y0: 20, x1: 50, y1: 40 } }
    render(<DockOverlayView guide={old} />)
    expect(opacity('dock-slot-lines')).toBe('0')
    expect(box(screen.getByTestId('dock-preview'))).toEqual({ left: '10px', top: '20px', width: '40px', height: '20px' })
    expect(opacity('dock-preview')).toBe('1')
  })
})

describe('DockOverlayView — motion', () => {
  it('when it appears it shows with a 150 ms fade · on null it fades and then removes the layer after 150 ms', () => {
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

  it('if a drawing comes again while fading it does not remove the layer and shows again', () => {
    vi.useFakeTimers()
    const { rerender } = render(<DockOverlayView guide={view()} />)
    rerender(<DockOverlayView guide={null} />)
    act(() => vi.advanceTimersByTime(100))
    rerender(<DockOverlayView guide={view({ preview: null })} />)
    expect(screen.getByTestId('dock-overlay').style.opacity).toBe('1')
    act(() => vi.advanceTimersByTime(500))
    expect(screen.getByTestId('dock-overlay').style.opacity).toBe('1')
  })

  it('the rectangle to attach — when it disappears it fades at the last position · when it appears again it does not slide but is in place · moving between bands is 70 ms', () => {
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

  it('even the first appearing rectangle to attach appears with the opacity of a pre-laid element (0 → 1 in the layer)', () => {
    const { rerender } = render(<DockOverlayView guide={view({ preview: null })} />)
    expect(opacity('dock-preview')).toBe('0')
    rerender(<DockOverlayView guide={view()} />)
    expect(opacity('dock-preview')).toBe('1')
    expect(screen.getByTestId('dock-preview').style.transition).toBe(FADE)
  })

  it('zone lines — when they disappear they fade leaving the last lines', () => {
    const { rerender } = render(<DockOverlayView guide={view({ preview: null })} />)
    rerender(<DockOverlayView guide={view({ slot: null, preview: null })} />)
    expect(opacity('dock-slot-lines')).toBe('0')
    expect(screen.getByTestId('dock-slot-centre').getAttribute('d')).toBe('M60 108H540V748H60Z')
    expect(screen.getByTestId('dock-slot-lines').style.transition).toBe(FADE)
  })

  it('with reduced motion (prefers-reduced-motion: reduce) transition is 0 ms · on null it is removed at once', () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q === '(prefers-reduced-motion: reduce)' }))
    const { rerender } = render(<DockOverlayView guide={view()} />)
    expect(screen.getByTestId('dock-overlay').style.transition).toBe('opacity 0ms ease-out')
    rerender(<DockOverlayView guide={view({ preview: { x0: 0, y0: 0, x1: 10, y1: 10 } })} />)
    expect(screen.getByTestId('dock-preview').style.transition).toContain('left 0ms')
    rerender(<DockOverlayView guide={null} />)
    expect(screen.queryByTestId('dock-overlay')).toBeNull()
  })
})
