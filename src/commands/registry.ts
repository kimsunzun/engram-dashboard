// ADR-0055 ★Not a state authority★: it holds only discovery/routing/meta. For execution, the handler routes to an existing entry point (store action /
//   existing invoke) (0 new singletons · 0 new arg parsing, ADR-0035 layout authority kept). Consumers (human clicks ·
//   global keydown · window.__engramCmd) are all unified on the single run(id, args) (§5).
//
// ★Stay DOM-free★: it is a pure Map, so it is unit-tested headless (vitest). DOM/Tauri dependence is pushed out to
//   adapters (*Commands.ts) and the keybinding listener.

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
 *
 * Attached because arguments cannot be filled from the name alone (ADR-0156 — the value that removed the round trip of asking the owner back). On the Rust side the
 * declaration macro fills it automatically, but TypeScript has no macro, so **this is the only place written by hand**.
 * The shell unfolds this value into one daemon catalog entry and registers it (`src-tauri/src/view_commands.rs`).
 */
export interface CommandHelp {
  /** What this command does — one line. */
  summary: string
  /**
   * Read or write — ★it cannot be omitted★.
   *
   * A Rust declaration decides this next to compilation with `#[effect(..)]`, but TypeScript has no such place. If the shell picked a default,
   * the day the first query command is attached the registry would advertise a false marker, and that value feeds directly into the daemon's write-retention
   * accounting — so whoever attaches `help` must answer every time (the shell drops an entry lacking this field from registration).
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
  /** Human-readable name (for palette · menu display, a follow-up consumer). */
  title: string
  /** Category for grouping (palette sections, etc.). */
  category?: string
  /** Default key combination hint (e.g. 'Ctrl+Shift+T'). Referenced by the keybinding consumer. */
  keybinding?: string
  /**
   * A description that makes it callable from outside (LLM · CLI). ★Omitted = a command that runs only inside this window★ — what the shell puts
   * on the daemon registry is only those that have this field (`offeredCommands` in `commands/viewCommandBridge.ts`).
   */
  help?: CommandHelp
  /**
   * ★If present, this command runs **only on the human path** — the value is the reason (carried as is in the rejection wording)★.
   *
   * One entry of this registry **serves two consumers**: human clicks · keybindings (`dispatch.fireAndForget` →
   * [`runAsHuman`]) and the LLM surface (`window.__engramCmd` · the bus bridge → [`run`]). Mostly that is an advantage
   * (§5 「같은 핸들을 흔든다」 — drive the same handle), but once an **action that must be open to only one of the two** appears, that dual role becomes a hole.
   * The axis that closes it is then **the caller, not an argument value**, so `when` cannot tell them apart — `when` is a UI context
   * gate that does not know the caller and is read only by the keybinding consumer (neither entry point, [`run`] nor [`runAsHuman`], reads it).
   *
   * ★Check before writing — this field is an exception to §5, and an exception needs a reason★: 「LLM 이 못 지나는 UI 가
   * 뒤에 있다」 (a UI the LLM cannot get past is in the way) is **not** a reason (that is a defect to fix, not grounds to close). A legitimate reason is 「사람이 그
   * 자리에 있어야 한다는 **정책 결정**이 있고, 그 결정의 정본이 딴 데 있다」 (there is a **policy decision** that a human must be there, and its canonical source lives elsewhere).
   *
   * ★Today **no command** uses this field (2026-09-22)★ — the two codex creation doors, its only users, lifted
   * the gate (reason = the head comment of `agentCommands` · canonical source = `commands::LLM_BACKEND_POLICY` in
   * `engram-dashboard-agent`). ★Even so we do not remove this field★: we need a means to close on the day the condition defined by the paragraph above
   * holds again, and that condition is not 「LLM 이 못 지나는 UI」 (a UI the LLM cannot get past) but a **policy decision**.
   * The gate's own behavior is still measured by `registry.test.ts` with a synthetic command — it does not die even with 0 users.
   *
   * ★This field is on a different axis from `help`★ — no `help` = **not advertised** on the daemon registry, and that does not block
   * execution (the bus bridge's `settle` calls `run` as long as there is a name). Do not read removing the advertisement as having closed it.
   */
  humanOnly?: string
  /** Exposure/executability condition (a later when-context is outside the skeleton, ADR-0055). */
  when?: () => boolean
  /**
   * Route to an existing store action/invoke. The return is passed through as is —
   * some are Promises (e.g. layout createTab → Promise<id>), so cdp/call sites must be able to await.
   */
  run: (args?: CommandArgs) => unknown
}

