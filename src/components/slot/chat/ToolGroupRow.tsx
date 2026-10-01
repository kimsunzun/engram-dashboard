// ADR-0239: 도구 묶음 한 줄 — 요약 머리와 펼침 · 접힘. 대화 뷰에서 펼침 상태를 읽고 바꾸는 것은 이 컴포넌트뿐이다 —
//   `StructuredTextView` 는 state · effect · 스토어 구독이 없는 순수 렌더로 남는다(ADR-0050).
//   ★안쪽에 `ChatRow` 레일을 두지 않는다★(ADR-0051): 묶음이 레일 한 행이고 레일 위치는 행 목록으로 계산된다 — 멤버마다
//   레일을 그리면 계산과 DOM 이 갈린다. 멤버 행은 `renderMember` 로 받는다 — 행 컴포넌트는 `StructuredTextView` 안에
//   있고, 그 파일을 여기서 import 하면 순환이다.

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
  /** 벤더 결과 본문이 오류인 호출 id(`vendorErrorIdsOf`) — 한 렌더에 한 번 지어 모든 묶음이 나눠 쓴다. */
  vendorErrorIds: ReadonlySet<string>
  runPos: RailRunPosition | undefined
  /** 이 묶음이 목록의 마지막 묶음인가 — `onGroupToggle` 에 그대로 실린다. */
  isLast: boolean
  /** 머리 밑 멤버 한 줄(펼쳤을 때 · 도는 동안의 마지막 호출) — 레일 없는 행 컴포넌트를 돌려준다. */
  renderMember: (item: StructuredItem) => ReactNode
  /** 펼침 상태의 슬롯 — 없으면 고른 값이 없는 묶음으로 그리고 머리를 눌러도 바뀌지 않는다. */
  slotId?: string
  /** 사람이 머리를 눌러 펼쳤다 — 접을 때와 command 로 바뀔 때는 부르지 않는다. */
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
  // ADR-0263 결정 1: 고른 값이 없으면 도는 동안 지금 도는 호출(마지막 호출) 한 줄을 머리 밑에 두고, 끝나면 머리만 남긴다.
  //   고른 값은 양방향으로 이긴다 — 도는 중에 접으면 그 한 줄도 없다.
  const shown = open ? row.members : chosen === undefined && row.live ? row.calls.slice(-1) : []
  // ADR-0241: 요약 셈은 펼친 행 배지와 같은 판정(`toolCallVerdict`)이다 — 거부는 오류 수에 안 든다.
  const parts = summaryParts(summarizeGroup(row.calls, vendorErrorIds, DECLINED_MARK))
  const toggle = () => {
    if (slotId === undefined) return
    const next = !open
    // ★펼침을 바꾸는 길은 스토어 `setOpen` 하나다★ — 사람 토글과 LLM command 가 같은 길을 탄다(CLAUDE.md 「LLM-우선 제어」).
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
          {/* ADR-0263: 진행형 표시는 `live` 가 정한다. 요약 칸 밖에 두어 좁은 폭에서 잘리는 쪽은 요약이다. */}
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
