// Tool bundle one-line test — summary head (including tense) · expansion state (default collapsed · store · command path) · expansion notification (ADR-0239 · ADR-0241 ·
//   TRD S21-chat-ux §4-3 · §4-4 · §4-5). Member row shapes are checked by `StructuredTextView.test.tsx` — here members are drawn only as markers.

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { t } from '../../../i18n'
import { useToolGroupStore } from '../../../store/toolGroupStore'
import type { StructuredItem, ToolItem } from '../structuredAccumulator'
import { ToolGroupRow, type ToolGroupDisplayRow } from './ToolGroupRow'

beforeEach(() => useToolGroupStore.setState({ bySlot: {} }))
afterEach(() => cleanup())

let nextItemId = 0
function tool(id: string, extra: Partial<ToolItem> = {}): ToolItem {
  return { kind: 'tool', name: 'Read', argsJson: '{}', id, category: 'Read', resultMark: null, itemId: nextItemId++, ...extra }
}
function group(calls: ToolItem[], live = false, key = `tool:${calls[0].id}`): ToolGroupDisplayRow {
  return { kind: 'toolGroup', key, members: calls, calls, live }
}
const member = (item: StructuredItem) => <div data-member={item.itemId}>member</div>

function renderRow(
  row: ToolGroupDisplayRow,
  extra: { slotId?: string; isLast?: boolean; onGroupToggle?: (isLast: boolean) => void; vendorErrorIds?: Set<string> } = {},
) {
  return render(
    <ToolGroupRow
      row={row}
      vendorErrorIds={extra.vendorErrorIds ?? new Set()}
      runPos="single"
      isLast={extra.isLast ?? true}
      renderMember={member}
      slotId={extra.slotId}
      onGroupToggle={extra.onGroupToggle}
    />,
  )
}
const root = (c: HTMLElement) => c.querySelector('[data-tool-group]') as HTMLElement
const header = () => screen.getByRole('button')
const memberCount = (c: HTMLElement) => c.querySelectorAll('[data-member]').length

describe('ToolGroupRow — summary head', () => {
  it('joins count per kind · error · rejection in a fixed order — errors a red tone · rejection an orange token', () => {
    const calls = [
      tool('a', { category: 'Read' }),
      tool('b', { category: 'Search' }),
      tool('c', { category: 'Command', resultMark: 'failed' }),
      tool('d', { category: 'Command', resultMark: 'refused' }),
    ]
    const { container } = renderRow(group(calls))
    const line = [
      t('chat.toolGroupSearch', { count: '1' }),
      t('chat.toolGroupRead', { count: '1' }),
      t('chat.toolGroupCommand', { count: '2' }),
      t('chat.toolGroupErrors', { count: '1' }),
      t('chat.toolGroupDeclined', { count: '1' }),
    ].join(' · ')
    expect(header().textContent).toBe(line)
    expect(root(container).getAttribute('data-tool-group-live')).toBe('0')
    expect(screen.getByText(t('chat.toolGroupErrors', { count: '1' })).className).toContain('text-red-500')
    const declined = screen.getByText(t('chat.toolGroupDeclined', { count: '1' }))
    expect(declined.style.color).toBe('var(--status-blocked)')
    expect(declined.className).not.toContain('text-red-500')
    expect(root(container).getAttribute('data-tool-group-count')).toBe('4')
  })

  it('errors counted by vendor error id also stand in the head (claude) · the rejection marker wins over the vendor error', () => {
    const calls = [tool('a'), tool('b'), tool('c', { resultMark: 'declined' })]
    renderRow(group(calls), { vendorErrorIds: new Set(['b', 'c']) })
    expect(header().textContent).toBe(
      [
        t('chat.toolGroupRead', { count: '3' }),
        t('chat.toolGroupErrors', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '1' }),
      ].join(' · '),
    )
  })

  // ADR-0263 (amendment of ADR-0239 decision 4): progressive while running · when done that indicator drops and only the per-kind count remains (past tense).
  it('if live a progressive indicator is attached after the summary and drops when finished — the count grows in place meanwhile', () => {
    const [a, b, c] = [tool('a'), tool('b'), tool('c')]
    const { container, rerender } = renderRow(group([a, b], true))
    expect(root(container).getAttribute('data-tool-group-live')).toBe('1')
    expect(header().textContent).toBe(t('chat.toolGroupRead', { count: '2' }) + t('chat.toolGroupRunning'))
    const node = root(container)

    rerender(<ToolGroupRow row={group([a, b, c], true)} vendorErrorIds={new Set()} runPos="single" isLast renderMember={member} />)
    expect(root(container)).toBe(node)
    expect(header().textContent).toBe(t('chat.toolGroupRead', { count: '3' }) + t('chat.toolGroupRunning'))

    rerender(<ToolGroupRow row={group([a, b, c], false)} vendorErrorIds={new Set()} runPos="single" isLast renderMember={member} />)
    expect(root(container).getAttribute('data-tool-group-live')).toBe('0')
    expect(header().textContent).toBe(t('chat.toolGroupRead', { count: '3' }))
    expect(screen.queryByText(t('chat.toolGroupRunning'))).toBeNull()
  })
})

