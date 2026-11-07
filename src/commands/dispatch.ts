// ADR-0055 ★Separation of the two execution paths★:
//   - fireAndForget(id, args)  ← human UI clicks · keybindings (FIX-4): the result does not matter, a failure is only a warn. Swallowed.
//   - run(id, args)            ← cdp/__engramCmd/await call sites: expose the return · throw as is (await · error observation).
//   Click/keybinding consumers must reuse this helper (do not reimplement the safety net — this file is the copy template).

import { runAsHuman } from './registry'
import type { CommandArgs } from './registry'

/**
 * For consumers that do not await the result, such as click/keybinding.
 * - Synchronous throw: catch with try/catch and warn (so the listener does not die).
 * - Asynchronous reject: swallow with Promise.resolve(result).catch — it does not narrow with instanceof Promise, so
 *   it also covers thenable/cross-realm Promises (FIX-3). run() itself is not touched, so await call sites lose nothing.
 */
export function fireAndForget(id: string, args?: CommandArgs): void {
  try {
    // ★It is `runAsHuman`, not `registry.run`★: the callers of this path are human clicks · keybindings, so they
    //   do not pass the `humanOnly` gate (the doc of that field is canonical). Reverting this to `run` would block the path by which a human
    //   creates codex from the tree menu — the gate was aimed only at the LLM path.
    const result = runAsHuman(id, args)
    Promise.resolve(result).catch((err) => console.warn(`[commands] '${id}' 실패(async):`, err))
  } catch (err) {
    console.warn(`[commands] '${id}' 실패:`, err)
  }
}
