// 미니 Markdown 파서 단위테스트 — parseMarkdown/parseInline 순수 함수 검증.
// 사용자 #1 페인포인트(raw markdown 가독성)의 회귀 안전망. React 무관 → DOM 불필요.

import { describe, it, expect } from 'vitest'
import { parseMarkdown, parseInline, type MdNode } from './markdown'

/** 헬퍼 — n번째 노드를 타입 좁혀 꺼낸다(테스트 가독성). */
function nodeAt(nodes: MdNode[], i: number): MdNode {
  return nodes[i]
}

describe('parseMarkdown — 블록', () => {
  it('헤더 레벨 #/##/### 을 1/2/3 으로 파싱', () => {
    const nodes = parseMarkdown('# H1\n\n## H2\n\n### H3')
    const headings = nodes.filter((n) => n.type === 'heading')
    expect(headings.map((n) => (n.type === 'heading' ? n.level : 0))).toEqual([1, 2, 3])
  })

  it('펜스 코드블록을 추출하고 안쪽 마크다운은 해석하지 않는다', () => {
    const nodes = parseMarkdown('text before\n\n```python\ndef peek():\n    return self._items[0]\n```\n\nafter')
    const code = nodes.find((n) => n.type === 'code')
    expect(code).toBeDefined()
    if (code?.type === 'code') {
      expect(code.lang).toBe('python')
      expect(code.code).toContain('def peek():')
      expect(code.isDiff).toBe(false)
    }
    // 펜스 앞뒤 문단도 분리돼야 한다.
    expect(nodes.filter((n) => n.type === 'paragraph')).toHaveLength(2)
  })

  it('diff 펜스: +/- 줄에 add/del kind 를 태깅(헤더 ---/+++ 는 ctx)', () => {
    const diff = ['```diff', '--- a/x.py', '+++ b/x.py', '+added line', '-removed line', ' context'].join('\n')
    const nodes = parseMarkdown(diff)
    const code = nodes.find((n) => n.type === 'code')
    expect(code?.type === 'code' && code.isDiff).toBe(true)
    if (code?.type === 'code') {
      const kinds = code.lines.map((l) => l.kind)
      // --- , +++ , +added , -removed , ctx
      expect(kinds).toEqual(['ctx', 'ctx', 'add', 'del', 'ctx'])
    }
  })

  it('lang 없이도 본문에 +/- 가 있으면 diff 로 자동 판정', () => {
    const nodes = parseMarkdown('```\n+new\n-old\n```')
    const code = nodes.find((n) => n.type === 'code')
    expect(code?.type === 'code' && code.isDiff).toBe(true)
  })

  it('불릿 리스트(- )를 list(ordered:false)로 묶는다', () => {
    const nodes = parseMarkdown('- first\n- second\n- third')
    const list = nodeAt(nodes, 0)
    expect(list.type).toBe('list')
    if (list.type === 'list') {
      expect(list.ordered).toBe(false)
      expect(list.items).toHaveLength(3)
    }
  })

  it('번호 리스트(1. )를 list(ordered:true)로 묶는다', () => {
    const nodes = parseMarkdown('1. one\n2. two')
    const list = nodeAt(nodes, 0)
    expect(list.type).toBe('list')
    if (list.type === 'list') {
      expect(list.ordered).toBe(true)
      expect(list.items).toHaveLength(2)
    }
  })

  it('불릿↔번호 종류가 바뀌면 별도 리스트로 쪼갠다', () => {
    const nodes = parseMarkdown('- a\n1. b')
    const lists = nodes.filter((n) => n.type === 'list')
    expect(lists).toHaveLength(2)
  })

  it('평문은 문단으로 떨어진다(fallback)', () => {
    const nodes = parseMarkdown('just some plain prose with no markdown')
    expect(nodes).toHaveLength(1)
    expect(nodes[0].type).toBe('paragraph')
  })
})

describe('parseInline — 서식', () => {
  it('**bold** 를 bold inline 으로', () => {
    const inl = parseInline('a **strong** b')
    expect(inl.map((x) => x.type)).toEqual(['text', 'bold', 'text'])
    expect(inl.find((x) => x.type === 'bold')?.text).toBe('strong')
  })

  it('*italic* 를 italic inline 으로', () => {
    const inl = parseInline('an *emphasized* word')
    expect(inl.some((x) => x.type === 'italic')).toBe(true)
  })

  it('`inline code` 를 code inline 으로, 안쪽 별표는 코드로 보존', () => {
    const inl = parseInline('use `a*b` here')
    const code = inl.find((x) => x.type === 'code')
    expect(code?.text).toBe('a*b') // ** 로 안 깨짐
  })

  it('서식 없는 텍스트는 text 1개로 fallback', () => {
    const inl = parseInline('nothing special')
    expect(inl).toEqual([{ type: 'text', text: 'nothing special' }])
  })
})
