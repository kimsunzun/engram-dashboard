// ADR-0050: StructuredTextView dispatch 테스트 — items 스트림의 각 kind 가 기대한 자체 채팅 컴포넌트/역할로
//   매핑되는지 단언한다(순수 렌더 — 구독/누적 무관). 결정적 어댑터 동작(매핑·흡수·필터)을 검증하고, leaf
//   내부 렌더(chat/*)는 스모크 수준만 본다(react-markdown 등 세부는 leaf 자체 테스트의 몫).

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import {
  computeRailRunPositions,
  StructuredTextView,
  type ChatRowKind,
} from './StructuredTextView'
import type { StructuredItem } from './structuredAccumulator'

afterEach(() => cleanup())

// ── ADR-0051: rail run-position 순수 함수 ──────────────────────────────────────────
describe('computeRailRunPositions (ADR-0051 dot-rail clean-ends)', () => {
  it('assistant 한 행만 있으면 single(고립 dot — 연결선 없음)', () => {
    expect(computeRailRunPositions(['assistant'])).toEqual(['single'])
  })

  it('연속 assistant 3행 → top/mid/bottom', () => {
    const kinds: ChatRowKind[] = ['assistant', 'assistant', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', 'mid', 'bottom'])
  })

  it('boundary(user 버블/separator)가 run 을 끊는다', () => {
    // a a | boundary | a  →  두 run: [top,bottom] 과 [single]. boundary 는 null.
    const kinds: ChatRowKind[] = ['assistant', 'assistant', 'boundary', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', 'bottom', null, 'single'])
  })

  it('skip(usage/흡수 tool_result)은 run 을 끊지 않는다(DOM 없음 → 시각적 인접)', () => {
    // a skip a  →  skip 을 무시하면 두 assistant 는 인접 run → top/bottom. skip 은 null.
    const kinds: ChatRowKind[] = ['assistant', 'skip', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual(['top', null, 'bottom'])
  })

  it('boundary 로 시작/끝나는 혼합 시퀀스', () => {
    // boundary a a boundary a a a
    const kinds: ChatRowKind[] = [
      'boundary',
      'assistant',
      'assistant',
      'boundary',
      'assistant',
      'assistant',
      'assistant',
    ]
    expect(computeRailRunPositions(kinds)).toEqual([
      null,
      'top',
      'bottom',
      null,
      'top',
      'mid',
      'bottom',
    ])
  })

  it('전부 skip 이면 위치 없음(all null)', () => {
    expect(computeRailRunPositions(['skip', 'skip'])).toEqual([null, null])
  })

  it('선행 skip 은 top 을 무너뜨리지 않는다(맨 앞 assistant 는 여전히 top)', () => {
    const kinds: ChatRowKind[] = ['skip', 'assistant', 'assistant']
    expect(computeRailRunPositions(kinds)).toEqual([null, 'top', 'bottom'])
  })
})

// ── ADR-0051: rail 연결선 clean-ends 렌더 ──────────────────────────────────────────
describe('StructuredTextView rail line clean-ends (ADR-0051)', () => {
  it('단일 assistant 행(single)은 연결선을 그리지 않는다(dot 만)', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'solo', itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    // single = 연결선 span(w-px bg-border) 없음. dot(rounded-full)은 있다.
    expect(container.querySelector('.w-px.bg-border')).toBeNull()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeTruthy()
  })

  it('연속 assistant 행이면 연결선(w-px bg-border)이 그려진다', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'a', itemId: 0 },
      { kind: 'text', text: 'b', itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 두 행이 이어지므로 연결선 span 이 존재한다(최소 1개 — top 은 아래로, bottom 은 위로).
    expect(container.querySelector('.w-px.bg-border')).toBeTruthy()
  })
})

describe('StructuredTextView dispatch (ADR-0050)', () => {
  it('text item → assistant markdown 본문으로 렌더된다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'hello **world**', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    // 자체 Markdown 이 마크다운을 렌더(bold 는 <strong>).
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

  it('structured label=thinking(내용 있음) → ThoughtRow(제목 토글)로 렌더되고, 클릭하면 본문이 펼쳐진다', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: 'let me reason' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    const toggle = screen.getByRole('button', { name: /Thought/ })
    expect(toggle).toBeTruthy()
    // 접힌 상태 — 본문은 아직 없다.
    expect(screen.queryByText('let me reason')).toBeNull()
    fireEvent.click(toggle)
    expect(screen.getByText('let me reason')).toBeTruthy()
  })

  // ★NEW★: 빈 thinking(암호화 thinking — opus 는 signature 만 emit)도 "Thought" 라벨을 렌더한다(비-인터랙티브).
  //   이전 라운드는 빈 thinking 을 아예 걸렀지만, 이제는 추론 존재를 보여야 하므로 라벨을 남긴다.
  it('빈 thinking(공백/누락) → 비-인터랙티브 "Thought" 라벨을 렌더한다(펼침 불가)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'thinking', json: JSON.stringify({ thinking: '   ' }), itemId: 0 },
    ]
    render(<StructuredTextView items={items} />)
    // "Thought" 라벨은 있다(추론 존재 노출).
    expect(screen.getByText('Thought')).toBeTruthy()
    // 하지만 비-인터랙티브 — aria-expanded 가 없다(펼침 chevron 없음).
    const btn = screen.getByText('Thought').closest('button')
    expect(btn?.getAttribute('aria-expanded')).toBeNull()
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

  it('usage item → 아무것도 렌더하지 않는다(메시지별 토큰 칩 미표시)', () => {
    const items: StructuredItem[] = [{ kind: 'usage', inputTokens: 2, outputTokens: 5, itemId: 0 }]
    const { container } = render(<StructuredTextView items={items} />)
    // 누적 item 종류는 유지하되 렌더는 생략 — in/out 텍스트가 화면에 없어야 한다.
    expect(screen.queryByText(/in 2/)).toBeNull()
    expect(screen.queryByText(/out 5/)).toBeNull()
    // usage 만 있는 items 는 보이는 행을 만들지 않는다(ChatRow 래퍼도 없음).
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
    // 점선 레일/구분선이 없다 — separator 는 눈에 띄는 divider 가 아니라 aria-hidden 스페이서다.
    expect(container.querySelector('div[aria-hidden].border-t')).toBeNull()
    const spacer = container.querySelector('div[aria-hidden]')
    expect(spacer).toBeTruthy()
    expect(spacer?.className).toContain('h-3')
  })

  it('streaming=true 면 스트림 끝에 라이브 신호("Thinking…" pulse 라벨)를 붙인다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'working', itemId: 0 }]
    render(<StructuredTextView items={items} streaming />)
    // streaming ThoughtRow — 라벨 "Thinking…" 이 보인다.
    expect(screen.getByText('Thinking…')).toBeTruthy()
  })

  it('streaming=false(기본)면 라이브 Thinking 신호가 없다', () => {
    const items: StructuredItem[] = [{ kind: 'text', text: 'done', itemId: 0 }]
    render(<StructuredTextView items={items} />)
    expect(screen.queryByText('Thinking…')).toBeNull()
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

  // ── ADR-0050: dot-rail 스켈레톤 구조 ───────────────────────────────────────────────
  it('점선(border-dashed) 레일 대신 dot-rail 골격을 쓴다', () => {
    const items: StructuredItem[] = [
      { kind: 'text', text: 'hi', itemId: 0 },
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 1 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 이전 시안의 좌측 세로 점선 border(border-dashed) 레일 세그먼트는 쓰지 않는다.
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
    // gutter 안에 점 마커(size-1.5 rounded-full bg-muted)가 있다.
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
    // tool 행 = 초록 점(실행 신호), text 행 = muted 점.
    expect(container.querySelector('.rounded-full.bg-green-500')).toBeTruthy()
    expect(container.querySelector('.rounded-full.bg-muted')).toBeTruthy()
  })

  it('user 버블 행은 rail gutter/점 마커가 없다(plain full-width)', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'ping' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 유저 버블은 plain ChatRow — 점 마커도 콘텐츠 컬럼 래퍼도 없다.
    expect(container.querySelector('.rounded-full.bg-muted')).toBeNull()
    expect(container.querySelector('.flex-1.min-w-0')).toBeNull()
    // outer 래퍼는 여전히 relative px-4(flex 아님) — top-padding 은 CSS 변수 inline style(ADR-0051).
    const row = container.querySelector('.relative.px-4')
    expect(row).toBeTruthy()
    expect(row?.className).not.toContain('flex')
  })

  it('structured label=user → 확장 룩 버블(rounded-md border bg-elevated)로 렌더', () => {
    const items: StructuredItem[] = [
      { kind: 'structured', label: 'user', json: JSON.stringify({ text: 'do the thing' }), itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    const bubble = screen.getByText('do the thing')
    // 확장 룩 유저 버블: rounded-md border bg-elevated(다크에서 페이지보다 한 단계 밝은 배경 — 가시성).
    expect(bubble.className).toContain('rounded-md')
    expect(bubble.className).toContain('border')
    expect(bubble.className).toContain('bg-elevated')
    // 사용자 박스는 편집/토글 버튼이 없는 plain 텍스트 박스다.
    expect(container.querySelectorAll('button').length).toBe(0)
  })

  it('tool item → 헤더(아이콘 + bold 이름) + bg-surface 박스로 렌더', () => {
    const items: StructuredItem[] = [
      { kind: 'tool', name: 'Bash', argsJson: '{"command":"ls"}', id: 'tu_1', itemId: 0 },
    ]
    const { container } = render(<StructuredTextView items={items} />)
    // 헤더에 bold 도구명(HEADER_CLASSNAMES 패턴).
    const title = screen.getByText('Bash')
    expect(title.className).toContain('font-bold')
    // 도구 본문은 bg-surface rounded-sm border 박스.
    expect(container.querySelector('.bg-surface.rounded-sm')).toBeTruthy()
  })
})
