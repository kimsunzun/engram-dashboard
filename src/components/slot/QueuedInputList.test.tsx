// ADR-0231: drawing rules of the queued-input list — no head line · all open items (including cancel-pending) · 3 + 「N more; click to expand」 ·
//   full text in tooltip · ✕ on items not yet handed over (a button reachable with Tab) · ✕ locked while waiting for a cancel reply · no ✕ on handed-over items and
//   on items whose reply said it could not be removed ·
//   ✕ only dispatches the command and never hides itself.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const dispatchMock = vi.hoisted(() => ({ fireAndForget: vi.fn() }))
vi.mock('../../commands/dispatch', () => ({
  fireAndForget: (...args: unknown[]) => dispatchMock.fireAndForget(...args),
}))

import { QueuedInputList } from './QueuedInputList'
import type { QueuedEntry } from './queuedInputReducer'
import { t } from '../../i18n'

const AGENT = 'agent-1'
const waiting = (id: string, text = `text ${id}`): QueuedEntry => ({
  id,
  text,
  phase: { state: 'queued' },
  sent: false,
})
const cancelling = (id: string, answer: 'none' | 'not_removed' = 'none'): QueuedEntry => ({
  id,
  text: `text ${id}`,
  phase: { state: 'cancelling', answer, vendorClosed: false },
  sent: false,
})
const sent = (entry: QueuedEntry): QueuedEntry => ({ ...entry, sent: true })
const removeButton = (id: string): HTMLButtonElement | null =>
  document.querySelector(`[data-queued-input="${id}"] button`)
const shownIds = (): string[] =>
  Array.from(document.querySelectorAll('[data-queued-input]')).map((el) => el.getAttribute('data-queued-input') ?? '')
const moreButton = (): HTMLElement | null => document.querySelector('[data-queued-more="1"]')

beforeEach(() => dispatchMock.fireAndForget.mockClear())
afterEach(() => cleanup())

