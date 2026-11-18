// ADR-0055/0057: human clicks (TabBar·Ctrl+Tab) and the LLM (__engramCmd) pass through this same store action (§5 single control surface).
// ★Window resolution★: taken from the command args; when omitted, falls back to this webview window (readWindowLabelFromHash), so "focused window" consumers like Ctrl+Tab need no label.

import { invoke } from '@tauri-apps/api/core'

import type {
  LayoutNode,
  SlotContent,
  WindowCloseResult,
  WindowControlAction,
  WindowControlOutcome,
} from '../api/layoutTypes'
import { t } from '../i18n'
import { matchDeclaredSpelling } from './enumArg'
import { register } from './registry'
import { readWindowLabelFromHash, useViewStore } from '../store/viewStore'
import type { WindowTabsPayload } from '../store/viewStore'

// Empty, blank-only or non-string values do not fold into the fallback — an empty argument is usually an unfilled shell variable and folding would touch another window (same clause as `text` in the bus `layout/commands.rs`). Only undefined · null (as in optionalUuidArg) falls back to this webview window.
function resolveWindow(cmd: string, args?: Record<string, unknown>): string {
  const w = args?.window
  if (w === undefined || w === null) return readWindowLabelFromHash()
  if (typeof w !== 'string' || w.trim().length === 0) {
    throw new Error(
      `[${cmd}] window must be a non-empty string — an empty argument is usually an unfilled shell variable (got: ${describeArg(w)})`,
    )
  }
  return w
}

// JSON.stringify throws on BigInt and cyclic objects — then that TypeError would go out instead of the error prefixed with `[<cmd>]`.
function describeArg(v: unknown): string {
  try {
    return JSON.stringify(v) ?? String(v)
  } catch {
    return typeof v === 'bigint' ? `${String(v)}n` : typeof v
  }
}

// Turns a shell refusal (invoke refuses with a string) into an Error carrying the command name, so a bus answer's error text shows which command it belongs to.
async function tagShellRejection<T>(cmd: string, call: Promise<T>): Promise<T> {
  try {
    return await call
  } catch (e) {
    throw new Error(`[${cmd}] ${e instanceof Error ? e.message : String(e)}`)
  }
}

/**
 * Tab list of the `window` argument from the shell, which resolves it with the bus's resolver (takes window_id and a label in the same run, refuses numbers — TRD S26-window-chrome §4-5); the answer's `label` is the resolved window. ★Do not look it up in the web store (`windows`)★ — its key is the label only, so a window_id silently finds nothing. A shell refusal is thrown as an Error carrying the command name.
 */
function listTabsOf(cmd: string, window: string): Promise<WindowTabsPayload> {
  return tagShellRejection(cmd, invoke<WindowTabsPayload>('list_tabs', { window }))
}

register({
  id: 'tab.create',
  title: () => t('tab.create'),
  category: 'tab',
  // name is trimmed like tab.rename, and blank-only throws before invoke — when omitted, an automatic name.
  run: args => {
    let name: string | undefined
    if (typeof args?.name === 'string') {
      name = args.name.trim()
      if (name.length === 0) throw new Error('[tab.create] name cannot be blank')
    }
    return useViewStore.getState().createTab(resolveWindow('tab.create', args), name)
  },
})

register({
  id: 'tab.switch',
  title: () => t('tab.switch'),
  category: 'tab',
  run: args => {
    const view = args?.view
    if (typeof view !== 'string') throw new Error('[tab.switch] view (tab id) required')
    return useViewStore.getState().switchTab(resolveWindow('tab.switch', args), view)
  },
})

register({
  id: 'tab.close',
  title: () => t('tab.closeCmd'),
  category: 'tab',
  // view omitted = the active tab of the given window (window) — not this webview's active one (S4-F1).
  run: async args => {
    const window = resolveWindow('tab.close', args)
    const view = args?.view
    if (typeof view === 'string') return useViewStore.getState().closeTab(window, view)
    const tabs = await listTabsOf('tab.close', window)
    return useViewStore.getState().closeTab(tabs.label, tabs.active)
  },
})

