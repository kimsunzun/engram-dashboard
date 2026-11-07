// Chat markdown renderer — our own implementation (benchmark = Claude Code VSCode extension look, pixels adjusted later).
//   Only assistant body (trusted content) goes through this renderer. Tool IN/OUT and escape-hatch json are untrusted
//   text, so StructuredTextView draws them only as a literal <pre> (InertCode) — they do not come here.
//
// No custom remark plugin for bare URL autolinks — remark-gfm already handles it, so
//   an unist-util-visit based transform is unnecessary.
import type { ComponentProps, HTMLAttributes } from 'react'
import { useCallback, useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeHighlight, { type Options } from 'rehype-highlight'
import rehypeKatex from 'rehype-katex'
import remarkGfm from 'remark-gfm'
import remarkMath from 'remark-math'

import { CopyButton } from './CopyButton'
import { t } from '../../../i18n'
import './chat.css'
import 'katex/dist/katex.min.css'

// If the stream output leaks a zero-width character right before a ``` fence (U+200B/200C/200D/2060/FEFF), a fence opener must be
//   "0–3 spaces + backticks", but zero-width is non-whitespace so micromark misses the fence and parses two ``` as an inline
//   code-span pair → the code block collapses into a paragraph. In our trusted content zero-width has no meaning,
//   so removing it before parsing is safe and makes the render robust. (Stated with \u escapes — leaving a literal
//   zero-width in the source risks invisible edits.)
const ZERO_WIDTH_RE = /[\u200B\u200C\u200D\u2060\uFEFF]/g
const stripZeroWidth = (text: string): string => text.replace(ZERO_WIDTH_RE, '')

// rehype-highlight recognizes only registered language names. Handling dotted language names = defense for the case
//   where the stream passes a file name as the language.
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
      <CopyButton getText={getText} label={t('common.codeCopy')} />
    </div>
  )
}

interface MarkdownProps {
  markdown?: string
}

/**
 * Renders the whole markdown document as one <ReactMarkdown> (no block splitting — if split, a uniformly indented
 * document collapses into a single code token).
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