describe('QueuedInputList(ADR-0231)', () => {
  it('draws nothing if there are no items', () => {
    const { container } = render(<QueuedInputList agentId={AGENT} entries={[]} />)
    expect(container.firstChild).toBeNull()
  })

  // ADR-0231 · ADR-0235 — ✕ removes the row only after the registry confirms it was removed (it does not hide in advance).
  it('cancel-pending is also drawn — while waiting for the answer ✕ is locked', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), cancelling('B'), waiting('C')]} />)
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(removeButton('B')?.disabled).toBe(true)
    expect(removeButton('A')?.disabled).toBe(false)
    expect(document.querySelector('[data-queued-input="B"]')?.getAttribute('data-queued-state')).toBe('cancelling')
    fireEvent.click(removeButton('B')!)
    expect(dispatchMock.fireAndForget).not.toHaveBeenCalled()
  })

  it('a row that got a could-not-remove answer stands like a handed-over row — the text already went over so there is no ✕', () => {
    render(<QueuedInputList agentId={AGENT} entries={[cancelling('B', 'not_removed'), waiting('C')]} />)
    expect(shownIds()).toEqual(['B', 'C'])
    expect(removeButton('B')).toBeNull()
    expect(removeButton('C')).not.toBeNull()
    expect(document.querySelector('[data-queued-input="B"]')?.getAttribute('data-queued-state')).toBe('sent')
  })

  it('a handed-over row has no ✕ — even if it was cancel-pending (✕ overlapped the handover) it stands as 「sent」', () => {
    render(
      <QueuedInputList
        agentId={AGENT}
        entries={[sent(waiting('A')), waiting('B'), sent(cancelling('C', 'not_removed'))]}
      />,
    )
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(removeButton('A')).toBeNull()
    expect(removeButton('C')).toBeNull()
    expect(removeButton('B')).not.toBeNull()
    expect(document.querySelector('[data-queued-input="A"]')?.getAttribute('data-queued-state')).toBe('sent')
    expect(screen.getAllByRole('button', { name: t('chat.queuedRemove') })).toHaveLength(1)
  })

  it('there is no head line — apart from item lines and ✕ there is no other text', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A', 'hello')]} />)
    const list = document.querySelector('[data-queued-inputs="1"]') as HTMLElement
    expect(list.textContent).toBe('hello')
    expect(list.querySelector('h1,h2,h3,h4,h5,h6,header')).toBeNull()
  })

  it('up to 3 are not folded', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), waiting('B'), waiting('C')]} />)
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(moreButton()).toBeNull()
  })

  it('over 3 shows the oldest 3 + 「N more」 — pressing expands', () => {
    const entries = ['A', 'B', 'C', 'D', 'E'].map((id) => waiting(id))
    render(<QueuedInputList agentId={AGENT} entries={entries} />)
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(moreButton()?.textContent).toBe(t('chat.queuedMore', { count: '2' }))
    expect(moreButton()?.tagName).toBe('BUTTON')
    fireEvent.click(moreButton()!)
    expect(shownIds()).toEqual(['A', 'B', 'C', 'D', 'E'])
    expect(moreButton()).toBeNull()
  })

  it('the folded shape is adjusted by prop defaults (count · visible side · expanded from the start)', () => {
    const entries = ['A', 'B', 'C', 'D'].map((id) => waiting(id))
    render(<QueuedInputList agentId={AGENT} entries={entries} collapsedCount={1} collapsedSide="newest" />)
    expect(shownIds()).toEqual(['D'])
    expect(moreButton()?.textContent).toBe(t('chat.queuedMore', { count: '3' }))
    cleanup()
    render(<QueuedInputList agentId={AGENT} entries={entries} defaultExpanded />)
    expect(shownIds()).toEqual(['A', 'B', 'C', 'D'])
  })

  it('cut to one line and the full text goes in the tooltip', () => {
    const long = '아주 긴 글 '.repeat(40)
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A', long)]} />)
    const text = document.querySelector('[data-queued-input="A"] span') as HTMLElement
    expect(text.className).toContain('truncate')
    expect(text.className).toContain('text-muted')
    expect(text.getAttribute('title')).toBe(long)
  })

  it('✕ is on every item, is a button reached by Tab, and the tooltip is 「remove from list」', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), waiting('B')]} />)
    const buttons = screen.getAllByRole('button', { name: t('chat.queuedRemove') })
    expect(buttons).toHaveLength(2)
    for (const b of buttons) {
      expect(b.getAttribute('title')).toBe('목록에서 빼기')
      expect(b.getAttribute('tabindex')).not.toBe('-1')
      expect((b as HTMLButtonElement).type).toBe('button')
    }
  })

  it('✕ only dispatches agent.cancelQueuedInput with inputId and does not hide itself', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), waiting('B')]} />)
    fireEvent.click(screen.getAllByRole('button', { name: t('chat.queuedRemove') })[1])
    expect(dispatchMock.fireAndForget).toHaveBeenCalledTimes(1)
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('agent.cancelQueuedInput', {
      agentId: AGENT,
      inputId: 'B',
    })
    expect(shownIds()).toEqual(['A', 'B'])
  })

  // ADR-0237: even if the pressed button locks or disappears, focus does not drop to the document — the area's Esc must reach the area root.
  it('pressing ✕ · 「N more」 moves the focus the button held to the area container (tabindex -1)', () => {
    const entries = ['A', 'B', 'C', 'D'].map((id) => waiting(id))
    render(
      <div data-testid="slot" tabIndex={-1}>
        <QueuedInputList agentId={AGENT} entries={entries} />
      </div>,
    )
    const slot = screen.getByTestId('slot')

    removeButton('A')!.focus()
    fireEvent.click(removeButton('A')!)
    expect(document.activeElement).toBe(slot)

    moreButton()!.focus()
    fireEvent.click(moreButton()!)
    expect(moreButton()).toBeNull()
    expect(document.activeElement).toBe(slot)
  })
})
