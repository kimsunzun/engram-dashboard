// ADR-0050: 결정적 어댑터 동작(매핑·흡수·필터)을 검증하고, leaf 내부 렌더(chat/*)는 스모크 수준만 본다
//   (react-markdown 등 세부는 leaf 자체 테스트의 몫).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import { StructuredTextView } from './StructuredTextView'
import {
  StructuredEventAccumulator,
  type StructuredItem,
  type ToolCategory,
  type ToolItem,
} from './structuredAccumulator'
import { t } from '../../i18n'
import { useToolGroupStore } from '../../store/toolGroupStore'

// 도구 묶음 펼침 스토어는 zustand 싱글톤이라 시험 간 격리.
beforeEach(() => useToolGroupStore.setState({ bySlot: {} }))
afterEach(() => cleanup())

// (computeRailRunPositions 순수 함수 테스트는 ADR-0053 구조 분할로 ./chat/railPositions.test.ts 로 이사.)

// ── ADR-0051: rail 연결선 clean-ends 렌더 ──────────────────────────────────────────
describe('StructuredTextView rail line clean-ends (ADR-0051)', () => {
  it('단일 assistant 행(single)은 연결선을 그리지 않는다(dot 만)', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'solo', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.w-px.bg-border')).toBeNull()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeTruthy()
  })

  it('연속 assistant 행이면 연결선(w-px bg-border)이 그려진다', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'a', itemId: 0 },
      { kind: 'text', text: 'b', itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.w-px.bg-border')).toBeTruthy()
  })
})

