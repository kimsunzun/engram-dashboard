// ADR-0231: 대기 입력 목록의 그리기 규칙 — 머리줄 없음 · queued 만 · 3 + 「외 N개」(눌러 펼침) · 툴팁 전문 ·
//   ✕ 는 모든 항목에(Tab 으로 닿는 버튼) · ✕ 는 명령을 디스패치할 뿐 스스로 감추지 않는다.

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
const waiting = (id: string, text = `text ${id}`): QueuedEntry => ({ id, text, phase: { state: 'queued' } })
const cancelling = (id: string): QueuedEntry => ({
  id,
  text: `text ${id}`,
  phase: { state: 'cancelling', answer: 'none', vendorClosed: false },
})
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

  it('queued 만 그린다 — 취소 대기는 그리지 않는다(남는 것이 없으면 통째로 안 그린다)', () => {
    render(<QueuedInputList agentId={AGENT} entries={[waiting('A'), cancelling('B'), waiting('C')]} />)
    expect(shownIds()).toEqual(['A', 'C'])
    cleanup()
    const { container } = render(<QueuedInputList agentId={AGENT} entries={[cancelling('B')]} />)
    expect(container.firstChild).toBeNull()
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
