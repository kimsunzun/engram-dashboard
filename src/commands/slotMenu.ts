// ADR-0064: slot right-click menu items are assembled only as command-id references (no direct store calls from the menu). Common ops contribute with target='*', content-specific items with target=<SlotContentType>, via registerSlotMenu.
//
// ★DOM-free★: pure Map/functions so it is unit-tested headless; DOM/Tauri dependence lives in the command handlers.

import type { SlotContent } from '../api/layoutTypes'
import { getCommand } from './registry'

export type SlotMenuTarget = SlotContent['type'] | '*'

/**
 * Execution context of the slot that opened the menu; `agentId` only for an assigned slot.
 * ★Only source of menu state (☑ · enabled)★ — the assembly site (`LayoutLeaf`) subscribes to the store and carries state here, so an open menu redraws.
 */
export interface SlotMenuCtx {
  viewId: string | null
  slotId: string
  agentId?: string | null
  content?: SlotContent
}

/**
 * ADR-0064 fixed schema + ADR-0065 additive extension. One of two shapes:
 *   - Execution item: `commandId`, no `children`.
 *   - Container (1-level submenu): `children` + own `id` and `title`, no `commandId`.
 * Neither or both is a dev error (fail-loud, ADR-0065 §Impact/invariants).
 */
export interface SlotMenuItem {
  /** Registry command id of an execution item (e.g. 'slot.close'); omitted for a container. Unregistered → buildSlotMenu skips + logs. */
  commandId?: string
  /** Container identity: registration key, resolved id (`container:<id>`) and last sort tie-break. ADR-0373: an id, not the title, so it is language-independent. */
  id?: string
  /** Container display name, called at build time (ADR-0373). Omitted for an execution item. */
  title?: () => string
  /** Sort axis 1 — 'content' before 'slot-ops' (GROUP_ORDER). */
  group: string
  order: number
  /** ADR-0065 exclusion — hides the item in slots of the listed content types (subtraction only, not an allowlist). E.g. `['empty']` keeps the '*' registration but drops it from empty slots. */
  hideOn?: SlotContent['type'][]
  /** ADR-0065 1-level submenu children (flat execution items only). Present → container. */
  children?: SlotMenuItem[]
  /**
   * Toggle ☑ (renderer = `menuitemcheckbox`); not for containers. ★Reads only ctx, never the store★ (ADR-0064) — the title is fixed at build time, so ctx is the only state carrier.
   */
  checked?: (ctx: SlotMenuCtx) => boolean
  /** False → drawn disabled. Same rule as `checked` (★reads only ctx★). Not the command's `when`, which the menu does not read. Not for containers. */
  enabled?: (ctx: SlotMenuCtx) => boolean
}

/** buildSlotMenu output: items the renderer uses as is. A container (children present) is not run; its children unfold as a hover flyout. */
export interface ResolvedSlotMenuItem {
  id: string
  title: string
  run: (args?: Record<string, unknown>) => unknown
  group: string
  /** First of its group with another group before it → renderer draws a divider. */
  separatorBefore: boolean
  /** ADR-0065 resolved children of a container; undefined for an execution item. */
  children?: ResolvedSlotMenuItem[]
  checked?: (ctx: SlotMenuCtx) => boolean
  enabled?: (ctx: SlotMenuCtx) => boolean
}

// ★Group render order★ (ADR-0064 §5): 'content' above 'slot-ops'. Unlisted groups sort last by name — add a new group here to fix its order.
const GROUP_ORDER: Record<string, number> = {
  content: 0,
  'slot-ops': 1,
}

function groupRank(group: string): number {
  return GROUP_ORDER[group] ?? Number.POSITIVE_INFINITY
}

const contributions = new Map<SlotMenuTarget, SlotMenuItem[]>()

function itemKey(item: SlotMenuItem): string {
  return item.commandId ?? `container:${item.id ?? ''}`
}

/**
 * Register a slot menu contribution. Re-contributing the same (target, key) warns and replaces (last wins; reveals HMR re-evaluation, duplicate loads, typos).
 * key = commandId, or 'container:'+id for a container.
 */