describe('StructuredTextView dispatch (ADR-0050)', () => {
  it('text item → assistant markdown 본문으로 렌더된다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'hello **world**', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('world').tagName.toLowerCase()).toBe('strong')
    expect(screen.getByText(/hello/)).toBeTruthy()
  })

  it("structured label=user → 사용자 박스로 렌더(text 추출)", () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'please fix it' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('please fix it')).toBeTruthy()
  })

  it('structured label=user 가 tool_result 면 독립 렌더하지 않는다(도구 OUT 에 흡수)', () => {
    const items: StructuredItem[] = [
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_1', content: 'RESULT_BODY' }),
        itemId: 0,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByText('RESULT_BODY')).toBeNull()
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  it('structured label=thinking(내용 있음) → ThoughtRow(제목 토글)로 렌더되고, 클릭하면 본문이 펼쳐진다', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: 'let me reason' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    const toggle = screen.getByRole('button', { name: /Thought/ })
    expect(toggle).toBeTruthy()
    expect(screen.queryByText('let me reason')).toBeNull()
    fireEvent.click(toggle)
    expect(screen.getByText('let me reason')).toBeTruthy()
  })

  // 빈 thinking = 암호화 thinking(opus 는 signature 만 emit). rowKindOf 도 'skip' 으로 동기화해 rail 계산과
  //   DOM 이 일치(ADR-0051).
  it('빈 thinking(공백/누락) → 아무 행도 렌더하지 않는다(빈 "Thought" 클러터 제거)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: '   ' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByText('Thought')).toBeNull()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeNull()
    expect(container.querySelector('.relative.flex.px-4')).toBeNull()
  })

  it('structured 기타 label → 접힘 generic 블록(label 헤더 토글)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'mystery', json: '{"a":1}', itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    const toggle = screen.getByRole('button', { name: /mystery/ })
    expect(toggle.getAttribute('aria-expanded')).toBe('false')
    fireEvent.click(toggle)
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
  })

  it('tool item → 이름+힌트 헤더(접힘), 클릭하면 IN(args) 상세가 펼쳐진다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', category: 'Read', resultMark: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const header = screen.getByRole('button', { name: /Read/ })
    expect(header.getAttribute('aria-expanded')).toBe('false')
    expect(container.querySelector('pre')).toBeNull()
    fireEvent.click(header)
    expect(header.getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByText('In')).toBeTruthy()
    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    expect(pre?.textContent).toContain('a.ts')
  })

  it('tool item 이 매칭 tool_result 를 가지면 펼침 시 OUT 결과를 함께 그린다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_9', category: 'Command', resultMark: null, itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_9', content: 'FILE_LISTING' }),
        itemId: 1,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const header = screen.getByRole('button', { name: /Bash/ })
    fireEvent.click(header)
    expect(screen.getByText('Out')).toBeTruthy()
    const pres = Array.from(container.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('FILE_LISTING'))).toBe(true)
  })

  it('usage item → 아무것도 렌더하지 않는다(메시지별 토큰 칩 미표시)', () => {
    const items: StructuredItem[] = [{ kind: 'usage', inputTokens: 2, outputTokens: 5, itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByText(/in 2/)).toBeNull()
    expect(screen.queryByText(/out 5/)).toBeNull()
    expect(container.querySelector('.relative.px-4')).toBeNull()
  })

  it('error item → 붉은 에러 행(메시지 노출)', () => {
    const items: StructuredItem[] = [{ kind: 'error', message: 'boom happened', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('boom happened')).toBeTruthy()
  })

  // ── 턴 결말 셋은 화면에서 서로 구별된다(그 구별이 결말 칸의 존재 이유) ──────────────────
  it('outcome=failed → 실패 문구 + 사유(detail) 노출', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'failed', detail: 'model refused', itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnFailed'))).toBeTruthy()
    expect(screen.getByText('model refused')).toBeTruthy()
  })

  it('outcome=failed 사유가 없으면 사유 줄을 만들지 않는다(빈 줄 금지)', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'failed', detail: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnFailed'))).toBeTruthy()
    expect(container.querySelector('.whitespace-pre-wrap')).toBeNull()
  })

  it('outcome=interrupted 는 실패로 그려지지 않는다(붉은 톤·실패 문구 없음)', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'interrupted', detail: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnInterrupted'))).toBeTruthy()
    expect(screen.queryByText(t('chat.turnFailed'))).toBeNull()
    expect(container.querySelector('.text-red-500')).toBeNull()
    // rail 점도 실패 톤이 아니다(error 는 bg-red-500 점을 쓴다).
    expect(container.querySelector('.rounded-full.bg-red-500')).toBeNull()
  })

  it('outcome=unknown 은 실패가 아니라 「모름」으로 그려진다', () => {
    const items: StructuredItem[] = [
      { kind: 'outcome', outcome: 'unknown', detail: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.turnUnknown'))).toBeTruthy()
    expect(container.querySelector('.text-red-500')).toBeNull()
  })

  it('결말 셋은 서로 다른 문구를 쓴다(한 모양으로 뭉개지지 않는다)', () => {
    const labels = [t('chat.turnFailed'), t('chat.turnInterrupted'), t('chat.turnUnknown')]
    expect(new Set(labels).size).toBe(3)
  })

  // ── 모르는 이벤트 표식 — 보이되 프로토콜 낱말은 안 보인다 ─────────────────────────────
  it('unsupported item → 건수를 담은 고지 한 줄(원본 이름·JSON 없음)', () => {
    const items: StructuredItem[] = [{ kind: 'unsupported', count: 3, itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.getByText(t('chat.unsupportedEvent', { count: '3' }))).toBeTruthy()
    // 펼칠 것도 없다 — GenericItemRow(JSON 코드블록)로 새지 않는다.
    expect(container.querySelectorAll('button').length).toBe(0)
    expect(container.querySelector('pre')).toBeNull()
  })

  it('separator item → 옅은 세로 스페이서(border-t divider 없음)', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'a', itemId: 0 },
      { kind: 'separator', itemId: 1 },
      { kind: 'text', text: 'b', itemId: 2 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('div[aria-hidden].border-t')).toBeNull()
    const spacer = container.querySelector('div[aria-hidden]')
    expect(spacer).toBeTruthy()
    expect(spacer?.className).toContain('h-3')
  })

  // 사용자 결정 2026-10-01: 대기 표시는 목록 밖 입력창 위 줄(RichSlot · `chat/WaitRow.tsx`)의 몫이다.
  it('streaming 이어도 목록 안에 대기 표시를 그리지 않고, 마지막 행이 그쪽으로 레일을 잇지 않는다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'working', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} streaming />)
    expect(screen.queryByText('Wait')).toBeNull()
    expect(container.querySelector('[data-wait-strip]')).toBeNull()
    expect(container.querySelector('.w-px.bg-border')).toBeNull() // 한 행 = single — 연결선 없음
  })

  // ── 마지막 행 아래 여백 — 턴 구분선과 같은 높이(h-3) 블록이 정확히 하나 ──────────────────────
  const bottomSpacers = (c: HTMLElement) => c.querySelectorAll('[data-chat-bottom-spacer]')
  const lastChildClass = (c: HTMLElement) => (c.firstElementChild as HTMLElement).lastElementChild?.className ?? ''

  it('끝이 턴 구분선이 아니면 여백 블록(h-3)을 하나 붙인다 — 오류 · 모르는 사건 행이 끝이어도', () => {
    for (const last of [
      { kind: 'text', text: 'working', itemId: 1 },
      { kind: 'error', message: 'overloaded [willRetry]', itemId: 1 },
      { kind: 'unsupported', count: 2, itemId: 1 },
    ] as StructuredItem[]) {
      const { container } = render(<StructuredTextView items={[{ kind: 'text', text: 'a', itemId: 0 }, last]} streaming />)
      expect(bottomSpacers(container)).toHaveLength(1)
      expect(lastChildClass(container)).toContain('h-3')
      cleanup()
    }
  })

  it('끝이 턴 구분선이면(뒤에 그리지 않는 행만 있어도) 여백 블록을 따로 두지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'done', itemId: 0 },
      { kind: 'separator', itemId: 1 },
      { kind: 'usage', inputTokens: 2, outputTokens: 5, itemId: 2 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(bottomSpacers(container)).toHaveLength(0)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(1)
  })

  it('그리는 행이 없으면 여백 블록도 없다', () => {
    const { container } = render(<StructuredTextView items={[]} streaming />)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(0)
  })

  // ★턴 끝에 목록 높이가 들썩이지 않는다★ — 여백 블록이 구분선과 자리를 바꿀 뿐, 끝의 h-3 은 하나 그대로다.
  it('턴이 끝나 구분선이 붙어도 끝의 h-3 은 정확히 하나다', () => {
    const reply: StructuredItem = { kind: 'text', text: 'reply', itemId: 0 }
    const { container, rerender } = render(<StructuredTextView items={[reply]} streaming />)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(1)
    rerender(<StructuredTextView items={[reply, { kind: 'separator', itemId: 1 }]} />)
    expect(container.querySelectorAll('div[aria-hidden].h-3')).toHaveLength(1)
    expect(bottomSpacers(container)).toHaveLength(0)
    expect(lastChildClass(container)).toContain('h-3')
  })

  it('malformed json 이 와도 throw 하지 않고 폴백 렌더한다(안전 파서)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: '{bad json', itemId: 0 },
    ]
    // extractText 폴백 = raw json → 비어있지 않으므로 ThoughtRow(인터랙티브 "Thought")가 뜬다(throw 없이).
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    expect(screen.getByRole('button', { name: /Thought/ })).toBeTruthy()
  })

  // ── FIX 1: tool_result 흡수는 label 무관 ──────────────────────────────────────────
  it('structured tool_result 가 NON-user label 이어도 독립 렌더하지 않는다(label 무관 흡수 — FIX 1)', () => {
    const items: StructuredItem[] = [
      {
        kind: 'structured',
        label: 'mystery',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_x', content: 'HIDDEN_BODY' }),
        itemId: 0,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 이전 버그: user 분기 밖 tool_result 가 GenericItemRow 로 standalone 렌더됐다.
    expect(screen.queryByText('HIDDEN_BODY')).toBeNull()
    expect(screen.queryByRole('button', { name: /mystery/ })).toBeNull()
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  // ── tool id=null: OUT 없이 안전 렌더 ──────────────────────────────────────────────
  it('tool item 이 id=null 이면 OUT 블록 없이 name/hint 만 렌더하고 crash 하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Glob', argsJson: '{"pattern":"**/*.ts"}', id: null, category: 'Search', resultMark: null, itemId: 0 },
    ]
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    const header = screen.getByRole('button', { name: /Glob/ })
    fireEvent.click(header)
    expect(screen.getByText('In')).toBeTruthy()
    expect(screen.queryByText('Out')).toBeNull()
  })

  // ── 매칭 안 되는 tool_result: standalone 렌더 안 함 ───────────────────────────────
  it('id 가 어떤 tool 과도 매칭되지 않는 tool_result 는 standalone 렌더하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', category: 'Read', resultMark: null, itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_ORPHAN', content: 'ORPHAN_BODY' }),
        itemId: 1,
      },
    ]
    render(<StructuredTextView items={items} />)
    expect(screen.queryByText('ORPHAN_BODY')).toBeNull()
    // 유일한 button = 매칭 없는 tool 헤더(Read) 하나뿐(고아 tool_result 는 button 을 만들지 않음).
    expect(screen.getAllByRole('button').length).toBe(1)
  })

  // ── malformed json 폴백(throw 금지) — tool args + generic json ────────────────────
  it('malformed argsJson·generic json 이 와도 폴백 렌더하고 throw 하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bad', argsJson: '{not valid', id: 'tu_2', category: 'Other', resultMark: null, itemId: 0 },
      { kind: 'structured', label: 'weird', json: '{also bad', itemId: 1 },
    ]
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    fireEvent.click(screen.getByRole('button', { name: /Bad/ }))
    const toolPre = document.querySelector('pre')
    expect(toolPre?.textContent).toContain('{not valid')
    fireEvent.click(screen.getByRole('button', { name: /weird/ }))
    const pres = Array.from(document.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('{also bad'))).toBe(true)
  })

  // ── FIX 2: 도구 OUT 의 삼중 백틱은 inert(마크다운 승격 금지) ─────────────────────
  it('도구 OUT 에 삼중 백틱+마크다운이 있어도 inert 하다(heading 등 마크다운 요소 미생성 — FIX 2)', () => {
    const evil = '```\n# NOT_A_HEADING\n[link](http://evil.example)\n```'
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Cat', argsJson: '{"path":"x"}', id: 'tu_3', category: 'Other', resultMark: null, itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_3', content: evil }),
        itemId: 1,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    fireEvent.click(screen.getByRole('button', { name: /Cat/ }))
    expect(container.querySelector('h1')).toBeNull()
    expect(container.querySelector('a')).toBeNull()
    const pres = Array.from(container.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('# NOT_A_HEADING'))).toBe(true)
    expect(pres.some((p) => p.textContent?.includes('[link](http://evil.example)'))).toBe(true)
  })

  // ── ADR-0050: dot-rail 스켈레톤 구조 ───────────────────────────────────────────────
  it('점선(border-dashed) 레일 대신 dot-rail 골격을 쓴다', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'hi', itemId: 0 },
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', category: 'Read', resultMark: null, itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.border-dashed')).toBeNull()
    // rail 행은 ChatRow 래퍼(relative flex px-4)로 감싸진다 — top-padding 은 CSS 변수 inline style(ADR-0051).
    expect(container.querySelector('.relative.flex.px-4')).toBeTruthy()
  })

  it('assistant-side 행(text)은 좌측 rail gutter + 점 마커를 렌더한다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'hello', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    // rail 모드 래퍼는 flex 행(relative flex px-4) — top-padding 은 CSS 변수 inline style(ADR-0051).
    const row = container.querySelector('.relative.flex.px-4')
    expect(row).toBeTruthy()
    expect(row?.className).toContain('flex')
    const dot = container.querySelector('.rounded-full.bg-muted')
    expect(dot).toBeTruthy()
    // 콘텐츠 컬럼은 flex-1 min-w-0(긴 토큰 오버플로 방지).
    expect(container.querySelector('.flex-1.min-w-0')).toBeTruthy()
    // ※연결선(w-px bg-border)은 run 길이에 따라 조건부다(single=없음) — 별도 clean-ends 테스트 참조.
  })

  it('rail 점 색 = 행 종류: tool 은 초록(bg-green-500), 추론/본문은 muted', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'hi', itemId: 0 },
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', category: 'Command', resultMark: null, itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.rounded-full.bg-green-500')).toBeTruthy()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeTruthy()
  })

  it('user 버블 행은 rail gutter/점 마커가 없다(plain full-width)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'ping' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(container.querySelector('.rounded-full.bg-muted')).toBeNull()
    expect(container.querySelector('.flex-1.min-w-0')).toBeNull()
    // outer 래퍼는 여전히 relative px-4(flex 아님) — top-padding 은 CSS 변수 inline style(ADR-0051).
    const row = container.querySelector('.relative.px-4')
    expect(row).toBeTruthy()
    expect(row?.className).not.toContain('flex')
  })

  it('structured label=user → 확장 룩 버블(rounded-[0.75rem] border bg-elevated, 인셋 마진)으로 렌더', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'do the thing' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const bubble = screen.getByText('do the thing')
    // bg-elevated = 다크에서 페이지보다 한 단계 밝은 배경(가시성).
    expect(bubble.className).toContain('rounded-[0.75rem]')
    expect(bubble.className).toContain('border')
    expect(bubble.className).toContain('bg-elevated')
    expect((bubble as HTMLElement).style.marginLeft).toBe('0.75rem')
    expect((bubble as HTMLElement).style.marginRight).toBe('0.75rem')
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  it('tool item → 헤더(아이콘 + bold 이름) + bg-surface 박스로 렌더', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', category: 'Command', resultMark: null, itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const title = screen.getByText('Bash')
    expect(title.className).toContain('font-bold')
    expect(container.querySelector('.bg-surface.rounded-sm')).toBeTruthy()
  })
})

