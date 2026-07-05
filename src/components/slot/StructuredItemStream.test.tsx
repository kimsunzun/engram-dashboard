// StructuredItemStream 렌더 테스트 — 칩이 접힌 채 뜨고, 클릭하면 펼쳐지는지(ADR-0045 §52 사용자 결정).
// 순수 렌더(구독/누적 무관) — StructuredItem[] 를 직접 넣어 DOM 을 관측한다.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { StructuredItemStream } from './StructuredItemStream'
import type { StructuredItem } from './structuredAccumulator'

afterEach(() => cleanup())

describe('StructuredItemStream', () => {
  it('tool 칩은 접힌 한 줄로 뜨고(상세 숨김), 클릭하면 args 상세가 펼쳐진다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 0 },
    ]
    render(<StructuredItemStream items={items} />)

    const chip = screen.getByRole('button', { name: /Read/ })
    // 접힌 상태 — aria-expanded=false, 상세(path)는 아직 DOM 에 없다.
    expect(chip.getAttribute('aria-expanded')).toBe('false')
    expect(screen.queryByText(/"path"/)).toBeNull()

    fireEvent.click(chip)
    // 펼침 — aria-expanded=true, args JSON 상세가 나타난다.
    expect(chip.getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByText(/"path": "a\.ts"/)).toBeTruthy()

    // 다시 클릭 → 접힘.
    fireEvent.click(chip)
    expect(chip.getAttribute('aria-expanded')).toBe('false')
    expect(screen.queryByText(/"path"/)).toBeNull()
  })

  it('usage/error 칩도 각각 접힌 한 줄 버튼으로 렌더된다', () => {
    const items: StructuredItem[] = [
      { kind: 'usage', inputTokens: 10, outputTokens: 5, itemId: 0 },
      { kind: 'error', message: 'boom', itemId: 1 },
    ]
    render(<StructuredItemStream items={items} />)
    expect(screen.getByRole('button', { name: /usage/ })).toBeTruthy()
    expect(screen.getByRole('button', { name: /error/ })).toBeTruthy()
    // error 요약은 접힌 상태에서도 헤더에 보인다(한 줄 요약).
    expect(screen.getByText('boom')).toBeTruthy()
  })

  it('separator item 은 구분선(hr)으로, text 는 Markdown 본문으로 렌더된다', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'hello world', itemId: 0 },
      { kind: 'separator', itemId: 1 },
      { kind: 'text', text: 'next turn', itemId: 2 },
    ]
    const { container } = render(<StructuredItemStream items={items} />)
    expect(container.querySelector('hr.si-separator')).toBeTruthy()
    expect(screen.getByText('hello world')).toBeTruthy()
    expect(screen.getByText('next turn')).toBeTruthy()
  })
})
