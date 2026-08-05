// ADR-0050: 결정적 어댑터 동작(매핑·흡수·필터)을 검증하고, leaf 내부 렌더(chat/*)는 스모크 수준만 본다
//   (react-markdown 등 세부는 leaf 자체 테스트의 몫).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { StructuredTextView } from './StructuredTextView'
import type { StructuredItem } from './structuredAccumulator'

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
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 0 },
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

  it('streaming=true 면 스트림 끝에 대기 인디케이터(WaitRow "Wait" 라벨)를 붙인다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'working', itemId: 0 }]
    render(<StructuredTextView items={items} streaming />)
    // 경과 초는 타이머 flakiness 회피로 단언 안 함.
    expect(screen.getByText('Wait')).toBeTruthy()
  })

  it('streaming=true 면 items 가 비어도 대기 인디케이터를 즉시 보여준다(showTail = streaming)', () => {
    const { container } = render(<StructuredTextView items={[]} streaming />)
    expect(screen.getByText('Wait')).toBeTruthy()
    // rail 경로 크래시 없음: kinds=['assistant'] → single, tailPos='single'(연결선 없음).
    expect(container.querySelector('.relative.flex.px-4')).toBeTruthy()
    expect(container.querySelector('.w-px.bg-border')).toBeNull()
  })

  it('streaming=false(기본)면 대기 인디케이터가 없다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'done', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.queryByText('Wait')).toBeNull()
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
      { kind: 'tool', name: 'Glob', argsJson: '{"pattern":"**/*.ts"}', id: null, itemId: 0 },
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
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 0 },
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
      { kind: 'tool', name: 'Bad', argsJson: '{not valid', id: 'tu_2', itemId: 0 },
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
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 1 },
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
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', itemId: 1 },
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
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const title = screen.getByText('Bash')
    expect(title.className).toContain('font-bold')
    expect(container.querySelector('.bg-surface.rounded-sm')).toBeTruthy()
  })
})