// ── ADR-0239 · ADR-0241: 도구 묶음 · 끝 표식 (TRD S21-chat-ux §4-6 · §4-7 ⑨ 프론트) ──────────────

let nextToolItemId = 1000
function toolItem(
  name: string,
  id: string | null,
  category: ToolCategory,
  extra: Partial<ToolItem> = {},
): ToolItem {
  return {
    kind: 'tool',
    name,
    argsJson: '{"path":"a.ts"}',
    id,
    category,
    resultMark: null,
    itemId: nextToolItemId++,
    ...extra,
  }
}
/** claude 가 결과 본문을 싣는 운반 행 — 그리지 않는다(skip). */
function resultCarrier(toolUseId: string, isError: boolean): StructuredItem {
  return {
    kind: 'structured',
    label: 'user',
    json: JSON.stringify({ type: 'tool_result', tool_use_id: toolUseId, content: 'out', is_error: isError }),
    itemId: nextToolItemId++,
  }
}
const textItem = (s: string): StructuredItem => ({ kind: 'text', text: s, itemId: nextToolItemId++ })
const thoughtItem = (s: string): StructuredItem => ({
  kind: 'structured',
  label: 'thinking',
  json: JSON.stringify({ thinking: s }),
  itemId: nextToolItemId++,
})
const interruptedOutcome = (): StructuredItem => ({
  kind: 'outcome',
  outcome: 'interrupted',
  detail: null,
  itemId: nextToolItemId++,
})

