// 거부 표식 스위치의 대안 값(`DECLINED_MARK = 'error'` — 거부를 오류로 함께 센다)이 화면까지 닿나(ADR-0241 · TRD
//   S21-chat-ux §4-7 ⑨). 스위치는 모듈 상수라 파일 단위 모킹으로만 바꿀 수 있어 따로 떼었다 — 기본값('own')의 모양은
//   `StructuredTextView.test.tsx` 가 본다.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { t } from '../../i18n'
import { useToolGroupStore } from '../../store/toolGroupStore'
import type { ToolItem } from './structuredAccumulator'
import { StructuredTextView } from './StructuredTextView'

vi.mock('./chat/toolRuns', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./chat/toolRuns')>()
  return { ...actual, DECLINED_MARK: 'error' as const }
})

beforeEach(() => useToolGroupStore.setState({ bySlot: {} }))
afterEach(() => cleanup())

let nextItemId = 0
const tool = (id: string, resultMark: ToolItem['resultMark']): ToolItem => ({
  kind: 'tool',
  name: 'commandExecution',
  argsJson: '{}',
  id,
  category: 'Command',
  resultMark,
  itemId: nextItemId++,
})

describe("DECLINED_MARK = 'error'", () => {
  it('거부 둘 다 오류로 센다 — 머리 「오류 2」 · 붉은 Error 배지 · 사유 줄 없음 · data-tool-mark 는 표식 그대로', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container } = render(
      <StructuredTextView items={[tool('c1', 'refused'), tool('c2', 'declined')]} slotId="s1" />,
    )
    const header = container.querySelector('[data-tool-group] button') as HTMLButtonElement
    expect(header.textContent).toBe(
      [t('chat.toolGroupCommand', { count: '2' }), t('chat.toolGroupErrors', { count: '2' })].join(' · '),
    )
    fireEvent.click(header)
    expect(screen.getAllByText('Error')).toHaveLength(2)
    expect(screen.queryByText(t('chat.toolDeclined'))).toBeNull()
    expect(container.querySelector('[data-tool-declined-reason]')).toBeNull()
    const marks = Array.from(container.querySelectorAll('[data-tool-mark]')).map((el) => el.getAttribute('data-tool-mark'))
    expect(marks).toEqual(['declined', 'declined'])
  })
})