const registry = new Map<string, Command>()

/**
 * A duplicate id is not silently overwritten but warned about and then registered (dev safety — reveals adapter double loads and
 * typos). The last registration wins (so HMR re-evaluation updates to the latest definition).
 */
export function register(cmd: Command): void {
  if (registry.has(cmd.id)) {
    console.warn(`[commands] 중복 id 재등록 — 기존 command 를 덮어씀: '${cmd.id}'`)
  }
  registry.set(cmd.id, cmd)
}

/** The common part of the two entry points — only resolves the id (the gate differs per caller). */
function resolve(id: string): Command {
  const cmd = registry.get(id)
  if (!cmd) {
    throw new Error(`[commands] 알 수 없는 command id: '${id}'`)
  }
  return cmd
}

/**
 * ★The entry point of the LLM surface★ — `window.__engramCmd.run` · the bus bridge (`viewCommandBridge`) · cdp eval come
 * here. Human clicks · keybindings go to [`runAsHuman`] (via `dispatch.fireAndForget`).
 *
 * An unknown id clearly throws (a silent no-op makes LLM/cdp debugging hard).
 * The handler return is returned as is → call sites can await a Promise.
 *
 * ★`humanOnly` blocks only here★ — the doc of that field bears when to use it. The rejection is not silent (it throws
 * carrying the reason): this stops the LLM from reading it as 「눌렸는데 아무 일도 안 일어났다」 (pressed, but nothing happened) and circling the same spot.
 */
export function run(id: string, args?: CommandArgs): unknown {
  const cmd = resolve(id)
  if (cmd.humanOnly) {
    throw new Error(
      `[commands] '${id}' 는 이 표면(LLM·버스)으로는 실행하지 않는다 — ${cmd.humanOnly}`,
    )
  }
  return cmd.run(args)
}

/**
 * ★The entry point of the human path★ — only clicks · keybindings call it (`dispatch.fireAndForget` is the only caller, and that
 * file is the copy template). Not passing the `humanOnly` gate is the reason this function exists.
 *
 * ★Do not expose this to `__engramCmd` (`store/eventBus.ts`) or the bus bridge (`commands/viewCommandBridge.ts`)★ —
 * the moment it is exposed, the gate of [`run`] gains a bypass and guards nothing. ★There is no gate that measures that prohibition
 * (it is call-site discipline)★ — what `commands/registry.test.ts` measures goes only as far as showing that the two entry points actually
 * behave differently.
 */
export function runAsHuman(id: string, args?: CommandArgs): unknown {
  return resolve(id).run(args)
}

/**
 * ★Give `help` as a value copy — a shallow copy is not enough★.
 *
 * The contract of returning a snapshot (the two functions below) broke only at that field the moment `help` became an object:
 * `list().find(...)!.help.summary = 'x'` directly changes a registered command (bypassing the registry Map entry).
 * We must split down through the two nested levels (each field of `args`, the `enum` array) for no alias to remain.
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
 * undefined if absent.
 * ★Differs from run()★: run executes (throws on an unknown id), get is a pure lookup (discovery/meta). Not a state authority.
 * ★Returns a copy★ (FIX-C): handing over the stored original lets a caller swap the registry Map entry with getCommand(id)!.run = evil
 *   and bypass the register()/duplicate-warning invariant. list() gives the same copy, so it is consistent. Only `help` is not enough
 *   with a shallow copy, so it is split out separately ([`copyHelp`]). No hot path → copy cost ignored.
 */
export function getCommand(id: string): Command | undefined {
  const cmd = registry.get(id)
  return cmd ? { ...cmd, help: copyHelp(cmd.help) } : undefined
}

/**
 * A meta snapshot of registered commands (for discovery — palette · LLM introspection).
 *
 * ★`humanOnly` is carried too★ — whoever reads this list is exactly who [`run`] will block, so if not carried the LLM learns it is closed only
 * after calling what it saw in the list (that round trip is the very asking-back §5 meant to remove).
 */
export function list(): Array<
  Pick<Command, 'id' | 'title' | 'category' | 'keybinding' | 'help' | 'humanOnly'>
> {
  return Array.from(registry.values()).map(
    ({ id, title, category, keybinding, help, humanOnly }) => ({
      id,
      title,
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
