// Regression background (zero-width): real model stream output carried U+200B (ZERO WIDTH SPACE) right before each ``` fence.
//   A fence opener must be "0–3 spaces + backticks", but ZWSP is non-whitespace so micromark misses the fence and parses two ``` as
//   an inline code-span pair → the code block collapses into <p><code>…</code></p>.
//   Defended by removing zero-width before render (stripZeroWidth).

import { cleanup, render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { Markdown } from './Markdown'

afterEach(() => cleanup())

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

describe('Markdown render (single ReactMarkdown, multi-block)', () => {
  it('heading → <h2>, GFM table → <table>(<td>), fenced code → parsed as highlighted <pre>/<code>', () => {
    const { container } = render(<Markdown markdown={MULTIBLOCK} />)

    const h2 = container.querySelector('h2')
    expect(h2).toBeTruthy()
    expect(h2?.textContent).toBe('Section Title')

    const table = container.querySelector('table')
    expect(table).toBeTruthy()
    const tds = container.querySelectorAll('td')
    expect(tds.length).toBe(4)
    expect(Array.from(tds).map((td) => td.textContent?.trim())).toEqual(['foo', '1', 'bar', '2'])

    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    const code = pre?.querySelector('code')
    expect(code).toBeTruthy()
    expect(code?.className).toContain('hljs')
    expect(code?.className).toContain('language-python')
    expect(code?.textContent).toContain('print')

    const text = container.textContent ?? ''
    expect(text).not.toContain('##')
    expect(text).not.toContain('| Name |')
    expect(text).not.toContain('```')
  })

  it('simple content (paragraph + bullet list + bold) also renders correctly', () => {
    const simple = ['A **bold** word.', '', '- first', '- second', ''].join('\n')
    const { container } = render(<Markdown markdown={simple} />)
    expect(container.querySelector('strong')?.textContent).toBe('bold')
    expect(container.querySelector('p')).toBeTruthy()
    expect(container.querySelector('ul')).toBeTruthy()
    expect(container.querySelectorAll('li').length).toBe(2)
  })
})

// To keep literal invisible chars out of the source, ZWSP is built explicitly with String.fromCharCode(0x200b).
const ZWSP = String.fromCharCode(0x200b)
const ZW_MULTIBLOCK = `### 테스트\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n${ZWSP}\`\`\`js\nconsole.log(9)\n${ZWSP}\`\`\`\n` // hangul-ok

describe('Markdown zero-width defense (stripZeroWidth — fence collapse caused by U+200B before ```)', () => {
  it('even with U+200B before the fence, heading → <h3>, table → <table>, fenced code → rendered as <pre>/<code>', () => {
    // Guarantees the test uses the real bug input.
    expect(ZW_MULTIBLOCK.split(ZWSP).length - 1).toBe(2)

    const { container } = render(<Markdown markdown={ZW_MULTIBLOCK} />)

    const h3 = container.querySelector('h3')
    expect(h3).toBeTruthy()
    expect(h3?.textContent).toBe('테스트')

    expect(container.querySelector('table')).toBeTruthy()
    const tds = container.querySelectorAll('td')
    expect(tds.length).toBe(4)
    expect(Array.from(tds).map((td) => td.textContent?.trim())).toEqual(['1', '2', '3', '4'])

    const pre = container.querySelector('pre')
    expect(pre).toBeTruthy()
    const code = pre?.querySelector('code')
    expect(code).toBeTruthy()
    expect(code?.className).toContain('hljs')
    expect(code?.className).toContain('language-js')
    expect(code?.textContent).toContain('console.log(9)')

    const text = container.textContent ?? ''
    expect(text).not.toContain('###')
    expect(text).not.toContain('| A |')
    expect(text).not.toContain('```')
    expect(text).not.toContain(ZWSP)
  })
})

describe('Markdown KaTeX math (remark-math + rehype-katex)', () => {
  it('the $$x^2$$ block math renders as a .katex element', () => {
    const { container } = render(<Markdown markdown={'$$x^2$$'} />)
    expect(container.querySelector('.katex')).toBeTruthy()
  })
})
