// ADR-0242: the 「맨 아래로」 (jump to bottom) button — shown after a short delay when follow is released and there is something to scroll (U3), and pressing it pins
//   to the bottom at once.
//   Placed as a child of ScrollArea: the absolute reference is the seam's Root (relative) so it does not scroll (`ui/scroll-area.tsx` header).

import { useEffect, useState } from 'react'
import { ChevronDown } from 'lucide-react'

import { t } from '../../../i18n'
import { keepFocusInSlot } from '../slotFocus'
import { JUMP_BUTTON_MODE, type ScrollFollow } from './useScrollFollow'

/** t3code precedent — keeps the button from flickering on a short release like a tab switch. */
export const JUMP_BUTTON_DELAY_MS = 150

export function JumpToBottom({
  follow,
}: {
  follow: Pick<ScrollFollow, 'pinned' | 'unseenGrowth' | 'scrollable' | 'pin'>
}) {
  const free = JUMP_BUTTON_MODE === 'whenFree' ? !follow.pinned : !follow.pinned && follow.unseenGrowth
  // If the viewport grows or content shrinks and everything fits while off the bottom, it is already 「바닥」 (the bottom) — the pin is not measured again, so it is hidden here.
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
      // It is an immediate write — a smooth scroll makes intermediate scroll events that the core reads as user scrolling.
      // ADR-0237: once pinned this button vanishes at once — leave the held focus inside the area (viewport) so Esc reaches that area.
      onClick={(e) => {
        follow.pin()
        keepFocusInSlot(e.currentTarget)
      }}
      // bottom-7 · centered — a height set back when the old name label covered the bottom of the region. The label now stands on the wait row (WaitStrip) outside the region and
      //   does not cover it — whether to lower the button is a visible change so it was left to the user (2026-10-01).
      className="absolute bottom-7 left-1/2 z-10 -translate-x-1/2 rounded-full border border-border bg-surface p-1 text-muted shadow-sm hover:text-foreground"
    >
      <ChevronDown className="size-4" />
    </button>
  )
}
