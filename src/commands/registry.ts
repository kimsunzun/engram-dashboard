// ADR-0055 ★Not a state authority★: holds only discovery/routing/meta. A handler routes to an existing entry point (store action / existing invoke) — 0 new singletons · 0 new arg parsing, ADR-0035 layout authority kept.
// Consumers (human clicks · global keydown · window.__engramCmd) all go through the single run(id, args) (§5).
// ★Stay DOM-free★: a pure Map, unit-tested headless (vitest); DOM/Tauri dependence lives in adapters (*Commands.ts) and the keybinding listener.

/** Each handler destructures only the keys it needs (no variadic arguments, ADR-0055). */
export type CommandArgs = Record<string, unknown>

/** The shape of one argument field — unfolds directly into a JSON Schema fragment (shell `view_commands.rs`). */
export interface CommandArgSchema {
  type?: 'string' | 'number' | 'integer' | 'boolean' | 'object' | 'array'
  enum?: string[]
  description?: string
}

/**
 * A description that makes this command callable from outside — ★if present it goes on the bus, if absent it stays only inside this window★.
 * Arguments cannot be filled from the name alone (ADR-0156 — removes the round trip of asking the owner back). On the Rust side the declaration macro fills it; TypeScript has no macro, so **this is the only place written by hand**.
 * The shell unfolds it into one daemon catalog entry (`src-tauri/src/view_commands.rs`).
 */
export interface CommandHelp {
  /** What this command does — one line. */
  summary: string
  /**
   * Read or write — ★it cannot be omitted★. Rust decides this next to compilation with `#[effect(..)]`; TypeScript has no such place, and a shell default would advertise a false marker the day the first query command is attached — this value feeds the daemon's write-retention accounting,
   * so whoever attaches `help` must answer every time (the shell drops an entry lacking this field from registration).
   */
  effect: 'read' | 'write'
  /** Argument name → shape. Omit if there are no arguments. */
  args?: Record<string, CommandArgSchema>
  /** Argument names that must be present. */
  required?: string[]
}

export interface Command {
  /** Globally unique id (e.g. 'tab.next'). Consumers direct execution by this id alone. */
  id: string
  /** Human-readable name (menu display · `list()`), a function called each time it is shown so it follows the current UI language. Do not call `t()` at registration and close over the result (ADR-0373). */
  title: () => string
  /** Category for grouping (palette sections, etc.). */
  category?: string
  /** Default key combination hint (e.g. 'Ctrl+Shift+T'). Referenced by the keybinding consumer. */
  keybinding?: string
  /** A description that makes it callable from outside (LLM · CLI). ★Omitted = runs only inside this window★ — the shell puts on the daemon registry only commands that have it (`offeredCommands` in `commands/viewCommandBridge.ts`). */
  help?: CommandHelp
  /**
   * ★If present, this command runs **only on the human path** — the value is the reason (carried as is in the rejection wording)★.
   *
   * One registry entry **serves two consumers**: human clicks · keybindings (`dispatch.fireAndForget` → [`runAsHuman`]) and the LLM surface (`window.__engramCmd` · the bus bridge → [`run`]) — mostly an advantage (§5 「drive the same handle」),
   * but an **action that must be open to only one of the two** makes that dual role a hole. The closing axis is **the caller, not an argument value**, so `when` cannot tell them apart (it is a UI context gate read only by the keybinding consumer; neither [`run`] nor [`runAsHuman`] reads it).
   *
   * ★Check before writing — this field is an exception to §5, and an exception needs a reason★: 「a UI the LLM cannot get past is in the way」 is **not** a reason (a defect to fix). A legitimate reason is 「a **policy decision** that a human must be there, whose canonical source lives elsewhere」.
   *
   * ★Today **no command** uses this field★ — its only users, the two codex creation doors, lifted the gate (reason = the head comment of `agentCommands` · canonical source = `commands::LLM_BACKEND_POLICY` in `engram-dashboard-agent`).
   * ★Even so do not remove this field★: it is the means to close on the day a policy decision recurs. The gate's behavior is still measured by `registry.test.ts` with a synthetic command.
   *
   * ★On a different axis from `help`★ — no `help` = **not advertised** on the daemon registry, and that does not block execution (the bus bridge's `settle` calls `run` as long as there is a name). Removing the advertisement is not closing it.
   */
  humanOnly?: string
  /** Exposure/executability condition (a later when-context is outside the skeleton, ADR-0055). */
  when?: () => boolean
  /** Route to an existing store action/invoke. The return is passed through as is — some are Promises (e.g. layout createTab → Promise<id>), so cdp/call sites must be able to await. */
  run: (args?: CommandArgs) => unknown
}

