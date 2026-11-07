// TRD S21-slot-docking §3-9 「웹」: the waiting-mode watch of this window's webview. Entry point = `watchSpareMode` — one per window. A window of the prewarmed window pool
//   (hidden prewarmed window) is drawn in waiting mode (empty frame), the rest as normal.
// Listening is attached with `target` = its own label — the body has no window label, and attaching with the default target `Any` would also receive events sent to another window
//   (the same clause of `src/commands/viewCommandBridge.ts`). `window_role` is asked after attaching listening — so events arriving in between are not missed.
// The mode is one direction (waiting → normal) — the `window_role` answer and `spare:adopted` come by different paths and have no order. If a late answer `spare`
//   overwrote an adoption seen first, the adopted window would stay an empty frame.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { SpareAdopted } from '../../../src-tauri/bindings/SpareAdopted'
import type { WindowRoleReply } from '../../../src-tauri/bindings/WindowRoleReply'

/** `spare` = waiting mode (tab bar + empty canvas · 0-tab self-close off · no tab pull-in · measuring reports as usual) · `normal` = normal. */
export type WindowMode = 'spare' | 'normal'

/**
 * Mode notice. `generation` = the adoption number this page received (`spare:adopted` body · `window_role` answer — I26) — the shell first screen door receives only the notices of the adopted
 * window that carry this number. `null` = not received. `0` is also a number.
 */
export interface ModeChange {
  mode: WindowMode
  generation: number | null
}

/** Shell `spare::wire::EVT_SPARE_ADOPTED` — body = `SpareAdopted`. */
export const EVT_SPARE_ADOPTED = 'spare:adopted'
/** Shell `commands::spare::window_role` — argument `{ adoptable }` = `spare:adopted` listening was attached. */
export const CMD_WINDOW_ROLE = 'window_role'

/**
 * Start receiving this window's mode — notified through `onMode`: one `window_role` answer, and `normal` + that number on every `spare:adopted`.
 * A `window_role` answer that arrives after an adoption was seen is not notified (one direction). If `window_role` fails it is `normal` (the same drawing as before waiting mode).
 * If `spare:adopted` listening cannot be attached, it does not enter waiting mode — it asks with `adoptable: false` so the shell closes the pool's window,
 * and whatever the answer is it is `normal` (I28 — a waiting window without listening stays empty even when adopted). The same value may be notified repeatedly. After
 * cleanup it neither notifies nor asks.
 *
 * @returns cleanup function — returned immediately without waiting for registration.
 */
// ADR-0295
// ADR-0299
export function watchSpareMode(onMode: (change: ModeChange) => void): () => void {
  let disposed = false
  let adopted = false
  let unlisten: UnlistenFn | null = null
  const label = getCurrentWindow().label

  void (async () => {
    let listening = false
    try {
      const off = await listen<SpareAdopted>(
        EVT_SPARE_ADOPTED,
        e => {
          if (disposed) return
          adopted = true
          onMode({ mode: 'normal', generation: generationOf(e.payload) })
        },
        { target: label },
      )
      if (disposed) {
        off()
        return
      }
      unlisten = off
      listening = true
    } catch (err) {
      console.error(`[spareMode] ${EVT_SPARE_ADOPTED} 구독 실패 — 대기 모드로 들지 않는다(adoptable: false):`, err)
    }
    // A cleaned-up watch does not ask — if it asks with `adoptable: false`, the shell closes that window even if StrictMode's second watch attached listening.
    if (disposed) return
    let change: ModeChange = { mode: 'normal', generation: null }
    try {
      const reply = await invoke<WindowRoleReply>(CMD_WINDOW_ROLE, { adoptable: listening })
      if (reply.role === 'spare') {
        if (listening) change = { mode: 'spare', generation: null }
      } else if (reply.role === 'model') {
        change = { mode: 'normal', generation: generationOf(reply) }
      }
    } catch (err) {
      console.warn(`[spareMode] ${CMD_WINDOW_ROLE} 실패 — 보통으로 그린다:`, err)
    }
    if (!disposed && !adopted) onMode(change)
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = null
  }
}

// The body is a value from another process, so its shape is not trusted — if the handler throws on a broken body, the adopted window stays an empty frame. If only the number is lost,
//   it is shown at the shell deadline.
function generationOf(body: unknown): number | null {
  const generation = (body as { generation?: unknown } | null)?.generation
  return typeof generation === 'number' ? generation : null
}
