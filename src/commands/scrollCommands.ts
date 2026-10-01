// ADR-0242: 채팅·텍스트 슬롯(RichSlot · DomSlot)의 스크롤 따라가기를 부르는 창 안 command — `slot.scrollToBottom`.
//   손잡이는 슬롯의 훅이 마운트하며 올린 모듈 맵(`followRegistry`)에서 찾고, 상태는 그 훅이 소유한다
//   (ADR-0055 — 레지스트리는 상태 권위가 아니다). 사람의 「맨 아래로」 버튼과 LLM(`__engramCmd`)이 같은 `pin()` 을 흔든다.
//
// ★`help` 를 달지 않는다 — 버스에 올리지 말 것★(ADR-0167): 스크롤 위치는 창마다 따로 있는 프론트 상태다.
//   버스 봉투가 갈 창은 셸이 고르므로 그 슬롯을 소유하지 않은 창이 받을 수 있고, 그 창엔 손잡이가 없어 살아
//   있는 슬롯에 「없다」로 답한다. 라우팅 사유의 정본 = `renderModeCommands.ts` 머리 주석.

import { t } from '../i18n'
import { getFollow } from '../components/slot/scrollFollow/followRegistry'
import { register } from './registry'

register({
  id: 'slot.scrollToBottom',
  title: t('slot.scrollToBottom'),
  category: 'slot',
  // 돌려주는 값 = `{ pinned: true }`. 숨은 탭이면 붙음만 세우고 쓰기는 그 탭이 보일 때 한다.
  run: args => {
    const cmd = 'slot.scrollToBottom'
    const slotId = args?.slotId
    if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
    const handle = getFollow(slotId)
    if (!handle) throw new Error(`[${cmd}] 이 창에 슬롯 '${slotId}' 의 대화·텍스트 뷰가 없다`)
    handle.pin()
    return { pinned: true }
  },
})
