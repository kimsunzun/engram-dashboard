// 「맨 아래로」 버튼 — U3(바닥이 아니면 늘 · 스크롤할 것이 있을 때만 · 150 ms 늦춤) 시험(ADR-0242 · TRD §2-4).

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
  it('붙어 있으면 없다', () => {
    render(<JumpToBottom follow={follow(true)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS * 4))
    expect(button()).toBeNull()
  })

  it('떨어지면 150 ms 뒤에 뜬다 — 새 내용이 없어도(U3 「바닥이 아니면 늘」)', () => {
    render(<JumpToBottom follow={follow(false, false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS - 1))
    expect(button()).toBeNull()
    act(() => vi.advanceTimersByTime(1))
    expect(button()).not.toBeNull()
    expect(button()!.getAttribute('aria-label')).toBe(t('slot.scrollToBottom'))
    expect(button()!.getAttribute('title')).toBe(t('slot.scrollToBottom'))
  })

  it('150 ms 안에 다시 붙으면 뜨지 않는다(탭 전환 깜빡임)', () => {
    const { rerender } = render(<JumpToBottom follow={follow(false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS / 2))
    rerender(<JumpToBottom follow={follow(true)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS * 2))
    expect(button()).toBeNull()
  })

  it('붙으면 곧바로 사라지고, 다시 떨어지면 150 ms 를 다시 기다린다', () => {
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

  it('떨어져 있어도 스크롤할 것이 없으면 뜨지 않는다', () => {
    render(<JumpToBottom follow={follow(false, true, false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS * 4))
    expect(button()).toBeNull()
  })

  it('떠 있다가 내용이 다 들어오면(뷰포트가 커짐 · 내용이 줄어듦) 곧바로 사라진다', () => {
    const { rerender } = render(<JumpToBottom follow={follow(false)} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    expect(button()).not.toBeNull()
    rerender(<JumpToBottom follow={follow(false, false, false)} />)
    expect(button()).toBeNull()
  })

  it('누르면 pin 을 부른다', () => {
    const f = follow(false)
    render(<JumpToBottom follow={f} />)
    act(() => vi.advanceTimersByTime(JUMP_BUTTON_DELAY_MS))
    fireEvent.click(button()!)
    expect(f.pin).toHaveBeenCalledTimes(1)
  })
})
