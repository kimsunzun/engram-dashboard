// ADR-0064 / ADR-0060 / ADR-0011: empty·agent are core content types with no separate feature module
//   (unlike agent_list=agentCommands, which has its own module), so their
//   dedicated commands and contributions are gathered here in one place. Every item attaches to the menu only by command id reference (ADR-0064 invariant —
//   no direct store calls from the menu).

import { open } from '@tauri-apps/plugin-dialog'

import { t } from '../i18n'
import { agentClient } from '../api/clientFactory'
import { useMonitoringPickerStore } from '../store/monitoringPickerStore'
import { useViewStore } from '../store/viewStore'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

/** Backend-authoritative coordinates — every fill-op does setSlotContent/assign with these coordinates. */
function requireCoords(
  args: { viewId?: unknown; slotId?: unknown } | undefined,
  cmd: string,
): { viewId: string; slotId: string } {
  const viewId = args?.viewId
  const slotId = args?.slotId
  if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${cmd}] viewId required`)
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId required`)
  return { viewId, slotId }
}

// ── empty slot fill-ops ────────────────────────────────────────────────────────

register({
  id: 'slot.fill.agentList',
  title: () => t('slot.fillAgentList'),
  category: 'slot',
  // ADR-0063: setSlotContent → invoke(set_slot_content) → reflected by emit.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.fill.agentList')
    return useViewStore.getState().setSlotContent(viewId, slotId, { type: 'agent_list' })
  },
})

register({
  id: 'slot.fill.usage',
  title: () => t('slot.fillUsage'),
  category: 'slot',
  // Placed with both vendors on — turning one off is done by the popup's 「Show in slot」 row through `usageSlot.toggle*` (ADR-0258).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.fill.usage')
    return useViewStore
      .getState()
      .setSlotContent(viewId, slotId, { type: 'usage', show_claude: true, show_codex: true })
  },
})

register({
  id: 'slot.createAgentHere',
  title: () => t('agent.create'),
  category: 'slot',
  // ★spawn + assign to this slot★ (ADR-0011 + ADR-0035): spawnAgent is daemon authority, assignAgent is layout authority
  //   invoke — the two authorities are separate. async — passes the Promise on so the caller (cdp/menu) can await it.
  run: async args => {
    const { viewId, slotId } = requireCoords(args, 'slot.createAgentHere')
    // Native OS folder picker window (outside the webview). directory+multiple:false → returns string | null.
    const picked = await open({ directory: true, multiple: false, title: t('dialog.pickAgentCwd') })
    const cwd = typeof picked === 'string' ? picked : null
    if (!cwd) return // cancelled — no-op
    // ★claude is named explicitly★ — the backend field of the spawn packet has no default (the daemon refuses its absence),
    //   so the caller must say what it picked. That this command cannot pick codex is not a blank but a placement matter —
    //   the door where a human picks the backend is on the tree side (`agentlist.create*`), and this command is out of the menu
    //   (ADR-0067 — comment below).
    const agent = await agentClient.spawnAgent(cwd, 'claude')
    // ADR-0035: assignment is a backend-authoritative invoke too (assign_agent) — reflected by emit with no optimistic update.
    return useViewStore.getState().assignAgent(viewId, slotId, agent.id)
  },
})

register({
  id: 'slot.assignRunningAgent',
  title: () => t('agent.monitor'),
  category: 'slot',
  // ★Placement target = the right-clicked slot (explicit)★ (ADR-0067): ctx.viewId/slotId are the coordinates of the right-clicked slot (focus-independent
  //   — focus-steal blocked at the source). Here it only opens the popup at those coordinates; the actual placement (assign_agent) is passed by the popup's
  //   on-select to viewStore.assignAgent (§5 single control surface — no separate placement state). Opening the popup itself
  //   does not touch focused_slot_id (right-click focus invariant).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.assignRunningAgent')
    useMonitoringPickerStore.getState().open(viewId, slotId)
  },
})

// group='content' — rendered above the common slot-ops.
// ★"New content ▶" one-level submenu (ADR-0065)★: folds the content-filling items (tree · usage) into one container to
//   tidy the empty slot menu ("what goes into this area" = separating the content plane). Child order = tree → usage.
//   The commands stay directly callable as the registry single source (§5 unchanged) — the submenu is presentation only.
//   An extension spot where future backend types (codex/gemini) attach as children.
// ADR-0067: `slot.createAgentHere` was removed from this menu — spawning lives in the tree (reserved profile double-click + agent_list menu `agentlist.createAgent`); the definition stays for direct calls · possible future reuse.
registerSlotMenu('empty', [
  {
    id: 'slot.newContent',
    title: () => t('slot.newContent'),
    group: 'content',
    order: 10,
    children: [
      { commandId: 'slot.fill.agentList', group: 'content', order: 10 },
      { commandId: 'slot.fill.usage', group: 'content', order: 30 },
    ],
  },
])

// ADR-0067: "Monitor agent" contribution — shown on empty·agent slots (assigns a running agent to the right-clicked slot).
//   ★hideOn = ['agent_list','usage']★: on the source slot (tree) and the usage slot, "monitor an agent
//   here" is meaningless (the content of those slots is itself UI). The universal '*' registration covers empty·agent together and only these types
//   are taken out by subtraction (ADR-0065 hideOn — not an allowlist, keeps the common single source).
registerSlotMenu('*', [
  {
    commandId: 'slot.assignRunningAgent',
    group: 'content',
    order: 5,
    hideOn: ['agent_list', 'usage'],
  },
])

// ── agent slot only ────────────────────────────────────────────────────────────

register({
  id: 'agent.kill',
  title: () => t('agent.kill'),
  category: 'agent',
  // ADR-0011: the slot stays as is (layout unchanged) and only the agent is killed.
  //   agentId comes from the execution context (ctx) — the menu passes the slot's assigned agentId.
  run: args => {
    const agentId = args?.agentId
    if (typeof agentId !== 'string' || agentId.length === 0) throw new Error('[agent.kill] agentId required')
    return agentClient.killAgent(agentId)
  },
})

registerSlotMenu('agent', [{ commandId: 'agent.kill', group: 'content', order: 10 }])
