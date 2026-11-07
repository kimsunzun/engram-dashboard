// Commands of the usage slot (TRD S21 usage-limit-slot §1-8) — refresh and the per-vendor display toggle. The ⟳ of the small display (all enabled vendors) ·
//   the per-vendor ⟳ in the popup (`vendor` argument — that one vendor · ADR-0259) · the display toggle row in the popup call these.
//
// ★Does not contribute to the slot right-click menu (user decision 2026-09-29)★ — refresh is carried by the ⟳ of the small display · popup, and the display toggle by the popup.
// ★Do not attach `help` (TRD §3 #26)★ — if attached it goes on the bus and overlaps the daemon's `usage.refresh`. The path from outside (LLM · CLI) already
//   exists: query = `usage.get`/`usage.refresh`, display = the show_* fields of `layout.setSlotContent`. Inside the window,
//   call these three as is with `__engramCmd.run(id, {viewId, slotId, content})` (refresh with `vendor` added is one vendor).
// ADR-0258
// ★A display toggle writes only its own field★ — from the slot content the caller gave, it sends one absolute value with that field flipped to `set_usage_slot`,
//   and the shell keeps the other fields at their current values inside the lock. If both fields were built and sent (full replacement), another vendor's
//   toggle made in succession before the broadcast would be reverted to the old value — the webview's content is stale until the broadcast arrives. Being an absolute value, pressing the same field twice
//   before the broadcast sends the same value twice (the second press does not revert the first).

import type { SlotContent } from '../api/layoutTypes'
import type { AgentBackendKind } from '../api/types'
import { t } from '../i18n'
import { blocksRefresh, refreshableVendors, useUsageStore } from '../store/usageStore'
import { useViewStore } from '../store/viewStore'
import { register, type CommandArgs } from './registry'

type UsageContent = Extract<SlotContent, { type: 'usage' }>

/** The slot content field that turns each vendor on and off — also the table that tells apart the `vendor` argument (when vendors grow, the type catches the missing line). */
const SHOW_FIELD: Record<AgentBackendKind, 'show_claude' | 'show_codex'> = {
  claude: 'show_claude',
  codex: 'show_codex',
}

function isVendor(value: unknown): value is AgentBackendKind {
  return typeof value === 'string' && Object.prototype.hasOwnProperty.call(SHOW_FIELD, value)
}

/**
 * Usage content where both fields are boolean — the ⟳ of all enabled vendors reads both fields. The ⟳ and toggle of one vendor read only their own field, but
 * we keep a single argument contract.
 */
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
    const content = requireUsageContent(args, cmd)
    const usage = useUsageStore.getState()
    // Do not swallow it as a silent no-op — the caller would read it as 「됐다」 (done). Both ⟳ block the press when there is nowhere to send,
    //   (`UsageSlot`) so what reaches here from there is only the gap where a rejection arrived after drawing and before the press, and that throw is
    //   caught by the `fireAndForget` the button calls and left in the log.
    const vendor = args?.vendor
    if (vendor !== undefined) {
      if (!isVendor(vendor)) {
        throw new Error(`[${cmd}] vendor 는 ${Object.keys(SHOW_FIELD).join('·')} 중 하나(받음: ${JSON.stringify(vendor)})`)
      }
      if (!content[SHOW_FIELD[vendor]]) throw new Error(`[${cmd}] ${vendor} 는 이 슬롯에서 꺼져 있다`)
      if (blocksRefresh(usage.vendors[vendor]?.snapshot.state)) {
        throw new Error(`[${cmd}] ${vendor} 는 거절 중이다 — 기한 안엔 새로고침하지 않는다`)
      }
      return usage.refresh(vendor)
    }
    const targets = refreshableVendors(content, usage.vendors)
    if (targets.length === 0) throw new Error(`[${cmd}] 새로고침할 회사가 없다 — 켠 회사가 없거나 전부 거절 중`)
    return Promise.all(targets.map(target => usage.refresh(target))).then(() => undefined)
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
