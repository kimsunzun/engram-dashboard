// ADR-0064 / ADR-0055 / ADR-0035: each command takes the execution context (viewId · slotId) through run(args) and only routes to a viewStore action (0 new state paths — layout authority is the backend ViewManager). Human right click · palette · keybindings · the LLM (__engramCmd) run the same command (§5 single control surface).
// ★Common = '*' single source★ (ADR-0064 invariant): these attach to every slot via registerSlotMenu('*', …); content components do not redeclare them (drift). Content-specific items go in each content module.
// ★Popup detach is common★ (ADR-0064): after the slot content union (ADR-0060) a slot must be movable to another window regardless of content kind, so the agent gating is removed and it lives in '*'.

import { invoke } from '@tauri-apps/api/core'

import { t } from '../i18n'
import { useViewStore, readWindowLabelFromHash } from '../store/viewStore'
import type { SplitDir } from '../api/layoutTypes'
import { register } from './registry'
import { registerSlotMenu } from './slotMenu'

/** The execution context argument of the common slot-op commands (a single bag, ADR-0055). */
interface SlotCtx {
  viewId?: unknown
  slotId?: unknown
}

/** Both must be strings — the backend authority coordinate system. */
function requireCoords(args: SlotCtx | undefined, cmd: string): { viewId: string; slotId: string } {
  const viewId = args?.viewId
  const slotId = args?.slotId
  if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${cmd}] viewId required`)
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId required`)
  return { viewId, slotId }
}

function registerSplit(id: string, title: () => string, dir: SplitDir): void {
  register({
    id,
    title,
    category: 'slot',
    // ADR-0035: viewStore.split → invoke(split_slot) → reflected through emit (no optimistic update).
    run: args => {
      const { viewId, slotId } = requireCoords(args, id)
      return useViewStore.getState().split(viewId, slotId, dir)
    },
  })
}

// ★Label↔direction wiring = vim convention (ADR-0140)★: `Split horizontally` = a horizontal split line dividing top/bottom → `top_bottom`. The label word reads reversed, but flipping it would break the convention the user chose (the tmux axis convention is the alternative ADR-0140 rejected). The only point where the two vocabularies meet.
registerSplit('slot.split.topBottom', () => t('slot.splitTopBottom'), 'top_bottom')
registerSplit('slot.split.leftRight', () => t('slot.splitLeftRight'), 'left_right')

register({
  id: 'slot.focus',
  title: () => t('slot.focus'),
  category: 'slot',
  // ADR-0066: click-to-focus — viewStore.focusSlot → invoke(focus_slot) → emit(layout:updated) updates the ring (no optimistic update); LayoutLeaf's slot onClick calls the same handle.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.focus')
    return useViewStore.getState().focusSlot(viewId, slotId)
  },
})

register({
  id: 'slot.popout',
  title: () => t('slot.popout'),
  category: 'slot',
  // ADR-0057/0064: MOVE (detach) a slot to a new tab of a new runtime popup window — viewStore.moveSlotToWindow → invoke(move_slot_to_window, toWindow=null); independent of content kind.
  // ★Known transitional split — the same id receives and gives different things on two surfaces★: the shell bus also has `slot.popout` (`src-tauri/src/layout/commands.rs`). Receives: here no destination argument, so always a new window (toWindow is not passed), while the bus can pick an existing window with to_window. Gives: here `{window, tab, window_id, number}`, there `{window, new_view_id, window_id, number}`. Do not fix only one side — joining is the follow-up step that retires this registry (same memo on the other side).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.popout')
    return useViewStore.getState().moveSlotToWindow(viewId, slotId)
  },
})

register({
  id: 'slot.empty',
  title: () => t('slot.empty'),
  category: 'slot',
  help: {
    summary: 'Returns that slot to an empty area (the slot itself stays — to remove it, slot.close).',
    effect: 'write',
    args: {
      viewId: { type: 'string', description: 'Tab id (UUID) — given by tab.list.' },
      slotId: { type: 'string', description: 'Slot id (UUID) — given by slot.resolveSpatial.' },
    },
    required: ['viewId', 'slotId'],
  },
  // ADR-0063: viewStore.setSlotContent → invoke → emit.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.empty')
    return useViewStore.getState().setSlotContent(viewId, slotId, { type: 'empty' })
  },
})

register({
  id: 'slot.close',
  title: () => t('slot.close'),
  category: 'slot',
  // ADR-0035: viewStore.closeSlot → invoke(close_slot) (sibling promotion).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.close')
    return useViewStore.getState().closeSlot(viewId, slotId)
  },
})

register({
  id: 'slot.resolveSpatial',
  title: () => t('slot.resolveSpatial'),
  category: 'slot',
  // ★Spatial/directional token → slot id (ADR-0068 — §5 backend authority resolver)★: a spatial reference such as "bottom right" · "right of this slot" is resolved by the backend from the logical plan (split direction · ratio), independent of pixels · getBoundingClientRect (layout truth = backend ViewManager, ADR-0035). The frontend only invokes resolve_spatial — no frontend computation. A relative direction is based on the target View's focused_slot_id.
  run: args => {
    const token = args?.token
    if (typeof token !== 'string' || token.length === 0) throw new Error('[slot.resolveSpatial] token required')
    const viewId = typeof args?.viewId === 'string' && args.viewId.length > 0 ? args.viewId : undefined
    // If viewId is not specified, pass this webview window (main · popup label) so the backend targets that window's active tab.
    const window = viewId ? undefined : readWindowLabelFromHash()
    return invoke<string | null>('resolve_spatial', { token, window: window ?? null, viewId: viewId ?? null })
  },
})

// ★Common slot ops contribution = '*' single source★ (ADR-0064). group='slot-ops' (below the content items); Close is conventionally last, so order 99.
// ★hideOn trim (ADR-0065)★: slot.empty (re-emptying an empty slot is a no-op) is excluded with hideOn:['empty'] — a subtraction filter that keeps the '*' registration (single-source invariant), not a per-content-type redeclaration (ADR-0065 rejected alternative).
// ADR-0228: slot.popout gets no hideOn — an empty slot is also a popout target.
registerSlotMenu('*', [
  { commandId: 'slot.split.topBottom', group: 'slot-ops', order: 10 },
  { commandId: 'slot.split.leftRight', group: 'slot-ops', order: 20 },
  { commandId: 'slot.popout', group: 'slot-ops', order: 30 },
  { commandId: 'slot.empty', group: 'slot-ops', order: 40, hideOn: ['empty'] },
  { commandId: 'slot.close', group: 'slot-ops', order: 99 },
])