const groups = (c: HTMLElement) => Array.from(c.querySelectorAll<HTMLElement>('[data-tool-group]'))
const groupHeader = (g: HTMLElement) => g.querySelector('button') as HTMLButtonElement
const railRows = (c: HTMLElement) => c.querySelectorAll('.relative.flex.px-4').length
const railDots = (c: HTMLElement) => c.querySelectorAll('.rounded-full').length
const summary = (...parts: string[]) => parts.join(' · ')

describe('StructuredTextView 도구 묶음 (ADR-0239)', () => {
  it('도구 하나는 묶지 않는다 — 오늘 그 행 그대로', () => {
    const { container } = render(
      <StructuredTextView items={[toolItem('Read', 'r1', 'Read'), textItem('done')]} />,
    )
    expect(groups(container)).toEqual([])
    expect(screen.getByRole('button', { name: 'Read' })).toBeTruthy()
  })

  it('도구 둘 이상 = 묶음 하나 · 운반 행 · 빈 생각이 끼어도 이어진다 · DOM 표지', () => {
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      resultCarrier('r1', false),
      thoughtItem('   '),
      toolItem('Grep', 'g1', 'Search'),
      textItem('answer'),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const [g] = groups(container)
    expect(groups(container)).toHaveLength(1)
    expect(g.getAttribute('data-tool-group')).toBe('tool:r1')
    expect(g.getAttribute('data-tool-group-open')).toBe('0')
    expect(g.getAttribute('data-tool-group-count')).toBe('2')
    expect(groupHeader(g).textContent).toBe(
      summary(t('chat.toolGroupSearch', { count: '1' }), t('chat.toolGroupRead', { count: '1' })),
    )
    // 접힌 묶음은 멤버 행을 그리지 않는다.
    expect(screen.queryByRole('button', { name: 'Read' })).toBeNull()
  })

  it('첫 호출에 백엔드 id 가 없으면 키 = item:<itemId>', () => {
    const first = toolItem('Read', null, 'Read')
    const { container } = render(<StructuredTextView items={[first, toolItem('Read', 'r2', 'Read')]} />)
    expect(groups(container)[0].getAttribute('data-tool-group')).toBe(`item:${first.itemId}`)
  })

  it('레일은 행 목록으로 센다 — 묶음 = 레일 한 행 · 펼쳐도 멤버는 레일을 그리지 않는다(ADR-0051)', () => {
    const items: StructuredItem[] = [
      textItem('before'),
      toolItem('Read', 'r1', 'Read'),
      toolItem('Read', 'r2', 'Read'),
      toolItem('Bash', 'b1', 'Command'),
      textItem('after'),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 글 · 묶음 · 글 = 세 행 — 셋이 이어져 연결선도 셋(top · mid · bottom).
    expect(railRows(container)).toBe(3)
    expect(railDots(container)).toBe(3)
    expect(container.querySelectorAll('.w-px.bg-border').length).toBe(3)

    useToolGroupStore.getState().bind('s1', 'agent-1')
    cleanup()
    const expanded = render(<StructuredTextView items={items} slotId="s1" />)
    fireEvent.click(groupHeader(groups(expanded.container)[0]))
    expect(screen.getAllByRole('button', { name: 'Read' })).toHaveLength(2)
    expect(screen.getByRole('button', { name: 'Bash' })).toBeTruthy()
    expect(railRows(expanded.container)).toBe(3)
    expect(railDots(expanded.container)).toBe(3)
  })

  it('펼친 묶음 [도구 · 생각 · 도구] — 생각 행이 묶음 안에 그려지고 레일은 묶음을 한 행으로 센다', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items: StructuredItem[] = [
      textItem('before'),
      toolItem('Read', 'r1', 'Read'),
      thoughtItem('mid thought'),
      toolItem('Grep', 'g1', 'Search'),
      textItem('after'),
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    fireEvent.click(groupHeader(g))
    expect(g.getAttribute('data-tool-group-open')).toBe('1')
    expect(g.contains(screen.getByText('Thought'))).toBe(true)
    expect(g.contains(screen.getByRole('button', { name: 'Read' }))).toBe(true)
    expect(g.contains(screen.getByRole('button', { name: 'Grep' }))).toBe(true)
    expect(railRows(container)).toBe(3)
    expect(railDots(container)).toBe(3)
  })

  // ADR-0263 결정 1: 도는 묶음은 통째로 펼치지 않는다 — 머리(진행형) + 지금 도는 마지막 호출 한 줄, 끝나면 머리만(과거형).
  const readCall = (id: string) => toolItem('Read', id, 'Read', { argsJson: JSON.stringify({ path: `${id}.ts` }) })
  const shownHints = () => screen.queryAllByText(/^r\d\.ts$/).map((el) => el.textContent)

  it('도는 턴의 마지막 묶음은 머리 + 마지막 호출 한 줄 · 호출이 늘면 그 줄이 바뀌고 · 글이 오면 머리만', () => {
    const calls = [readCall('r1'), readCall('r2')]
    const { container, rerender } = render(<StructuredTextView items={calls} streaming />)
    const g = groups(container)[0]
    const head = groupHeader(g)
    const live = () => groups(container)[0].getAttribute('data-tool-group-live')
    expect(g.getAttribute('data-tool-group-open')).toBe('0')
    expect(live()).toBe('1')
    expect(screen.getByText(t('chat.toolGroupRunning'))).toBeTruthy()
    expect(shownHints()).toEqual(['r2.ts'])

    const more = [...calls, thoughtItem('hmm'), readCall('r3')]
    rerender(<StructuredTextView items={more} streaming />)
    expect(groups(container)[0]).toBe(g) // 같은 노드에서 수만 는다
    expect(groupHeader(g)).toBe(head)
    expect(g.getAttribute('data-tool-group-count')).toBe('3')
    expect(g.getAttribute('data-tool-group-open')).toBe('0')
    expect(shownHints()).toEqual(['r3.ts'])

    rerender(<StructuredTextView items={[...more, textItem('here')]} streaming />)
    expect(groups(container)[0]).toBe(g)
    expect(live()).toBe('0')
    expect(screen.queryByText(t('chat.toolGroupRunning'))).toBeNull()
    expect(shownHints()).toEqual([])
  })

  it('턴이 끝나면 꼬리 묶음도 머리만 — 과거형', () => {
    const calls = [readCall('r1'), readCall('r2')]
    const { container, rerender } = render(<StructuredTextView items={calls} streaming />)
    rerender(<StructuredTextView items={calls} />)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
    expect(groups(container)[0].getAttribute('data-tool-group-live')).toBe('0')
    expect(shownHints()).toEqual([])
  })

  it('사람 토글이 이긴다 — 도는 중 펼치면 전부 보이며 자라고 턴 끝에도 그대로 · 도는 중 접으면 머리만', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const calls = [readCall('r1'), readCall('r2')]
    const { container, rerender } = render(<StructuredTextView items={calls} streaming slotId="s1" />)
    fireEvent.click(groupHeader(groups(container)[0]))
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('1')
    expect(shownHints()).toEqual(['r1.ts', 'r2.ts'])

    const more = [...calls, readCall('r3')]
    rerender(<StructuredTextView items={more} streaming slotId="s1" />)
    expect(shownHints()).toEqual(['r1.ts', 'r2.ts', 'r3.ts'])

    fireEvent.click(groupHeader(groups(container)[0]))
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
    expect(shownHints()).toEqual([]) // 도는 중이어도 고른 접힘은 마지막 호출 줄까지 걷는다
    fireEvent.click(groupHeader(groups(container)[0]))

    const ended = [...more, textItem('answer')]
    rerender(<StructuredTextView items={ended} slotId="s1" />)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('1')
    expect(shownHints()).toEqual(['r1.ts', 'r2.ts', 'r3.ts'])
    fireEvent.click(groupHeader(groups(container)[0]))
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
    rerender(<StructuredTextView items={[...ended]} slotId="s1" />)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
  })

  it('같은 사건열을 새 누산기로 다시 먹여도(replay) 고른 펼침이 그대로 붙는다', () => {
    const events = [
      { type: 'ToolCall', name: 'Read', args_json: '{}', id: 'r1', turn_id: null, message_id: null, category: 'Read' },
      { type: 'ToolCall', name: 'Read', args_json: '{}', id: 'r2', turn_id: null, message_id: null, category: 'Read' },
      { type: 'TextDelta', text: 'answer', turn_id: null, message_id: null },
    ]
    const snapshotOf = () => {
      const acc = new StructuredEventAccumulator()
      for (const ev of events) acc.feed(new TextEncoder().encode(JSON.stringify(ev as StructuredEvent)))
      return acc.snapshot()
    }
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container, rerender } = render(<StructuredTextView items={snapshotOf()} slotId="s1" />)
    fireEvent.click(groupHeader(groups(container)[0]))
    rerender(<StructuredTextView items={snapshotOf()} slotId="s1" />)
    expect(groups(container)[0].getAttribute('data-tool-group')).toBe('tool:r1')
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('1')
  })

  it('slotId 가 없으면 늘 접힌 채이고 머리를 눌러도 바뀌지 않는다', () => {
    const calls = [toolItem('Read', 'r1', 'Read'), toolItem('Read', 'r2', 'Read')]
    const { container } = render(<StructuredTextView items={calls} />)
    const header = groupHeader(groups(container)[0])
    expect(header.disabled).toBe(true)
    fireEvent.click(header)
    expect(groups(container)[0].getAttribute('data-tool-group-open')).toBe('0')
  })

  it('펼침 알림 — 마지막이 아닌 묶음은 false · 마지막 묶음은 true', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const onGroupToggle = vi.fn()
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      toolItem('Read', 'r2', 'Read'),
      textItem('middle'),
      toolItem('Bash', 'b1', 'Command'),
      toolItem('Bash', 'b2', 'Command'),
    ]
    const { container } = render(
      <StructuredTextView items={items} slotId="s1" onGroupToggle={onGroupToggle} />,
    )
    const [first, last] = groups(container)
    fireEvent.click(groupHeader(first))
    fireEvent.click(groupHeader(last))
    expect(onGroupToggle.mock.calls).toEqual([[false], [true]])
  })
})