register({
  id: 'tab.next',
  title: () => t('tab.next'),
  category: 'tab',
  keybinding: 'Ctrl+Tab',
  // ★State exactly what changes when omitted★: a bus caller has no "own window"; the target is the destination window the shell picks (usually main), and resolveWindow's fallback reads that window's URL hash — so omitting means effectively main. This text is the LLM's only basis for passing window, so do not say "this window".
  help: {
    summary: "Moves that window's active tab to the next tab (does nothing when there is only one tab).",
    effect: 'write',
    args: {
      window: {
        type: 'string',
        description:
          'Target window — a window_id from window.list (the same window across restarts) or a label within the same run. Numbers are not accepted (they get reused — refused if given). When omitted it is effectively main — a bus call has no "calling window", so the hash of the destination window the shell picked is read, and that value falls to main.',
      },
    },
  },
  // ★Ctrl+Tab (D-8)★: the same command path as human keys and clicks (§5). Active tab and tab order are based on the shell answer (list_tabs).
  run: async args => {
    const tabs = await listTabsOf('tab.next', resolveWindow('tab.next', args))
    if (tabs.tabs.length <= 1) return
    const idx = tabs.tabs.findIndex(t => t.id === tabs.active)
    const next = tabs.tabs[idx < 0 ? 0 : (idx + 1) % tabs.tabs.length]
    return useViewStore.getState().switchTab(tabs.label, next.id)
  },
})

register({
  id: 'tab.rename',
  title: () => t('tab.rename'),
  category: 'tab',
  // ★rename (ADR-0057)★: one surface with human double-click inline edit (§5). An empty/blank name throws before invoke (same normalization as the TabBar boundary); view comes from the backend view_owner, so no window is needed.
  run: args => {
    const view = args?.view
    if (typeof view !== 'string' || view.length === 0) throw new Error('[tab.rename] view (tab id) required')
    const name = args?.name
    if (typeof name !== 'string') throw new Error('[tab.rename] name (string) required')
    const trimmed = name.trim()
    if (trimmed.length === 0) throw new Error('[tab.rename] name cannot be blank')
    return useViewStore.getState().renameTab(view, trimmed)
  },
})

register({
  id: 'window.create',
  title: () => t('window.create'),
  category: 'window',
  // New empty window (1 empty tab) (D-6) → {window, window_id, number}, same set as the shell bus `window.create` answer (TRD S26-window-chrome §5-3).
  run: () => useViewStore.getState().createWindow(),
})

// ── Window state commands (TRD S26-window-chrome §6-4) ──
// Same ids as the shell bus table — ★no `help`★ (the same name on the daemon roster again bounces the whole registration packet — `offeredCommands` in `viewCommandBridge.ts`). All go to the single invoke `window_control`; the answer is trimmed to the bus answer's shape so both surfaces return the same fields.

// The `outcome` spelling of the bus `window.close` — the invoke answer's `close` is snake_case.
const CLOSE_OUTCOME: Record<WindowCloseResult, 'Closed' | 'HiddenAll' | 'Kept'> = {
  closed: 'Closed',
  hidden_all: 'HiddenAll',
  kept: 'Kept',
}

function registerWindowControl(
  id: string,
  title: () => string,
  action: WindowControlAction,
  answer: (outcome: WindowControlOutcome) => Record<string, unknown>,
): void {
  // The all-windows verbs (`hide_all` · `show_all`) ignore the `window` argument like the bus — the field invoke requires gets this window's label (the shell does not pick by it). A given `window` is still shape-checked with resolveWindow (an empty argument is usually an unfilled shell variable).
  const allWindows = action === 'hide_all' || action === 'show_all'
  register({
    id,
    title,
    category: 'window',
    run: args => {
      const target = resolveWindow(id, args)
      return tagShellRejection(
        id,
        useViewStore.getState().controlWindow(allWindows ? readWindowLabelFromHash() : target, action),
      ).then(answer)
    },
  })
}

