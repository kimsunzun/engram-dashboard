// ADR-0055: the registry is not a state authority — these five only route to viewStore's render mode actions (0 new state paths). Human clicks · the palette · the LLM (__engramCmd) run the same command (§5 single control surface).
// ★Unlike the sibling slot.*, these take no backend authority loop★: renderModeOverride is a frontend-only override, so no invoke→emit (ADR-0035; the exception's contract = the JSDoc of the renderModeOverride field in viewStore). Their return is always undefined — no completion point to await.
// ★★Do not put them on the bus — adding `help` allows writes to a window that does not own the slot★★ (ADR-0167): renderModeOverride is frontend state **per window (webview)**; sibling slot.* go through invoke to **one backend authority**, but these five write to **the calling window's** store. The envelope's host window is chosen by the shell at delivery and moves while the app runs (`Registered::repick` in `src-tauri/src/view_commands.rs`), so the write can land in a window that does not own the slot and answer `ok` with the screen unchanged — read as success.
// ★Window coordinates as an argument are no escape hatch★: the dispatcher always sends to the host alone and ignores that argument; delivering to a designated window needs a routing change, which is alternative 1 that the ADR rejected (its "structurally impossible" overstates — listeners and reports already exist per window; only selection is unimplemented). The only blocker now is the **absence of `help`** (`offeredCommands` in `commands/viewCommandBridge.ts` splits boarding by that field).

import { t } from '../i18n'
import { RENDER_MODES, type RenderMode } from '../components/slot/renderMode'
import { useViewStore } from '../store/viewStore'
import { matchDeclaredSpelling } from './enumArg'
import { register } from './registry'

/** The execution context argument of the render mode commands (a single bag, ADR-0055). */
interface RenderModeCtx {
  slotId?: unknown
  mode?: unknown
}

/**
 * ★Does not take viewId★: the override key is the slot node.id (`LayoutLeaf`'s `renderModeOverride[slotId]`) and slotId is a globally unique UUID across windows (ADR-0035 invariant), so viewId narrows nothing; requiring it like the sibling slot.* would make the caller (LLM) fill a value the target action never writes.
 * ★Unknown fields are not rejected (extra keys ignored)★ — ADR-0055 "arguments = one object (a bag); each handler destructures only the keys it needs". So if these ids are **later** contributed to the slot menu (no contribution now — no registerSlotMenu call here), the menu's execution bag `{viewId, slotId, agentId, content}` (`commandArgs` of `SlotContextMenu`) fits as is. Do not transplant ADR-0157's "reject any field not in the declaration" — that list must be **derived from the argument declaration**, and these five attach no `help`, so there is nothing to derive from.
 */
function requireSlotId(args: RenderModeCtx | undefined, cmd: string): string {
  const slotId = args?.slotId
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId required`)
  return slotId
}

/**
 * ★An invalid mode throws, never a silent no-op★: the store's `setRenderMode` warns and ignores an invalid mode, so passing it through would look like `undefined` = success (the bus bridge also replies without an error — `viewCommandBridge.settle`). Same judgment as `registry.run` throwing on an unknown id and the sibling slot.* coordinate guard.
 * ★Not the ADR-0157 "unknown field" rule★ — that tells apart a **field whose name is unknown**; this catches a known field whose **value is outside the valid set**.
 * ★Case is folded, the valid set is left as is★ — whatever the caller writes, what reaches the store is an element of `RENDER_MODES` (rule for command argument enum words in general); an unknown word still throws, so an invalid value never reaches the store's `isRenderMode` guard.
 */
function requireMode(args: RenderModeCtx | undefined, cmd: string): RenderMode {
  const mode = matchDeclaredSpelling(args?.mode, RENDER_MODES)
  if (!mode) {
    throw new Error(
      `[${cmd}] mode must be one of ${RENDER_MODES.join('|')} — got: ${JSON.stringify(args?.mode)}`,
    )
  }
  return mode
}

register({
  id: 'slot.renderMode.set',
  title: () => t('slot.renderModeSet'),
  category: 'slot',
  run: args => {
    const cmd = 'slot.renderMode.set'
    // Call the store *after* checking both arguments — so there is no partial application (only mode invalid but state already written).
    const slotId = requireSlotId(args, cmd)
    const mode = requireMode(args, cmd)
    useViewStore.getState().setRenderMode(slotId, mode)
  },
})

register({
  id: 'slot.renderMode.clear',
  title: () => t('slot.renderModeClear'),
  category: 'slot',
  run: args => {
    useViewStore.getState().clearRenderMode(requireSlotId(args, 'slot.renderMode.clear'))
  },
})

// ── DOM mode aliases (= thin wrappers over renderMode 'dom' — the only domain state is the one renderModeOverride) ──────────

register({
  id: 'slot.domMode.enable',
  title: () => t('slot.domModeEnable'),
  category: 'slot',
  run: args => {
    useViewStore.getState().enableDomMode(requireSlotId(args, 'slot.domMode.enable'))
  },
})

register({
  id: 'slot.domMode.disable',
  title: () => t('slot.domModeDisable'),
  category: 'slot',
  run: args => {
    useViewStore.getState().disableDomMode(requireSlotId(args, 'slot.domMode.disable'))
  },
})

register({
  id: 'slot.domMode.toggle',
  title: () => t('slot.domModeToggle'),
  category: 'slot',
  // ★The toggle decision is made by the store★ — reading the current value here and branching would make the registry a second state interpreter (ADR-0055); read-decide-write must finish inside one store action.
  run: args => {
    useViewStore.getState().toggleDomMode(requireSlotId(args, 'slot.domMode.toggle'))
  },
})
