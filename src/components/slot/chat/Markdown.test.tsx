// Markdown 렌더 테스트 — 우리 자체 채팅 마크다운 렌더러(chat/Markdown.tsx)가 assistant/thinking
//   신뢰 콘텐츠(heading·GFM 표·펜스 코드·수식)를 올바르게 파싱·렌더하는지 단언한다.
//
// 회귀 배경(zero-width): 실제 모델 스트림 출력이 각 ``` 펜스 바로 앞에 U+200B(ZERO WIDTH SPACE)를
//   실어보냈다. 펜스 오프너는 "공백 0–3 + 백틱" 이어야 하는데 ZWSP 는 비공백이라 micromark 가 펜스를
//   놓치고 두 ``` 를 인라인 code-span 쌍으로 파싱한다 → 코드블록이 <p><code>…</code></p> 로 붕괴한다.
//   렌더 전에 zero-width 를 제거(stripZeroWidth)해 방어한다. 아래 테스트가 EXACT 실패 입력으로 못박는다.

import { cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { Markdown } from './Markdown'

afterEach(() => cleanup())

// heading + GFM 표(2열 2행) + python 펜스 코드가 한 문서에 섞인 멀티블록 케이스.
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

describe('Markdown 렌더(단일 ReactMarkdown, 멀티블록)', () => {
  it('heading → <h2>, GFM 표 → <table>(<td>), 펜스 코드 → 하이라이트된 <pre>/<code> 로 파싱된다', () => {
    const { container } = render(<Markdown markdown={MULTIBLOCK} />)

    const h2 = container.querySelector('h2')
    expect(h2).toBeTruthy()
    expect(h2?.textContent).toBe('Section Title')

    const table = container.querySelector('table')
    expect(table).toBeTruthy()
    const tds = container.querySelectorAll('td')
    expect(tds.length).toBe(4) // 2열 × 2행 데이터 셀
    expect(Array.from(tds).map((td) => td.textContent?.trim())).toEqual(['foo', '1', 'bar', '2'])

    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    const code = pre?.querySelector('code')
    expect(code).toBeTruthy()
    // code-lang 정규화 → language-python, rehype-highlight → hljs 클래스.
    expect(code?.className).toContain('hljs')
    expect(code?.className).toContain('language-python')
    expect(code?.textContent).toContain('print')

    // 원시 마커(##, 표 파이프, ``` 펜스)가 리터럴 텍스트로 남지 않는다.
    const text = container.textContent ?? ''
    expect(text).not.toContain('##')
    expect(text).not.toContain('| Name |')
    expect(text).not.toContain('```')
  })

  it('단순 콘텐츠(문단 + 불릿 리스트 + bold)도 올바르게 렌더된다', () => {
    const simple = ['A **bold** word.', '', '- first', '- second', ''].join('\n')
    const { container } = render(<Markdown markdown={simple} />)
    expect(container.querySelector('strong')?.textContent).toBe('bold')
    expect(container.querySelector('p')).toBeTruthy()
    expect(container.querySelector('ul')).toBeTruthy()
    expect(container.querySelectorAll('li').length).toBe(2)
  })
})

// EXACT 실패 입력 재현: 각 ``` 펜스 바로 앞에 U+200B(ZWSP). 소스에 리터럴 invisible char 를 두지 않으려고
//   ZWSP 는 String.fromCharCode(0x200b) 로 명시 구성한다.
const ZWSP = String.fromCharCode(0x200b)
const ZW_MULTIBLOCK = `### 테스트\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n${ZWSP}\`\`\`js\nconsole.log(9)\n${ZWSP}\`\`\`\n`

describe('Markdown zero-width 방어(stripZeroWidth — ``` 앞 U+200B 로 인한 펜스 붕괴)', () => {
  it('펜스 앞 U+200B 가 있어도 heading → <h3>, 표 → <table>, 펜스 코드 → <pre>/<code> 로 렌더된다', () => {
    // 입력에 실제로 U+200B 가 2개 들어있음을 먼저 단언(테스트가 실제 버그 입력을 쓰는지 보증).
    expect(ZW_MULTIBLOCK.split(ZWSP).length - 1).toBe(2)

    const { container } = render(<Markdown markdown={ZW_MULTIBLOCK} />)

    const h3 = container.querySelector('h3')
    expect(h3).toBeTruthy()
    expect(h3?.textContent).toBe('테스트')

    expect(container.querySelector('table')).toBeTruthy()
    const tds = container.querySelectorAll('td')
    expect(tds.length).toBe(4)
    expect(Array.from(tds).map((td) => td.textContent?.trim())).toEqual(['1', '2', '3', '4'])

    // ★ 핵심 회귀: 펜스 코드가 인라인 code-span 으로 붕괴하지 않고 진짜 <pre><code> 블록으로 렌더된다.
    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    const code = pre?.querySelector('code')
    expect(code).toBeTruthy()
    expect(code?.className).toContain('hljs')
    expect(code?.className).toContain('language-js')
    expect(code?.textContent).toContain('console.log(9)')

    // 원시 마커·U+200B 모두 리터럴로 남지 않는다.
    const text = container.textContent ?? ''
    expect(text).not.toContain('###')
    expect(text).not.toContain('| A |')
    expect(text).not.toContain('```')
    expect(text).not.toContain(ZWSP)
  })
})

describe('Markdown KaTeX 수식(remark-math + rehype-katex)', () => {
  it('$$x^2$$ 블록 수식이 .katex 요소로 렌더된다', () => {
    const { container } = render(<Markdown markdown={'$$x^2$$'} />)
    expect(container.querySelector('.katex')).toBeTruthy()
  })
})
