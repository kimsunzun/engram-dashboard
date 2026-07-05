// Originally from Cline (https://github.com/cline/cline) — Copyright Cline Bot Inc. Licensed under Apache-2.0.
// See LICENSES/cline-Apache-2.0.txt. Modified by Netmarble F&C / Engram, 2026.
// Changes: removed VSCode wiring — (1) useExtensionState() Plan/Act-mode ActModeHighlight + remarkHighlightActMode
//   plugin (dropped, along with the `strong` component override); (2) FileServiceClient file-existence check
//   (InlineCodeWithFileCheck + remarkMarkPotentialFilePaths dropped; inline code renders as plain <code>);
//   removed MermaidBlock and UnsafeImage (not ported — mermaid fences and <img> fall back to default rendering).
//   Kept the react-markdown render path, remark-gfm/remarkPreventBoldFilenames/remarkUrlToLink plugins, the
//   code-lang normalizer, PreWithCopyButton (WithCopyButton), and MarkdownBlock/MemoizedMarkdownBlock exports.
//   The `.inline-markdown-block` styling lives in ./cline.css (token-mapped to our theme; imports the hljs palette).
import { marked } from 'marked'
import type { ComponentProps } from 'react'
import React, { memo, useMemo, useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeHighlight, { type Options } from 'rehype-highlight'
import remarkGfm from 'remark-gfm'
import type { Node } from 'unist'
import { visit } from 'unist-util-visit'
import { cn } from '@/lib/utils'
import { WithCopyButton } from './CopyButton'
import './cline.css'

function parseMarkdownIntoBlocks(markdown: string): string[] {
	try {
		const tokens = marked.lexer(markdown)
		return tokens?.map((token) => token.raw)
	} catch {
		return [markdown]
	}
}

const MemoizedMarkdownBlock = memo(
	({ content }: { content: string }) => {
		return (
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
				{content}
			</ReactMarkdown>
		)
	},
	(prevProps, nextProps) => {
		if (prevProps.content !== nextProps.content) return false
		return true
	},
)

MemoizedMarkdownBlock.displayName = 'MemoizedMarkdownBlock'

const MemoizedMarkdown = memo(({ content, id }: { content: string; id: string }) => {
	const blocks = useMemo(() => parseMarkdownIntoBlocks(content), [content])
	return blocks?.map((block, index) => <MemoizedMarkdownBlock content={block} key={`${id}-block_${index}`} />)
})

MemoizedMarkdown.displayName = 'MemoizedMarkdown'

interface MarkdownBlockProps {
	markdown?: string
	compact?: boolean
	showCursor?: boolean
}

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
	return (
		<div className="inline-markdown-block">
			<span
				className={cn('inline [&>p]:mt-0', {
					'inline-cursor-container': showCursor,
					'[&>p]:m-0': compact,
				})}>
				{markdown ? <MemoizedMarkdown content={markdown} id="markdown-block" /> : markdown}
			</span>
		</div>
	)
})

export default MarkdownBlock