describe('StructuredTextView 도구 끝 표식 (ADR-0241)', () => {
  it('codex 실패(resultMark failed) → 붉은 Error 배지 · data-tool-mark="failed" · Out 칸 없음', () => {
    const { container } = render(
      <StructuredTextView items={[toolItem('commandExecution', 'c1', 'Command', { resultMark: 'failed' })]} />,
    )
    const row = container.querySelector('[data-tool-mark]') as HTMLElement
    expect(row.getAttribute('data-tool-mark')).toBe('failed')
    expect(screen.getByText('Error').className).toContain('text-red-500')
    fireEvent.click(screen.getByRole('button', { name: 'commandExecution' }))
    expect(screen.getByText('In')).toBeTruthy()
    expect(screen.queryByText('Out')).toBeNull()
  })

  it('claude 벤더 오류(is_error) → 붉은 Error 배지 · data-tool-mark 는 없다(표식이 아니다)', () => {
    const { container } = render(
      <StructuredTextView items={[toolItem('Bash', 'b1', 'Command'), resultCarrier('b1', true)]} />,
    )
    expect(screen.getByText('Error').className).toContain('text-red-500')
    expect(container.querySelector('[data-tool-mark]')).toBeNull()
  })

  it.each([
    ['refused', 'chat.toolRefusedReason'],
    ['declined', 'chat.toolDeclinedReason'],
  ] as const)('거부 행(%s) — 주황 「거부됨」 배지 · 출처별 사유 줄(접힌 채로도) · 붉은 표시 없음', (mark, reasonKey) => {
    const { container } = render(
      <StructuredTextView items={[toolItem('commandExecution', 'c1', 'Command', { resultMark: mark })]} />,
    )
    const badge = screen.getByText(t('chat.toolDeclined'))
    expect(badge.style.color).toBe('var(--status-blocked)')
    expect(badge.style.borderColor).toBe('var(--status-blocked)')
    expect(screen.queryByText('Error')).toBeNull()
    expect(container.querySelector('.text-red-500, .border-red-500, .border-red-500\\/60')).toBeNull()
    // 머리(도구 이름)는 기본 톤이다.
    expect(screen.getByText('commandExecution').className).toContain('text-foreground')

    const row = container.querySelector('[data-tool-mark]') as HTMLElement
    expect(row.getAttribute('data-tool-mark')).toBe('declined')
    const detail = screen.getByRole('button', { name: 'commandExecution' })
    expect(detail.getAttribute('aria-expanded')).toBe('false')
    const reason = container.querySelector('[data-tool-declined-reason]') as HTMLElement
    expect(reason.getAttribute('data-tool-declined-reason')).toBe(reasonKey)
    expect(reason.textContent).toBe(t(reasonKey))
  })

  it('두 출처는 배지 · 색 · data-tool-mark 가 같고 사유 키만 갈린다 — 묶음 요약 「거부 2」 · 오류 칸 없음', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items = [
      toolItem('commandExecution', 'c1', 'Command', { resultMark: 'refused' }),
      toolItem('fileChange', 'f1', 'Edit', { resultMark: 'declined' }),
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupEdit', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '2' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    const marks = Array.from(container.querySelectorAll('[data-tool-mark]')).map((el) => el.getAttribute('data-tool-mark'))
    expect(marks).toEqual(['declined', 'declined'])
    const badges = screen.getAllByText(t('chat.toolDeclined'))
    expect(badges.map((b) => b.style.color)).toEqual(['var(--status-blocked)', 'var(--status-blocked)'])
    const reasons = Array.from(container.querySelectorAll('[data-tool-declined-reason]')).map((el) =>
      el.getAttribute('data-tool-declined-reason'),
    )
    expect(reasons).toEqual(['chat.toolRefusedReason', 'chat.toolDeclinedReason'])
  })

  it('묶음 요약은 「오류 1 · 거부 1」 순서 — 거부는 오류 수에 안 든다', () => {
    const items = [
      toolItem('commandExecution', 'c1', 'Command', { resultMark: 'failed' }),
      toolItem('commandExecution', 'c2', 'Command', { resultMark: 'refused' }),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(groupHeader(groups(container)[0]).textContent).toBe(
      summary(
        t('chat.toolGroupCommand', { count: '2' }),
        t('chat.toolGroupErrors', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '1' }),
      ),
    )
  })

  it('거부 표식이 벤더 오류 본문을 이긴다 — 배지 · 머리 셈 둘 다', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      toolItem('Bash', 'b1', 'Command', { resultMark: 'declined' }),
      resultCarrier('b1', true),
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupDeclined', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    expect(screen.getByText(t('chat.toolDeclined'))).toBeTruthy()
    expect(screen.queryByText('Error')).toBeNull()
  })

  it('사유 줄은 행이 아니다 — 레일 행 · 점 수가 표식 없는 같은 목록과 같다(ADR-0051)', () => {
    const list = (mark: ToolItem['resultMark']): StructuredItem[] => [
      textItem('before'),
      toolItem('commandExecution', 'c1', 'Command', { resultMark: mark }),
      textItem('after'),
    ]
    const plain = render(<StructuredTextView items={list(null)} />)
    const plainCounts = [railRows(plain.container), railDots(plain.container)]
    cleanup()
    const marked = render(<StructuredTextView items={list('refused')} />)
    expect(marked.container.querySelector('[data-tool-declined-reason]')).toBeTruthy()
    expect([railRows(marked.container), railDots(marked.container)]).toEqual(plainCounts)
  })

  // ★TRD §11 ⑪ · §4-7 ⑨ ⓐ 고정★: 끊긴 턴의 도구 실패는 그대로 붉은 「오류」로 보이고, 바로 아래 강조된 「중단됨」
  //   행이 맥락을 준다 — 끊긴 턴 안의 실패를 따로 가르지 않는다.
  it('ⓐ claude(합성 줄 없는 경우) — 끊긴 턴 도구의 tool_result is_error → 붉은 「Error」 · 머리 「오류 1」 · 아래에 중단 행', () => {
    useToolGroupStore.getState().bind('s1', 'agent-1')
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      resultCarrier('r1', false),
      toolItem('Bash', 'b1', 'Command'),
      resultCarrier('b1', true),
      interruptedOutcome(),
      { kind: 'separator', itemId: nextToolItemId++ },
    ]
    const { container } = render(<StructuredTextView items={items} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupErrors', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    expect(screen.getByText('Error').className).toContain('text-red-500')
    expect(container.querySelector('[data-tool-mark]')).toBeNull()
    const outcome = screen.getByText(t('chat.turnInterrupted'))
    expect(g.compareDocumentPosition(outcome) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  // ADR-0243: claude 의 실제 모양 — 죽은 도구의 is_error 결과 뒤에 합성 끊김 줄이 오고, 결말 행은 없다.
  it('ⓐ claude — 끊긴 도구 is_error → 붉은 「Error」 · 머리 「오류 1」 · 아래에 끊김 표시 행 · 중단 결말 행 없음', () => {
    const acc = new StructuredEventAccumulator()
    const feed = (ev: unknown) => acc.feed(new TextEncoder().encode(JSON.stringify(ev as StructuredEvent)))
    const call = (name: string, id: string, category: ToolCategory) => ({
      type: 'ToolCall',
      name,
      args_json: '{}',
      id,
      turn_id: null,
      message_id: null,
      category,
    })
    const result = (id: string, isError: boolean) => ({
      type: 'Structured',
      kind: 'user',
      json: JSON.stringify({ type: 'tool_result', tool_use_id: id, content: 'out', is_error: isError }),
    })
    feed(call('Read', 'r1', 'Read'))
    feed(result('r1', false))
    feed(call('Bash', 'b1', 'Command'))
    feed(result('b1', true))
    feed({
      type: 'Structured',
      kind: 'interrupted',
      json: JSON.stringify({ text: '[Request interrupted by user for tool use]' }),
    })
    feed({ type: 'TurnEnd', turn_id: null, outcome: { kind: 'Interrupted' } })

    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container } = render(<StructuredTextView items={acc.snapshot()} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupErrors', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    expect(screen.getByText('Error').className).toContain('text-red-500')
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    expect(note.textContent).toBe('[Request interrupted by user for tool use]')
    expect(g.compareDocumentPosition(note) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.queryByText(t('chat.turnInterrupted'))).toBeNull()
  })

  it('ⓐ codex — 끊긴 턴 뒤 늦게 온 ToolResult{Failed} → 그 행 「Error」 · 머리 「오류 1」 · 중단 행은 그대로', () => {
    const acc = new StructuredEventAccumulator()
    const feed = (ev: unknown) => acc.feed(new TextEncoder().encode(JSON.stringify(ev as StructuredEvent)))
    const call = (id: string, category: ToolCategory) => ({
      type: 'ToolCall',
      name: 'commandExecution',
      args_json: '{}',
      id,
      turn_id: 't1',
      message_id: null,
      category,
    })
    feed(call('c1', 'Read'))
    feed(call('c2', 'Command'))
    feed({ type: 'TurnEnd', turn_id: 't1', outcome: { kind: 'Interrupted' } })
    feed({ type: 'ToolResult', id: 'c2', outcome: 'Failed' })

    useToolGroupStore.getState().bind('s1', 'agent-1')
    const { container } = render(<StructuredTextView items={acc.snapshot()} slotId="s1" />)
    const [g] = groups(container)
    expect(groupHeader(g).textContent).toBe(
      summary(
        t('chat.toolGroupRead', { count: '1' }),
        t('chat.toolGroupCommand', { count: '1' }),
        t('chat.toolGroupErrors', { count: '1' }),
      ),
    )
    fireEvent.click(groupHeader(g))
    const marks = Array.from(container.querySelectorAll('[data-tool-mark]')).map((el) => el.getAttribute('data-tool-mark'))
    expect(marks).toEqual(['failed'])
    expect(screen.getByText('Error').className).toContain('text-red-500')
    const outcome = screen.getByText(t('chat.turnInterrupted'))
    expect(g.compareDocumentPosition(outcome) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('종류가 있으면 종류 아이콘 · Other 면 이름 휴리스틱(옛 데몬 모습 그대로)', () => {
    const { container, rerender } = render(<StructuredTextView items={[toolItem('Glob', 'g1', 'Search')]} />)
    const iconOf = () => container.querySelector('.mb-3 svg')?.getAttribute('class') ?? ''
    expect(iconOf()).toContain('lucide-search')
    rerender(<StructuredTextView items={[toolItem('Glob', 'g1', 'Other')]} />)
    expect(iconOf()).toContain('lucide-folder-open')
    rerender(<StructuredTextView items={[toolItem('Task', 'a1', 'Agent')]} />)
    expect(iconOf()).toContain('lucide-bot')
    rerender(<StructuredTextView items={[toolItem('mcp__x__y', 'm1', 'Mcp')]} />)
    expect(iconOf()).toContain('lucide-plug')
  })

  it('타입 밖 종류 낱말(캐스트로 든 항목)도 렌더를 깨뜨리지 않고 이름 휴리스틱으로 간다', () => {
    const odd = toolItem('Glob', 'g1', 'Weird' as unknown as ToolCategory)
    const { container } = render(<StructuredTextView items={[odd]} />)
    expect(container.querySelector('.mb-3 svg')?.getAttribute('class')).toContain('lucide-folder-open')
  })
})

// ── ADR-0237 U8: 중단 행 강조 ────────────────────────────────────────────────────────────
describe('StructuredTextView 결말 행 톤 (ADR-0237 U8)', () => {
  const titleOf = (outcome: 'failed' | 'interrupted' | 'unknown', text: string) => {
    render(<StructuredTextView items={[{ kind: 'outcome', outcome, detail: null, itemId: 0 }]} />)
    const title = screen.getByText(text)
    return { title, line: title.parentElement as HTMLElement }
  }

  it('중단 = 굵게 + 강조색(text-accent) · 아이콘 · 문구 그대로', () => {
    const { title, line } = titleOf('interrupted', t('chat.turnInterrupted'))
    expect(title.className).toContain('font-bold')
    expect(line.className).toContain('text-accent')
    expect(line.className).not.toContain('text-muted')
    expect(line.querySelector('svg')?.getAttribute('class')).toContain('lucide-circle-stop')
  })

  it('실패 = 오늘처럼 붉은 톤 + 굵게', () => {
    const { title, line } = titleOf('failed', t('chat.turnFailed'))
    expect(title.className).toContain('font-bold')
    expect(line.className).toContain('text-red-500')
    expect(line.className).not.toContain('text-accent')
  })

  it('모름 = 오늘처럼 muted · 굵게 아님', () => {
    const { title, line } = titleOf('unknown', t('chat.turnUnknown'))
    expect(title.className).not.toContain('font-bold')
    expect(line.className).toContain('text-muted')
    expect(line.className).not.toContain('text-accent')
  })
})

// ── ADR-0243: claude 끊김 표시 행 ───────────────────────────────────────────────────────
describe('StructuredTextView 끊김 표시 행 (ADR-0243)', () => {
  const noteItem = (text: string): StructuredItem => ({ kind: 'interruptNote', text, itemId: nextToolItemId++ })

  it('원문을 글자 그대로 — 마크다운 문자도 태그가 되지 않는다', () => {
    const text = '[Request interrupted by user] **bold** `code` # h'
    const { container } = render(<StructuredTextView items={[noteItem(text)]} />)
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    expect(note.textContent).toBe(text)
    expect(note.querySelector('strong, code, h1')).toBeNull()
  })

  it('U8 중단 행과 같은 모양 — 강조색 · 굵게 · CircleStop', () => {
    const { container } = render(<StructuredTextView items={[noteItem('[Request interrupted by user]')]} />)
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    expect(note.className).toContain('text-accent')
    expect(screen.getByText('[Request interrupted by user]').className).toContain('font-bold')
    expect(note.querySelector('svg')?.getAttribute('class')).toContain('lucide-circle-stop')
  })

  it('결말 행의 중단 갈래와 줄 · 글 클래스가 같다(U8 모양이 두 행에서 갈라지지 않는다)', () => {
    const { container } = render(
      <StructuredTextView
        items={[
          noteItem('[Request interrupted by user]'),
          { kind: 'outcome', outcome: 'interrupted', detail: null, itemId: nextToolItemId++ },
        ]}
      />,
    )
    const note = container.querySelector('[data-interrupt-note]') as HTMLElement
    const outcomeLine = screen.getByText(t('chat.turnInterrupted')).parentElement as HTMLElement
    expect(note.className).toBe(outcomeLine.className)
    expect(note.querySelector('span')?.className).toBe(outcomeLine.querySelector('span')?.className)
  })

  it('레일 한 행이고 도구 묶음을 끊는다', () => {
    const items: StructuredItem[] = [
      toolItem('Read', 'r1', 'Read'),
      toolItem('Read', 'r2', 'Read'),
      noteItem('[Request interrupted by user]'),
      toolItem('Read', 'r3', 'Read'),
      toolItem('Read', 'r4', 'Read'),
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(groups(container)).toHaveLength(2)
    // 묶음 · 표시 행 · 묶음 = 세 행.
    expect(railRows(container)).toBe(3)
    expect(railDots(container)).toBe(3)
  })
})
