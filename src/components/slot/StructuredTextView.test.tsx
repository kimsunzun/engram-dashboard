// ADR-0048: StructuredTextView dispatch 테스트 — items 스트림의 각 kind 가 기대한 Cline 이식 컴포넌트/역할로
//   매핑되는지 단언한다(순수 렌더 — 구독/누적 무관). 결정적 어댑터 동작(매핑·흡수·필터)을 검증하고, Cline
//   leaf 내부 렌더는 스모크 수준만 본다(react-markdown 등 세부는 이식물의 몫).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { StructuredTextView } from './StructuredTextView'
import type { StructuredItem } from './structuredAccumulator'

afterEach(() => cleanup())

describe('StructuredTextView dispatch (ADR-0048)', () => {
  it('text item → assistant markdown 본문으로 렌더된다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'hello **world**', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    // MarkdownRow → MarkdownBlock 이 마크다운을 렌더(bold 는 <strong>).
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
    // tool_result 는 standalone 으로 뜨지 않는다(매칭 tool 이 없으면 어디에도 안 보인다).
    expect(screen.queryByText('RESULT_BODY')).toBeNull()
    // 콘텐츠 컬럼에 아무 행도 렌더되지 않는다(레일 게터 div 만 남지 않도록 null 반환).
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  it('structured label=thinking → Cline ThinkingRow(제목 토글)로 렌더되고, 클릭하면 본문이 펼쳐진다', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: 'let me reason' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    const toggle = screen.getByRole('button', { name: /Thinking/ })
    expect(toggle).toBeTruthy()
    // 접힌 상태 — 본문은 아직 없다.
    expect(screen.queryByText('let me reason')).toBeNull()
    fireEvent.click(toggle)
    expect(screen.getByText('let me reason')).toBeTruthy()
  })

  it('빈 thinking(공백/누락)은 렌더하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: '   ' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    expect(screen.queryByRole('button', { name: /Thinking/ })).toBeNull()
    expect(container.querySelectorAll('button').length).toBe(0)
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
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const header = screen.getByRole('button', { name: /Read/ })
    expect(header.getAttribute('aria-expanded')).toBe('false')
    // 접힌 상태 — args 코드(<pre>) 상세는 아직 없다.
    expect(container.querySelector('pre')).toBeNull()
    fireEvent.click(header)
    expect(header.getAttribute('aria-expanded')).toBe('true')
    // 펼치면 IN 라벨 + args JSON(InertCode → 리터럴 <pre>)이 보인다(FIX 2: 마크다운 파싱 안 함).
    expect(screen.getByText('In')).toBeTruthy()
    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    expect(pre?.textContent).toContain('a.ts')
  })

  it('tool item 이 매칭 tool_result 를 가지면 펼침 시 OUT 결과를 함께 그린다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_9', itemId: 0 },
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
    // OUT 결과 본문은 InertCode 리터럴 <pre> 안에 그대로 렌더된다(FIX 2).
    const pres = Array.from(container.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('FILE_LISTING'))).toBe(true)
  })

  it('usage item → muted 토큰 칩(in/out 표기)', () => {
    const items: StructuredItem[] = [{ kind: 'usage', inputTokens: 10, outputTokens: 5, itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText(/in 10 · out 5/)).toBeTruthy()
  })

  it('error item → 붉은 에러 행(메시지 노출)', () => {
    const items: StructuredItem[] = [{ kind: 'error', message: 'boom happened', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.getByText('boom happened')).toBeTruthy()
  })

  it('separator item → full-width divider(border-top div)', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'a', itemId: 0 },
      { kind: 'separator', itemId: 1 },
      { kind: 'text', text: 'b', itemId: 2 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // separator 는 aria-hidden border-t div 로 렌더된다.
    expect(container.querySelector('div[aria-hidden].border-t')).toBeTruthy()
  })

  it('streaming=true 면 스트림 끝에 Thinking 라이브 신호(제목)를 붙인다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'working', itemId: 0 }]
    render(<StructuredTextView items={items} streaming />)
    // isStreaming ThinkingRow — 제목 "Thinking" 이 보인다.
    expect(screen.getByText('Thinking')).toBeTruthy()
  })

  it('streaming=false(기본)면 라이브 Thinking 신호가 없다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'done', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.queryByText('Thinking')).toBeNull()
  })

  it('malformed json 이 와도 throw 하지 않고 폴백 렌더한다(안전 파서)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: '{bad json', itemId: 0 },
    ]
    // extractText 폴백 = raw json → 비어있지 않으므로 ThinkingRow 가 뜬다(throw 없이).
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    expect(screen.getByRole('button', { name: /Thinking/ })).toBeTruthy()
  })

  // ── FIX 1: tool_result 흡수는 label 무관 ──────────────────────────────────────────
  it('structured tool_result 가 NON-user label 이어도 독립 렌더하지 않는다(label 무관 흡수 — FIX 1)', () => {
    const items: StructuredItem[] = [
      {
        kind: 'structured',
        label: 'mystery', // user 가 아닌 label
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_x', content: 'HIDDEN_BODY' }),
        itemId: 0,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 이전 버그: user 분기 밖 tool_result 가 GenericItemRow 로 standalone 렌더됐다. 이제 흡수되어 아무것도 안 뜬다.
    expect(screen.queryByText('HIDDEN_BODY')).toBeNull()
    expect(screen.queryByRole('button', { name: /mystery/ })).toBeNull()
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  // ── tool id=null: OUT 없이 안전 렌더 ──────────────────────────────────────────────
  it('tool item 이 id=null 이면 OUT 블록 없이 name/hint 만 렌더하고 crash 하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Glob', argsJson: '{"pattern":"**/*.ts"}', id: null, itemId: 0 },
    ]
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    const header = screen.getByRole('button', { name: /Glob/ })
    fireEvent.click(header)
    // IN 은 있고 OUT 은 없다(id=null → results.get 조회 자체를 안 함).
    expect(screen.getByText('In')).toBeTruthy()
    expect(screen.queryByText('Out')).toBeNull()
  })

  // ── 매칭 안 되는 tool_result: standalone 렌더 안 함 ───────────────────────────────
  it('id 가 어떤 tool 과도 매칭되지 않는 tool_result 는 standalone 렌더하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_ORPHAN', content: 'ORPHAN_BODY' }),
        itemId: 1,
      },
    ]
    render(<StructuredTextView items={items} />)
    // 고아 tool_result 는 흡수 규칙상 어디에도 안 그린다(매칭 tool 없음). 펼치기 전이므로 매칭 tool OUT 도 미노출.
    expect(screen.queryByText('ORPHAN_BODY')).toBeNull()
    // 유일한 button = 매칭 없는 tool 헤더(Read) 하나뿐(고아 tool_result 는 button 을 만들지 않음).
    expect(screen.getAllByRole('button').length).toBe(1)
  })

  // ── malformed json 폴백(throw 금지) — tool args + generic json ────────────────────
  it('malformed argsJson·generic json 이 와도 폴백 렌더하고 throw 하지 않는다', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bad', argsJson: '{not valid', id: 'tu_2', itemId: 0 },
      { kind: 'structured', label: 'weird', json: '{also bad', itemId: 1 },
    ]
    expect(() => render(<StructuredTextView items={items} />)).not.toThrow()
    // tool 헤더 펼치면 pretty() 폴백(raw 원문)이 <pre> 로 뜬다.
    fireEvent.click(screen.getByRole('button', { name: /Bad/ }))
    const toolPre = document.querySelector('pre')
    expect(toolPre?.textContent).toContain('{not valid')
    // generic 헤더 펼치면 pretty() 폴백 raw json 이 <pre> 로 뜬다.
    fireEvent.click(screen.getByRole('button', { name: /weird/ }))
    const pres = Array.from(document.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('{also bad'))).toBe(true)
  })

  // ── FIX 2: 도구 OUT 의 삼중 백틱은 inert(마크다운 승격 금지) ─────────────────────
  it('도구 OUT 에 삼중 백틱+마크다운이 있어도 inert 하다(heading 등 마크다운 요소 미생성 — FIX 2)', () => {
    const evil = '```\n# NOT_A_HEADING\n[link](http://evil.example)\n```'
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Cat', argsJson: '{"path":"x"}', id: 'tu_3', itemId: 0 },
      {
        kind: 'structured',
        label: 'user',
        json: JSON.stringify({ type: 'tool_result', tool_use_id: 'tu_3', content: evil }),
        itemId: 1,
      },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    fireEvent.click(screen.getByRole('button', { name: /Cat/ }))
    // 마크다운 파싱이 일어났다면 <h1>·<a> 가 생긴다 — inert 이므로 생기지 않는다.
    expect(container.querySelector('h1')).toBeNull()
    expect(container.querySelector('a')).toBeNull()
    // 내용은 리터럴 텍스트로 그대로 보존된다(<pre> 안).
    const pres = Array.from(container.querySelectorAll('pre'))
    expect(pres.some((p) => p.textContent?.includes('# NOT_A_HEADING'))).toBe(true)
    expect(pres.some((p) => p.textContent?.includes('[link](http://evil.example)'))).toBe(true)
  })
})
