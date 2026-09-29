// 사용량 슬롯 메뉴(TRD S21 usage-limit-slot §1-8 · PRD R8) — ⟳ 와 회사별 표시 토글, 그리고 그 기여.
//
// ★`help` 를 달지 않는다(TRD §3 #26)★ — 달면 버스에 올라 데몬의 `usage.refresh` 와 겹친다. 밖(LLM·CLI)의 길은 이미
//   있다: 조회 = `usage.get`/`usage.refresh`, 표시 = `layout.setSlotContent` 의 show_* 칸. 창 안에서는
//   `__engramCmd.run(id, {viewId, slotId, content})` 로 이 셋을 그대로 부른다.
// ★표시 토글은 전량 교체다★ — 메뉴 ctx 의 슬롯 내용에서 한 칸만 뒤집어 두 칸을 다 써 보낸다. 빠진 칸을 지금 값으로
//   채우는 병합은 버스 쪽(`layout.setSlotContent`)의 몫이다(TRD §3 #20).

import type { SlotContent } from '../api/layoutTypes'
import type { AgentBackendKind } from '../api/types'
import { t } from '../i18n'
import { blocksRefresh, useUsageStore } from '../store/usageStore'
import { useViewStore } from '../store/viewStore'
import { register, type CommandArgs } from './registry'
import { registerSlotMenu, type SlotMenuCtx } from './slotMenu'

type UsageContent = Extract<SlotContent, { type: 'usage' }>

const VENDORS: readonly AgentBackendKind[] = ['claude', 'codex']

function shows(content: UsageContent, vendor: AgentBackendKind): boolean {
  return vendor === 'claude' ? content.show_claude : content.show_codex
}

function usageContentOf(args: CommandArgs | undefined): UsageContent | null {
  const content = args?.content as SlotContent | undefined
  return content?.type === 'usage' ? content : null
}

function requireUsageContent(args: CommandArgs | undefined, cmd: string): UsageContent {
  const content = usageContentOf(args)
  if (!content) throw new Error(`[${cmd}] 사용량 슬롯의 content 필요(받음: ${JSON.stringify(args?.content)})`)
  return content
}

/** 켠 회사 중 보이는 거절이 아닌 것. 값을 아직 못 받은 회사도 든다 — ⟳ 가 첫 값을 부르는 길이다. */
function refreshTargets(content: UsageContent): AgentBackendKind[] {
  const vendors = useUsageStore.getState().vendors
  return VENDORS.filter(vendor => shows(content, vendor) && !blocksRefresh(vendors[vendor]?.snapshot.state))
}

register({
  id: 'usageSlot.refresh',
  title: t('usage.menuRefresh'),
  category: 'usage',
  when: args => {
    const content = usageContentOf(args)
    return content !== null && refreshTargets(content).length > 0
  },
  run: args => {
    const cmd = 'usageSlot.refresh'
    const targets = refreshTargets(requireUsageContent(args, cmd))
    // 조용한 no-op 으로 삼키지 않는다 — 메뉴에선 `when` 이 이 자리를 비활성으로 막고, 여기 닿는 것은 직접 호출뿐이다.
    if (targets.length === 0) throw new Error(`[${cmd}] 새로고침할 회사가 없다 — 켠 회사가 없거나 전부 거절 중`)
    const usage = useUsageStore.getState()
    return Promise.all(targets.map(vendor => usage.refresh(vendor))).then(() => undefined)
  },
})

function registerToggle(id: string, vendor: AgentBackendKind): void {
  register({
    id,
    title: t('usage.menuShow', { vendor: vendor === 'claude' ? t('usage.vendorClaude') : t('usage.vendorCodex') }),
    category: 'usage',
    run: args => {
      const viewId = args?.viewId
      const slotId = args?.slotId
      if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${id}] viewId 필요`)
      if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${id}] slotId 필요`)
      const current = requireUsageContent(args, id)
      const next: UsageContent = {
        type: 'usage',
        show_claude: vendor === 'claude' ? !current.show_claude : current.show_claude,
        show_codex: vendor === 'codex' ? !current.show_codex : current.show_codex,
      }
      return useViewStore.getState().setSlotContent(viewId, slotId, next)
    },
  })
}

registerToggle('usageSlot.toggleClaude', 'claude')
registerToggle('usageSlot.toggleCodex', 'codex')

function checkedFor(vendor: AgentBackendKind): (ctx: SlotMenuCtx) => boolean {
  return ctx => ctx.content?.type === 'usage' && shows(ctx.content, vendor)
}

registerSlotMenu('usage', [
  { commandId: 'usageSlot.refresh', group: 'content', order: 10 },
  { commandId: 'usageSlot.toggleClaude', group: 'content', order: 20, checked: checkedFor('claude') },
  { commandId: 'usageSlot.toggleCodex', group: 'content', order: 30, checked: checkedFor('codex') },
])
