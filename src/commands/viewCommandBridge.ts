// Tells the shell this window's commands and passes envelopes the shell sends down into the registry (TRD §6 Step 4).
// ★Not a second control surface★: execution still goes through the single `registry.run(id, args)` (ADR-0055 §5); no new global handle or registry — only the **outside path** to that entrance grows.
// ★Calls invoke/listen directly★: an exception to "components and stores depend only on agentClient" — a backend-authority surface uses Tauri directly (as `theme/uiSettings.ts` · `store/viewStore.ts`; the command registry's owner is the daemon and the shell relays).
// ★The shell decides which window receives the envelope★: every window's App reports but the shell sends to one (`src-tauri/src/view_commands.rs`, "why the delivery target is one window"). Put no window-picking code here, or two places would believe in different destinations.

import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { list, run, type CommandHelp } from './registry'

/** Shell → this window. Comes only to the window picked as the destination. */
const EVT_COMMAND_REQUEST = 'command:request'
/** Boot report — once per window. */
const CMD_REPORT_VIEW_COMMANDS = 'report_view_commands'
/** Outcome retrieval — exactly once per envelope. */
const CMD_REPORT_OUTCOME = 'report_command_outcome'

interface CommandRequest {
  request_id: string
  name: string
  args: unknown
}

/** The entry sent to the shell — only the name and shape (titles · keybindings belong to the screen so they are not carried). */
export interface OfferedCommand {
  name: string
  help: CommandHelp
}

/**
 * Commands to offer outside — ★only those with `help`★. That field is itself the mark of "callable from outside" (`Command.help` in `registry.ts`); a separate list could diverge from the declaration — putting a command on the bus is adding its `help` field.
 * ★Do not filter the names the shell and daemon answer here★: shell table names are filtered once on the Rust side (`reserved_names` in `src-tauri/src/view_commands.rs`; a copy here would diverge). Names the daemon answers are filtered by no one — a `help` on such a name makes the daemon bounce the whole registration packet (ADR-0270 decision 2).
 * ★`humanOnly` ones are left out too — but that is **not the closing mechanism**★: the real block is the single `registry.run` (the bridge's `settle` calls it for any name); leaving them out only avoids advertising a name that will surely be bounced. Do not read this filter as a gate.
 */
export function offeredCommands(): OfferedCommand[] {
  return list()
    .filter((cmd): cmd is typeof cmd & { help: CommandHelp } => Boolean(cmd.help?.summary?.trim()))
    .filter(cmd => !cmd.humanOnly)
    .map(cmd => ({ name: cmd.id, help: cmd.help }))
}

/** Send it back so the shell can correlate the outcome — ★exactly once, success or failure★. */
async function answer(requestId: string, ok: unknown, error: string | null): Promise<void> {
  try {
    await invoke(CMD_REPORT_OUTCOME, { requestId, ok: ok ?? null, error })
  } catch (e) {
    // If this fails the shell waits until the deadline and answers TIMEOUT — passing it silently leaves the cause nowhere (the window-side log is the only clue).
    console.error(`[viewCommandBridge] failed to reply with the outcome (${requestId}):`, e)
  }
}

/**
 * `run` may return a Promise or a value (the `Command.run` contract in `registry.ts`) — wait for both.
 * ★Values that cannot be carried as JSON are dropped★: the return shape was never contracted (varies per command), so the shell's `ok` is "anything". Reporting a succeeded operation as failed because serialization blew up would be worse.
 */
async function settle(request: CommandRequest): Promise<void> {
  try {
    const args = (request.args ?? {}) as Record<string, unknown> | undefined
    const value = await run(request.name, args)
    await answer(request.request_id, jsonSafe(value), null)
  } catch (e) {
    await answer(request.request_id, null, e instanceof Error ? e.message : String(e))
  }
}

function jsonSafe(value: unknown): unknown {
  try {
    return JSON.parse(JSON.stringify(value ?? null)) as unknown
  } catch {
    return null
  }
}

/**
 * Attach the subscription and report this window's list; returns a disposer (prevents listener accumulation on unmount/HMR).
 * ★Attach the subscription **first**★ — an envelope can arrive as soon as the report fills the shell's registry; with no listener it never reaches this window and hangs until the shell's deadline (same ordering clause as `theme/uiSettings.ts`).
 * ★No retry★ — a failed subscription leaves only this window unable to receive; the shell fixes the destination to one window so other windows are unaffected, and the skipped report keeps the shell from picking this window.
 */
export function installViewCommandBridge(): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined

  void (async () => {
    try {
      // ★★Do not drop `target` — else the same command runs once per window★★: `listen()` defaults to `Any`, and Tauri wakes **every** `Any` listener **regardless of the filter** (`match_any_or_filter`), so even with the shell's `emit_to(label, …)` main · tree · popout all receive and answer the same envelope (a TRD §4-⑤ violation: one reply per `request_id`). Registered with its own label, only the same-label listener among `AnyLabel` ones is hit. ★The label comes from Tauri★ — never infer it from the hash or settings.
      const off = await listen<CommandRequest>(EVT_COMMAND_REQUEST, e => {
        // Discard whatever arrives after cleanup — a dead instance (first StrictMode run · pre-HMR edition) would answer the same envelope twice (the shell leaves the second as a warning).
        if (disposed) return
        void settle(e.payload)
      }, { target: getCurrentWindow().label })
      if (disposed) {
        off()
        return
      }
      unlisten = off
    } catch (e) {
      console.error(
        `[viewCommandBridge] subscribing to ${EVT_COMMAND_REQUEST} failed — this window cannot receive bus commands:`,
        e,
      )
      return
    }

    try {
      await invoke(CMD_REPORT_VIEW_COMMANDS, { commands: offeredCommands() })
    } catch (e) {
      console.warn(`[viewCommandBridge] ${CMD_REPORT_VIEW_COMMANDS} failed:`, e)
    }
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = undefined
  }
}
