// The wiring that tells the shell this window's commands and passes envelopes the shell sent down into the registry (TRD §6 Step 4).
//
// ★Not a second control surface★: execution still goes through the single `registry.run(id, args)` (ADR-0055 §5). It creates no new
// global handle and no new registry — what grows is only the **outside path** arriving at that entrance.
//
// ★Calls invoke/listen directly★: as an exception to 「컴포넌트·스토어는 agentClient 에만 의존한다」 (components and stores depend only on agentClient), a surface where the backend is
// the authority uses Tauri directly (the same exception used by `theme/uiSettings.ts` · `store/viewStore.ts` — the owner of the command
// registry is the daemon and the shell puts it on its behalf).
//
// ★Which window receives the envelope is decided by the shell★ — this App comes up in every window and all of them report, but the shell sends
// to only one of them (`src-tauri/src/view_commands.rs` 「배달 대상이 창 하나인 이유」). So we put no
// code here that picks a window — if we did, two places would believe different windows to be the destination.

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
 * Commands to offer outside — ★only those with `help`★.
 *
 * That field is itself the mark of 「밖에서 부를 수 있다」 (callable from outside) (`Command.help` in `registry.ts`). The reason we keep no separate list
 * is that if the list and the declaration diverge, only the list falls behind — putting a new command on the bus is a matter of adding the one
 * `help` field to its registration.
 *
 * ★We do not filter the names the shell and daemon answer here★: shell table names are filtered in one place on the Rust side
 * (`reserved_names` in `src-tauri/src/view_commands.rs` — a copy here makes the two lists diverge). Names the daemon answers
 * are filtered by no one — attaching `help` to such a name makes the daemon bounce the whole registration packet (ADR-0270 decision 2).
 *
 * ★Those with `humanOnly` are also left out — though that is **not the means of closing**★. The place that actually blocks is the single `registry.run`
 * (the bridge's `settle` calls it as long as there is a name), and leaving them out here is merely the hygiene of 「부르면 반드시 반려될
 * 이름을 명부에 광고하지 않는다」 (do not advertise on the registry a name that will surely be bounced when called). Do not read this filter as a gate.
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
    // If this fails the shell waits until the deadline and answers with TIMEOUT — passing it silently leaves its cause nowhere
    //   (the window-side log is the only clue).
    console.error(`[viewCommandBridge] 결말 회신 실패 (${requestId}):`, e)
  }
}

/**
 * The return of `run` may be a Promise or a value (the `Command.run` contract in `registry.ts`) — wait for both.
 * ★Values that cannot be carried as JSON are dropped★: the return shape was never contracted (it varies per command), so the `ok`
 * the shell advertises is also 「무엇이든」 (anything). If serialization blows up, it is worse for an operation that succeeded to be reported as a failure because of it.
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
 * Attach the subscription and report this window's list. The return value is a disposer (prevents duplicate listener accumulation on unmount/HMR).
 *
 * ★Attach the subscription **first**★ — an envelope can arrive from the moment the report fills the shell's registry, and if there is no listener then,
 * that envelope does **not even arrive** at this window and hangs until the shell's deadline (the same ordering clause in `theme/uiSettings.ts`).
 * ★No retry is attached★ — if the subscription fails this window cannot receive envelopes, but the shell fixes the destination to one window,
 * so that failure does not block other windows. And since the report is skipped, the shell does not pick this window as the destination either.
 */
export function installViewCommandBridge(): () => void {
  let disposed = false
  let unlisten: UnlistenFn | undefined

  void (async () => {
    try {
      // ★★Do not drop `target` — then the same command runs as many times as there are windows★★
      //   The default target of `listen()` is `Any`, and Tauri wakes **every** listener registered as `Any`
      //   **regardless of the filter** (`match_any_or_filter` — `*target == Any || filter(target)`). That is, even if the shell picks one window and
      //   sends with `emit_to(label, …)`, with the default registration main · tree · popout all receive it and each runs the same envelope
      //   and each answers (one reply per `request_id` — a TRD §4-⑤ violation). If registered with its own label,
      //   among `AnyLabel` listeners only the one with the same label is hit.
      //   ★The label is given by Tauri★ — do not infer it from the hash or settings (if that value is wrong, you receive another window's envelope).
      const off = await listen<CommandRequest>(EVT_COMMAND_REQUEST, e => {
        // Discard whatever arrives after cleanup — if a dead instance (the first StrictMode run · the pre-HMR edition) answers, the same
        //   envelope gets two answers (the shell leaves the second as a warning).
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
        `[viewCommandBridge] ${EVT_COMMAND_REQUEST} 구독 실패 — 이 창은 버스 명령을 못 받는다:`,
        e,
      )
      return
    }

    try {
      await invoke(CMD_REPORT_VIEW_COMMANDS, { commands: offeredCommands() })
    } catch (e) {
      console.warn(`[viewCommandBridge] ${CMD_REPORT_VIEW_COMMANDS} 실패:`, e)
    }
  })()

  return () => {
    disposed = true
    unlisten?.()
    unlisten = undefined
  }
}
