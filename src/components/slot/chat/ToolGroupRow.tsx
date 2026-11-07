// ADR-0239: tool bundle one line — summary head and expand · collapse. This component alone reads and changes the expansion state in the conversation view —
//   `StructuredTextView` stays a pure render with no state · effect · store subscription (ADR-0050).
//   ★No `ChatRow` rail inside★ (ADR-0051): the bundle is one rail row and rail positions are computed from the row list — if each member
//   drew a rail, the calculation and the DOM would diverge. Member rows are received via `renderMember` — the row component lives inside `StructuredTextView`,
//   and importing that file here would be circular.

import { Fragment, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, Layers } from 'lucide-react'

import { cn } from '@/lib/utils'
import { t } from '../../../i18n'
import { chosenOpen, useToolGroupStore } from '../../../store/toolGroupStore'
import type { StructuredItem } from '../structuredAccumulator'
import { ChatRow } from './ChatRow'
import type { RailRunPosition } from './railPositions'
import {
  DECLINED_MARK,
  summarizeGroup,
  summaryParts,
  type DisplayRow,
  type SummaryTone,
} from './toolRuns'

export type ToolGroupDisplayRow = Extract<DisplayRow, { kind: 'toolGroup' }>

export interface ToolGroupRowProps {
  row: ToolGroupDisplayRow
  /** Call ids whose vendor result body is an error (`vendorErrorIdsOf`) — built once per render and shared by all bundles. */
  vendorErrorIds: ReadonlySet<string>
  runPos: RailRunPosition | undefined
  /** Is this bundle the last bundle of the list — carried as is in `onGroupToggle`. */
  isLast: boolean
  /** One member line under the head (when expanded · the last call while running) — returns a row component without a rail. */
  renderMember: (item: StructuredItem) => ReactNode
  /** The slot of expansion state — without it the bundle is drawn as one with no chosen value and pressing the head changes nothing. */
  slotId?: string
  /** A human pressed the head to expand — not called on collapse or when changed by a command. */
  onGroupToggle?: (isLast: boolean) => void
}

function toneClass(tone: SummaryTone): string | undefined {
  return tone === 'error' ? 'text-red-500' : tone === 'default' ? 'text-foreground' : undefined
}

export function ToolGroupRow({
  row,
  vendorErrorIds,
  runPos,
  isLast,
  renderMember,
  slotId,
  onGroupToggle,
}: ToolGroupRowProps) {
  const chosen = useToolGroupStore((s) => chosenOpen(s, slotId, row.key))
  const open = chosen ?? false
  // ADR-0263 decision 1: with no chosen value, while running the call running now (the last call) is put as one line under the head, and when done only the head remains.
  //   The chosen value wins both ways — if collapsed while running, that one line is gone too.
  const shown = open ? row.members : chosen === undefined && row.live ? row.calls.slice(-1) : []
  // ADR-0241: the summary count is the same decision as the expanded row badge (`toolCallVerdict`) — a decline does not count toward the error number.
  const parts = summaryParts(summarizeGroup(row.calls, vendorErrorIds, DECLINED_MARK))
  const toggle = () => {
    if (slotId === undefined) return
    const next = !open
    // ★The only way to change expansion is the store `setOpen`★ — the human toggle and the LLM command take the same path (CLAUDE.md 「LLM-우선 제어」).
    if (!useToolGroupStore.getState().setOpen(slotId, row.key, next)) {
      console.warn(`[ToolGroupRow] 슬롯 '${slotId}' 이 묶여 있지 않아 묶음 펼침을 적지 못했다 — 대화 뷰의 bind 가 빠졌다`)
      return
    }
    if (next) onGroupToggle?.(isLast)
  }
  return (
    <ChatRow rail tone="tool" runPos={runPos}>
      <div
        data-tool-group={row.key}
        data-tool-group-open={open ? '1' : '0'}
        data-tool-group-count={row.calls.length}
        data-tool-group-live={row.live ? '1' : '0'}
      >
        <button
          type="button"
          onClick={toggle}
          disabled={slotId === undefined}
          aria-expanded={open}
          className={cn(
            'flex w-full items-center gap-2.5 select-none text-left',
            slotId === undefined ? 'cursor-default' : 'cursor-pointer',
          )}
        >
          {open ? (
            <ChevronDown className="size-3.5 flex-none text-muted" />
          ) : (
            <ChevronRight className="size-3.5 flex-none text-muted" />
          )}
          <Layers className="size-3.5 flex-none text-foreground" />
          <span className="truncate font-bold">
            {parts.map((part, i) => (
              <Fragment key={part.key}>
                {i > 0 && <span className="text-muted"> · </span>}
                <span
                  className={toneClass(part.tone)}
                  style={part.tone === 'declined' ? { color: 'var(--status-blocked)' } : undefined}
                >
                  {t(part.key, { count: String(part.count) })}
                </span>
              </Fragment>
            ))}
          </span>
          {/* ADR-0263: the progressive indicator is decided by `live`. It is put outside the summary area, so what gets cut at narrow width is the summary. */}
          {row.live && (
            <span className="flex-none animate-pulse text-muted">{t('chat.toolGroupRunning')}</span>
          )}
        </button>
        {shown.length > 0 && (
          <div className="mt-1 border-l border-border pl-3">
            {shown.map((item) => (
              <div key={item.itemId} style={{ paddingTop: 'var(--chat-rail-row-pt)' }}>
                {renderMember(item)}
              </div>
            ))}
          </div>
        )}
      </div>
    </ChatRow>
  )
}
