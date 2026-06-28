// 미니 Markdown 렌더층(MarkdownView) — MdNode[] → React. 파싱(markdown.ts)과 분리.
// (파일명을 markdown.tsx 가 아니라 MarkdownView.tsx 로: markdown.ts 와 basename 충돌 회피.)
// ★층 분리★: parseMarkdown 이 트리를 내고, 여기서 트리만 React 로 그린다.
// 색/폰트/간격은 전부 CSS 변수(.md-* class)로 — 인라인 px·color 금지(프리셋이 CSS var 하나로 reflow).

import type { MdInline, MdNode } from './markdown'
import { parseMarkdown } from './markdown'
import { useRenderSettings } from './renderSettings'
import { LazyMonacoCodeBlock, LazyMonacoDiff } from './MonacoLazy'

/** 인라인 조각들 → React 노드. */
function renderInlines(inlines: MdInline[]) {
  return inlines.map((inl, i) => {
    switch (inl.type) {
      case 'bold':
        return <strong key={i}>{inl.text}</strong>
      case 'italic':
        return <em key={i}>{inl.text}</em>
      case 'code':
        return (
          <code key={i} className="md-inline-code">
            {inl.text}
          </code>
        )
      case 'text':
        return <span key={i}>{inl.text}</span>
    }
  })
}

/** 한 MdNode → React. */
function renderNode(node: MdNode, key: number) {
  switch (node.type) {
    case 'heading': {
      const Tag = (`h${node.level}` as 'h1' | 'h2' | 'h3')
      return (
        <Tag key={key} className={`md-h md-h${node.level}`}>
          {renderInlines(node.inlines)}
        </Tag>
      )
    }
    case 'paragraph':
      return (
        <p key={key} className="md-p">
          {renderInlines(node.inlines)}
        </p>
      )
    case 'list': {
      const Tag = node.ordered ? 'ol' : 'ul'
      return (
        <Tag key={key} className="md-list">
          {node.items.map((item, i) => (
            <li key={i}>{renderInlines(item)}</li>
          ))}
        </Tag>
      )
    }
    case 'code':
      // 컴포넌트로 분리 — 펜스 렌더는 토글(useRenderSettings)에 따라 분기한다.
      return <CodeBlock key={key} node={node} />
  }
}

/** 펜스 코드 블록 — 토글에 따라 자체 렌더(plain/inline) ↔ Monaco 분기. */
function CodeBlock({ node }: { node: Extract<MdNode, { type: 'code' }> }) {
  const { codeRender, diffRender } = useRenderSettings()

  // diff 펜스 + diffRender=monaco → Monaco diff. (parser 가 isDiff 로 판정)
  if (node.isDiff && diffRender === 'monaco') {
    return <LazyMonacoDiff diff={node.code} />
  }
  // 비-diff 코드 펜스 + codeRender=monaco → Monaco 코드(언어 강조).
  if (!node.isDiff && codeRender === 'monaco') {
    return <LazyMonacoCodeBlock code={node.code} lang={node.lang} />
  }

  // 기본 = 자체 렌더. diff 면 줄마다 add/del class 로 색칠, 아니면 모노박스.
  return (
    <pre className={`md-code${node.isDiff ? ' md-code-diff' : ''}`}>
      {node.isDiff
        ? node.lines.map((l, i) => (
            <div key={i} className={`md-diff-line md-diff-${l.kind}`}>
              {l.text || ' '}
            </div>
          ))
        : node.code}
    </pre>
  )
}

/** 텍스트 블록 1개를 Markdown 으로 렌더(파싱 → 렌더 한 번에). */
export function Markdown({ text }: { text: string }) {
  const nodes = parseMarkdown(text)
  return <div className="md">{nodes.map((n, i) => renderNode(n, i))}</div>
}
