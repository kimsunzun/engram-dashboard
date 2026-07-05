// Originally from Cline (https://github.com/cline/cline) — Copyright Cline Bot Inc. Licensed under Apache-2.0.
// See LICENSES/cline-Apache-2.0.txt. Modified by Netmarble F&C / Engram, 2026.
// Changes: removed VSCode wiring — (1) useExtensionState() Plan/Act-mode ActModeHighlight + remarkHighlightActMode
//   plugin (dropped, along with the `strong` component override); (2) FileServiceClient file-existence check
//   (InlineCodeWithFileCheck + remarkMarkPotentialFilePaths dropped; inline code renders as plain <code>);
//   removed MermaidBlock and UnsafeImage (not ported — mermaid fences and <img> fall back to default rendering).
//   Kept the react-markdown render path, remark-gfm/remarkPreventBoldFilenames/remarkUrlToLink plugins, the
//   code-lang normalizer, and PreWithCopyButton (WithCopyButton). (3) Added stripZeroWidth() sanitization of the
//   markdown string before rendering (zero-width chars from model stream output broke fenced-code detection).
//   The `.inline-markdown-block` styling lives in ./cline.css (token-mapped to our theme; imports the hljs palette).
//
//   FIX (multi-block markdown rendering) — 2026-07: a heading + GFM table + fenced code block rendered as a single
//   raw <pre> (##/pipes/``` shown as literal text), while simple paragraphs/lists/bold worked. Two compounding root
//   causes, both removed here:
//     (1) parseMarkdownIntoBlocks (Cline's streaming-perf split via marked.lexer, rendered per-block through
//         SEPARATE <ReactMarkdown> instances) is fragile: whenever the whole doc is uniformly indented (≥4 spaces)
//         or collapses to a list context, marked.lexer returns a SINGLE `code` token, so the entire response
//         renders as one literal <pre> (exactly the observed single-raw-pre with rehype-highlight classes, no
//         <h2>/<table>). We now render the ENTIRE markdown through ONE <ReactMarkdown> — no block-splitting — so
//         the top-level structure is always parsed by remark as one coherent document.
//     (2) The output was wrapped in an inline <span> (display:inline) that contained block-level h2/table/pre —
//         illegal block-in-inline nesting that mis-renders in a real WebView. Now wrapped in a block <div>.
//   Dropped the marked dependency and the MemoizedMarkdown/MemoizedMarkdownBlock split exports (no longer needed).
//
//   FIX (zero-width chars break fenced code) — 2026-07: real model output (streamed) carried a U+200B ZERO WIDTH
//   SPACE immediately before each ``` fence (charcodes [...,8203,96,96,96,...]). A fenced-code opener in
//   CommonMark/micromark must be 0–3 spaces then the backticks; a ZWSP is a NON-space character, so the opener
//   check fails and micromark instead parses the two ``` markers as an INLINE code-span pair. AST-confirmed: with
//   the ZWSP the mdast is `paragraph > text "<U+200B>" + inlineCode` (renders as <p><U+200B><code>js
//   console.log(9) <U+200B></code></p>);
//   after stripping it, the node is a proper `code lang="js"` block. (Heading/table survived either way, but the
//   code block did not.) The ZWSP almost certainly originates upstream in the model's stream output / backend
//   decoder — the fix here is DEFENSIVE frontend sanitization so ALL markdown rendering is protected. See
//   stripZeroWidth below.
import type { ComponentProps } from 'react'
import React, { memo, useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeHighlight, { type Options } from 'rehype-highlight'
import remarkGfm from 'remark-gfm'
import type { Node } from 'unist'
import { visit } from 'unist-util-visit'
import { cn } from '@/lib/utils'
import { WithCopyButton } from './CopyButton'
import './cline.css'

interface MarkdownBlockProps {
	markdown?: string
	compact?: boolean
	showCursor?: boolean
}

// Defensive sanitization: strip invisible zero-width characters before markdown parsing.
// WHY: real model stream output injected U+200B ZERO WIDTH SPACE right before ``` fences; because a fenced-code
//   opener must be 0–3 SPACES then the backticks, a non-space ZWSP makes micromark miss the fence and instead
//   parse the two ``` as an inline code span — the code block collapses into a paragraph. These chars are never
//   semantically meaningful in our (trusted) assistant/thinking markdown, so removing them is safe and makes
//   rendering robust regardless of where upstream (stream decoder/backend) leaked them. Covers: U+200B ZWSP,
//   U+200C ZWNJ, U+200D ZWJ, U+2060 WORD JOINER, U+FEFF ZWNBSP/BOM.
//   (Written as \u escapes on purpose — literal zero-width chars in the source are an invisible edit hazard.)
const ZERO_WIDTH_RE = /[\u200B\u200C\u200D\u2060\uFEFF]/g
const stripZeroWidth = (text: string): string => text.replace(ZERO_WIDTH_RE, '')

/**
 * Custom remark plugin that converts plain URLs in text into clickable links
 *
 * The original bug: We were converting text nodes into paragraph nodes,
 * which broke the markdown structure because text nodes should remain as text nodes
 * within their parent elements (like paragraphs, list items, etc.).
 * This caused the entire content to disappear because the structure became invalid.
 */
const remarkUrlToLink = () => {
	return (tree: Node) => {
		// Visit all "text" nodes in the markdown AST (Abstract Syntax Tree)
		visit(tree, 'text', (node: any, index, parent) => {
			const urlRegex = /https?:\/\/[^\s<>)"]+/g
			const matches = node.value.match(urlRegex)
			if (!matches) {
				return
			}

			const parts = node.value.split(urlRegex)
			const children: any[] = []

			parts.forEach((part: string, i: number) => {
				if (part) {
					children.push({ type: 'text', value: part })
				}
				if (matches[i]) {
					children.push({
						type: 'link',
						url: matches[i],
						children: [{ type: 'text', value: matches[i] }],
					})
				}
			})

			// Fix: Instead of converting the node to a paragraph (which broke things),
			// we replace the original text node with our new nodes in the parent's children array.
			// This preserves the document structure while adding our links.
			if (parent) {
				parent.children.splice(index, 1, ...children)
			}
		})
	}
}

/**
 * Custom remark plugin that prevents filenames with extensions from being parsed as bold text
 * For example: __init__.py should not be rendered as bold "init" followed by ".py"
 * Solves https://github.com/cline/cline/issues/1028
 */
const remarkPreventBoldFilenames = () => {
	return (tree: any) => {
		visit(tree, 'strong', (node: any, index: number | undefined, parent: any) => {
			// Only process if there's a next node (potential file extension)
			if (!parent || typeof index === 'undefined' || index === parent.children.length - 1) {
				return
			}

			const nextNode = parent.children[index + 1]

			// Check if next node is text and starts with . followed by extension
			if (nextNode.type !== 'text' || !nextNode.value.match(/^\.[a-zA-Z0-9]+/)) {
				return
			}

			// If the strong node has multiple children, something weird is happening
			if (node.children?.length !== 1) {
				return
			}

			// Get the text content from inside the strong node
			const strongContent = node.children?.[0]?.value
			if (!strongContent || typeof strongContent !== 'string') {
				return
			}

			// Validate that the strong content is a valid filename
			if (!strongContent.match(/^[a-zA-Z0-9_-]+$/)) {
				return
			}

			// Combine into a single text node
			const newNode = {
				type: 'text',
				value: `__${strongContent}__${nextNode.value}`,
			}

			// Replace both nodes with the combined text node
			parent.children.splice(index, 2, newNode)
		})
	}
}

const PreWithCopyButton = ({ children, ...preProps }: React.HTMLAttributes<HTMLPreElement>) => {
	const preRef = useRef<HTMLPreElement>(null)

	const handleCopy = () => {
		if (preRef.current) {
			const codeElement = preRef.current.querySelector('code')
			const textToCopy = codeElement ? codeElement.textContent : preRef.current.textContent

			if (!textToCopy) {
				return
			}
			return textToCopy
		}
		return null
	}

	return (
		<WithCopyButton ariaLabel="Copy code" onCopy={handleCopy} position="top-right">
			<pre {...preProps} ref={preRef}>
				{children}
			</pre>
		</WithCopyButton>
	)
}

const MarkdownBlock = memo(({ markdown, compact, showCursor }: MarkdownBlockProps) => {
	// FIX: block container (was an inline <span>) so block-level h2/table/pre nest legally, and a SINGLE
	//   <ReactMarkdown> over the whole doc (no marked.lexer block-splitting) so multi-block structure survives.
	// FIX: strip zero-width chars BEFORE parsing — a leaked U+200B before a ``` fence otherwise makes micromark
	//   miss the fenced-code opener (see stripZeroWidth). Applies to the whole doc so every construct is protected.
	const clean = markdown ? stripZeroWidth(markdown) : markdown
	return (
		<div className="inline-markdown-block">
			<div
				className={cn('[&>p]:mt-0', {
					'inline-cursor-container': showCursor,
					// compact 전달 경로 미구현(현재 호출자 없음) + cline.css 언레이어드 규칙이 @layer utilities를 이기므로
					// 중첩 <p>의 margin-top: 1em은 완전히 억제되지 않는다 — 향후 호출자가 생기면 이 제한을 확인할 것.
				'[&>p]:m-0': compact,
				})}>
				{clean ? (
					<ReactMarkdown
						components={{
							pre: ({ children, ...preProps }: React.HTMLAttributes<HTMLPreElement>) => {
								return <PreWithCopyButton {...preProps}>{children}</PreWithCopyButton>
							},
							code: (props: ComponentProps<'code'> & { [key: string]: any }) => {
								return <code {...props} />
							},
						}}
						rehypePlugins={[[rehypeHighlight as any, {} as Options]]}
						remarkPlugins={[
							[remarkGfm, { singleTilde: false }],
							remarkPreventBoldFilenames,
							remarkUrlToLink,
							() => {
								return (tree: any) => {
									visit(tree, 'code', (node: any) => {
										if (!node.lang) {
											node.lang = 'javascript'
										} else if (node.lang.includes('.')) {
											node.lang = node.lang.split('.').slice(-1)[0]
										}
									})
								}
							},
						]}>
						{clean}
					</ReactMarkdown>
				) : (
					clean
				)}
			</div>
		</div>
	)
})

export default MarkdownBlock
