// ADR-0064: assemble the slot right-click menu items only as "command id references" (no direct store calls from the menu —
//   ADR-0064 invariant). Common slot ops are contributed with target='*', and content-specific items with target=<SlotContentType>,
//   through the same API (registerSlotMenu) (in the vein of VS Code contributes.menus).
//
// ★Stay DOM-free★: like registry.ts, it is a pure Map/functions, so it is unit-tested headless (vitest). DOM/Tauri
//   dependence is pushed out to the command handlers (*Commands.ts / slotCommands.ts).

import type { SlotContent } from '../api/layoutTypes'
import { getCommand } from './registry'

/** '*' = all slots (common). */
export type SlotMenuTarget = SlotContent['type'] | '*'

/**
 * The execution context of the slot that opened the menu — the assembly site (`LayoutLeaf`) carries it. `agentId` only for an assigned slot.
 * ★The only source of menu state (☑ · enabled)★ — [`SlotMenuItem.checked`] · [`SlotMenuItem.enabled`] read only this. If store
 * state is needed, the assembly site subscribes and carries it here (since it subscribes, an open menu is redrawn to follow).
 */
export interface SlotMenuCtx {
  viewId: string | null
  slotId: string
  agentId?: string | null
  content?: SlotContent
}

/**
 * A contribution item (ADR-0064 fixed schema + ADR-0065 additive extension). It is one of two shapes:
 *   - **Execution item**: has `commandId` (+ no `children`). title/run are resolved from the registry at build time.
 *   - **Container** (1-level submenu): has `children` + its own `title` required (+ no `commandId`).
 * Neither or both is a dev error (fail-loud, ADR-0065 §영향/불변식). hideOn · children · title are all
 * optional, so an existing `{commandId, group, order}` contribution stays valid unchanged (backward compatible).
 */
export interface SlotMenuItem {
  /** The registry command id of an execution item (e.g. 'slot.close'). Omitted for a container. fail-loud: if unregistered, buildSlotMenu skips + logs an error. */
  commandId?: string
  /** Display name of a container (has children). Omitted for an execution item (the registry title is used). */
  title?: string
  /** Sort axis 1 (group) — 'content' (content-specific) first, 'slot-ops' (common) later. Order fixed by GROUP_ORDER. */
  group: string
  /** Sort axis 2 (order within the group). Ascending within the same group. */
  order: number
  /**
   * ADR-0065 exclusion condition — hides this item in slots of the listed content types (subtraction only, not an allowlist).
   * E.g. `['empty']` on slot.empty → the '*' universal registration is kept but removed only from empty slots (common ops single source kept).
   */
  hideOn?: SlotContent['type'][]
  /** ADR-0065 1-level submenu children. If present this item is a container (title required without commandId). Children are flat execution items only (no nesting). */
  children?: SlotMenuItem[]
  /**
   * If present, this execution item is a toggle and the value is the ☑ (renderer = `menuitemcheckbox`). Cannot be attached to a container.
   * ★Reads only ctx — do not read the store★ (ADR-0064 「메뉴에서 직접 store 호출 금지」 — no direct store calls from the menu): the title is fixed at build time, so
   * this is the only place to carry state, and the assembly site puts state in ctx for it.
   */
  checked?: (ctx: SlotMenuCtx) => boolean
  /**
   * If false the renderer draws it disabled (clicking does not run it). Always enabled if absent. Cannot be attached to a container. Same rule as `checked`
   * — ★reads only ctx★. A different axis from the command's `when`: `when` is a UI context gate that keybindings 「못 본 것처럼 넘기는」
   * (pass over as if unseen), and the menu does not read it.
   */
  enabled?: (ctx: SlotMenuCtx) => boolean
}

/**
 * The buildSlotMenu output = resolved items the renderer uses as is (+ group boundary marks).
 * If children exist it is a container (1-level submenu) — the renderer does not call run in this case and unfolds children as a hover flyout.
 */
export interface ResolvedSlotMenuItem {
  id: string
  title: string
  run: (args?: Record<string, unknown>) => unknown
  group: string
  /** True if this item is the first of its group and another group came before it (the renderer draws a divider). */
  separatorBefore: boolean
  /** ADR-0065 1-level submenu — the resolved children if a container. undefined if an execution item. */
  children?: ResolvedSlotMenuItem[]
  /** The contribution's [`SlotMenuItem.checked`] as is. If absent it is not a toggle. */
  checked?: (ctx: SlotMenuCtx) => boolean
  /** The contribution's [`SlotMenuItem.enabled`] as is. If absent always enabled. */
  enabled?: (ctx: SlotMenuCtx) => boolean
}

