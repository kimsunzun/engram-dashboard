// 채팅 마크다운 렌더러 — 우리 자체 구현(벤치마크 = Claude Code VSCode 확장 룩, 픽셀은 후속 조정).
//   assistant 본문(신뢰 콘텐츠)만 이 렌더러를 태운다. 도구 IN/OUT·탈출구 json 은 신뢰할 수 없는
//   텍스트라 StructuredTextView 가 리터럴 <pre>(InertCode)로만 그린다 — 여기 오지 않는다.
//
// 파서 구성: react-markdown + remark-gfm(표·취소선·자동링크) + remark-math/rehype-katex(수식)
//   + rehype-highlight(코드 하이라이트). 커스텀 remark 플러그인은 두지 않는다 —
//   bare URL 자동링크는 remark-gfm 이 이미 처리하므로 unist-util-visit 기반 변환이 불필요.
import type { ComponentProps, HTMLAttributes } from 'react'
import { useCallback, useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeHighlight, { type Options } from 'rehype-highlight'
import rehypeKatex from 'rehype-katex'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'

import { CopyButton } from './CopyButton'
import './chat.css'
import 'katex/dist/katex.min.css'

// 스트림 출력이 ``` 펜스 바로 앞에 zero-width 문자를 흘리면(U+200B/200C/200D/2060/FEFF) 펜스 오프너는
//   "공백 0–3 + 백틱" 이어야 하는데 zero-width 는 비공백이라 micromark 가 펜스를 놓치고 두 ``` 를 인라인
//   code-span 쌍으로 파싱한다 → 코드블록이 문단으로 붕괴한다. 우리 신뢰 콘텐츠에서 zero-width 는 의미가
//   없으므로 파싱 전에 제거하는 게 안전하고 렌더를 견고하게 한다. (\u 이스케이프로 명시 — 소스에 리터럴
//   zero-width 를 두면 보이지 않는 편집 위험.)
const ZERO_WIDTH_RE = /[\u200B\u200C\u200D\u2060\uFEFF]/g
const stripZeroWidth = (text: string): string => text.replace(ZERO_WIDTH_RE, '')

// 코드 언어 별칭 정규화 — rehype-highlight 는 등록 언어명만 인식한다. 언어 미지정은 javascript 로,
//   `foo.ts` 처럼 점이 낀 언어명은 마지막 세그먼트만 취한다(스트림이 파일명을 언어로 흘리는 경우 방어).
function normalizeCodeLang() {
  return (tree: unknown) => {
    const visit = (node: any): void => {
      if (node && typeof node === 'object') {
        if (node.type === 'code') {
          if (!node.lang) node.lang = 'javascript'
          else if (typeof node.lang === 'string' && node.lang.includes('.'))
            node.lang = node.lang.split('.').slice(-1)[0]
        }
        if (Array.isArray(node.children)) node.children.forEach(visit)
      }
    }
    visit(tree)
  }
}

/** <pre> 렌더 — hover 시 우상단에 복사 버튼을 얹는다. group 래퍼로 hover 노출을 제어. */
function PreBlock({ children, ...preProps }: HTMLAttributes<HTMLPreElement>) {
  const preRef = useRef<HTMLPreElement>(null)
  const getText = useCallback(() => {
    const el = preRef.current
    if (!el) return null
    const code = el.querySelector('code')
    return code ? code.textContent : el.textContent
  }, [])
  return (
    <div className="group relative">
      <pre {...preProps} ref={preRef}>
        {children}
      </pre>
      <CopyButton getText={getText} label="코드 복사" />
    </div>
  )
}

interface MarkdownProps {
  markdown?: string
}

/**
 * 전체 마크다운 문서를 하나의 <ReactMarkdown> 으로 렌더한다(블록 분할 금지 — 분할하면 uniformly 들여쓴
 * 문서가 단일 code 토큰으로 붕괴). 렌더 전에 zero-width 를 제거해 펜스 붕괴를 막는다.
 */
export function Markdown({ markdown }: MarkdownProps) {
  const clean = markdown ? stripZeroWidth(markdown) : ''
  if (!clean) return null
  return (
    <div className="chat-markdown">
      <ReactMarkdown
        remarkPlugins={[[remarkGfm, { singleTilde: false }], remarkMath, normalizeCodeLang]}
        rehypePlugins={[rehypeKatex, [rehypeHighlight as any, {} as Options]]}
        components={{
          pre: (props: HTMLAttributes<HTMLPreElement>) => <PreBlock {...props} />,
          code: (props: ComponentProps<'code'>) => <code {...props} />,
        }}
      >
        {clean}
      </ReactMarkdown>
    </div>
  )
}

export default Markdown
