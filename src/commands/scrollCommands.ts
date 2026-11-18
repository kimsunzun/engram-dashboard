// ADR-0242: the in-window command that invokes scroll-follow of chat · text slots (RichSlot · DomSlot) — `slot.scrollToBottom`.
//   The handle is found in the module map (`followRegistry`) that the slot's hook raised on mount, and the state is owned by that hook
//   (ADR-0055 — the registry is not a state authority). The human 「to bottom」 button and the LLM (`__engramCmd`) shake the same `pin()`.
//
// ★Do not attach `help` — do not put it on the bus★ (ADR-0167): scroll position is frontend state that exists separately per window.
//   The window the bus envelope goes to is chosen by the shell, so a window that does not own that slot may receive it, and that window has no handle so it answers a live
//   slot with 「not found」. The canonical source of the routing reason = the head comment of `renderModeCommands.ts`.

import { t } from '../i18n'
import { getFollow } from '../components/slot/scrollFollow/followRegistry'
import { register } from './registry'

register({
  id: 'slot.scrollToBottom',
  title: () => t('slot.scrollToBottom'),
  category: 'slot',
  // Return value = `{ pinned: true }`. For a hidden tab, only set the pinned state and do the scroll write when that tab is shown.
  run: args => {
    const cmd = 'slot.scrollToBottom'
    const slotId = args?.slotId
    if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId required`)
    const handle = getFollow(slotId)
    if (!handle) throw new Error(`[${cmd}] this window has no conversation or text view of slot '${slotId}'`)
    handle.pin()
    return { pinned: true }
  },
})
