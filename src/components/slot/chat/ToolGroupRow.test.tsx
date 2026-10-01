// 도구 묶음 한 줄 시험 — 요약 머리(시제 포함) · 펼침 상태(기본 접힘 · 스토어 · command 경로) · 펼침 알림(ADR-0239 · ADR-0241 ·
//   TRD S21-chat-ux §4-3 · §4-4 · §4-5). 멤버 행 모양은 `StructuredTextView.test.tsx` 가 본다 — 여기서는 멤버를 표지로만 그린다.

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

describe('ToolGroupRow — 요약 머리', () => {
  it('종류별 수 · 오류 · 거부를 고정 순서로 잇는다 — 오류는 붉은 톤 · 거부는 주황 토큰', () => {
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

  it('벤더 오류 id 로 센 오류도 머리에 선다(claude) · 거부 표식이 벤더 오류를 이긴다', () => {
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

  // ADR-0263 (ADR-0239 결정 4 개정): 도는 동안 진행형 · 끝나면 그 표시가 빠져 종류별 수만 남는다(과거형).
  it('live 면 요약 뒤에 진행형 표시가 붙고, 끝나면 빠진다 — 그 사이 수는 제자리에서 자란다', () => {
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

describe('ToolGroupRow — 펼침', () => {
  const shownIds = (c: HTMLElement) =>
    Array.from(c.querySelectorAll('[data-member]')).map((el) => el.getAttribute('data-member'))

  // ADR-0263 결정 1(사용자 결정 2026-10-01): 도는 동안 머리 + 마지막 호출 한 줄 · 끝나면 머리만.
  it('고른 값이 없으면 도는 동안 마지막 호출 한 줄만 · 호출이 늘면 그 줄이 새 마지막으로 바뀐다 · 끝나면 머리만', () => {
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

  it('슬롯이 없으면 머리를 눌러도 안 바뀐다 — 도는 동안의 한 줄도 그대로', () => {
    const calls = [tool('a'), tool('b')]
    renderRow(group(calls, true))
    expect((header() as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(header())
    expect(header().getAttribute('aria-expanded')).toBe('false')
    expect(memberCount(document.body)).toBe(1)
  })

  it('펼치면 멤버를 원래 순서로 그린다', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const calls = [tool('a'), tool('b'), tool('c')]
    const { container } = renderRow(group(calls), { slotId: 's1' })
    fireEvent.click(header())
    expect(Array.from(container.querySelectorAll('[data-member]')).map((el) => el.getAttribute('data-member'))).toEqual(
      calls.map((c) => String(c.itemId)),
    )
  })

  it('사람 토글이 스토어에 적히고 두 방향 다 이긴다 — 도는 중 펼치면 전부 · 도는 중 접으면 머리만', () => {
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
    expect(memberCount(container)).toBe(0) // 도는 중이어도 고른 접힘은 마지막 호출 줄까지 걷는다
    fireEvent.click(header())

    // 턴이 끝나도 사람이 펼친 묶음은 펼친 채다 — 접으면 접힌 값이 적힌다.
    rerender(
      <ToolGroupRow row={group(calls, false)} vendorErrorIds={new Set()} runPos="single" isLast renderMember={member} slotId="s1" />,
    )
    expect(root(container).getAttribute('data-tool-group-open')).toBe('1')
    expect(memberCount(container)).toBe(2)
    fireEvent.click(header())
    expect(useToolGroupStore.getState().bySlot['s1'].open).toEqual({ [row.key]: false })
    expect(root(container).getAttribute('data-tool-group-open')).toBe('0')
  })

  it('command 경로(스토어 setOpen)로 바뀐 값을 그대로 그린다 — 펼침 알림은 부르지 않는다', () => {
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

  it('펼침 알림은 사람이 펼칠 때만 · isLast 를 싣는다 — 접을 때는 없다', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const onGroupToggle = vi.fn()
    renderRow(group([tool('a'), tool('b')]), { slotId: 's1', isLast: false, onGroupToggle })
    fireEvent.click(header())
    expect(onGroupToggle).toHaveBeenCalledTimes(1)
    expect(onGroupToggle).toHaveBeenLastCalledWith(false)
    fireEvent.click(header())
    expect(onGroupToggle).toHaveBeenCalledTimes(1)
  })

  it('슬롯이 묶여 있지 않으면 아무것도 안 바꾸고 알림도 없이 경고만 남긴다', () => {
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
