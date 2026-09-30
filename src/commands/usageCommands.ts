// 사용량 슬롯의 command(TRD S21 usage-limit-slot §1-8) — ⟳ 와 회사별 표시 토글. 작은 표시의 ⟳ 버튼과 팝업의 표시
//   토글 줄이 이것을 부른다.
//
// ★슬롯 우클릭 메뉴에 기여하지 않는다(사용자 결정 2026-09-29)★ — 새로고침은 작은 표시의 ⟳, 표시 토글은 팝업이 진다.
// ★`help` 를 달지 않는다(TRD §3 #26)★ — 달면 버스에 올라 데몬의 `usage.refresh` 와 겹친다. 밖(LLM·CLI)의 길은 이미
//   있다: 조회 = `usage.get`/`usage.refresh`, 표시 = `layout.setSlotContent` 의 show_* 칸. 창 안에서는
//   `__engramCmd.run(id, {viewId, slotId, content})` 로 이 셋을 그대로 부른다.
// ADR-0258
// ★표시 토글은 제 칸 하나만 쓴다★ — 부른 쪽이 준 슬롯 내용에서 그 칸을 뒤집은 절댓값 하나를 `set_usage_slot` 으로
//   보내고, 다른 칸은 셸이 락 안에서 지금 값으로 지킨다. 두 칸을 다 지어 보내면(전량 교체) 방송 전에 연달은 다른 회사
//   토글을 옛 값으로 되돌린다 — 웹뷰의 content 는 방송이 닿기 전엔 낡았다. 절댓값이라 방송 전에 같은 칸을 두 번
//   누르면 같은 값이 두 번 간다(둘째 누름이 첫째를 되돌리지 않는다).

import type { SlotContent } from '../api/layoutTypes'
import type { AgentBackendKind } from '../api/types'
import { t } from '../i18n'
import { refreshableVendors, useUsageStore } from '../store/usageStore'
import { useViewStore } from '../store/viewStore'
import { register, type CommandArgs } from './registry'

type UsageContent = Extract<SlotContent, { type: 'usage' }>

/** 두 칸이 다 boolean 인 사용량 content — ⟳ 는 두 칸을 다 읽는다. 토글은 제 칸만 읽지만 인자 계약을 ⟳ 와 하나로 둔다. */
function requireUsageContent(args: CommandArgs | undefined, cmd: string): UsageContent {
  const content = args?.content as Partial<Record<string, unknown>> | undefined
  if (content?.type !== 'usage') {
    throw new Error(`[${cmd}] 사용량 슬롯의 content 필요(받음: ${JSON.stringify(args?.content)})`)
  }
  if (typeof content.show_claude !== 'boolean' || typeof content.show_codex !== 'boolean') {
    throw new Error(`[${cmd}] content 의 show_claude·show_codex 는 둘 다 boolean 이어야 함(받음: ${JSON.stringify(content)})`)
  }
  return content as UsageContent
}

register({
  id: 'usageSlot.refresh',
  title: t('usage.refreshAll'),
  category: 'usage',
  run: args => {
    const cmd = 'usageSlot.refresh'
    const usage = useUsageStore.getState()
    const targets = refreshableVendors(requireUsageContent(args, cmd), usage.vendors)
    // 조용한 no-op 으로 삼키지 않는다 — 부른 쪽이 「됐다」로 읽는다. 작은 표시의 ⟳ 는 대상이 없으면 누름을 막으므로
    //   (`UsageSlot`) 거기서 여기 닿는 것은 그린 뒤 누르기 전에 거절이 닿은 틈뿐이고, 그 throw 는 버튼이 부르는
    //   `fireAndForget` 이 잡아 로그로 남긴다.
    if (targets.length === 0) throw new Error(`[${cmd}] 새로고침할 회사가 없다 — 켠 회사가 없거나 전부 거절 중`)
    return Promise.all(targets.map(vendor => usage.refresh(vendor))).then(() => undefined)
  },
})

function registerToggle(id: string, vendor: AgentBackendKind): void {
  register({
    id,
    title: t('usage.showVendor', { vendor: vendor === 'claude' ? t('usage.vendorClaude') : t('usage.vendorCodex') }),
    category: 'usage',
    run: args => {
      const viewId = args?.viewId
      const slotId = args?.slotId
      if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${id}] viewId 필요`)
      if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${id}] slotId 필요`)
      const current = requireUsageContent(args, id)
      const shows = vendor === 'claude' ? { show_claude: !current.show_claude } : { show_codex: !current.show_codex }
      return useViewStore.getState().setUsageSlot(viewId, slotId, shows)
    },
  })
}

registerToggle('usageSlot.toggleClaude', 'claude')
registerToggle('usageSlot.toggleCodex', 'codex')
