// ADR-0239: 채팅 뷰의 창 안 command — 지금은 도구 묶음 펼치기 · 접기 `chat.toolGroup.setExpanded` 하나.
//   상태는 `store/toolGroupStore.ts` 가 소유하고 여기서는 인자를 검문해 그 `setOpen` 으로 라우팅만 한다(ADR-0055 —
//   레지스트리는 상태 권위가 아니다).
//
// ★`help` 를 달지 않는다 — 버스에 올리지 말 것★(ADR-0167): 펼침 상태는 창마다 따로 있는 프론트 상태다. 버스 봉투가
//   갈 창은 셸이 고르므로 그 슬롯을 소유하지 않은 창이 받을 수 있고, 그 창에서는 살아 있는 슬롯에 「없다」로 답한다.
//   라우팅 사유의 정본 = `renderModeCommands.ts` 머리 주석.

import { t } from '../i18n'
import { isToolGroupKey } from '../components/slot/chat/toolRuns'
import { useToolGroupStore } from '../store/toolGroupStore'
import { register } from './registry'

register({
  id: 'chat.toolGroup.setExpanded',
  title: t('chat.toolGroupSetExpanded'),
  category: 'chat',
  // 인자 = `{ slotId, groupKey, expanded }` · `groupKey` = 묶음 뿌리의 DOM `data-tool-group` 값 · 돌려주는 값 = `{ expanded }`.
  run: args => {
    const cmd = 'chat.toolGroup.setExpanded'
    const slotId = args?.slotId
    if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
    const groupKey = args?.groupKey
    if (typeof groupKey !== 'string' || !isToolGroupKey(groupKey)) {
      // 모양만 본다 — 접두 없이 백엔드 호출 id 를 넘기는 실수는 잡지만, 모양이 맞고 그리는 묶음이 없는 키(묶음 머리가
      //   아닌 호출의 id 등)는 그대로 적히고 성공으로 답한다. 키가 그려져 있는지는 이 창의 렌더만 안다.
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
