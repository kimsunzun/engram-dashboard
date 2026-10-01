// ADR-0242: 「맨 아래로」 버튼 — 따라가기가 풀려 있고 스크롤할 것이 있으면(U3) 잠깐 뒤 뜨고, 누르면 곧바로
//   바닥에 붙인다.
//   ScrollArea 의 자식으로 둔다: absolute 기준이 seam 의 Root(relative)라 스크롤되지 않는다(`ui/scroll-area.tsx` 헤더).

import { useEffect, useState } from 'react'
import { ChevronDown } from 'lucide-react'

import { t } from '../../../i18n'
import { keepFocusInSlot } from '../slotFocus'
import { JUMP_BUTTON_MODE, type ScrollFollow } from './useScrollFollow'

/** t3code 선례 — 탭 전환 같은 짧은 풀림에 버튼이 깜빡이지 않게 한다. */
export const JUMP_BUTTON_DELAY_MS = 150

export function JumpToBottom({
  follow,
}: {
  follow: Pick<ScrollFollow, 'pinned' | 'unseenGrowth' | 'scrollable' | 'pin'>
}) {
  const free = JUMP_BUTTON_MODE === 'whenFree' ? !follow.pinned : !follow.pinned && follow.unseenGrowth
  // 떨어진 채 뷰포트가 커지거나 내용이 줄어 다 들어오면 이미 「바닥」이다 — 붙음은 다시 재지 않으므로 여기서 가린다.
  const wanted = free && follow.scrollable
  const [shown, setShown] = useState(false)

  useEffect(() => {
    if (!wanted) return
    const timer = setTimeout(() => setShown(true), JUMP_BUTTON_DELAY_MS)
    return () => {
      clearTimeout(timer)
      setShown(false)
    }
  }, [wanted])

  if (!wanted || !shown) return null
  return (
    <button
      type="button"
      data-jump-to-bottom="1"
      aria-label={t('slot.scrollToBottom')}
      title={t('slot.scrollToBottom')}
      // 즉시 쓰기다 — 부드러운 스크롤은 중간 scroll 이벤트를 만들어 코어가 사용자 스크롤로 읽는다.
      // ADR-0237: 붙으면 이 버튼이 곧바로 사라진다 — 쥔 포커스를 칸 안(뷰포트)에 남겨 Esc 가 그 칸에 닿게 한다.
      onClick={(e) => {
        follow.pin()
        keepFocusInSlot(e.currentTarget)
      }}
      // bottom-7 · 가운데 — 옛 이름 라벨이 영역 바닥을 덮던 때 정한 높이다. 라벨은 이제 영역 밖 대기 줄(WaitStrip)에 서서
      //   덮지 않는다 — 버튼을 내릴지는 보이는 변화라 사용자 몫으로 남겼다(2026-10-01).
      className="absolute bottom-7 left-1/2 z-10 -translate-x-1/2 rounded-full border border-border bg-surface p-1 text-muted shadow-sm hover:text-foreground"
    >
      <ChevronDown className="size-4" />
    </button>
  )
}
