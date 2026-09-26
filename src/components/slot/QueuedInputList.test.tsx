// ADR-0231: 대기 입력 목록의 그리기 규칙 — 머리줄 없음 · 열린 항목 전부(취소 대기 포함) · 3 + 「외 N개」(눌러 펼침) ·
//   툴팁 전문 · ✕ 는 넘기지 않은 항목에(Tab 으로 닿는 버튼) · 취소 답을 기다리는 동안 ✕ 잠김 · 넘긴 항목과
//   못 뺐다는 답이 온 항목은 ✕ 없음 ·
//   ✕ 는 명령을 디스패치할 뿐 스스로 감추지 않는다.

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
  it('항목이 없으면 아무것도 그리지 않는다', () => {
    const { container } = render(<QueuedInputList agentId={AGENT} entries={[]} />)
    expect(container.firstChild).toBeNull()
  })

  // ADR-0231: 단순화 계획 2026-09-26 항목 8 — ✕ 는 명부가 뺐다고 확인한 뒤에만 행을 뺀다(미리 감추지 않는다).
  it('취소 대기도 그린다 — 답을 기다리는 동안 ✕ 가 잠긴다', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), cancelling('B'), waiting('C')]} />)
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(removeButton('B')?.disabled).toBe(true)
    expect(removeButton('A')?.disabled).toBe(false)
    expect(document.querySelector('[data-queued-input="B"]')?.getAttribute('data-queued-state')).toBe('cancelling')
    fireEvent.click(removeButton('B')!)
    expect(dispatchMock.fireAndForget).not.toHaveBeenCalled()
  })

  it('못 뺐다는 답이 온 행은 넘긴 행처럼 선다 — 글이 이미 넘어갔으니 ✕ 가 없다', () => {
    render(<QueuedInputList agentId={AGENT} entries={[cancelling('B', 'not_removed'), waiting('C')]} />)
    expect(shownIds()).toEqual(['B', 'C'])
    expect(removeButton('B')).toBeNull()
    expect(removeButton('C')).not.toBeNull()
    expect(document.querySelector('[data-queued-input="B"]')?.getAttribute('data-queued-state')).toBe('sent')
  })

  it('넘긴 행은 ✕ 가 없다 — 취소 대기였어도(✕ 가 넘기기와 겹쳤다) 「보냄」으로 선다', () => {
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

  it('머리줄이 없다 — 항목 줄과 ✕ 말고 다른 글이 없다', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A', 'hello')]} />)
    const list = document.querySelector('[data-queued-inputs="1"]') as HTMLElement
    expect(list.textContent).toBe('hello')
    expect(list.querySelector('h1,h2,h3,h4,h5,h6,header')).toBeNull()
  })

  it('3개까지는 접지 않는다', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), waiting('B'), waiting('C')]} />)
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(moreButton()).toBeNull()
  })

  it('3개를 넘으면 가장 오래된 3개 + 「외 N개」 — 누르면 펼친다', () => {
    const entries = ['A', 'B', 'C', 'D', 'E'].map((id) => waiting(id))
    render(<QueuedInputList agentId={AGENT} entries={entries} />)
    expect(shownIds()).toEqual(['A', 'B', 'C'])
    expect(moreButton()?.textContent).toBe(t('chat.queuedMore', { count: '2' }))
    expect(moreButton()?.tagName).toBe('BUTTON')
    fireEvent.click(moreButton()!)
    expect(shownIds()).toEqual(['A', 'B', 'C', 'D', 'E'])
    expect(moreButton()).toBeNull()
  })

  it('접힘 모양은 props 기본값으로 조정한다(개수 · 보이는 쪽 · 처음부터 펼침)', () => {
    const entries = ['A', 'B', 'C', 'D'].map((id) => waiting(id))
    render(<QueuedInputList agentId={AGENT} entries={entries} collapsedCount={1} collapsedSide="newest" />)
    expect(shownIds()).toEqual(['D'])
    expect(moreButton()?.textContent).toBe(t('chat.queuedMore', { count: '3' }))
    cleanup()
    render(<QueuedInputList agentId={AGENT} entries={entries} defaultExpanded />)
    expect(shownIds()).toEqual(['A', 'B', 'C', 'D'])
  })

  it('한 줄로 잘리고 툴팁에 전문이 실린다', () => {
    const long = '아주 긴 글 '.repeat(40)
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A', long)]} />)
    const text = document.querySelector('[data-queued-input="A"] span') as HTMLElement
    expect(text.className).toContain('truncate')
    expect(text.className).toContain('text-muted')
    expect(text.getAttribute('title')).toBe(long)
  })

  it('✕ 는 모든 항목에 있고, Tab 으로 닿는 버튼이며 툴팁은 「목록에서 빼기」', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), waiting('B')]} />)
    const buttons = screen.getAllByRole('button', { name: t('chat.queuedRemove') })
    expect(buttons).toHaveLength(2)
    for (const b of buttons) {
      expect(b.getAttribute('title')).toBe('목록에서 빼기')
      expect(b.getAttribute('tabindex')).not.toBe('-1')
      expect((b as HTMLButtonElement).type).toBe('button')
    }
  })

  it('✕ 는 inputId 와 함께 agent.cancelQueuedInput 을 디스패치할 뿐 스스로 감추지 않는다', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), waiting('B')]} />)
    fireEvent.click(screen.getAllByRole('button', { name: t('chat.queuedRemove') })[1])
    expect(dispatchMock.fireAndForget).toHaveBeenCalledTimes(1)
    expect(dispatchMock.fireAndForget).toHaveBeenCalledWith('agent.cancelQueuedInput', {
      agentId: AGENT,
      inputId: 'B',
    })
    expect(shownIds()).toEqual(['A', 'B'])
  })
})