registerWindowControl('window.minimize', () => t('window.minimize'), 'minimize', () => ({}))
registerWindowControl('window.maximize', () => t('window.maximize'), 'maximize', o => ({ deferred: o.deferred }))
registerWindowControl('window.unmaximize', () => t('window.unmaximize'), 'unmaximize', o => ({ deferred: o.deferred }))
registerWindowControl('window.raise', () => t('window.raise'), 'raise', o => ({
  shown_all: o.shown_all,
  focused: o.focused,
  pending: o.pending,
}))
registerWindowControl('window.hideAll', () => t('window.hideAll'), 'hide_all', () => ({}))
registerWindowControl('window.showAll', () => t('window.showAll'), 'show_all', o => ({ focused: o.focused }))

register({
  id: 'window.close',
  title: () => t('window.close'),
  category: 'window',
  // Popout = close · main = hide all windows (shell decides — TRD §5-4). `if_empty` matches the bus (§5-5); with it, omitting `window` falls back to this window's label, which the shell (finding only by window_id) cannot match — it closes nothing (`Kept`), so it throws.
  run: args => {
    const raw = args?.if_empty
    if (raw !== undefined && raw !== null && typeof raw !== 'boolean') {
      throw new Error(`[window.close] if_empty must be a boolean (got: ${describeArg(raw)})`)
    }
    const ifEmpty = raw === true
    if (ifEmpty && (args?.window === undefined || args.window === null)) {
      throw new Error('[window.close] if_empty requires window (the window_id of that window)')
    }
    return tagShellRejection(
      'window.close',
      useViewStore.getState().controlWindow(resolveWindow('window.close', args), 'close', ifEmpty),
    ).then(o => ({ outcome: o.close === null ? null : CLOSE_OUTCOME[o.close] }))
  },
})

/** tab/slot are backend ViewId/slot ids (Uuid), so the format must match. */
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/**
 * ★spawn_into argument normalization (FIX 5)★: tab/slot = a UUID string or unspecified (null). A wrong type (a number etc.) used to degrade silently to null ("spawned but placed wrong"), so present-but-invalid throws **before** invoke to block the side-effecting spawn.
 */
function optionalUuidArg(v: unknown, name: string): string | null {
  if (v === undefined || v === null) return null
  if (typeof v !== 'string' || !UUID_RE.test(v)) {
    throw new Error(`[agent.spawnInto] ${name} must be a UUID string (got: ${JSON.stringify(v)})`)
  }
  return v
}

// SlotContent union tag whitelist (set_slot_content validation — ADR-0060/0063). ★Order = declared spelling★: whatever letter case the caller uses, the tag sent to the backend folds to an element of this array (CLI and this surface must agree).
const SLOT_CONTENT_TYPES = ['empty', 'agent', 'agent_list', 'preset_palette', 'usage'] as const

const USAGE_SHOW_FIELDS = ['show_claude', 'show_codex'] as const

function findSlotContent(node: LayoutNode, slotId: string): SlotContent | null {
  if (node.type === 'slot') return node.id === slotId ? node.content : null
  return findSlotContent(node.a, slotId) ?? findSlotContent(node.b, slotId)
}

/**
 * A missing show field of a usage slot = the slot's current value if it is already usage (TRD §3 #20). The shell bus `layout.setSlotContent` merges inside the lock; this path does the same so both LLM paths agree. Not usage yet or view unknown → passed missing (shell serde default reads a missing field as true, `SlotContent::Usage`).
 * ★Based on the layout the webview received★ — it can drift from the shell's authoritative value (consecutive calls before the broadcast). The atomic merge is the shell's `set_usage_slot` (bus and `usageSlot.toggle*`). ★So an LLM sending usage content through this path (`__engramCmd`·cdp) gets both show_claude and show_codex★ — to keep a missing field at its current value use the bus `layout.setSlotContent`. This note is not in `prompts/engram-help.md` because that reader is a CLI agent that cannot call window JS.
 */
