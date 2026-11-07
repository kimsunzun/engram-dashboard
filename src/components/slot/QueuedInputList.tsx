// Queued-input list — text sent mid-turn stands in gray above the input box until it is handed over to the agent (ADR-0231).
//
// ★No head line — it does not read turn or tool state★ (PRD §3-2): only the items received decide what to draw.
// ★✕ only dispatches the command and never hides itself★: an item leaves when the daemon registry's outcome returns as a ring event and
//   the accumulator removes that item — keeping a hidden state per window would diverge across other windows and app restarts. On a refusal (input
//   lease) there is no event, so the item rightly stays. A case where the text could not be withdrawn is not reported separately either.
// ★The shape of ✕ is decided by registry state★ (ADR-0235): a row whose cancel request is waiting for a reply cannot have ✕ pressed
//   (the row stays), and a row the path handed to the agent (`sent`) has no ✕ — handed-over text cannot be withdrawn and only
//   leaves on receipt. A row whose reply said it could not be removed (`not_removed`) is drawn like a handed-over row (no ✕) — the text already
//   went through and pressing ✕ again can do nothing.

import { useState } from 'react'
import { X } from 'lucide-react'

import { fireAndForget } from '../../commands/dispatch'
import { t } from '../../i18n'
import type { QueuedEntry } from './queuedInputReducer'
import { keepFocusInSlot } from './slotFocus'

export interface QueuedInputListProps {
  /** The agent ✕ aims at. */
  agentId: string
  /**
   * Items to draw — in the order held (oldest first). Pass the accumulator's `snapshotQueued()` as is (a uuid already
   * drawn as a bubble is dropped there). Cancel-pending items are drawn too — only ✕ is locked while waiting for the reply.
   */
  entries: readonly QueuedEntry[]
  // ★The three below are values the user decided to 「나중에 조정」 (adjust later)★ — so adjusting ends at this one place for defaults.
  /** Number visible while collapsed. If there are more, the rest fold into 「외 N개」 (N more). */
  collapsedCount?: number
  /** The side visible while collapsed. */
  collapsedSide?: 'oldest' | 'newest'
  /** Whether to start expanded. Expansion is this window's drawing state — when the list empties and this component goes down, it is collapsed again. */
  defaultExpanded?: boolean
}

// ADR-0231
export function QueuedInputList({
  agentId,
  entries,
  collapsedCount = 3,
  collapsedSide = 'oldest',
  defaultExpanded = false,
}: QueuedInputListProps) {
  const [expanded, setExpanded] = useState(defaultExpanded)
  if (entries.length === 0) return null

  const collapsed = !expanded && entries.length > collapsedCount
  const hidden = collapsed ? entries.length - collapsedCount : 0
  const shown = !collapsed
    ? entries
    : collapsedSide === 'oldest'
      ? entries.slice(0, collapsedCount)
      : entries.slice(entries.length - collapsedCount)
  const more = hidden > 0 && (
    <li>
      <button
        type="button"
        data-queued-more="1"
        // ADR-0237: expanding makes this button vanish — leave the held focus inside the area so Esc reaches that area.
        onClick={(e) => {
          setExpanded(true)
          keepFocusInSlot(e.currentTarget)
        }}
        className="rounded px-1 text-[12px] text-muted hover:text-foreground focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent"
      >
        {t('chat.queuedMore', { count: String(hidden) })}
      </button>
    </li>
  )

  return (
    <ul
      data-queued-inputs="1"
      aria-label={t('chat.queuedListLabel')}
      // Binds the height so a long expanded list does not push the input box out (the conversation area is what shrinks).
      className="flex max-h-40 flex-col gap-0.5 overflow-y-auto px-2 pt-1.5"
    >
      {collapsedSide === 'newest' && more}
      {shown.map((entry) => {
        // A cancel request is waiting for a reply — if the reply says it could not be removed (`not_removed`), it stands like a handed-over row (the text already went through).
        const asking = entry.phase.state === 'cancelling' && entry.phase.answer === 'none'
        const handedOver =
          entry.sent || (entry.phase.state === 'cancelling' && entry.phase.answer === 'not_removed')
        return (
          <li
            key={entry.id}
            data-queued-input={entry.id}
            data-queued-state={handedOver ? 'sent' : asking ? 'cancelling' : 'queued'}
            className="flex min-w-0 items-center gap-1"
          >
            <span title={entry.text} className="min-w-0 flex-1 truncate text-[12px] text-muted">
              {entry.text}
            </span>
            {!handedOver && (
              <button
                type="button"
                title={t('chat.queuedRemove')}
                aria-label={t('chat.queuedRemove')}
                disabled={asking}
                // ADR-0237: this button will soon lock (waiting for the cancel reply) or leave with its row — leave the held focus inside the area so Esc reaches
                //   that area.
                onClick={(e) => {
                  fireAndForget('agent.cancelQueuedInput', { agentId, inputId: entry.id })
                  keepFocusInSlot(e.currentTarget)
                }}
                className="flex-none rounded p-0.5 text-muted hover:text-foreground disabled:opacity-40 disabled:hover:text-muted focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent"
              >
                <X size={12} aria-hidden="true" />
              </button>
            )}
          </li>
        )
      })}
      {collapsedSide === 'oldest' && more}
    </ul>
  )
}
