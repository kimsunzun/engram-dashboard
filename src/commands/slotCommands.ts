// ADR-0064 / ADR-0055 / ADR-0035: each command receives the execution context (viewId · slotId) through run(args) and only routes to a viewStore
//   action (0 new state paths — the layout authority is the backend ViewManager, ADR-0035). Human right click ·
//   palette · keybindings · the LLM (__engramCmd) all run the same command with the context argument (§5 single control surface).
//
// ★Common = '*' single source★ (ADR-0064 invariant): these items attach to every slot via registerSlotMenu('*', …) —
//   content components do not redeclare them (redeclaring = drift, review reject). Content-specific items go in each content module.
//
// ★Popup detach = promoted to common★ (ADR-0064): the old menu enabled 'detach to popup' only when a live agent existed, but
//   after the slot content union (ADR-0060), a slot must be movable to another window regardless of content kind →
//   remove the agent gating and move it to '*' (common).

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
  if (typeof viewId !== 'string' || viewId.length === 0) throw new Error(`[${cmd}] viewId 필요`)
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
  return { viewId, slotId }
}

function registerSplit(id: string, title: string, dir: SplitDir): void {
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

// ★Label↔direction wiring = vim convention (ADR-0140)★: `가로 분할` = a horizontal line (a horizontal split line) appears, dividing into top/bottom →
//   `top_bottom`. Reading only the label word it looks reversed, but flipping it would break the convention the user chose
//   (reverting to the tmux axis convention = the alternative ADR-0140 rejected). This is the only point where the two vocabularies meet.
registerSplit('slot.split.topBottom', t('slot.splitTopBottom'), 'top_bottom')
registerSplit('slot.split.leftRight', t('slot.splitLeftRight'), 'left_right')

register({
  id: 'slot.focus',
  title: t('slot.focus'),
  category: 'slot',
  // ADR-0066: click-to-focus — viewStore.focusSlot → invoke(focus_slot) → emit(layout:updated) updates the ring
  //   (no optimistic update, §5 single control surface). The slot onClick of LayoutLeaf calls the same handle.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.focus')
    return useViewStore.getState().focusSlot(viewId, slotId)
  },
})

register({
  id: 'slot.popout',
  title: t('slot.popout'),
  category: 'slot',
  // ADR-0057/0064: MOVE (detach) a slot to a new tab of a new runtime popup window — viewStore.moveSlotToWindow →
  //   invoke(move_slot_to_window, toWindow=null). Independent of content kind (agent gating removed, ADR-0064).
  // ★Known transitional split — the same id receives and gives different things on two surfaces★: the shell's command bus also has
  //   `slot.popout` (`src-tauri/src/layout/commands.rs`). What it receives — here there is no destination argument so it is always
  //   a new window (the call below does not pass toWindow), whereas that side can also pick an existing window with to_window. What it gives — here it is
  //   `{window, tab, window_id, number}`, there `{window, new_view_id, window_id, number}`. Do not fix only one side — joining is the job of the follow-up step that
  //   retires this registry. The same memo is attached on the other side.
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.popout')
    return useViewStore.getState().moveSlotToWindow(viewId, slotId)
  },
})

register({
  id: 'slot.empty',
  title: t('slot.empty'),
  category: 'slot',
  help: {
    summary: '그 슬롯을 빈 칸으로 되돌린다(슬롯 자체는 남는다 — 없애려면 slot.close).',
    effect: 'write',
    args: {
      viewId: { type: 'string', description: '탭 id(UUID) — tab.list 가 준다.' },
      slotId: { type: 'string', description: '슬롯 id(UUID) — slot.resolveSpatial 이 준다.' },
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
  title: t('slot.close'),
  category: 'slot',
  // ADR-0035: viewStore.closeSlot → invoke(close_slot) (sibling promotion).
  run: args => {
    const { viewId, slotId } = requireCoords(args, 'slot.close')
    return useViewStore.getState().closeSlot(viewId, slotId)
  },
})

register({
  id: 'slot.resolveSpatial',
  title: t('slot.resolveSpatial'),
  category: 'slot',
  // ★Spatial/directional token → slot id resolution (ADR-0068 — §5 backend authority resolver)★: maps a spatial reference such as "bottom right" · "right of this slot"
  //   to a slot id through derivation from the logical plan (split direction · ratio) (independent of pixels · getBoundingClientRect —
  //   the layout truth is the backend ViewManager, ADR-0035). The frontend does not compute but only invokes the backend resolve_spatial
  //   (no optimistic frontend computation — §5 single control surface). A relative direction is based on the target View's focused_slot_id.
  run: args => {
    const token = args?.token
    if (typeof token !== 'string' || token.length === 0) throw new Error('[slot.resolveSpatial] token 필요')
    const viewId = typeof args?.viewId === 'string' && args.viewId.length > 0 ? args.viewId : undefined
    // If viewId is not specified, pass this webview window (main · popup label) so the backend targets that window's active tab.
    const window = viewId ? undefined : readWindowLabelFromHash()
    return invoke<string | null>('resolve_spatial', { token, window: window ?? null, viewId: viewId ?? null })
  },
})

// ★Common slot ops contribution = '*' (all slots) single source★ (ADR-0064). group='slot-ops' (rendered below the content items).
//   Close is by convention at the very bottom, so order 99.
// ★hideOn trim (ADR-0065)★: slot.empty (re-emptying an empty slot = no-op) is meaningless on an empty slot, so it is
//   excluded with hideOn:['empty']. The '*' universal registration is kept (common ops single-source invariant) — it is a subtraction filter,
//   not a per-content-type redeclaration (see the rejected alternative of ADR-0065).
// ADR-0228: slot.popout gets no hideOn — an empty slot is also a popout target (user decision).
registerSlotMenu('*', [
  { commandId: 'slot.split.topBottom', group: 'slot-ops', order: 10 },
  { commandId: 'slot.split.leftRight', group: 'slot-ops', order: 20 },
  { commandId: 'slot.popout', group: 'slot-ops', order: 30 },
  { commandId: 'slot.empty', group: 'slot-ops', order: 40, hideOn: ['empty'] },
  { commandId: 'slot.close', group: 'slot-ops', order: 99 },
])