function mergeUsageShows(viewId: string, slotId: string, content: Record<string, unknown>): Record<string, unknown> {
  const layout = useViewStore.getState().layouts[viewId]?.layout
  const current = layout ? findSlotContent(layout, slotId) : null
  if (current?.type !== 'usage') return content
  const merged = { ...content }
  for (const field of USAGE_SHOW_FIELDS) if (merged[field] === undefined) merged[field] = current[field]
  return merged
}

/**
 * ★SlotContent variant shape validation (FIX LOW)★: checking only the tag lets a malformed value like `{type:'agent'}` (no agent_id) pass and fail late in Rust deserialization. Required fields per variant are validated at the registry boundary so a bad value throws **before** invoke (like optionalUuidArg). The tag is already validated by SLOT_CONTENT_TYPES.
 */
function validateSlotContent(content: { type: string } & Record<string, unknown>): void {
  switch (content.type) {
    case 'agent':
      if (typeof content.agent_id !== 'string' || content.agent_id.length === 0) {
        throw new Error(
          `[layout.setSlotContent] agent variant requires a string agent_id (got: ${JSON.stringify(content)})`,
        )
      }
      break
    case 'empty':
    case 'agent_list':
    case 'preset_palette':
      // Unit variant with no extra fields — passes when the tag matches (extra fields are ignored by the backend).
      break
    case 'usage':
      // Missing fields are not filled here — the caller fills them with that slot's current value (`mergeUsageShows`).
      for (const field of USAGE_SHOW_FIELDS) {
        if (content[field] !== undefined && typeof content[field] !== 'boolean') {
          throw new Error(
            `[layout.setSlotContent] ${field} of the usage variant must be a boolean (got: ${JSON.stringify(content)})`,
          )
        }
      }
      break
    // default is unreachable (already filtered by SLOT_CONTENT_TYPES at the caller) — defence only.
  }
}

register({
  id: 'layout.setSlotContent',
  title: () => t('slot.setContent'),
  category: 'layout',
  // ★set_slot_content (ADR-0063)★: human right-click = one surface with the LLM (§5).
  run: args => {
    const viewId = args?.viewId
    const slotId = args?.slotId
    const raw = args?.content
    if (typeof viewId !== 'string') throw new Error('[layout.setSlotContent] viewId required')
    if (typeof slotId !== 'string') throw new Error('[layout.setSlotContent] slotId required')
    const type = matchDeclaredSpelling((raw as { type?: unknown } | undefined)?.type, SLOT_CONTENT_TYPES)
    if (!type) {
      throw new Error(
        `[layout.setSlotContent] content.type must be a SlotContent variant (got: ${JSON.stringify(raw)})`,
      )
    }
    // The folded tag is put back — extra fields (agent_id etc.) pass through as the caller gave them, and only the tag is swapped.
    const content = { ...(raw as Record<string, unknown>), type }
    validateSlotContent(content)
    const sent = type === 'usage' ? mergeUsageShows(viewId, slotId, content) : content
    return useViewStore.getState().setSlotContent(viewId, slotId, sent as SlotContent)
  },
})

register({
  id: 'agent.spawnInto',
  title: () => t('agent.spawnInto'),
  category: 'agent',
  // ★spawn_into (D-7, TRD §6 G9)★: invokes the backend composite command (spawn + tab creation if needed + slot assignment; backend authority without store state — ADR-0057). Returns the new AgentId. Slot policy and failure visibility are the backend's.
  // ★FIX 5★: tab/slot get UUID validation before invoke (bad value → throw before the spawn).
  run: args => {
    const cwd = args?.cwd
    if (typeof cwd !== 'string' || cwd.length === 0) throw new Error('[agent.spawnInto] cwd required')
    const tab = optionalUuidArg(args?.tab, 'tab')
    const slot = optionalUuidArg(args?.slot, 'slot')
    const backend = typeof args?.backend === 'string' ? args.backend : null
    return invoke<string>('spawn_into', {
      window: resolveWindow('agent.spawnInto', args),
      tab,
      slot,
      backend,
      cwd,
    })
  },
})
