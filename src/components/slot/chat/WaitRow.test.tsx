// Wait indicator row test — an always-present fixed-height row · the indicator only while streaming · identity of elapsed seconds (user decision 2026-10-01 ·
//   ADR-0244 「interrupting」).

import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { t } from '../../../i18n'
import { WaitStrip } from './WaitRow'

afterEach(() => {
  cleanup()
  vi.useRealTimers()
})

const strip = (c: HTMLElement) => c.querySelector('[data-wait-strip="1"]') as HTMLElement

describe('WaitStrip — place', () => {
  it('even when not streaming the line stands and is empty — the height is fixed by one variable', () => {
    const { container, rerender } = render(<WaitStrip streaming={false} interrupting={false} />)
    const node = strip(container)
    expect(node).not.toBeNull()
    expect(node.childElementCount).toBe(0)
    expect(node.style.height).toBe('var(--chat-wait-strip-h)')

    rerender(<WaitStrip streaming interrupting={false} />)
    expect(strip(container)).toBe(node)
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(node.style.height).toBe('var(--chat-wait-strip-h)')

    rerender(<WaitStrip streaming={false} interrupting={false} />)
    expect(strip(container)).toBe(node)
    expect(node.childElementCount).toBe(0)
  })

  it('the label is the right cell — it stands after the waiting text, and is the same node even when the waiting indicator turns on and off', () => {
    const label = <span data-test-label="1">agent</span>
    const { container, rerender } = render(<WaitStrip streaming={false} interrupting={false} label={label} />)
    const node = container.querySelector('[data-test-label="1"]') as HTMLElement
    expect(strip(container).contains(node)).toBe(true)

    rerender(<WaitStrip streaming interrupting={false} label={label} />)
    expect(container.querySelector('[data-test-label="1"]')).toBe(node)
    expect(screen.getByText(t('chat.wait')).compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()

    rerender(<WaitStrip streaming={false} interrupting={false} label={label} />)
    expect(container.querySelector('[data-test-label="1"]')).toBe(node)
  })
})

// ── ADR-0244: 「interrupting」 after sending an interrupt ──────────────────────────────────────
describe('WaitStrip — interrupting (ADR-0244)', () => {
  it('if interrupting it draws CircleStop + 「interrupting…」 instead of Wait', () => {
    const { container } = render(<WaitStrip streaming interrupting />)
    const row = container.querySelector('[data-wait-interrupting="1"]') as HTMLElement
    expect(row).not.toBeNull()
    expect(row.textContent).toBe(t('chat.interrupting'))
    expect(row.querySelector('svg')?.getAttribute('class')).toContain('lucide-circle-stop')
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
  })

  it('if not streaming it draws nothing even if interrupting', () => {
    const { container } = render(<WaitStrip streaming={false} interrupting />)
    expect(container.querySelector('[data-wait-interrupting="1"]')).toBeNull()
    expect(screen.queryByText(t('chat.interrupting'))).toBeNull()
  })
})

describe('WaitStrip — elapsed seconds', () => {
  it('it is not remounted when moving in and out of interrupting — the elapsed seconds continue', () => {
    vi.useFakeTimers()
    const { rerender } = render(<WaitStrip streaming interrupting={false} />)
    act(() => vi.advanceTimersByTime(3000))
    expect(screen.getByText('3s')).toBeTruthy()
    rerender(<WaitStrip streaming interrupting />)
    expect(screen.queryByText(t('chat.wait'))).toBeNull()
    act(() => vi.advanceTimersByTime(2000))
    rerender(<WaitStrip streaming interrupting={false} />)
    expect(screen.getByText(t('chat.wait'))).toBeTruthy()
    expect(screen.getByText('5s')).toBeTruthy()
  })

  it('if streaming goes off and on again (a new turn) it counts from 0', () => {
    vi.useFakeTimers()
    const { rerender } = render(<WaitStrip streaming interrupting={false} />)
    act(() => vi.advanceTimersByTime(4000))
    expect(screen.getByText('4s')).toBeTruthy()
    rerender(<WaitStrip streaming={false} interrupting={false} />)
    act(() => vi.advanceTimersByTime(1000))
    rerender(<WaitStrip streaming interrupting={false} />)
    expect(screen.getByText('0s')).toBeTruthy()
    act(() => vi.advanceTimersByTime(1000))
    expect(screen.getByText('1s')).toBeTruthy()
  })
})
