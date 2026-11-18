// 「jump to bottom」 button — U3 (always when not at the bottom · only when there is something to scroll · delayed 150 ms) test (ADR-0242 · TRD §2-4).

import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { t } from '../../../i18n'
import { JUMP_BUTTON_DELAY_MS, JumpToBottom } from './JumpToBottom'

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

const button = (): HTMLElement | null => document.querySelector('[data-jump-to-bottom="1"]')

function follow(pinned: boolean, unseenGrowth = false, scrollable = true) {
  return { pinned, unseenGrowth, scrollable, pin: vi.fn() }
}

describe('JumpToBottom', () => {
  it('absent if attached', () => {
    render(<JumpToBottom follow={follow(true)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS * 4))
    expect(button()).toBeNull()
  })

  it('after detaching it appears after 150 ms — even with no new content (U3 「whenever not at the bottom」)', () => {
    render(<JumpToBottom follow={follow(false, false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS - 1))
    expect(button()).toBeNull()
    act(() => vi.advanceTimersByTime(1))
    expect(button()).not.toBeNull()
    expect(button()!.getAttribute('aria-label')).toBe(t('slot.scrollToBottom'))
    expect(button()!.getAttribute('title')).toBe(t('slot.scrollToBottom'))
  })

  it('if it re-attaches within 150 ms it does not appear (tab-switch flicker)', () => {
    const { rerender } = render(<JumpToBottom follow={follow(false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS / 2))
    rerender(<JumpToBottom follow={follow(true)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS * 2))
    expect(button()).toBeNull()
  })

  it('on attach it disappears at once, and on detaching again it waits 150 ms again', () => {
    const { rerender } = render(<JumpToBottom follow={follow(false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    expect(button()).not.toBeNull()
    rerender(<JumpToBottom follow={follow(true)} />)
    expect(button()).toBeNull()
    rerender(<JumpToBottom follow={follow(false)} />)
    expect(button()).toBeNull()
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    expect(button()).not.toBeNull()
  })

  it('even when detached it does not appear if there is nothing to scroll', () => {
    render(<JumpToBottom follow={follow(false, true, false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS * 4))
    expect(button()).toBeNull()
  })

  it('if it is up and the content all fits (the viewport grows · the content shrinks) it disappears at once', () => {
    const { rerender } = render(<JumpToBottom follow={follow(false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    expect(button()).not.toBeNull()
    rerender(<JumpToBottom follow={follow(false, false, false)} />)
    expect(button()).toBeNull()
  })

  it('pressing calls pin', () => {
    const f = follow(false)
    render(<JumpToBottom follow={f} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    fireEvent.click(button()!)
    expect(f.pin).toHaveBeenCalledTimes(1)
  })

  // ADR-0237: pressing makes this button vanish — if focus drops to the document, that area's Esc does not reach the area root.
  it('pressing moves the focus the button held to the enclosing viewport (tabindex -1)', () => {
    const { getByTestId } = render(
      <div data-testid="viewport" tabIndex={-1}>
        <JumpToBottom follow={follow(false)} />
      </div>,
    )
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    button()!.focus()
    fireEvent.click(button()!)
    expect(document.activeElement).toBe(getByTestId('viewport'))
  })
})
