// NoticeOverlay — the layer that covers the layout with notices (ADR-0276 · ADR-0277).
//
// jsdom cannot measure size — assert 「does not push · covers only as much as its rows」 by the computed position and structure.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import NoticeOverlay from './NoticeOverlay'

afterEach(cleanup)

function Nothing() {
  return null
}

const overlay = () => screen.getByTestId('notice-overlay')
const stack = () => screen.getByTestId('notice-stack')

describe('NoticeOverlay', () => {
  it('the computed position is absolute and attaches to the top of the reference area · left and right ends, and z is 40', () => {
    render(
      <NoticeOverlay>
        <div role="status">a</div>
      </NoticeOverlay>,
    )
    const style = getComputedStyle(overlay())
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(style.zIndex).toBe('40')
    expect(overlay().contains(screen.getByRole('status'))).toBe(true)
  })

  it('it covers only as much as the rows — height · bottom end are not set', () => {
    render(
      <NoticeOverlay>
        <div role="status">a</div>
      </NoticeOverlay>,
    )
    expect(overlay().style.height).toBe('')
    expect(overlay().style.bottom).toBe('')
    expect(overlay().className).not.toMatch(/\binset-0\b|\bbottom-0\b|\bh-full\b|\bh-screen\b/)
  })

  it('if no notice is drawn the layer has no content', () => {
    render(
      <NoticeOverlay>
        <Nothing />
        <Nothing />
      </NoticeOverlay>,
    )
    expect(stack().childElementCount).toBe(0)
    expect(stack().textContent).toBe('')
  })

  it('if it overflows it scrolls — the scroll node has a height cap and vertical scroll', () => {
    render(
      <NoticeOverlay>
        <div role="status">a</div>
      </NoticeOverlay>,
    )
    const viewport = overlay().querySelector<HTMLElement>('[data-radix-scroll-area-viewport]')
    expect(viewport).not.toBeNull()
    expect(viewport?.className).toContain('max-h-[33vh]')
    expect(viewport?.style.overflowY).toBe('scroll')
    expect(viewport?.contains(stack())).toBe(true)
  })

  it('with several it stacks in the given order and separates rows with a 1px gap of the app background', () => {
    render(
      <NoticeOverlay>
        <div role="alert">first</div>
        <Nothing />
        <div role="status">second</div>
      </NoticeOverlay>,
    )
    expect([...stack().children]).toEqual([screen.getByRole('alert'), screen.getByRole('status')])
    expect(stack().className).toMatch(/\bflex-col\b/)
    expect(stack().className).toMatch(/\bgap-px\b/)
    expect(stack().className).toMatch(/\bbg-background\b/)
  })
})