export function registerSlotMenu(target: SlotMenuTarget, items: SlotMenuItem[]): void {
  const list = contributions.get(target) ?? []
  for (const item of items) {
    const key = itemKey(item)
    const idx = list.findIndex(existing => itemKey(existing) === key)
    if (idx >= 0) {
      console.warn(`[slotMenu] duplicate contribution re-registered — replaced: target='${target}' key='${key}'`)
      list[idx] = item
    } else {
      list.push(item)
    }
  }
  contributions.set(target, list)
}

function compareItems(a: SlotMenuItem, b: SlotMenuItem): number {
  const gr = groupRank(a.group) - groupRank(b.group)
  if (gr !== 0) return gr
  if (a.group !== b.group) return a.group < b.group ? -1 : 1
  if (a.order !== b.order) return a.order - b.order
  const ka = itemKey(a)
  const kb = itemKey(b)
  return ka < kb ? -1 : ka > kb ? 1 : 0
}

/**
 * Null (skip) when unresolvable. ★fail-loud but crash-free (FIX-1)★: runs at right-click render time, so a throw would blank the app — console.error and skip the item.
 */
function resolveRunnable(commandId: string, target: SlotContent['type']): ResolvedSlotMenuItem | null {
  const cmd = getCommand(commandId)
  if (!cmd) {
    console.error(`[slotMenu] unregistered commandId "${commandId}" (target=${target}) — skipped`)
    return null
  }
  return { id: cmd.id, title: cmd.title(), run: cmd.run, group: '', separatorBefore: false }
}

function withCtxPredicates(entry: ResolvedSlotMenuItem, item: SlotMenuItem): ResolvedSlotMenuItem {
  return {
    ...entry,
    ...(item.checked ? { checked: item.checked } : {}),
    ...(item.enabled ? { enabled: item.enabled } : {}),
  }
}

/**
 * ADR-0065 §Impact/invariants shape check: execution item (commandId, no children) XOR container (non-empty children + id + title, no commandId). Violation → console.error, return false (skip).
 * ★1 level only★: nested=true (already a container child) with children is forbidden.
 */
function validateItemShape(item: SlotMenuItem, target: SlotMenuTarget, nested: boolean): boolean {
  const hasCmd = typeof item.commandId === 'string' && item.commandId.length > 0
  const hasChildren = Array.isArray(item.children) && item.children.length > 0
  const label = item.commandId ?? item.id ?? '(untitled)'
  if (hasCmd && hasChildren) {
    console.error(`[slotMenu] item "${label}" (target=${target}) — cannot have both commandId and children (execution item XOR container) — skipped`)
    return false
  }
  if (!hasCmd && !hasChildren) {
    console.error(`[slotMenu] item "${label}" (target=${target}) — has neither commandId nor children (execution item XOR container required) — skipped`)
    return false
  }
  if (hasChildren) {
    if (nested) {
      console.error(`[slotMenu] item "${label}" (target=${target}) — nesting of 2+ levels not allowed (children inside children) — skipped`)
      return false
    }
    if (typeof item.id !== 'string' || item.id.length === 0) {
      console.error(`[slotMenu] container "${label}" (target=${target}) — a container needs an id — skipped`)
      return false
    }
    if (typeof item.title !== 'function') {
      console.error(`[slotMenu] container "${label}" (target=${target}) — a container needs a title function — skipped`)
      return false
    }
    for (const field of ['checked', 'enabled'] as const) {
      if (item[field]) {
        console.error(`[slotMenu] container "${label}" (target=${target}) — a container cannot have ${field} — skipped`)
        return false
      }
    }
  }
  return true
}