describe('ToolGroupRow — expansion', () => {
  const shownIds = (c: HTMLElement) =>
    Array.from(c.querySelectorAll('[data-member]')).map((el) => el.getAttribute('data-member'))

  // ADR-0263 decision 1 (user decision 2026-10-01): while running head + the last call as one line · when done head only.
  it('with no chosen value only the last call line while running · when calls grow that line becomes the new last · head only when finished', () => {
    const [a, b, c] = [tool('a'), tool('b'), tool('c')]
    const { container, rerender } = renderRow(group([a, b], true))
    const node = root(container)
    const head = header()
    expect(node.getAttribute('data-tool-group-open')).toBe('0')
    expect(head.getAttribute('aria-expanded')).toBe('false')
    expect(shownIds(container)).toEqual([String(b.itemId)])

    rerender(<ToolGroupRow row={group([a, b, c], true)} vendorErrorIds={new Set()} runPos="single" isLast renderMember={member} />)
    expect(root(container)).toBe(node)
    expect(header()).toBe(head)
    expect(shownIds(container)).toEqual([String(c.itemId)])

    rerender(<ToolGroupRow row={group([a, b, c], false)} vendorErrorIds={new Set()} runPos="single" isLast renderMember={member} />)
    expect(root(container)).toBe(node)
    expect(header()).toBe(head)
    expect(shownIds(container)).toEqual([])
  })

  it('without a slot pressing the head changes nothing — the one line while running as is', () => {
    const calls = [tool('a'), tool('b')]
    renderRow(group(calls, true))
    expect((header() as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(header())
    expect(header().getAttribute('aria-expanded')).toBe('false')
    expect(memberCount(document.body)).toBe(1)
  })

  it('expanding draws the members in the original order', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const calls = [tool('a'), tool('b'), tool('c')]
    const { container } = renderRow(group(calls), { slotId: 's1' })
    fireEvent.click(header())
    expect(Array.from(container.querySelectorAll('[data-member]')).map((el) => el.getAttribute('data-member'))).toEqual(
      calls.map((c) => String(c.itemId)),
    )
  })

  it('the human toggle is written to the store and wins both ways — expanding while running shows all · collapsing while running leaves only the head', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const calls = [tool('a'), tool('b')]
    const row = group(calls, true)
    const { container, rerender } = renderRow(row, { slotId: 's1' })
    expect(root(container).getAttribute('data-tool-group-open')).toBe('0')
    expect(memberCount(container)).toBe(1)
    fireEvent.click(header())
    expect(useToolGroupStore.getState().bySlot['s1'].open).toEqual({ [row.key]: true })
    expect(root(container).getAttribute('data-tool-group-open')).toBe('1')
    expect(memberCount(container)).toBe(2)
    fireEvent.click(header())
    expect(useToolGroupStore.getState().bySlot['s1'].open).toEqual({ [row.key]: false })
    expect(memberCount(container)).toBe(0) // Even while running, a chosen collapse also takes away the last call line
    fireEvent.click(header())

    // A bundle a human expanded stays expanded even when the turn ends — if collapsed, the collapsed value is recorded.
    rerender(
      <ToolGroupRow row={group(calls, false)} vendorErrorIds={new Set()} runPos="single" isLast renderMember={member} slotId="s1" />,
    )
    expect(root(container).getAttribute('data-tool-group-open')).toBe('1')
    expect(memberCount(container)).toBe(2)
    fireEvent.click(header())
    expect(useToolGroupStore.getState().bySlot['s1'].open).toEqual({ [row.key]: false })
    expect(root(container).getAttribute('data-tool-group-open')).toBe('0')
  })

  it('draws the value changed via the command path (store setOpen) as is — it does not call the expansion notice', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const onGroupToggle = vi.fn()
    const row = group([tool('a'), tool('b')])
    const { container } = renderRow(row, { slotId: 's1', onGroupToggle })
    expect(root(container).getAttribute('data-tool-group-open')).toBe('0')
    act(() => {
      useToolGroupStore.getState().setOpen('s1', row.key, true)
    })
    expect(root(container).getAttribute('data-tool-group-open')).toBe('1')
    expect(onGroupToggle).not.toHaveBeenCalled()
  })

  it('the expansion notice only when a human expands · carries isLast — none when collapsing', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const onGroupToggle = vi.fn()
    renderRow(group([tool('a'), tool('b')]), { slotId: 's1', isLast: false, onGroupToggle })
    fireEvent.click(header())
    expect(onGroupToggle).toHaveBeenCalledTimes(1)
    expect(onGroupToggle).toHaveBeenLastCalledWith(false)
    fireEvent.click(header())
    expect(onGroupToggle).toHaveBeenCalledTimes(1)
  })

  it('if the slot is not bound it changes nothing and leaves only a warning without a notice', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const onGroupToggle = vi.fn()
      const { container } = renderRow(group([tool('a'), tool('b')]), { slotId: 'unbound', onGroupToggle })
      fireEvent.click(header())
      expect(root(container).getAttribute('data-tool-group-open')).toBe('0')
      expect(onGroupToggle).not.toHaveBeenCalled()
      expect(warn).toHaveBeenCalledTimes(1)
    } finally {
      warn.mockRestore()
    }
  })
})
