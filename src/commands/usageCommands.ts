// 사용량 슬롯 메뉴(TRD S21 usage-limit-slot §1-8 · PRD R8) — ⟳ 와 회사별 표시 토글, 그리고 그 기여.
//
// ★`help` 를 달지 않는다(TRD §3 #26)★ — 달면 버스에 올라 데몬의 `usage.refresh` 와 겹친다. 밖(LLM·CLI)의 길은 이미
//   있다: 조회 = `usage.get`/`usage.refresh`, 표시 = `layout.setSlotContent` 의 show_* 칸. 창 안에서는
//   `__engramCmd.run(id, {viewId, slotId, content})` 로 이 셋을 그대로 부른다.
// ★표시 토글은 전량 교체다★ — 메뉴 ctx 의 슬롯 내용에서 한 칸만 뒤집어 두 칸을 다 써 보낸다. 빠진 칸을 지금 값으로
//   채우는 병합은 `layout.setSlotContent`(버스 · `__engramCmd` 양쪽)의 몫이다(TRD §3 #20).

import type { SlotContent } from '../api/layoutTypes'
import type { AgentBackendKind } from '../api/types'
import { t } from '../i18n'
import { refreshableVendors, useUsageStore } from '../store/usageStore'
import { useViewStore } from '../store/viewStore'
import { register, type CommandArgs } from './registry'
import { registerSlotMenu, SLOT_MENU_ORIGIN, type SlotMenuCtx } from './slotMenu'

type UsageContent = Extract<SlotContent, { type: 'usage' }>

function shows(content: UsageContent, vendor: AgentBackendKind): boolean {
  return vendor === 'claude' ? content.show_claude : content.show_codex
}

/** 두 칸이 다 boolean 인 사용량 content — 토글이 한 칸을 뒤집어 두 칸을 다 써 보내므로 하나라도 빠지면 못 짓는다. */
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
  title: t('usage.menuRefresh'),
  category: 'usage',
  run: args => {
    const cmd = 'usageSlot.refresh'
    const usage = useUsageStore.getState()
    const targets = refreshableVendors(requireUsageContent(args, cmd), usage.vendors)
    if (targets.length === 0) {
      // ★메뉴에서 왔으면 건너뛰고 직접 호출이면 throw 한다★ — 메뉴는 대상이 없으면 이 항목을 비활성으로 그리므로(아래
      //   `enabled`) 여기 닿는 것은 그린 뒤 누르기 전에 거절이 닿은 틈뿐이고, 사람에게 오류로 돌려줄 일이 아니다.
      //   직접 호출(`__engramCmd`)은 조용한 no-op 으로 삼키지 않는다 — 부른 쪽이 「됐다」로 읽는다.
      if (args?.origin === SLOT_MENU_ORIGIN) {
        console.debug(`[${cmd}] 누른 때 새로고침할 회사가 없다 — 건너뜀`)
        return undefined
      }
      throw new Error(`[${cmd}] 새로고침할 회사가 없다 — 켠 회사가 없거나 전부 거절 중`)
    }
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
  { commandId: 'usageSlot.refresh', group: 'content', order: 10, enabled: ctx => ctx.usageRefreshable === true },
  { commandId: 'usageSlot.toggleClaude', group: 'content', order: 20, checked: checkedFor('claude') },
  { commandId: 'usageSlot.toggleCodex', group: 'content', order: 30, checked: checkedFor('codex') },
])
