// ADR-0239: the in-window command of the chat view — for now just one, `chat.toolGroup.setExpanded`, which expands · collapses a tool group.
//   The state is owned by `store/toolGroupStore.ts`; here we only vet the arguments and route to its `setOpen` (ADR-0055 —
//   the registry is not the state authority).
//
// ★Do not attach `help` — do not put it on the bus★ (ADR-0167): expanded state is frontend state that exists separately per window. The window the bus envelope
//   goes to is chosen by the shell, so a window that does not own that slot may receive it, and that window answers a live slot with 「없다」 (not found).
//   The canonical source of the routing reason = the head comment of `renderModeCommands.ts`.

import { t } from '../i18n'
import { isToolGroupKey } from '../components/slot/chat/toolRuns'
import { useToolGroupStore } from '../store/toolGroupStore'
import { register } from './registry'

register({
  id: 'chat.toolGroup.setExpanded',
  title: t('chat.toolGroupSetExpanded'),
  category: 'chat',
  // Arguments = `{ slotId, groupKey, expanded }` · `groupKey` = the DOM `data-tool-group` value of the group root · return value = `{ expanded }`.
  run: args => {
    const cmd = 'chat.toolGroup.setExpanded'
    const slotId = args?.slotId
    if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
    const groupKey = args?.groupKey
    if (typeof groupKey !== 'string' || !isToolGroupKey(groupKey)) {
      // Only the shape is checked — a mistake of passing a backend call id without the prefix is caught, but a key whose shape fits and has no group drawn (the id of a call
      //   that is not a group head, etc.) is recorded as is and answers success. Whether the key is drawn is known only by this window's render.
      throw new Error(
        `[${cmd}] groupKey 는 묶음의 data-tool-group 값(tool:… 또는 item:…)이어야 한다 — 받은 값: ${JSON.stringify(groupKey)}`,
      )
    }
    const expanded = args?.expanded
    if (typeof expanded !== 'boolean') {
      throw new Error(`[${cmd}] expanded 는 true|false 여야 한다 — 받은 값: ${JSON.stringify(expanded)}`)
    }
    if (!useToolGroupStore.getState().setOpen(slotId, groupKey, expanded)) {
      throw new Error(`[${cmd}] 이 창에 지금 마운트된 슬롯 '${slotId}' 의 대화 뷰가 없다`)
    }
    return { expanded }
  },
})
