// ADR-0048: MarkdownBlock 렌더 테스트 — assistant/thinking 마크다운(신뢰 콘텐츠)이 heading·GFM 표·펜스 코드를
//   포함하는 멀티블록 문서로 와도 올바르게 파싱·렌더되는지 단언한다.
//
// 회귀 배경(FIX): 예전엔 marked.lexer 로 top-level 블록을 쪼개 각 블록을 SEPARATE <ReactMarkdown> 로 그렸고,
//   출력을 inline <span>(display:inline) 으로 감쌌다. 이 두 요인이 겹쳐, heading+표+코드가 섞인 응답이 단일
//   raw <pre> 로 떨어지고(##/파이프/``` 가 리터럴 텍스트), <h2>·<table> 이 사라졌다. 이제 전체 마크다운을 하나의
//   <ReactMarkdown> 으로(블록 분할 제거) 블록 <div> 안에서 그린다. 이 테스트가 그 회귀를 못박는다.
//
// 회귀 배경(FIX2 — zero-width): 실제 모델 스트림 출력이 각 ``` 펜스 바로 앞에 U+200B(ZERO WIDTH SPACE)를
//   실어보냈다. 펜스 오프너는 "공백 0–3 + 백틱" 이어야 하는데 ZWSP 는 비공백이라 micromark 가 펜스를 놓치고
//   두 ``` 를 INLINE code-span 쌍으로 파싱한다 → 코드블록이 <p><code>…</code></p> 로 붕괴(<pre> 없음). AST 로
//   확인: ZWSP 있으면 `paragraph > text + inlineCode`, 제거하면 `code lang="js"`. 이제 렌더 전에 zero-width 를
//   제거(stripZeroWidth)한다. 아래 테스트가 EXACT 실패 입력(U+200B 포함)으로 그 회귀를 못박는다.

import { cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import MarkdownBlock from './MarkdownBlock'

afterEach(() => cleanup())

// heading + GFM 표(2열 2행) + python 펜스 코드가 한 문서에 섞인 케이스(버그 재현 입력).
const MULTIBLOCK = [
  '## Section Title',
  '',
  '| Name | Value |',
  '| ---- | ----- |',
  '| foo  | 1     |',
  '| bar  | 2     |',
  '',
  '```python',
  'print("hello")',
  '```',
  '',
].join('\n')

describe('MarkdownBlock 멀티블록 렌더(FIX: 단일 ReactMarkdown + 블록 컨테이너)', () => {
  it('heading → <h2>, GFM 표 → <table>(<td>), 펜스 코드 → 하이라이트된 <pre>/<code> 로 파싱된다', () => {
    const { container } = render(<MarkdownBlock markdown={MULTIBLOCK} />)

    // heading 이 <h2> 로 승격(리터럴 "##" 아님).
    const h2 = container.querySelector('h2')
    expect(h2).toBeTruthy()
    expect(h2?.textContent).toBe('Section Title')

    // GFM 표가 remark-gfm 로 실제 <table> 로 파싱된다(파이프 리터럴 아님).
    const table = container.querySelector('table')
    expect(table).toBeTruthy()
    const tds = container.querySelectorAll('td')
    expect(tds.length).toBe(4) // 2열 × 2행 데이터 셀
    const cellText = Array.from(tds).map((td) => td.textContent?.trim())
    expect(cellText).toEqual(['foo', '1', 'bar', '2'])

    // 펜스 코드가 rehype-highlight 로 하이라이트된 <pre><code> 로 파싱된다(``` 리터럴 아님).
    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    const code = pre?.querySelector('code')
    expect(code).toBeTruthy()
    // 코드-lang 정규화 플러그인 → language-python, rehype-highlight → hljs 클래스.
    expect(code?.className).toContain('hljs')
    expect(code?.className).toContain('language-python')
    expect(code?.textContent).toContain('print')

    // 원시 마크다운 마커(##, 표 파이프, ``` 펜스)가 리터럴 텍스트로 남지 않는다.
    const text = container.textContent ?? ''
    expect(text).not.toContain('##')
    expect(text).not.toContain('| Name |')
    expect(text).not.toContain('```')
  })

  it('출력 컨테이너는 인라인 <span> 이 아니라 블록 <div> 다(block-in-inline 렌더 회피)', () => {
    const { container } = render(<MarkdownBlock markdown={MULTIBLOCK} />)
    const wrapper = container.querySelector('.inline-markdown-block')
    expect(wrapper).toBeTruthy()
    // 예전 버그: h2/table/pre 를 담는 컨테이너가 inline <span>(display:inline) 이었다. 이제 <div>.
    const inner = wrapper?.firstElementChild
    expect(inner?.tagName.toLowerCase()).toBe('div')
    // h2/table/pre 가 이 블록 컨테이너의 하위에 있다.
    expect(inner?.querySelector('h2')).toBeTruthy()
    expect(inner?.querySelector('table')).toBeTruthy()
    expect(inner?.querySelector('pre')).toBeTruthy()
  })

  it('단순 콘텐츠(문단 + 불릿 리스트 + bold)도 그대로 올바르게 렌더된다', () => {
    const simple = ['A **bold** word.', '', '- first', '- second', ''].join('\n')
    const { container } = render(<MarkdownBlock markdown={simple} />)

    // bold → <strong>.
    const strong = container.querySelector('strong')
    expect(strong?.textContent).toBe('bold')
    // 문단 → <p>.
    expect(container.querySelector('p')).toBeTruthy()
    // 불릿 리스트 → <ul><li>×2.
    expect(container.querySelector('ul')).toBeTruthy()
    expect(container.querySelectorAll('li').length).toBe(2)
    // 표/코드 블록은 없다(단순 케이스).
    expect(container.querySelector('table')).toBeNull()
    expect(container.querySelector('pre')).toBeNull()
  })
})

// EXACT 실패 입력 재현: 라이브 앱에서 CDP 로 캡처한 문자열. 각 ``` 펜스 바로 앞에 U+200B(ZWSP) 2개.
//   charcodes 로 확인됨: [...,10,10,8203,96,96,96,...]. heading/표 라인 자체는 깨끗(숨은 문자 없음).
//   소스에 리터럴 invisible char 를 두지 않으려고 ZWSP 는 String.fromCharCode(0x200b) 로 명시 구성한다.
const ZWSP = String.fromCharCode(0x200b)
const ZW_MULTIBLOCK = `### 테스트\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n${ZWSP}\`\`\`js\nconsole.log(9)\n${ZWSP}\`\`\`\n`

describe('MarkdownBlock zero-width 방어(FIX2: stripZeroWidth — ``` 앞 U+200B 로 인한 펜스 붕괴)', () => {
  it('펜스 앞 U+200B 가 있어도 heading → <h3>, 표 → <table>, 펜스 코드 → 하이라이트된 <pre>/<code> 로 렌더된다', () => {
    // 입력에 실제로 U+200B 가 2개 들어있음을 먼저 단언(테스트가 실제 버그 입력을 쓰는지 보증).
    expect(ZW_MULTIBLOCK.split(ZWSP).length - 1).toBe(2)

    const { container } = render(<MarkdownBlock markdown={ZW_MULTIBLOCK} />)

    // heading → <h3>(리터럴 "###" 아님).
    const h3 = container.querySelector('h3')
    expect(h3).toBeTruthy()
    expect(h3?.textContent).toBe('테스트')

    // GFM 표 → 실제 <table>(파이프 리터럴 아님), 데이터 셀 4개.
    expect(container.querySelector('table')).toBeTruthy()
    const tds = container.querySelectorAll('td')
    expect(tds.length).toBe(4)
    expect(Array.from(tds).map((td) => td.textContent?.trim())).toEqual(['1', '2', '3', '4'])

    // ★ 핵심 회귀: 펜스 코드가 inline code-span 으로 붕괴하지 않고 진짜 <pre><code> 블록으로 렌더된다.
    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    const code = pre?.querySelector('code')
    expect(code).toBeTruthy()
    // code-lang 정규화 → language-js, rehype-highlight → hljs 클래스.
    expect(code?.className).toContain('hljs')
    expect(code?.className).toContain('language-js')
    expect(code?.textContent).toContain('console.log(9)')

    // 원시 마커(###, 표 파이프, ``` 펜스)도 U+200B 도 리터럴로 남지 않는다.
    const text = container.textContent ?? ''
    expect(text).not.toContain('###')
    expect(text).not.toContain('| A |')
    expect(text).not.toContain('```')
    expect(text).not.toContain(ZWSP)
  })
})