// ★Group render order fixed★ (ADR-0064 §5): content-specific ('content') on top, common slot ops ('slot-ops') below,
//   with a divider between. A group not listed here is pushed back (Infinity) with a stable name-order sort — when adding a new group
//   specify it here to fix the order (no implicit order).
const GROUP_ORDER: Record<string, number> = {
  content: 0,
  'slot-ops': 1,
}

function groupRank(group: string): number {
  return GROUP_ORDER[group] ?? Number.POSITIVE_INFINITY
}

const contributions = new Map<SlotMenuTarget, SlotMenuItem[]>()

function itemKey(item: SlotMenuItem): string {
  return item.commandId ?? `container:${item.title ?? ''}`
}

/**
 * Register a slot menu contribution. Re-contributing the same (target, key) is not silently overwritten but warned about and then replaced
 * (isomorphic to registry.register — reveals HMR re-evaluation, duplicate loads, and typos). idempotent: the last registration wins.
 * key = commandId (execution item) or 'container:'+title (container) — a container has no commandId, so it is identified by title.
 */
export function registerSlotMenu(target: SlotMenuTarget, items: SlotMenuItem[]): void {
  const list = contributions.get(target) ?? []
  for (const item of items) {
    const key = itemKey(item)
    const idx = list.findIndex(existing => itemKey(existing) === key)
    if (idx >= 0) {
      console.warn(`[slotMenu] 중복 기여 재등록 — 교체: target='${target}' key='${key}'`)
      list[idx] = item
    } else {
      list.push(item)
    }
  }
  contributions.set(target, list)
}

/** Independent of registration order — deterministic by (group, order) alone. */
function compareItems(a: SlotMenuItem, b: SlotMenuItem): number {
  const gr = groupRank(a.group) - groupRank(b.group)
  if (gr !== 0) return gr
  // Different group names of the same rank (both unregistered, etc.) — stabilized by name order.
  if (a.group !== b.group) return a.group < b.group ? -1 : 1
  if (a.order !== b.order) return a.order - b.order
  const ka = itemKey(a)
  const kb = itemKey(b)
  return ka < kb ? -1 : ka > kb ? 1 : 0
}

/**
 * null (skip) on failure (unregistered, etc.).
 * ★fail-loud but crash-free (FIX-1)★: it is called at render time (right click), so a throw would blank the app without an error boundary.
 *   Loudly console.error and skip only that item as null (full boot-time validation is validateSlotMenuContributions).
 */
function resolveRunnable(commandId: string, target: SlotContent['type']): ResolvedSlotMenuItem | null {
  const cmd = getCommand(commandId)
  if (!cmd) {
    console.error(`[slotMenu] unregistered commandId "${commandId}" (target=${target}) — skipped`)
    return null
  }
  return { id: cmd.id, title: cmd.title, run: cmd.run, group: '', separatorBefore: false }
}

function withCtxPredicates(entry: ResolvedSlotMenuItem, item: SlotMenuItem): ResolvedSlotMenuItem {
  return {
    ...entry,
    ...(item.checked ? { checked: item.checked } : {}),
    ...(item.enabled ? { enabled: item.enabled } : {}),
  }
}

/**
 * Item shape validation (ADR-0065 §영향/불변식) — execution item XOR container. On violation, log loudly and return false (skip).
 *   - Execution item: commandId present + children absent.
 *   - Container: children present (non-empty) + title present + commandId absent.
 * ★Limited to 1 level★: if nested=true (= this item is already a child of a container), having children again is 2-level nesting and forbidden.
 * ★fail-loud but crash-free★: isomorphic to the FIX-1 style of buildSlotMenu — dev error (console.error) + skip.
 */
