// 대기 입력 목록 — 턴 도중에 보낸 글이 에이전트에 넘어가기 전까지 입력창 위에 회색으로 선다(ADR-0231).
//
// ★머리줄이 없다 — 턴·도구 상태를 읽지 않는다★(PRD §3-2): 무엇을 그릴지는 받은 항목만 정한다.
// ★✕ 는 명령을 디스패치할 뿐 스스로 감추지 않는다★: 항목이 빠지는 것은 데몬 명부의 결말이 링 사건으로 돌아와
//   누산기가 그 항목을 뺄 때다 — 창마다 감춤 상태를 두면 다른 창·앱 재시작과 갈린다. 거절(입력 임대)이면
//   사건이 없어 항목이 그대로 남는 것이 맞다. 글을 거두지 못한 경우도 따로 알리지 않는다.
// ★✕ 의 모양은 명부 상태가 정한다★(ADR-0235): 취소 요청이 답을 기다리는 행은 ✕ 를 누를 수
//   없고(행은 그대로 남는다), 통로가 에이전트에 넘긴 행(`sent`)은 ✕ 가 없다 — 넘긴 글은 거둘 수 없고 받음으로만
//   빠진다. 못 뺐다는 답(`not_removed`)이 온 행도 넘긴 행처럼 그린다(✕ 없음) — 글은 이미 넘어갔고 ✕ 를 다시
//   눌러도 할 수 있는 일이 없다.

import { useState } from 'react'
import { X } from 'lucide-react'

import { fireAndForget } from '../../commands/dispatch'
import { t } from '../../i18n'
import type { QueuedEntry } from './queuedInputReducer'

export interface QueuedInputListProps {
  /** ✕ 가 겨누는 에이전트. */
  agentId: string
  /**
   * 그릴 항목 — 든 순서(가장 오래된 것이 앞). 누산기 `snapshotQueued()` 를 그대로 준다(이미 말풍선으로 그린
   * uuid 는 거기서 빠진다). 취소 대기 항목도 그린다 — 답을 기다리는 동안 ✕ 만 잠긴다.
   */
  entries: readonly QueuedEntry[]
  // ★아래 셋은 사용자가 「나중에 조정」하기로 한 값이다★ — 조정이 이 기본값 한 자리에서 끝나게 둔다.
  /** 접힌 동안 보이는 개수. 이보다 많으면 나머지를 「외 N개」로 접는다. */
  collapsedCount?: number
  /** 접힌 동안 보이는 쪽. */
  collapsedSide?: 'oldest' | 'newest'
  /** 처음부터 펼쳐 둘지. 펼침은 이 창의 그리기 상태다 — 목록이 비어 이 컴포넌트가 내려가면 다시 접힌다. */
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
        onClick={() => setExpanded(true)}
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
      // 펼친 목록이 길어도 입력창을 밀어내지 않게 높이를 묶는다(대화 영역이 줄어드는 쪽).
      className="flex max-h-40 flex-col gap-0.5 overflow-y-auto px-2 pt-1.5"
    >
      {collapsedSide === 'newest' && more}
      {shown.map((entry) => {
        // 취소 요청이 답을 기다린다 — 못 뺐다는 답(`not_removed`)이 오면 넘긴 행처럼 선다(글이 이미 넘어갔다).
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
                onClick={() => fireAndForget('agent.cancelQueuedInput', { agentId, inputId: entry.id })}
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
