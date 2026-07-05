// Originally from Cline (https://github.com/cline/cline) — Copyright Cline Bot Inc. Licensed under Apache-2.0.
// See LICENSES/cline-Apache-2.0.txt. Modified by Netmarble F&C / Engram, 2026.
// Changes: copied verbatim (no VSCode wiring); MarkdownBlock import points to our ported copy.
import { memo } from 'react'
import MarkdownBlock from './MarkdownBlock'

export const MarkdownRow = memo(({ markdown, showCursor }: { markdown?: string; showCursor?: boolean }) => {
	return (
		<div className="wrap-anywhere overflow-hidden [&_p]:mb-0">
			<MarkdownBlock markdown={markdown} showCursor={showCursor} />
		</div>
	)
})

MarkdownRow.displayName = 'MarkdownRow'
