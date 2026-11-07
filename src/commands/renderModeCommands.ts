// ADR-0055: the registry is not a state authority — these five only route to the render mode actions of viewStore
//   (0 new state paths). Human clicks · the palette · the LLM (__engramCmd) run the same command (§5 single control surface).
//
// ★Unlike the sibling slot.*, these do not take the backend authority loop★: renderModeOverride is a frontend-only override, so
//   it does not go through invoke→emit (ADR-0035) (the contract of that exception = the JSDoc of the renderModeOverride field in viewStore).
//   So the return of these five is always undefined — there is no completion point for call sites to await.
//
// ★★Do not put them on the bus — adding `help` makes writes possible to a window that does not own the slot★★ (ADR-0167)
//   This is the practical consequence of the item above. renderModeOverride is frontend state that exists **separately per window (webview)**. The changes of the sibling slot.*
//   go through invoke and are written to **one backend authority**, but these five write to **the calling window's** store.
//   Yet the window (host) the envelope goes to is chosen by the shell at delivery time and moves around even while the app runs
//   (`Registered::repick` in `src-tauri/src/view_commands.rs`), so it can happen that it is
//   recorded in the store of a window that does not own that slot and answers `ok` with the screen changed in no way — the caller reads it as success.
//   ★Attaching window coordinates as an argument is not an escape hatch★ — the dispatcher always sends to the host alone and
//   ignores that argument. To deliver to the designated window the routing implementation would have to change, and that is alternative 1 that ADR rejected
//   (the 「구조상 불가능」 (structurally impossible) written there is a stronger phrase than the code — listeners and reports already exist per window and only the unimplemented part is
//   the selection. Measured 2026-08-23). The only thing blocking this now is the **absence of `help`**
//   (`offeredCommands` in `commands/viewCommandBridge.ts` splits boarding by that one field).

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
 * ★Does not take viewId★: the override key is the slot node.id (the `LayoutLeaf`'s
 * `renderModeOverride[slotId]`), and slotId is a globally unique UUID across windows (ADR-0035 invariant), so there is nothing to narrow with viewId.
 * If we required here the viewId that the sibling slot.* requires, the caller (LLM) would have to
 * find and fill a value the target action does not even write.
 *
 * ★Unknown fields are not rejected (extra keys ignored)★ — exactly the 「인자 = 객체 하나(가방), 각 handler 가
 * 필요한 키만 destructure」 (arguments = one object (a bag); each handler destructures only the keys it needs) of ADR-0055. Thanks to that, if these ids are **later** contributed to the slot menu (there is
 * no contribution now — this file has no registerSlotMenu call), the execution bag `{viewId, slotId, agentId, content}` the menu passes
 * (`commandArgs` of `SlotContextMenu`) fits as is. Do not transplant here the 「입구에서 선언에 없는 칸을 거절한다」 (reject at the entrance any field not in the declaration) of ADR-0157 —
 * that rejection list must be **derived from the argument declaration** (same ADR invariant: no hand-maintained
 * copy), and these five attach no `help`, so there is no declaration to derive from.
 */
function requireSlotId(args: RenderModeCtx | undefined, cmd: string): string {
  const slotId = args?.slotId
  if (typeof slotId !== 'string' || slotId.length === 0) throw new Error(`[${cmd}] slotId 필요`)
  return slotId
}

/**
 * ★An invalid mode throws — it is not swallowed as a no-op★. The store's `setRenderMode` warns and ignores an invalid mode,
 * so if we pass it through without filtering, a call where nothing happened looks like a `undefined` return = success
 * (the bus bridge also replies with an outcome without an error — `viewCommandBridge.settle`). The same judgment is already
 * planted at this registry's entry point: the ground for `registry.run` throwing on an unknown id is 「조용한 no-op 은 LLM/cdp
 * 디버깅을 어렵게 함」 (a silent no-op makes LLM/cdp debugging hard), and the coordinate guard of the sibling slot.* also throws at the same place.
 *
 * ★This is not the 「모르는 칸」 (unknown field) rule (ADR-0157)★ — what that rule tells apart is a **field whose name is unknown**,
 * and what is caught here is a known field whose **value is outside the valid set**.
 *
 * ★Case is folded and the valid set is left as is★ — whatever the caller writes, what goes to the store is an element of `RENDER_MODES`
 * (user decision 2026-09-23 — the rule for command argument enum words in general). Even after folding, an unknown word still
 * throws here, so an invalid value never reaches the `isRenderMode` guard of the store below.
 */
function requireMode(args: RenderModeCtx | undefined, cmd: string): RenderMode {
  const mode = matchDeclaredSpelling(args?.mode, RENDER_MODES)
  if (!mode) {
    throw new Error(
      `[${cmd}] mode 는 ${RENDER_MODES.join('|')} 중 하나여야 한다 — 받은 값: ${JSON.stringify(args?.mode)}`,
    )
  }
  return mode
}

register({
  id: 'slot.renderMode.set',
  title: t('slot.renderModeSet'),
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
  title: t('slot.renderModeClear'),
  category: 'slot',
  run: args => {
    useViewStore.getState().clearRenderMode(requireSlotId(args, 'slot.renderMode.clear'))
  },
})

// ── DOM mode aliases (= thin wrappers over renderMode 'dom' — the only domain state is the one renderModeOverride) ──────────

register({
  id: 'slot.domMode.enable',
  title: t('slot.domModeEnable'),
  category: 'slot',
  run: args => {
    useViewStore.getState().enableDomMode(requireSlotId(args, 'slot.domMode.enable'))
  },
})

register({
  id: 'slot.domMode.disable',
  title: t('slot.domModeDisable'),
  category: 'slot',
  run: args => {
    useViewStore.getState().disableDomMode(requireSlotId(args, 'slot.domMode.disable'))
  },
})

register({
  id: 'slot.domMode.toggle',
  title: t('slot.domModeToggle'),
  category: 'slot',
  // ★The toggle decision is made by the store★ — reading the current value here and branching would make the registry a second
  //   place that interprets state (ADR-0055 「상태 권위 아님」 — not a state authority). Read-decide-write must finish inside a single store action.
  run: args => {
    useViewStore.getState().toggleDomMode(requireSlotId(args, 'slot.domMode.toggle'))
  },
})
