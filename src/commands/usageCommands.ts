// Commands of the usage slot (TRD S21 usage-limit-slot §1-8) — refresh and the per-vendor display toggle. The ⟳ of the small display (all enabled vendors) · the per-vendor ⟳ in the popup (`vendor` argument — that one vendor · ADR-0259) · the popup's display toggle row call these.
// ★Does not contribute to the slot right-click menu★ — refresh is carried by the ⟳ of the small display · popup, the display toggle by the popup.
// ★Do not attach `help` (TRD §3 #26)★ — it would go on the bus and overlap the daemon's `usage.refresh`. The outside path (LLM · CLI) already exists: query = `usage.get`/`usage.refresh`, display = the show_* fields of `layout.setSlotContent`. Inside the window call these three as is with `__engramCmd.run(id, {viewId, slotId, content})` (refresh with `vendor` added is one vendor).
// ADR-0258
// ★A display toggle writes only its own field★ — from the slot content the caller gave, it sends one absolute value with that field flipped to `set_usage_slot`, and the shell keeps the other fields at their current values inside the lock. Building and sending both fields (full replacement) would revert another vendor's toggle made in succession before the broadcast to the old value — the webview's content is stale until then. Being absolute, pressing the same field twice before the broadcast sends the same value twice (the second does not revert the first).

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
 * Usage content where both fields are boolean — the ⟳ of all enabled vendors reads both fields; the ⟳ and toggle of one vendor read only their own field, but a single argument contract is kept.
 */
function requireUsageContent(args: CommandArgs | undefined, cmd: string): UsageContent {
  const content = args?.content as Partial<Record<string, unknown>> | undefined
  if (content?.type !== 'usage') {
    throw new Error(`[${cmd}] usage slot content required (got: ${JSON.stringify(args?.content)})`)
  }
  if (typeof content.show_claude !== 'boolean' || typeof content.show_codex !== 'boolean') {
    throw new Error(`[${cmd}] content's show_claude·show_codex must both be booleans (got: ${JSON.stringify(content)})`)
  }
  return content as UsageContent
}

register({
  id: 'usageSlot.refresh',
  title: () => t('usage.refreshAll'),
  category: 'usage',
  run: args => {
    const cmd = 'usageSlot.refresh'
    const content = requireUsageContent(args, cmd)
    const usage = useUsageStore.getState()
    // Not swallowed as a silent no-op — the caller would read it as "done". Both ⟳ block the press when there is nowhere to send (`UsageSlot`), so what reaches here is only the gap where a rejection arrived after drawing and before the press; that throw is caught by the `fireAndForget` the button calls and left in the log.
    const vendor = args?.vendor
    if (vendor !== undefined) {
      if (!isVendor(vendor)) {
        throw new Error(`[${cmd}] vendor must be one of ${Object.keys(SHOW_FIELD).join('·')} (got: ${JSON.stringify(vendor)})`)
      }
      if (!content[SHOW_FIELD[vendor]]) throw new Error(`[${cmd}] ${vendor} is turned off in this slot`)
      if (blocksRefresh(usage.vendors[vendor]?.snapshot.state)) {
        throw new Error(`[${cmd}] ${vendor} is being refused — no refresh before the deadline`)
      }
      return usage.refresh(vendor)
    }
    const targets = refreshableVendors(content, usage.vendors)
    if (targets.length === 0) throw new Error(`[${cmd}] no vendor to refresh — no vendor is on, or all are being refused`)
    return Promise.all(targets.map(target => usage.refresh(target))).then(() => undefined)
  },
})

function registerToggle(id: string, vendor: AgentBackendKind): void {
  register({
    id,
    title: () => t('usage.showVendor', { vendor: vendor === 'claude' ? t('usage.vendorClaude') : t('usage.vendorCodex') }),
    category: 'usage',
    run: args => {
      const viewId = args?.viewId
      const slotId = args?.slotId
      if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${id}] viewId required`)
      if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${id}] slotId required`)
      const current = requireUsageContent(args, id)
      const shows = vendor === 'claude' ? { show_claude: !current.show_claude } : { show_codex: !current.show_codex }
      return useViewStore.getState().setUsageSlot(viewId, slotId, shows)
    },
  })
}

registerToggle('usageSlot.toggleClaude', 'claude')
registerToggle('usageSlot.toggleCodex', 'codex')