/**
 * Contributions for contentType ∪ '*', sorted, resolved against the registry.
 * ★Order is independent of registration order★ (ADR-0064): (GROUP_ORDER rank, order), ties by key — no wobble from import/HMR order.
 * ★hideOn (ADR-0065)★: removed first (subtraction; '*' universality kept). Applies to children too (FIX-2).
 * ★1-level submenu (ADR-0065)★: children keep declaration order; a child with children is 2-level nesting → dev error + skip.
 * ★Empty container omitted (FIX-1)★: if all children are skipped it would be a dead leaf, so console.error + skip the whole container.
 * ★dedupe (FIX-2)★: same key from '*' and contentType → keep the first in final order (defensive; content sorts before slot-ops).
 * ★fail-loud but crash-free (FIX-1)★: unregistered commandId or shape violation → console.error + skip, never throw.
 * separatorBefore: true on the first item of each new group (not the very first).
 */
export function buildSlotMenu(contentType: SlotContent['type']): ResolvedSlotMenuItem[] {
  const merged: SlotMenuItem[] = [...(contributions.get('*') ?? []), ...(contributions.get(contentType) ?? [])]

  const visible = merged.filter(item => !(item.hideOn?.includes(contentType) ?? false))

  const sorted = visible.slice().sort(compareItems)

  const resolved: ResolvedSlotMenuItem[] = []
  const seen = new Set<string>()
  let prevGroup: string | null = null
  for (const item of sorted) {
    // ★A skip does not update seen · prevGroup★ — so the divider calculation follows the actual render order.
    const key = itemKey(item)
    if (seen.has(key)) continue
    if (!validateItemShape(item, contentType, false)) continue

    let entry: ResolvedSlotMenuItem
    if (item.children) {
      const children: ResolvedSlotMenuItem[] = []
      for (const child of item.children) {
        if (child.hideOn?.includes(contentType) ?? false) continue
        if (!validateItemShape(child, contentType, true)) continue
        // A container's children are always execution items (validateItemShape nested=true prevents them from holding children again).
        const rc = resolveRunnable(child.commandId as string, contentType)
        if (rc) children.push(withCtxPredicates(rc, child))
      }
      if (children.length === 0) {
        console.error(
          `[slotMenu] container "${item.id}" (target=${contentType}) — all children were skipped, leaving an empty container — omitted`,
        )
        continue
      }
      entry = {
        // ADR-0373: id comes from the container's own id so the React key · data-attr survive a language switch. run is a no-op.
        id: `container:${item.id}`,
        title: (item.title as () => string)(),
        run: () => undefined,
        group: item.group,
        separatorBefore: prevGroup !== null && prevGroup !== item.group,
        children,
      }
    } else {
      const rr = resolveRunnable(item.commandId as string, contentType)
      if (!rr) continue
      entry = withCtxPredicates(
        {
          ...rr,
          group: item.group,
          separatorBefore: prevGroup !== null && prevGroup !== item.group,
        },
        item,
      )
    }
    seen.add(key)
    resolved.push(entry)
    prevGroup = item.group
  }
  return resolved
}

/**
 * Boot-time validation of every registered contribution (no right click needed): (1) shape, recursively into children (FIX-3); (2) commandId exists in the registry for execution items and container children (FIX-1).
 * console.error only, never throw. ★No sorting/resolve/hideOn★ — pure validation.
 */
export function validateSlotMenuContributions(): void {
  for (const [target, items] of contributions) {
    for (const item of items) {
      // FIX-3: on violation the commandId validation below is meaningless, so skip.
      if (!validateItemShape(item, target, false)) continue
      if (item.children) {
        for (const child of item.children) {
          if (!validateItemShape(child, target, true)) continue
          if (child.commandId && !getCommand(child.commandId)) {
            console.error(
              `[slotMenu] unregistered commandId "${child.commandId}" (target=${target}) — contribution will be skipped at render`,
            )
          }
        }
        continue
      }
      if (item.commandId && !getCommand(item.commandId)) {
        console.error(
          `[slotMenu] unregistered commandId "${item.commandId}" (target=${target}) — contribution will be skipped at render`,
        )
      }
    }
  }
}

/** Test only — resets contributions between tests; never call from production code. */
export function __resetSlotMenuForTest(): void {
  contributions.clear()
}
