// TRD S21-slot-docking §3-9 "web": the waiting-mode watch of this window's webview (`watchSpareMode`, one per window). A window of the prewarmed pool (hidden) is drawn in waiting mode (empty frame), the rest as normal.
// Listening attaches with `target` = its own label — the body has no window label, and the default `Any` would also receive events sent to another window (same clause as `src/commands/viewCommandBridge.ts`). `window_role` is asked after attaching, so events arriving between are not missed.
// The mode is one-way (waiting → normal) — the `window_role` answer and `spare:adopted` come by different paths with no order; a late `spare` answer overwriting an adoption seen first would leave the adopted window an empty frame.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { SpareAdopted } from '../../../src-tauri/bindings/SpareAdopted'
import type { WindowRoleReply } from '../../../src-tauri/bindings/WindowRoleReply'

/** `spare` = waiting mode (tab bar + empty canvas · 0-tab self-close off · no tab pull-in · measuring reports as usual) · `normal` = normal. */
export type WindowMode = 'spare' | 'normal'

/**
 * Mode notice. `generation` = the adoption number this page received (`spare:adopted` body · `window_role` answer — I26); the shell's first-screen door accepts only notices of the adopted window carrying it. `null` = not received; `0` is also a number.
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
 * A `window_role` answer arriving after an adoption was seen is not notified (one-way). If `window_role` fails → `normal`.
 * If `spare:adopted` listening cannot be attached, it does not enter waiting mode — it asks with `adoptable: false` so the shell closes the pool's window, and whatever the answer is → `normal` (I28 — a waiting window without listening stays empty even when adopted). The same value may be notified repeatedly. After cleanup it neither notifies nor asks.
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
      console.error(`[spareMode] ${EVT_SPARE_ADOPTED} subscription failed — not entering waiting mode (adoptable: false):`, err)
    }
    // A cleaned-up watch does not ask — asking with `adoptable: false` would make the shell close that window even if StrictMode's second watch attached listening.
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
      console.warn(`[spareMode] ${CMD_WINDOW_ROLE} failed — drawing as normal:`, err)
    }
    if (!disposed && !adopted) onMode(change)
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = null
  }
}

// The body comes from another process, so its shape is not trusted — a handler throwing on a broken body would leave the adopted window an empty frame; if only the number is lost it shows at the shell deadline.
function generationOf(body: unknown): number | null {
  const generation = (body as { generation?: unknown } | null)?.generation
  return typeof generation === 'number' ? generation : null
}