const registry = new Map<string, Command>()

/** A duplicate id is warned about, then registered (dev safety — reveals adapter double loads and typos); the last registration wins (so HMR re-evaluation updates to the latest definition). */
export function register(cmd: Command): void {
  if (registry.has(cmd.id)) {
    console.warn(`[commands] duplicate id re-registered — overwrote the existing command: '${cmd.id}'`)
  }
  registry.set(cmd.id, cmd)
}

/** The common part of the two entry points — only resolves the id (the gate differs per caller). */
function resolve(id: string): Command {
  const cmd = registry.get(id)
  if (!cmd) {
    throw new Error(`[commands] unknown command id: '${id}'`)
  }
  return cmd
}

/**
 * ★The entry point of the LLM surface★ — `window.__engramCmd.run` · the bus bridge (`viewCommandBridge`) · cdp eval come here; human clicks · keybindings go to [`runAsHuman`] (via `dispatch.fireAndForget`).
 * An unknown id clearly throws (a silent no-op makes LLM/cdp debugging hard). The handler return is returned as is → call sites can await a Promise.
 * ★`humanOnly` blocks only here★ — see that field's doc. The rejection is not silent (it throws carrying the reason), so the LLM does not read it as 「pressed, but nothing happened」 and circle the same spot.
 */
export function run(id: string, args?: CommandArgs): unknown {
  const cmd = resolve(id)
  if (cmd.humanOnly) {
    throw new Error(
      `[commands] '${id}' is not run through this surface (LLM · bus) — ${cmd.humanOnly}`,
    )
  }
  return cmd.run(args)
}

/**
 * ★The entry point of the human path★ — only clicks · keybindings call it (`dispatch.fireAndForget` is the only caller, and that file is the copy template); not passing the `humanOnly` gate is why it exists.
 * ★Do not expose this to `__engramCmd` (`store/eventBus.ts`) or the bus bridge (`commands/viewCommandBridge.ts`)★ — exposing it gives the gate of [`run`] a bypass. ★No gate measures that prohibition (call-site discipline)★ — `commands/registry.test.ts` only shows the two entry points behave differently.
 */
export function runAsHuman(id: string, args?: CommandArgs): unknown {
  return resolve(id).run(args)
}

/**
 * ★Give `help` as a value copy — a shallow copy is not enough★: the snapshot contract of the two functions below broke at that field once `help` became an object (`list().find(...)!.help.summary = 'x'` directly changed a registered command, bypassing the Map entry).
 * Split down through the two nested levels (each field of `args`, the `enum` array) so no alias remains.
 */
function copyHelp(help: CommandHelp | undefined): CommandHelp | undefined {
  if (!help) return undefined
  const args = help.args
    ? Object.fromEntries(
        Object.entries(help.args).map(([field, schema]) => [
          field,
          { ...schema, ...(schema.enum ? { enum: [...schema.enum] } : {}) },
        ]),
      )
    : undefined
  return {
    ...help,
    ...(args ? { args } : {}),
    ...(help.required ? { required: [...help.required] } : {}),
  }
}

/**
 * undefined if absent. ★Differs from run()★: run executes (throws on an unknown id), get is a pure lookup (discovery/meta). Not a state authority.
 * ★Returns a copy★ (FIX-C): handing over the stored original would let a caller swap the registry Map entry with getCommand(id)!.run = evil and bypass the register()/duplicate-warning invariant. list() gives the same copy. `help` needs the nested split ([`copyHelp`]). No hot path → copy cost ignored.
 */
export function getCommand(id: string): Command | undefined {
  const cmd = registry.get(id)
  return cmd ? { ...cmd, help: copyHelp(cmd.help) } : undefined
}

/** A command as `list()` gives it — the title resolved to text. */
type CommandSummary = Pick<Command, 'id' | 'category' | 'keybinding' | 'help' | 'humanOnly'> & { title: string }

/**
 * A meta snapshot of registered commands (for discovery — palette · LLM introspection). Titles are resolved at the call, in the current UI language (ADR-0373).
 * ★`humanOnly` is carried too★ — the reader of this list is exactly who [`run`] will block; without it the LLM learns it is closed only after calling what it saw in the list (the asking-back round trip §5 meant to remove).
 */
export function list(): CommandSummary[] {
  return Array.from(registry.values()).map(
    ({ id, title, category, keybinding, help, humanOnly }) => ({
      id,
      title: title(),
      category,
      keybinding,
      help: copyHelp(help),
      humanOnly,
    }),
  )
}

/** Test only — registry reset (isolation between tests). Do not call from production code. */
export function __resetRegistryForTest(): void {
  registry.clear()
}