function validateItemShape(item: SlotMenuItem, target: SlotMenuTarget, nested: boolean): boolean {
  const hasCmd = typeof item.commandId === 'string' && item.commandId.length > 0
  const hasChildren = Array.isArray(item.children) && item.children.length > 0
  const label = item.commandId ?? item.title ?? '(untitled)'
  if (hasCmd && hasChildren) {
    console.error(`[slotMenu] item "${label}" (target=${target}) — commandId 와 children 을 동시에 가질 수 없음 (실행 항목 XOR 컨테이너) — skipped`)
    return false
  }
  if (!hasCmd && !hasChildren) {
    console.error(`[slotMenu] item "${label}" (target=${target}) — commandId 도 children 도 없음 (실행 항목 XOR 컨테이너 필요) — skipped`)
    return false
  }
  if (hasChildren) {
    if (nested) {
      console.error(`[slotMenu] item "${label}" (target=${target}) — 2단+ 중첩 금지(children 안의 children) — skipped`)
      return false
    }
    if (typeof item.title !== 'string' || item.title.length === 0) {
      console.error(`[slotMenu] container "${label}" (target=${target}) — 컨테이너는 title 필수 — skipped`)
      return false
    }
    for (const field of ['checked', 'enabled'] as const) {
      if (item[field]) {
        console.error(`[slotMenu] container "${label}" (target=${target}) — 컨테이너는 ${field} 를 가질 수 없음 — skipped`)
        return false
      }
    }
  }
  return true
}

/**
 * Gather the contentType contributions ∪ '*' (common) contributions, sort deterministically → registry resolve → an array of renderable items.
 *
 * ★Sorting is independent of registration order — only by (groupRank, order)★ (ADR-0064): so the render does not wobble with
 *   import order or HMR order. For stable sorting, group uses the GROUP_ORDER rank, within it order, and ties break by key (commandId/title).
 * ★hideOn filter (ADR-0065)★: an item whose hideOn includes the contentType is removed first (subtraction — the '*' universality is kept,
 *   only the specific type is excluded). Trims slot.empty on empty slots while keeping the common ops single-source ('*') invariant.
 * ★1-level submenu (ADR-0065)★: a container with children passes the title through + resolves each child's commandId.
 *   ★Children keep declaration order★ (no reordering, unlike the top level) — the contribution already lists them in the intended relative order (ADR-0065
 *   "keeping their relative order"). If a child has children again that is 2-level nesting, so dev error + skip (nested=true).
 *   ★hideOn also applies to children (FIX-2)★: run the same subtraction as the top level on each child (visibility contract = children included).
 *   ★Omit an empty container (FIX-1)★: if all children are skipped (unregistered · 2-level nesting · hideOn) and it empties, the container would be mistaken for a leaf and
 *   become a dead item, so the whole container gets console.error + skip (do not emit it with children:[]).
 * ★dedupe (FIX-2)★: if '*' (common) and contentType (specific) contribute the same key, duplicate items · duplicate React keys result.
 *   After sorting, dedupe by key and leave only the *first appearance* in the final order (the content group sorts before slot-ops,
 *   so if content redeclares a common op the content side wins — though redeclaring a common op should not exist in the first place, and this is a defensive dedupe).
 * ★fail-loud but crash-free (FIX-1)★: an unregistered commandId or a shape violation does not throw but does console.error + skip.
 * separatorBefore: true on the first item where the group changes (excluding the very first) → the renderer draws a divider above it.
 */
export function buildSlotMenu(contentType: SlotContent['type']): ResolvedSlotMenuItem[] {
  // contentType==='*' is impossible by type (SlotContent['type']).
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
          `[slotMenu] container "${item.title}" (target=${contentType}) — 자식이 모두 skip 되어 빈 컨테이너 — omitted`,
        )
        continue
      }
      entry = {
        // A container id is synthesized from the title (no commandId) — for the React key · data-attr. run is a no-op (the renderer does not call it).
        id: `container:${item.title}`,
        title: item.title as string,
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
 * Full boot-time validation (FIX-1 + FIX-3) — inspects every registered contribution right at boot without a right click. Two kinds:
 *   1. **Shape validation (FIX-3)** — run validateItemShape recursively down to the children (both-cmd-and-children / 2-level nesting /
 *      container title missing). The old version looked only at commandId existence, so shape violations were caught only on right click (buildSlotMenu) —
 *      now they are discovered at boot (dev-time).
 *   2. **commandId existence (FIX-1)** — check that the commandId of execution items · container children is in the registry.
 * Both console.error without crashing (minimal boot side effects — paired with the skip+log of buildSlotMenu, do not promote to a throw).
 * ★Unlike buildSlotMenu, it does no sorting/resolve/hideOn★ — pure validation only (visibility is a render concern).
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

/** Test only — contribution reset (isolation between tests). Do not call from production code (mirrors registry.__reset…). */
export function __resetSlotMenuForTest(): void {
  contributions.clear()
}
